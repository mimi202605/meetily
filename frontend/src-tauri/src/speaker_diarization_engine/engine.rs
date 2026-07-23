// speaker_diarization_engine/engine.rs
//
// Wraps sherpa-onnx OfflineSpeakerDiarization to label speakers in meeting audio.
// Post-processing only: called after recording stops, never during real-time ASR.

use log::{info, warn, error};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use sherpa_onnx::{
    FastClusteringConfig,
    OfflineSpeakerDiarization,
    OfflineSpeakerDiarizationConfig,
    OfflineSpeakerSegmentationModelConfig,
    OfflineSpeakerSegmentationPyannoteModelConfig,
    SpeakerEmbeddingExtractorConfig,
};

/// A single speaker segment with start/end times (seconds from recording start)
/// and an integer speaker ID (0, 1, 2, ...).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpeakerSegment {
    pub start: f32,
    pub end: f32,
    pub speaker: i32,
}

/// Bundled model filename constants (relative to bundled models dir).
pub const PYANNOTE_MODEL_DIR: &str = "sherpa-onnx-pyannote-segmentation-3-0";
pub const PYANNOTE_MODEL_FILE: &str = "model.onnx";
pub const ERES2NET_MODEL_FILE: &str = "3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx";

/// Speaker diarization engine wrapping sherpa-onnx OfflineSpeakerDiarization.
pub struct SpeakerDiarizationEngine {
    diarizer: RwLock<Option<OfflineSpeakerDiarization>>,
    models_dir: PathBuf,
}

impl SpeakerDiarizationEngine {
    pub fn new(models_dir: PathBuf) -> Self {
        Self {
            diarizer: RwLock::new(None),
            models_dir,
        }
    }

    /// Bundled diarization models directory (pyannote + eres2net).
    fn diarization_models_dir(&self) -> PathBuf {
        self.models_dir.join("speaker-diarization")
    }

    /// Pyannote segmentation model path.
    fn pyannote_model_path(&self) -> PathBuf {
        self.diarization_models_dir()
            .join(PYANNOTE_MODEL_DIR)
            .join(PYANNOTE_MODEL_FILE)
    }

    /// ERes2Net speaker embedding model path.
    fn eres2net_model_path(&self) -> PathBuf {
        self.diarization_models_dir().join(ERES2NET_MODEL_FILE)
    }

    /// Check that both bundled model files exist.
    pub fn is_ready(&self) -> bool {
        let pyannote = self.pyannote_model_path();
        let eres2net = self.eres2net_model_path();
        let ready = pyannote.exists() && eres2net.exists();
        if !ready {
            info!(
                "[Diarization] Models not ready: pyannote={} exists={}, eres2net={} exists={}",
                pyannote.display(),
                pyannote.exists(),
                eres2net.display(),
                eres2net.exists()
            );
        }
        ready
    }

    /// Build OfflineSpeakerDiarization with bundled models.
    /// num_clusters = 0 means auto-detect speaker count.
    pub fn load(&self) -> Result<(), String> {
        if !self.is_ready() {
            return Err("Diarization models not found".to_string());
        }

        let pyannote_path = self.pyannote_model_path();
        let eres2net_path = self.eres2net_model_path();

        info!(
            "[Diarization] Loading models: pyannote={}, eres2net={}",
            pyannote_path.display(),
            eres2net_path.display()
        );
        let start = std::time::Instant::now();

        let segmentation = OfflineSpeakerSegmentationModelConfig {
            pyannote: OfflineSpeakerSegmentationPyannoteModelConfig {
                model: Some(pyannote_path.to_string_lossy().to_string()),
            },
            num_threads: num_cpus(),
            debug: false,
            provider: Some("cpu".to_string()),
        };

        let embedding = SpeakerEmbeddingExtractorConfig {
            model: Some(eres2net_path.to_string_lossy().to_string()),
            num_threads: num_cpus(),
            debug: false,
            provider: Some("cpu".to_string()),
        };

        // num_clusters = 0 => auto-detect speakers via clustering threshold.
        let clustering = FastClusteringConfig {
            num_clusters: 0,
            threshold: 0.5,
        };

        let config = OfflineSpeakerDiarizationConfig {
            segmentation,
            embedding,
            clustering,
            min_duration_on: 0.3,
            min_duration_off: 0.5,
        };

        let diarizer = OfflineSpeakerDiarization::create(&config)
            .ok_or_else(|| "Failed to create OfflineSpeakerDiarization".to_string())?;

        *self.diarizer.write().unwrap() = Some(diarizer);

        let elapsed = start.elapsed();
        info!(
            "[Diarization] Models loaded in {:.2}s",
            elapsed.as_secs_f64()
        );
        Ok(())
    }

    /// Run diarization on a complete waveform (16kHz mono f32 samples).
    /// Returns speaker segments sorted by start time.
    pub fn diarize(&self, samples: &[f32]) -> Result<Vec<SpeakerSegment>, String> {
        let guard = self.diarizer.read().unwrap();
        let diarizer = guard
            .as_ref()
            .ok_or_else(|| "Diarizer not loaded. Call load() first.".to_string())?;

        let result = diarizer
            .process(samples)
            .ok_or_else(|| "Diarization process() returned None".to_string())?;

        let num_speakers = result.num_speakers();
        let num_segments = result.num_segments();
        info!(
            "[Diarization] Detected {} speakers, {} segments",
            num_speakers, num_segments
        );

        let segments = result
            .sort_by_start_time()
            .into_iter()
            .map(|s| SpeakerSegment {
                start: s.start,
                end: s.end,
                speaker: s.speaker,
            })
            .collect();

        Ok(segments)
    }
}

/// Get the number of CPU cores (capped at 8 for diarization workload).
fn num_cpus() -> i32 {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4);
    cores.min(8).max(1)
}
