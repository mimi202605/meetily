// sherpa_asr_engine/sherpa_asr_engine.rs
//
// Sherpa-ONNX ASR engine: pure Rust speech recognition using ONNX Runtime.
// Supports SenseVoice-Small (multilingual) and Paraformer-zh (Chinese).
// No Python dependency - statically linked via sherpa-onnx crate.

use log::{info, warn, error};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use serde::{Deserialize, Serialize};
use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig,
    OfflineSenseVoiceModelConfig, OfflineParaformerModelConfig,
};
use tauri::{AppHandle, Manager, Runtime};
use std::sync::OnceLock;

// ============================================================
// Bundled models directory (production: resource_dir/models)
// ============================================================

/// Bundled models directory (resolved lazily; set via set_bundled_models_dir).
static BUNDLED_MODELS_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Set the bundled models directory from the app resource_dir.
/// Called once during app setup. Safe to call multiple times (first wins).
pub fn set_bundled_models_dir<R: Runtime>(app: &AppHandle<R>) {
    let _ = BUNDLED_MODELS_DIR.set(
        app.path().resource_dir()
            .ok()
            .map(|rd| rd.join("models"))
    );
    info!("[SherpaASR] Bundled models dir: {:?}", BUNDLED_MODELS_DIR.get());
}

/// Get the configured bundled models directory, if set.
pub fn get_bundled_models_dir() -> Option<PathBuf> {
    BUNDLED_MODELS_DIR.get().and_then(|o| o.clone())
}

/// Bundled SenseVoice model directory path (production: <resource>/models/sense-voice/<dir>).
fn bundled_sense_voice_dir() -> Option<PathBuf> {
    get_bundled_models_dir().map(|d| {
        d.join("sense-voice")
            .join("sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17")
    })
}

// ============================================================
// Model definitions
// ============================================================

/// Default model: SenseVoice-Small int8 (228MB, supports zh/en/ja/ko/yue)
pub const DEFAULT_MODEL_NAME: &str = "sense-voice-zh-en-ja-ko-yue-int8";

/// Model catalog entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDef {
    pub name: String,
    pub display_name: String,
    pub model_type: String,  // "sense_voice" or "paraformer"
    pub download_url: String,
    /// Mirror URLs for regions where GitHub is blocked (e.g. mainland China)
    #[serde(default)]
    pub mirror_urls: Vec<String>,
    pub size_mb: u64,
    pub description: String,
    /// Directory name after extraction
    pub dir_name: String,
    /// ONNX model filename within the directory
    pub model_file: String,
    /// Tokens filename within the directory
    pub tokens_file: String,
}

/// Model status for discovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: String,
    pub display_name: String,
    pub status: ModelStatus,
    pub size_mb: u64,
    pub description: String,
    pub model_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ModelStatus {
    Available,
    Missing,
    Downloading { progress: u8 },
    Error(String),
}

/// Get the model catalog
pub fn get_model_catalog() -> Vec<ModelDef> {
    // GitHub is blocked in mainland China (DNS error 11001).
    // Mirror URLs are tried in order if the primary URL fails.
    // gh-proxy format: prepend the proxy URL to the full GitHub URL.
    vec![
        ModelDef {
            name: "sense-voice-zh-en-ja-ko-yue-int8".to_string(),
            display_name: "SenseVoice-Small (中英日韩粤)".to_string(),
            model_type: "sense_voice".to_string(),
            download_url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17.tar.bz2".to_string(),
            mirror_urls: vec![
                "https://gh.api.99988866.xyz/https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17.tar.bz2".to_string(),
                "https://ghproxy.net/https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17.tar.bz2".to_string(),
                "https://mirror.ghproxy.com/https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17.tar.bz2".to_string(),
            ],
            size_mb: 228,
            description: "SenseVoice-Small int8 模型。支持中文、英语、日语、韩语、粤语。CPU 极快，自带标点。模型仅 228MB。".to_string(),
            dir_name: "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17".to_string(),
            model_file: "model.int8.onnx".to_string(),
            tokens_file: "tokens.txt".to_string(),
        },
        ModelDef {
            name: "paraformer-zh-int8".to_string(),
            display_name: "Paraformer-zh (中文)".to_string(),
            model_type: "paraformer".to_string(),
            download_url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-paraformer-zh-2023-09-14.tar.bz2".to_string(),
            mirror_urls: vec![
                "https://gh.api.99988866.xyz/https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-paraformer-zh-2023-09-14.tar.bz2".to_string(),
                "https://ghproxy.net/https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-paraformer-zh-2023-09-14.tar.bz2".to_string(),
                "https://mirror.ghproxy.com/https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-paraformer-zh-2023-09-14.tar.bz2".to_string(),
            ],
            size_mb: 223,
            description: "Paraformer-zh int8 模型。FunASR 系列中文语音识别模型，非自回归，推理速度快。支持中文和英文。".to_string(),
            dir_name: "sherpa-onnx-paraformer-zh-2023-09-14".to_string(),
            model_file: "model.int8.onnx".to_string(),
            tokens_file: "tokens.txt".to_string(),
        },
    ]
}

/// Get a model definition by name
pub fn get_model_by_name(name: &str) -> Option<ModelDef> {
    get_model_catalog().into_iter().find(|m| m.name == name)
}

// ============================================================
// Engine state
// ============================================================

/// Sherpa-ONNX ASR engine
pub struct SherpaAsrEngine {
    models_dir: PathBuf,
    recognizer: RwLock<Option<OfflineRecognizer>>,
    current_model: RwLock<Option<String>>,
}

impl SherpaAsrEngine {
    pub fn new(models_dir: PathBuf) -> Self {
        Self {
            models_dir,
            recognizer: RwLock::new(None),
            current_model: RwLock::new(None),
        }
    }

    /// Get the model directory path for a given model.
    /// Checks bundled dir first (production), falls back to app_data models dir.
    pub fn model_path(&self, model_name: &str) -> Option<PathBuf> {
        let model_def = get_model_by_name(model_name)?;

        // 1. Bundled (production) path for SenseVoice (only if files actually exist).
        if model_name == DEFAULT_MODEL_NAME {
            if let Some(bundled) = bundled_sense_voice_dir() {
                if bundled.exists()
                    && bundled.join(&model_def.model_file).exists()
                    && bundled.join(&model_def.tokens_file).exists()
                {
                    return Some(bundled);
                }
                log::warn!(
                    "[SherpaASR] Bundled model dir not found or incomplete: {}, falling back to app_data",
                    bundled.display()
                );
            }
        }

        // 2. Fallback: app_data models dir (dev or downloaded models).
        Some(self.models_dir.join("sherpa_asr").join(&model_def.dir_name))
    }

    /// Check if a model is downloaded
    pub fn is_model_downloaded(&self, model_name: &str) -> bool {
        if let Some(model_dir) = self.model_path(model_name) {
            let model_def = get_model_by_name(model_name);
            if let Some(def) = model_def {
                return model_dir.join(&def.model_file).exists()
                    && model_dir.join(&def.tokens_file).exists();
            }
        }
        false
    }

    /// Discover available models
    pub fn discover_models(&self) -> Vec<ModelInfo> {
        let catalog = get_model_catalog();
        catalog
            .into_iter()
            .map(|def| {
                let status = if self.is_model_downloaded(&def.name) {
                    ModelStatus::Available
                } else {
                    ModelStatus::Missing
                };
                ModelInfo {
                    name: def.name,
                    display_name: def.display_name,
                    status,
                    size_mb: def.size_mb,
                    description: def.description,
                    model_type: def.model_type,
                }
            })
            .collect()
    }

    /// Check if any models are available (downloaded)
    pub fn has_available_models(&self) -> bool {
        get_model_catalog()
            .iter()
            .any(|def| self.is_model_downloaded(&def.name))
    }

    /// Check if model is loaded (recognizer exists)
    pub fn is_model_loaded(&self) -> bool {
        self.recognizer.read().unwrap().is_some()
    }

    /// Get current model name
    pub fn get_current_model(&self) -> Option<String> {
        self.current_model.read().unwrap().clone()
    }

    /// Load a model into the recognizer
    pub fn load_model(&self, model_name: &str) -> Result<(), String> {
        let model_def = get_model_by_name(model_name)
            .ok_or_else(|| format!("Unknown model: {}", model_name))?;

        let model_dir = self.model_path(model_name)
            .ok_or_else(|| "Failed to get model path".to_string())?;

        let model_file = model_dir.join(&model_def.model_file);
        let tokens_file = model_dir.join(&model_def.tokens_file);

        if !model_file.exists() {
            return Err(format!("Model file not found: {}", model_file.display()));
        }
        if !tokens_file.exists() {
            return Err(format!("Tokens file not found: {}", tokens_file.display()));
        }

        info!("[SherpaASR] Loading model: {} ({})", model_name, model_def.model_type);
        let start = std::time::Instant::now();

        let mut config = OfflineRecognizerConfig::default();
        config.model_config.tokens = Some(tokens_file.to_string_lossy().to_string());
        config.model_config.num_threads = num_cpus();
        config.model_config.debug = false;
        config.model_config.provider = Some("cpu".to_string());

        match model_def.model_type.as_str() {
            "sense_voice" => {
                config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
                    model: Some(model_file.to_string_lossy().to_string()),
                    language: Some("auto".to_string()),
                    use_itn: true,
                };
            }
            "paraformer" => {
                config.model_config.paraformer = OfflineParaformerModelConfig {
                    model: Some(model_file.to_string_lossy().to_string()),
                };
            }
            _ => return Err(format!("Unknown model type: {}", model_def.model_type)),
        }

        let recognizer = OfflineRecognizer::create(&config)
            .ok_or_else(|| "Failed to create recognizer".to_string())?;

        *self.recognizer.write().unwrap() = Some(recognizer);
        *self.current_model.write().unwrap() = Some(model_name.to_string());

        let elapsed = start.elapsed();
        info!("[SherpaASR] Model loaded in {:.2}s: {}", elapsed.as_secs_f64(), model_name);

        Ok(())
    }

    /// Transcribe audio samples
    pub fn transcribe_audio(
        &self,
        samples: &[f32],
        sample_rate: i32,
        _language: Option<String>,
    ) -> Result<String, String> {
        let recognizer_guard = self.recognizer.read().unwrap();
        let recognizer = recognizer_guard
            .as_ref()
            .ok_or_else(|| "No model loaded. Please load a model first.".to_string())?;

        let stream = recognizer.create_stream();
        stream.accept_waveform(sample_rate, samples);
        recognizer.decode(&stream);

        let result = stream.get_result()
            .ok_or_else(|| "Failed to get result".to_string())?;

        Ok(result.text)
    }

    /// Validate that a model is ready for transcription
    pub fn validate_model_ready(&self, model_name: &str) -> Result<String, String> {
        if !self.is_model_downloaded(model_name) {
            return Err(format!("Model '{}' not downloaded. Please download it first.", model_name));
        }

        // Load model if not already loaded
        if !self.is_model_loaded() || self.get_current_model().as_deref() != Some(model_name) {
            self.load_model(model_name)?;
        }

        Ok(model_name.to_string())
    }
}

/// Get the number of CPU cores (with a sane cap)
fn num_cpus() -> i32 {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4);
    // Cap at 8 to avoid oversubscription
    cores.min(8).max(1)
}

/// Global engine instance
pub static SHERPA_ASR_ENGINE: once_cell::sync::Lazy<RwLock<Option<Arc<SherpaAsrEngine>>>> =
    once_cell::sync::Lazy::new(|| RwLock::new(None));

/// Get or initialize the engine
pub fn get_or_init_engine(models_dir: PathBuf) -> Arc<SherpaAsrEngine> {
    let mut guard = SHERPA_ASR_ENGINE.write().unwrap();
    if guard.is_none() {
        let engine = Arc::new(SherpaAsrEngine::new(models_dir));
        *guard = Some(engine.clone());
        return engine;
    }
    guard.as_ref().unwrap().clone()
}
