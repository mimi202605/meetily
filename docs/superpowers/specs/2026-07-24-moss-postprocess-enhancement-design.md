# MOSS-Transcribe-Diarize 部分功能移植增强设计

**日期**: 2026-07-24
**状态**: Approved
**来源**: 移植 [OpenMOSS/MOSS-Transcribe-Diarize](https://github.com/OpenMOSS/MOSS-Transcribe-Diarize) 的后处理与导出算法

## 1. 背景与目标

### 1.1 背景

当前 Meetily 系统的转写流水线为：

```
sherpa-onnx 实时转写 → chunks（text + audio_start/end_time）
                          ↓
录音后 diarization → SpeakerSegment（start, end, speaker:i32）
                          ↓
align_transcripts_with_speakers() → chunk.speaker = 最长重叠段
```

存在 4 个不足：
1. sherpa-onnx 按 VAD 切分，产生大量 1-2 秒短句碎片
2. 单个 chunk 可能超过 10 秒，不利阅读
3. diarization 段与 chunk 时间可能交错重叠
4. 无字幕导出能力（仅文本/JSON）

### 1.2 目标

移植 MOSS-Transcribe-Diarize 仓库的**纯算法部分**（后处理、字幕导出、热词提示机制）到 Rust，**不引入 Python 依赖，不改动 sherpa-onnx/diarization 核心引擎，不引入 MOSS 模型本体**。

### 1.3 不做的事（YAGNI）

- ❌ 不引入 Python/PyTorch/MOSS 模型本体
- ❌ 不实现流式 transcript 解析器（用户已排除）
- ❌ 不改动 sherpa-onnx / diarization 引擎核心
- ❌ 不做字幕烧录（ffmpeg burn-in），仅导出文本文件
- ❌ 不做实时字幕预览 UI（仅导出功能）

## 2. MOSS 源码分类

| MOSS 模块 | 功能 | 依赖 | 可移植性 |
|---|---|---|---|
| `subtitle/postprocess.py` | 字幕段规范化（合并/拆分/修复重叠） | 纯 Python | ✅ 纯 Rust 重写 |
| `subtitle/export.py` | SRT/ASS/JSON 字幕导出 | 纯 Python | ✅ 纯 Rust 重写 |
| `subtitle/layout.py` | 重叠字幕 lane 分配 | 纯 Python | ✅ 纯 Rust 重写 |
| `subtitle/models.py` | 字幕段/样式数据结构 | 纯 Python | ✅ 纯 Rust 重写 |
| `examples/prompts.md` | 提示词工程（热词/说话人模式） | 文档 | ✅ 可直接借鉴 |
| `inference_utils.py` | 音频加载/推理工具 | torch/transformers/av | ❌ 不移植 |
| `modeling_*.py` / `processing_*.py` | MOSS 模型本体 | torch | ❌ 不移植 |

## 3. 架构总览

在现有 Rust/Tauri 架构中新增 3 个独立模块：

```
frontend/src-tauri/src/
├── speaker_diarization_engine/   (现有，不动)
│   └── engine.rs                 align_transcripts_with_speakers()
│            ↓
├── transcript_postprocess/       (新增 P0 - 移植 MOSS postprocess.py)
│   └── mod.rs                    normalize() / fix_overlaps() / merge_adjacent() / split_long()
│            ↓
├── subtitle_export/              (新增 P0 - 移植 MOSS export.py + layout.py)
│   └── mod.rs                    export_srt() / export_ass() / export_json() / assign_lanes()
│
├── hotword_correction/           (新增 P1 - 借鉴 MOSS prompts.md + 复用 llm_client)
│   └── mod.rs                    correct_transcript() (调用现有 summary/llm_client)
│
└── summary/llm_client.rs         (现有，复用，不动)
```

3 个模块相互独立，可并行/顺序实施。依赖关系：`transcript_postprocess` → `subtitle_export`（共用数据类型）→ `hotword_correction`（依赖 postprocess + llm_client）。

## 4. 统一数据结构

现有系统存在 speaker 类型不一致：
- DB `Transcript.speaker: Option<String>`
- diarization `SpeakerSegment.speaker: i32`

为避免侵入现有结构，新增统一中间类型：

```rust
// transcript_postprocess/mod.rs
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessableSegment {
    pub id: String,
    pub start: f32,          // 秒
    pub end: f32,            // 秒
    pub speaker: String,     // 统一用 String（如 "S01"、"0"、"1"）
    pub text: String,
}
```

**转换适配器**：
- 从 `TranscriptChunkForAlignment`（diarization engine）→ `ProcessableSegment`：`speaker` 字段 `i32` → `String::parse`，空则 `"S00"`
- 从 DB `Transcript` → `ProcessableSegment`：直接字段映射，`speaker` 为空则 `"S00"`
- 从 `ProcessableSegment` → 字幕导出格式：直接使用

## 5. 模块 1：transcript_postprocess（P0）

### 5.1 移植来源

MOSS `subtitle/postprocess.py` 的 4 个核心函数。

### 5.2 接口

```rust
pub struct NormalizeConfig {
    pub min_duration: f32,   // 默认 1.0s
    pub max_duration: f32,   // 默认 6.0s
    pub max_chars: usize,    // 默认 24（中文友好）
    pub merge_gap: f32,      // 默认 0.3s
}

impl Default for NormalizeConfig {
    fn default() -> Self {
        Self { min_duration: 1.0, max_duration: 6.0, max_chars: 24, merge_gap: 0.3 }
    }
}

pub fn normalize(segments: &mut Vec<ProcessableSegment>, config: &NormalizeConfig)
pub fn fix_overlaps(segments: &mut Vec<ProcessableSegment>, min_duration: f32)
pub fn merge_adjacent(segments: &mut Vec<ProcessableSegment>, merge_gap: f32, max_chars: usize)
pub fn split_long_segments(segments: &mut Vec<ProcessableSegment>, min_duration: f32, max_duration: f32, max_chars: usize)
```

### 5.3 算法细节（与 MOSS 一致）

#### fix_overlaps
游标从 0 开始，对每段：
- `start = max(seg.start, cursor)`
- `end = max(seg.end, start + min_duration)`
- 更新 `cursor = end`

保证时间单调不重叠，每段至少 `min_duration`。

#### merge_adjacent
遍历相邻段，满足以下全部条件则合并：
- `previous.speaker == segment.speaker`
- `0 <= gap = segment.start - previous.end <= merge_gap`
- 合并后文本字数 `<= 2 * max_chars`

合并后：`end = max(previous.end, segment.end)`，文本用 `_join_text` 拼接。

#### split_long_segments
对每段，若 `duration > max_duration` 且 `text.len() > max_chars`：
1. 调用 `split_text` 按标点切分
2. 按字数比例分配时间，最后一段取 `segment.end`
3. 每子段至少 `min_duration`

`split_text` 切分规则：
- 遇标点（`。！？!?；;，、 `）且当前累积 ≥ `max_chars/2` 时切断
- 累积 ≥ `max_chars` 时强制切断
- 切分后紧凑合并：相邻 chunk 合并后字数 ≤ `max_chars` 则合并

#### _join_text
- ASCII 文本之间加空格
- CJK 文本之间不加空格

#### normalize
依次执行：
1. `_prepare_segments`：strip 文本、跳过空段、start ≥ 0、end ≥ start、按 (start, end) 排序
2. `fix_overlaps`
3. `merge_adjacent`
4. `split_long_segments`
5. `fix_overlaps`（再次修复，因为 split 可能产生新重叠）

### 5.4 集成点

在 `speaker_diarization_engine/engine.rs` 的 `align_transcripts_with_speakers` 返回后调用：

```rust
let mut chunks = align_transcripts_with_speakers(chunks, &segments);
// 新增：MOSS 风格后处理
let mut processable: Vec<ProcessableSegment> = chunks.iter().map(Into::into).collect();
transcript_postprocess::normalize(&mut processable, &NormalizeConfig::default());
// 转换回 chunks 或直接用于后续导出
```

**注意**：后处理是可选的、非破坏性的。原始 chunk 数据不修改，后处理结果用于显示和导出。

## 6. 模块 2：subtitle_export（P0）

### 6.1 移植来源

MOSS `subtitle/export.py` + `subtitle/layout.py` + `subtitle/models.py`。

### 6.2 接口

```rust
pub struct AssStyle {
    pub font_name: String,                    // 默认 "Noto Sans CJK SC"
    pub font_size: Option<usize>,             // None 则按视频高度 4.5% 计算
    pub alignment: usize,                     // 默认 2（底部居中）
    pub margin_v: usize,                      // 默认 56
    pub outline: usize,                       // 默认 3
    pub shadow: usize,                        // 默认 1
    pub show_speaker: bool,                   // 默认 true
    pub speaker_colors: bool,                 // 默认 true
    pub primary_color: String,                // 默认 "&H00FFFFFF"
    pub outline_color: String,                // 默认 "&H00000000"
    pub back_color: String,                   // 默认 "&H64000000"
    pub speaker_names: HashMap<String, String>, // S01 → "张三"
}

impl Default for AssStyle { /* 上述默认值 */ }

pub fn export_srt(
    segments: &[ProcessableSegment],
    show_speaker: bool,
    speaker_names: &HashMap<String, String>,
) -> String

pub fn export_ass(
    segments: &[ProcessableSegment],
    style: &AssStyle,
    video_width: usize,   // 默认 1920
    video_height: usize,  // 默认 1080
) -> String

pub fn export_json(segments: &[ProcessableSegment]) -> String

fn assign_overlap_lanes(segments: &[ProcessableSegment]) -> Vec<usize>
```

### 6.3 MOSS 亮点保留

#### 8 色说话人调色板（ASS）
```rust
const SPEAKER_COLORS: &[&str] = &[
    "&H00FFFFFF", "&H005BE7FF", "&H0086F28F", "&H00BBA7FF",
    "&H0000D7FF", "&H00FFB56B", "&H00FF8EDB", "&H00D8D8D8",
];
```
按 speaker 排序，循环分配颜色，为每个 speaker 生成独立 ASS Style。

#### assign_overlap_lanes
- Lane 0 是底部基准行，更大的 lane 号向上堆叠
- 按 (start, end, 原索引) 排序
- 对每段：找到第一个 `lane_end <= start` 的 lane 复用，否则新建 lane
- 用于 ASS 的 `MarginV` 计算：`margin_v = style.margin_v + lane * font_size`

#### 时间格式化
- SRT: `HH:MM:SS,mmm`（逗号分隔毫秒）
- ASS: `H:MM:SS.cc`（点分隔厘秒）

#### ASS 特殊字符转义
- `\` → `\\`
- `{` → `(`
- `}` → `)`
- `\n` → `\N`

#### 显示文本
- `show_speaker=true` 时：`{speaker_name_or_id}: {text}`
- `speaker_names` 优先，否则用原始 speaker id

### 6.4 Tauri Command

```rust
#[tauri::command]
async fn export_subtitle(
    app: AppHandle,
    state: State<...>,
    meeting_id: String,
    format: String,                        // "srt" | "ass" | "json"
    show_speaker: Option<bool>,
    speaker_names: Option<HashMap<String, String>>,
    apply_postprocess: Option<bool>,       // 是否先 normalize，默认 true
) -> Result<String, String>
```

**数据流**：
1. DB 查询该 meeting 的所有 transcripts（按 audio_start_time 排序）
2. 转换为 `Vec<ProcessableSegment>`
3. 若 `apply_postprocess != Some(false)`，调用 `normalize`
4. 按 format 调用对应导出函数
5. 返回字符串，前端触发文件下载

### 6.5 前端集成

会议详情页新增"导出字幕"按钮，下拉菜单三选项：SRT / ASS / JSON。
- SRT/ASS 触发文件下载
- JSON 可选直接预览或下载

## 7. 模块 3：hotword_correction（P1）

### 7.1 借鉴来源

MOSS `examples/prompts.md` 的热词提示 prompt 模式。

### 7.2 设计

**不新增 LLM 客户端**，复用 `summary/llm_client.rs` 的多 provider 支持（OpenAI/Claude/Groq/Ollama/OpenRouter/CustomOpenAI）。

```rust
pub async fn correct_transcript_with_hotwords(
    transcript_segments: &[ProcessableSegment],
    hotwords: &[String],
    llm_config: &LlmConfig,
) -> Result<Vec<ProcessableSegment>, String>
```

### 7.3 Prompt 设计

借鉴 MOSS 热词 prompt 模式：

```
请修正以下会议转写文本中的专有名词错误。

热词提示：{热词1, 热词2, 热词3}

转写文本（按段）：
[S01] (0.48-1.66) 欢迎各位参加审计会议
[S02] (1.66-3.20) 今天讨论审计法执行情况
...

要求：
1. 仅修正专有名词（人名/机构/法规/术语），使其匹配热词
2. 不改变语义和句子结构
3. 保持 [Sxx] (start-end) text 格式输出
4. 修正词用热词中的正确写法
```

输入：将 `ProcessableSegment` 序列化为上述格式。
输出：LLM 返回文本，解析回 `ProcessableSegment`（复用简单的行解析，不引入完整流式解析器）。

### 7.4 热词存储

新增 DB 表 `hotwords`：

```sql
CREATE TABLE hotwords (
    id TEXT PRIMARY KEY,
    word TEXT NOT NULL,
    category TEXT,                          -- 可选分类（人名/机构/法规/术语）
    scope TEXT NOT NULL DEFAULT 'global',   -- 'global' 或 meeting_id
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
```

- 用户在设置页维护热词列表（全局或按会议）
- 新增 migration: `20260724000001_create_hotwords.sql`

### 7.5 Tauri Commands

```rust
#[tauri::command]
async fn get_hotwords(app, state, scope: Option<String>) -> Result<Vec<Hotword>, String>

#[tauri::command]
async fn add_hotword(app, state, word: String, category: Option<String>, scope: Option<String>) -> Result<String, String>

#[tauri::command]
async fn delete_hotword(app, state, id: String) -> Result<(), String>

#[tauri::command]
async fn correct_transcript_with_hotwords(
    app, state, meeting_id: String,
) -> Result<Vec<ProcessableSegment>, String>
```

`correct_transcript_with_hotwords` 为异步长任务，通过 Tauri event 上报进度：
- `hotword-correction-start`：开始
- `hotword-correction-progress`：LLM 调用中
- `hotword-correction-complete`：完成，携带结果
- `hotword-correction-error`：失败

### 7.6 前端集成

- 设置页新增"热词管理"区块（复用现有 SettingTabs）
- 会议详情页新增"智能修正"按钮（可选触发，非自动）
- 修正结果可预览对比，用户确认后才写入 DB

### 7.7 应用场景

审计会议中的：
- 被审计单位名称（如"XX集团有限公司"）
- 法规条文（如《审计法》《财政违法行为处罚处分条例》）
- 专业术语（如"实质性测试""符合性测试"）

sherpa-onnx 可能识别错误，LLM 配合热词可批量修正。

## 8. 错误处理

| 模块 | 错误场景 | 处理方式 |
|---|---|---|
| transcript_postprocess | 空段、NaN 时间 | `normalize` 跳过空段；NaN 时间视为 0.0 并 warn |
| subtitle_export | DB 查询失败 | 向上传播 `Err(String)` |
| subtitle_export | meeting 无 transcript | 返回空字符串，前端提示"无转写内容" |
| hotword_correction | LLM 调用失败 | 返回原始 segments + warn 日志，不中断 |
| hotword_correction | LLM 输出解析失败 | 回退原始文本，warn 日志 |
| hotword_correction | 无热词配置 | 直接返回原始 segments，跳过 LLM 调用 |

## 9. 测试策略

### 9.1 单元测试（与 MOSS 行为对齐）

`transcript_postprocess`：
- `test_fix_overlaps_no_overlap`：无重叠时不变
- `test_fix_overlaps_with_overlap`：重叠段被游标推进
- `test_fix_overlaps_min_duration`：短段被延长到 min_duration
- `test_merge_adjacent_same_speaker`：同 speaker + 小 gap 合并
- `test_merge_different_speaker_no_merge`：不同 speaker 不合并
- `test_merge_gap_too_large`：gap > merge_gap 不合并
- `test_split_long_by_punctuation`：按标点切分
- `test_split_short_unchanged`：短段不变
- `test_split_text_cjk_punctuation`：中文标点切分
- `test_join_text_ascii_space`：ASCII 间加空格
- `test_join_text_cjk_no_space`：中文间不加空格
- `test_normalize_full_pipeline`：完整流水线

`subtitle_export`：
- `test_export_srt_format`：SRT 时间格式、序号、空行
- `test_export_srt_with_speaker`：包含 speaker 前缀
- `test_export_ass_speaker_colors`：每个 speaker 独立 Style
- `test_export_ass_escape`：特殊字符转义
- `test_export_json_structure`：JSON 结构正确
- `test_assign_lanes_no_overlap`：无重叠时全 lane 0
- `test_assign_lanes_with_overlap`：重叠时分层
- `test_format_srt_time`：时间格式化
- `test_format_ass_time`：时间格式化

### 9.2 集成测试

- 从 DB 读取真实 meeting → normalize → export_srt，验证输出可被 ffmpeg 解析
- 热词修正：mock LLM 响应，验证专有名词被替换、格式保持

## 10. 实施顺序

1. **P0 - transcript_postprocess**（模块 1）：无依赖，先做
2. **P0 - subtitle_export**（模块 2）：依赖模块 1 的 `ProcessableSegment`
3. **P1 - hotword_correction**（模块 3）：依赖模块 1 + 现有 llm_client

每个模块独立提交，便于 review 和回滚。

## 11. 依赖关系

- **新增 Rust 依赖**：无（全部用标准库 + serde）
- **复用现有**：`summary/llm_client.rs`、`database/` 模块、Tauri command 体系
- **不动**：sherpa-onnx、diarization、whisper、parakeet 引擎

## 12. 与 MOSS 模型本体的关系

本方案**不使用 MOSS 模型**，仅借鉴其后处理与导出算法。未来若 MOSS 发布 ONNX 量化版或用户配置远程 MOSS API，模块 1-2 的后处理/导出可直接复用（它们与模型无关，只处理结构化 segment）。
