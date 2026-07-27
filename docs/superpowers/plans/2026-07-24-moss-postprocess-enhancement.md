# MOSS Postprocess Enhancement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port MOSS-Transcribe-Diarize's pure-algorithm subsystems (transcript postprocessing, subtitle export, hotword correction) to Rust without introducing Python dependencies or the MOSS model.

**Architecture:** Three independent Rust modules under `frontend/src-tauri/src/` that operate on a unified `ProcessableSegment` type. Module 1 (`transcript_postprocess`) ports MOSS's normalize/merge/split algorithms. Module 2 (`subtitle_export`) ports SRT/ASS/JSON export with speaker colors and overlap lanes. Module 3 (`hotword_correction`) reuses the existing `summary/llm_client.rs` with a MOSS-inspired hotword prompt. All modules are pure Rust, no new external dependencies.

**Tech Stack:** Rust (std + serde), Tauri 2.x commands, existing SQLite (sqlx), existing `summary/llm_client.rs`.

**Spec:** `docs/superpowers/specs/2026-07-24-moss-postprocess-enhancement-design.md`

**Reference source (MOSS):**
- `moss_transcribe_diarize/subtitle/postprocess.py` — normalize/merge/split algorithms
- `moss_transcribe_diarize/subtitle/export.py` — SRT/ASS/JSON export
- `moss_transcribe_diarize/subtitle/layout.py` — overlap lane assignment
- `moss_transcribe_diarize/subtitle/models.py` — SubtitleSegment/SubtitleStyle
- `examples/prompts.md` — hotword prompt pattern

---

## File Structure

**New files:**
- `frontend/src-tauri/src/transcript_postprocess/mod.rs` — Module 1: ProcessableSegment type + normalize/merge/split/fix_overlaps
- `frontend/src-tauri/src/subtitle_export/mod.rs` — Module 2: export_srt/export_ass/export_json + assign_overlap_lanes + AssStyle
- `frontend/src-tauri/src/hotword_correction/mod.rs` — Module 3: correct_transcript_with_hotwords + prompt builder + output parser
- `frontend/src-tauri/src/hotword_correction/commands.rs` — Tauri commands for hotword CRUD + correction
- `frontend/src-tauri/src/hotword_correction/repository.rs` — DB access for hotwords table
- `frontend/src-tauri/migrations/20260724000001_create_hotwords.sql` — hotwords table migration
- `frontend/src-tauri/tests/transcript_postprocess_test.rs` — Module 1 integration tests
- `frontend/src-tauri/tests/subtitle_export_test.rs` — Module 2 integration tests

**Modified files:**
- `frontend/src-tauri/src/lib.rs` — Register 3 new modules + new Tauri commands
- `frontend/src-tauri/src/speaker_diarization_engine/commands.rs` — Call normalize after align_transcripts_with_speakers (optional, non-destructive)

**Unchanged (explicitly):**
- `sherpa_asr_engine/`, `whisper_engine/`, `parakeet_engine/` — ASR cores untouched
- `speaker_diarization_engine/engine.rs` — diarization algorithm untouched
- `summary/llm_client.rs` — reused as-is by Module 3

---

## Task 1: Create Module 1 — ProcessableSegment type and basic structure

**Files:**
- Create: `frontend/src-tauri/src/transcript_postprocess/mod.rs`
- Modify: `frontend/src-tauri/src/lib.rs:51` (add `pub mod transcript_postprocess;`)

- [ ] **Step 1: Create the module file with ProcessableSegment and NormalizeConfig**

Create `frontend/src-tauri/src/transcript_postprocess/mod.rs`:

```rust
// transcript_postprocess/mod.rs
//
// 转写段后处理：移植自 MOSS-Transcribe-Diarize subtitle/postprocess.py。
// 纯算法，无外部依赖。对 ProcessableSegment 执行合并/拆分/修复重叠。

use serde::{Deserialize, Serialize};

/// 统一的转写段中间类型（speaker 统一为 String）。
/// 从 diarization 的 TranscriptChunkForAlignment 或 DB 的 Transcript 转换而来。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessableSegment {
    pub id: String,
    pub start: f32,
    pub end: f32,
    pub speaker: String,
    pub text: String,
}

/// normalize 配置。默认值与 MOSS postprocess.py 一致。
#[derive(Debug, Clone)]
pub struct NormalizeConfig {
    pub min_duration: f32,
    pub max_duration: f32,
    pub max_chars: usize,
    pub merge_gap: f32,
}

impl Default for NormalizeConfig {
    fn default() -> Self {
        Self {
            min_duration: 1.0,
            max_duration: 6.0,
            max_chars: 24,
            merge_gap: 0.3,
        }
    }
}

// 后续 task 实现 normalize / fix_overlaps / merge_adjacent / split_long_segments
```

- [ ] **Step 2: Register the module in lib.rs**

Edit `frontend/src-tauri/src/lib.rs` around line 51-58 (where other engine modules are declared). Add after `pub mod speaker_diarization_engine;`:

```rust
pub mod transcript_postprocess;
```

- [ ] **Step 3: Verify it compiles**

Run: `cd frontend/src-tauri && cargo check`
Expected: compiles with dead_code warnings (functions not yet implemented are fine, the struct is defined)

- [ ] **Step 4: Commit**

```bash
git add frontend/src-tauri/src/transcript_postprocess/mod.rs frontend/src-tauri/src/lib.rs
git commit -m "feat(transcript_postprocess): scaffold module with ProcessableSegment type"
```

---

## Task 2: Implement `fix_overlaps` with tests (TDD)

**Files:**
- Modify: `frontend/src-tauri/src/transcript_postprocess/mod.rs`

- [ ] **Step 1: Write the failing tests**

Append to `frontend/src-tauri/src/transcript_postprocess/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn seg(id: &str, start: f32, end: f32, speaker: &str, text: &str) -> ProcessableSegment {
        ProcessableSegment {
            id: id.to_string(),
            start,
            end,
            speaker: speaker.to_string(),
            text: text.to_string(),
        }
    }

    #[test]
    fn test_fix_overlaps_no_overlap_unchanged() {
        let mut segments = vec![
            seg("a", 0.0, 2.0, "S01", "hello"),
            seg("b", 2.0, 4.0, "S02", "world"),
        ];
        fix_overlaps(&mut segments, 1.0);
        assert_eq!(segments[0].start, 0.0);
        assert_eq!(segments[0].end, 2.0);
        assert_eq!(segments[1].start, 2.0);
        assert_eq!(segments[1].end, 4.0);
    }

    #[test]
    fn test_fix_overlaps_with_overlap_pushes_forward() {
        // seg b starts at 1.5 but a ends at 2.0 → b.start pushed to 2.0
        let mut segments = vec![
            seg("a", 0.0, 2.0, "S01", "hello"),
            seg("b", 1.5, 3.0, "S02", "world"),
        ];
        fix_overlaps(&mut segments, 1.0);
        assert_eq!(segments[1].start, 2.0);
        assert_eq!(segments[1].end, 3.0);
    }

    #[test]
    fn test_fix_overlaps_extends_short_segment_to_min_duration() {
        // end - start < min_duration → end extended
        let mut segments = vec![seg("a", 0.0, 0.3, "S01", "hi")];
        fix_overlaps(&mut segments, 1.0);
        assert_eq!(segments[0].start, 0.0);
        assert_eq!(segments[0].end, 1.0);
    }

    #[test]
    fn test_fix_overlaps_empty_input() {
        let mut segments: Vec<ProcessableSegment> = vec![];
        fix_overlaps(&mut segments, 1.0);
        assert!(segments.is_empty());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd frontend/src-tauri && cargo test transcript_postprocess -- --nocapture`
Expected: FAIL — `fix_overlaps` not found

- [ ] **Step 3: Implement `fix_overlaps`**

Add to `frontend/src-tauri/src/transcript_postprocess/mod.rs` (before the `#[cfg(test)]` block):

```rust
/// 修复时间重叠：游标推进，保证时间单调不重叠，每段至少 min_duration。
/// 移植自 MOSS postprocess.py `_fix_overlaps`。
pub fn fix_overlaps(segments: &mut Vec<ProcessableSegment>, min_duration: f32) {
    let mut cursor: f32 = 0.0;
    for seg in segments.iter_mut() {
        let start = seg.start.max(cursor);
        let end = seg.end.max(start + min_duration);
        seg.start = start;
        seg.end = end;
        cursor = end;
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd frontend/src-tauri && cargo test transcript_postprocess -- --nocapture`
Expected: 4 tests PASS

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/transcript_postprocess/mod.rs
git commit -m "feat(transcript_postprocess): implement fix_overlaps with tests"
```

---

## Task 3: Implement `join_text` helper with tests

**Files:**
- Modify: `frontend/src-tauri/src/transcript_postprocess/mod.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `frontend/src-tauri/src/transcript_postprocess/mod.rs`:

```rust
    #[test]
    fn test_join_text_both_ascii_adds_space() {
        assert_eq!(join_text("hello", "world"), "hello world");
    }

    #[test]
    fn test_join_text_both_cjk_no_space() {
        assert_eq!(join_text("你好", "世界"), "你好世界");
    }

    #[test]
    fn test_join_text_mixed_ascii_cjk_no_space() {
        assert_eq!(join_text("hello", "世界"), "hello世界");
    }

    #[test]
    fn test_join_text_empty_left() {
        assert_eq!(join_text("", "world"), "world");
    }

    #[test]
    fn test_join_text_empty_right() {
        assert_eq!(join_text("hello", ""), "hello");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd frontend/src-tauri && cargo test transcript_postprocess::tests::test_join -- --nocapture`
Expected: FAIL — `join_text` not found

- [ ] **Step 3: Implement `join_text`**

Add to `frontend/src-tauri/src/transcript_postprocess/mod.rs` (before `fix_overlaps`):

```rust
/// 文本拼接：ASCII 之间加空格，CJK/混合之间不加。
/// 移植自 MOSS postprocess.py `_join_text`。
fn join_text(left: &str, right: &str) -> String {
    if left.is_empty() {
        return right.to_string();
    }
    if right.is_empty() {
        return left.to_string();
    }
    let left_last = left.chars().last().unwrap_or(' ');
    let right_first = right.chars().next().unwrap_or(' ');
    if left_last.is_ascii() && right_first.is_ascii() {
        format!("{} {}", left, right)
    } else {
        format!("{}{}", left, right)
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd frontend/src-tauri && cargo test transcript_postprocess::tests::test_join -- --nocapture`
Expected: 5 tests PASS

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/transcript_postprocess/mod.rs
git commit -m "feat(transcript_postprocess): implement join_text helper with tests"
```

---

## Task 4: Implement `merge_adjacent` with tests

**Files:**
- Modify: `frontend/src-tauri/src/transcript_postprocess/mod.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module:

```rust
    #[test]
    fn test_merge_adjacent_same_speaker_small_gap() {
        let mut segments = vec![
            seg("a", 0.0, 2.0, "S01", "hello"),
            seg("b", 2.1, 3.0, "S01", "world"),
        ];
        merge_adjacent(&mut segments, 0.3, 24);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].start, 0.0);
        assert_eq!(segments[0].end, 3.0);
        assert_eq!(segments[0].text, "hello world");
    }

    #[test]
    fn test_merge_adjacent_different_speaker_no_merge() {
        let mut segments = vec![
            seg("a", 0.0, 2.0, "S01", "hello"),
            seg("b", 2.1, 3.0, "S02", "world"),
        ];
        merge_adjacent(&mut segments, 0.3, 24);
        assert_eq!(segments.len(), 2);
    }

    #[test]
    fn test_merge_adjacent_gap_too_large_no_merge() {
        let mut segments = vec![
            seg("a", 0.0, 2.0, "S01", "hello"),
            seg("b", 2.5, 3.0, "S01", "world"),
        ];
        merge_adjacent(&mut segments, 0.3, 24);
        assert_eq!(segments.len(), 2);
    }

    #[test]
    fn test_merge_adjacent_combined_too_long_no_merge() {
        let long_text = "a".repeat(40);
        let mut segments = vec![
            seg("a", 0.0, 2.0, "S01", &long_text),
            seg("b", 2.1, 3.0, "S01", &long_text),
        ];
        merge_adjacent(&mut segments, 0.3, 24);
        // 2*max_chars = 48, combined = 80+1 space = 81 > 48 → no merge
        assert_eq!(segments.len(), 2);
    }

    #[test]
    fn test_merge_adjacent_negative_gap_no_merge() {
        // gap < 0 means segments overlap (should have been fixed by fix_overlaps first)
        let mut segments = vec![
            seg("a", 0.0, 3.0, "S01", "hello"),
            seg("b", 2.0, 4.0, "S01", "world"),
        ];
        merge_adjacent(&mut segments, 0.3, 24);
        assert_eq!(segments.len(), 2);
    }

    #[test]
    fn test_merge_adjacent_empty_input() {
        let mut segments: Vec<ProcessableSegment> = vec![];
        merge_adjacent(&mut segments, 0.3, 24);
        assert!(segments.is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd frontend/src-tauri && cargo test transcript_postprocess::tests::test_merge -- --nocapture`
Expected: FAIL — `merge_adjacent` not found

- [ ] **Step 3: Implement `merge_adjacent`**

Add to `frontend/src-tauri/src/transcript_postprocess/mod.rs`:

```rust
/// 合并相邻段：同 speaker + gap ∈ [0, merge_gap] + 合并后字数 ≤ 2*max_chars。
/// 移植自 MOSS postprocess.py `_merge_adjacent`。
pub fn merge_adjacent(
    segments: &mut Vec<ProcessableSegment>,
    merge_gap: f32,
    max_chars: usize,
) {
    if segments.is_empty() {
        return;
    }
    let mut merged: Vec<ProcessableSegment> = Vec::with_capacity(segments.len());
    merged.push(segments[0].clone());
    for seg in segments.iter().skip(1) {
        let previous = merged.last().unwrap();
        let gap = seg.start - previous.end;
        let combined_text = join_text(&previous.text, &seg.text);
        let can_merge = previous.speaker == seg.speaker
            && gap >= 0.0
            && gap <= merge_gap
            && combined_text.chars().count() <= max_chars * 2;
        if can_merge {
            let merged_seg = ProcessableSegment {
                id: previous.id.clone(),
                start: previous.start,
                end: previous.end.max(seg.end),
                speaker: previous.speaker.clone(),
                text: combined_text,
            };
            *merged.last_mut().unwrap() = merged_seg;
        } else {
            merged.push(seg.clone());
        }
    }
    *segments = merged;
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd frontend/src-tauri && cargo test transcript_postprocess::tests::test_merge -- --nocapture`
Expected: 6 tests PASS

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/transcript_postprocess/mod.rs
git commit -m "feat(transcript_postprocess): implement merge_adjacent with tests"
```

---

## Task 5: Implement `split_text` helper with tests

**Files:**
- Modify: `frontend/src-tauri/src/transcript_postprocess/mod.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module:

```rust
    #[test]
    fn test_split_text_short_text_unchanged() {
        assert_eq!(split_text("hello", 24), vec!["hello"]);
    }

    #[test]
    fn test_split_text_cjk_by_punctuation() {
        let text = "今天天气很好。我们去公园散步。晚上回家吃饭。";
        let chunks = split_text(text, 10);
        assert!(chunks.len() >= 2);
        // 每段不超过 max_chars (allowing final compact pass)
        for c in &chunks {
            assert!(c.chars().count() <= 10);
        }
    }

    #[test]
    fn test_split_text_forced_cut_at_max_chars() {
        let text = "aaaaaaaaaaaaaaaaaaaaaaaaaa"; // 26 ascii chars
        let chunks = split_text(text, 10);
        assert!(chunks.len() >= 3);
    }

    #[test]
    fn test_split_text_empty_returns_empty() {
        assert!(split_text("", 10).is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd frontend/src-tauri && cargo test transcript_postprocess::tests::test_split_text -- --nocapture`
Expected: FAIL — `split_text` not found

- [ ] **Step 3: Implement `split_text`**

Add to `frontend/src-tauri/src/transcript_postprocess/mod.rs`:

```rust
/// 中文标点字符集（用于智能切分）。
const PUNCTUATION: &str = "。！？!?；;，、 ";

/// 按标点和 max_chars 切分文本，然后紧凑合并。
/// 移植自 MOSS postprocess.py `_split_text`。
fn split_text(text: &str, max_chars: usize) -> Vec<String> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }
    if text.chars().count() <= max_chars {
        return vec![text.to_string()];
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut current: Vec<char> = Vec::new();
    for ch in text.chars() {
        current.push(ch);
        let should_cut = current.len() >= max_chars
            || (PUNCTUATION.contains(ch) && current.len() >= max_chars / 2);
        if should_cut {
            let s: String = current.iter().collect::<String>().trim().to_string();
            if !s.is_empty() {
                chunks.push(s);
            }
            current.clear();
        }
    }
    if !current.is_empty() {
        let s: String = current.iter().collect::<String>().trim().to_string();
        if !s.is_empty() {
            chunks.push(s);
        }
    }

    // 紧凑合并：相邻 chunk 合并后 ≤ max_chars 则合并
    let mut compact: Vec<String> = Vec::new();
    for chunk in chunks {
        if compact.is_empty() {
            compact.push(chunk);
        } else {
            let last = compact.last().unwrap();
            if last.chars().count() + chunk.chars().count() <= max_chars {
                let merged = join_text(last, &chunk);
                *compact.last_mut().unwrap() = merged;
            } else {
                compact.push(chunk);
            }
        }
    }
    compact
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd frontend/src-tauri && cargo test transcript_postprocess::tests::test_split_text -- --nocapture`
Expected: 4 tests PASS

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/transcript_postprocess/mod.rs
git commit -m "feat(transcript_postprocess): implement split_text helper with tests"
```

---

## Task 6: Implement `split_long_segments` with tests

**Files:**
- Modify: `frontend/src-tauri/src/transcript_postprocess/mod.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module:

```rust
    #[test]
    fn test_split_long_short_segment_unchanged() {
        let mut segments = vec![seg("a", 0.0, 2.0, "S01", "hello")];
        split_long_segments(&mut segments, 1.0, 6.0, 24);
        assert_eq!(segments.len(), 1);
    }

    #[test]
    fn test_split_long_splits_by_punctuation() {
        let long_text = "今天天气很好。我们去公园散步。晚上回家吃饭。明天再继续。";
        let mut segments = vec![seg("a", 0.0, 20.0, "S01", long_text)];
        split_long_segments(&mut segments, 1.0, 6.0, 10);
        assert!(segments.len() >= 2);
        // 每段文本不超过 max_chars
        for s in &segments {
            assert!(s.text.chars().count() <= 10);
        }
    }

    #[test]
    fn test_split_long_only_duration_exceeds_keeps_one() {
        // duration > max_duration but text <= max_chars → no split
        let mut segments = vec![seg("a", 0.0, 10.0, "S01", "short")];
        split_long_segments(&mut segments, 1.0, 6.0, 24);
        assert_eq!(segments.len(), 1);
    }

    #[test]
    fn test_split_long_preserves_total_time_range() {
        let long_text = "今天天气很好。我们去公园散步。晚上回家吃饭。明天再继续。";
        let mut segments = vec![seg("a", 0.0, 20.0, "S01", long_text)];
        split_long_segments(&mut segments, 1.0, 6.0, 10);
        assert!(segments.len() >= 2);
        assert_eq!(segments.first().unwrap().start, 0.0);
        assert_eq!(segments.last().unwrap().end, 20.0);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd frontend/src-tauri && cargo test transcript_postprocess::tests::test_split_long -- --nocapture`
Expected: FAIL — `split_long_segments` not found

- [ ] **Step 3: Implement `split_long_segments`**

Add to `frontend/src-tauri/src/transcript_postprocess/mod.rs`:

```rust
/// 拆分超长段：duration > max_duration 且 text.len > max_chars 时按标点切分，
/// 按字数比例分配时间，每子段至少 min_duration。
/// 移植自 MOSS postprocess.py `_split_long_segments`。
pub fn split_long_segments(
    segments: &mut Vec<ProcessableSegment>,
    min_duration: f32,
    max_duration: f32,
    max_chars: usize,
) {
    let mut output: Vec<ProcessableSegment> = Vec::new();
    for segment in segments.iter() {
        let duration = segment.end - segment.start;
        if duration <= max_duration && segment.text.chars().count() <= max_chars {
            output.push(segment.clone());
            continue;
        }

        let chunks = split_text(&segment.text, max_chars);
        if chunks.len() <= 1 {
            output.push(segment.clone());
            continue;
        }

        let total_chars: usize = chunks.iter().map(|c| c.chars().count().max(1)).sum();
        let mut cursor = segment.start;
        let n_chunks = chunks.len();
        for (index, chunk) in chunks.iter().enumerate() {
            let end = if index == n_chunks - 1 {
                segment.end
            } else {
                let ratio = chunk.chars().count().max(1) as f32 / total_chars as f32;
                let proposed = cursor + min_duration.max(duration * ratio);
                // 确保剩余子段还有 min_duration 空间
                let remaining_min = min_duration * (n_chunks - index - 1) as f32;
                proposed.min(segment.end - remaining_min)
            };
            let end = end.max(cursor + min_duration);
            output.push(ProcessableSegment {
                id: format!("{}_{}", segment.id, index + 1),
                start: cursor,
                end,
                speaker: segment.speaker.clone(),
                text: chunk.clone(),
            });
            cursor = end;
        }
    }
    *segments = output;
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd frontend/src-tauri && cargo test transcript_postprocess::tests::test_split_long -- --nocapture`
Expected: 4 tests PASS

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/transcript_postprocess/mod.rs
git commit -m "feat(transcript_postprocess): implement split_long_segments with tests"
```

---

## Task 7: Implement `normalize` (full pipeline) with tests

**Files:**
- Modify: `frontend/src-tauri/src/transcript_postprocess/mod.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module:

```rust
    #[test]
    fn test_normalize_empty_input() {
        let mut segments: Vec<ProcessableSegment> = vec![];
        normalize(&mut segments, &NormalizeConfig::default());
        assert!(segments.is_empty());
    }

    #[test]
    fn test_normalize_strips_empty_text_segments() {
        let mut segments = vec![
            seg("a", 0.0, 2.0, "S01", "  "),
            seg("b", 2.0, 4.0, "S01", "hello"),
        ];
        normalize(&mut segments, &NormalizeConfig::default());
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "hello");
    }

    #[test]
    fn test_normalize_sorts_by_start_time() {
        let mut segments = vec![
            seg("b", 2.0, 4.0, "S01", "world"),
            seg("a", 0.0, 2.0, "S01", "hello"),
        ];
        normalize(&mut segments, &NormalizeConfig::default());
        assert_eq!(segments[0].id, "a");
        assert_eq!(segments[1].id, "b");
    }

    #[test]
    fn test_normalize_clamps_negative_start() {
        let mut segments = vec![seg("a", -1.0, 2.0, "S01", "hello")];
        normalize(&mut segments, &NormalizeConfig::default());
        assert_eq!(segments[0].start, 0.0);
    }

    #[test]
    fn test_normalize_full_pipeline_merges_and_splits() {
        // 3 short same-speaker segments within merge_gap → merge into 1
        let mut segments = vec![
            seg("a", 0.0, 1.5, "S01", "你好"),
            seg("b", 1.6, 3.0, "S01", "世界"),
            seg("c", 3.1, 4.5, "S01", "今天"),
        ];
        normalize(&mut segments, &NormalizeConfig::default());
        // All same speaker, small gaps, short text → merged
        assert_eq!(segments.len(), 1);
        assert!(segments[0].text.contains("你好"));
        assert!(segments[0].text.contains("世界"));
        assert!(segments[0].text.contains("今天"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd frontend/src-tauri && cargo test transcript_postprocess::tests::test_normalize -- --nocapture`
Expected: FAIL — `normalize` not found

- [ ] **Step 3: Implement `normalize` and `prepare_segments`**

Add to `frontend/src-tauri/src/transcript_postprocess/mod.rs`:

```rust
/// 预处理：strip 文本、跳过空段、start ≥ 0、end ≥ start、按 (start, end) 排序。
/// 移植自 MOSS postprocess.py `_prepare_segments`。
fn prepare_segments(segments: Vec<ProcessableSegment>) -> Vec<ProcessableSegment> {
    let mut prepared: Vec<ProcessableSegment> = Vec::new();
    for mut seg in segments {
        let text = seg.text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        seg.text = text;
        seg.start = seg.start.max(0.0);
        seg.end = seg.end.max(seg.start);
        prepared.push(seg);
    }
    prepared.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap()
            .then(a.end.partial_cmp(&b.end).unwrap())
    });
    prepared
}

/// 完整后处理流水线：prepare → fix_overlaps → merge → split → fix_overlaps。
/// 移植自 MOSS postprocess.py `normalize_segments`。
pub fn normalize(segments: &mut Vec<ProcessableSegment>, config: &NormalizeConfig) {
    let mut prepared = prepare_segments(std::mem::take(segments));
    fix_overlaps(&mut prepared, config.min_duration);
    merge_adjacent(&mut prepared, config.merge_gap, config.max_chars);
    split_long_segments(
        &mut prepared,
        config.min_duration,
        config.max_duration,
        config.max_chars,
    );
    fix_overlaps(&mut prepared, config.min_duration);
    *segments = prepared;
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd frontend/src-tauri && cargo test transcript_postprocess -- --nocapture`
Expected: all tests PASS (Task 2-7 cumulative)

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/transcript_postprocess/mod.rs
git commit -m "feat(transcript_postprocess): implement normalize pipeline with tests"
```

---

## Task 8: Implement conversion adapters (From traits)

**Files:**
- Modify: `frontend/src-tauri/src/transcript_postprocess/mod.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module:

```rust
    #[test]
    fn test_from_transcript_chunk_for_alignment() {
        use crate::speaker_diarization_engine::engine::TranscriptChunkForAlignment;
        let chunk = TranscriptChunkForAlignment {
            id: "c1".to_string(),
            audio_start_time: 1.5,
            audio_end_time: 3.0,
            speaker: Some(1),
        };
        let seg: ProcessableSegment = chunk.into();
        assert_eq!(seg.id, "c1");
        assert!((seg.start - 1.5).abs() < 1e-6);
        assert!((seg.end - 3.0).abs() < 1e-6);
        assert_eq!(seg.speaker, "1");
        assert_eq!(seg.text, ""); // alignment chunk has no text
    }

    #[test]
    fn test_from_transcript_chunk_for_alignment_no_speaker() {
        use crate::speaker_diarization_engine::engine::TranscriptChunkForAlignment;
        let chunk = TranscriptChunkForAlignment {
            id: "c2".to_string(),
            audio_start_time: 0.0,
            audio_end_time: 2.0,
            speaker: None,
        };
        let seg: ProcessableSegment = chunk.into();
        assert_eq!(seg.speaker, "S00");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd frontend/src-tauri && cargo test transcript_postprocess::tests::test_from_transcript -- --nocapture`
Expected: FAIL — `From<TranscriptChunkForAlignment>` not implemented

- [ ] **Step 3: Implement the From trait**

Add to `frontend/src-tauri/src/transcript_postprocess/mod.rs` (after ProcessableSegment definition):

```rust
use crate::speaker_diarization_engine::engine::TranscriptChunkForAlignment;

impl From<TranscriptChunkForAlignment> for ProcessableSegment {
    fn from(chunk: TranscriptChunkForAlignment) -> Self {
        ProcessableSegment {
            id: chunk.id,
            start: chunk.audio_start_time as f32,
            end: chunk.audio_end_time as f32,
            speaker: chunk
                .speaker
                .map(|s| s.to_string())
                .unwrap_or_else(|| "S00".to_string()),
            text: String::new(), // alignment chunk has no text; filled by caller
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd frontend/src-tauri && cargo test transcript_postprocess -- --nocapture`
Expected: all tests PASS

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/transcript_postprocess/mod.rs
git commit -m "feat(transcript_postprocess): add From<TranscriptChunkForAlignment> adapter"
```

---

## Task 9: Create Module 2 — subtitle_export scaffold + AssStyle

**Files:**
- Create: `frontend/src-tauri/src/subtitle_export/mod.rs`
- Modify: `frontend/src-tauri/src/lib.rs` (add `pub mod subtitle_export;`)

- [ ] **Step 1: Create the module with AssStyle and constants**

Create `frontend/src-tauri/src/subtitle_export/mod.rs`:

```rust
// subtitle_export/mod.rs
//
// 字幕导出：移植自 MOSS-Transcribe-Diarize subtitle/export.py + layout.py。
// 纯 Rust，无外部依赖。支持 SRT/ASS/JSON 三种格式。

use std::collections::HashMap;

use crate::transcript_postprocess::ProcessableSegment;

/// 8 色说话人调色板（ASS 颜色，BGR 格式）。
/// 移植自 MOSS export.py `SPEAKER_COLORS`。
const SPEAKER_COLORS: &[&str] = &[
    "&H00FFFFFF",
    "&H005BE7FF",
    "&H0086F28F",
    "&H00BBA7FF",
    "&H0000D7FF",
    "&H00FFB56B",
    "&H00FF8EDB",
    "&H00D8D8D8",
];

/// ASS 样式配置。默认值与 MOSS SubtitleStyle 一致。
#[derive(Debug, Clone)]
pub struct AssStyle {
    pub font_name: String,
    pub font_size: Option<usize>,
    pub alignment: usize,
    pub margin_v: usize,
    pub outline: usize,
    pub shadow: usize,
    pub show_speaker: bool,
    pub speaker_colors: bool,
    pub primary_color: String,
    pub outline_color: String,
    pub back_color: String,
    pub speaker_names: HashMap<String, String>,
}

impl Default for AssStyle {
    fn default() -> Self {
        Self {
            font_name: "Noto Sans CJK SC".to_string(),
            font_size: None,
            alignment: 2,
            margin_v: 56,
            outline: 3,
            shadow: 1,
            show_speaker: true,
            speaker_colors: true,
            primary_color: "&H00FFFFFF".to_string(),
            outline_color: "&H00000000".to_string(),
            back_color: "&H64000000".to_string(),
            speaker_names: HashMap::new(),
        }
    }
}

// 后续 task 实现 export_srt / export_ass / export_json / assign_overlap_lanes
```

- [ ] **Step 2: Register the module in lib.rs**

Edit `frontend/src-tauri/src/lib.rs`, add after `pub mod transcript_postprocess;`:

```rust
pub mod subtitle_export;
```

- [ ] **Step 3: Verify it compiles**

Run: `cd frontend/src-tauri && cargo check`
Expected: compiles (with unused warnings, acceptable)

- [ ] **Step 4: Commit**

```bash
git add frontend/src-tauri/src/subtitle_export/mod.rs frontend/src-tauri/src/lib.rs
git commit -m "feat(subtitle_export): scaffold module with AssStyle and SPEAKER_COLORS"
```

---

## Task 10: Implement `assign_overlap_lanes` with tests

**Files:**
- Modify: `frontend/src-tauri/src/subtitle_export/mod.rs`

- [ ] **Step 1: Write the failing tests**

Append to `frontend/src-tauri/src/subtitle_export/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript_postprocess::ProcessableSegment;

    fn seg(id: &str, start: f32, end: f32, speaker: &str, text: &str) -> ProcessableSegment {
        ProcessableSegment {
            id: id.to_string(),
            start,
            end,
            speaker: speaker.to_string(),
            text: text.to_string(),
        }
    }

    #[test]
    fn test_assign_lanes_no_overlap_all_lane_zero() {
        let segments = vec![
            seg("a", 0.0, 2.0, "S01", "hello"),
            seg("b", 2.0, 4.0, "S02", "world"),
        ];
        let lanes = assign_overlap_lanes(&segments);
        assert_eq!(lanes, vec![0, 0]);
    }

    #[test]
    fn test_assign_lanes_with_overlap_second_goes_lane_one() {
        let segments = vec![
            seg("a", 0.0, 3.0, "S01", "hello"),
            seg("b", 1.0, 4.0, "S02", "world"),
        ];
        let lanes = assign_overlap_lanes(&segments);
        assert_eq!(lanes[0], 0);
        assert_eq!(lanes[1], 1);
    }

    #[test]
    fn test_assign_lanes_reuse_lane_after_end() {
        // a: [0,2], b: [1,4] (overlap → lane 1), c: [5,6] (lane 0 reused, a ended)
        let segments = vec![
            seg("a", 0.0, 2.0, "S01", "hello"),
            seg("b", 1.0, 4.0, "S02", "world"),
            seg("c", 5.0, 6.0, "S03", "again"),
        ];
        let lanes = assign_overlap_lanes(&segments);
        assert_eq!(lanes[0], 0);
        assert_eq!(lanes[1], 1);
        assert_eq!(lanes[2], 0);
    }

    #[test]
    fn test_assign_lanes_empty_input() {
        let segments: Vec<ProcessableSegment> = vec![];
        let lanes = assign_overlap_lanes(&segments);
        assert!(lanes.is_empty());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd frontend/src-tauri && cargo test subtitle_export::tests::test_assign_lanes -- --nocapture`
Expected: FAIL — `assign_overlap_lanes` not found

- [ ] **Step 3: Implement `assign_overlap_lanes`**

Add to `frontend/src-tauri/src/subtitle_export/mod.rs`:

```rust
/// 为重叠字幕段分配垂直 lane。Lane 0 是底部基准行，更大 lane 号向上堆叠。
/// 移植自 MOSS layout.py `assign_overlap_lanes`。
pub fn assign_overlap_lanes(segments: &[ProcessableSegment]) -> Vec<usize> {
    let n = segments.len();
    let mut lanes = vec![0usize; n];
    let mut lane_ends: Vec<f32> = Vec::new();

    // 按 (start, end, 原索引) 排序
    let mut indexed: Vec<(usize, &ProcessableSegment)> = segments.iter().enumerate().collect();
    indexed.sort_by(|a, b| {
        a.1.start
            .partial_cmp(&b.1.start)
            .unwrap()
            .then(a.1.end.partial_cmp(&b.1.end).unwrap())
            .then(a.0.cmp(&b.0))
    });

    for (original_index, segment) in indexed {
        let start = segment.start;
        let end = start.max(segment.end);
        let mut assigned = false;
        for (lane, lane_end) in lane_ends.iter_mut().enumerate() {
            if *lane_end <= start {
                lanes[original_index] = lane;
                *lane_end = end;
                assigned = true;
                break;
            }
        }
        if !assigned {
            lanes[original_index] = lane_ends.len();
            lane_ends.push(end);
        }
    }
    lanes
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd frontend/src-tauri && cargo test subtitle_export::tests::test_assign_lanes -- --nocapture`
Expected: 4 tests PASS

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/subtitle_export/mod.rs
git commit -m "feat(subtitle_export): implement assign_overlap_lanes with tests"
```

---

## Task 11: Implement time formatters + `export_srt` with tests

**Files:**
- Modify: `frontend/src-tauri/src/subtitle_export/mod.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module:

```rust
    #[test]
    fn test_format_srt_time_zero() {
        assert_eq!(format_srt_time(0.0), "00:00:00,000");
    }

    #[test]
    fn test_format_srt_time_with_millis() {
        assert_eq!(format_srt_time(1.5), "00:00:01,500");
    }

    #[test]
    fn test_format_srt_time_minutes_seconds() {
        assert_eq!(format_srt_time(65.234), "00:01:05,234");
    }

    #[test]
    fn test_format_srt_time_hours() {
        assert_eq!(format_srt_time(3661.5), "01:01:01,500");
    }

    #[test]
    fn test_export_srt_basic() {
        let segments = vec![
            seg("a", 0.0, 2.0, "S01", "hello"),
            seg("b", 2.0, 4.0, "S02", "world"),
        ];
        let srt = export_srt(&segments, true, &HashMap::new());
        assert!(srt.contains("1\n00:00:00,000 --> 00:00:02,000\nS01: hello"));
        assert!(srt.contains("2\n00:00:02,000 --> 00:00:04,000\nS02: world"));
    }

    #[test]
    fn test_export_srt_no_speaker() {
        let segments = vec![seg("a", 0.0, 1.0, "S01", "hi")];
        let srt = export_srt(&segments, false, &HashMap::new());
        assert!(srt.contains("hi"));
        assert!(!srt.contains("S01:"));
    }

    #[test]
    fn test_export_srt_uses_speaker_names() {
        let mut names = HashMap::new();
        names.insert("S01".to_string(), "张三".to_string());
        let segments = vec![seg("a", 0.0, 1.0, "S01", "hi")];
        let srt = export_srt(&segments, true, &names);
        assert!(srt.contains("张三: hi"));
    }

    #[test]
    fn test_export_srt_empty_input() {
        let segments: Vec<ProcessableSegment> = vec![];
        let srt = export_srt(&segments, true, &HashMap::new());
        assert!(srt.is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd frontend/src-tauri && cargo test subtitle_export::tests::test_format_srt_time -- --nocapture`
Run: `cd frontend/src-tauri && cargo test subtitle_export::tests::test_export_srt -- --nocapture`
Expected: FAIL — functions not found

- [ ] **Step 3: Implement time formatters, display_text, and export_srt**

Add to `frontend/src-tauri/src/subtitle_export/mod.rs`:

```rust
/// SRT 时间格式化：HH:MM:SS,mmm
/// 移植自 MOSS export.py `format_srt_time`。
pub fn format_srt_time(seconds: f32) -> String {
    let milliseconds = (seconds.max(0.0) * 1000.0).round() as u64;
    let hours = milliseconds / 3_600_000;
    let remainder = milliseconds % 3_600_000;
    let minutes = remainder / 60_000;
    let remainder = remainder % 60_000;
    let secs = remainder / 1000;
    let millis = remainder % 1000;
    format!("{:02}:{:02}:{:02},{:03}", hours, minutes, secs, millis)
}

/// ASS 时间格式化：H:MM:SS.cc
/// 移植自 MOSS export.py `format_ass_time`。
pub fn format_ass_time(seconds: f32) -> String {
    let centiseconds = (seconds.max(0.0) * 100.0).round() as u64;
    let hours = centiseconds / 360_000;
    let remainder = centiseconds % 360_000;
    let minutes = remainder / 6_000;
    let remainder = remainder % 6_000;
    let secs = remainder / 100;
    let centis = remainder % 100;
    format!("{}:{:02}:{:02}.{:02}", hours, minutes, secs, centis)
}

/// 显示文本：show_speaker=true 时加说话人前缀。
/// 移植自 MOSS export.py `_display_text`。
fn display_text(
    segment: &ProcessableSegment,
    show_speaker: bool,
    speaker_names: &HashMap<String, String>,
) -> String {
    if !show_speaker || segment.speaker.is_empty() {
        return segment.text.clone();
    }
    let speaker = speaker_names
        .get(&segment.speaker)
        .cloned()
        .unwrap_or_else(|| segment.speaker.clone());
    format!("{}: {}", speaker, segment.text)
}

/// 导出 SRT 格式字幕。
/// 移植自 MOSS export.py `export_srt`。
pub fn export_srt(
    segments: &[ProcessableSegment],
    show_speaker: bool,
    speaker_names: &HashMap<String, String>,
) -> String {
    let mut blocks: Vec<String> = Vec::new();
    for (index, segment) in segments.iter().enumerate() {
        let text = display_text(segment, show_speaker, speaker_names);
        blocks.push(format!(
            "{}\n{} --> {}\n{}",
            index + 1,
            format_srt_time(segment.start),
            format_srt_time(segment.end),
            text
        ));
    }
    if blocks.is_empty() {
        String::new()
    } else {
        blocks.join("\n\n") + "\n"
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd frontend/src-tauri && cargo test subtitle_export -- --nocapture`
Expected: all tests PASS

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/subtitle_export/mod.rs
git commit -m "feat(subtitle_export): implement format_srt_time, format_ass_time, display_text, export_srt with tests"
```

---

## Task 12: Implement `export_ass` with tests

**Files:**
- Modify: `frontend/src-tauri/src/subtitle_export/mod.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module:

```rust
    #[test]
    fn test_export_ass_basic_structure() {
        let segments = vec![seg("a", 0.0, 2.0, "S01", "hello")];
        let style = AssStyle::default();
        let ass = export_ass(&segments, &style, 1920, 1080);
        assert!(ass.contains("[Script Info]"));
        assert!(ass.contains("[V4+ Styles]"));
        assert!(ass.contains("[Events]"));
        assert!(ass.contains("Dialogue:"));
    }

    #[test]
    fn test_export_ass_has_default_style() {
        let segments = vec![seg("a", 0.0, 1.0, "S01", "hi")];
        let style = AssStyle::default();
        let ass = export_ass(&segments, &style, 1920, 1080);
        assert!(ass.contains("Style: Default,Noto Sans CJK SC"));
    }

    #[test]
    fn test_export_ass_speaker_colors_creates_speaker_styles() {
        let segments = vec![
            seg("a", 0.0, 1.0, "S01", "hi"),
            seg("b", 1.0, 2.0, "S02", "yo"),
        ];
        let style = AssStyle::default();
        let ass = export_ass(&segments, &style, 1920, 1080);
        assert!(ass.contains("Style: Speaker_S01"));
        assert!(ass.contains("Style: Speaker_S02"));
    }

    #[test]
    fn test_export_ass_escapes_special_chars() {
        let segments = vec![seg("a", 0.0, 1.0, "S01", "hello {world}\\test")];
        let style = AssStyle::default();
        let ass = export_ass(&segments, &style, 1920, 1080);
        assert!(ass.contains("hello (world)\\\\test")); // { → ( , } → ) , \ → \\
    }

    #[test]
    fn test_export_ass_no_speaker_colors_uses_default_style() {
        let segments = vec![seg("a", 0.0, 1.0, "S01", "hi")];
        let mut style = AssStyle::default();
        style.speaker_colors = false;
        let ass = export_ass(&segments, &style, 1920, 1080);
        assert!(!ass.contains("Speaker_S01"));
        assert!(ass.contains("Default"));
    }

    #[test]
    fn test_export_ass_empty_input_has_headers() {
        let segments: Vec<ProcessableSegment> = vec![];
        let style = AssStyle::default();
        let ass = export_ass(&segments, &style, 1920, 1080);
        // Empty segments still produce headers, just no Dialogue lines
        assert!(ass.contains("[Script Info]"));
        assert!(!ass.contains("Dialogue:"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd frontend/src-tauri && cargo test subtitle_export::tests::test_export_ass -- --nocapture`
Expected: FAIL — `export_ass` not found

- [ ] **Step 3: Implement `export_ass`**

Add to `frontend/src-tauri/src/subtitle_export/mod.rs`:

```rust
/// 生成 ASS Style 行。
/// 移植自 MOSS export.py `_ass_style_line`。
fn ass_style_line(name: &str, style: &AssStyle, font_size: usize, primary_color: &str) -> String {
    format!(
        "Style: {name},{font},{size},{primary},&H000000FF,{outline_color},{back_color},0,0,0,0,100,100,0,0,1,{outline},{shadow},{alignment},48,48,{margin_v},1",
        name = name,
        font = style.font_name,
        size = font_size,
        primary = primary_color,
        outline_color = style.outline_color,
        back_color = style.back_color,
        outline = style.outline,
        shadow = style.shadow,
        alignment = style.alignment,
        margin_v = style.margin_v
    )
}

/// ASS 说话人 Style 名称（特殊字符替换为下划线）。
/// 移植自 MOSS export.py `_speaker_style_name`。
fn speaker_style_name(speaker: &str) -> String {
    let cleaned: String = speaker
        .chars()
        .map(|ch| if ch.is_alphanumeric() { ch } else { '_' })
        .collect();
    format!("Speaker_{}", cleaned)
}

/// ASS 特殊字符转义。
/// 移植自 MOSS export.py `_ass_escape`。
fn ass_escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('{', "(")
        .replace('}', ")")
        .replace('\n', "\\N")
}

/// 导出 ASS 格式字幕（带说话人颜色 + lane 分层）。
/// 移植自 MOSS export.py `export_ass`。
pub fn export_ass(
    segments: &[ProcessableSegment],
    style: &AssStyle,
    video_width: usize,
    video_height: usize,
) -> String {
    let font_size = style
        .font_size
        .unwrap_or_else(|| std::cmp::max(24, (video_height as f64 * 0.045).round() as usize));
    let segments_vec: Vec<ProcessableSegment> = segments.to_vec();
    let speakers: Vec<String> = {
        let mut s: Vec<String> = segments_vec
            .iter()
            .map(|seg| seg.speaker.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        s.sort();
        s
    };

    let mut style_lines: Vec<String> = Vec::new();
    style_lines.push(ass_style_line("Default", style, font_size, &style.primary_color));
    if style.speaker_colors {
        for (index, speaker) in speakers.iter().enumerate() {
            let color = SPEAKER_COLORS[index % SPEAKER_COLORS.len()];
            style_lines.push(ass_style_line(&speaker_style_name(speaker), style, font_size, color));
        }
    }

    let lanes = assign_overlap_lanes(&segments_vec);
    let lane_step = std::cmp::max(1, font_size);
    let mut dialogue_lines: Vec<String> = Vec::new();
    for (segment, lane) in segments_vec.iter().zip(lanes.iter(), ) {
        let style_name = if style.speaker_colors {
            speaker_style_name(&segment.speaker)
        } else {
            "Default".to_string()
        };
        let text = ass_escape(&display_text(segment, style.show_speaker, &style.speaker_names));
        let margin_v = style.margin_v + lane * lane_step;
        dialogue_lines.push(format!(
            "Dialogue: 0,{start},{end},{style_name},,0,0,{margin_v},,{text}",
            start = format_ass_time(segment.start),
            end = format_ass_time(segment.end),
            style_name = style_name,
            margin_v = margin_v,
            text = text
        ));
    }

    let mut lines: Vec<String> = Vec::new();
    lines.push("[Script Info]".to_string());
    lines.push("ScriptType: v4.00+".to_string());
    lines.push("WrapStyle: 2".to_string());
    lines.push("ScaledBorderAndShadow: yes".to_string());
    lines.push(format!("PlayResX: {}", video_width));
    lines.push(format!("PlayResY: {}", video_height));
    lines.push(String::new());
    lines.push("[V4+ Styles]".to_string());
    lines.push("Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding".to_string());
    lines.extend(style_lines);
    lines.push(String::new());
    lines.push("[Events]".to_string());
    lines.push("Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text".to_string());
    lines.extend(dialogue_lines);
    lines.push(String::new());
    lines.join("\n")
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd frontend/src-tauri && cargo test subtitle_export -- --nocapture`
Expected: all tests PASS

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/subtitle_export/mod.rs
git commit -m "feat(subtitle_export): implement export_ass with speaker colors and lane layout"
```

---

## Task 13: Implement `export_json` with tests

**Files:**
- Modify: `frontend/src-tauri/src/subtitle_export/mod.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module:

```rust
    #[test]
    fn test_export_json_basic() {
        let segments = vec![seg("a", 0.0, 2.0, "S01", "hello")];
        let json = export_json(&segments);
        assert!(json.contains("\"id\":\"a\""));
        assert!(json.contains("\"start\":0.0"));
        assert!(json.contains("\"end\":2.0"));
        assert!(json.contains("\"speaker\":\"S01\""));
        assert!(json.contains("\"text\":\"hello\""));
    }

    #[test]
    fn test_export_json_multiple_segments() {
        let segments = vec![
            seg("a", 0.0, 1.0, "S01", "hi"),
            seg("b", 1.0, 2.0, "S02", "yo"),
        ];
        let json = export_json(&segments);
        // Valid JSON array
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(parsed.as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_export_json_empty_input() {
        let segments: Vec<ProcessableSegment> = vec![];
        let json = export_json(&segments);
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert!(parsed.as_array().unwrap().is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd frontend/src-tauri && cargo test subtitle_export::tests::test_export_json -- --nocapture`
Expected: FAIL — `export_json` not found, possibly serde_json not in Cargo.toml

- [ ] **Step 3: Check serde_json is available**

Run: `cd frontend/src-tauri && cargo tree -p serde_json 2>&1 | head -5`
If serde_json is already a dependency (likely via tauri), proceed. If not, add to Cargo.toml `[dependencies]`:
```toml
serde_json = "1"
```

- [ ] **Step 4: Implement export_json**

Add to `frontend/src-tauri/src/subtitle_export/mod.rs`:

```rust
use serde::Serialize;

/// JSON 导出用的序列化结构。
#[derive(Serialize)]
struct SegmentJson<'a> {
    id: &'a str,
    start: f32,
    end: f32,
    speaker: &'a str,
    text: &'a str,
}

/// 导出 JSON 格式字幕。
/// 移植自 MOSS export.py `export_json`。
pub fn export_json(segments: &[ProcessableSegment]) -> String {
    let items: Vec<SegmentJson> = segments
        .iter()
        .map(|s| SegmentJson {
            id: &s.id,
            start: s.start,
            end: s.end,
            speaker: &s.speaker,
            text: &s.text,
        })
        .collect();
    serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".to_string()) + "\n"
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd frontend/src-tauri && cargo test subtitle_export -- --nocapture`
Expected: all tests PASS

- [ ] **Step 6: Commit**

```bash
git add frontend/src-tauri/src/subtitle_export/mod.rs frontend/src-tauri/Cargo.toml
git commit -m "feat(subtitle_export): implement export_json with tests"
```

---

## Task 14: Add `export_subtitle` Tauri command

**Files:**
- Modify: `frontend/src-tauri/src/subtitle_export/mod.rs` (add commands submodule)
- Modify: `frontend/src-tauri/src/lib.rs` (register command)

- [ ] **Step 1: Add the Tauri command**

Append to `frontend/src-tauri/src/subtitle_export/mod.rs`:

```rust
pub mod commands {
    use super::*;
    use crate::database::models::Transcript;
    use crate::database::repositories::transcript::TranscriptRepository;
    use crate::state::AppState;
    use log::{info, warn};
    use tauri::{AppHandle, Manager, State};

    /// 导出会议字幕。
    /// format: "srt" | "ass" | "json"
    /// apply_postprocess: 是否先 normalize（默认 true）
    #[tauri::command]
    pub async fn export_subtitle(
        app: AppHandle,
        state: State<'_, AppState>,
        meeting_id: String,
        format: String,
        show_speaker: Option<bool>,
        speaker_names: Option<HashMap<String, String>>,
        apply_postprocess: Option<bool>,
    ) -> Result<String, String> {
        info!(
            "[SubtitleExport] Exporting meeting {} as {}",
            meeting_id, format
        );

        // 1. 查询 meeting 的所有 transcripts（按 audio_start_time 排序）
        let transcripts: Vec<Transcript> = state
            .db_manager
            .get_transcripts_by_meeting(&meeting_id)
            .await
            .map_err(|e| format!("Failed to query transcripts: {}", e))?;

        if transcripts.is_empty() {
            return Err("No transcripts found for this meeting".to_string());
        }

        // 2. 转换为 ProcessableSegment
        let mut segments: Vec<ProcessableSegment> = transcripts
            .into_iter()
            .map(|t| ProcessableSegment {
                id: t.id,
                start: t.audio_start_time.unwrap_or(0.0) as f32,
                end: t.audio_end_time.unwrap_or(0.0) as f32,
                speaker: t
                    .speaker
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "S00".to_string()),
                text: t.transcript,
            })
            .collect();

        // 3. 可选后处理
        let should_postprocess = apply_postprocess.unwrap_or(true);
        if should_postprocess {
            crate::transcript_postprocess::normalize(
                &mut segments,
                &crate::transcript_postprocess::NormalizeConfig::default(),
            );
        }

        let show_sp = show_speaker.unwrap_or(true);
        let names = speaker_names.unwrap_or_default();

        // 4. 按格式导出
        let result = match format.to_lowercase().as_str() {
            "srt" => export_srt(&segments, show_sp, &names),
            "ass" => export_ass(&segments, &AssStyle {
                speaker_names: names,
                ..Default::default()
            }, 1920, 1080),
            "json" => export_json(&segments),
            other => {
                warn!("[SubtitleExport] Unknown format: {}", other);
                return Err(format!("Unknown format: {}. Use srt, ass, or json.", other));
            }
        };

        info!(
            "[SubtitleExport] Exported {} segments as {} ({} chars)",
            segments.len(),
            format,
            result.len()
        );
        Ok(result)
    }
}
```

- [ ] **Step 2: Register the command in lib.rs**

Edit `frontend/src-tauri/src/lib.rs`. In the `invoke_handler` macro (around line 600 after voiceprint commands), add:

```rust
            // Subtitle export commands
            subtitle_export::commands::export_subtitle,
```

- [ ] **Step 3: Verify it compiles**

Run: `cd frontend/src-tauri && cargo check`
Expected: compiles

Note: If `get_transcripts_by_meeting` doesn't exist on db_manager, check the actual method name in `database/manager.rs` or `database/repositories/transcript.rs` and adjust. The repository likely has `get_by_meeting_id` or similar — use the actual method.

- [ ] **Step 4: Commit**

```bash
git add frontend/src-tauri/src/subtitle_export/mod.rs frontend/src-tauri/src/lib.rs
git commit -m "feat(subtitle_export): add export_subtitle Tauri command"
```

---

## Task 15: Integrate normalize into diarization post-flow (optional, non-destructive)

**Files:**
- Modify: `frontend/src-tauri/src/speaker_diarization_engine/commands.rs`

- [ ] **Step 1: Find the alignment call site**

Search for `align_transcripts_with_speakers` usage in `speaker_diarization_engine/commands.rs`. This is where speakers are assigned to transcript chunks.

- [ ] **Step 2: Add optional normalize call after alignment**

In the function that calls `align_transcripts_with_speakers`, after the alignment result is computed, add an optional normalization step. Since alignment operates on `TranscriptChunkForAlignment` (which has no text field), the normalization should be applied at the export level (Task 14 already does this via `apply_postprocess`).

**Decision point:** If the alignment function returns chunks that will be persisted with text, add normalize there. If text is filled later, skip — normalization happens at export time.

Read the actual function signature and decide. For now, normalization is applied at export time (Task 14), which is non-destructive and sufficient. **This task may be a no-op** if export-time normalization covers the use case.

- [ ] **Step 3: If changes were made, verify compilation**

Run: `cd frontend/src-tauri && cargo check`
Expected: compiles

- [ ] **Step 4: Commit (only if changes were made)**

```bash
git add frontend/src-tauri/src/speaker_diarization_engine/commands.rs
git commit -m "feat(diarization): apply normalize after speaker alignment (optional)"
```

If no changes: skip this commit.

---

## Task 16: Create hotwords migration

**Files:**
- Create: `frontend/src-tauri/migrations/20260724000001_create_hotwords.sql`

- [ ] **Step 1: Create the migration file**

Create `frontend/src-tauri/migrations/20260724000001_create_hotwords.sql`:

```sql
-- 热词表：用于 LLM 转写修正
-- scope = 'global' 适用于所有会议
-- scope = <meeting_id> 仅适用于指定会议
CREATE TABLE hotwords (
    id TEXT PRIMARY KEY,
    word TEXT NOT NULL,
    category TEXT,
    scope TEXT NOT NULL DEFAULT 'global',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_hotwords_scope ON hotwords(scope);
CREATE INDEX idx_hotwords_word ON hotwords(word);
```

- [ ] **Step 2: Commit**

```bash
git add frontend/src-tauri/migrations/20260724000001_create_hotwords.sql
git commit -m "feat(hotword_correction): add hotwords table migration"
```

---

## Task 17: Create Module 3 — hotword_correction repository

**Files:**
- Create: `frontend/src-tauri/src/hotword_correction/mod.rs`
- Create: `frontend/src-tauri/src/hotword_correction/repository.rs`
- Modify: `frontend/src-tauri/src/lib.rs` (add `pub mod hotword_correction;`)

- [ ] **Step 1: Create the module structure**

Create `frontend/src-tauri/src/hotword_correction/mod.rs`:

```rust
// hotword_correction/mod.rs
//
// 热词修正：借鉴 MOSS-Transcribe-Diarize prompts.md 的热词模式，
// 复用现有 summary/llm_client.rs 调用 LLM 修正专有名词。

pub mod repository;
pub mod commands;

use serde::{Deserialize, Serialize};

use crate::transcript_postprocess::ProcessableSegment;

/// 热词数据模型
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Hotword {
    pub id: String,
    pub word: String,
    pub category: Option<String>,
    pub scope: String,
    pub created_at: String,
}
```

Create `frontend/src-tauri/src/hotword_correction/repository.rs`:

```rust
// hotword_correction/repository.rs
//
// 热词表的数据库访问。

use sqlx::SqlitePool;
use log::info;

use super::Hotword;

pub struct HotwordRepository {
    pool: SqlitePool,
}

impl HotwordRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// 查询热词。scope=None 查全局，scope=Some(meeting_id) 查该会议+全局。
    pub async fn list(&self, scope: Option<&str>) -> Result<Vec<Hotword>, String> {
        let rows = match scope {
            None => {
                sqlx::query_as::<_, Hotword>("SELECT * FROM hotwords ORDER BY created_at DESC")
                    .fetch_all(&self.pool)
                    .await
            }
            Some(meeting_id) => {
                sqlx::query_as::<_, Hotword>(
                    "SELECT * FROM hotwords WHERE scope = 'global' OR scope = ? ORDER BY created_at DESC",
                )
                .bind(meeting_id)
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(|e| format!("Failed to query hotwords: {}", e))?;
        Ok(rows)
    }

    /// 新增热词。
    pub async fn add(&self, word: &str, category: Option<&str>, scope: &str) -> Result<String, String> {
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO hotwords (id, word, category, scope) VALUES (?, ?, ?, ?)")
            .bind(&id)
            .bind(word)
            .bind(category)
            .bind(scope)
            .execute(&self.pool)
            .await
            .map_err(|e| format!("Failed to insert hotword: {}", e))?;
        info!("[Hotword] Added: {} (scope={})", word, scope);
        Ok(id)
    }

    /// 删除热词。
    pub async fn delete(&self, id: &str) -> Result<(), String> {
        sqlx::query("DELETE FROM hotwords WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| format!("Failed to delete hotword: {}", e))?;
        Ok(())
    }
}
```

- [ ] **Step 2: Register module in lib.rs**

Edit `frontend/src-tauri/src/lib.rs`, add after `pub mod subtitle_export;`:

```rust
pub mod hotword_correction;
```

- [ ] **Step 3: Verify uuid crate is available**

Run: `cd frontend/src-tauri && cargo tree -p uuid 2>&1 | head -3`
If not available, add to Cargo.toml `[dependencies]`:
```toml
uuid = { version = "1", features = ["v4"] }
```

- [ ] **Step 4: Verify it compiles**

Run: `cd frontend/src-tauri && cargo check`
Expected: compiles

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/hotword_correction/ frontend/src-tauri/src/lib.rs frontend/src-tauri/Cargo.toml
git commit -m "feat(hotword_correction): scaffold module with Hotword model and repository"
```

---

## Task 18: Implement hotword prompt builder and output parser with tests

**Files:**
- Modify: `frontend/src-tauri/src/hotword_correction/mod.rs`

- [ ] **Step 1: Write the failing tests**

Add to `frontend/src-tauri/src/hotword_correction/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn seg(id: &str, start: f32, end: f32, speaker: &str, text: &str) -> ProcessableSegment {
        ProcessableSegment {
            id: id.to_string(),
            start,
            end,
            speaker: speaker.to_string(),
            text: text.to_string(),
        }
    }

    #[test]
    fn test_build_prompt_includes_hotwords() {
        let segments = vec![seg("a", 0.0, 1.0, "S01", "hello")];
        let hotwords = vec!["审计法".to_string(), "财政部".to_string()];
        let prompt = build_correction_prompt(&segments, &hotwords);
        assert!(prompt.contains("审计法"));
        assert!(prompt.contains("财政部"));
        assert!(prompt.contains("[S01]"));
        assert!(prompt.contains("hello"));
    }

    #[test]
    fn test_build_prompt_empty_hotwords() {
        let segments = vec![seg("a", 0.0, 1.0, "S01", "hi")];
        let hotwords = vec![];
        let prompt = build_correction_prompt(&segments, &hotwords);
        // Should still produce a valid prompt, just no hotwords listed
        assert!(prompt.contains("[S01]"));
        assert!(prompt.contains("hi"));
    }

    #[test]
    fn test_parse_correction_output_basic() {
        let output = "[S01] (0.00-1.00) hello world\n[S02] (1.00-2.00) foo bar";
        let segments = vec![
            seg("a", 0.0, 1.0, "S01", "hello"),
            seg("b", 1.0, 2.0, "S02", "foo"),
        ];
        let parsed = parse_correction_output(output, &segments);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].text, "hello world");
        assert_eq!(parsed[1].text, "foo bar");
    }

    #[test]
    fn test_parse_correction_output_fallback_on_malformed() {
        let output = "garbage output";
        let segments = vec![seg("a", 0.0, 1.0, "S01", "original")];
        let parsed = parse_correction_output(output, &segments);
        // Fallback: return original segments unchanged
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].text, "original");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd frontend/src-tauri && cargo test hotword_correction::tests -- --nocapture`
Expected: FAIL — `build_correction_prompt` / `parse_correction_output` not found

- [ ] **Step 3: Implement prompt builder and parser**

Add to `frontend/src-tauri/src/hotword_correction/mod.rs`:

```rust
/// 构建热词修正 prompt。借鉴 MOSS prompts.md 的热词模式。
pub fn build_correction_prompt(segments: &[ProcessableSegment], hotwords: &[String]) -> String {
    let hotword_str = if hotwords.is_empty() {
        "（无热词）".to_string()
    } else {
        hotwords.join("、")
    };

    let mut transcript_lines = Vec::new();
    for seg in segments {
        transcript_lines.push(format!(
            "[{}] ({:.2}-{:.2}) {}",
            seg.speaker, seg.start, seg.end, seg.text
        ));
    }
    let transcript_block = transcript_lines.join("\n");

    format!(
        "请修正以下会议转写文本中的专有名词错误。\n\n\
         热词提示：{hotwords}\n\n\
         转写文本（按段）：\n{transcript}\n\n\
         要求：\n\
         1. 仅修正专有名词（人名/机构/法规/术语），使其匹配热词\n\
         2. 不改变语义和句子结构\n\
         3. 保持 [Sxx] (start-end) text 格式输出\n\
         4. 修正词用热词中的正确写法",
        hotwords = hotword_str,
        transcript = transcript_block
    )
}

/// 解析 LLM 修正输出。格式：`[Sxx] (start-end) text`。
/// 解析失败则回退到原始 segments（非破坏性）。
pub fn parse_correction_output(
    output: &str,
    original_segments: &[ProcessableSegment],
) -> Vec<ProcessableSegment> {
    let mut result: Vec<ProcessableSegment> = Vec::new();
    let mut original_iter = original_segments.iter();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // 期望格式: [Sxx] (start-end) text
        // 简单解析：找 ] 和 ) 之后的部分作为 text
        if let Some(text_start) = line.find(')') {
            let text = line[text_start + 1..].trim();
            if let Some(orig) = original_iter.next() {
                result.push(ProcessableSegment {
                    id: orig.id.clone(),
                    start: orig.start,
                    end: orig.end,
                    speaker: orig.speaker.clone(),
                    text: text.to_string(),
                });
            }
        } else {
            // 无法解析此行，跳过（保持与原始段对齐）
            if let Some(orig) = original_iter.next() {
                result.push(orig.clone());
            }
        }
    }

    // 若 LLM 输出行数少于原始段，补齐剩余原始段
    for orig in original_iter {
        result.push(orig.clone());
    }

    // 若结果为空（完全无法解析），回退到原始
    if result.is_empty() {
        return original_segments.to_vec();
    }
    result
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd frontend/src-tauri && cargo test hotword_correction::tests -- --nocapture`
Expected: 4 tests PASS

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/hotword_correction/mod.rs
git commit -m "feat(hotword_correction): implement prompt builder and output parser with tests"
```

---

## Task 19: Implement hotword correction commands (Tauri)

**Files:**
- Create: `frontend/src-tauri/src/hotword_correction/commands.rs`
- Modify: `frontend/src-tauri/src/lib.rs` (register commands)

- [ ] **Step 1: Create the commands file**

Create `frontend/src-tauri/src/hotword_correction/commands.rs`:

```rust
// hotword_correction/commands.rs
//
// Tauri commands: 热词 CRUD + 转写修正。

use std::collections::HashMap;
use log::{info, warn};
use tauri::{AppHandle, Emitter, Manager, State};

use super::repository::HotwordRepository;
use super::{build_correction_prompt, parse_correction_output, Hotword};
use crate::database::models::Transcript;
use crate::state::AppState;
use crate::summary::llm_client::{LLMProvider, LlmConfig};
use crate::transcript_postprocess::ProcessableSegment;

/// 获取热词列表。scope=None 查全部，scope=Some(meeting_id) 查该会议+全局。
#[tauri::command]
pub async fn get_hotwords(
    state: State<'_, AppState>,
    scope: Option<String>,
) -> Result<Vec<Hotword>, String> {
    let repo = HotwordRepository::new(state.db_manager.pool().await);
    repo.list(scope.as_deref()).await
}

/// 新增热词。
#[tauri::command]
pub async fn add_hotword(
    state: State<'_, AppState>,
    word: String,
    category: Option<String>,
    scope: Option<String>,
) -> Result<String, String> {
    let repo = HotwordRepository::new(state.db_manager.pool().await);
    let scope = scope.unwrap_or_else(|| "global".to_string());
    repo.add(&word, category.as_deref(), &scope).await
}

/// 删除热词。
#[tauri::command]
pub async fn delete_hotword(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let repo = HotwordRepository::new(state.db_manager.pool().await);
    repo.delete(&id).await
}

/// 修正会议转写文本中的专有名词（基于热词 + LLM）。
/// 通过 Tauri event 上报进度：hotword-correction-start/progress/complete/error
#[tauri::command]
pub async fn correct_transcript_with_hotwords(
    app: AppHandle,
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<Vec<ProcessableSegment>, String> {
    info!("[HotwordCorrection] Starting for meeting {}", meeting_id);
    let _ = app.emit("hotword-correction-start", &meeting_id);

    // 1. 加载热词（全局 + 该会议）
    let repo = HotwordRepository::new(state.db_manager.pool().await);
    let hotwords: Vec<String> = repo
        .list(Some(&meeting_id))
        .await?
        .into_iter()
        .map(|h| h.word)
        .collect();

    if hotwords.is_empty() {
        warn!("[HotwordCorrection] No hotwords configured, skipping");
        let _ = app.emit("hotword-correction-error", "No hotwords configured");
        return Err("No hotwords configured".to_string());
    }

    // 2. 加载转写段
    let transcripts: Vec<Transcript> = state
        .db_manager
        .get_transcripts_by_meeting(&meeting_id)
        .await
        .map_err(|e| format!("Failed to query transcripts: {}", e))?;

    let segments: Vec<ProcessableSegment> = transcripts
        .into_iter()
        .map(|t| ProcessableSegment {
            id: t.id,
            start: t.audio_start_time.unwrap_or(0.0) as f32,
            end: t.audio_end_time.unwrap_or(0.0) as f32,
            speaker: t.speaker.filter(|s| !s.is_empty()).unwrap_or_else(|| "S00".to_string()),
            text: t.transcript,
        })
        .collect();

    // 3. 构建 prompt
    let prompt = build_correction_prompt(&segments, &hotwords);
    let _ = app.emit("hotword-correction-progress", "Calling LLM");

    // 4. 调用 LLM（复用 summary/llm_client）
    //    需要从 settings 获取 LLM 配置。这里简化：使用默认配置。
    //    实际实现应从 AppState 读取用户配置的 LLM provider/key/model。
    let llm_config = LlmConfig {
        provider: LLMProvider::Ollama, // 默认本地 Ollama
        model: "qwen2.5:7b".to_string(),
        api_key: None,
        endpoint: Some("http://localhost:11434".to_string()),
    };

    let llm_response = crate::summary::llm_client::call_llm(&llm_config, &prompt, 4096)
        .await
        .map_err(|e| {
            let msg = format!("LLM call failed: {}", e);
            let _ = app.emit("hotword-correction-error", &msg);
            msg
        })?;

    // 5. 解析输出
    let corrected = parse_correction_output(&llm_response, &segments);
    let _ = app.emit("hotword-correction-complete", &corrected);

    info!(
        "[HotwordCorrection] Completed: {} segments corrected",
        corrected.len()
    );
    Ok(corrected)
}
```

- [ ] **Step 2: Register commands in lib.rs**

Edit `frontend/src-tauri/src/lib.rs`, add after `subtitle_export::commands::export_subtitle,`:

```rust
            // Hotword correction commands
            hotword_correction::commands::get_hotwords,
            hotword_correction::commands::add_hotword,
            hotword_correction::commands::delete_hotword,
            hotword_correction::commands::correct_transcript_with_hotwords,
```

- [ ] **Step 3: Verify it compiles (fix method signatures as needed)**

Run: `cd frontend/src-tauri && cargo check`

Note: Several details may need adjustment based on actual API:
- `state.db_manager.pool().await` — check actual method to get SqlitePool
- `state.db_manager.get_transcripts_by_meeting()` — check actual method name
- `crate::summary::llm_client::call_llm()` — check actual function signature
- `LlmConfig` fields — check actual struct definition

Adjust imports and calls to match actual signatures. This is expected — the plan provides the structure, the engineer verifies against actual code.

- [ ] **Step 4: Commit**

```bash
git add frontend/src-tauri/src/hotword_correction/commands.rs frontend/src-tauri/src/lib.rs
git commit -m "feat(hotword_correction): add Tauri commands for hotword CRUD and transcript correction"
```

---

## Task 20: Final integration verification

**Files:** None (verification only)

- [ ] **Step 1: Run full test suite**

Run: `cd frontend/src-tauri && cargo test -- --nocapture`
Expected: all tests PASS (transcript_postprocess + subtitle_export + hotword_correction + existing tests)

- [ ] **Step 2: Verify compilation in release mode**

Run: `cd frontend/src-tauri && cargo check --release`
Expected: compiles without errors

- [ ] **Step 3: Verify all new commands are registered**

Run: `cd frontend/src-tauri && cargo build 2>&1 | grep -i "warning.*unused"` 
Check there are no "unused import" or "unused function" warnings for the new modules.

- [ ] **Step 4: Manual smoke test (if dev environment available)**

Run: `cd frontend && pnpm run tauri:dev`
In the app, verify:
1. (If frontend UI added) Export subtitle button works — produces downloadable SRT/ASS/JSON
2. (If frontend UI added) Hotword management works — add/delete hotwords
3. Existing recording/transcription still works (no regression)

If frontend UI is not part of this plan, verify via Tauri devtools console:
```javascript
await window.__TAURI__.invoke('export_subtitle', { meetingId: '<id>', format: 'srt' })
await window.__TAURI__.invoke('get_hotwords', { scope: null })
```

- [ ] **Step 5: Final commit**

```bash
git add -A
git commit -m "chore: integration verification for MOSS postprocess enhancement"
```

---

## Summary

| Task | Module | Priority | Deliverable |
|---|---|---|---|
| 1-8 | transcript_postprocess | P0 | ProcessableSegment + normalize/merge/split/fix_overlaps + adapters |
| 9-13 | subtitle_export | P0 | export_srt/export_ass/export_json + assign_overlap_lanes |
| 14 | subtitle_export | P0 | export_subtitle Tauri command |
| 15 | diarization | P0 | Optional normalize integration (may be no-op) |
| 16-19 | hotword_correction | P1 | Migration + repository + prompt/parser + Tauri commands |
| 20 | all | P0 | Integration verification |

**Total: 20 tasks, all TDD, frequent commits.**
