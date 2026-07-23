// speaker_diarization_engine/commands.rs
//
// Tauri commands for the speaker diarization engine.

use std::sync::{Arc, Mutex};
use log::{info, warn};
use tauri::{AppHandle, Manager, Runtime};
use std::path::PathBuf;

use super::engine::{SpeakerDiarizationEngine, SpeakerSegment};

/// Global engine instance.
static DIARIZATION_ENGINE: Mutex<Option<Arc<SpeakerDiarizationEngine>>> = Mutex::new(None);

/// Global bundled models directory (resolved at init from resource_dir or dev fallback).
static MODELS_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Set the bundled models directory. Called during app setup.
/// Production: <resource_dir>/models
/// Development: <project>/frontend/src-tauri/sherpa-libs/models
pub fn set_models_directory<R: Runtime>(app: &AppHandle<R>) {
    let models_dir = app.path().resource_dir()
        .map(|rd| rd.join("models"))
        .unwrap_or_else(|_| {
            // Dev fallback: look for sherpa-libs/models next to src-tauri.
            PathBuf::from("sherpa-libs/models")
        });

    if !models_dir.exists() {
        warn!("[Diarization] Bundled models dir not found: {}", models_dir.display());
    }
    info!("[Diarization] Models directory set to: {}", models_dir.display());

    let mut guard = MODELS_DIR.lock().unwrap();
    *guard = Some(models_dir);
}

/// Get the configured bundled models directory.
pub fn get_models_directory() -> PathBuf {
    MODELS_DIR.lock().unwrap().clone()
        .unwrap_or_else(|| PathBuf::from("sherpa-libs/models"))
}

/// Get or create the engine instance.
fn get_engine() -> Arc<SpeakerDiarizationEngine> {
    let mut guard = DIARIZATION_ENGINE.lock().unwrap();
    if guard.is_none() {
        let models_dir = get_models_directory();
        let engine = Arc::new(SpeakerDiarizationEngine::new(models_dir));
        *guard = Some(engine.clone());
        return engine;
    }
    guard.as_ref().unwrap().clone()
}

/// Initialize the diarization engine (sets models dir + creates engine).
#[tauri::command]
pub async fn speaker_diarization_init<R: Runtime>(
    app: AppHandle<R>,
) -> Result<(), String> {
    info!("[Diarization] Initializing engine");
    set_models_directory(&app);
    let _engine = get_engine();
    Ok(())
}

/// Check whether bundled diarization models are present.
#[tauri::command]
pub async fn speaker_diarization_is_ready() -> Result<bool, String> {
    let engine = get_engine();
    Ok(engine.is_ready())
}

/// Run diarization on an audio file path. Decodes the file to 16kHz mono PCM,
/// then runs OfflineSpeakerDiarization. Runs in spawn_blocking to avoid
/// blocking the async runtime.
#[tauri::command]
pub async fn speaker_diarization_process(
    audio_path: String,
) -> Result<Vec<SpeakerSegment>, String> {
    info!("[Diarization] Processing audio: {}", audio_path);

    let engine = get_engine();

    // Load model if not already loaded.
    {
        let needs_load = {
            // We expose load via a fresh check: if is_ready but diarizer None, load.
            // is_ready only checks file existence; the actual diarizer instance is lazy.
            true // always attempt load (load() is idempotent-safe via create)
        };
        if needs_load {
            // Load inside blocking thread to avoid blocking async runtime.
            let engine_clone = engine.clone();
            tokio::task::spawn_blocking(move || engine_clone.load())
                .await
                .map_err(|e| format!("Join error: {}", e))??;
        }
    }

    // Decode audio file -> 16kHz mono f32 samples.
    let path = std::path::Path::new(&audio_path).to_path_buf();
    let decoded = tokio::task::spawn_blocking(move || {
        crate::audio::decoder::decode_audio_file(&path)
    })
    .await
    .map_err(|e| format!("Decode join error: {}", e))?
    .map_err(|e| format!("Failed to decode audio: {}", e))?;

    let samples = decoded.to_whisper_format();
    info!(
        "[Diarization] Decoded {} samples ({:.1}s) at {}Hz",
        samples.len(),
        decoded.duration_seconds,
        decoded.sample_rate
    );

    // Skip if recording is too short to be meaningful.
    if decoded.duration_seconds < 1.0 {
        warn!("[Diarization] Audio too short ({:.2}s), skipping diarization", decoded.duration_seconds);
        return Ok(Vec::new());
    }

    // Run diarization in blocking thread.
    let engine_clone = engine.clone();
    let segments = tokio::task::spawn_blocking(move || engine_clone.diarize(&samples))
        .await
        .map_err(|e| format!("Diarize join error: {}", e))??;

    info!("[Diarization] Returning {} segments", segments.len());
    Ok(segments)
}
