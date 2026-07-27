// speaker_diarization_engine/commands.rs
//
// Tauri commands for the speaker diarization engine.

use std::sync::{Arc, Mutex};
use log::{info, warn};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use std::path::PathBuf;

use super::engine::{SpeakerDiarizationEngine, SpeakerSegment, align_transcripts_with_speakers, TranscriptChunkForAlignment};
use crate::api::TranscriptSegment;
use crate::database::models::Transcript;
use crate::database::repositories::meeting::MeetingsRepository;
use crate::state::AppState;

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

/// Run diarization on an audio file and update transcript segments with speaker labels.
/// This is a reusable helper for import and retranscription flows.
///
/// - Decodes audio → PCM samples
/// - Runs OfflineSpeakerDiarization
/// - Aligns speaker segments with transcript chunks by timestamp overlap
/// - Updates `segments` in place with speaker IDs
/// - Optionally matches voiceprints to populate `speaker_name`
/// - Rewrites transcripts.json in `folder`
/// - Emits `transcript-diarized` event so the frontend can update
pub async fn run_diarization_on_segments<R: Runtime>(
    app: &AppHandle<R>,
    folder: &std::path::Path,
    audio_path: &std::path::Path,
    segments: &mut [crate::api::TranscriptSegment],
    meeting_id: Option<&str>,
) -> Result<(), String> {
    use crate::audio::common::write_transcripts_json;

    // Emit "processing started" toast trigger.
    let _ = app.emit("transcript-diarization-started", serde_json::json!({}));

    // Ensure engine is initialized.
    set_models_directory(app);

    // Run diarization (decodes audio + runs model).
    let audio_path_str = audio_path.to_string_lossy().to_string();
    let speaker_segments = speaker_diarization_process(audio_path_str).await?;

    if speaker_segments.is_empty() {
        info!("[Diarization] No segments returned (audio too short or no speech); skipping speaker labels");
        let _ = app.emit("transcript-diarization-error", serde_json::json!({"error": "no_speech"}));
        return Ok(());
    }

    // Build alignment chunks from transcript segments.
    let chunks: Vec<TranscriptChunkForAlignment> = segments
        .iter()
        .map(|s| TranscriptChunkForAlignment {
            id: s.id.clone(),
            audio_start_time: s.audio_start_time.unwrap_or(0.0),
            audio_end_time: s.audio_end_time.unwrap_or(0.0),
            speaker: None,
        })
        .collect();

    let aligned = align_transcripts_with_speakers(chunks, &speaker_segments);

    // Apply aligned speaker IDs back onto segments.
    // Clear speaker_name so that a stale name from a previous run (potentially
    // tied to a different speaker ID) does not persist when the new speaker ID
    // has no voiceprint match. Voiceprint matching below repopulates it.
    for (seg, aligned_chunk) in segments.iter_mut().zip(aligned.iter()) {
        seg.speaker = aligned_chunk.speaker;
        seg.speaker_name = None;
    }

    // === Voiceprint matching ===
    // After alignment assigns speaker IDs, attempt to match each speaker cluster
    // against registered voiceprints. Non-fatal: failures are logged and skipped.
    let mut speaker_names: std::collections::HashMap<i32, String> = std::collections::HashMap::new();
    if let Some(mid) = meeting_id {
        if let Ok(voiceprint_engine) = crate::voiceprint_engine::engine::get_engine() {
            match try_match_voiceprints(app, audio_path, &speaker_segments, &voiceprint_engine, Some(mid)).await {
                Ok(names) => {
                    for (sid, name) in &names {
                        speaker_names.insert(*sid, name.clone());
                    }
                    for seg in segments.iter_mut() {
                        if let Some(sid) = seg.speaker {
                            if let Some(name) = speaker_names.get(&sid) {
                                seg.speaker_name = Some(name.clone());
                            }
                        }
                    }
                }
                Err(e) => log::warn!("[Diarization] Voiceprint matching failed (non-fatal): {}", e),
            }
        }
    }

    // Rewrite transcripts.json with speaker field populated.
    if let Err(e) = write_transcripts_json(folder, segments) {
        warn!("[Diarization] Failed to rewrite transcripts.json: {}", e);
    }

    let num_speakers = speaker_segments.iter().map(|s| s.speaker).max().map(|m| m + 1).unwrap_or(0);
    info!(
        "[Diarization] Success: {} speakers, {} segments labeled",
        num_speakers,
        segments.len()
    );

    // Build speaker_names payload: map every speaker ID (0..num_speakers) to an
    // optional name so the frontend can render labels for all detected speakers.
    let speaker_names_payload: std::collections::HashMap<i32, Option<String>> = (0..num_speakers)
        .map(|sid| (sid, speaker_names.get(&sid).cloned()))
        .collect();

    // Emit diarized transcripts to frontend.
    let _ = app.emit(
        "transcript-diarized",
        serde_json::json!({
            "transcripts": segments,
            "num_speakers": num_speakers,
            "speaker_names": speaker_names_payload,
        }),
    );

    Ok(())
}

/// Helper: match voiceprints against speaker segments.
///
/// Decodes audio, extracts an embedding per speaker cluster (averaging segments
/// that share the same speaker ID), computes a centroid, and matches it against
/// registered voiceprints. When `meeting_id` is `Some`, persists auto overrides
/// to the database; when `None`, only returns the name mapping (used by the live
/// recording flow where no meeting record exists yet).
///
/// `extract_embedding` uses `block_in_place` internally, so it is called directly
/// from this async context — never from `spawn_blocking`.
pub async fn try_match_voiceprints<R: Runtime>(
    app: &AppHandle<R>,
    audio_path: &std::path::Path,
    speaker_segments: &[super::engine::SpeakerSegment],
    voiceprint_engine: &std::sync::Arc<crate::voiceprint_engine::engine::VoiceprintEngine>,
    meeting_id: Option<&str>,
) -> Result<std::collections::HashMap<i32, String>, String> {
    use crate::voiceprint_engine::repository::{list_voiceprints, upsert_override};

    let state = app.state::<crate::state::AppState>();
    let pool = state.db_manager.pool();

    // Load registered voiceprints
    let vp_records = list_voiceprints(pool).await?;
    if vp_records.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let vp_with_embeddings: Vec<(String, String, Vec<f32>)> = vp_records.iter()
        .map(|r| {
            let emb = crate::voiceprint_engine::engine::VoiceprintEngine::deserialize_embedding(&r.embedding);
            (r.id.clone(), r.name.clone(), emb)
        })
        .collect();

    // Decode audio (decode_audio_file does not use block_in_place → spawn_blocking is safe)
    let audio_path_clone = audio_path.to_path_buf();
    let decoded = tokio::task::spawn_blocking(move || crate::audio::decoder::decode_audio_file(&audio_path_clone))
        .await
        .map_err(|e| format!("Decode join error: {}", e))?
        .map_err(|e| format!("Failed to decode audio: {}", e))?;
    let samples = decoded.to_whisper_format();
    let sample_rate = 16000.0f32;

    // Extract embeddings per speaker cluster (directly in async context — extract_embedding
    // uses block_in_place internally and must NOT be called from spawn_blocking).
    let mut cluster_embeddings: std::collections::HashMap<i32, Vec<Vec<f32>>> = std::collections::HashMap::new();
    for seg in speaker_segments {
        let start_sample = (seg.start * sample_rate) as usize;
        let end_sample = (seg.end * sample_rate) as usize;
        if end_sample <= start_sample || end_sample > samples.len() { continue; }
        let seg_samples = samples[start_sample..end_sample].to_vec();
        match voiceprint_engine.extract_embedding(&seg_samples) {
            Ok(emb) => { cluster_embeddings.entry(seg.speaker).or_default().push(emb); }
            Err(e) => log::warn!("[Voiceprint] Embedding extract failed: {}", e),
        }
    }

    // Compute centroids and match
    let mut result = std::collections::HashMap::new();
    for (speaker_id, embs) in &cluster_embeddings {
        if embs.is_empty() { continue; }
        let dim = embs[0].len();
        let mut sum = vec![0.0f32; dim];
        for emb in embs { for (i, &v) in emb.iter().enumerate() { sum[i] += v; } }
        let avg: Vec<f32> = sum.iter().map(|v| v / embs.len() as f32).collect();
        let norm: f32 = avg.iter().map(|v| v * v).sum::<f32>().sqrt();
        let centroid: Vec<f32> = if norm > 0.0 { avg.iter().map(|v| v / norm).collect() } else { avg };

        if let Some(m) = crate::voiceprint_engine::engine::VoiceprintEngine::match_against(&centroid, &vp_with_embeddings, 0.6) {
            // Only persist overrides when a meeting_id is available
            if let Some(mid) = meeting_id {
                let _ = upsert_override(pool, mid, *speaker_id, &m.voiceprint_id, "auto").await;
            }
            result.insert(*speaker_id, m.name);
        }
    }
    Ok(result)
}

/// Manually trigger speaker diarization for a meeting.
///
/// Loads transcripts from the database, finds the meeting's audio file,
/// runs diarization via `run_diarization_on_segments`, and persists the
/// updated speaker labels back to the database.
///
/// Emits the same `transcript-diarization-started` / `transcript-diarized` /
/// `transcript-diarization-error` events as the automatic flow so the
/// frontend's existing event listeners handle UI updates.
#[tauri::command]
pub async fn run_speaker_diarization(
    app: AppHandle,
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<(), String> {
    info!("[Diarization] Manual trigger for meeting {}", meeting_id);

    let pool = state.db_manager.pool();

    // 1. Resolve the meeting folder path from the database.
    let meeting = MeetingsRepository::get_meeting_metadata(pool, &meeting_id)
        .await
        .map_err(|e| format!("Failed to query meeting: {}", e))?
        .ok_or_else(|| format!("Meeting not found: {}", meeting_id))?;

    let folder_path_str = meeting
        .folder_path
        .ok_or_else(|| format!("Meeting {} has no folder_path", meeting_id))?;
    let folder = std::path::Path::new(&folder_path_str);

    if !folder.exists() {
        return Err(format!(
            "Meeting folder does not exist: {}",
            folder.display()
        ));
    }

    // 2. Locate the audio file inside the meeting folder.
    let audio_path = find_audio_file(folder)
        .ok_or_else(|| format!("No audio file found in {}", folder.display()))?;
    info!("[Diarization] Using audio file: {}", audio_path.display());

    // 3. Load existing transcripts from the database.
    let transcripts: Vec<Transcript> =
        sqlx::query_as::<_, Transcript>(
            "SELECT * FROM transcripts WHERE meeting_id = ? ORDER BY audio_start_time",
        )
        .bind(&meeting_id)
        .fetch_all(pool)
        .await
        .map_err(|e| format!("Failed to query transcripts: {}", e))?;

    if transcripts.is_empty() {
        return Err(format!("No transcripts found for meeting {}", meeting_id));
    }

    // Convert DB Transcript rows to TranscriptSegment for diarization alignment.
    let mut segments: Vec<TranscriptSegment> = transcripts
        .iter()
        .map(|t| TranscriptSegment {
            id: t.id.clone(),
            text: t.transcript.clone(),
            timestamp: t.timestamp.clone(),
            audio_start_time: t.audio_start_time,
            audio_end_time: t.audio_end_time,
            duration: t.duration,
            speaker: t.speaker.as_ref().and_then(|s| s.parse().ok()),
            speaker_name: t.speaker_name.clone(),
        })
        .collect();

    // 4. Run diarization (decodes audio, aligns speakers, rewrites transcripts.json,
    //    and emits `transcript-diarized` to the frontend).
    run_diarization_on_segments(&app, folder, &audio_path, &mut segments, Some(&meeting_id)).await?;

    // 5. Persist updated speaker labels back to the database (best-effort).
    //    Update ALL segments: when speaker is None, bind NULL to clear any
    //    stale label from a previous diarization run, keeping the DB in sync
    //    with transcripts.json.
    for seg in &segments {
        let _ = sqlx::query(
            "UPDATE transcripts SET speaker = ?, speaker_name = ? WHERE id = ? AND meeting_id = ?",
        )
        .bind(seg.speaker.map(|s| s.to_string()))
        .bind(&seg.speaker_name)
        .bind(&seg.id)
        .bind(&meeting_id)
        .execute(pool)
        .await;
    }

    Ok(())
}

/// Find an audio file inside a meeting folder.
///
/// Tries the `audio_file` field in `metadata.json` first, then falls back to
/// scanning the folder for common audio extensions (`.wav` preferred).
fn find_audio_file(folder: &std::path::Path) -> Option<std::path::PathBuf> {
    // 1. Try metadata.json -> audio_file
    let metadata_path = folder.join("metadata.json");
    if let Ok(content) = std::fs::read_to_string(&metadata_path) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(audio_file) = json.get("audio_file").and_then(|v| v.as_str()) {
                let path = folder.join(audio_file);
                if path.exists() {
                    return Some(path);
                }
            }
        }
    }

    // 2. Fallback: scan the folder for audio files, preferring .wav then .mp4.
    let preferred_order = [
        "wav", "mp4", "m4a", "mp3", "flac", "ogg", "aac", "mkv", "webm", "wma",
    ];
    let entries: Vec<_> = std::fs::read_dir(folder).ok()?.flatten().collect();

    for ext in &preferred_order {
        for entry in &entries {
            let path = entry.path();
            if let Some(ext_str) = path.extension().and_then(|e| e.to_str()) {
                if ext_str.eq_ignore_ascii_case(ext) {
                    return Some(path);
                }
            }
        }
    }

    None
}
