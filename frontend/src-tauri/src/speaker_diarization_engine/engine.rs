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
}
