# Speaker Diarization for SenseVoice Transcription Engine

**Date**: 2026-07-23
**Status**: Approved (pending spec review)

## 1. Overview

Add speaker diarization (说话人分离) to the default transcription model (SenseVoice-Small) so that meeting transcripts automatically label which speaker said what. Uses the `sherpa-onnx` Rust crate's native `OfflineSpeakerDiarization` API — pure Rust, no Python dependency.

### Key Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Implementation path | Pure Rust: sherpa-onnx native | Maintains single-executable, no-Python architecture |
| Processing timing | Post-processing (after recording stops) | Global clustering gives best accuracy; doesn't affect real-time transcription |
| Speaker count | Automatic detection (`num_clusters: 0`) | Best UX for meetings with unknown speaker count |
| Model distribution | All 3 models bundled in installer | China mainland network can't reliably download from GitHub |
| Diarization toggle | Always enabled (no toggle) | Simplest UX; diarization is a core feature |
| Alignment strategy | Timestamp-based merge | Reuses existing `audio_start_time`/`audio_end_time` infrastructure |

### Models Bundled

| Model | Purpose | Size |
|-------|---------|------|
| SenseVoice int8 | ASR transcription | ~228 MB |
| sherpa-onnx-pyannote-segmentation-3-0 | Speaker segmentation | ~6 MB |
| 3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k | Speaker embedding extraction | ~40 MB |

**Total installer size**: ~55 MB → ~330 MB

## 2. Architecture

### Data Flow

```
Recording:  Mic → real-time chunked ASR → TranscriptUpdate{audio_start_time, audio_end_time, text}
Stop:       → save audio.mp4 → [NEW] trigger diarization post-processing
                                        ↓
                                decode audio.mp4 → PCM samples
                                        ↓
                                OfflineSpeakerDiarization::process(samples)
                                        ↓
                                speaker segments [(start, end, speaker_id), ...]
                                        ↓
                                align with TranscriptUpdate by timestamp → backfill speaker field
                                        ↓
                                emit "transcript-diarized" → frontend updates UI
```

### New Module: `speaker_diarization_engine/`

```
src-tauri/src/
├── sherpa_asr_engine/              # Existing: ASR engine
├── speaker_diarization_engine/     # NEW: speaker diarization engine
│   ├── mod.rs
│   ├── engine.rs                   # OfflineSpeakerDiarization wrapper
│   └── commands.rs                 # Tauri commands
└── audio/
    └── recording_saver.rs          # Modified: trigger diarization after stop
```

## 3. Rust Backend

### 3.1 `speaker_diarization_engine/engine.rs`

Wraps `OfflineSpeakerDiarization` from the `sherpa-onnx` crate.

```rust
pub struct SpeakerDiarizationEngine {
    diarizer: RwLock<Option<OfflineSpeakerDiarization>>,
    models_dir: PathBuf,
}
```

**Methods**:
- `new(models_dir: PathBuf) -> Self`
- `is_ready(&self) -> bool` — checks that pyannote + ERes2Net model files exist
- `load(&self) -> Result<(), String>` — creates `OfflineSpeakerDiarization` instance with config:
  - `segmentation`: pyannote model path
  - `embedding`: ERes2Net model path
  - `clustering`: `FastClusteringConfig { num_clusters: 0 }` (0 = auto-detect)
  - `min_duration_on: 0.3` (minimum speech segment 0.3s)
  - `min_duration_off: 0.5` (minimum silence 0.5s)
- `diarize(&self, samples: &[f32], sample_rate: i32) -> Result<Vec<SpeakerSegment>, String>`
  - Calls `diarizer.process(samples)`, returns sorted segments

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakerSegment {
    pub start: f32,       // seconds from recording start
    pub end: f32,         // seconds from recording start
    pub speaker: i32,     // speaker ID (0, 1, 2, ...)
}
```

### 3.2 `speaker_diarization_engine/commands.rs`

**Tauri commands**:
- `speaker_diarization_init()` — initialize engine
- `speaker_diarization_is_ready() -> bool` — check readiness
- `speaker_diarization_process(audio_path: String) -> Result<Vec<SpeakerSegment>, String>`
  - Internally: `decode_audio_file(path)` → PCM → `engine.diarize(samples)`

### 3.3 Modified: `recording_saver.rs` stop flow

After `finalize()` saves `audio.mp4`, add diarization post-processing:

```rust
// 1. [EXISTING] finalize() → save audio.mp4
// 2. [EXISTING] save transcripts.json
// 3. [NEW] Run diarization (always enabled)
if speaker_diarization_is_ready() {
    match speaker_diarization_process(audio_path) {
        Ok(segments) => {
            let aligned = align_transcripts_with_speakers(transcripts, &segments);
            emit("transcript-diarized", &aligned);
            // Update transcripts.json with speaker field
        }
        Err(e) => {
            log::warn!("Diarization failed, transcripts unaffected: {}", e);
            emit("transcript-diarization-error", &e);
        }
    }
}
```

### 3.4 Alignment Algorithm: `align_transcripts_with_speakers`

For each transcript chunk (which has `audio_start_time` and `audio_end_time`):
1. Find all speaker segments that overlap with the chunk's time range
2. Assign the speaker from the segment with the longest overlap duration
3. If no overlap (gap in diarization), assign the nearest preceding speaker

Returns updated transcript list with `speaker: Option<i32>` field added to each chunk.

## 4. Model Bundling

### 4.1 File Layout

```
d:\meetily\sherpa-libs\models\
├── sense-voice\
│   └── sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17\
│       ├── model.int8.onnx
│       └── tokens.txt
├── speaker-diarization\
│   ├── sherpa-onnx-pyannote-segmentation-3-0\
│   │   └── model.onnx
│   └── 3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx
```

### 4.2 `tauri.conf.json` Resources

Add model files to `bundle.resources` (Map format, installs to exe directory):

```json
"resources": {
  "sherpa-libs/models/sense-voice/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17/model.int8.onnx": "./models/sense-voice/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17/model.int8.onnx",
  "sherpa-libs/models/sense-voice/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17/tokens.txt": "./models/sense-voice/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17/tokens.txt",
  "sherpa-libs/models/speaker-diarization/sherpa-onnx-pyannote-segmentation-3-0/model.onnx": "./models/speaker-diarization/sherpa-onnx-pyannote-segmentation-3-0/model.onnx",
  "sherpa-libs/models/speaker-diarization/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx": "./models/speaker-diarization/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx"
}
```

### 4.3 Model Path Resolution

New helper: `get_bundled_models_dir(app_handle) -> PathBuf`
- **Production**: `app.path().resource_dir()` + `models/`
- **Development**: `d:\meetily\sherpa-libs\models\`

**Modified `sherpa_asr_engine`**:
- `is_model_downloaded()` and `load_model()` check bundled dir first, then `app_data_dir/models/sherpa_asr/` fallback
- `sherpa_asr_has_available_models()` returns `true` (SenseVoice is bundled)
- Onboarding download step for transcription engine removed

**Modified `speaker_diarization_engine`**:
- Model paths always read from bundled directory

### 4.4 Download Logic Retained

TranscriptSettings "download model" button retained for non-default models (e.g., Paraformer). SenseVoice defaults to bundled — no download needed.

## 5. Frontend Changes

### 5.1 Transcript Type Update

Add optional `speaker` field to transcript type in TranscriptContext:

```typescript
interface Transcript {
  // ... existing fields ...
  speaker?: number;  // speaker ID (0, 1, 2, ...)
}
```

### 5.2 Listen for Diarization Events

In the transcript page component:

```typescript
useEffect(() => {
  const unlisten = listen<{ transcripts: Transcript[] }>(
    'transcript-diarized',
    (event) => {
      setTranscripts(event.payload.transcripts);
    }
  );
  return () => { unlisten.then(fn => fn()); };
}, []);
```

### 5.3 Transcript UI: Speaker Grouping

Render transcripts grouped by speaker:
- Each speaker gets a distinct color (Speaker 1 = blue, 2 = green, 3 = orange, etc.)
- Speaker label header above their utterances
- Clicking a speaker header collapses/expands their utterances
- During diarization processing: show "正在分离说话人..." loading state

### 5.4 Post-Recording Toasts

- On stop: "正在处理说话人分离..."
- On success: "说话人分离完成，识别到 N 位说话人"
- On failure: "说话人分离失败，转录结果不受影响"

### 5.5 Onboarding Changes (transcription engine now bundled)

**SetupOverviewStep.tsx**:
- Remove "Step 1: Download Transcription Engine"
- Only show "Step 1: Download Summary Engine" (renumbered)
- Description: "新际审会议助手 需要您下载摘要 AI 模型才能正常工作。"
- Total steps: 2 (Mac: 3)

**DownloadProgressStep.tsx**:
- Remove transcription engine download card
- Description: "下载摘要引擎后，您就可以开始使用 新际审会议助手。"
- After summary engine download completes, user can enter the system
- Remove transcription retry logic (no longer needed)

## 6. Error Handling & Edge Cases

### 6.1 Diarization Failure Doesn't Affect Transcription
- Diarization is a post-processing enhancement
- On failure: transcripts save and display normally, just without speaker labels
- Toast: "说话人分离失败，转录结果不受影响"
- `transcripts.json` `speaker` field = `null`

### 6.2 Single Speaker
- If only 1 speaker detected (monologue/single-person recording): display "说话人 1" normally
- No special handling needed

### 6.3 Short Recordings
- Duration < 1 second: skip diarization (too short to be meaningful)
- Duration < 5 seconds: still run, may only have 1 speaker segment

### 6.4 Audio Decode Failure
- If `audio.mp4` decode fails (corrupt file etc.): skip diarization, log warning
- Does not affect completed transcripts

### 6.5 Models Missing (Development Environment)
- If bundled models dir doesn't exist (dev environment without models): `speaker_diarization_is_ready()` returns `false`
- `stop_recording` checks `is_ready()`, skips diarization if not ready
- Log: "Speaker diarization models not found, skipping"

### 6.6 Performance
- 1-hour audio diarization: ~5-15 seconds (CPU 8 threads)
- Runs in `tokio::task::spawn_blocking`, doesn't block async runtime
- Frontend shows loading state during processing

## 7. Files to Create/Modify

### New Files
- `src-tauri/src/speaker_diarization_engine/mod.rs`
- `src-tauri/src/speaker_diarization_engine/engine.rs`
- `src-tauri/src/speaker_diarization_engine/commands.rs`
- `sherpa-libs/models/speaker-diarization/sherpa-onnx-pyannote-segmentation-3-0/model.onnx` (download)
- `sherpa-libs/models/speaker-diarization/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx` (download)
- `sherpa-libs/models/sense-voice/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17/model.int8.onnx` (move from download to bundled)
- `sherpa-libs/models/sense-voice/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17/tokens.txt` (move from download to bundled)

### Modified Files
- `src-tauri/Cargo.toml` — no new dependencies (sherpa-onnx already includes diarization API)
- `src-tauri/tauri.conf.json` — add model files to `bundle.resources`
- `src-tauri/src/lib.rs` — register `speaker_diarization_engine` module and commands
- `src-tauri/src/sherpa_asr_engine/sherpa_asr_engine.rs` — model path resolution checks bundled dir first
- `src-tauri/src/sherpa_asr_engine/commands.rs` — `has_available_models` returns true for bundled SenseVoice
- `src-tauri/src/audio/recording_saver.rs` — trigger diarization after `finalize()`
- `src-tauri/src/audio/transcription/worker.rs` — add `speaker` field to `TranscriptUpdate`
- `frontend/src/contexts/TranscriptContext.tsx` — add `speaker` field to transcript type
- `frontend/src/components/onboarding/steps/SetupOverviewStep.tsx` — remove transcription download step
- `frontend/src/components/onboarding/steps/DownloadProgressStep.tsx` — remove transcription download card
- `frontend/src/components/onboarding/OnboardingFlow.tsx` — adjust step count
- `frontend/src/components/VirtualizedTranscriptView.tsx` — speaker grouping UI (group by speaker, color-coded labels)
- `frontend/src/components/MeetingDetails/TranscriptPanel.tsx` — pass speaker field through to view
- `frontend/src/app/_components/TranscriptPanel.tsx` — pass speaker field through to view (home page variant)

## 8. Testing Strategy

- Unit test: `align_transcripts_with_speakers` with mock segments and transcripts
- Integration test: run diarization on a known multi-speaker WAV file, verify speaker count
- Manual test: record a 2-person conversation, verify speaker labels appear after stop
- Edge case: single speaker recording, verify "说话人 1" displays
- Edge case: very short recording (<1s), verify diarization is skipped gracefully
