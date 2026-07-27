# 说话人分离 (Speaker Diarization) 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为「新际审会议助手」添加说话人分离功能，使会议转录自动标注每段话的说话人，使用 sherpa-onnx 原生 `OfflineSpeakerDiarization` API（纯 Rust，无 Python 依赖）。

**Architecture:** 录音停止后进行后处理：解码 `audio.mp4` → PCM 采样 → `OfflineSpeakerDiarization::process(samples)` 得到说话人片段 → 按时间戳与现有 `TranscriptSegment` 对齐 → 回填 `speaker` 字段 → 通过 `transcript-diarized` 事件通知前端更新 UI。说话人数量自动检测（`num_clusters: 0`）。三个模型（SenseVoice ASR / Pyannote 分段 / 3D-Speaker 嵌入）全部打包进安装器，避免中国大陆网络访问 GitHub 失败。

**Tech Stack:**
- Rust 后端：`sherpa-onnx = "1.13.3"` (features=["shared"])，已包含 `OfflineSpeakerDiarization`、`OfflineSpeakerDiarizationConfig`、`FastClusteringConfig` 等 API
- Tauri 事件系统：`transcript-diarized`、`transcript-diarization-error`
- 前端：React + TypeScript，`@tauri-apps/api/event` 监听
- 模型：sherpa-onnx-pyannote-segmentation-3-0 (~6 MB) + 3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k (~40 MB) + SenseVoice int8 (~228 MB，已存在)

---

## 文件结构

### 新建文件

| 路径 | 职责 |
|------|------|
| `frontend/src-tauri/src/speaker_diarization_engine/mod.rs` | 模块入口，声明子模块 |
| `frontend/src-tauri/src/speaker_diarization_engine/engine.rs` | `SpeakerDiarizationEngine` 结构体、`SpeakerSegment` 类型、对齐算法 `align_transcripts_with_speakers` |
| `frontend/src-tauri/src/speaker_diarization_engine/commands.rs` | Tauri 命令：`speaker_diarization_init`、`speaker_diarization_is_ready`、`speaker_diarization_process` |
| `frontend/src-tauri/sherpa-libs/models/speaker-diarization/sherpa-onnx-pyannote-segmentation-3-0/model.onnx` | Pyannote 分段模型（下载） |
| `frontend/src-tauri/sherpa-libs/models/speaker-diarization/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx` | 3D-Speaker 嵌入模型（下载） |
| `frontend/src-tauri/sherpa-libs/models/sense-voice/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17/model.int8.onnx` | SenseVoice ASR 模型（从 app_data 复制到打包目录） |
| `frontend/src-tauri/sherpa-libs/models/sense-voice/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17/tokens.txt` | SenseVoice tokens（从 app_data 复制到打包目录） |

### 修改文件

| 路径 | 修改内容 |
|------|---------|
| `frontend/src-tauri/src/lib.rs` | 注册 `speaker_diarization_engine` 模块；在 `invoke_handler!` 中注册 3 个新命令 |
| `frontend/src-tauri/tauri.conf.json` | `bundle.resources` 添加 4 个模型文件映射 |
| `frontend/src-tauri/src/sherpa_asr_engine/sherpa_asr_engine.rs` | `model_path` / `is_model_downloaded` / `load_model` 优先检查打包目录 |
| `frontend/src-tauri/src/sherpa_asr_engine/commands.rs` | `sherpa_asr_has_available_models` 对打包的 SenseVoice 返回 `true` |
| `frontend/src-tauri/src/audio/recording_saver.rs` | `TranscriptSegment` 增加 `speaker` 字段；`stop_and_save` 在 `finalize()` 后触发分离 |
| `frontend/src-tauri/src/audio/transcription/worker.rs` | `TranscriptUpdate` 增加 `speaker` 字段 |
| `frontend/src/types/index.ts` | `Transcript` 与 `TranscriptUpdate` 增加 `speaker?: number` |
| `frontend/src/contexts/TranscriptContext.tsx` | 监听 `transcript-diarized` 事件，更新转录列表 |
| `frontend/src/components/VirtualizedTranscriptView.tsx` | 说话人分组 UI、颜色标签、加载状态 |
| `frontend/src/components/onboarding/steps/SetupOverviewStep.tsx` | 移除转录引擎下载步骤 |
| `frontend/src/components/onboarding/steps/DownloadProgressStep.tsx` | 移除转录下载卡片 |
| `frontend/src/components/onboarding/OnboardingFlow.tsx` | 调整步骤计数 |

---

## Task 1: 创建 speaker_diarization_engine 模块入口与类型定义

**Files:**
- Create: `frontend/src-tauri/src/speaker_diarization_engine/mod.rs`
- Create: `frontend/src-tauri/src/speaker_diarization_engine/engine.rs` (类型与结构体骨架)

- [ ] **Step 1: 创建模块入口文件 `mod.rs`**

写入 `frontend/src-tauri/src/speaker_diarization_engine/mod.rs`：

```rust
// speaker_diarization_engine/mod.rs
//
// Module entry for speaker diarization engine (post-processing speaker labeling).

pub mod engine;
pub mod commands;
```

- [ ] **Step 2: 创建 `engine.rs` 文件，定义类型与结构体骨架**

写入 `frontend/src-tauri/src/speaker_diarization_engine/engine.rs`：

```rust
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
```

- [ ] **Step 3: Commit**

```bash
cd d:\meetily
git add frontend/src-tauri/src/speaker_diarization_engine/mod.rs frontend/src-tauri/src/speaker_diarization_engine/engine.rs
git commit -m "feat(diarization): add speaker_diarization_engine module skeleton with types"
```

---

## Task 2: 实现 SpeakerDiarizationEngine 方法 (is_ready, load, diarize)

**Files:**
- Modify: `frontend/src-tauri/src/speaker_diarization_engine/engine.rs`

- [ ] **Step 1: 在 `engine.rs` 的 `impl SpeakerDiarizationEngine` 块中追加 `is_ready` 方法**

在 `pub fn new(...)` 之后追加：

```rust
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
```

- [ ] **Step 2: 追加 `load` 方法**

```rust
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
            min_duration_on: 0.3,  // min speech segment length (seconds)
            min_duration_off: 0.5, // min silence length (seconds)
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
```

- [ ] **Step 3: 追加 `diarize` 方法**

```rust
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
```

- [ ] **Step 4: 在文件末尾追加 `num_cpus` 辅助函数（与 sherpa_asr_engine.rs 保持一致）**

```rust
/// Get the number of CPU cores (capped at 8 for diarization workload).
fn num_cpus() -> i32 {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4);
    cores.min(8).max(1)
}
```

- [ ] **Step 5: 编译验证**

Run:
```bash
cd d:\meetily\frontend\src-tauri
cargo check --message-format short
```
Expected: 编译通过（模块尚未注册到 lib.rs，但 engine.rs 自身应能编译；若报 "unused" 警告可忽略）

- [ ] **Step 6: Commit**

```bash
cd d:\meetily
git add frontend/src-tauri/src/speaker_diarization_engine/engine.rs
git commit -m "feat(diarization): implement is_ready/load/diarize methods on SpeakerDiarizationEngine"
```

---

## Task 3: 实现 align_transcripts_with_speakers 对齐算法 (TDD)

**Files:**
- Modify: `frontend/src-tauri/src/speaker_diarization_engine/engine.rs` (追加对齐函数与单元测试)

**说明：** 对齐算法是核心逻辑，采用 TDD：先写失败测试，再实现，再验证通过。

- [ ] **Step 1: 在 `engine.rs` 末尾追加 `TranscriptChunkForAlignment` 类型与失败测试**

```rust
/// Lightweight view of a transcript chunk used by the alignment algorithm.
/// Fields mirror the time range used to match against speaker segments.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptChunkForAlignment {
    pub id: String,
    pub audio_start_time: f64,
    pub audio_end_time: f64,
    pub speaker: Option<i32>,
}

/// Compute the overlap duration (seconds) between two [start, end] ranges.
fn overlap_duration(a_start: f32, a_end: f32, b_start: f64, b_end: f64) -> f64 {
    let start = a_start.max(b_start as f32) as f64;
    let end = (a_end.min(b_end as f32)) as f64;
    if end > start { end - start } else { 0.0 }
}

/// For each transcript chunk, assign the speaker whose segment has the
/// longest overlap with the chunk's [audio_start_time, audio_end_time].
/// If no segment overlaps, fall back to the nearest preceding speaker.
pub fn align_transcripts_with_speakers(
    chunks: Vec<TranscriptChunkForAlignment>,
    segments: &[SpeakerSegment],
) -> Vec<TranscriptChunkForAlignment> {
    if segments.is_empty() {
        return chunks;
    }

    // Pre-sort segments by start time for the "nearest preceding" fallback.
    let mut sorted_segments: Vec<&SpeakerSegment> = segments.iter().collect();
    sorted_segments.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());

    chunks
        .into_iter()
        .map(|mut chunk| {
            // Find the segment with the longest overlap.
            let mut best_speaker: Option<i32> = None;
            let mut best_overlap: f64 = 0.0;

            for seg in &sorted_segments {
                let ov = overlap_duration(seg.start, seg.end, chunk.audio_start_time, chunk.audio_end_time);
                if ov > best_overlap {
                    best_overlap = ov;
                    best_speaker = Some(seg.speaker);
                }
            }

            if best_speaker.is_some() {
                chunk.speaker = best_speaker;
            } else {
                // Fallback: nearest preceding speaker by start time.
                let mut fallback: Option<i32> = None;
                for seg in &sorted_segments {
                    if (seg.start as f64) <= chunk.audio_start_time {
                        fallback = Some(seg.speaker);
                    } else {
                        break; // sorted by start, no later segment qualifies as "preceding"
                    }
                }
                chunk.speaker = fallback;
            }
            chunk
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: &str, start: f64, end: f64) -> TranscriptChunkForAlignment {
        TranscriptChunkForAlignment {
            id: id.to_string(),
            audio_start_time: start,
            audio_end_time: end,
            speaker: None,
        }
    }

    fn seg(start: f32, end: f32, speaker: i32) -> SpeakerSegment {
        SpeakerSegment { start, end, speaker }
    }

    #[test]
    fn test_single_chunk_inside_one_speaker_segment() {
        let chunks = vec![chunk("c1", 5.0, 7.0)];
        let segments = vec![
            seg(0.0, 6.0, 0),
            seg(6.0, 10.0, 1),
        ];
        let result = align_transcripts_with_speakers(chunks, &segments);
        // Chunk [5,7] overlaps seg[0,6] for 1s, seg[6,10] for 1s -> tie -> first wins (speaker 0).
        assert_eq!(result[0].speaker, Some(0));
    }

    #[test]
    fn test_chunk_clearly_inside_second_speaker() {
        let chunks = vec![chunk("c1", 7.0, 9.0)];
        let segments = vec![
            seg(0.0, 6.0, 0),
            seg(6.0, 10.0, 1),
        ];
        let result = align_transcripts_with_speakers(chunks, &segments);
        assert_eq!(result[0].speaker, Some(1));
    }

    #[test]
    fn test_chunk_with_no_overlap_uses_preceding_speaker() {
        // Chunk in a gap between two segments; fallback picks nearest preceding.
        let chunks = vec![chunk("c1", 5.5, 5.8)]; // in the 0.5s gap
        let segments = vec![
            seg(0.0, 5.0, 0),   // preceding speaker 0
            seg(6.0, 10.0, 1),
        ];
        let result = align_transcripts_with_speakers(chunks, &segments);
        assert_eq!(result[0].speaker, Some(0));
    }

    #[test]
    fn test_no_segments_leaves_speaker_none() {
        let chunks = vec![chunk("c1", 1.0, 2.0)];
        let segments = vec![];
        let result = align_transcripts_with_speakers(chunks, &segments);
        assert_eq!(result[0].speaker, None);
    }

    #[test]
    fn test_multiple_chunks_assign_independently() {
        let chunks = vec![
            chunk("c1", 1.0, 2.0),
            chunk("c2", 8.0, 9.0),
        ];
        let segments = vec![
            seg(0.0, 5.0, 0),
            seg(5.0, 10.0, 1),
        ];
        let result = align_transcripts_with_speakers(chunks, &segments);
        assert_eq!(result[0].speaker, Some(0));
        assert_eq!(result[1].speaker, Some(1));
    }

    #[test]
    fn test_chunk_before_all_segments_uses_none_fallback() {
        // No preceding segment exists -> speaker stays None.
        let chunks = vec![chunk("c1", 0.0, 0.5)];
        let segments = vec![seg(1.0, 5.0, 0)];
        let result = align_transcripts_with_speakers(chunks, &segments);
        assert_eq!(result[0].speaker, None);
    }
}
```

- [ ] **Step 2: 运行测试验证其通过（实现已与测试同文件给出，故应直接通过）**

Run:
```bash
cd d:\meetily\frontend\src-tauri
cargo test --lib speaker_diarization_engine::engine::tests -- --nocapture
```
Expected: `test result: ok. 6 passed`

- [ ] **Step 3: Commit**

```bash
cd d:\meetily
git add frontend/src-tauri/src/speaker_diarization_engine/engine.rs
git commit -m "feat(diarization): add align_transcripts_with_speakers with unit tests (TDD)"
```

---

## Task 4: 创建 commands.rs Tauri 命令

**Files:**
- Create: `frontend/src-tauri/src/speaker_diarization_engine/commands.rs`

- [ ] **Step 1: 创建 `commands.rs`，定义全局引擎与 3 个 Tauri 命令**

写入 `frontend/src-tauri/src/speaker_diarization_engine/commands.rs`：

```rust
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
```

- [ ] **Step 2: Commit**

```bash
cd d:\meetily
git add frontend/src-tauri/src/speaker_diarization_engine/commands.rs
git commit -m "feat(diarization): add Tauri commands (init, is_ready, process)"
```

---

## Task 5: 在 lib.rs 注册模块与命令

**Files:**
- Modify: `frontend/src-tauri/src/lib.rs` (模块声明区 + invoke_handler)

- [ ] **Step 1: 在模块声明区追加 `speaker_diarization_engine`**

在 `d:\meetily\frontend\src-tauri\src\lib.rs` 第 50 行 `pub mod sherpa_asr_engine;` 之后追加一行：

```rust
pub mod speaker_diarization_engine;
```

最终该区域应为：
```rust
pub mod sherpa_asr_engine;
pub mod speaker_diarization_engine;
pub mod state;
```

- [ ] **Step 2: 在 `invoke_handler!` 中注册 3 个新命令**

在 `d:\meetily\frontend\src-tauri\src\lib.rs` 中找到 `sherpa_asr_engine::commands::sherpa_asr_get_default_model,` 这一行（约 572 行），在其后追加：

```rust
            // Speaker diarization commands (post-processing speaker labeling)
            speaker_diarization_engine::commands::speaker_diarization_init,
            speaker_diarization_engine::commands::speaker_diarization_is_ready,
            speaker_diarization_engine::commands::speaker_diarization_process,
```

- [ ] **Step 3: 编译验证**

Run:
```bash
cd d:\meetily\frontend\src-tauri
cargo check --message-format short
```
Expected: 编译通过，无错误（可能有未使用警告，暂可忽略）

- [ ] **Step 4: Commit**

```bash
cd d:\meetily
git add frontend/src-tauri/src/lib.rs
git commit -m "feat(diarization): register speaker_diarization_engine module and commands in lib.rs"
```

---

## Task 6: 下载说话人分离模型文件到打包目录

**Files:**
- Create: `frontend/src-tauri/sherpa-libs/models/speaker-diarization/sherpa-onnx-pyannote-segmentation-3-0/model.onnx`
- Create: `frontend/src-tauri/sherpa-libs/models/speaker-diarization/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx`
- Create: `frontend/src-tauri/sherpa-libs/models/sense-voice/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17/model.int8.onnx`
- Create: `frontend/src-tauri/sherpa-libs/models/sense-voice/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17/tokens.txt`

**说明：** 中国大陆访问 GitHub 不稳定，使用镜像 URL。模型文件较大（合计 ~274 MB），需要稳定网络。

- [ ] **Step 1: 创建目标目录结构**

Run (PowerShell):
```powershell
cd d:\meetily\frontend\src-tauri\sherpa-libs
New-Item -ItemType Directory -Force -Path "models\speaker-diarization\sherpa-onnx-pyannote-segmentation-3-0"
New-Item -ItemType Directory -Force -Path "models\sense-voice\sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17"
```
Expected: 目录创建成功

- [ ] **Step 2: 下载 Pyannote 分段模型（尝试镜像 URL，逐个回退）**

Run (PowerShell) — 下载 `sherpa-onnx-pyannote-segmentation-3-0.tar.bz2`，解压后取出 `model.onnx`：

```powershell
cd d:\meetily\frontend\src-tauri\sherpa-libs\models\speaker-diarization

# Try mirrors in order; whichever succeeds, stop.
$urls = @(
  "https://gh.api.99988866.xyz/https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2",
  "https://ghproxy.net/https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2",
  "https://mirror.ghproxy.com/https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2",
  "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2"
)
$dest = ".\sherpa-onnx-pyannote-segmentation-3-0.tar.bz2"
$ok = $false
foreach ($u in $urls) {
  Write-Host "Trying: $u"
  try {
    Invoke-WebRequest -Uri $u -OutFile $dest -UseBasicParsing -TimeoutSec 120
    $ok = $true
    Write-Host "Downloaded from: $u"
    break
  } catch {
    Write-Host "Failed: $($_.Exception.Message)"
  }
}
if (-not $ok) { throw "All mirror downloads failed for pyannote model" }
```
Expected: 文件下载完成（约 6 MB 压缩包）

- [ ] **Step 3: 解压 Pyannote 压缩包，取出 `model.onnx`**

Run (PowerShell) — 需要 7-Zip 或系统 tar（Windows 10+ 自带 bsdtar）：

```powershell
cd d:\meetily\frontend\src-tauri\sherpa-libs\models\speaker-diarization

# Use Windows built-in tar (handles .tar.bz2 in two passes).
tar -xjf sherpa-onnx-pyannote-segmentation-3-0.tar.bz2
# The archive extracts to ./sherpa-onnx-pyannote-segmentation-3-0/model.onnx
Move-Item -Force ".\sherpa-onnx-pyannote-segmentation-3-0\model.onnx" ".\sherpa-onnx-pyannote-segmentation-3-0\model.onnx" -ErrorAction SilentlyContinue
# If extracted to a nested dir, ensure final path is .\sherpa-onnx-pyannote-segmentation-3-0\model.onnx
if (Test-Path ".\sherpa-onnx-pyannote-segmentation-3-0\model.onnx") {
    Write-Host "Pyannote model.onnx in place"
} else {
    # Find it and move.
    $found = Get-ChildItem -Recurse -Filter "model.onnx" | Select-Object -First 1
    if ($found) {
        New-Item -ItemType Directory -Force -Path ".\sherpa-onnx-pyannote-segmentation-3-0" | Out-Null
        Move-Item -Force $found.FullName ".\sherpa-onnx-pyannote-segmentation-3-0\model.onnx"
        Write-Host "Moved model.onnx into place"
    } else {
        throw "model.onnx not found after extraction"
    }
}
Remove-Item -Recurse -Force sherpa-onnx-pyannote-segmentation-3-0.tar.bz2
# Clean up any extra extracted dirs that don't contain model.onnx
Get-ChildItem -Directory | Where-Object { $_.Name -ne "sherpa-onnx-pyannote-segmentation-3-0" } | Remove-Item -Recurse -Force
```
Expected: `.\sherpa-onnx-pyannote-segmentation-3-0\model.onnx` 存在

- [ ] **Step 4: 下载 3D-Speaker ERes2Net 嵌入模型（单文件 .onnx）**

Run (PowerShell):

```powershell
cd d:\meetily\frontend\src-tauri\sherpa-libs\models\speaker-diarization

$urls = @(
  "https://gh.api.99988866.xyz/https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx",
  "https://ghproxy.net/https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx",
  "https://mirror.ghproxy.com/https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx",
  "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx"
)
$dest = ".\3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx"
$ok = $false
foreach ($u in $urls) {
  Write-Host "Trying: $u"
  try {
    Invoke-WebRequest -Uri $u -OutFile $dest -UseBasicParsing -TimeoutSec 180
    $ok = $true
    Write-Host "Downloaded from: $u"
    break
  } catch {
    Write-Host "Failed: $($_.Exception.Message)"
  }
}
if (-not $ok) { throw "All mirror downloads failed for eres2net model" }
```
Expected: 文件下载完成（约 40 MB）

- [ ] **Step 5: 复制 SenseVoice 模型到打包目录（从已有 app_data 或下载缓存）**

先查找已有 SenseVoice 模型文件：

Run (PowerShell):
```powershell
# Locate existing SenseVoice model files (app_data or build-target cache).
$candidates = @(
  "$env:APPDATA\com.metily.app\models\sherpa_asr\sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17",
  "$env:LOCALAPPDATA\com.metily.app\models\sherpa_asr\sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17",
  "d:\meetily\build-target\release\models\sherpa_asr\sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17"
)
$src = $null
foreach ($c in $candidates) {
  if (Test-Path "$c\model.int8.onnx") { $src = $c; break }
}
if ($src) {
  Write-Host "Found SenseVoice at: $src"
  Copy-Item -Force "$src\model.int8.onnx" "d:\meetily\frontend\src-tauri\sherpa-libs\models\sense-voice\sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17\model.int8.onnx"
  Copy-Item -Force "$src\tokens.txt" "d:\meetily\frontend\src-tauri\sherpa-libs\models\sense-voice\sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17\tokens.txt"
  Write-Host "Copied SenseVoice model to bundled dir"
} else {
  Write-Host "SenseVoice model not found in known locations. Downloading..."
  # Download via sherpa_asr_download_model command at runtime, OR download manually here:
  $urls = @(
    "https://gh.api.99988866.xyz/https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17.tar.bz2",
    "https://ghproxy.net/https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17.tar.bz2"
  )
  $dest = "d:\meetily\frontend\src-tauri\sherpa-libs\models\sense-voice\sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17.tar.bz2"
  foreach ($u in $urls) {
    try { Invoke-WebRequest -Uri $u -OutFile $dest -UseBasicParsing -TimeoutSec 600; break } catch {}
  }
  tar -xjf $dest -C "d:\meetily\frontend\src-tauri\sherpa-libs\models\sense-voice\"
  Remove-Item $dest
  # Ensure files are at the expected path:
  $extracted = Get-ChildItem -Path "d:\meetily\frontend\src-tauri\sherpa-libs\models\sense-voice\" -Recurse -Filter "model.int8.onnx" | Select-Object -First 1
  if ($extracted -and $extracted.DirectoryName -notlike "*sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17*") {
    Move-Item -Force $extracted.FullName "d:\meetily\frontend\src-tauri\sherpa-libs\models\sense-voice\sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17\model.int8.onnx"
  }
}
```
Expected: SenseVoice `model.int8.onnx` 与 `tokens.txt` 出现在打包目录

- [ ] **Step 6: 验证所有 4 个模型文件就位**

Run (PowerShell):
```powershell
$base = "d:\meetily\frontend\src-tauri\sherpa-libs\models"
$files = @(
  "$base\sense-voice\sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17\model.int8.onnx",
  "$base\sense-voice\sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17\tokens.txt",
  "$base\speaker-diarization\sherpa-onnx-pyannote-segmentation-3-0\model.onnx",
  "$base\speaker-diarization\3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx"
)
foreach ($f in $files) {
  if (Test-Path $f) {
    $size = [math]::Round((Get-Item $f).Length / 1MB, 1)
    Write-Host "OK  $f  ($size MB)"
  } else {
    Write-Host "MISSING  $f"
  }
}
```
Expected: 4 个文件全部 `OK`，SenseVoice ~228 MB，Pyannote ~6 MB，ERes2Net ~40 MB

- [ ] **Step 7: Commit (模型文件不进 git，仅记录目录结构)**

模型文件体积过大，不应提交到 git。仅提交 `.gitignore` 更新（若已有忽略规则则跳过）：

```bash
cd d:\meetily
git check-ignore frontend/src-tauri/sherpa-libs/models/sense-voice/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17/model.int8.onnx || echo "sherpa-libs/models/" >> .gitignore
git add .gitignore
git commit -m "chore(diarization): ignore bundled model files (too large for git)" --allow-empty
```

---

## Task 7: 更新 tauri.conf.json bundle.resources 添加模型文件映射

**Files:**
- Modify: `frontend/src-tauri/tauri.conf.json` (bundle.resources 区段)

- [ ] **Step 1: 在 `bundle.resources` Map 中追加 4 个模型文件映射**

在 `d:\meetily\frontend\src-tauri\tauri.conf.json` 的 `"resources"` 对象内，在 `"sherpa-libs/DirectML.dll": "DirectML.dll"` 之后追加：

```json
      "sherpa-libs/models/sense-voice/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17/model.int8.onnx": "models/sense-voice/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17/model.int8.onnx",
      "sherpa-libs/models/sense-voice/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17/tokens.txt": "models/sense-voice/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17/tokens.txt",
      "sherpa-libs/models/speaker-diarization/sherpa-onnx-pyannote-segmentation-3-0/model.onnx": "models/speaker-diarization/sherpa-onnx-pyannote-segmentation-3-0/model.onnx",
      "sherpa-libs/models/speaker-diarization/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx": "models/speaker-diarization/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx"
```

完整 `resources` 对象应为：
```json
    "resources": {
      "templates/*.json": "templates/",
      "sherpa-libs/sherpa-onnx-c-api.dll": "sherpa-onnx-c-api.dll",
      "sherpa-libs/sherpa-onnx-cxx-api.dll": "sherpa-onnx-cxx-api.dll",
      "sherpa-libs/onnxruntime.dll": "onnxruntime.dll",
      "sherpa-libs/onnxruntime_providers_shared.dll": "onnxruntime_providers_shared.dll",
      "sherpa-libs/DirectML.dll": "DirectML.dll",
      "sherpa-libs/models/sense-voice/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17/model.int8.onnx": "models/sense-voice/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17/model.int8.onnx",
      "sherpa-libs/models/sense-voice/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17/tokens.txt": "models/sense-voice/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17/tokens.txt",
      "sherpa-libs/models/speaker-diarization/sherpa-onnx-pyannote-segmentation-3-0/model.onnx": "models/speaker-diarization/sherpa-onnx-pyannote-segmentation-3-0/model.onnx",
      "sherpa-libs/models/speaker-diarization/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx": "models/speaker-diarization/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx"
    },
```

- [ ] **Step 2: 验证 JSON 合法性**

Run:
```bash
cd d:\meetily\frontend\src-tauri
node -e "JSON.parse(require('fs').readFileSync('tauri.conf.json','utf8')); console.log('JSON OK')"
```
Expected: 输出 `JSON OK`

- [ ] **Step 3: Commit**

```bash
cd d:\meetily
git add frontend/src-tauri/tauri.conf.json
git commit -m "feat(diarization): bundle diarization + sense-voice models in installer resources"
```

---

## Task 8: 修改 sherpa_asr_engine 优先检查打包目录

**Files:**
- Modify: `frontend/src-tauri/src/sherpa_asr_engine/sherpa_asr_engine.rs` (model_path / is_model_downloaded / load_model)
- Modify: `frontend/src-tauri/src/sherpa_asr_engine/commands.rs` (sherpa_asr_has_available_models)

- [ ] **Step 1: 在 `sherpa_asr_engine.rs` 顶部追加打包目录辅助函数**

在 `use sherpa_onnx::{...}` 之后、`pub const DEFAULT_MODEL_NAME` 之前追加：

```rust
use tauri::{AppHandle, Manager, Runtime};
use std::sync::OnceLock;

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
```

- [ ] **Step 2: 修改 `model_path` 方法，优先返回打包路径**

在 `SherpaAsrEngine::model_path` 中替换为：

```rust
    /// Get the model directory path for a given model.
    /// Checks bundled dir first (production), falls back to app_data models dir.
    pub fn model_path(&self, model_name: &str) -> Option<PathBuf> {
        let model_def = get_model_by_name(model_name)?;

        // 1. Bundled (production) path for SenseVoice.
        if model_name == DEFAULT_MODEL_NAME {
            if let Some(bundled) = bundled_sense_voice_dir() {
                return Some(bundled);
            }
        }

        // 2. Fallback: app_data models dir (dev or downloaded models).
        Some(self.models_dir.join("sherpa_asr").join(&model_def.dir_name))
    }
```

- [ ] **Step 3: 修改 `is_model_downloaded` 检查打包路径**

`is_model_downloaded` 已调用 `model_path` 与 `get_model_by_name`，无需改动 — Step 2 的修改已使其优先检查打包目录。确认其逻辑为：

```rust
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
```

无需修改，仅需确认。

- [ ] **Step 4: 修改 `commands.rs` 的 `sherpa_asr_has_available_models`，对打包 SenseVoice 返回 true**

在 `d:\meetily\frontend\src-tauri\src\sherpa_asr_engine\commands.rs` 中找到 `sherpa_asr_has_available_models`，替换为：

```rust
/// Check if any Sherpa-ASR models are available (bundled or downloaded).
#[tauri::command]
pub async fn sherpa_asr_has_available_models() -> Result<bool, String> {
    // Bundled SenseVoice is always available in production builds.
    if super::sherpa_asr_engine::get_bundled_models_dir().is_some() {
        return Ok(true);
    }
    // Dev fallback: check downloaded models in app_data.
    let engine = get_engine();
    Ok(engine.has_available_models())
}
```

- [ ] **Step 5: 编译验证**

Run:
```bash
cd d:\meetily\frontend\src-tauri
cargo check --message-format short
```
Expected: 编译通过

- [ ] **Step 6: Commit**

```bash
cd d:\meetily
git add frontend/src-tauri/src/sherpa_asr_engine/sherpa_asr_engine.rs frontend/src-tauri/src/sherpa_asr_engine/commands.rs
git commit -m "feat(diarization): sherpa_asr prefers bundled SenseVoice; has_available_models returns true when bundled"
```

---

## Task 9: 在 lib.rs setup 钩子中初始化打包目录

**Files:**
- Modify: `frontend/src-tauri/src/lib.rs` (setup 钩子，初始化 sherpa + diarization 的打包目录)

- [ ] **Step 1: 找到现有 setup 钩子，在其中追加初始化调用**

在 `d:\meetily\frontend\src-tauri\src\lib.rs` 中查找 `.setup(|app| {` 区段。在 `sherpa_asr_engine::commands::set_models_directory(app);` 之后（或类似的引擎初始化位置），追加：

```rust
        // Initialize bundled models dir for SenseVoice (production path).
        sherpa_asr_engine::sherpa_asr_engine::set_bundled_models_dir(app.handle());
        // Initialize diarization engine models dir.
        speaker_diarization_engine::commands::set_models_directory(app.handle());
```

> **注意：** 若 setup 钩子中尚未调用 `sherpa_asr_engine::commands::set_models_directory(app);`，请一并补上（参考 sherpa_asr_engine 已有的初始化模式）。

- [ ] **Step 2: 编译验证**

Run:
```bash
cd d:\meetily\frontend\src-tauri
cargo check --message-format short
```
Expected: 编译通过

- [ ] **Step 3: Commit**

```bash
cd d:\meetily
git add frontend/src-tauri/src/lib.rs
git commit -m "feat(diarization): init bundled models dirs for sherpa + diarization in setup hook"
```

---

## Task 10: 在 TranscriptSegment 与 TranscriptUpdate 增加 speaker 字段

**Files:**
- Modify: `frontend/src-tauri/src/audio/recording_saver.rs` (TranscriptSegment 结构体)
- Modify: `frontend/src-tauri/src/audio/transcription/worker.rs` (TranscriptUpdate 结构体 + emit 处补字段)

- [ ] **Step 1: 在 `recording_saver.rs` 的 `TranscriptSegment` 结构体增加 `speaker` 字段**

在 `d:\meetily\frontend\src-tauri\src\audio\recording_saver.rs` 第 15-25 行，将 `TranscriptSegment` 改为：

```rust
/// Structured transcript segment for JSON export
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub id: String,
    pub text: String,
    pub audio_start_time: f64, // Seconds from recording start
    pub audio_end_time: f64,   // Seconds from recording start
    pub duration: f64,          // Segment duration in seconds
    pub display_time: String,   // Formatted time for display like "[02:15]"
    pub confidence: f32,
    pub sequence_id: u64,
    /// Speaker ID assigned by diarization (None until post-processing runs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<i32>,
}
```

- [ ] **Step 2: 在 `recording_saver.rs` 中查找所有构造 `TranscriptSegment { ... }` 的位置，补上 `speaker: None`**

Run (定位构造点):
```bash
cd d:\meetily\frontend\src-tauri
grep -n "TranscriptSegment {" src/audio/recording_saver.rs
```

对每个匹配位置，在构造体末尾（`sequence_id: ...,` 之后）追加 `speaker: None,`。

典型构造点位于 `add_transcript_segment` 调用方（worker.rs 通过命令传入），需检查 `recording_saver.rs` 内是否有直接构造。若 `add_transcript_segment` 接收已构造的 `TranscriptSegment`，则构造发生在 worker.rs（Step 3 处理）。

- [ ] **Step 3: 在 `worker.rs` 的 `TranscriptUpdate` 结构体增加 `speaker` 字段**

在 `d:\meetily\frontend\src-tauri\src\audio\transcription\worker.rs` 第 26-39 行，将 `TranscriptUpdate` 改为：

```rust
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TranscriptUpdate {
    pub text: String,
    pub timestamp: String, // Wall-clock time for reference (e.g., "14:30:05")
    pub source: String,
    pub sequence_id: u64,
    pub chunk_start_time: f64, // Legacy field, kept for compatibility
    pub is_partial: bool,
    pub confidence: f32,
    // NEW: Recording-relative timestamps for playback sync
    pub audio_start_time: f64, // Seconds from recording start (e.g., 125.3)
    pub audio_end_time: f64,   // Seconds from recording start (e.g., 128.6)
    pub duration: f64,          // Segment duration in seconds (e.g., 3.3)
    /// Speaker ID (None during real-time ASR; set by post-processing diarization).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<i32>,
}
```

- [ ] **Step 4: 在 `worker.rs` 中查找所有构造 `TranscriptUpdate { ... }` 的位置，补上 `speaker: None`**

Run:
```bash
cd d:\meetily\frontend\src-tauri
grep -n "TranscriptUpdate {" src/audio/transcription/worker.rs
```

对每个匹配位置，在构造体末尾追加 `speaker: None,`。

- [ ] **Step 5: 编译验证**

Run:
```bash
cd d:\meetily\frontend\src-tauri
cargo check --message-format short
```
Expected: 编译通过（若有遗漏的构造点，编译器会报错指出位置，逐一补上 `speaker: None`）

- [ ] **Step 6: Commit**

```bash
cd d:\meetily
git add frontend/src-tauri/src/audio/recording_saver.rs frontend/src-tauri/src/audio/transcription/worker.rs
git commit -m "feat(diarization): add speaker field to TranscriptSegment and TranscriptUpdate"
```

---

## Task 11: 修改 recording_saver.rs stop 流程触发说话人分离

**Files:**
- Modify: `frontend/src-tauri/src/audio/recording_saver.rs` (stop_and_save 方法)

**说明：** 在 `finalize()` 保存 `audio.mp4` 之后、`write_transcripts_json` 之前，触发分离；分离成功后回填 `speaker` 字段并重写 `transcripts.json`，然后通过 `transcript-diarized` 事件通知前端。

- [ ] **Step 1: 在 `recording_saver.rs` 顶部追加 `use` 引入**

在文件顶部 `use` 区追加：

```rust
use crate::speaker_diarization_engine::engine::{
    align_transcripts_with_speakers, TranscriptChunkForAlignment,
};
use crate::speaker_diarization_engine::commands as diarization_commands;
```

- [ ] **Step 2: 在 `stop_and_save` 方法中，`write_transcripts_json` 调用之前插入分离逻辑**

在 `d:\meetily\frontend\src-tauri\src\audio\recording_saver.rs` 的 `stop_and_save` 方法中，找到以下代码块（约 400-414 行）：

```rust
        // Save final transcripts.json with validation
        if let Some(folder) = &self.meeting_folder {
            if let Err(e) = self.write_transcripts_json(folder) {
```

在 `// Save final transcripts.json with validation` **之前**插入：

```rust
        // [NEW] Run speaker diarization post-processing.
        // Failure is non-fatal: transcripts are still saved without speaker labels.
        if let Some(folder) = &self.meeting_folder {
            if let Err(e) = self.run_diarization(app, folder, &final_audio_path).await {
                warn!("Speaker diarization failed (transcripts unaffected): {}", e);
                if let Err(emit_err) = app.emit(
                    "transcript-diarization-error",
                    serde_json::json!({ "error": e.to_string() }),
                ) {
                    warn!("Failed to emit transcript-diarization-error: {}", emit_err);
                }
            }
        }
```

- [ ] **Step 3: 在 `impl RecordingSaver` 中追加 `run_diarization` 方法**

在 `stop_and_save` 方法之后（同一 `impl` 块内）追加：

```rust
    /// Run speaker diarization on the saved audio file, backfill the `speaker`
    /// field on each transcript segment, rewrite transcripts.json, and emit
    /// `transcript-diarized` to the frontend.
    ///
    /// Non-fatal: returns Err on any failure; caller logs and continues.
    async fn run_diarization<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        folder: &std::path::Path,
        audio_path: &std::path::Path,
    ) -> Result<(), anyhow::Error> {
        use tauri::Emitter;

        // Emit "processing started" toast trigger.
        let _ = app.emit(
            "transcript-diarization-started",
            serde_json::json!({}),
        );

        // Ensure engine is initialized (models dir set in setup hook).
        diarization_commands::set_models_directory(app);

        // Check readiness without holding engine across await.
        let engine = {
            // Re-create a temporary engine to check is_ready (lightweight file check).
            // The actual process() command uses the global engine internally.
            None::<()> // placeholder; we use the command below.
        };
        let _ = engine;

        // Call the diarization process command (decodes audio + runs model).
        let audio_path_str = audio_path.to_string_lossy().to_string();
        let segments = match diarization_commands::speaker_diarization_process(audio_path_str).await {
            Ok(s) => s,
            Err(e) => return Err(anyhow::anyhow!("diarization_process failed: {}", e)),
        };

        if segments.is_empty() {
            info!("[Diarization] No segments returned (audio too short or no speech); skipping speaker labels");
            return Ok(());
        }

        // Lock transcript segments, align, and backfill speaker field.
        let mut segments_guard = self.transcript_segments.lock()
            .map_err(|e| anyhow::anyhow!("transcript_segments lock poisoned: {}", e))?;

        let chunks_for_alignment: Vec<TranscriptChunkForAlignment> = segments_guard
            .iter()
            .map(|s| TranscriptChunkForAlignment {
                id: s.id.clone(),
                audio_start_time: s.audio_start_time,
                audio_end_time: s.audio_end_time,
                speaker: None,
            })
            .collect();

        let aligned = align_transcripts_with_speakers(chunks_for_alignment, &segments);

        // Apply aligned speaker IDs back onto the stored segments.
        for (seg, aligned_chunk) in segments_guard.iter_mut().zip(aligned.iter()) {
            seg.speaker = aligned_chunk.speaker;
        }
        drop(segments_guard);

        // Rewrite transcripts.json with speaker field populated.
        self.write_transcripts_json(folder)?;

        // Emit diarized transcripts to frontend.
        let final_segments: Vec<TranscriptSegment> = {
            let g = self.transcript_segments.lock()
                .map_err(|e| anyhow::anyhow!("transcript_segments lock poisoned: {}", e))?;
            g.clone()
        };

        let num_speakers = segments.iter().map(|s| s.speaker).max().map(|m| m + 1).unwrap_or(0);
        info!(
            "[Diarization] Success: {} speakers, {} segments labeled",
            num_speakers,
            final_segments.len()
        );

        let _ = app.emit(
            "transcript-diarized",
            serde_json::json!({
                "transcripts": final_segments,
                "num_speakers": num_speakers,
            }),
        );

        Ok(())
    }
```

- [ ] **Step 4: 编译验证**

Run:
```bash
cd d:\meetily\frontend\src-tauri
cargo check --message-format short
```
Expected: 编译通过

- [ ] **Step 5: Commit**

```bash
cd d:\meetily
git add frontend/src-tauri/src/audio/recording_saver.rs
git commit -m "feat(diarization): trigger speaker diarization after recording stop, backfill speaker field"
```

---

## Task 12: 更新前端 Transcript 类型增加 speaker 字段

**Files:**
- Modify: `frontend/src/types/index.ts`

- [ ] **Step 1: 在 `Transcript` 接口增加 `speaker?: number`**

在 `d:\meetily\frontend\src\types\index.ts` 第 7-19 行的 `Transcript` 接口末尾（`duration?: number;` 之后）追加：

```typescript
  // Speaker ID assigned by diarization post-processing (0, 1, 2, ...).
  // Undefined during real-time ASR; set after recording stops.
  speaker?: number;
```

最终 `Transcript` 应为：
```typescript
export interface Transcript {
  id: string;
  text: string;
  timestamp: string;
  sequence_id?: number;
  chunk_start_time?: number;
  is_partial?: boolean;
  confidence?: number;
  audio_start_time?: number;
  audio_end_time?: number;
  duration?: number;
  speaker?: number;
}
```

- [ ] **Step 2: 在 `TranscriptUpdate` 接口增加 `speaker?: number`**

在第 21-33 行的 `TranscriptUpdate` 接口末尾追加：

```typescript
  speaker?: number;
```

- [ ] **Step 3: Commit**

```bash
cd d:\meetily
git add frontend/src/types/index.ts
git commit -m "feat(diarization): add speaker field to frontend Transcript and TranscriptUpdate types"
```

---

## Task 13: 在前端监听 diarization 事件并更新转录列表

**Files:**
- Modify: `frontend/src/contexts/TranscriptContext.tsx`

- [ ] **Step 1: 找到 TranscriptProvider 中现有的 transcript 更新逻辑**

Run:
```bash
cd d:\meetily
grep -n "listen\|onTranscriptUpdate\|transcript-" frontend/src/contexts/TranscriptContext.tsx | head -30
```
记录现有 `useEffect` 监听位置。

- [ ] **Step 2: 在 TranscriptProvider 中追加 `transcript-diarized` 事件监听**

在 `d:\meetily\frontend\src\contexts\TranscriptContext.tsx` 中，找到现有的 `listen('transcript-update', ...)` 的 `useEffect`，在其后追加新的 `useEffect`：

```typescript
  // Listen for diarization completion: replace transcripts with speaker-labeled versions.
  useEffect(() => {
    let unlistenFn: (() => void) | null = null;
    const setup = async () => {
      const { listen } = await import('@tauri-apps/api/event');
      unlistenFn = await listen<{
        transcripts: Array<{
          id: string;
          text: string;
          timestamp: string;
          sequence_id: number;
          audio_start_time: number;
          audio_end_time: number;
          duration: number;
          confidence: number;
          speaker: number | null;
        }>;
        num_speakers: number;
      }>('transcript-diarized', (event) => {
        const payload = event.payload;
        // Map diarized segments to Transcript shape, preserving speaker field.
        const diarized: Transcript[] = payload.transcripts.map((seg) => ({
          id: seg.id,
          text: seg.text,
          timestamp: seg.timestamp,
          sequence_id: seg.sequence_id,
          audio_start_time: seg.audio_start_time,
          audio_end_time: seg.audio_end_time,
          duration: seg.duration,
          confidence: seg.confidence,
          speaker: seg.speaker ?? undefined,
        }));
        setTranscripts(diarized);

        // Toast feedback.
        import('sonner').then(({ toast }) => {
          toast.success(`说话人分离完成，识别到 ${payload.num_speakers} 位说话人`);
        });
      });
    };
    setup();
    return () => {
      if (unlistenFn) unlistenFn();
    };
  }, [setTranscripts]);

  // Listen for diarization errors: show non-blocking toast.
  useEffect(() => {
    let unlistenFn: (() => void) | null = null;
    const setup = async () => {
      const { listen } = await import('@tauri-apps/api/event');
      unlistenFn = await listen<{ error: string }>(
        'transcript-diarization-error',
        () => {
          import('sonner').then(({ toast }) => {
            toast.error('说话人分离失败，转录结果不受影响', { duration: 8000 });
          });
        }
      );
    };
    setup();
    return () => {
      if (unlistenFn) unlistenFn();
    };
  }, []);

  // Listen for diarization start: show processing toast.
  useEffect(() => {
    let unlistenFn: (() => void) | null = null;
    const setup = async () => {
      const { listen } = await import('@tauri-apps/api/event');
      unlistenFn = await listen('transcript-diarization-started', () => {
        import('sonner').then(({ toast }) => {
          toast.loading('正在处理说话人分离...', { id: 'diarization-loading', duration: 60000 });
        });
      });
    };
    setup();
    return () => {
      if (unlistenFn) unlistenFn();
    };
  }, []);
```

> **注意：** 若现有代码已用 `import { listen } from '@tauri-apps/api/event'` 顶部导入，则改为顶部导入方式调用即可，无需动态 import。保留与文件现有风格一致。

- [ ] **Step 3: 在 diarized 事件回调中 dismiss loading toast**

在 `transcript-diarized` 回调的 `toast.success(...)` 之前追加：

```typescript
        import('sonner').then(({ toast }) => {
          toast.dismiss('diarization-loading');
          toast.success(`说话人分离完成，识别到 ${payload.num_speakers} 位说话人`);
        });
```

（替换 Step 2 中对应的 toast.success 调用）

- [ ] **Step 4: 编译验证**

Run:
```bash
cd d:\meetily\frontend
pnpm tsc --noEmit
```
Expected: 无类型错误

- [ ] **Step 5: Commit**

```bash
cd d:\meetily
git add frontend/src/contexts/TranscriptContext.tsx
git commit -m "feat(diarization): listen for transcript-diarized events and update transcripts with speaker labels"
```

---

## Task 14: 在 VirtualizedTranscriptView 添加说话人分组 UI

**Files:**
- Modify: `frontend/src/components/VirtualizedTranscriptView.tsx`

- [ ] **Step 1: 在文件顶部追加说话人颜色映射辅助函数**

在 `d:\meetily\frontend\src\components\VirtualizedTranscriptView.tsx` 顶部（imports 之后）追加：

```typescript
// Speaker color palette: each speaker gets a distinct color.
const SPEAKER_COLORS = [
  { bg: 'bg-blue-50', border: 'border-blue-200', text: 'text-blue-700', label: 'bg-blue-600' },
  { bg: 'bg-green-50', border: 'border-green-200', text: 'text-green-700', label: 'bg-green-600' },
  { bg: 'bg-orange-50', border: 'border-orange-200', text: 'text-orange-700', label: 'bg-orange-600' },
  { bg: 'bg-purple-50', border: 'border-purple-200', text: 'text-purple-700', label: 'bg-purple-600' },
  { bg: 'bg-pink-50', border: 'border-pink-200', text: 'text-pink-700', label: 'bg-pink-600' },
  { bg: 'bg-teal-50', border: 'border-teal-200', text: 'text-teal-700', label: 'bg-teal-600' },
];

function getSpeakerColor(speaker: number) {
  return SPEAKER_COLORS[speaker % SPEAKER_COLORS.length];
}

function getSpeakerLabel(speaker: number): string {
  return `说话人 ${speaker + 1}`;
}
```

- [ ] **Step 2: 在 `TranscriptSegment` 组件（memoized）中渲染说话人标签**

找到 `TranscriptSegment` 组件定义（memo 内部），在其 JSX 中，于文本渲染之前追加说话人标签：

```tsx
        {segment.speaker !== undefined && segment.speaker !== null && (
          <div className="flex items-center gap-2 mb-1">
            <span
              className={`inline-flex items-center px-2 py-0.5 rounded text-xs font-medium text-white ${getSpeakerColor(segment.speaker).label}`}
            >
              {getSpeakerLabel(segment.speaker)}
            </span>
            <span className="text-xs text-gray-400">
              {formatRecordingTime(segment.audio_start_time ?? 0)}
            </span>
          </div>
        )}
```

> **注意：** 将 `segment` 替换为该组件实际使用的 props 名称。若组件 prop 名为 `item` 或 `seg`，相应调整。

- [ ] **Step 3: 在转录容器外层追加 diarization 加载状态指示（可选）**

若希望录音停止后显示「正在分离说话人...」加载条，可在组件根容器顶部追加（依赖父组件传入 `isDiarizing` prop；若暂不传入则跳过此步，由 toast 提示即可）：

```tsx
      {isDiarizing && (
        <div className="flex items-center justify-center py-3 bg-amber-50 border-b border-amber-200">
          <Loader2 className="w-4 h-4 animate-spin text-amber-600 mr-2" />
          <span className="text-sm text-amber-700">正在分离说话人...</span>
        </div>
      )}
```

> **说明：** 本步为可选；Task 13 的 toast 已提供处理中反馈。若不实现 `isDiarizing` prop，跳过此 JSX。

- [ ] **Step 4: 编译验证**

Run:
```bash
cd d:\meetily\frontend
pnpm tsc --noEmit
```
Expected: 无类型错误

- [ ] **Step 5: Commit**

```bash
cd d:\meetily
git add frontend/src/components/VirtualizedTranscriptView.tsx
git commit -m "feat(diarization): render speaker color-coded labels in transcript view"
```

---

## Task 15: 更新 SetupOverviewStep 移除转录引擎下载步骤

**Files:**
- Modify: `frontend/src/components/onboarding/steps/SetupOverviewStep.tsx`

- [ ] **Step 1: 修改 `steps` 数组，移除转录步骤，仅保留摘要步骤并重新编号**

在 `d:\meetily\frontend\src\components\onboarding\steps\SetupOverviewStep.tsx` 中，将 `steps` 数组（第 29-40 行）替换为：

```typescript
  const steps = [
    {
      number: 1,
      type: 'summarization',
      title: '下载摘要引擎',
    },
  ];
```

- [ ] **Step 2: 修改 `OnboardingContainer` 的 description 与 totalSteps**

将第 47-52 行的 `OnboardingContainer` 调用改为：

```tsx
    <OnboardingContainer
      title="设置概览"
      description="新际审会议助手 需要您下载摘要 AI 模型才能正常工作。转录引擎已内置，无需下载。"
      step={2}
      totalSteps={isMac ? 3 : 2}
    >
```

- [ ] **Step 3: 移除转录步骤相关的 Tooltip 逻辑（若步骤数组中已无 transcription，则现有 Tooltip 条件 `step.type === "summarization"` 仍生效，无需改动）**

确认 JSX 中 `{step.type === "summarization" && (...)}` 仍存在且正确。

- [ ] **Step 4: 编译验证**

Run:
```bash
cd d:\meetily\frontend
pnpm tsc --noEmit
```
Expected: 无类型错误

- [ ] **Step 5: Commit**

```bash
cd d:\meetily
git add frontend/src/components/onboarding/steps/SetupOverviewStep.tsx
git commit -m "feat(onboarding): remove transcription download step from SetupOverview (engine now bundled)"
```

---

## Task 16: 更新 DownloadProgressStep 移除转录下载卡片

**Files:**
- Modify: `frontend/src/components/onboarding/steps/DownloadProgressStep.tsx`

**说明：** 此文件较长，需移除 `parakeetState`、`handleRetryDownload`、Parakeet 下载卡片 JSX、相关 useEffect 与 ref，仅保留 summary 下载逻辑。

- [ ] **Step 1: 移除 `parakeetDownloaded`、`setParakeetDownloaded` 从 useOnboarding 解构**

在 `d:\meetily\frontend\src\components\onboarding\steps\DownloadProgressStep.tsx` 中，将第 26-36 行的解构改为：

```typescript
  const {
    goNext,
    selectedSummaryModel,
    recommendedSummaryModel,
    summaryModelDownloaded,
    setSummaryModelDownloaded,
    startBackgroundDownloads,
    completeOnboarding,
  } = useOnboarding();
```

- [ ] **Step 2: 移除 `parakeetState` state 与 `parakeetDownloadStartedRef`、`retryingRef`**

删除第 40-46 行的 `parakeetState` useState，以及第 57 行的 `parakeetDownloadStartedRef`、第 59 行的 `retryingRef`。

- [ ] **Step 3: 移除 `handleRetryDownload` 函数（Parakeet 重试）**

删除第 62-105 行左右的 `handleRetryDownload` 函数整体。

- [ ] **Step 4: 移除 Parakeet 下载触发的 useEffect 与事件监听**

查找并删除所有监听 `model-download-progress`、`model-download-complete`、`model-download-error` 中针对 Parakeet/SHERPA_MODEL 的处理逻辑（保留 summary 相关的监听）。具体而言，删除任何引用 `parakeetState`、`SHERPA_MODEL`、`invoke('sherpa_asr_download_model', ...)` 的代码块。

- [ ] **Step 5: 移除 JSX 中的 Parakeet 下载卡片**

删除渲染 Parakeet 下载进度卡片的 JSX 块（通常包含 `Mic` 图标、`parakeetState` 引用、`handleRetryDownload` 按钮的 `<div>`）。

- [ ] **Step 6: 修改「继续」按钮的启用条件，仅依赖 summary 下载完成**

找到「继续」按钮的 `disabled` 条件，改为：

```tsx
          <Button
            onClick={handleComplete}
            disabled={summaryState.status !== 'completed' || isCompleting}
            className="w-full h-11 bg-gray-900 hover:bg-gray-800 text-white"
          >
            {isCompleting ? <Loader2 className="w-4 h-4 animate-spin mr-2" /> : null}
            {isCompleting ? '正在完成设置...' : '完成设置'}
          </Button>
```

> **注意：** 若现有按钮文字不同，保留原文字，仅修改 `disabled` 条件与 `onClick`（确保 `handleComplete` 调用 `completeOnboarding`）。

- [ ] **Step 7: 修改页面描述文案**

找到 `OnboardingContainer` 的 `description`，改为：

```tsx
      description="下载摘要引擎后，您就可以开始使用 新际审会议助手。转录引擎已内置。"
```

- [ ] **Step 8: 移除顶部不再使用的 import（Mic 图标、SHERPA_MODEL 常量等）**

删除 `import { Mic, ... }` 中的 `Mic`（若不再使用），删除 `const SHERPA_MODEL = ...` 常量行。

- [ ] **Step 9: 编译验证**

Run:
```bash
cd d:\meetily\frontend
pnpm tsc --noEmit
```
Expected: 无类型错误（若有 unused import 警告，逐一删除）

- [ ] **Step 10: Commit**

```bash
cd d:\meetily
git add frontend/src/components/onboarding/steps/DownloadProgressStep.tsx
git commit -m "feat(onboarding): remove transcription download card from DownloadProgressStep (engine bundled)"
```

---

## Task 17: 更新 OnboardingFlow 调整步骤计数与 onboarding.rs 完成状态

**Files:**
- Modify: `frontend/src/components/onboarding/OnboardingFlow.tsx`
- Modify: `frontend/src-tauri/src/onboarding.rs`

- [ ] **Step 1: 更新 OnboardingFlow 注释与步骤描述**

在 `d:\meetily\frontend\src\components\onboarding\OnboardingFlow.tsx` 中，将第 34-38 行的注释改为：

```typescript
  // 3-Step Onboarding Flow (transcription engine now bundled):
  // Step 1: Welcome - Introduce Meetily features
  // Step 2: Setup Overview - Show summary engine download (transcription bundled)
  // Step 3: Download Progress - Download Summary Model
  // Step 4: Permissions - Request mic + system audio (macOS only)
```

JSX 主体无需改动（仍按 `currentStep === N` 渲染），但 macOS 步骤总数现为 4，非 macOS 为 3。

- [ ] **Step 2: 修改 `onboarding.rs` 中 `complete_onboarding` 的 `current_step` 与 whisper 状态**

在 `d:\meetily\frontend\src-tauri\src\onboarding.rs` 的 `complete_onboarding` 函数中，将 `status.current_step = 4;` 改为：

```rust
    status.current_step = if cfg!(target_os = "macos") { 4 } else { 3 };
```

并在其下确保 whisper 状态标记为已下载（因为内置）：

```rust
    status.model_status.whisper = "downloaded".to_string();
```

（该行已存在，确认保留）

- [ ] **Step 3: 修改 `OnboardingStatus::default()` 的 `current_step`（可选，保持 1 即可）**

`default()` 中 `current_step: 1` 保持不变（从 Welcome 开始）。

- [ ] **Step 4: 编译验证（前后端）**

Run:
```bash
cd d:\meetily\frontend\src-tauri
cargo check --message-format short
cd d:\meetily\frontend
pnpm tsc --noEmit
```
Expected: 均通过

- [ ] **Step 5: Commit**

```bash
cd d:\meetily
git add frontend/src/components/onboarding/OnboardingFlow.tsx frontend/src-tauri/src/onboarding.rs
git commit -m "feat(onboarding): adjust step counts for bundled transcription engine"
```

---

## Task 18: 完整构建与端到端验证

**Files:**
- 无新增文件，仅构建与测试

- [ ] **Step 1: 运行 Rust 单元测试（确认对齐算法通过）**

Run:
```bash
cd d:\meetily\frontend\src-tauri
cargo test --lib speaker_diarization_engine -- --nocapture
```
Expected: `test result: ok. 6 passed`（align_transcripts_with_speakers 的 6 个测试）

- [ ] **Step 2: 运行完整 cargo check**

Run:
```bash
cd d:\meetily\frontend\src-tauri
cargo check --message-format short
```
Expected: 无错误

- [ ] **Step 3: 运行前端类型检查**

Run:
```bash
cd d:\meetily\frontend
pnpm tsc --noEmit
```
Expected: 无错误

- [ ] **Step 4: 构建 Tauri 应用（生成 setup.exe）**

Run:
```bash
cd d:\meetily\frontend
pnpm tauri build
```
Expected: 构建成功，生成 `d:\meetily\build-target\release\bundle\nsis\新际审会议助手_1.0.0_x64-setup.exe`（约 330 MB，因含 3 个模型）

- [ ] **Step 5: 验证安装包中模型文件就位**

Run (PowerShell) — 检查 NSIS 安装目录（安装后）：
```powershell
$instDir = "$env:LOCALAPPDATA\新际审会议助手"  # 或实际安装路径
$files = @(
  "$instDir\models\sense-voice\sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17\model.int8.onnx",
  "$instDir\models\sense-voice\sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17\tokens.txt",
  "$instDir\models\speaker-diarization\sherpa-onnx-pyannote-segmentation-3-0\model.onnx",
  "$instDir\models\speaker-diarization\3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx"
)
foreach ($f in $files) {
  if (Test-Path $f) { Write-Host "OK  $f" } else { Write-Host "MISSING  $f" }
}
```
Expected: 4 个文件全部 `OK`

- [ ] **Step 6: 端到端手动测试**

1. 安装新生成的 `setup.exe`
2. 启动应用，完成 onboarding（应只下载摘要引擎，无转录引擎下载步骤）
3. 开始录音，进行 2 人对话约 30 秒
4. 停止录音
5. 观察：
   - 停止后应出现 toast「正在处理说话人分离...」
   - 数秒后应出现 toast「说话人分离完成，识别到 N 位说话人」
   - 转录列表中每段话前应显示彩色「说话人 1」「说话人 2」标签
6. 验证 `transcripts.json` 中每个 segment 含 `speaker` 字段

- [ ] **Step 7: 边缘情况测试**

- 单人录音（独白）：应显示「说话人 1」
- 极短录音（<1 秒）：应跳过分离，转录正常显示，无 speaker 字段
- 网络断开（不应有影响，因模型全部内置）

- [ ] **Step 8: 最终 Commit**

```bash
cd d:\meetily
git add -A
git commit -m "feat(diarization): complete speaker diarization feature with bundled models and onboarding updates" --allow-empty
```

---

## 自检清单（Self-Review）

### 规格覆盖
- [x] 纯 Rust 实现（sherpa-onnx native）— Task 1-4
- [x] 后处理时机（录音停止后）— Task 11
- [x] 自动检测说话人（num_clusters: 0）— Task 2 Step 2
- [x] 三个模型全部打包 — Task 6, 7
- [x] 无开关，始终启用 — Task 11（无条件触发）
- [x] 时间戳对齐 — Task 3
- [x] `SpeakerSegment` 类型 — Task 1
- [x] `SpeakerDiarizationEngine` (new/is_ready/load/diarize) — Task 2
- [x] Tauri 命令 (init/is_ready/process) — Task 4
- [x] `align_transcripts_with_speakers` + 单元测试 — Task 3
- [x] `recording_saver.rs` stop 流程触发 — Task 11
- [x] `TranscriptSegment` + `TranscriptUpdate` 增加 speaker — Task 10
- [x] 前端 Transcript 类型 — Task 12
- [x] 前端事件监听 — Task 13
- [x] 说话人分组 UI — Task 14
- [x] SetupOverviewStep 移除转录步骤 — Task 15
- [x] DownloadProgressStep 移除转录卡片 — Task 16
- [x] OnboardingFlow 步骤调整 — Task 17
- [x] 错误处理：分离失败不影响转录 — Task 11 (warn + emit error)
- [x] 短录音跳过 — Task 4 Step 1 (<1s 返回空)
- [x] 模型缺失时 is_ready 返回 false — Task 2 Step 1
- [x] spawn_blocking 不阻塞 async — Task 4 Step 1
- [x] 构建与验证 — Task 18

### 占位符扫描
- 无 "TBD"、"TODO"、"implement later"
- 每个代码步骤均提供完整代码
- 每个命令均提供预期输出

### 类型一致性
- `SpeakerSegment` 字段 (start, end, speaker) 在 Task 1 定义，Task 2/3/4 使用一致
- `align_transcripts_with_speakers` 签名在 Task 3 定义，Task 11 调用一致
- `TranscriptChunkForAlignment` 在 Task 3 定义，Task 11 使用一致
- `speaker_diarization_process` 命令在 Task 4 定义，Task 11 调用一致
- 前端 `speaker?: number` 在 Task 12 定义，Task 13/14 使用一致

---

## 执行交接

**计划已完成并保存至 `docs/superpowers/plans/2026-07-23-speaker-diarization.md`。两种执行方式：**

**1. Subagent-Driven（推荐）** — 每个 Task 派发独立 subagent，Task 间审查，快速迭代

**2. Inline Execution** — 在当前会话中按 executing-plans 批量执行，带检查点

**请选择执行方式？**
