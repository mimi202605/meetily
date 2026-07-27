// subtitle_export/mod.rs
//
// 移植自 MOSS-Transcribe-Diarize 的 subtitle/export.py 与 subtitle/layout.py。
// 纯 Rust 实现，仅依赖 serde / serde_json（通过 tauri 间接引入），
// 用于将转录分段导出为 SRT / ASS / JSON 字幕格式。

use std::collections::HashMap;

use crate::transcript_postprocess::ProcessableSegment;

/// 说话人颜色循环表（ASS BGR 格式，AlphaBBGGRR）。
/// 对应 export.py 中的 SPEAKER_COLORS。
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

/// ASS 字幕样式配置。对应 export.py 中的 AssStyle。
pub struct AssStyle {
    /// 字体名称，默认 "Noto Sans CJK SC"
    pub font_name: String,
    /// 字体大小；None → max(24, video_height * 0.045)
    pub font_size: Option<usize>,
    /// 对齐方式（ASS numpad），默认 2（底部居中）
    pub alignment: usize,
    /// 垕直边距，默认 56
    pub margin_v: usize,
    /// 描边宽度，默认 3
    pub outline: usize,
    /// 阴影距离，默认 1
    pub shadow: usize,
    /// 是否在字幕中显示说话人标签，默认 true
    pub show_speaker: bool,
    /// 是否为每个说话人生成独立颜色样式，默认 true
    pub speaker_colors: bool,
    /// 主文字颜色，默认白色
    pub primary_color: String,
    /// 描边颜色，默认黑色
    pub outline_color: String,
    /// 背景颜色（阴影/背景框），默认半透明黑
    pub back_color: String,
    /// 说话人 ID → 显示名称的映射，默认空
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

/// 为分段分配重叠车道。
/// 算法：按 (start, end, 原始索引) 排序；对每段找到第一个 lane_end <= start 的车道复用，
/// 否则新建车道。车道 0 = 底部，编号越大越往上堆叠。
/// 返回值按原始顺序对应每段的车道编号。
pub fn assign_overlap_lanes(segments: &[ProcessableSegment]) -> Vec<usize> {
    if segments.is_empty() {
        return Vec::new();
    }
    // 带（原始索引, 段引用）的列表，排序后用于贪心分配
    let mut indexed: Vec<(usize, &ProcessableSegment)> = segments.iter().enumerate().collect();
    indexed.sort_by(|a, b| {
        a.1.start
            .total_cmp(&b.1.start)
            .then(a.1.end.total_cmp(&b.1.end))
            .then(a.0.cmp(&b.0))
    });

    // lanes[i] = 第 i 条车道当前的结束时间
    let mut lanes: Vec<f32> = Vec::new();
    let mut result = vec![0usize; segments.len()];

    for (orig_idx, seg) in indexed {
        // 找到第一条 lane_end <= seg.start 的车道复用
        let reused = lanes.iter().position(|lane_end| *lane_end <= seg.start);
        let lane = match reused {
            Some(i) => {
                lanes[i] = seg.end;
                i
            }
            None => {
                lanes.push(seg.end);
                lanes.len() - 1
            }
        };
        result[orig_idx] = lane;
    }
    result
}

/// 将秒数格式化为 SRT 时间戳："HH:MM:SS,mmm"（毫秒前为逗号）。
pub fn format_srt_time(seconds: f32) -> String {
    // 负数钳制为 0，避免出现 -01:00 之类异常输出
    let total_ms = ((seconds * 1000.0).round() as i64).max(0);
    let hours = total_ms / 3_600_000;
    let minutes = (total_ms % 3_600_000) / 60_000;
    let secs = (total_ms % 60_000) / 1000;
    let ms = total_ms % 1000;
    format!("{:02}:{:02}:{:02},{:03}", hours, minutes, secs, ms)
}

/// 将秒数格式化为 ASS 时间戳："H:MM:SS.cc"（百分秒前为点，小时无前导零）。
pub fn format_ass_time(seconds: f32) -> String {
    let total_cs = ((seconds * 100.0).round() as i64).max(0);
    let hours = total_cs / 360_000;
    let minutes = (total_cs % 360_000) / 6_000;
    let secs = (total_cs % 6_000) / 100;
    let cs = total_cs % 100;
    format!("{}:{:02}:{:02}.{:02}", hours, minutes, secs, cs)
}

/// 生成单条字幕的显示文本。
/// 若 show_speaker 且 speaker 非空 → "{name_or_id}: {text}"（speaker_names 可覆盖显示名）；
/// 否则直接返回 text。
fn display_text(
    segment: &ProcessableSegment,
    show_speaker: bool,
    speaker_names: &HashMap<String, String>,
) -> String {
    if show_speaker && !segment.speaker.is_empty() {
        let name = speaker_names
            .get(&segment.speaker)
            .map(|s| s.as_str())
            .unwrap_or(&segment.speaker);
        format!("{}: {}", name, segment.text)
    } else {
        segment.text.clone()
    }
}

/// 将分段导出为 SRT 字幕字符串。
/// 格式：序号\n开始 --> 结束\n文本\n\n（块间用空行分隔，末尾保留换行）。
pub fn export_srt(
    segments: &[ProcessableSegment],
    show_speaker: bool,
    speaker_names: &HashMap<String, String>,
) -> String {
    let mut out = String::new();
    for (i, seg) in segments.iter().enumerate() {
        let start = format_srt_time(seg.start);
        let end = format_srt_time(seg.end);
        let text = display_text(seg, show_speaker, speaker_names);
        out.push_str(&format!("{}\n{} --> {}\n{}\n\n", i + 1, start, end, text));
    }
    out
}

/// 生成一条 ASS V4+ Style 行。
/// 字段顺序遵循 ASS 标准：Name,Fontname,Fontsize,PrimaryColour,SecondaryColour,
/// OutlineColour,BackColour,Bold,Italic,Underline,StrikeOut,ScaleX,ScaleY,Spacing,
/// Angle,BorderStyle,Outline,Shadow,Alignment,MarginL,MarginR,MarginV,Encoding。
fn ass_style_line(name: &str, style: &AssStyle, font_size: usize, primary_color: &str) -> String {
    format!(
        "Style: {},{},{},{},{},{},{},0,0,0,0,100,100,0,0,1,{},{},{},10,10,{},1",
        name,
        style.font_name,
        font_size,
        primary_color,
        primary_color,        // SecondaryColour 沿用主色
        style.outline_color,  // OutlineColour
        style.back_color,     // BackColour
        style.outline,
        style.shadow,
        style.alignment,
        style.margin_v,
    )
}

/// 根据说话人 ID 生成样式名："Speaker_{仅保留字母数字下划线}"。
fn speaker_style_name(speaker: &str) -> String {
    let sanitized: String = speaker
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    format!("Speaker_{}", sanitized)
}

/// ASS 文本转义：\ → \\，{ → (，} → )，换行 → \N。
fn ass_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '{' => out.push('('),
            '}' => out.push(')'),
            '\n' => out.push_str("\\N"),
            c => out.push(c),
        }
    }
    out
}

/// 将分段导出为 ASS 字幕字符串。
/// 包含 [Script Info] / [V4+ Styles] / [Events] 三段；
/// 若 speaker_colors 则按说话人排序生成对应样式，颜色取自 SPEAKER_COLORS 循环；
/// 对话行使用 assign_overlap_lanes 计算垂直堆叠，margin_v = style.margin_v + lane * max(1, font_size)。
pub fn export_ass(
    segments: &[ProcessableSegment],
    style: &AssStyle,
    video_width: usize,
    video_height: usize,
) -> String {
    // 字体大小：显式指定则用之，否则 max(24, video_height * 0.045)
    let font_size = style
        .font_size
        .unwrap_or_else(|| std::cmp::max(24, (video_height as f64 * 0.045) as usize));

    let mut out = String::new();

    // ---- [Script Info] ----
    out.push_str("[Script Info]\n");
    out.push_str("Title: Subtitles\n");
    out.push_str("ScriptType: v4.00+\n");
    out.push_str(&format!("PlayResX: {}\n", video_width));
    out.push_str(&format!("PlayResY: {}\n", video_height));
    out.push_str("WrapStyle: 0\n");
    out.push('\n');

    // ---- [V4+ Styles] ----
    out.push_str("[V4+ Styles]\n");
    out.push_str("Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n");
    // Default 样式始终输出
    out.push_str(&ass_style_line(
        "Default",
        style,
        font_size,
        &style.primary_color,
    ));
    out.push('\n');

    // 说话人颜色样式：按唯一说话人排序后循环取色
    if style.speaker_colors {
        let mut speakers: Vec<String> = segments
            .iter()
            .map(|s| s.speaker.clone())
            .filter(|s| !s.is_empty())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        speakers.sort();
        for (i, sp) in speakers.iter().enumerate() {
            let color = SPEAKER_COLORS[i % SPEAKER_COLORS.len()];
            out.push_str(&ass_style_line(
                &speaker_style_name(sp),
                style,
                font_size,
                color,
            ));
            out.push('\n');
        }
    }
    out.push('\n');

    // ---- [Events] ----
    out.push_str("[Events]\n");
    out.push_str("Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n");

    let lanes = assign_overlap_lanes(segments);
    let line_height = std::cmp::max(1, font_size);
    for (seg, &lane) in segments.iter().zip(lanes.iter()) {
        let start = format_ass_time(seg.start);
        let end = format_ass_time(seg.end);
        let text = display_text(seg, style.show_speaker, &style.speaker_names);
        let escaped = ass_escape(&text);
        // 启用说话人颜色时使用对应样式，否则统一 Default
        let style_name = if style.speaker_colors && !seg.speaker.is_empty() {
            speaker_style_name(&seg.speaker)
        } else {
            "Default".to_string()
        };
        let margin_v = style.margin_v + lane * line_height;
        out.push_str(&format!(
            "Dialogue: 0,{},{},{},,0,0,{},,{}\n",
            start, end, style_name, margin_v, escaped
        ));
    }

    out
}

/// 将分段导出为 JSON 字符串（pretty-printed）。
/// 每个元素包含 {id, start, end, speaker, text}。
pub fn export_json(segments: &[ProcessableSegment]) -> String {
    serde_json::to_string_pretty(segments).unwrap_or_else(|_| "[]".to_string())
}

/// Tauri 命令模块，路径 subtitle_export::commands::export_subtitle。
pub mod commands {
    use super::*;
    use crate::database::models::Transcript;
    use crate::state::AppState;
    use crate::transcript_postprocess::NormalizeConfig;
    use sqlx::query_as;
    use tauri::State;

    /// 导出会议转录为指定格式的字幕字符串。
    ///
    /// - `format`: "srt" | "ass" | "json"
    /// - `show_speaker`: 是否在字幕中显示说话人（默认 true）
    /// - `speaker_names`: 说话人 ID → 显示名映射
    /// - `apply_postprocess`: 是否应用 transcript_postprocess::normalize（默认 true）
    #[tauri::command]
    pub async fn export_subtitle(
        state: State<'_, AppState>,
        meeting_id: String,
        format: String,
        show_speaker: Option<bool>,
        speaker_names: Option<HashMap<String, String>>,
        apply_postprocess: Option<bool>,
    ) -> Result<String, String> {
        let pool = state.db_manager.pool();
        let transcripts = query_as::<_, Transcript>(
            "SELECT * FROM transcripts WHERE meeting_id = ? ORDER BY audio_start_time",
        )
        .bind(&meeting_id)
        .fetch_all(pool)
        .await
        .map_err(|e| format!("Failed to query transcripts: {}", e))?;

        if transcripts.is_empty() {
            return Err("No transcripts found for this meeting".to_string());
        }

        // 转换为 ProcessableSegment
        let mut segments: Vec<ProcessableSegment> = transcripts
            .iter()
            .map(|t| ProcessableSegment {
                id: t.id.clone(),
                start: t.audio_start_time.unwrap_or(0.0) as f32,
                end: t.audio_end_time.unwrap_or(0.0) as f32,
                speaker: t
                    .speaker
                    .as_ref()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_default(),
                text: t.transcript.clone(),
            })
            .collect();

        // 默认应用规范化处理
        if apply_postprocess != Some(false) {
            crate::transcript_postprocess::normalize(&mut segments, &NormalizeConfig::default());
        }

        let show_sp = show_speaker.unwrap_or(true);

        // Auto-populate speaker_names from transcript speaker_name fields
        // for any speaker that doesn't already have a name.
        let mut final_names = speaker_names.unwrap_or_default();
        for t in &transcripts {
            if let (Some(speaker), Some(name)) = (&t.speaker, &t.speaker_name) {
                let speaker = speaker.trim();
                let name = name.trim();
                if !speaker.is_empty() && !name.is_empty() && !final_names.contains_key(speaker) {
                    final_names.insert(speaker.to_string(), name.to_string());
                }
            }
        }

        match format.as_str() {
            "srt" => Ok(export_srt(&segments, show_sp, &final_names)),
            "ass" => {
                let style = AssStyle {
                    show_speaker: show_sp,
                    speaker_names: final_names,
                    ..Default::default()
                };
                Ok(export_ass(&segments, &style, 1920, 1080))
            }
            "json" => Ok(export_json(&segments)),
            other => Err(format!("Unknown format: {}", other)),
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

    // ---- assign_overlap_lanes ----

    #[test]
    fn no_overlap_all_lane_zero() {
        // [0,2]+[2,4] → [0,0]：第二段开始时第一段已结束，复用 lane 0
        let segments = vec![seg("a", 0.0, 2.0, "S01", "x"), seg("b", 2.0, 4.0, "S01", "y")];
        let lanes = assign_overlap_lanes(&segments);
        assert_eq!(lanes, vec![0, 0]);
    }

    #[test]
    fn with_overlap_second_goes_lane_one() {
        // [0,3]+[1,4] → [0,1]：第二段与第一段重叠，新建 lane 1
        let segments = vec![seg("a", 0.0, 3.0, "S01", "x"), seg("b", 1.0, 4.0, "S02", "y")];
        let lanes = assign_overlap_lanes(&segments);
        assert_eq!(lanes, vec![0, 1]);
    }

    #[test]
    fn reuse_lane_after_end() {
        // [0,2]+[1,4]+[5,6] → [0,1,0]：第三段开始时 lane 0 已空闲，复用
        let segments = vec![
            seg("a", 0.0, 2.0, "S01", "x"),
            seg("b", 1.0, 4.0, "S02", "y"),
            seg("c", 5.0, 6.0, "S01", "z"),
        ];
        let lanes = assign_overlap_lanes(&segments);
        assert_eq!(lanes, vec![0, 1, 0]);
    }

    #[test]
    fn empty_input() {
        let lanes = assign_overlap_lanes(&[]);
        assert!(lanes.is_empty());
    }

    // ---- format_srt_time ----

    #[test]
    fn zero() {
        assert_eq!(format_srt_time(0.0), "00:00:00,000");
    }

    #[test]
    fn with_millis() {
        assert_eq!(format_srt_time(1.5), "00:00:01,500");
    }

    #[test]
    fn minutes_seconds() {
        assert_eq!(format_srt_time(65.234), "00:01:05,234");
    }

    #[test]
    fn hours() {
        assert_eq!(format_srt_time(3661.5), "01:01:01,500");
    }

    // ---- format_ass_time ----

    #[test]
    fn test_format_ass_time_basic() {
        assert_eq!(format_ass_time(0.0), "0:00:00.00");
        assert_eq!(format_ass_time(1.5), "0:00:01.50");
        assert_eq!(format_ass_time(65.234), "0:01:05.23");
        assert_eq!(format_ass_time(3661.5), "1:01:01.50");
    }

    // ---- export_srt ----

    #[test]
    fn srt_basic() {
        let segments = vec![
            seg("a", 0.0, 2.0, "S01", "hello"),
            seg("b", 2.0, 4.0, "S02", "world"),
        ];
        let out = export_srt(&segments, true, &HashMap::new());
        assert!(out.contains("1\n"), "should contain index");
        assert!(out.contains("00:00:00,000 --> 00:00:02,000"), "should contain time range");
        assert!(out.contains("S01: hello"), "should contain speaker label and text");
        assert!(out.contains("S02: world"));
    }

    #[test]
    fn srt_no_speaker() {
        let segments = vec![seg("a", 0.0, 2.0, "S01", "hello")];
        let out = export_srt(&segments, false, &HashMap::new());
        assert!(out.contains("hello"));
        assert!(!out.contains("S01:"));
    }

    #[test]
    fn srt_uses_speaker_names() {
        let mut names = HashMap::new();
        names.insert("S01".to_string(), "张三".to_string());
        let segments = vec![seg("a", 0.0, 2.0, "S01", "hi")];
        let out = export_srt(&segments, true, &names);
        assert!(out.contains("张三: hi"));
    }

    #[test]
    fn srt_empty_input() {
        let out = export_srt(&[], true, &HashMap::new());
        assert_eq!(out, "");
    }

    // ---- export_ass ----

    #[test]
    fn ass_basic_structure() {
        let segments = vec![seg("a", 0.0, 2.0, "S01", "hello")];
        let style = AssStyle::default();
        let out = export_ass(&segments, &style, 1920, 1080);
        assert!(out.contains("[Script Info]"));
        assert!(out.contains("[V4+ Styles]"));
        assert!(out.contains("[Events]"));
        assert!(out.contains("Dialogue:"));
    }

    #[test]
    fn ass_has_default_style() {
        let segments = vec![seg("a", 0.0, 2.0, "S01", "hello")];
        let style = AssStyle::default();
        let out = export_ass(&segments, &style, 1920, 1080);
        assert!(out.contains("Style: Default,Noto Sans CJK SC"));
    }

    #[test]
    fn ass_speaker_colors_creates_speaker_styles() {
        let segments = vec![
            seg("a", 0.0, 2.0, "S01", "hello"),
            seg("b", 2.0, 4.0, "S02", "world"),
        ];
        let style = AssStyle::default();
        let out = export_ass(&segments, &style, 1920, 1080);
        assert!(out.contains("Speaker_S01"), "should contain Speaker_S01 style");
        assert!(out.contains("Speaker_S02"), "should contain Speaker_S02 style");
    }

    #[test]
    fn ass_escapes_special_chars() {
        // 输入字面量: hello {world}\test （单个反斜杠）
        // 期望输出包含: hello (world)\\test （两个反斜杠）
        let segments = vec![seg("a", 0.0, 2.0, "S01", "hello {world}\\test")];
        let style = AssStyle {
            show_speaker: false,
            ..Default::default()
        };
        let out = export_ass(&segments, &style, 1920, 1080);
        assert!(
            out.contains("hello (world)\\\\test"),
            "expected escaped content, got: {}",
            out
        );
    }

    #[test]
    fn ass_no_speaker_colors_uses_default() {
        let segments = vec![seg("a", 0.0, 2.0, "S01", "hello")];
        let style = AssStyle {
            speaker_colors: false,
            ..Default::default()
        };
        let out = export_ass(&segments, &style, 1920, 1080);
        assert!(!out.contains("Speaker_S01"), "should not contain speaker styles");
        assert!(out.contains("Default"), "should contain Default style");
    }

    // ---- export_json ----

    #[test]
    fn json_basic() {
        let segments = vec![seg("a", 0.0, 2.0, "S01", "hello")];
        let out = export_json(&segments);
        assert!(out.contains("\"id\""));
        assert!(out.contains("\"start\""));
        assert!(out.contains("\"end\""));
        assert!(out.contains("\"speaker\""));
        assert!(out.contains("\"text\""));
    }

    #[test]
    fn json_multiple_segments() {
        let segments = vec![
            seg("a", 0.0, 2.0, "S01", "hello"),
            seg("b", 2.0, 4.0, "S02", "world"),
        ];
        let out = export_json(&segments);
        let parsed: Vec<serde_json::Value> =
            serde_json::from_str(&out).expect("should be valid JSON array");
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn json_empty_input() {
        let out = export_json(&[]);
        let parsed: Vec<serde_json::Value> =
            serde_json::from_str(&out).expect("should be valid JSON empty array");
        assert!(parsed.is_empty());
    }
}
