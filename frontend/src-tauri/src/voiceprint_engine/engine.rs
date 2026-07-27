// voiceprint_engine/engine.rs
//
// 声纹嵌入提取与匹配。共用 diarization engine 的 CAM++ 模型。

use log::{info, warn};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use serde::Serialize;
use sherpa_onnx::{SpeakerEmbeddingExtractor, SpeakerEmbeddingExtractorConfig};

use crate::speaker_diarization_engine::engine::CAMPLUS_MODEL_FILE;

/// 匹配结果
#[derive(Debug, Clone, Serialize)]
pub struct VoiceprintMatch {
    pub voiceprint_id: String,
    pub name: String,
    pub similarity: f32,
}

/// 声纹引擎（共用 CAM++ 模型）
pub struct VoiceprintEngine {
    extractor: RwLock<Option<SpeakerEmbeddingExtractor>>,
    models_dir: PathBuf,
}

/// 全局单例
static ENGINE: Mutex<Option<Arc<VoiceprintEngine>>> = Mutex::new(None);

/// 设置 models_dir（在 app setup 中调用）
pub fn set_models_directory(models_dir: PathBuf) {
    let mut guard = ENGINE.lock().unwrap();
    if guard.is_none() {
        let engine = Arc::new(VoiceprintEngine::new(models_dir));
        *guard = Some(engine);
        info!("[Voiceprint] Engine created with models_dir");
    }
}

/// 获取引擎（必须先 set_models_directory）
pub fn get_engine() -> Result<Arc<VoiceprintEngine>, String> {
    let guard = ENGINE.lock().unwrap();
    guard.as_ref().cloned().ok_or_else(|| "VoiceprintEngine not initialized. Call set_models_directory first.".to_string())
}

impl VoiceprintEngine {
    pub fn new(models_dir: PathBuf) -> Self {
        Self {
            extractor: RwLock::new(None),
            models_dir,
        }
    }

    fn camplus_model_path(&self) -> PathBuf {
        self.models_dir.join("speaker-diarization").join(CAMPLUS_MODEL_FILE)
    }

    /// 懒加载 SpeakerEmbeddingExtractor
    fn ensure_extractor(&self) -> Result<(), String> {
        let mut guard = self.extractor.write().unwrap();
        if guard.is_some() {
            return Ok(());
        }
        let model_path = self.camplus_model_path();
        if !model_path.exists() {
            return Err(format!("CAM++ model not found: {}", model_path.display()));
        }
        let config = SpeakerEmbeddingExtractorConfig {
            model: Some(model_path.to_string_lossy().to_string()),
            num_threads: 4,
            debug: false,
            provider: Some("cpu".to_string()),
        };
        let extractor = SpeakerEmbeddingExtractor::create(&config)
            .ok_or_else(|| "Failed to create SpeakerEmbeddingExtractor".to_string())?;
        *guard = Some(extractor);
        info!("[Voiceprint] Extractor loaded from {}", model_path.display());
        Ok(())
    }

    /// 从音频样本提取嵌入向量并 L2 归一化
    /// samples: 16kHz mono f32 PCM
    //
    // NOTE: sherpa-onnx 的 SpeakerEmbeddingExtractor::compute() 接收 &OnlineStream，
    // 而非原始 &[f32]。因此需要先创建 stream，喂入样本，再计算嵌入。
    // 这里使用 tokio::task::block_in_place 包装阻塞的 ONNX 推理调用，
    // 要求调用方处于多线程 tokio 运行时的 worker 线程上。
    // 若后续 commands 层改用 spawn_blocking 调用本方法，应移除 block_in_place。
    pub fn extract_embedding(&self, samples: &[f32]) -> Result<Vec<f32>, String> {
        self.ensure_extractor()?;
        let guard = self.extractor.read().unwrap();
        let extractor = guard.as_ref().ok_or("Extractor not loaded")?;

        let embedding = tokio::task::block_in_place(|| -> Result<Vec<f32>, String> {
            let stream = extractor
                .create_stream()
                .ok_or_else(|| "Failed to create OnlineStream".to_string())?;
            stream.accept_waveform(16000, samples);
            stream.input_finished();
            extractor
                .compute(&stream)
                .ok_or_else(|| "SpeakerEmbeddingExtractor.compute() returned None".to_string())
        })?;

        // L2 归一化
        let norm: f32 = embedding.iter().map(|v| v * v).sum::<f32>().sqrt();
        let normalized: Vec<f32> = if norm > 0.0 {
            embedding.iter().map(|v| v / norm).collect()
        } else {
            warn!("[Voiceprint] Zero-norm embedding detected");
            embedding
        };
        Ok(normalized)
    }

    /// 序列化嵌入为字节（小端 f32）
    pub fn serialize_embedding(embedding: &[f32]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(embedding.len() * 4);
        for &v in embedding {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes
    }

    /// 反序列化嵌入
    pub fn deserialize_embedding(bytes: &[u8]) -> Vec<f32> {
        bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    }

    /// 计算两个 L2 归一化向量的余弦相似度（即点积）
    pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() {
            warn!("[Voiceprint] Embedding dim mismatch: {} vs {}", a.len(), b.len());
            return 0.0;
        }
        a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
    }

    /// 将输入嵌入与所有已注册声纹比对，返回最佳匹配
    /// voiceprints: [(id, name, embedding)]
    /// threshold: 默认 0.6
    pub fn match_against(
        embedding: &[f32],
        voiceprints: &[(String, String, Vec<f32>)],
        threshold: f32,
    ) -> Option<VoiceprintMatch> {
        let mut best: Option<VoiceprintMatch> = None;
        let mut best_sim = threshold;
        for (id, name, vp_emb) in voiceprints {
            let sim = Self::cosine_similarity(embedding, vp_emb);
            if sim > best_sim {
                best_sim = sim;
                best = Some(VoiceprintMatch {
                    voiceprint_id: id.clone(),
                    name: name.clone(),
                    similarity: sim,
                });
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        assert!((VoiceprintEngine::cosine_similarity(&a, &a) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_cosine_similarity_orthogonal_vectors() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!(VoiceprintEngine::cosine_similarity(&a, &b).abs() < 1e-5);
    }

    #[test]
    fn test_match_against_below_threshold_returns_none() {
        let embedding = vec![1.0, 0.0];
        let voiceprints = vec![
            ("id1".to_string(), "张三".to_string(), vec![0.0, 1.0]),
        ];
        assert!(VoiceprintEngine::match_against(&embedding, &voiceprints, 0.6).is_none());
    }

    #[test]
    fn test_match_against_picks_highest_similarity() {
        let embedding = vec![1.0, 0.0];
        let voiceprints = vec![
            ("id1".to_string(), "张三".to_string(), vec![0.9, 0.43589]),
            ("id2".to_string(), "李四".to_string(), vec![1.0, 0.0]),
        ];
        let result = VoiceprintEngine::match_against(&embedding, &voiceprints, 0.6).unwrap();
        assert_eq!(result.voiceprint_id, "id2");
        assert_eq!(result.name, "李四");
    }

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        let original = vec![1.5, -2.3, 0.0, 3.14];
        let bytes = VoiceprintEngine::serialize_embedding(&original);
        let restored = VoiceprintEngine::deserialize_embedding(&bytes);
        assert_eq!(original.len(), restored.len());
        for (a, b) in original.iter().zip(restored.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }
}
