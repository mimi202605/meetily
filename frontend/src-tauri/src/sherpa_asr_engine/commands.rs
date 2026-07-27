// sherpa_asr_engine/commands.rs
//
// Tauri commands for the Sherpa-ONNX ASR engine.

use std::sync::{Arc, Mutex};
use log::{info, warn, error};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use std::path::PathBuf;

use super::sherpa_asr_engine::{
    SherpaAsrEngine, ModelInfo, ModelStatus,
    get_model_catalog, get_model_by_name, DEFAULT_MODEL_NAME,
};

/// Global engine instance (separate from the one in sherpa_asr_engine.rs for command access)
pub static SHERPA_ENGINE: Mutex<Option<Arc<SherpaAsrEngine>>> = Mutex::new(None);

/// Global models directory
static MODELS_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Set the models directory (called during app setup)
pub fn set_models_directory<R: Runtime>(app: &AppHandle<R>) {
    let app_data_dir = app.path().app_data_dir()
        .expect("Failed to get app data dir");
    let models_dir = app_data_dir.join("models");
    if !models_dir.exists() {
        let _ = std::fs::create_dir_all(&models_dir);
    }
    info!("[SherpaASR] Models directory set to: {}", models_dir.display());
    let mut guard = MODELS_DIR.lock().unwrap();
    *guard = Some(models_dir);
}

/// Get models directory
pub fn get_models_directory() -> PathBuf {
    MODELS_DIR.lock().unwrap().clone()
        .unwrap_or_else(|| PathBuf::from("models"))
}

/// Get or create the engine instance
fn get_engine() -> Arc<SherpaAsrEngine> {
    let mut guard = SHERPA_ENGINE.lock().unwrap();
    if guard.is_none() {
        let models_dir = get_models_directory();
        let engine = Arc::new(SherpaAsrEngine::new(models_dir));
        *guard = Some(engine.clone());
        return engine;
    }
    guard.as_ref().unwrap().clone()
}

/// Initialize the Sherpa-ASR engine
#[tauri::command]
pub async fn sherpa_asr_init() -> Result<(), String> {
    info!("[SherpaASR] Initializing engine");
    let _engine = get_engine();
    Ok(())
}

/// Get available Sherpa-ASR models
#[tauri::command]
pub async fn sherpa_asr_get_available_models() -> Result<Vec<ModelInfo>, String> {
    let engine = get_engine();
    Ok(engine.discover_models())
}

/// Check if any Sherpa-ASR models are available (bundled or downloaded).
#[tauri::command]
pub async fn sherpa_asr_has_available_models() -> Result<bool, String> {
    // Check if bundled SenseVoice model files actually exist on disk.
    let engine = get_engine();
    if engine.is_model_downloaded(super::sherpa_asr_engine::DEFAULT_MODEL_NAME) {
        return Ok(true);
    }
    // Fallback: check downloaded models in app_data.
    Ok(engine.has_available_models())
}

/// Download a Sherpa-ASR model
#[tauri::command]
pub async fn sherpa_asr_download_model(
    app_handle: tauri::AppHandle,
    model_name: String,
) -> Result<(), String> {
    info!("[SherpaASR] Starting download for model: {}", model_name);

    let model_def = get_model_by_name(&model_name)
        .ok_or_else(|| format!("Unknown model: {}", model_name))?;

    let models_dir = get_models_directory();
    let sherpa_dir = models_dir.join("sherpa_asr");
    if !sherpa_dir.exists() {
        std::fs::create_dir_all(&sherpa_dir)
            .map_err(|e| format!("Failed to create sherpa_asr dir: {}", e))?;
    }

    // Check if already downloaded
    let model_dir = sherpa_dir.join(&model_def.dir_name);
    let model_file = model_dir.join(&model_def.model_file);
    let tokens_file = model_dir.join(&model_def.tokens_file);

    if model_file.exists() && tokens_file.exists() {
        info!("[SherpaASR] Model already downloaded: {}", model_name);
        app_handle.emit("model-download-complete", serde_json::json!({
            "modelName": model_name
        })).ok();
        return Ok(());
    }

    // Download the tar.bz2 file
    let tar_path = sherpa_dir.join(format!("{}.tar.bz2", model_def.dir_name));

    // Build list of URLs to try: primary first, then mirrors (for regions where GitHub is blocked)
    let mut url_list = vec![model_def.download_url.clone()];
    url_list.extend(model_def.mirror_urls.iter().cloned());

    info!("[SherpaASR] Download URLs (will try in order):");
    for (i, u) in url_list.iter().enumerate() {
        info!("[SherpaASR]   {}: {}", i, u);
    }

    // Emit download started
    app_handle.emit("model-download-progress", serde_json::json!({
        "modelName": model_name,
        "progress": 0,
        "downloaded_mb": 0.0,
        "total_mb": model_def.size_mb
    })).ok();

    // Use reqwest blocking client in a separate thread to avoid blocking async runtime
    let tar_path_clone = tar_path.clone();
    let app_handle_clone = app_handle.clone();
    let model_name_clone = model_name.clone();
    let expected_size_bytes = model_def.size_mb * 1024 * 1024;

    let download_result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        use std::io::{Read, Write};

        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

        let mut downloaded: u64 = 0;
        let mut last_error: String = String::new();
        const MAX_RETRIES_PER_URL: u32 = 3;

        // Try each URL in order; on read errors, retry with HTTP Range to resume.
        for (url_idx, url) in url_list.iter().enumerate() {
            for attempt in 0..MAX_RETRIES_PER_URL {
                if attempt > 0 {
                    info!("[SherpaASR] Retrying URL {} (attempt {}/{}) after read error, resuming from {} bytes", url_idx, attempt + 1, MAX_RETRIES_PER_URL, downloaded);
                    std::thread::sleep(std::time::Duration::from_secs(2));
                } else {
                    info!("[SherpaASR] Trying URL {}: {}", url_idx, url);
                }

                // Build request; use Range header to resume from current position if applicable
                let mut req = client.get(url);
                if downloaded > 0 {
                    req = req.header("Range", format!("bytes={}-", downloaded));
                }

                let resp = match req.send() {
                    Ok(r) => r,
                    Err(e) => {
                        last_error = format!("Failed to connect to {}: {}", url, e);
                        warn!("[SherpaASR] URL {} connection failed: {}", url_idx, e);
                        break; // Connection failed - try next URL
                    }
                };

                let status = resp.status();
                let is_resuming = status == reqwest::StatusCode::PARTIAL_CONTENT;

                if !status.is_success() {
                    last_error = format!("HTTP {} from {}", status, url);
                    warn!("[SherpaASR] URL {} returned status: {}", url_idx, status);
                    break; // HTTP error - try next URL
                }

                if is_resuming {
                    info!("[SherpaASR] Resuming download from {} bytes", downloaded);
                } else if downloaded > 0 {
                    // Server didn't honor Range request (returned 200 OK) - restart from beginning
                    info!("[SherpaASR] Server doesn't support Range, restarting from beginning");
                    downloaded = 0;
                }

                // Determine effective total size
                let content_length = resp.content_length().unwrap_or(0);
                let effective_total = if content_length > 0 {
                    if is_resuming {
                        downloaded + content_length
                    } else {
                        content_length
                    }
                } else {
                    info!("[SherpaASR] Content-Length not provided, using expected size: {} bytes", expected_size_bytes);
                    expected_size_bytes
                };
                info!("[SherpaASR] Download size: {} bytes (effective total: {} bytes)", content_length, effective_total);

                // Open file: append if resuming, create/truncate if starting fresh
                let mut file = if is_resuming {
                    std::fs::OpenOptions::new()
                        .write(true)
                        .append(true)
                        .open(&tar_path_clone)
                        .map_err(|e| format!("Failed to open file for append: {}", e))?
                } else {
                    std::fs::File::create(&tar_path_clone)
                        .map_err(|e| format!("Failed to create temp file: {}", e))?
                };

                let mut buffer = [0u8; 65536]; // 64KB buffer
                let mut stream = resp;
                let mut last_progress: u8 = ((downloaded as f64 / effective_total as f64) * 100.0).min(100.0) as u8;
                let mut read_failed = false;

                loop {
                    match stream.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(n) => {
                            file.write_all(&buffer[..n])
                                .map_err(|e| format!("File write error: {}", e))?;

                            downloaded += n as u64;

                            let progress = ((downloaded as f64 / effective_total as f64) * 100.0).min(100.0) as u8;
                            let downloaded_mb = (downloaded as f64) / (1024.0 * 1024.0);
                            let total_mb = (effective_total as f64) / (1024.0 * 1024.0);

                            if progress >= last_progress + 2 || progress >= 100 || downloaded >= effective_total {
                                let _ = app_handle_clone.emit("model-download-progress", serde_json::json!({
                                    "modelName": model_name_clone,
                                    "progress": progress,
                                    "downloaded_mb": (downloaded_mb * 10.0).round() / 10.0,
                                    "total_mb": (total_mb * 10.0).round() / 10.0
                                }));
                                last_progress = progress;
                            }
                        }
                        Err(e) => {
                            last_error = format!("Download read error: {}", e);
                            warn!("[SherpaASR] Read error on URL {} attempt {}: {}", url_idx, attempt + 1, e);
                            read_failed = true;
                            break;
                        }
                    }
                }

                file.flush().ok();
                drop(file);

                if !read_failed {
                    // Download completed successfully
                    info!("[SherpaASR] Download complete: {} bytes", downloaded);
                    return Ok(());
                }

                // Read error occurred - will retry same URL with Range to resume
            }
        }

        Err(format!(
            "All download URLs failed after retries. Last error: {}. If you are in mainland China, please check your network connection.",
            last_error
        ))
    }).await
    .map_err(|e| format!("Download task failed: {}", e))?;

    if let Err(e) = download_result {
        error!("[SherpaASR] Download failed: {}", e);
        app_handle.emit("model-download-error", serde_json::json!({
            "modelName": model_name,
            "error": e
        })).ok();
        // Clean up partial download
        let _ = std::fs::remove_file(&tar_path);
        return Err(e);
    }

    // Extract tar.bz2 using system tar command (available on Windows 10+)
    info!("[SherpaASR] Extracting: {}", tar_path.display());

    let extract_result = std::process::Command::new("tar")
        .arg("xjf")
        .arg(&tar_path)
        .arg("-C")
        .arg(&sherpa_dir)
        .output();

    // Clean up tar.bz2
    let _ = std::fs::remove_file(&tar_path);

    match extract_result {
        Ok(output) => {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                error!("[SherpaASR] Extraction failed: {}", stderr);
                return Err(format!("Failed to extract model: {}", stderr));
            }
            info!("[SherpaASR] Extraction complete");
        }
        Err(e) => {
            error!("[SherpaASR] Failed to run tar command: {}", e);
            return Err(format!("Failed to run tar command: {}. Make sure tar is available (Windows 10+ includes it).", e));
        }
    }

    // Verify extraction
    if !model_file.exists() || !tokens_file.exists() {
        error!("[SherpaASR] Model files not found after extraction");
        return Err("Model files not found after extraction".to_string());
    }

    // Emit completion
    app_handle.emit("model-download-complete", serde_json::json!({
        "modelName": model_name
    })).ok();

    info!("[SherpaASR] Model download complete: {}", model_name);
    Ok(())
}

/// Load a Sherpa-ASR model
#[tauri::command]
pub async fn sherpa_asr_load_model(
    model_name: String,
) -> Result<(), String> {
    info!("[SherpaASR] Loading model: {}", model_name);
    let engine = get_engine();
    engine.load_model(&model_name)
}

/// Validate that a Sherpa-ASR model is ready (internal generic version)
pub async fn sherpa_asr_validate_model_ready_internal<R: Runtime>(
    app_handle: &AppHandle<R>,
) -> Result<String, String> {
    info!("[SherpaASR] Validating model readiness");

    // Get the model from transcript config, or use default
    let config = match crate::api::api::api_get_transcript_config(
        app_handle.clone(),
        app_handle.clone().state(),
        None,
    ).await {
        Ok(Some(config)) => config,
        _ => {
            crate::api::api::TranscriptConfig {
                provider: "sherpaAsr".to_string(),
                model: DEFAULT_MODEL_NAME.to_string(),
                api_key: None,
            }
        }
    };

    let model_name = if config.model.is_empty() {
        DEFAULT_MODEL_NAME.to_string()
    } else {
        config.model
    };

    let engine = get_engine();
    engine.validate_model_ready(&model_name)
}

/// Validate that a Sherpa-ASR model is ready (Tauri command wrapper)
#[tauri::command]
pub async fn sherpa_asr_validate_model_ready(
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    sherpa_asr_validate_model_ready_internal(&app_handle).await
}

/// Check if model is loaded
#[tauri::command]
pub async fn sherpa_asr_is_model_loaded() -> Result<bool, String> {
    let engine = get_engine();
    Ok(engine.is_model_loaded())
}

/// Get current model name
#[tauri::command]
pub async fn sherpa_asr_get_current_model() -> Result<Option<String>, String> {
    let engine = get_engine();
    Ok(engine.get_current_model())
}

/// Stop/cleanup the engine
#[tauri::command]
pub async fn sherpa_asr_stop() -> Result<(), String> {
    info!("[SherpaASR] Stopping engine");
    let engine = get_engine();
    // The recognizer is dropped when we replace it with None
    // But we can't directly clear it from here since it's behind RwLock
    // Just log - the engine will be recreated if needed
    Ok(())
}

/// Get the default model name
#[tauri::command]
pub async fn sherpa_asr_get_default_model() -> Result<String, String> {
    Ok(DEFAULT_MODEL_NAME.to_string())
}
