# 声纹注册与识别 + 转录/分离问题修复 设计文档

**日期**: 2026-07-23
**状态**: 待评审
**作者**: 协作设计（用户 + 助手）

---

## 背景与目标

meetily 当前已实现说话人分离（基于 sherpa-onnx 的 `OfflineSpeakerDiarization`，使用 pyannote 分割 + 3D-Speaker ERes2Net 嵌入），但存在以下问题：

1. **新功能需求**：用户希望在设置页面录制 5 秒音频注册声纹，标注姓名后，后续录音中识别到该人员声音时直接显示姓名，而非"说话人 1/2"。
2. **转录模型未就绪 bug**：点击开始录音仍提示"转录模型未就绪"，即使模型已下载。
3. **ERes2Net 速度过慢**：需切换为更快的 CAM++ 模型。
4. **同一人被识别为不同说话人**：聚类阈值过严 + ERes2Net 准确度不足。
5. **分离过程吞文字**：分离完成后部分文本消失。

本设计同时解决上述 5 个问题。

---

## 用户决策（已确认）

| 决策点 | 选择 |
|---|---|
| 声纹样本来源 | 从用户已上传/录制的会议音频中抓取（必须已完成说话人分离） |
| 抓取方式 | 用户选择会议 → 列出该会议已识别的说话人 → 选择说话人 → 系统自动抓取该说话人最长的 5 秒 segment 作为样本 |
| 识别到已注册声纹的显示方式 | 完全替换为姓名，未识别仍显示"说话人 N" |
| 注册新声纹后处理历史会议 | 用户手动触发，提供"重新识别说话人"按钮 |
| 匹配失败兜底 | 支持手动指派说话人关联到已注册声纹 |
| 声纹匹配阈值（CAM++） | 0.6（起始值，可在实现时微调） |
| 手动指派作用域 | 声纹注册全局（"张三"跨会议复用），指派关系按会议存储（因 speaker_id 是每会议独立的聚类编号） |

---

## 架构概览

```
┌─────────────────────────────────────────────────────────────┐
│                       设置页面（前端）                        │
│  通用 │ 录音 │ 转录 │ 摘要 │ 声纹（新增）                      │
│                          │                                  │
│                          ▼                                  │
│          VoiceprintSettings.tsx                            │
│   ┌────────────────────────────────────┐                    │
│   │ 声纹列表 + 添加/删除/试听           │                    │
│   │ 录制 5 秒样本 → 标注姓名 → 保存     │                    │
│   └────────────────────────────────────┘                    │
└──────────────────────────┬──────────────────────────────────┘
                           │ Tauri 命令
                           ▼
┌─────────────────────────────────────────────────────────────┐
│                    后端 voiceprint_engine（新增）            │
│  ┌─────────────────────┐  ┌──────────────────────────────┐  │
│  │ VoiceprintEngine    │  │ SpeakerEmbeddingExtractor     │  │
│  │ - extract(samples)  │←─│  (CAM++ 模型，与分离共享)      │  │
│  │ - match(embedding)  │  └──────────────────────────────┘  │
│  │ - list/delete       │                                    │
│  └──────────┬──────────┘                                    │
│             │                                               │
│             ▼                                               │
│  ┌──────────────────────────────────────────────────────┐  │
│  │ SQLite: voiceprints 表 + meeting_speaker_overrides   │  │
│  └──────────────────────────────────────────────────────┘  │
└──────────────────────────┬──────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────┐
│              分离管道集成（speaker_diarization_engine）       │
│  1. pyannote 分割 + CAM++ 嵌入                              │
│  2. FastClustering（threshold: 0.4）                        │
│  3. align_transcripts_with_speakers（保持不变）             │
│  4. 新增：按聚类计算质心嵌入 → 匹配已注册声纹                │
│  5. 新增：应用 meeting_speaker_overrides 手动指派           │
│  6. 发射 transcript-diarized 事件（含 speaker_name 字段）   │
└─────────────────────────────────────────────────────────────┘
```

---

## 第 1 部分：声纹注册与识别（新功能）

### 1.1 数据库 Schema

新增两个表（通过 sqlx 迁移文件）：

```sql
-- migrations/20260723000001_create_voiceprints.sql
CREATE TABLE voiceprints (
    id TEXT PRIMARY KEY,                  -- UUID
    name TEXT NOT NULL,                   -- 用户标注的姓名，如"张三"
    embedding BLOB NOT NULL,              -- 序列化的 Vec<f32>，约 192/256 维（CAM++）
    audio_path TEXT NOT NULL,             -- 5 秒 WAV 样本路径（保留在 app_data/voiceprints/）
    created_at TEXT NOT NULL              -- ISO 8601
);

CREATE INDEX idx_voiceprints_name ON voiceprints(name);

-- migrations/20260723000002_create_meeting_speaker_overrides.sql
CREATE TABLE meeting_speaker_overrides (
    meeting_id TEXT NOT NULL,             -- meetings.id 外键
    speaker_id INTEGER NOT NULL,          -- 分离聚类 ID（0,1,2...）
    voiceprint_id TEXT NOT NULL,          -- voiceprints.id 外键
    source TEXT NOT NULL DEFAULT 'manual',-- 'manual' | 'auto'（区分自动匹配与手动指派）
    PRIMARY KEY (meeting_id, speaker_id),
    FOREIGN KEY (voiceprint_id) REFERENCES voiceprints(id) ON DELETE CASCADE
);
```

**设计说明**：
- `voiceprints.embedding` 用 `bincode` 或裸 `f32` 字节序序列化存储
- `meeting_speaker_overrides` 同时承载自动匹配结果（`source='auto'`）和手动指派（`source='manual'`），手动优先级更高
- `ON DELETE CASCADE`：删除声纹时自动清除关联的 override

### 1.2 后端 voiceprint_engine 模块

**位置**: `d:\meetily\frontend\src-tauri\src\voiceprint_engine\`

```
voiceprint_engine/
├── mod.rs              # 模块声明
├── engine.rs           # VoiceprintEngine 核心
├── commands.rs         # Tauri 命令
└── repository.rs       # 数据库操作
```

#### 1.2.1 VoiceprintEngine（engine.rs）

```rust
pub struct VoiceprintEngine {
    extractor: RwLock<Option<SpeakerEmbeddingExtractor>>,
    models_dir: PathBuf,
}

impl VoiceprintEngine {
    /// 共用 CAM++ 模型路径（与 diarization engine 一致）
    pub fn new(models_dir: PathBuf) -> Self { ... }

    /// 从音频样本提取嵌入向量
    /// samples: 16kHz mono f32 PCM
    /// 返回归一化的嵌入向量（L2 范数 = 1）
    pub fn extract_embedding(&self, samples: &[f32]) -> Result<Vec<f32>, String> { ... }

    /// 计算两个嵌入向量的余弦相似度
    pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 { ... }

    /// 将输入嵌入与所有已注册声纹比对，返回最佳匹配
    /// threshold: 默认 0.6，低于此值返回 None
    pub fn match_against(
        &self,
        embedding: &[f32],
        voiceprints: &[(String, String, Vec<f32>)], // (id, name, embedding)
        threshold: f32,
    ) -> Option<VoiceprintMatch> { ... }
}

pub struct VoiceprintMatch {
    pub voiceprint_id: String,
    pub name: String,
    pub similarity: f32,
}
```

**关键点**：
- `SpeakerEmbeddingExtractor` 来自 sherpa-onnx crate，与 diarization engine 共用同一个 CAM++ 模型文件
- 嵌入向量做 L2 归一化后存储，余弦相似度简化为点积
- 阈值 0.6 作为起始值（CAM++ 在 3D-Speaker 官方推荐 0.5–0.7 之间）

#### 1.2.2 Tauri 命令（commands.rs）

| 命令 | 参数 | 返回 | 说明 |
|---|---|---|---|
| `voiceprint_list` | - | `Vec<VoiceprintDto>` | 列出所有已注册声纹 |
| `voiceprint_list_meetings_with_speakers` | - | `Vec<MeetingWithSpeakersDto>` | 列出所有已完成说话人分离的会议，含每会议的 speaker_id 列表，供声纹注册时选择 |
| `voiceprint_extract_sample` | `meeting_id, speaker_id` | `ExtractedSampleDto` | 从指定会议的指定说话人 segments 中自动选取最长的一段（≥3秒，≤10秒），切出对应音频片段保存为 WAV，返回路径和时长 |
| `voiceprint_register` | `name: String, audio_path: String` | `VoiceprintDto` | 解码音频 → 提取嵌入 → 存库（含将 temp 文件移动到 `voiceprints/<id>.wav`）→ 返回 |
| `voiceprint_delete` | `id: String` | `bool` | 删除声纹 + 关联 override（CASCADE） |
| `voiceprint_match_meeting` | `meeting_id: String` | `MeetingMatchResult` | 对指定会议重新执行声纹识别（手动触发） |
| `voiceprint_assign_speaker` | `meeting_id, speaker_id, voiceprint_id` | `bool` | 手动指派某说话人 = 某声纹 |
| `voiceprint_get_meeting_names` | `meeting_id` | `HashMap<i32, String>` | 获取某会议的说话人 ID → 姓名映射（供前端渲染） |

**`voiceprint_match_meeting` 流程**：
1. 读取会议音频文件路径，解码为 16kHz mono f32 PCM
2. 运行 diarization 获取 speaker segments（复用现有 `speaker_diarization_process`），得到 `Vec<SpeakerSegment { start, end, speaker }>`
3. 按 speaker ID 分组，对每个聚类的所有 segment：用 `start/end` 时间从原始 PCM 切片 → 调用 `extract_embedding` 得到该 segment 的嵌入向量
4. 对每个聚类的所有 segment 嵌入求**简单算术平均**（非时间加权），得到聚类质心
5. 用质心匹配已注册声纹（阈值 0.6），选择相似度最高者
6. 匹配成功的写入 `meeting_speaker_overrides`（`source='auto'`）；若已存在 `source='manual'` 记录则跳过（手动优先）
7. 重新发射 `transcript-diarized` 事件，payload 含 `speaker_names: {0: "张三", 1: null}`
8. 更新 `transcripts.json` 和 SQLite 的 `transcripts.speaker_name` 字段（新增列，见 1.4）

#### 1.2.3 声纹样本抓取实现

**`voiceprint_list_meetings_with_speakers`**：查询 `meetings` 表 join `transcripts` 表（`speaker IS NOT NULL`），返回每个有 speaker 标注的会议及其 distinct speaker_id 列表。

**`voiceprint_extract_sample` 流程**：
1. 查询指定 meeting_id 的 transcripts，过滤出 `speaker = speaker_id` 的所有 segment，按 duration 降序排序
2. 选取 duration 最长的一段（若 > 10 秒则截取前 10 秒；若 < 3 秒则拼接相邻同 speaker segment 直到 ≥3 秒或用尽）
3. 读取会议音频文件（`folder_path/audio.mp4`），解码为 16kHz mono PCM
4. 按 segment 的 `audio_start_time` / `audio_end_time` 切片
5. 保存为 WAV 到 `app_data/voiceprints/temp/<uuid>.wav`
6. 返回 `{ audio_path, duration_seconds, segment_start, segment_end }`

**优势**：无需用户额外录音，直接利用已有会议内容；样本质量与实际识别场景一致。

### 1.3 分离管道集成

修改 `speaker_diarization_engine/engine.rs` 和 `recording_saver.rs`：

**`engine.rs` 改动**：
- `diarize()` 方法返回的 `SpeakerSegment` 增加 `embedding: Vec<f32>` 字段（每个 segment 的嵌入向量），供后续聚类质心计算使用
- 实际上 sherpa-onnx 的 `OfflineSpeakerDiarization::process()` 内部已计算嵌入但未暴露。**方案调整**：不修改 sherpa-onnx 调用，而是在 voiceprint matching 阶段独立重新提取每个 segment 的嵌入（性能可接受，因为只在手动触发时执行）

**`recording_saver.rs::run_diarization` 改动**：
- 在 `align_transcripts_with_speakers` 之后，调用 `voiceprint_engine.match_against_meeting(meeting_id, segments)`
- 若有匹配，将 `speaker_name` 写入 `TranscriptSegment`（新增字段）
- 发射的 `transcript-diarized` 事件 payload 增加 `speaker_names: HashMap<i32, Option<String>>`

### 1.4 TranscriptSegment 数据结构变更

**后端**（`recording_saver.rs`, `api.rs`, `common.rs`）：
```rust
pub struct TranscriptSegment {
    // ... 现有字段 ...
    pub speaker: Option<i32>,
    pub speaker_name: Option<String>,  // 新增：来自声纹匹配或手动指派
}
```

**数据库**（新增迁移）：
```sql
-- migrations/20260723000003_add_speaker_name.sql
ALTER TABLE transcripts ADD COLUMN speaker_name TEXT;
```

**前端类型**（`types/index.ts`）：
```typescript
export interface Transcript {
    // ... 现有字段 ...
    speaker?: number;
    speaker_name?: string;  // 新增
}
```

### 1.5 前端实现

#### 1.5.1 设置页面新标签页

**修改** `d:\meetily\frontend\src\app\settings\page.tsx`：

在 `TABS` 数组末尾添加第 5 个标签：
```typescript
const TABS = [
  { value: 'general', label: '通用', icon: Settings2 },
  { value: 'recording', label: '录音', icon: Mic },
  { value: 'Transcriptionmodels', label: '转录', icon: DatabaseIcon },
  { value: 'summaryModels', label: '摘要', icon: SparkleIcon },
  { value: 'voiceprint', label: '声纹', icon: Fingerprint },  // 新增
] as const;
```

`Fingerprint` 图标来自 `lucide-react`。

在 `<TabsContent>` 区域添加：
```tsx
<TabsContent value="voiceprint" className="...">
  <VoiceprintSettings />
</TabsContent>
```

#### 1.5.2 VoiceprintSettings 组件

**新建** `d:\meetily\frontend\src\components\VoiceprintSettings.tsx`：

**UI 结构**：
```
┌─────────────────────────────────────────────┐
│ 🎙️ 声纹管理                                 │
│ 注册声纹后，系统会自动识别已知说话人          │
├─────────────────────────────────────────────┤
│ [+ 添加声纹]                                │
├─────────────────────────────────────────────┤
│ 已注册声纹 (3)                              │
│ ┌─────────────────────────────────────────┐ │
│ │ 张三        ▶ 试听    🗑️ 删除           │ │
│ │ 创建于 2026-07-23 10:30                 │ │
│ ├─────────────────────────────────────────┤ │
│ │ 李四        ▶ 试听    🗑️ 删除           │ │
│ │ ...                                     │ │
│ └─────────────────────────────────────────┘ │
└─────────────────────────────────────────────┘
```

**添加声纹 Modal**：
```
┌─────────────────────────────────────────────┐
│ 注册新声纹                              [×] │
├─────────────────────────────────────────────┤
│ 姓名: [_______________________]             │
│                                             │
│ 从已有会议抓取声纹样本:                     │
│ 会议: [▼ 选择会议            ]              │
│ 说话人: [▼ 说话人 1          ]              │
│                                             │
│ ┌─────────────────────────────────────────┐ │
│ │ 已抓取样本: 5.2 秒 (00:23-00:28)        │ │
│ │ [▶ 播放] [↻ 重新抓取]                   │ │
│ └─────────────────────────────────────────┘ │
│                                             │
│ 提示: 样本从该说话人最长的语音片段自动截取   │
│                                             │
│              [取消]  [保存声纹]              │
└─────────────────────────────────────────────┘
```

**交互流程**：
1. 点击"添加声纹" → 打开 Modal
2. 输入姓名
3. 从下拉选择会议（仅显示已完成说话人分离的会议）
4. 从下拉选择说话人（显示"说话人 1/2/..."或已匹配姓名）
5. 系统自动调用 `voiceprint_extract_sample` 抓取最长 segment
6. 显示样本时长和起止时间，可试听
7. 点击"保存声纹" → 调用 `voiceprint_register` → 成功 toast → 列表刷新

#### 1.5.3 转录视图显示姓名

**修改** `d:\meetily\frontend\src\components\VirtualizedTranscriptView.tsx`：

```typescript
function getSpeakerLabel(speaker: number, speakerName?: string): string {
    if (speakerName) return speakerName;
    return `说话人 ${speaker + 1}`;
}
```

通过 `VoiceprintContext`（新建）或直接从 transcript 对象的 `speaker_name` 字段读取姓名。

#### 1.5.4 会议详情页：重新识别 + 手动指派

**修改**会议详情页（`MeetingDetails` 组件）：
- 新增"重新识别说话人"按钮 → 调用 `invoke('voiceprint_match_meeting', { meetingId })`
- 每个"说话人 N"旁边新增下拉菜单 → 调用 `voiceprint_assign_speaker` 手动指派

---

## 第 2 部分：转录模型未就绪 Bug 修复（Task 2）

### 2.1 根因汇总

| # | Bug | 文件 | 行号 |
|---|---|---|---|
| 1 | 默认 provider 错误返回 `parakeet` | `api/api.rs` | 642 |
| 2 | TranscriptSettings 不持久化配置 | `TranscriptSettings.tsx` | 全文 |
| 3 | useRecordingStart 缺 `parakeet` case | `useRecordingStart.ts` | 70-104 |
| 4 | 错误被吞掉，用户看不到真实原因 | `useRecordingStart.ts` | 74-80 |

### 2.2 修复方案

**修复 1 — `api/api.rs:642`**：
```rust
// 修改前
Ok(None) => {
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

**修复 2 — TranscriptSettings.tsx 持久化**：
在 provider/model 变更时（`onValueChange` 回调）立即调用 `api_save_transcript_config`，无需防抖（用户操作频率低）。

**修复 3 — useRecordingStart.ts 添加 parakeet case**：
```typescript
case 'parakeet': {
    try {
        await invoke<string>('parakeet_validate_model_ready');
        return true;
    } catch (e) {
        console.error('[Recording] Parakeet model not ready:', e);
        return false;
    }
}
```

**修复 4 — 显示真实错误**：
```typescript
// 修改前
catch (e) {
    console.log('[Recording] Sherpa-ASR model not ready:', e);
    return false;
}

// 修改后
catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    console.error('[Recording] Sherpa-ASR model validation failed:', msg);
    lastValidationError = msg;  // 存储最近错误
    return false;
}
```

在 toast 中显示 `lastValidationError` 而非通用"未就绪"消息。

**修复 5 — 统一默认值**：将 5 处默认值统一为 `sherpaAsr` / SenseVoice，单一数据源。

---

## 第 3 部分：ERes2Net → CAM++（Task 3）

### 3.1 模型替换

**修改** `d:\meetily\frontend\src-tauri\src\speaker_diarization_engine\engine.rs`：

```rust
// 修改前
pub const ERES2NET_MODEL_FILE: &str = "3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx";

// 修改后
pub const CAMPLUS_MODEL_FILE: &str = "3dspeaker_speech_campplus_sv_zh-cn_16k-common.onnx";
```

同步更新 `eres2net_model_path()` → `camplus_model_path()`，以及 `is_ready()` 中的日志字符串。

### 3.2 模型下载

CAM++ 模型下载 URL（来自 k2-fsa/sherpa-onnx releases 的 `speaker-recongition-models` tag）：
- 直接: `https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_campplus_sv_zh-cn_16k-common.onnx`
- 镜像 1: `https://gh.api.99988866.xyz/https://github.com/...`
- 镜像 2: `https://ghproxy.net/https://github.com/...`
- 镜像 3: `https://mirror.ghproxy.com/https://github.com/...`

下载脚本位置：`d:\meetily\frontend\src-tauri\scripts\download-camplus-model.ps1`（开发期一次性下载，放入 `sherpa-libs/models/speaker-diarization/`）

### 3.3 tauri.conf.json 更新

**修改** `d:\meetily\frontend\src-tauri\tauri.conf.json` 的 `bundle.resources`：

```json
// 修改前
"sherpa-libs/models/speaker-diarization/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx": "models/speaker-diarization/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx"

// 修改后
"sherpa-libs/models/speaker-diarization/3dspeaker_speech_campplus_sv_zh-cn_16k-common.onnx": "models/speaker-diarization/3dspeaker_speech_campplus_sv_zh-cn_16k-common.onnx"
```

### 3.4 清理旧模型

删除本地 `sherpa-libs/models/speaker-diarization/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx`（约 40MB）。

---

## 第 4 部分：同一人识别为不同说话人（Task 4）

### 4.1 根因

1. ERes2Net 中文准确度低于 CAM++ → Task 3 替换解决
2. `FastClusteringConfig.threshold = 0.5` 过严 → 同一人嵌入稍有差异即分到不同聚类

### 4.2 修复方案

**修改** `engine.rs:116-119`：

```rust
// 修改前
let clustering = FastClusteringConfig {
    num_clusters: 0,
    threshold: 0.5,
};

// 修改后
let clustering = FastClusteringConfig {
    num_clusters: 0,
    threshold: 0.4,  // 降低阈值，更宽松地合并同一说话人
};
```

**说明**：`threshold` 是余弦距离阈值，值越小越容易合并。0.4 是 sherpa-onnx 社区常见推荐值。

### 4.3 声纹注册兜底

即使聚类仍偶发分裂，声纹注册提供权威覆盖：
- 若两个聚类都匹配到同一已注册声纹（相似度 > 0.6），自动合并为同一姓名
- 用户手动指派可强制覆盖

---

## 第 5 部分：分离过程吞文字（Task 5）

### 5.1 排查方向

**嫌疑 1 — 后端过滤**：
检查 `run_diarization` 是否在发射 `transcript-diarized` 事件前过滤了某些 segment。当前代码（`recording_saver.rs:543-547`）克隆全部 `transcript_segments`，**不应**丢失。但需确认 `import.rs` 和 `retranscription.rs` 路径是否一致。

**嫌疑 2 — 前端替换而非合并**：
`TranscriptContext.tsx:408` 的 `setTranscripts(diarized)` **完全替换**数组。若分离期间有新的实时 transcript 到达（通过 `transcript-update` 事件），这些新 transcript 会被覆盖丢失。

**嫌疑 3 — VirtualizedTranscriptView 过滤**：
需检查该组件是否过滤 `speaker == null` 的 segment。

### 5.2 修复方案

**修复 1 — 前端合并而非替换**：
```typescript
// 修改前
setTranscripts(diarized);

// 修改后：按 id 合并，保留分离期间新到达的 transcript
setTranscripts(prev => {
    const map = new Map(prev.map(t => [t.id, t]));
    for (const d of diarized) {
        map.set(d.id, d);  // 覆盖已存在的，保留未在 diarized 中的
    }
    return Array.from(map.values()).sort((a, b) => a.sequence_id - b.sequence_id);
});
```

**修复 2 — 确保后端发射全部 segment**：
审计 `run_diarization`（recording_saver.rs）、`run_diarization_on_segments`（commands.rs）、`import.rs`、`retranscription.rs` 所有路径。当前 `recording_saver.rs:543-547` 已正确克隆全部 segments，无需修改；但需确认其他三条路径是否一致。若发现任何路径按 `speaker.is_some()` 过滤，则移除该过滤。

**修复 3 — 前端不过滤 null speaker**：
若 `VirtualizedTranscriptView` 存在过滤，移除或改为显示"未分配说话人"。

---

## 测试策略

### 后端单元测试

- `voiceprint_engine::engine`:
  - `test_extract_embedding_returns_normalized_vector`（L2 范数 ≈ 1.0）
  - `test_cosine_similarity_identical_vectors`（= 1.0）
  - `test_match_against_below_threshold_returns_none`
  - `test_match_against_picks_highest_similarity`
- `speaker_diarization_engine::engine`:
  - 现有 6 个对齐测试保持通过
  - 新增 `test_threshold_04_merges_close_embeddings`（待实现时验证）

### 集成测试

- 注册声纹 → 录制含该说话人的会议 → 验证 transcript 的 `speaker_name` 字段正确
- 删除声纹 → 验证 `meeting_speaker_overrides` 级联删除
- 手动指派 → 验证 override 优先于自动匹配
- CAM++ 模型加载 → `is_ready()` 返回 true
- 默认 provider 修复 → 全新数据库 `api_get_transcript_config` 返回 sherpaAsr

### 手动验证

- 安装新版本，不保存任何转录配置 → 点击开始录音 → 应能正常开始（不再提示"未就绪"）
- 注册 2 个声纹 → 录制 2 人对话 → 验证显示姓名且无文字丢失
- 历史会议点击"重新识别说话人" → 验证姓名正确回填
- 测试同一人长录音 → 验证不再分裂为多个说话人

---

## 风险与回退

| 风险 | 缓解 |
|---|---|
| CAM++ 模型在 Windows 加载失败 | 保留旧 ERes2Net 文件直到验证通过；`is_ready()` 失败时 diarization 优雅降级 |
| 声纹匹配误识别（张冠李戴） | 阈值 0.6 偏保守；提供手动指派兜底；`source='auto'` 标记可被手动覆盖 |
| 5 秒录音质量差导致注册失败 | UI 提示安静环境；录音后可试听重录；注册时验证嵌入非零 |
| 聚类阈值 0.4 导致不同人合并 | 声纹注册提供反向纠正（手动指派拆分） |

---

## 不在本期范围

- 实时声纹识别（录音过程中即时标注姓名）—— 当前仅后处理
- 跨会议声纹模型微调/再训练
- 声纹导入导出
- 多语言姓名支持特殊处理

---

## 文件清单（预计修改/新增）

**新增**:
- `frontend/src-tauri/src/voiceprint_engine/{mod,engine,commands,repository}.rs`
- `frontend/src-tauri/migrations/20260723000001_create_voiceprints.sql`
- `frontend/src-tauri/migrations/20260723000002_create_meeting_speaker_overrides.sql`
- `frontend/src-tauri/migrations/20260723000003_add_speaker_name.sql`
- `frontend/src/components/VoiceprintSettings.tsx`
- `frontend/src/contexts/VoiceprintContext.tsx`
- `frontend/src-tauri/scripts/download-camplus-model.ps1`

**修改**:
- `frontend/src-tauri/src/lib.rs`（注册新模块和命令）
- `frontend/src-tauri/src/speaker_diarization_engine/engine.rs`（CAM++ + threshold + embedding 暴露）
- `frontend/src-tauri/src/audio/recording_saver.rs`（集成声纹匹配）
- `frontend/src-tauri/src/audio/import.rs`（同上）
- `frontend/src-tauri/src/audio/retranscription.rs`（同上）
- `frontend/src-tauri/src/api/api.rs`（默认 provider 修复）
- `frontend/src-tauri/src/audio/recording_commands.rs`（如需）
- `frontend/src-tauri/src/audio/transcription/engine.rs`（统一默认值）
- `frontend/src-tauri/src/sherpa_asr_engine/commands.rs`（统一默认值）
- `frontend/src-tauri/tauri.conf.json`（CAM++ bundle）
- `frontend/src-tauri/Cargo.toml`（如需新依赖）
- `frontend/src/app/settings/page.tsx`（新标签页）
- `frontend/src/components/TranscriptSettings.tsx`（持久化配置）
- `frontend/src/components/VirtualizedTranscriptView.tsx`（显示姓名 + 不过滤）
- `frontend/src/contexts/TranscriptContext.tsx`（合并而非替换）
- `frontend/src/hooks/useRecordingStart.ts`（parakeet case + 错误显示）
- `frontend/src/types/index.ts`（speaker_name 字段）
- `frontend/src-tauri/src/audio/common.rs`（TranscriptSegment.speaker_name）
- `frontend/src-tauri/src/database/models.rs`（speaker_name 字段）
