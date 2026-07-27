# 声纹注册与识别 + 转录/分离问题修复 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现声纹注册/识别新功能，修复转录模型未就绪 bug，将 ERes2Net 替换为 CAM++，降低聚类阈值修复同一人分裂，修复分离过程吞文字问题，最后构建新的 setup.exe。

**Architecture:** 新增 `voiceprint_engine` Rust 模块（共用 CAM++ 模型提取嵌入，注册声纹存储于 SQLite，匹配时按聚类质心比对）。前端在设置页新增"声纹"标签页（5 秒录音 + 姓名标注），在转录视图按 `speaker_name` 显示姓名。Bug 修复集中在 `api.rs`、`useRecordingStart.ts`、`TranscriptSettings.tsx`、`engine.rs`、`TranscriptContext.tsx`。

**Tech Stack:** Rust + sherpa-onnx 1.13.3 + sqlx + Tauri 2 + React + TypeScript + shadcn/ui + CPAL

---

## 文件结构

**新增**:
- `frontend/src-tauri/src/voiceprint_engine/mod.rs` — 模块入口
- `frontend/src-tauri/src/voiceprint_engine/engine.rs` — VoiceprintEngine 核心（嵌入提取 + 余弦相似度 + 匹配）
- `frontend/src-tauri/src/voiceprint_engine/commands.rs` — Tauri 命令
- `frontend/src-tauri/src/voiceprint_engine/repository.rs` — 数据库 CRUD
- `frontend/src-tauri/migrations/20260723000001_create_voiceprints.sql`
- `frontend/src-tauri/migrations/20260723000002_create_meeting_speaker_overrides.sql`
- `frontend/src-tauri/migrations/20260723000003_add_speaker_name.sql`
- `frontend/src-tauri/scripts/download-camplus-model.ps1` — 一次性下载脚本
- `frontend/src/components/VoiceprintSettings.tsx` — 设置页声纹标签页组件
- `frontend/src/contexts/VoiceprintContext.tsx` — 声纹姓名缓存

**修改**:
- `frontend/src-tauri/src/lib.rs` — 注册 voiceprint_engine 模块 + 命令 + setup hook
- `frontend/src-tauri/src/speaker_diarization_engine/engine.rs` — CAM++ 替换 + threshold 0.4
- `frontend/src-tauri/src/api/api.rs` — TranscriptSegment 加 speaker_name + 默认 provider 修复
- `frontend/src-tauri/src/audio/recording_saver.rs` — TranscriptSegment 加 speaker_name + 集成声纹匹配
- `frontend/src-tauri/src/audio/common.rs` — 如有 TranscriptSegment 定义同步
- `frontend/src-tauri/src/database/models.rs` — Transcript 加 speaker_name 字段
- `frontend/src-tauri/src/speaker_diarization_engine/commands.rs` — run_diarization_on_segments 集成声纹
- `frontend/src-tauri/tauri.conf.json` — CAM++ bundle + 版本号
- `frontend/src-tauri/Cargo.toml` — 版本号
- `frontend/package.json` — 版本号
- `frontend/src/app/settings/page.tsx` — 新增声纹标签页
- `frontend/src/components/TranscriptSettings.tsx` — 持久化配置
- `frontend/src/components/VirtualizedTranscriptView.tsx` — 显示姓名 + 接收 speaker_name
- `frontend/src/contexts/TranscriptContext.tsx` — 合并而非替换 + speaker_name 字段
- `frontend/src/hooks/useRecordingStart.ts` — parakeet case + 错误显示
- `frontend/src/types/index.ts` — speaker_name 字段

---

## Task 1: 下载 CAM++ 模型并替换 ERes2Net 常量

**Files:**
- Create: `d:\meetily\frontend\src-tauri\scripts\download-camplus-model.ps1`
- Modify: `d:\meetily\frontend\src-tauri\src\speaker_diarization_engine\engine.rs:30-32, 60-63, 66-80`

- [ ] **Step 1: 创建 CAM++ 模型下载脚本**

写入 `d:\meetily\frontend\src-tauri\scripts\download-camplus-model.ps1`：

```powershell
# 下载 CAM++ 说话人嵌入模型（3D-Speaker）用于替代 ERes2Net
# 目标目录: sherpa-libs/models/speaker-diarization/
$ErrorActionPreference = "Stop"
$targetDir = Join-Path $PSScriptRoot "..\sherpa-libs\models\speaker-diarization"
if (-not (Test-Path $targetDir)) { New-Item -ItemType Directory -Path $targetDir -Force | Out-Null }
$targetFile = Join-Path $targetDir "3dspeaker_speech_campplus_sv_zh-cn_16k-common.onnx"
if (Test-Path $targetFile) { Write-Host "CAM++ model already exists, skipping."; exit 0 }

$baseGithub = "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_campplus_sv_zh-cn_16k-common.onnx"
$mirrors = @(
    "https://gh.api.99988866.xyz/$baseGithub",
    "https://ghproxy.net/$baseGithub",
    "https://mirror.ghproxy.com/$baseGithub",
    $baseGithub
)
foreach ($url in $mirrors) {
    Write-Host "Downloading from: $url"
    try {
        Invoke-WebRequest -Uri $url -OutFile $targetFile -UseBasicParsing -TimeoutSec 60
        $size = (Get-Item $targetFile).Length
        if ($size -gt 1MB) { Write-Host "Success: $size bytes"; exit 0 }
        Remove-Item $targetFile -Force -ErrorAction SilentlyContinue
    } catch { Write-Host "Failed: $_" }
}
throw "All mirrors failed to download CAM++ model"
```

- [ ] **Step 2: 运行下载脚本**

Run: `powershell -ExecutionPolicy Bypass -File d:\meetily\frontend\src-tauri\scripts\download-camplus-model.ps1`
Expected: 输出 "Success: <size> bytes"，文件存在于 `d:\meetily\frontend\src-tauri\sherpa-libs\models\speaker-diarization\3dspeaker_speech_campplus_sv_zh-cn_16k-common.onnx`

- [ ] **Step 3: 修改 engine.rs 模型常量**

修改 `d:\meetily\frontend\src-tauri\src\speaker_diarization_engine\engine.rs` 第 32 行：

```rust
// 修改前
pub const ERES2NET_MODEL_FILE: &str = "3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx";

// 修改后
pub const CAMPLUS_MODEL_FILE: &str = "3dspeaker_speech_campplus_sv_zh-cn_16k-common.onnx";
```

- [ ] **Step 4: 修改 engine.rs 模型路径方法**

修改 `engine.rs` 第 60-63 行：

```rust
// 修改前
/// ERes2Net speaker embedding model path.
fn eres2net_model_path(&self) -> PathBuf {
    self.diarization_models_dir().join(ERES2NET_MODEL_FILE)
}

// 修改后
/// CAM++ speaker embedding model path.
fn camplus_model_path(&self) -> PathBuf {
    self.diarization_models_dir().join(CAMPLUS_MODEL_FILE)
}
```

- [ ] **Step 5: 修改 engine.rs is_ready() 和 load() 中的引用**

修改 `engine.rs` 第 66-80 行（is_ready 方法）中所有 `eres2net` → `camplus`：

```rust
pub fn is_ready(&self) -> bool {
    let pyannote = self.pyannote_model_path();
    let camplus = self.camplus_model_path();
    let ready = pyannote.exists() && camplus.exists();
    if !ready {
        info!(
            "[Diarization] Models not ready: pyannote={} exists={}, camplus={} exists={}",
            pyannote.display(),
            pyannote.exists(),
            camplus.display(),
            camplus.exists()
        );
    }
    ready
}
```

修改 `engine.rs` 第 90 行（load 方法）：

```rust
// 修改前
let eres2net_path = self.eres2net_model_path();

// 修改后
let camplus_path = self.camplus_model_path();
```

修改 `engine.rs` 第 92-96 行：

```rust
info!(
    "[Diarization] Loading models: pyannote={}, camplus={}",
    pyannote_path.display(),
    camplus_path.display()
);
```

修改 `engine.rs` 第 108-113 行：

```rust
let embedding = SpeakerEmbeddingExtractorConfig {
    model: Some(camplus_path.to_string_lossy().to_string()),
    num_threads: num_cpus(),
    debug: false,
    provider: Some("cpu".to_string()),
};
```

- [ ] **Step 6: 修改 tauri.conf.json bundle resources**

修改 `d:\meetily\frontend\src-tauri\tauri.conf.json` 第 101 行：

```json
// 修改前
"sherpa-libs/models/speaker-diarization/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx": "models/speaker-diarization/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx"

// 修改后
"sherpa-libs/models/speaker-diarization/3dspeaker_speech_campplus_sv_zh-cn_16k-common.onnx": "models/speaker-diarization/3dspeaker_speech_campplus_sv_zh-cn_16k-common.onnx"
```

- [ ] **Step 7: 删除旧 ERes2Net 模型文件**

Run: `del d:\meetily\frontend\src-tauri\sherpa-libs\models\speaker-diarization\3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx`

- [ ] **Step 8: 验证编译**

Run: `cd d:\meetily\frontend\src-tauri && cargo check`
Expected: 无错误（warning 可接受）

- [ ] **Step 9: Commit**

```bash
git add frontend/src-tauri/src/speaker_diarization_engine/engine.rs frontend/src-tauri/tauri.conf.json frontend/src-tauri/scripts/download-camplus-model.ps1
git commit -m "feat(diarization): 替换 ERes2Net 为 CAM++ 模型以提升中文准确度"
```

---

## Task 2: 降低聚类阈值修复同一人分裂

**Files:**
- Modify: `d:\meetily\frontend\src-tauri\src\speaker_diarization_engine\engine.rs:116-119`

- [ ] **Step 1: 修改 FastClusteringConfig threshold**

修改 `engine.rs` 第 116-119 行：

```rust
// 修改前
let clustering = FastClusteringConfig {
    num_clusters: 0,
    threshold: 0.5,
};

// 修改后
let clustering = FastClusteringConfig {
    num_clusters: 0,
    threshold: 0.4,  // 降低阈值更宽松合并同一说话人（CAM++ 配合）
};
```

- [ ] **Step 2: 运行现有测试确认不破坏**

Run: `cd d:\meetily\frontend\src-tauri && cargo test --lib speaker_diarization_engine::engine::tests`
Expected: 6 个测试全部通过

- [ ] **Step 3: Commit**

```bash
git add frontend/src-tauri/src/speaker_diarization_engine/engine.rs
git commit -m "fix(diarization): 聚类阈值 0.5→0.4，缓解同一人被识别为多说话人"
```

---

## Task 3: 修复转录模型未就绪 bug（后端默认 provider）

**Files:**
- Modify: `d:\meetily\frontend\src-tauri\src\api\api.rs:640-647`

- [ ] **Step 1: 修改 api_get_transcript_config 默认返回值**

修改 `d:\meetily\frontend\src-tauri\src\api\api.rs` 第 640-647 行：

```rust
// 修改前
Ok(None) => {
    log_info!("No transcript config found, returning default.");
    Ok(Some(TranscriptConfig {
        provider: "parakeet".to_string(),
        model: crate::config::DEFAULT_PARAKEET_MODEL.to_string(),
        api_key: None,
    }))
}

// 修改后
Ok(None) => {
    log_info!("No transcript config found, returning default (sherpaAsr/SenseVoice).");
    Ok(Some(TranscriptConfig {
        provider: "sherpaAsr".to_string(),
        model: crate::sherpa_asr_engine::sherpa_asr_engine::DEFAULT_MODEL_NAME.to_string(),
        api_key: None,
    }))
}
```

- [ ] **Step 2: 验证 DEFAULT_MODEL_NAME 已导出**

Run: `cd d:\meetily\frontend\src-tauri && cargo check`
Expected: 无错误。若报错 "cannot find DEFAULT_MODEL_NAME"，检查 `sherpa_asr_engine.rs` 中 `pub const DEFAULT_MODEL_NAME` 是否存在且为 `pub`。

- [ ] **Step 3: Commit**

```bash
git add frontend/src-tauri/src/api/api.rs
git commit -m "fix(api): 默认转录 provider parakeet→sherpaAsr，修复首次启动未就绪"
```

---

## Task 4: 前端修复 useRecordingStart（parakeet case + 错误显示）

**Files:**
- Modify: `d:\meetily\frontend\src\hooks\useRecordingStart.ts:54-109, 145-148, 214-217, 302-305`

- [ ] **Step 1: 修改 checkTranscriptionModelReady 添加 parakeet case 并显示真实错误**

修改 `d:\meetily\frontend\src\hooks\useRecordingStart.ts` 第 54-109 行整个函数为：

```typescript
const checkTranscriptionModelReady = useCallback(async (): Promise<{ ready: boolean; error?: string }> => {
    try {
        const config = await invoke<TranscriptModelProps | null>('api_get_transcript_config');

        if (!config) {
            await invoke('sherpa_asr_init');
            const hasModels = await invoke<boolean>('sherpa_asr_has_available_models');
            return { ready: hasModels, error: hasModels ? undefined : '未找到可用的转录模型' };
        }

        const provider = config.provider;
        console.log(`[Recording] Checking model readiness - provider: ${provider}, model: ${config.model}`);

        switch (provider) {
            case 'sherpaAsr': {
                await invoke('sherpa_asr_init');
                try {
                    await invoke<string>('sherpa_asr_validate_model_ready');
                    return { ready: true };
                } catch (e: any) {
                    const msg = typeof e === 'string' ? e : (e?.message || String(e));
                    console.error('[Recording] Sherpa-ASR model validation failed:', msg);
                    return { ready: false, error: msg };
                }
            }
            case 'parakeet': {
                try {
                    await invoke<string>('parakeet_validate_model_ready');
                    return { ready: true };
                } catch (e: any) {
                    const msg = typeof e === 'string' ? e : (e?.message || String(e));
                    console.error('[Recording] Parakeet model validation failed:', msg);
                    return { ready: false, error: msg };
                }
            }
            case 'localWhisper': {
                try {
                    await invoke<string>('whisper_validate_model_ready');
                    return { ready: true };
                } catch (e: any) {
                    const msg = typeof e === 'string' ? e : (e?.message || String(e));
                    console.error('[Recording] Whisper model validation failed:', msg);
                    return { ready: false, error: msg };
                }
            }
            case 'deepgram':
            case 'groq':
            case 'openai':
            case 'elevenLabs': {
                if (!config.apiKey) {
                    return { ready: false, error: `${provider} API key 未配置` };
                }
                return { ready: true };
            }
            default: {
                await invoke('sherpa_asr_init');
                const hasModels = await invoke<boolean>('sherpa_asr_has_available_models');
                return { ready: hasModels, error: hasModels ? undefined : '未知 provider 且无可用模型' };
            }
        }
    } catch (error: any) {
        const msg = typeof error === 'string' ? error : (error?.message || String(error));
        console.error('Failed to check transcription model readiness:', msg);
        return { ready: false, error: msg };
    }
}, []);
```

- [ ] **Step 2: 修改所有调用点（共 3 处）显示真实错误**

第 135 行附近（手动开始）：

```typescript
// 修改前
const isReady = await checkTranscriptionModelReady();
if (!isReady) { ... toast.error('转录模型未就绪，请先下载转录模型再开始录音。'); ... }

// 修改后
const { ready, error } = await checkTranscriptionModelReady();
if (!ready) {
    const msg = error ? `转录模型未就绪：${error}` : '转录模型未就绪，请先下载转录模型再开始录音。';
    toast.error(msg, { duration: 8000 });
    showModal?.('modelSelector', msg);
    return;
}
```

应用相同模式到另外两处（自动开始和侧边栏直接开始），搜索 `checkTranscriptionModelReady` 全部调用点并修改。

- [ ] **Step 3: 验证 TypeScript 编译**

Run: `cd d:\meetily\frontend && pnpm tsc --noEmit`
Expected: 无错误

- [ ] **Step 4: Commit**

```bash
git add frontend/src/hooks/useRecordingStart.ts
git commit -m "fix(recording): 添加 parakeet case 并显示真实模型校验错误"
```

---

## Task 5: TranscriptSettings 持久化配置

**Files:**
- Modify: `d:\meetily\frontend\src\components\TranscriptSettings.tsx`

- [ ] **Step 1: 在 TranscriptSettings 中加入持久化逻辑**

在 `d:\meetily\frontend\src\components\TranscriptSettings.tsx` 中找到 provider 和 model 变更的 `onValueChange` 回调（搜索 `setTranscriptModelConfig`），在每个变更后立即调用 `api_save_transcript_config`。

在文件顶部 import 区添加：
```typescript
// 无需新 import，invoke 已导入
```

将所有 `setTranscriptModelConfig({ ... })` 调用包装为持久化版本。例如对 provider 切换：

```typescript
const handleProviderChange = async (newProvider: TranscriptModelProps['provider']) => {
    const newConfig = {
        ...transcriptModelConfig,
        provider: newProvider,
        model: newProvider === 'sherpaAsr' ? 'sense-voice-zh-en-ja-ko-yue-int8' : transcriptModelConfig.model,
        apiKey: (newProvider === 'deepgram' || newProvider === 'groq' || newProvider === 'openai' || newProvider === 'elevenLabs') ? apiKey : null,
    };
    setTranscriptModelConfig(newConfig);
    try {
        await invoke('api_save_transcript_config', {
            provider: newConfig.provider,
            model: newConfig.model,
            apiKey: newConfig.apiKey
        });
        console.log('[TranscriptSettings] Saved config:', newConfig.provider, newConfig.model);
    } catch (e) {
        console.error('[TranscriptSettings] Failed to save config:', e);
        toast.error('保存转录配置失败: ' + String(e));
    }
};
```

对 model 选择（sherpaAsr 子选择器和 localWhisper ModelManager 回调）应用相同模式：调用 `setTranscriptModelConfig` 后立即 `api_save_transcript_config`。

- [ ] **Step 2: 同步设置页面 page.tsx 默认值**

修改 `d:\meetily\frontend\src\app\settings\page.tsx` 第 40-41 行：

```typescript
// 修改前
provider: config.provider || 'localWhisper',
model: config.model || 'large-v3',

// 修改后
provider: config.provider || 'sherpaAsr',
model: config.model || 'sense-voice-zh-en-ja-ko-yue-int8',
```

- [ ] **Step 3: 验证编译**

Run: `cd d:\meetily\frontend && pnpm tsc --noEmit`
Expected: 无错误

- [ ] **Step 4: Commit**

```bash
git add frontend/src/components/TranscriptSettings.tsx frontend/src/app/settings/page.tsx
git commit -m "fix(settings): 转录配置变更立即持久化到后端，默认值统一为 sherpaAsr"
```

---

## Task 6: 修复分离过程吞文字（前端合并而非替换）

**Files:**
- Modify: `d:\meetily\frontend\src\contexts\TranscriptContext.tsx:394-413`

- [ ] **Step 1: 修改 transcript-diarized 事件处理为合并模式**

修改 `d:\meetily\frontend\src\contexts\TranscriptContext.tsx` 第 394-413 行：

```typescript
// 修改前
unlistenFn = await listen<{
    transcripts: Array<{...}>;
    num_speakers: number;
}>('transcript-diarized', (event) => {
    const payload = event.payload;
    const diarized: Transcript[] = payload.transcripts.map((seg) => ({...}));
    setTranscripts(diarized);
    toast.dismiss('diarization-loading');
    toast.success(`说话人分离完成，识别到 ${payload.num_speakers} 位说话人`);
});

// 修改后
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
        speaker_name?: string | null;
    }>;
    num_speakers: number;
    speaker_names?: Record<number, string | null>;
}>('transcript-diarized', (event) => {
    const payload = event.payload;
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
        speaker_name: seg.speaker_name ?? undefined,
    }));
    // 合并而非替换：保留分离期间新到达的 transcript
    setTranscripts(prev => {
        const map = new Map(prev.map(t => [t.id, t]));
        for (const d of diarized) {
            map.set(d.id, d);
        }
        return Array.from(map.values()).sort((a, b) => a.sequence_id - b.sequence_id);
    });
    toast.dismiss('diarization-loading');
    toast.success(`说话人分离完成，识别到 ${payload.num_speakers} 位说话人`);
});
```

- [ ] **Step 2: 验证编译**

Run: `cd d:\meetily\frontend && pnpm tsc --noEmit`
Expected: 无错误（Transcript 类型需先有 speaker_name 字段，见 Task 10）

- [ ] **Step 3: Commit（与 Task 10 一起提交，因类型依赖）**

暂不单独提交，等 Task 10 完成后一起提交。

---

## Task 7: 后端 TranscriptSegment 增加 speaker_name 字段

**Files:**
- Modify: `d:\meetily\frontend\src-tauri\src\api\api.rs:182-197`
- Modify: `d:\meetily\frontend\src-tauri\src\audio\recording_saver.rs:20-33`
- Modify: `d:\meetily\frontend\src-tauri\src\database\models.rs:25-40`
- Create: `d:\meetily\frontend\src-tauri\migrations\20260723000003_add_speaker_name.sql`

- [ ] **Step 1: 创建迁移文件**

写入 `d:\meetily\frontend\src-tauri\migrations\20260723000003_add_speaker_name.sql`：

```sql
-- 声纹匹配或手动指派的说话人姓名（覆盖默认"说话人 N"显示）
ALTER TABLE transcripts ADD COLUMN speaker_name TEXT;
```

- [ ] **Step 2: 修改 api.rs TranscriptSegment**

修改 `d:\meetily\frontend\src-tauri\src\api\api.rs` 第 182-197 行：

```rust
#[derive(Debug, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub id: String,
    pub text: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_start_time: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_end_time: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    /// Speaker ID assigned by diarization (None until post-processing runs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<i32>,
    /// Speaker name from voiceprint matching or manual assignment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_name: Option<String>,
}
```

- [ ] **Step 3: 修改 recording_saver.rs TranscriptSegment**

修改 `d:\meetily\frontend\src-tauri\src\audio\recording_saver.rs` 第 20-33 行：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub id: String,
    pub text: String,
    pub audio_start_time: f64,
    pub audio_end_time: f64,
    pub duration: f64,
    pub display_time: String,
    pub confidence: f32,
    pub sequence_id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<i32>,
    /// Speaker name from voiceprint matching or manual assignment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_name: Option<String>,
}
```

- [ ] **Step 4: 修改 database/models.rs Transcript**

修改 `d:\meetily\frontend\src-tauri\src\database\models.rs` 第 25-40 行：

```rust
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Transcript {
    pub id: String,
    pub meeting_id: String,
    pub transcript: String,
    pub timestamp: String,
    pub summary: Option<String>,
    pub action_items: Option<String>,
    pub key_points: Option<String>,
    pub audio_start_time: Option<f64>,
    pub audio_end_time: Option<f64>,
    pub duration: Option<f64>,
    pub speaker: Option<String>,
    /// Speaker name from voiceprint matching or manual assignment.
    pub speaker_name: Option<String>,
}
```

- [ ] **Step 5: 全局搜索所有构造 TranscriptSegment 的位置并补 speaker_name 字段**

Run: `cd d:\meetily\frontend\src-tauri && cargo check 2>&1 | findstr "missing field"`
对每个报错位置补 `speaker_name: None,`（api.rs 结构）或对应字段。

- [ ] **Step 6: 验证编译**

Run: `cd d:\meetily\frontend\src-tauri && cargo check`
Expected: 无错误

- [ ] **Step 7: Commit**

```bash
git add frontend/src-tauri/migrations/20260723000003_add_speaker_name.sql frontend/src-tauri/src/api/api.rs frontend/src-tauri/src/audio/recording_saver.rs frontend/src-tauri/src/database/models.rs
git commit -m "feat(db): TranscriptSegment 增加 speaker_name 字段"
```

---

## Task 8: 创建 voiceprints 和 meeting_speaker_overrides 表

**Files:**
- Create: `d:\meetily\frontend\src-tauri\migrations\20260723000001_create_voiceprints.sql`
- Create: `d:\meetily\frontend\src-tauri\migrations\20260723000002_create_meeting_speaker_overrides.sql`

- [ ] **Step 1: 创建 voiceprints 迁移**

写入 `d:\meetily\frontend\src-tauri\migrations\20260723000001_create_voiceprints.sql`：

```sql
-- 已注册声纹：存储姓名 + 嵌入向量 + 5秒样本音频路径
CREATE TABLE voiceprints (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    embedding BLOB NOT NULL,
    audio_path TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_voiceprints_name ON voiceprints(name);
```

- [ ] **Step 2: 创建 meeting_speaker_overrides 迁移**

写入 `d:\meetily\frontend\src-tauri\migrations\20260723000002_create_meeting_speaker_overrides.sql`：

```sql
-- 会议内说话人 ID → 已注册声纹的映射（含自动匹配和手动指派）
CREATE TABLE meeting_speaker_overrides (
    meeting_id TEXT NOT NULL,
    speaker_id INTEGER NOT NULL,
    voiceprint_id TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT 'manual',
    PRIMARY KEY (meeting_id, speaker_id),
    FOREIGN KEY (voiceprint_id) REFERENCES voiceprints(id) ON DELETE CASCADE
);
```

- [ ] **Step 3: Commit**

```bash
git add frontend/src-tauri/migrations/20260723000001_create_voiceprints.sql frontend/src-tauri/migrations/20260723000002_create_meeting_speaker_overrides.sql
git commit -m "feat(db): 新增 voiceprints 和 meeting_speaker_overrides 表"
```

---

## Task 9: 实现 voiceprint_engine repository

**Files:**
- Create: `d:\meetily\frontend\src-tauri\src\voiceprint_engine\mod.rs`
- Create: `d:\meetily\frontend\src-tauri\src\voiceprint_engine\repository.rs`

- [ ] **Step 1: 创建模块入口**

写入 `d:\meetily\frontend\src-tauri\src\voiceprint_engine\mod.rs`：

```rust
pub mod engine;
pub mod commands;
pub mod repository;
```

- [ ] **Step 2: 创建 repository 实现**

写入 `d:\meetily\frontend\src-tauri\src\voiceprint_engine\repository.rs`：

```rust
use sqlx::sqlite::SqlitePool;
use log::info;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceprintRecord {
    pub id: String,
    pub name: String,
    pub embedding: Vec<u8>,
    pub audio_path: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingSpeakerOverride {
    pub meeting_id: String,
    pub speaker_id: i32,
    pub voiceprint_id: String,
    pub source: String,
}

pub async fn insert_voiceprint(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    embedding: &[u8],
    audio_path: &str,
    created_at: &str,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO voiceprints (id, name, embedding, audio_path, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(name)
    .bind(embedding)
    .bind(audio_path)
    .bind(created_at)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to insert voiceprint: {}", e))?;
    Ok(())
}

pub async fn list_voiceprints(pool: &SqlitePool) -> Result<Vec<VoiceprintRecord>, String> {
    sqlx::query_as::<_, VoiceprintRecord>("SELECT id, name, embedding, audio_path, created_at FROM voiceprints ORDER BY created_at DESC")
        .fetch_all(pool)
        .await
        .map_err(|e| format!("Failed to list voiceprints: {}", e))
}

pub async fn get_voiceprint(pool: &SqlitePool, id: &str) -> Result<Option<VoiceprintRecord>, String> {
    sqlx::query_as::<_, VoiceprintRecord>("SELECT id, name, embedding, audio_path, created_at FROM voiceprints WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("Failed to get voiceprint: {}", e))
}

pub async fn delete_voiceprint(pool: &SqlitePool, id: &str) -> Result<(), String> {
    sqlx::query("DELETE FROM voiceprints WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to delete voiceprint: {}", e))?;
    Ok(())
}

pub async fn upsert_override(
    pool: &SqlitePool,
    meeting_id: &str,
    speaker_id: i32,
    voiceprint_id: &str,
    source: &str,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO meeting_speaker_overrides (meeting_id, speaker_id, voiceprint_id, source)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(meeting_id, speaker_id) DO UPDATE SET voiceprint_id = excluded.voiceprint_id, source = excluded.source
         WHERE source = 'manual' OR meeting_speaker_overrides.source != 'manual'",
    )
    .bind(meeting_id)
    .bind(speaker_id)
    .bind(voiceprint_id)
    .bind(source)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to upsert override: {}", e))?;
    info!("[Voiceprint] Override upserted: meeting={} speaker={} voiceprint={} source={}", meeting_id, speaker_id, voiceprint_id, source);
    Ok(())
}

pub async fn list_overrides_for_meeting(
    pool: &SqlitePool,
    meeting_id: &str,
) -> Result<Vec<MeetingSpeakerOverride>, String> {
    sqlx::query_as::<_, MeetingSpeakerOverride>(
        "SELECT meeting_id, speaker_id, voiceprint_id, source FROM meeting_speaker_overrides WHERE meeting_id = ?",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to list overrides: {}", e))
}
```

- [ ] **Step 3: 在 lib.rs 注册模块**

修改 `d:\meetily\frontend\src-tauri\src\lib.rs` 第 52 行附近（其他 pub mod 旁）：

```rust
pub mod voiceprint_engine;
```

- [ ] **Step 4: 验证编译（先建占位 engine.rs 和 commands.rs）**

为通过编译，先创建占位文件 `d:\meetily\frontend\src-tauri\src\voiceprint_engine\engine.rs`：

```rust
// 占位，将在 Task 10 实现
```

和 `d:\meetily\frontend\src-tauri\src\voiceprint_engine\commands.rs`：

```rust
// 占位，将在 Task 11 实现
```

Run: `cd d:\meetily\frontend\src-tauri && cargo check`
Expected: 无错误

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/voiceprint_engine/ frontend/src-tauri/src/lib.rs
git commit -m "feat(voiceprint): 添加模块骨架和 repository 数据库操作"
```

---

## Task 10: 实现 VoiceprintEngine 核心（嵌入提取 + 匹配）

**Files:**
- Modify: `d:\meetily\frontend\src-tauri\src\voiceprint_engine\engine.rs`

- [ ] **Step 1: 实现 VoiceprintEngine**

覆盖写入 `d:\meetily\frontend\src-tauri\src\voiceprint_engine\engine.rs`：

```rust
// voiceprint_engine/engine.rs
//
// 声纹嵌入提取与匹配。共用 diarization engine 的 CAM++ 模型。

use log::{info, warn};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use serde::{Deserialize, Serialize};
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
    pub fn extract_embedding(&self, samples: &[f32]) -> Result<Vec<f32>, String> {
        self.ensure_extractor()?;
        let guard = self.extractor.read().unwrap();
        let extractor = guard.as_ref().ok_or("Extractor not loaded")?;

        let embedding = tokio::task::block_in_place(|| extractor.compute(samples))
            .ok_or_else(|| "SpeakerEmbeddingExtractor.compute() returned None".to_string())?;

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
            ("id1".to_string(), "张三".to_string(), vec![0.9, 0.43589]),  // ~0.9
            ("id2".to_string(), "李四".to_string(), vec![1.0, 0.0]),      // 1.0
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
```

- [ ] **Step 2: 运行单元测试**

Run: `cd d:\meetily\frontend\src-tauri && cargo test --lib voiceprint_engine::engine::tests`
Expected: 5 个测试通过

- [ ] **Step 3: Commit**

```bash
git add frontend/src-tauri/src/voiceprint_engine/engine.rs
git commit -m "feat(voiceprint): VoiceprintEngine 嵌入提取与余弦相似度匹配实现"
```

---

## Task 11: 实现 voiceprint_engine Tauri 命令

**Files:**
- Modify: `d:\meetily\frontend\src-tauri\src\voiceprint_engine\commands.rs`

- [ ] **Step 1: 实现所有 Tauri 命令**

覆盖写入 `d:\meetily\frontend\src-tauri\src\voiceprint_engine\commands.rs`：

```rust
// voiceprint_engine/commands.rs
//
// Tauri 命令：声纹注册、列表、删除、从会议抓取样本、会议匹配、手动指派

use std::path::PathBuf;
use log::{info, warn, error};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::Utc;

use super::engine::{get_engine, VoiceprintMatch};
use super::repository::{
    insert_voiceprint, list_voiceprints, get_voiceprint, delete_voiceprint,
    upsert_override, list_overrides_for_meeting,
    VoiceprintRecord,
};
use crate::audio::decoder::decode_audio_file;
use crate::database::models::{MeetingModel, Transcript};
use sqlx::query_as;

#[derive(Debug, Serialize)]
pub struct VoiceprintDto {
    pub id: String,
    pub name: String,
    pub audio_path: String,
    pub created_at: String,
}

impl From<VoiceprintRecord> for VoiceprintDto {
    fn from(r: VoiceprintRecord) -> Self {
        Self {
            id: r.id,
            name: r.name,
            audio_path: r.audio_path,
            created_at: r.created_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct MeetingWithSpeakersDto {
    pub meeting_id: String,
    pub meeting_title: String,
    pub created_at: String,
    pub speaker_ids: Vec<i32>,
}

#[derive(Debug, Serialize)]
pub struct ExtractedSampleDto {
    pub audio_path: String,
    pub duration_seconds: f64,
    pub segment_start: f64,
    pub segment_end: f64,
}

#[derive(Debug, Serialize)]
pub struct MeetingMatchResult {
    pub matched: Vec<(i32, String, f32)>,  // (speaker_id, name, similarity)
    pub unmatched_speaker_ids: Vec<i32>,
}

/// 列出所有已注册声纹
#[tauri::command]
pub async fn voiceprint_list<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<VoiceprintDto>, String> {
    let pool = state.db_manager.pool();
    let records = list_voiceprints(pool).await?;
    Ok(records.into_iter().map(VoiceprintDto::from).collect())
}

/// 列出所有已完成说话人分离的会议（含 speaker_id 列表）
#[tauri::command]
pub async fn voiceprint_list_meetings_with_speakers<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<MeetingWithSpeakersDto>, String> {
    let pool = state.db_manager.pool();
    // 查询所有有 speaker 标注的会议
    let meetings: Vec<MeetingModel> = query_as::<_, MeetingModel>(
        "SELECT id, title, created_at, updated_at, folder_path FROM meetings WHERE folder_path IS NOT NULL ORDER BY created_at DESC"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to fetch meetings: {}", e))?;

    let mut result = Vec::new();
    for m in meetings {
        // 查询该会议的 distinct speaker_id
        let speaker_rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT speaker FROM transcripts WHERE meeting_id = ? AND speaker IS NOT NULL"
        )
        .bind(&m.id)
        .fetch_all(pool)
        .await
        .map_err(|e| format!("Failed to fetch speakers: {}", e))?;

        let speaker_ids: Vec<i32> = speaker_rows.into_iter()
            .filter_map(|(s,)| s.parse::<i32>().ok())
            .collect();

        if !speaker_ids.is_empty() {
            result.push(MeetingWithSpeakersDto {
                meeting_id: m.id,
                meeting_title: m.title,
                created_at: m.created_at.0.to_rfc3339(),
                speaker_ids,
            });
        }
    }
    Ok(result)
}

/// 从指定会议的指定说话人自动抓取最长 segment 作为声纹样本
#[tauri::command]
pub async fn voiceprint_extract_sample<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, crate::AppState>,
    meeting_id: String,
    speaker_id: i32,
) -> Result<ExtractedSampleDto, String> {
    let pool = state.db_manager.pool();
    let speaker_str = speaker_id.to_string();

    // 查询该 speaker 的所有 segments，按 duration 降序
    let segments: Vec<Transcript> = query_as::<_, Transcript>(
        "SELECT id, meeting_id, transcript, timestamp, summary, action_items, key_points, audio_start_time, audio_end_time, duration, speaker, speaker_name FROM transcripts WHERE meeting_id = ? AND speaker = ? ORDER BY (COALESCE(audio_end_time, 0) - COALESCE(audio_start_time, 0)) DESC"
    )
    .bind(&meeting_id)
    .bind(&speaker_str)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to fetch segments: {}", e))?;

    if segments.is_empty() {
        return Err(format!("Meeting {} has no segments for speaker {}", meeting_id, speaker_id));
    }

    // 选取最长 segment，若 > 10s 截取前 10s，若 < 3s 拼接相邻同 speaker segments
    let mut best_start = segments[0].audio_start_time.unwrap_or(0.0);
    let mut best_end = segments[0].audio_end_time.unwrap_or(0.0);

    // 若最长段不足 3 秒，尝试拼接
    if best_end - best_start < 3.0 {
        let mut combined_start = best_start;
        let mut combined_end = best_end;
        for seg in segments.iter().skip(1) {
            let s = seg.audio_start_time.unwrap_or(0.0);
            let e = seg.audio_end_time.unwrap_or(0.0);
            if s <= combined_end + 1.0 && e > combined_end {  // 相邻或接近
                combined_end = e;
                if combined_end - combined_start >= 3.0 { break; }
            }
        }
        best_start = combined_start;
        best_end = combined_end;
    }

    // 截取最长 10 秒
    if best_end - best_start > 10.0 {
        best_end = best_start + 10.0;
    }

    if best_end - best_start < 1.0 {
        return Err("该说话人的音频片段过短，无法提取有效声纹样本".to_string());
    }

    // 获取会议音频文件
    let meeting: MeetingModel = query_as::<_, MeetingModel>(
        "SELECT id, title, created_at, updated_at, folder_path FROM meetings WHERE id = ?"
    )
    .bind(&meeting_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("Failed to fetch meeting: {}", e))?
    .ok_or_else(|| format!("Meeting not found: {}", meeting_id))?;

    let folder_path = meeting.folder_path.as_ref()
        .ok_or_else(|| "Meeting has no folder_path".to_string())?;
    let audio_path = PathBuf::from(folder_path).join("audio.mp4");
    if !audio_path.exists() {
        return Err(format!("Audio file not found: {}", audio_path.display()));
    }

    // 解码音频
    let audio_path_clone = audio_path.clone();
    let decoded = tokio::task::spawn_blocking(move || decode_audio_file(&audio_path_clone))
        .await
        .map_err(|e| format!("Decode join error: {}", e))?
        .map_err(|e| format!("Failed to decode audio: {}", e))?;
    let samples = decoded.to_whisper_format();
    let sample_rate = decoded.sample_rate as f32;

    // 切片
    let start_sample = (best_start * sample_rate) as usize;
    let end_sample = (best_end * sample_rate) as usize;
    let end_sample = end_sample.min(samples.len());
    if end_sample <= start_sample {
        return Err("音频切片无效".to_string());
    }
    let seg_samples = samples[start_sample..end_sample].to_vec();

    // 保存为 WAV
    let app_data = app.path().app_data_dir()
        .map_err(|e| format!("Failed to get app_data_dir: {}", e))?;
    let temp_dir = app_data.join("voiceprints").join("temp");
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("Failed to create temp dir: {}", e))?;
    let sample_id = Uuid::new_v4().to_string();
    let wav_path = temp_dir.join(format!("{}.wav", sample_id));

    let wav_path_clone = wav_path.clone();
    tokio::task::spawn_blocking(move || write_wav_file(&wav_path_clone, &seg_samples, 16000))
        .await
        .map_err(|e| format!("WAV write join error: {}", e))??;

    let duration = best_end - best_start;
    info!("[Voiceprint] Extracted sample: meeting={} speaker={} duration={:.1}s path={}",
          meeting_id, speaker_id, duration, wav_path.display());

    Ok(ExtractedSampleDto {
        audio_path: wav_path.to_string_lossy().to_string(),
        duration_seconds: duration,
        segment_start: best_start,
        segment_end: best_end,
    })
}

fn write_wav_file(path: &std::path::Path, samples: &[f32], sample_rate: u32) -> Result<(), String> {
    use std::io::Write;
    let mut buf = Vec::new();
    let num_samples = samples.len();
    let data_size = num_samples * 2;
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_size as u32).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    buf.extend_from_slice(&2u16.to_le_bytes());
    buf.extend_from_slice(&16u16.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&(data_size as u32).to_le_bytes());
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        buf.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::File::create(path)
        .and_then(|mut f| f.write_all(&buf))
        .map_err(|e| format!("Failed to write WAV: {}", e))?;
    Ok(())
}

/// 注册声纹：解码音频 → 提取嵌入 → 存库
#[tauri::command]
pub async fn voiceprint_register<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, crate::AppState>,
    name: String,
    audio_path: String,
) -> Result<VoiceprintDto, String> {
    if name.trim().is_empty() {
        return Err("姓名不能为空".to_string());
    }

    let engine = get_engine()?;

    let path = PathBuf::from(&audio_path);
    let decoded = tokio::task::spawn_blocking(move || decode_audio_file(&path))
        .await
        .map_err(|e| format!("Decode join error: {}", e))?
        .map_err(|e| format!("Failed to decode audio: {}", e))?;

    if decoded.duration_seconds < 1.0 {
        return Err("音频过短，至少需要 1 秒".to_string());
    }

    let samples = decoded.to_whisper_format();

    let embedding = tokio::task::spawn_blocking(move || engine.extract_embedding(&samples))
        .await
        .map_err(|e| format!("Embedding join error: {}", e))??;

    let embedding_bytes = super::engine::VoiceprintEngine::serialize_embedding(&embedding);

    let app_data = app.path().app_data_dir()
        .map_err(|e| format!("Failed to get app_data_dir: {}", e))?;
    let voiceprints_dir = app_data.join("voiceprints");
    std::fs::create_dir_all(&voiceprints_dir)
        .map_err(|e| format!("Failed to create voiceprints dir: {}", e))?;

    let id = Uuid::new_v4().to_string();
    let permanent_path = voiceprints_dir.join(format!("{}.wav", id));
    std::fs::rename(&audio_path, &permanent_path)
        .or_else(|_| std::fs::copy(&audio_path, &permanent_path).map(|_| ()))
        .map_err(|e| format!("Failed to move audio file: {}", e))?;

    let created_at = Utc::now().to_rfc3339();
    let pool = state.db_manager.pool();
    insert_voiceprint(
        pool,
        &id,
        &name,
        &embedding_bytes,
        &permanent_path.to_string_lossy(),
        &created_at,
    ).await?;

    info!("[Voiceprint] Registered: id={} name={}", id, name);

    Ok(VoiceprintDto {
        id,
        name,
        audio_path: permanent_path.to_string_lossy().to_string(),
        created_at,
    })
}

/// 删除声纹（CASCADE 删除关联 override）
#[tauri::command]
pub async fn voiceprint_delete<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, crate::AppState>,
    id: String,
) -> Result<bool, String> {
    if let Some(record) = get_voiceprint(state.db_manager.pool(), &id).await? {
        let _ = std::fs::remove_file(&record.audio_path);
    }
    delete_voiceprint(state.db_manager.pool(), &id).await?;
    info!("[Voiceprint] Deleted: id={}", id);
    Ok(true)
}

/// 获取某会议的说话人 ID → 姓名映射
#[tauri::command]
pub async fn voiceprint_get_meeting_names<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, crate::AppState>,
    meeting_id: String,
) -> Result<std::collections::HashMap<i32, String>, String> {
    let pool = state.db_manager.pool();
    let overrides = list_overrides_for_meeting(pool, &meeting_id).await?;
    let mut result = std::collections::HashMap::new();
    for ov in overrides {
        if let Some(vp) = get_voiceprint(pool, &ov.voiceprint_id).await? {
            result.insert(ov.speaker_id, vp.name);
        }
    }
    Ok(result)
}

/// 手动指派某说话人 = 某声纹
#[tauri::command]
pub async fn voiceprint_assign_speaker<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, crate::AppState>,
    meeting_id: String,
    speaker_id: i32,
    voiceprint_id: String,
) -> Result<bool, String> {
    let pool = state.db_manager.pool();
    upsert_override(pool, &meeting_id, speaker_id, &voiceprint_id, "manual").await?;
    info!("[Voiceprint] Manual assign: meeting={} speaker={} voiceprint={}", meeting_id, speaker_id, voiceprint_id);
    Ok(true)
}
```

- [ ] **Step 2: 在 lib.rs 注册命令**

修改 `d:\meetily\frontend\src-tauri\src\lib.rs` 在 `speaker_diarization_engine::commands::speaker_diarization_process,`（第 582 行）后添加：

```rust
            // Voiceprint (speaker recognition) commands
            voiceprint_engine::commands::voiceprint_list,
            voiceprint_engine::commands::voiceprint_list_meetings_with_speakers,
            voiceprint_engine::commands::voiceprint_extract_sample,
            voiceprint_engine::commands::voiceprint_register,
            voiceprint_engine::commands::voiceprint_delete,
            voiceprint_engine::commands::voiceprint_get_meeting_names,
            voiceprint_engine::commands::voiceprint_assign_speaker,
```

- [ ] **Step 3: 在 lib.rs setup hook 中初始化 VoiceprintEngine models_dir**

修改 `d:\meetily\frontend\src-tauri\src\lib.rs` 第 431 行附近（diarization set_models_directory 之后）：

```rust
            // Initialize diarization engine models dir.
            speaker_diarization_engine::commands::set_models_directory(_app.handle());
            // Initialize voiceprint engine models dir (shares CAM++ with diarization).
            {
                let vp_models_dir = _app.handle().path().resource_dir()
                    .map(|rd| rd.join("models"))
                    .unwrap_or_else(|_| PathBuf::from("sherpa-libs/models"));
                voiceprint_engine::engine::set_models_directory(vp_models_dir);
            }
```

确保 `PathBuf` 已在 lib.rs 顶部 import（若没有则添加 `use std::path::PathBuf;`）。

- [ ] **Step 4: 验证编译**

Run: `cd d:\meetily\frontend\src-tauri && cargo check`
Expected: 无错误

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/voiceprint_engine/commands.rs frontend/src-tauri/src/lib.rs
git commit -m "feat(voiceprint): 实现 Tauri 命令（从会议抓取样本/注册/列表/删除/指派）"
```

---

## Task 12: 实现 voiceprint_match_meeting 命令

**Files:**
- Modify: `d:\meetily\frontend\src-tauri\src\voiceprint_engine\commands.rs`

- [ ] **Step 1: 添加 voiceprint_match_meeting 命令**

在 `d:\meetily\frontend\src-tauri\src\voiceprint_engine\commands.rs` 末尾添加：

```rust
/// 对指定会议重新执行声纹识别
#[tauri::command]
pub async fn voiceprint_match_meeting<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, crate::AppState>,
    meeting_id: String,
) -> Result<MeetingMatchResult, String> {
    use crate::speaker_diarization_engine::commands as diarization_cmds;
    use crate::database::models::{MeetingModel, Transcript};
    use sqlx::query_as;

    let pool = state.db_manager.pool();

    // 1. 获取会议的音频文件路径
    let meeting: MeetingModel = query_as::<_, MeetingModel>("SELECT id, title, created_at, updated_at, folder_path FROM meetings WHERE id = ?")
        .bind(&meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("Failed to fetch meeting: {}", e))?
        .ok_or_else(|| format!("Meeting not found: {}", meeting_id))?;

    let folder_path = meeting.folder_path.as_ref()
        .ok_or_else(|| "Meeting has no folder_path".to_string())?;
    let audio_path = std::path::PathBuf::from(folder_path).join("audio.mp4");
    if !audio_path.exists() {
        return Err(format!("Audio file not found: {}", audio_path.display()));
    }

    // 2. 运行 diarization 获取 speaker segments
    diarization_cmds::set_models_directory(&app);
    let audio_path_str = audio_path.to_string_lossy().to_string();
    let speaker_segments = diarization_cmds::speaker_diarization_process(audio_path_str).await?;

    if speaker_segments.is_empty() {
        return Ok(MeetingMatchResult {
            matched: Vec::new(),
            unmatched_speaker_ids: Vec::new(),
        });
    }

    // 3. 解码原始音频 PCM
    let audio_path_clone = audio_path.clone();
    let decoded = tokio::task::spawn_blocking(move || decode_audio_file(&audio_path_clone))
        .await
        .map_err(|e| format!("Decode join error: {}", e))?
        .map_err(|e| format!("Failed to decode audio: {}", e))?;
    let samples = decoded.to_whisper_format();
    let sample_rate = decoded.sample_rate as f32;

    // 4. 按 speaker ID 分组，提取每个 segment 的嵌入并求平均
    let engine = get_engine()?;
    let mut cluster_embeddings: std::collections::HashMap<i32, Vec<Vec<f32>>> = std::collections::HashMap::new();
    for seg in &speaker_segments {
        let start_sample = (seg.start * sample_rate) as usize;
        let end_sample = (seg.end * sample_rate) as usize;
        if end_sample <= start_sample || end_sample > samples.len() {
            continue;
        }
        let seg_samples = &samples[start_sample..end_sample];
        let engine_clone = engine.clone();
        let seg_samples_vec = seg_samples.to_vec();
        match tokio::task::spawn_blocking(move || engine_clone.extract_embedding(&seg_samples_vec)).await {
            Ok(Ok(emb)) => {
                cluster_embeddings.entry(seg.speaker).or_default().push(emb);
            }
            Ok(Err(e)) => warn!("[Voiceprint] Failed to extract embedding for speaker {}: {}", seg.speaker, e),
            Err(e) => warn!("[Voiceprint] Join error: {}", e),
        }
    }

    // 5. 计算聚类质心
    let mut centroids: std::collections::HashMap<i32, Vec<f32>> = std::collections::HashMap::new();
    for (speaker_id, embs) in &cluster_embeddings {
        if embs.is_empty() { continue; }
        let dim = embs[0].len();
        let mut sum = vec![0.0f32; dim];
        for emb in embs {
            for (i, &v) in emb.iter().enumerate() {
                sum[i] += v;
            }
        }
        let avg: Vec<f32> = sum.iter().map(|v| v / embs.len() as f32).collect();
        // 重新归一化
        let norm: f32 = avg.iter().map(|v| v * v).sum::<f32>().sqrt();
        let normalized: Vec<f32> = if norm > 0.0 {
            avg.iter().map(|v| v / norm).collect()
        } else {
            avg
        };
        centroids.insert(*speaker_id, normalized);
    }

    // 6. 加载所有已注册声纹
    let vp_records = list_voiceprints(pool).await?;
    let vp_with_embeddings: Vec<(String, String, Vec<f32>)> = vp_records.iter()
        .map(|r| {
            let emb = super::engine::VoiceprintEngine::deserialize_embedding(&r.embedding);
            (r.id.clone(), r.name.clone(), emb)
        })
        .collect();

    // 7. 匹配
    let mut matched = Vec::new();
    let mut unmatched = Vec::new();
    for (&speaker_id, centroid) in &centroids {
        if let Some(m) = VoiceprintEngine::match_against(centroid, &vp_with_embeddings, 0.6) {
            // 写入 override (source='auto')，不覆盖 manual
            upsert_override(pool, &meeting_id, speaker_id, &m.voiceprint_id, "auto").await?;
            matched.push((speaker_id, m.name, m.similarity));
        } else {
            unmatched.push(speaker_id);
        }
    }

    // 8. 更新 transcripts 表的 speaker_name 字段
    let overrides = list_overrides_for_meeting(pool, &meeting_id).await?;
    let mut name_map: std::collections::HashMap<i32, String> = std::collections::HashMap::new();
    for ov in &overrides {
        if let Some(vp) = get_voiceprint(pool, &ov.voiceprint_id).await? {
            name_map.insert(ov.speaker_id, vp.name);
        }
    }
    for (speaker_id, name) in &name_map {
        let speaker_str = speaker_id.to_string();
        sqlx::query("UPDATE transcripts SET speaker_name = ? WHERE meeting_id = ? AND speaker = ?")
            .bind(name)
            .bind(&meeting_id)
            .bind(&speaker_str)
            .execute(pool)
            .await
            .map_err(|e| format!("Failed to update speaker_name: {}", e))?;
    }

    // 9. 更新 transcripts.json
    let transcripts: Vec<Transcript> = query_as::<_, Transcript>(
        "SELECT id, meeting_id, transcript, timestamp, summary, action_items, key_points, audio_start_time, audio_end_time, duration, speaker, speaker_name FROM transcripts WHERE meeting_id = ?"
    )
    .bind(&meeting_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to fetch transcripts: {}", e))?;

    // 写回 transcripts.json（folder_path 下）
    let transcripts_json_path = std::path::PathBuf::from(folder_path).join("transcripts.json");
    let json_segments: Vec<serde_json::Value> = transcripts.iter().map(|t| {
        serde_json::json!({
            "id": t.id,
            "text": t.transcript,
            "timestamp": t.timestamp,
            "audio_start_time": t.audio_start_time,
            "audio_end_time": t.audio_end_time,
            "duration": t.duration,
            "speaker": t.speaker.as_ref().and_then(|s| s.parse::<i32>().ok()),
            "speaker_name": t.speaker_name,
        })
    }).collect();
    let json_value = serde_json::json!(json_segments);
    if let Err(e) = std::fs::write(&transcripts_json_path, serde_json::to_string_pretty(&json_value).unwrap()) {
        warn!("[Voiceprint] Failed to write transcripts.json: {}", e);
    }

    // 10. 发射 transcript-diarized 事件（带 speaker_names）
    let speaker_names: std::collections::HashMap<i32, Option<String>> = centroids.keys().map(|&sid| {
        let name = name_map.get(&sid).cloned();
        (sid, name)
    }).collect();
    let _ = app.emit(
        "transcript-diarized",
        serde_json::json!({
            "transcripts": json_segments,
            "num_speakers": centroids.len(),
            "speaker_names": speaker_names,
        }),
    );

    info!("[Voiceprint] Meeting {} matched: {} speakers, {} unmatched", meeting_id, matched.len(), unmatched.len());
    Ok(MeetingMatchResult { matched, unmatched_speaker_ids: unmatched })
}
```

- [ ] **Step 2: 在 lib.rs 注册 voiceprint_match_meeting**

在 `voiceprint_engine::commands::voiceprint_assign_speaker,` 后添加：

```rust
            voiceprint_engine::commands::voiceprint_match_meeting,
```

- [ ] **Step 3: 验证编译**

Run: `cd d:\meetily\frontend\src-tauri && cargo check`
Expected: 无错误

- [ ] **Step 4: Commit**

```bash
git add frontend/src-tauri/src/voiceprint_engine/commands.rs frontend/src-tauri/src/lib.rs
git commit -m "feat(voiceprint): voiceprint_match_meeting 命令实现聚类质心匹配"
```

---

## Task 13: 集成声纹匹配到 run_diarization_on_segments

**Files:**
- Modify: `d:\meetily\frontend\src-tauri\src\speaker_diarization_engine\commands.rs:143-207`

- [ ] **Step 1: 修改 run_diarization_on_segments 集成声纹匹配**

修改 `d:\meetily\frontend\src-tauri\src\speaker_diarization_engine\commands.rs` 第 143-207 行的 `run_diarization_on_segments` 函数。在 `align_transcripts_with_speakers` 和写回 segments 之后、发射事件之前，插入声纹匹配逻辑：

```rust
pub async fn run_diarization_on_segments<R: Runtime>(
    app: &AppHandle<R>,
    folder: &std::path::Path,
    audio_path: &std::path::Path,
    segments: &mut [crate::api::TranscriptSegment],
) -> Result<(), String> {
    use crate::audio::common::write_transcripts_json;

    let _ = app.emit("transcript-diarization-started", serde_json::json!({}));
    set_models_directory(app);

    let audio_path_str = audio_path.to_string_lossy().to_string();
    let speaker_segments = speaker_diarization_process(audio_path_str).await?;

    if speaker_segments.is_empty() {
        info!("[Diarization] No segments returned; skipping speaker labels");
        let _ = app.emit("transcript-diarization-error", serde_json::json!({"error": "no_speech"}));
        return Ok(());
    }

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
    for (seg, aligned_chunk) in segments.iter_mut().zip(aligned.iter()) {
        seg.speaker = aligned_chunk.speaker;
    }

    // === 新增：声纹匹配（自动填充 speaker_name） ===
    let mut speaker_names: std::collections::HashMap<i32, String> = std::collections::HashMap::new();
    if let Ok(voiceprint_engine) = crate::voiceprint_engine::engine::get_engine() {
        match try_match_voiceprints(app, audio_path, &speaker_segments, &voiceprint_engine).await {
            Ok(names) => {
                for (sid, name) in &names {
                    speaker_names.insert(*sid, name.clone());
                }
                // 应用到 segments
                for seg in segments.iter_mut() {
                    if let Some(sid) = seg.speaker {
                        if let Some(name) = speaker_names.get(&sid) {
                            seg.speaker_name = Some(name.clone());
                        }
                    }
                }
            }
            Err(e) => warn!("[Diarization] Voiceprint matching failed (non-fatal): {}", e),
        }
    }

    if let Err(e) = write_transcripts_json(folder, segments) {
        warn!("[Diarization] Failed to rewrite transcripts.json: {}", e);
    }

    let num_speakers = speaker_segments.iter().map(|s| s.speaker).max().map(|m| m + 1).unwrap_or(0);
    info!("[Diarization] Success: {} speakers, {} segments labeled, {} named", num_speakers, segments.len(), speaker_names.len());

    let speaker_names_payload: std::collections::HashMap<i32, Option<String>> = (0..num_speakers)
        .map(|sid| (sid, speaker_names.get(&sid).cloned()))
        .collect();

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

/// 辅助函数：对 speaker_segments 执行声纹匹配
async fn try_match_voiceprints<R: Runtime>(
    app: &AppHandle<R>,
    audio_path: &std::path::Path,
    speaker_segments: &[super::engine::SpeakerSegment],
    voiceprint_engine: &std::sync::Arc<crate::voiceprint_engine::engine::VoiceprintEngine>,
) -> Result<std::collections::HashMap<i32, String>, String> {
    use tauri::Manager;
    use crate::voiceprint_engine::repository::list_voiceprints;

    let state = app.state::<crate::AppState>();
    let pool = state.db_manager.pool();

    // 加载所有已注册声纹
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

    // 解码原始音频
    let audio_path_clone = audio_path.to_path_buf();
    let decoded = tokio::task::spawn_blocking(move || crate::audio::decoder::decode_audio_file(&audio_path_clone))
        .await
        .map_err(|e| format!("Decode join error: {}", e))?
        .map_err(|e| format!("Failed to decode audio: {}", e))?;
    let samples = decoded.to_whisper_format();
    let sample_rate = decoded.sample_rate as f32;

    // 按 speaker ID 分组提取嵌入
    let mut cluster_embeddings: std::collections::HashMap<i32, Vec<Vec<f32>>> = std::collections::HashMap::new();
    for seg in speaker_segments {
        let start_sample = (seg.start * sample_rate) as usize;
        let end_sample = (seg.end * sample_rate) as usize;
        if end_sample <= start_sample || end_sample > samples.len() { continue; }
        let seg_samples = samples[start_sample..end_sample].to_vec();
        let engine_clone = voiceprint_engine.clone();
        match tokio::task::spawn_blocking(move || engine_clone.extract_embedding(&seg_samples)).await {
            Ok(Ok(emb)) => { cluster_embeddings.entry(seg.speaker).or_default().push(emb); }
            Ok(Err(e)) => warn!("[Voiceprint] Embedding extract failed: {}", e),
            Err(e) => warn!("[Voiceprint] Join error: {}", e),
        }
    }

    // 计算质心并匹配
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
            // 写入 override (auto)
            let _ = crate::voiceprint_engine::repository::upsert_override(pool, "", *speaker_id, &m.voiceprint_id, "auto").await;
            result.insert(*speaker_id, m.name);
        }
    }
    Ok(result)
}
```

**注意**：上面 `upsert_override` 的 `meeting_id` 参数传了空字符串占位，因为此辅助函数不直接知道 meeting_id。需要让 `run_diarization_on_segments` 接收 `meeting_id: Option<&str>` 参数，并在调用 `try_match_voiceprints` 时传入。

**修正方案**：修改 `run_diarization_on_segments` 签名增加 `meeting_id: Option<&str>`，并修改所有 3 处调用点（recording_saver.rs, import.rs, retranscription.rs）传入正确的 meeting_id。

- [ ] **Step 2: 修改 run_diarization_on_segments 签名增加 meeting_id**

将上述代码中 `try_match_voiceprints` 的空字符串替换为 `meeting_id.unwrap_or("")`，并将 `run_diarization_on_segments` 签名改为：

```rust
pub async fn run_diarization_on_segments<R: Runtime>(
    app: &AppHandle<R>,
    folder: &std::path::Path,
    audio_path: &std::path::Path,
    segments: &mut [crate::api::TranscriptSegment],
    meeting_id: Option<&str>,  // 新增
) -> Result<(), String> {
```

并在 `try_match_voiceprints` 调用处传入 `meeting_id`，函数签名也增加该参数。

- [ ] **Step 3: 修改 3 处调用点**

`recording_saver.rs` run_diarization 方法（第 507 行附近，内部调用 speaker_diarization_process 后回写，注意它不调用 run_diarization_on_segments 而是内联实现）：此路径无需修改签名，但若需要声纹匹配也要传入 meeting_id。检查 recording_saver.rs 是否调用 run_diarization_on_segments。

Grep 确认：`recording_saver.rs` 不调用 `run_diarization_on_segments`，而是内联实现。需要单独处理。

修改 `import.rs:719` 调用点：
```rust
// 修改前
if let Err(e) = crate::speaker_diarization_engine::commands::run_diarization_on_segments(
    &app, &folder, &audio_path, &mut segments
).await { ... }

// 修改后
if let Err(e) = crate::speaker_diarization_engine::commands::run_diarization_on_segments(
    &app, &folder, &audio_path, &mut segments, Some(&meeting_id)
).await { ... }
```

修改 `retranscription.rs:545` 调用点类似，传入 `Some(&meeting_id)`。

对 `recording_saver.rs::run_diarization`：在写回 segments 和发射事件前，复制 `try_match_voiceprints` 逻辑（或提取为公共函数）。最简方案：让 `run_diarization` 也调用一个共享的 `apply_voiceprint_matching` 函数。

**简化处理**：将 `try_match_voiceprints` 改为 `pub` 并在 `recording_saver.rs::run_diarization` 中也调用一次（传入 meeting_id，从 metadata 获取）。

- [ ] **Step 4: 验证编译**

Run: `cd d:\meetily\frontend\src-tauri && cargo check`
Expected: 无错误

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/speaker_diarization_engine/commands.rs frontend/src-tauri/src/audio/import.rs frontend/src-tauri/src/audio/retranscription.rs frontend/src-tauri/src/audio/recording_saver.rs
git commit -m "feat(diarization): 集成声纹匹配，自动填充 speaker_name"
```

---

## Task 14: 前端类型添加 speaker_name 字段

**Files:**
- Modify: `d:\meetily\frontend\src\types\index.ts:19-21, 36, 113-116`

- [ ] **Step 1: 添加 speaker_name 字段**

修改 `d:\meetily\frontend\src\types\index.ts`：

第 19-21 行（TranscriptSegmentData）：
```typescript
// 修改前
speaker?: number;
}

// 修改后
speaker?: number;
speaker_name?: string;
}
```

第 36 行附近（TranscriptUpdate）：
```typescript
speaker?: number;
speaker_name?: string;
```

第 113-116 行（Transcript）：
```typescript
speaker?: number;
speaker_name?: string;
}
```

- [ ] **Step 2: 验证编译**

Run: `cd d:\meetily\frontend && pnpm tsc --noEmit`
Expected: 无错误

- [ ] **Step 3: Commit**

```bash
git add frontend/src/types/index.ts
git commit -m "feat(types): Transcript 类型添加 speaker_name 字段"
```

---

## Task 15: 修复 TranscriptContext 合并 + 接收 speaker_name

**Files:**
- Modify: `d:\meetily\frontend\src\contexts\TranscriptContext.tsx:394-413`

- [ ] **Step 1: 应用 Task 6 的合并修改**

按 Task 6 Step 1 的代码修改 `d:\meetily\frontend\src\contexts\TranscriptContext.tsx` 第 394-413 行。确保 `diarized` 映射包含 `speaker_name: seg.speaker_name ?? undefined`，并使用合并模式 `setTranscripts(prev => ...)`。

- [ ] **Step 2: 验证编译**

Run: `cd d:\meetily\frontend && pnpm tsc --noEmit`
Expected: 无错误

- [ ] **Step 3: Commit**

```bash
git add frontend/src/contexts/TranscriptContext.tsx
git commit -m "fix(transcript): 分离结果合并而非替换，保留分离期间新到达的 transcript"
```

---

## Task 16: VirtualizedTranscriptView 显示姓名

**Files:**
- Modify: `d:\meetily\frontend\src\components\VirtualizedTranscriptView.tsx:80-82, 85-100`

- [ ] **Step 1: 修改 getSpeakerLabel 接收 speaker_name**

修改 `d:\meetily\frontend\src\components\VirtualizedTranscriptView.tsx` 第 80-82 行：

```typescript
// 修改前
function getSpeakerLabel(speaker: number): string {
    return `说话人 ${speaker + 1}`;
}

// 修改后
function getSpeakerLabel(speaker: number, speakerName?: string): string {
    if (speakerName) return speakerName;
    return `说话人 ${speaker + 1}`;
}
```

- [ ] **Step 2: 修改 TranscriptSegment 组件接收 speaker_name**

修改 `d:\meetily\frontend\src\components\VirtualizedTranscriptView.tsx` 第 85-100 行附近（TranscriptSegment memo 组件）：

```typescript
const TranscriptSegment = memo(function TranscriptSegment({
    id,
    timestamp,
    text,
    confidence,
    speaker,
    speakerName,  // 新增
    isStreaming,
    showConfidence,
}: {
    id: string;
    timestamp: number;
    text: string;
    confidence?: number;
    speaker?: number;
    speakerName?: string;  // 新增
    isStreaming: boolean;
    showConfidence: boolean;
}) {
    // ... 在使用 getSpeakerLabel 的地方传入 speakerName
    // 例如：const label = speaker !== undefined ? getSpeakerLabel(speaker, speakerName) : null;
```

搜索文件中所有 `getSpeakerLabel(` 调用点，传入 `speakerName` 参数。

- [ ] **Step 3: 在父组件渲染时传入 speaker_name**

搜索 `TranscriptSegment` 组件的渲染位置（在 VirtualizedTranscriptView 内部 map segments 的地方），传入 `speakerName={seg.speaker_name}`：

```tsx
<TranscriptSegment
    key={seg.id}
    id={seg.id}
    // ... 其他 props
    speaker={seg.speaker}
    speakerName={seg.speaker_name}
    // ...
/>
```

- [ ] **Step 4: 验证编译**

Run: `cd d:\meetily\frontend && pnpm tsc --noEmit`
Expected: 无错误

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/VirtualizedTranscriptView.tsx
git commit -m "feat(ui): 转录视图显示声纹匹配的姓名"
```

---

## Task 17: 实现 VoiceprintSettings 组件

**Files:**
- Create: `d:\meetily\frontend\src\components\VoiceprintSettings.tsx`

- [ ] **Step 1: 创建 VoiceprintSettings 组件（从会议抓取样本）**

写入 `d:\meetily\frontend\src\components\VoiceprintSettings.tsx`：

```tsx
'use client';

import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from './ui/button';
import { Input } from './ui/input';
import { Label } from './ui/label';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from './ui/select';
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter } from './ui/dialog';
import { Fingerprint, Plus, Trash2, Play, Loader2, AlertCircle } from 'lucide-react';
import { toast } from 'sonner';

interface VoiceprintDto {
    id: string;
    name: string;
    audio_path: string;
    created_at: string;
}

interface MeetingWithSpeakersDto {
    meeting_id: string;
    meeting_title: string;
    created_at: string;
    speaker_ids: number[];
}

interface ExtractedSampleDto {
    audio_path: string;
    duration_seconds: number;
    segment_start: number;
    segment_end: number;
}

function formatTime(seconds: number): string {
    const m = Math.floor(seconds / 60);
    const s = Math.floor(seconds % 60);
    return `${m.toString().padStart(2, '0')}:${s.toString().padStart(2, '0')}`;
}

export function VoiceprintSettings() {
    const [voiceprints, setVoiceprints] = useState<VoiceprintDto[]>([]);
    const [meetings, setMeetings] = useState<MeetingWithSpeakersDto[]>([]);
    const [loading, setLoading] = useState(true);
    const [showAddModal, setShowAddModal] = useState(false);
    const [newName, setNewName] = useState('');
    const [selectedMeetingId, setSelectedMeetingId] = useState<string>('');
    const [selectedSpeakerId, setSelectedSpeakerId] = useState<string>('');
    const [extracting, setExtracting] = useState(false);
    const [extractedSample, setExtractedSample] = useState<ExtractedSampleDto | null>(null);
    const [isSaving, setIsSaving] = useState(false);
    const [playingId, setPlayingId] = useState<string | null>(null);

    const loadVoiceprints = useCallback(async () => {
        try {
            const list = await invoke<VoiceprintDto[]>('voiceprint_list');
            setVoiceprints(list);
        } catch (e) {
            toast.error('加载声纹列表失败: ' + String(e));
        } finally {
            setLoading(false);
        }
    }, []);

    const loadMeetings = useCallback(async () => {
        try {
            const list = await invoke<MeetingWithSpeakersDto[]>('voiceprint_list_meetings_with_speakers');
            setMeetings(list);
        } catch (e) {
            toast.error('加载会议列表失败: ' + String(e));
        }
    }, []);

    useEffect(() => {
        loadVoiceprints();
        loadMeetings();
    }, [loadVoiceprints, loadMeetings]);

    // 当选择会议时，重置说话人选择
    useEffect(() => {
        setSelectedSpeakerId('');
        setExtractedSample(null);
    }, [selectedMeetingId]);

    const selectedMeeting = meetings.find(m => m.meeting_id === selectedMeetingId);

    const handleExtract = async () => {
        if (!selectedMeetingId || !selectedSpeakerId) {
            toast.error('请先选择会议和说话人');
            return;
        }
        setExtracting(true);
        setExtractedSample(null);
        try {
            const sample = await invoke<ExtractedSampleDto>('voiceprint_extract_sample', {
                meetingId: selectedMeetingId,
                speakerId: parseInt(selectedSpeakerId)
            });
            setExtractedSample(sample);
            toast.success(`已抓取 ${sample.duration_seconds.toFixed(1)} 秒样本`);
        } catch (e) {
            toast.error('抓取样本失败: ' + String(e));
        } finally {
            setExtracting(false);
        }
    };

    const handleSave = async () => {
        if (!newName.trim()) { toast.error('请输入姓名'); return; }
        if (!extractedSample) { toast.error('请先抓取样本'); return; }
        setIsSaving(true);
        try {
            await invoke('voiceprint_register', {
                name: newName.trim(),
                audioPath: extractedSample.audio_path
            });
            toast.success(`声纹「${newName.trim()}」已注册`);
            // 重置 modal 状态
            setShowAddModal(false);
            setNewName('');
            setSelectedMeetingId('');
            setSelectedSpeakerId('');
            setExtractedSample(null);
            await loadVoiceprints();
        } catch (e) {
            toast.error('保存失败: ' + String(e));
        } finally {
            setIsSaving(false);
        }
    };

    const handleDelete = async (id: string, name: string) => {
        if (!confirm(`确定删除声纹「${name}」吗？关联的说话人指派也将被清除。`)) return;
        try {
            await invoke('voiceprint_delete', { id });
            toast.success(`已删除「${name}」`);
            await loadVoiceprints();
        } catch (e) {
            toast.error('删除失败: ' + String(e));
        }
    };

    const handlePlay = async (audioPath: string, id: string) => {
        if (playingId === id) { setPlayingId(null); return; }
        setPlayingId(id);
        try {
            const bytes = await invoke<number[]>('read_audio_file', { path: audioPath });
            const audioCtx = new (window.AudioContext || (window as any).webkitAudioContext)();
            const buffer = await audioCtx.decodeAudioData(new Uint8Array(bytes).buffer);
            const source = audioCtx.createBufferSource();
            source.buffer = buffer;
            source.connect(audioCtx.destination);
            source.onended = () => setPlayingId(null);
            source.start();
        } catch (e) {
            toast.error('播放失败: ' + String(e));
            setPlayingId(null);
        }
    };

    return (
        <div className="space-y-6">
            <div className="bg-white rounded-xl border border-gray-200/70 p-6 shadow-sm hover:shadow-md transition-shadow duration-300">
                <div className="flex items-start gap-3 mb-4">
                    <div className="w-10 h-10 rounded-lg bg-indigo-50 flex items-center justify-center flex-shrink-0">
                        <Fingerprint className="w-5 h-5 text-indigo-600" />
                    </div>
                    <div className="flex-1">
                        <h3 className="text-lg font-semibold text-gray-900">声纹管理</h3>
                        <p className="text-sm text-gray-500 mt-0.5">从已有会议抓取说话人声纹，注册后自动识别显示姓名</p>
                    </div>
                    <Button
                        onClick={() => { setShowAddModal(true); loadMeetings(); }}
                        className="bg-blue-600 hover:bg-blue-700"
                        disabled={meetings.length === 0}
                    >
                        <Plus className="w-4 h-4 mr-1" /> 添加声纹
                    </Button>
                </div>

                {meetings.length === 0 && (
                    <div className="mb-4 p-3 bg-yellow-50/70 border border-yellow-100 rounded text-sm text-yellow-800 flex items-start gap-2">
                        <AlertCircle className="w-4 h-4 mt-0.5 flex-shrink-0" />
                        <span>暂无已完成说话人分离的会议。请先录制或导入会议并完成分离。</span>
                    </div>
                )}

                {loading ? (
                    <div className="flex items-center justify-center py-12 text-gray-400">
                        <Loader2 className="w-6 h-6 animate-spin" />
                    </div>
                ) : voiceprints.length === 0 ? (
                    <div className="text-center py-12 text-gray-400">
                        <Fingerprint className="w-12 h-12 mx-auto mb-3 opacity-50" />
                        <p>暂无已注册声纹</p>
                        <p className="text-xs mt-1">点击右上角"添加声纹"从已有会议抓取</p>
                    </div>
                ) : (
                    <div className="space-y-2">
                        {voiceprints.map(vp => (
                            <div key={vp.id} className="flex items-center justify-between p-4 border border-gray-200/70 rounded-lg hover:bg-gray-50/50 transition-colors">
                                <div className="flex-1">
                                    <div className="font-medium text-gray-900">{vp.name}</div>
                                    <div className="text-xs text-gray-400 mt-0.5">
                                        创建于 {new Date(vp.created_at).toLocaleString('zh-CN')}
                                    </div>
                                </div>
                                <div className="flex items-center gap-2">
                                    <Button
                                        variant="outline"
                                        size="sm"
                                        onClick={() => handlePlay(vp.audio_path, vp.id)}
                                        disabled={playingId === vp.id}
                                    >
                                        {playingId === vp.id ? <Loader2 className="w-4 h-4 animate-spin" /> : <Play className="w-4 h-4" />}
                                    </Button>
                                    <Button
                                        variant="outline"
                                        size="sm"
                                        onClick={() => handleDelete(vp.id, vp.name)}
                                        className="text-red-600 hover:text-red-700 hover:bg-red-50"
                                    >
                                        <Trash2 className="w-4 h-4" />
                                    </Button>
                                </div>
                            </div>
                        ))}
                    </div>
                )}
            </div>

            <Dialog open={showAddModal} onOpenChange={(open) => {
                setShowAddModal(open);
                if (!open) {
                    setNewName('');
                    setSelectedMeetingId('');
                    setSelectedSpeakerId('');
                    setExtractedSample(null);
                }
            }}>
                <DialogContent className="sm:max-w-md">
                    <DialogHeader>
                        <DialogTitle>注册新声纹</DialogTitle>
                    </DialogHeader>
                    <div className="space-y-4 py-4">
                        <div className="space-y-2">
                            <Label htmlFor="vp-name">姓名</Label>
                            <Input
                                id="vp-name"
                                value={newName}
                                onChange={(e) => setNewName(e.target.value)}
                                placeholder="如：张三"
                                maxLength={30}
                            />
                        </div>

                        <div className="space-y-2">
                            <Label>从会议抓取声纹样本</Label>
                            <Select value={selectedMeetingId} onValueChange={setSelectedMeetingId}>
                                <SelectTrigger>
                                    <SelectValue placeholder="选择会议（已完成说话人分离）" />
                                </SelectTrigger>
                                <SelectContent>
                                    {meetings.map(m => (
                                        <SelectItem key={m.meeting_id} value={m.meeting_id}>
                                            {m.meeting_title} ({new Date(m.created_at).toLocaleDateString('zh-CN')})
                                        </SelectItem>
                                    ))}
                                </SelectContent>
                            </Select>
                        </div>

                        {selectedMeeting && (
                            <div className="space-y-2">
                                <Label>说话人</Label>
                                <Select value={selectedSpeakerId} onValueChange={setSelectedSpeakerId}>
                                    <SelectTrigger>
                                        <SelectValue placeholder="选择说话人" />
                                    </SelectTrigger>
                                    <SelectContent>
                                        {selectedMeeting.speaker_ids.map(sid => (
                                            <SelectItem key={sid} value={sid.toString()}>
                                                说话人 {sid + 1}
                                            </SelectItem>
                                        ))}
                                    </SelectContent>
                                </Select>
                            </div>
                        )}

                        {selectedMeetingId && selectedSpeakerId && (
                            <Button
                                onClick={handleExtract}
                                disabled={extracting}
                                variant="outline"
                                className="w-full"
                            >
                                {extracting ? (
                                    <><Loader2 className="w-4 h-4 mr-2 animate-spin" /> 正在抓取样本...</>
                                ) : extractedSample ? (
                                    <>↻ 重新抓取样本</>
                                ) : (
                                    <>抓取样本</>
                                )}
                            </Button>
                        )}

                        {extractedSample && (
                            <div className="border-2 border-green-200 bg-green-50/50 rounded-lg p-4 space-y-2">
                                <div className="text-green-700 font-medium text-sm">
                                    ✓ 已抓取样本: {extractedSample.duration_seconds.toFixed(1)} 秒
                                </div>
                                <div className="text-xs text-gray-600">
                                    片段区间: {formatTime(extractedSample.segment_start)} - {formatTime(extractedSample.segment_end)}
                                </div>
                                <Button
                                    variant="outline"
                                    size="sm"
                                    onClick={() => handlePlay(extractedSample.audio_path, 'temp')}
                                    disabled={playingId === 'temp'}
                                >
                                    {playingId === 'temp' ? <Loader2 className="w-4 h-4 animate-spin" /> : <Play className="w-4 h-4 mr-1" />}
                                    播放样本
                                </Button>
                            </div>
                        )}

                        <div className="flex items-start gap-2 text-xs text-gray-500 bg-blue-50/70 border border-blue-100 rounded p-2">
                            <AlertCircle className="w-3 h-3 mt-0.5 flex-shrink-0 text-blue-600" />
                            <span>系统自动从该说话人最长的语音片段截取（3-10秒），样本质量与实际识别场景一致</span>
                        </div>
                    </div>
                    <DialogFooter>
                        <Button variant="outline" onClick={() => setShowAddModal(false)}>
                            取消
                        </Button>
                        <Button
                            onClick={handleSave}
                            disabled={isSaving || !newName.trim() || !extractedSample}
                        >
                            {isSaving && <Loader2 className="w-4 h-4 mr-1 animate-spin" />}
                            保存声纹
                        </Button>
                    </DialogFooter>
                </DialogContent>
            </Dialog>
        </div>
    );
}
```

- [ ] **Step 2: 验证编译**

Run: `cd d:\meetily\frontend && pnpm tsc --noEmit`
Expected: 无错误。若 `read_audio_file` 命令不存在，需先确认后端是否提供（搜索 `read_audio_file` 命令注册）。若不存在，移除播放功能或新增该命令。

- [ ] **Step 3: Commit**

```bash
git add frontend/src/components/VoiceprintSettings.tsx
git commit -m "feat(ui): 声纹设置组件（从会议抓取样本/注册/删除/试听）"
```

---

## Task 18: 设置页添加声纹标签页

**Files:**
- Modify: `d:\meetily\frontend\src\app\settings\page.tsx:4, 16-21, 119-127`

- [ ] **Step 1: 添加 Fingerprint import**

修改 `d:\meetily\frontend\src\app\settings\page.tsx` 第 4 行：

```typescript
// 修改前
import { ArrowLeft, Settings2, Mic, Database as DatabaseIcon, SparkleIcon } from 'lucide-react';

// 修改后
import { ArrowLeft, Settings2, Mic, Database as DatabaseIcon, SparkleIcon, Fingerprint } from 'lucide-react';
```

- [ ] **Step 2: 添加 VoiceprintSettings import**

在第 11 行后添加：

```typescript
import { VoiceprintSettings } from '@/components/VoiceprintSettings';
```

- [ ] **Step 3: 在 TABS 数组添加声纹标签**

修改第 16-21 行：

```typescript
const TABS = [
  { value: 'general', label: '通用', icon: Settings2 },
  { value: 'recording', label: '录音', icon: Mic },
  { value: 'Transcriptionmodels', label: '转录', icon: DatabaseIcon },
  { value: 'summaryModels', label: '摘要', icon: SparkleIcon },
  { value: 'voiceprint', label: '声纹', icon: Fingerprint },
] as const;
```

- [ ] **Step 4: 添加 TabsContent**

在第 125-127 行（summaryModels TabsContent 后）添加：

```tsx
<TabsContent value="voiceprint">
  <VoiceprintSettings />
</TabsContent>
```

- [ ] **Step 5: 验证编译**

Run: `cd d:\meetily\frontend && pnpm tsc --noEmit`
Expected: 无错误

- [ ] **Step 6: Commit**

```bash
git add frontend/src/app/settings/page.tsx
git commit -m "feat(ui): 设置页新增声纹标签页"
```

---

## Task 19: 版本号升级到 1.3.0

**Files:**
- Modify: `d:\meetily\frontend\package.json`
- Modify: `d:\meetily\frontend\src-tauri\Cargo.toml`
- Modify: `d:\meetily\frontend\src-tauri\tauri.conf.json`

- [ ] **Step 1: 修改 package.json 版本**

在 `d:\meetily\frontend\package.json` 中找到 `"version"` 字段，改为 `"1.3.0"`。

- [ ] **Step 2: 修改 Cargo.toml 版本**

在 `d:\meetily\frontend\src-tauri\Cargo.toml` 中找到 `version = "..."` 改为 `version = "1.3.0"`。

- [ ] **Step 3: 修改 tauri.conf.json 版本**

在 `d:\meetily\frontend\src-tauri\tauri.conf.json` 中找到 `"version"` 字段改为 `"1.3.0"`。

- [ ] **Step 4: Commit**

```bash
git add frontend/package.json frontend/src-tauri/Cargo.toml frontend/src-tauri/tauri.conf.json
git commit -m "chore: 版本号升级到 1.3.0"
```

---

## Task 20: 构建并生成 setup.exe

**Files:** 无（仅构建）

- [ ] **Step 1: 验证后端编译**

Run: `cd d:\meetily\frontend\src-tauri && cargo check`
Expected: 无错误

- [ ] **Step 2: 验证前端编译**

Run: `cd d:\meetily\frontend && pnpm tsc --noEmit`
Expected: 无错误

- [ ] **Step 3: 构建 Tauri 应用**

Run: `cd d:\meetily\frontend && pnpm tauri build`
Expected: 构建成功，输出 setup.exe 到 `d:\meetily\build-target\release\bundle\nsis\新际审会议助手_1.3.0_x64-setup.exe`

- [ ] **Step 4: 验证 setup.exe 存在**

Run: `dir d:\meetily\build-target\release\bundle\nsis\新际审会议助手_1.3.0_x64-setup.exe`
Expected: 文件存在，大小约 240MB（因包含 CAM++ 模型）

- [ ] **Step 5: Commit 构建产物路径记录**

无需 commit 二进制，但记录路径供用户测试：
- 安装包: `d:\meetily\build-target\release\bundle\nsis\新际审会议助手_1.3.0_x64-setup.exe`

---

## 测试清单（手动验证）

完成所有 Task 后，执行以下手动验证：

1. **转录模型未就绪修复**：全新安装 → 不进入设置 → 直接点击开始录音 → 应能正常开始
2. **CAM++ 模型加载**：录音停止后查看日志，应看到 `[Diarization] Loading models: pyannote=..., camplus=...`
3. **同一人分裂缓解**：录制单人 3 分钟讲话 → 分离结果应为 1 个说话人（而非多个）
4. **声纹注册**：设置 → 声纹 → 添加 → 输入姓名 → 选择会议和说话人 → 抓取样本 → 保存 → 列表显示
5. **声纹识别**：注册声纹后 → 录制含该人的对话 → 分离完成后应显示姓名而非"说话人 1"
6. **手动指派**：会议详情 → 对未识别的说话人手动指派 → 姓名立即显示
7. **重新识别**：注册新声纹后 → 历史会议点击"重新识别说话人" → 姓名回填
8. **吞文字修复**：长录音（>10 分钟）→ 分离期间观察无文字丢失
9. **删除声纹**：删除声纹 → 关联会议的姓名回退为"说话人 N"

---

## 自我审查

**Spec 覆盖**：
- ✅ Task 1: CAM++ 替换（Spec 3.1-3.4）
- ✅ Task 2: 阈值 0.4（Spec 4.2）
- ✅ Task 3: 默认 provider 修复（Spec 2.2 修复 1）
- ✅ Task 4: parakeet case + 错误显示（Spec 2.2 修复 3、4）
- ✅ Task 5: 配置持久化（Spec 2.2 修复 2、5）
- ✅ Task 6: 前端合并修复（Spec 5.2 修复 1）
- ✅ Task 7: speaker_name 字段（Spec 1.4）
- ✅ Task 8: 数据库表（Spec 1.1）
- ✅ Task 9-10: voiceprint_engine（Spec 1.2.1）
- ✅ Task 11: Tauri 命令（Spec 1.2.2）
- ✅ Task 12: voiceprint_match_meeting（Spec 1.2.2 流程）
- ✅ Task 13: 分离管道集成（Spec 1.3）
- ✅ Task 14: 前端类型（Spec 1.4）
- ✅ Task 15: TranscriptContext（Spec 5.2 修复 1）
- ✅ Task 16: VirtualizedTranscriptView（Spec 1.5.3）
- ✅ Task 17: VoiceprintSettings（Spec 1.5.2）
- ✅ Task 18: 设置页标签（Spec 1.5.1）
- ⚠️ Spec 1.5.4 会议详情页"重新识别"按钮 + 手动指派 UI：未单独成 Task，建议作为 Task 21 补充

**补充 Task 21: 会议详情页手动指派 UI**

（见下方补充任务）

---

## Task 21: 会议详情页"重新识别说话人"按钮 + 手动指派

**Files:**
- Modify: 会议详情页组件（需先定位）

- [ ] **Step 1: 定位会议详情页组件**

Run: Glob `d:\meetily\frontend\src\app\meeting\**\*.tsx` 或 `d:\meetily\frontend\src\components\MeetingDetails\**\*.tsx`

找到会议详情页主组件，识别"说话人 N"标签渲染位置。

- [ ] **Step 2: 添加"重新识别说话人"按钮**

在会议详情页转录列表上方添加按钮：

```tsx
<Button
    variant="outline"
    size="sm"
    onClick={async () => {
        try {
            toast.loading('正在重新识别说话人...', { id: 'rematch' });
            const result = await invoke('voiceprint_match_meeting', { meetingId });
            toast.dismiss('rematch');
            toast.success(`识别完成：${result.matched.length} 位已识别，${result.unmatched_speaker_ids.length} 位未识别`);
            // 触发 transcripts 重新加载
            await reloadTranscripts();
        } catch (e) {
            toast.dismiss('rematch');
            toast.error('重新识别失败: ' + String(e));
        }
    }}
>
    <Fingerprint className="w-4 h-4 mr-1" /> 重新识别说话人
</Button>
```

- [ ] **Step 3: 在每个"说话人 N"标签旁添加手动指派下拉**

修改说话人标签渲染位置，添加 Select 下拉：

```tsx
<Select onValueChange={async (voiceprintId) => {
    await invoke('voiceprint_assign_speaker', {
        meetingId,
        speakerId: speaker,
        voiceprintId
    });
    toast.success('已指派');
    await reloadTranscripts();
}}>
    <SelectTrigger className="h-6 w-auto text-xs">
        <SelectValue placeholder={speakerName || `说话人 ${speaker + 1}`} />
    </SelectTrigger>
    <SelectContent>
        {voiceprints.map(vp => (
            <SelectItem key={vp.id} value={vp.id}>{vp.name}</SelectItem>
        ))}
    </SelectContent>
</Select>
```

需在组件顶部加载声纹列表：`const [voiceprints, setVoiceprints] = useState<VoiceprintDto[]>([]);` 并 `useEffect` 调用 `voiceprint_list`。

- [ ] **Step 4: 验证编译**

Run: `cd d:\meetily\frontend && pnpm tsc --noEmit`
Expected: 无错误

- [ ] **Step 5: Commit**

```bash
git add <会议详情页组件路径>
git commit -m "feat(ui): 会议详情页添加重新识别按钮和手动指派下拉"
```

---

## 最终构建

完成所有 Task（1-21）后，回到 Task 20 执行构建。
