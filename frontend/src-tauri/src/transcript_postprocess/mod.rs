// transcript_postprocess/mod.rs
//
// 移植自 MOSS-Transcribe-Diarize 的 subtitle/postprocess.py 算法。
// 纯 Rust 实现，仅依赖 serde，用于对转录分段进行规范化处理：
// 修复时间重叠、合并相邻同说话人分段、拆分过长分段。

use serde::{Deserialize, Serialize};

use crate::speaker_diarization_engine::engine::TranscriptChunkForAlignment;

/// 可处理的转录分段。对应 postprocess.py 中的 ProcessableSegment。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessableSegment {
    pub id: String,
    pub start: f32,
    pub end: f32,
    pub speaker: String,
    pub text: String,
}

/// 规范化处理的配置参数。
#[derive(Debug, Clone)]
pub struct NormalizeConfig {
    /// 最小分段时长（秒），过短的分段会被延长到此值
    pub min_duration: f32,
    /// 最大分段时长（秒），超过此值且文本过长时会尝试拆分
    pub max_duration: f32,
    /// 单段最大字符数，用于合并/拆分判定
    pub max_chars: usize,
    /// 合并相邻分段的最大时间间隔（秒）
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

/// 修复分段时间重叠：保证时间单调递增、不重叠，且每段至少 min_duration。
/// 算法：游标从 0.0 开始，对每段 start=max(seg.start, cursor)，
/// end=max(seg.end, start+min_duration)，然后 cursor=end。
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

/// 拼接两段文本：若左侧末字符与右侧首字符均为 ASCII 则加空格，否则直接拼接。
/// 空字符串处理：左空返回右，右空返回左。
fn join_text(left: &str, right: &str) -> String {
    if left.is_empty() {
        return right.to_string();
    }
    if right.is_empty() {
        return left.to_string();
    }
    let left_last = left.chars().last().unwrap();
    let right_first = right.chars().next().unwrap();
    if left_last.is_ascii() && right_first.is_ascii() {
        format!("{} {}", left, right)
    } else {
        format!("{}{}", left, right)
    }
}

/// 合并相邻同说话人分段。条件：同一说话人且间隔 ∈ [0, merge_gap]
/// 且合并后文本字符数 ≤ 2*max_chars。合并后 end=max(prev.end, seg.end)，
/// 文本由 join_text 拼接。
pub fn merge_adjacent(segments: &mut Vec<ProcessableSegment>, merge_gap: f32, max_chars: usize) {
    if segments.is_empty() {
        return;
    }
    let mut result: Vec<ProcessableSegment> = Vec::with_capacity(segments.len());
    for seg in segments.drain(..) {
        if let Some(last) = result.last_mut() {
            let gap = seg.start - last.end;
            let same_speaker = last.speaker == seg.speaker;
            let combined = last.text.chars().count() + seg.text.chars().count();
            if same_speaker && gap >= 0.0 && gap <= merge_gap && combined <= 2 * max_chars {
                last.end = last.end.max(seg.end);
                last.text = join_text(&last.text, &seg.text);
                continue;
            }
        }
        result.push(seg);
    }
    *segments = result;
}

/// 标点符号集合，用于拆分文本时的断句。
const PUNCTUATION: &str = "。！？!?；;，、 ";

/// 按标点和长度上限拆分文本。
/// 断句条件：当前长度 ≥ max_chars，或遇到标点且当前长度 ≥ max_chars/2。
/// 初步拆分后再做一次紧凑合并：相邻块合并后若 ≤ max_chars 则合并。
/// 使用 .chars().count() 统计字符数（非字节数）。空文本返回空 Vec。
fn split_text(text: &str, max_chars: usize) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        let cur_len = current.chars().count();
        if cur_len >= max_chars || (PUNCTUATION.contains(ch) && cur_len >= max_chars / 2) {
            chunks.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    // 紧凑合并：相邻块合并后若 ≤ max_chars 则合并
    let mut compacted: Vec<String> = Vec::new();
    for chunk in chunks {
        if let Some(last) = compacted.last_mut() {
            if last.chars().count() + chunk.chars().count() <= max_chars {
                last.push_str(&chunk);
                continue;
            }
        }
        compacted.push(chunk);
    }
    compacted
}

/// 拆分过长分段：仅当 duration > max_duration 且文本字符数 > max_chars 时拆分。
/// 按字符比例分配时间：ratio = chunk_chars / total_chars，
/// proposed_end = cursor + max(min_duration, duration * ratio)，
/// 并保证后续块有 min_duration 空间：end = min(proposed, seg.end - min_duration*remaining)。
/// 最后一块取 seg.end。子段 id = "{parent_id}_{index+1}"。
/// 仅 1 块或无法拆分时保留原段。
pub fn split_long_segments(
    segments: &mut Vec<ProcessableSegment>,
    min_duration: f32,
    max_duration: f32,
    max_chars: usize,
) {
    let mut result: Vec<ProcessableSegment> = Vec::new();
    for seg in segments.drain(..) {
        let duration = seg.end - seg.start;
        let text_len = seg.text.chars().count();
        if duration > max_duration && text_len > max_chars {
            let chunks = split_text(&seg.text, max_chars);
            if chunks.len() <= 1 {
                result.push(seg);
                continue;
            }
            let total_chars = text_len;
            let n = chunks.len();
            let mut cursor = seg.start;
            for (i, chunk) in chunks.iter().enumerate() {
                let is_last = i == n - 1;
                let end;
                if is_last {
                    end = seg.end;
                } else {
                    let chunk_chars = chunk.chars().count();
                    let ratio = chunk_chars as f32 / total_chars as f32;
                    let proposed = cursor + (min_duration).max(duration * ratio);
                    let remaining = (n - i - 1) as f32;
                    end = proposed
                        .min(seg.end - min_duration * remaining)
                        .max(cursor + min_duration);
                }
                result.push(ProcessableSegment {
                    id: format!("{}_{}", seg.id, i + 1),
                    start: cursor,
                    end,
                    speaker: seg.speaker.clone(),
                    text: chunk.clone(),
                });
                cursor = end;
            }
        } else {
            result.push(seg);
        }
    }
    *segments = result;
}

/// 预处理分段：去除文本首尾空白、跳过空文本段、
/// 将 start 钳制到 ≥ 0.0、end 钳制到 ≥ start，并按 (start, end) 排序。
fn prepare_segments(segments: Vec<ProcessableSegment>) -> Vec<ProcessableSegment> {
    let mut result: Vec<ProcessableSegment> = Vec::with_capacity(segments.len());
    for mut seg in segments {
        seg.text = seg.text.trim().to_string();
        if seg.text.is_empty() {
            continue;
        }
        if seg.start.is_nan() {
            log::warn!("[transcript_postprocess] NaN start time detected, clamping to 0.0");
            seg.start = 0.0;
        }
        if seg.end.is_nan() {
            log::warn!("[transcript_postprocess] NaN end time detected, clamping to 0.0");
            seg.end = 0.0;
        }
        seg.start = seg.start.max(0.0);
        seg.end = seg.end.max(seg.start);
        result.push(seg);
    }
    result.sort_by(|a, b| {
        a.start.total_cmp(&b.start).then(a.end.total_cmp(&b.end))
    });
    result
}

/// 完整规范化流水线：
/// prepare_segments → fix_overlaps(min_duration) → merge_adjacent(merge_gap, max_chars)
/// → split_long_segments(min_duration, max_duration, max_chars) → fix_overlaps(min_duration)
pub fn normalize(segments: &mut Vec<ProcessableSegment>, config: &NormalizeConfig) {
    *segments = prepare_segments(std::mem::take(segments));
    fix_overlaps(segments, config.min_duration);
    merge_adjacent(segments, config.merge_gap, config.max_chars);
    split_long_segments(segments, config.min_duration, config.max_duration, config.max_chars);
    fix_overlaps(segments, config.min_duration);
}

/// 从 TranscriptChunkForAlignment 适配为 ProcessableSegment。
/// 对齐用 chunk 不含文本，text 留空由调用方填充。
/// speaker 为 Option<i32>：Some(n) → "S{nn}"（零填充两位），None → "S00"。
impl From<TranscriptChunkForAlignment> for ProcessableSegment {
    fn from(chunk: TranscriptChunkForAlignment) -> Self {
        ProcessableSegment {
            id: chunk.id,
            start: chunk.audio_start_time as f32,
            end: chunk.audio_end_time as f32,
            speaker: chunk
                .speaker
                .map(|s| format!("S{:02}", s))
                .unwrap_or_else(|| "S00".to_string()),
            text: String::new(),
        }
    }
}

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

    // ---- fix_overlaps ----

    #[test]
    fn test_fix_overlaps_no_overlap_unchanged() {
        let mut segments = vec![
            seg("a", 0.0, 2.0, "S01", "x"),
            seg("b", 2.0, 4.0, "S01", "y"),
        ];
        fix_overlaps(&mut segments, 1.0);
        assert_eq!(segments[0].start, 0.0);
        assert_eq!(segments[0].end, 2.0);
        assert_eq!(segments[1].start, 2.0);
        assert_eq!(segments[1].end, 4.0);
    }

    #[test]
    fn test_fix_overlaps_with_overlap_pushes_forward() {
        let mut segments = vec![
            seg("a", 0.0, 2.0, "S01", "x"),
            seg("b", 1.5, 3.0, "S01", "y"),
        ];
        fix_overlaps(&mut segments, 1.0);
        assert_eq!(segments[1].start, 2.0);
    }

    #[test]
    fn test_fix_overlaps_extends_short_segment_to_min_duration() {
        let mut segments = vec![seg("a", 0.0, 0.3, "S01", "x")];
        fix_overlaps(&mut segments, 1.0);
        assert_eq!(segments[0].end, 1.0);
    }

    #[test]
    fn test_fix_overlaps_empty_input() {
        let mut segments: Vec<ProcessableSegment> = vec![];
        fix_overlaps(&mut segments, 1.0);
        assert!(segments.is_empty());
    }

    // ---- join_text ----

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

    // ---- merge_adjacent ----

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
        let long = "a".repeat(40);
        let mut segments = vec![
            seg("a", 0.0, 2.0, "S01", &long),
            seg("b", 2.1, 3.0, "S01", &long),
        ];
        merge_adjacent(&mut segments, 0.3, 24);
        assert_eq!(segments.len(), 2);
    }

    #[test]
    fn test_merge_adjacent_negative_gap_no_merge() {
        let mut segments = vec![
            seg("a", 0.0, 3.0, "S01", "hello"),
            seg("b", 2.5, 4.0, "S01", "world"),
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

    // ---- split_text ----

    #[test]
    fn test_split_text_short_text_unchanged() {
        let result = split_text("hello", 24);
        assert_eq!(result, vec!["hello".to_string()]);
    }

    #[test]
    fn test_split_text_cjk_by_punctuation() {
        let text = "今天天气很好。我们去公园散步。晚上回家吃饭。";
        let result = split_text(text, 10);
        assert!(
            result.len() >= 2,
            "expected >=2 chunks, got {}",
            result.len()
        );
        for chunk in &result {
            assert!(chunk.chars().count() <= 10, "chunk too long: {}", chunk);
        }
    }

    #[test]
    fn test_split_text_forced_cut_at_max_chars() {
        let text = "a".repeat(26);
        let result = split_text(&text, 10);
        assert!(
            result.len() >= 3,
            "expected >=3 chunks, got {}",
            result.len()
        );
    }

    #[test]
    fn test_split_text_empty_returns_empty() {
        let result = split_text("", 24);
        assert!(result.is_empty());
    }

    // ---- split_long_segments ----

    #[test]
    fn test_split_long_short_segment_unchanged() {
        let mut segments = vec![seg("a", 0.0, 2.0, "S01", "hello")];
        split_long_segments(&mut segments, 1.0, 6.0, 24);
        assert_eq!(segments.len(), 1);
    }

    #[test]
    fn test_split_long_splits_by_punctuation() {
        let text = "今天天气很好。我们去公园散步。晚上回家吃饭。";
        let mut segments = vec![seg("a", 0.0, 10.0, "S01", text)];
        split_long_segments(&mut segments, 1.0, 6.0, 10);
        assert!(
            segments.len() >= 2,
            "expected >=2 segments, got {}",
            segments.len()
        );
    }

    #[test]
    fn test_split_long_only_duration_exceeds_keeps_one() {
        let mut segments = vec![seg("a", 0.0, 10.0, "S01", "hello")];
        split_long_segments(&mut segments, 1.0, 6.0, 24);
        assert_eq!(segments.len(), 1);
    }

    #[test]
    fn test_split_long_preserves_total_time_range() {
        let text = "今天天气很好。我们去公园散步。晚上回家吃饭。";
        let mut segments = vec![seg("a", 0.0, 10.0, "S01", text)];
        split_long_segments(&mut segments, 1.0, 6.0, 10);
        assert!(!segments.is_empty());
        assert_eq!(segments.first().unwrap().start, 0.0);
        assert_eq!(segments.last().unwrap().end, 10.0);
    }

    // ---- normalize ----

    #[test]
    fn test_normalize_empty_input() {
        let mut segments: Vec<ProcessableSegment> = vec![];
        normalize(&mut segments, &NormalizeConfig::default());
        assert!(segments.is_empty());
    }

    #[test]
    fn test_normalize_strips_empty_text_segments() {
        let mut segments = vec![seg("a", 0.0, 2.0, "S01", "   ")];
        normalize(&mut segments, &NormalizeConfig::default());
        assert!(segments.is_empty());
    }

    #[test]
    fn test_normalize_sorts_by_start_time() {
        let mut segments = vec![
            seg("b", 5.0, 7.0, "S01", "world"),
            seg("a", 0.0, 2.0, "S01", "hello"),
        ];
        normalize(&mut segments, &NormalizeConfig::default());
        assert!(segments.len() >= 1);
        assert_eq!(segments[0].id, "a");
    }

    #[test]
    fn test_normalize_clamps_negative_start() {
        let mut segments = vec![seg("a", -1.0, 2.0, "S01", "hello")];
        normalize(&mut segments, &NormalizeConfig::default());
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].start, 0.0);
    }

    #[test]
    fn test_normalize_full_pipeline_merges_short_segments() {
        let mut segments = vec![
            seg("a", 0.0, 1.0, "S01", "hello"),
            seg("b", 1.1, 2.0, "S01", "world"),
            seg("c", 2.1, 3.0, "S01", "foo"),
        ];
        normalize(&mut segments, &NormalizeConfig::default());
        assert_eq!(
            segments.len(),
            1,
            "expected merged into 1, got {}: {:?}",
            segments.len(),
            segments
        );
        assert_eq!(segments[0].text, "hello world foo");
    }

    // ---- From<TranscriptChunkForAlignment> ----

    #[test]
    fn test_from_transcript_chunk_for_alignment() {
        let chunk = TranscriptChunkForAlignment {
            id: "c1".to_string(),
            audio_start_time: 1.5,
            audio_end_time: 3.5,
            speaker: Some(2),
        };
        let s: ProcessableSegment = chunk.into();
        assert_eq!(s.id, "c1");
        assert_eq!(s.start, 1.5);
        assert_eq!(s.end, 3.5);
        assert_eq!(s.speaker, "S02");
        assert_eq!(s.text, "");
    }

    #[test]
    fn test_from_transcript_chunk_for_alignment_no_speaker() {
        let chunk = TranscriptChunkForAlignment {
            id: "c2".to_string(),
            audio_start_time: 0.0,
            audio_end_time: 1.0,
            speaker: None,
        };
        let s: ProcessableSegment = chunk.into();
        assert_eq!(s.speaker, "S00");
    }

    #[test]
    fn test_normalize_handles_nan_times() {
        let mut segments = vec![
            ProcessableSegment {
                id: "a".to_string(),
                start: f32::NAN,
                end: 2.0,
                speaker: "S01".to_string(),
                text: "hello".to_string(),
            },
        ];
        normalize(&mut segments, &NormalizeConfig::default());
        assert_eq!(segments[0].start, 0.0);
    }

    #[test]
    fn test_split_long_segments_id_format() {
        let long_text = "今天天气很好。我们去公园散步。晚上回家吃饭。明天再继续。";
        let mut segments = vec![seg("orig", 0.0, 20.0, "S01", long_text)];
        split_long_segments(&mut segments, 1.0, 6.0, 10);
        assert!(segments.len() >= 2);
        for (i, s) in segments.iter().enumerate() {
            assert_eq!(s.id, format!("orig_{}", i + 1));
        }
    }
}
