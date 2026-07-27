// hotword_correction/mod.rs
//
// 热词修正模块。借鉴 MOSS-Transcribe-Diarize 的热词 prompt 模式：
// 通过 LLM 修正转写文本中的专有名词错误（人名/机构/法规/术语），
// 复用现有 summary/llm_client.rs 调用 LLM。

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

/// 构建热词修正 prompt。借鉴 MOSS prompts.md 的热词模式。
///
/// 输出格式：
/// ```text
/// 请修正以下会议转写文本中的专有名词错误。
///
/// 热词提示：{热词1、热词2、热词3}
///
/// 转写文本（按段）：
/// [S01] (0.48-1.66) 欢迎各位参加审计会议
/// [S02] (1.66-3.20) 今天讨论审计法执行情况
/// ...
///
/// 要求：
/// 1. 仅修正专有名词（人名/机构/法规/术语），使其匹配热词
/// 2. 不改变语义和句子结构
/// 3. 保持 [Sxx] (start-end) text 格式输出
/// 4. 修正词用热词中的正确写法
/// ```
///
/// 若 hotwords 为空，热词提示行写 "（无热词）"。
pub fn build_correction_prompt(segments: &[ProcessableSegment], hotwords: &[String]) -> String {
    // 热词提示行：空列表写 "（无热词）"
    let hotword_line = if hotwords.is_empty() {
        "（无热词）".to_string()
    } else {
        // 用中文顿号连接
        hotwords.join("、")
    };

    let mut prompt = String::new();
    prompt.push_str("请修正以下会议转写文本中的专有名词错误。\n\n");
    prompt.push_str(&format!("热词提示：{}\n\n", hotword_line));
    prompt.push_str("转写文本（按段）：\n");
    for seg in segments {
        prompt.push_str(&format!(
            "[{}] ({:.2}-{:.2}) {}\n",
            seg.speaker, seg.start, seg.end, seg.text
        ));
    }
    prompt.push_str("\n要求：\n");
    prompt.push_str("1. 仅修正专有名词（人名/机构/法规/术语），使其匹配热词\n");
    prompt.push_str("2. 不改变语义和句子结构\n");
    prompt.push_str("3. 保持 [Sxx] (start-end) text 格式输出\n");
    prompt.push_str("4. 修正词用热词中的正确写法\n");

    prompt
}

/// 解析 LLM 修正输出。格式：`[Sxx] (start-end) text`。
/// 解析失败则回退到原始 segments（非破坏性）。
///
/// 解析逻辑：
/// - 逐行处理，跳过空行
/// - 每行期望格式 `[Sxx] (start-end) text`
/// - 找到 `)` 字符后的部分作为修正文本
/// - 与 original_segments 按顺序对应（LLM 应保持段数不变）
/// - 若某行无法解析，使用原始段
/// - 若 LLM 输出行数少于原始段，补齐剩余原始段
/// - 若结果为空（完全无法解析），返回原始段
pub fn parse_correction_output(
    output: &str,
    original_segments: &[ProcessableSegment],
) -> Vec<ProcessableSegment> {
    // 原始段为空：直接返回空 Vec
    if original_segments.is_empty() {
        return Vec::new();
    }

    let mut result: Vec<ProcessableSegment> = Vec::with_capacity(original_segments.len());
    let mut orig_iter = original_segments.iter().enumerate().peekable();

    for line in output.lines() {
        let line = line.trim();
        // 跳过空行（不消耗原始段）
        if line.is_empty() {
            continue;
        }

        // 期望格式：[Sxx] (start-end) text
        // 找到 ')' 字符后的部分作为修正文本
        let parsed_text = line.find(')').map(|close_paren_idx| {
            line[close_paren_idx + 1..].trim().to_string()
        });

        match parsed_text {
            Some(text) if !text.is_empty() => {
                // 解析成功：用 LLM 修正文本覆盖原段文本，保留其他字段
                if let Some((_, orig)) = orig_iter.next() {
                    result.push(ProcessableSegment {
                        id: orig.id.clone(),
                        start: orig.start,
                        end: orig.end,
                        speaker: orig.speaker.clone(),
                        text,
                    });
                }
            }
            _ => {
                // 无法解析：使用原始段
                if let Some((_, orig)) = orig_iter.next() {
                    result.push(orig.clone());
                }
            }
        }
    }

    // 补齐剩余原始段（LLM 输出行数少于原始段时）
    for (_, orig) in orig_iter {
        result.push(orig.clone());
    }

    // 若结果为空（完全无法解析），返回原始段
    if result.is_empty() {
        return original_segments.to_vec();
    }

    result
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

    // ---- build_correction_prompt ----

    #[test]
    fn test_build_prompt_includes_hotwords() {
        let segments = vec![
            seg("t1", 0.48, 1.66, "S01", "欢迎各位参加审计会议"),
            seg("t2", 1.66, 3.20, "S01", "今天讨论审计法执行情况"),
        ];
        let hotwords = vec![
            "审计法".to_string(),
            "国务院".to_string(),
            "审计署".to_string(),
        ];
        let prompt = build_correction_prompt(&segments, &hotwords);

        // 包含热词
        assert!(prompt.contains("审计法"), "prompt 应包含热词 '审计法'");
        assert!(prompt.contains("国务院"), "prompt 应包含热词 '国务院'");
        assert!(prompt.contains("审计署"), "prompt 应包含热词 '审计署'");
        // 包含分段标记
        assert!(prompt.contains("[S01]"), "prompt 应包含 [S01]");
        // 包含原始文本
        assert!(
            prompt.contains("欢迎各位参加审计会议"),
            "prompt 应包含第一段文本"
        );
        // 包含格式化时间
        assert!(prompt.contains("(0.48-1.66)"), "prompt 应包含时间区间");
        // 包含要求说明
        assert!(prompt.contains("要求"), "prompt 应包含要求说明");
    }

    #[test]
    fn test_build_prompt_empty_hotwords() {
        let segments = vec![seg("t1", 0.0, 1.0, "S01", "hello")];
        let hotwords: Vec<String> = vec![];
        let prompt = build_correction_prompt(&segments, &hotwords);

        // 空热词时仍生成有效 prompt
        assert!(!prompt.is_empty(), "空热词时 prompt 不应为空");
        assert!(
            prompt.contains("（无热词）"),
            "空热词时应输出 '（无热词）' 提示，实际: {}",
            prompt
        );
        assert!(prompt.contains("[S01]"), "空热词时仍应包含分段标记");
        assert!(prompt.contains("hello"), "空热词时仍应包含文本");
    }

    // ---- parse_correction_output ----

    #[test]
    fn test_parse_correction_output_basic() {
        let original = vec![
            seg("t1", 0.48, 1.66, "S01", "欢迎各位参加审计会议"),
            seg("t2", 1.66, 3.20, "S02", "今天讨论审计法执行情况"),
        ];
        // LLM 修正输出：仅替换文本，保留 [Sxx] (start-end) 格式
        let llm_output = "[S01] (0.48-1.66) 欢迎各位参加审计署会议\n\
                          [S02] (1.66-3.20) 今天讨论审计法的执行情况\n";

        let result = parse_correction_output(llm_output, &original);

        assert_eq!(result.len(), 2, "应解析为 2 段");
        // 保留原始 id/start/end/speaker
        assert_eq!(result[0].id, "t1");
        assert_eq!(result[0].start, 0.48);
        assert_eq!(result[0].end, 1.66);
        assert_eq!(result[0].speaker, "S01");
        // 文本已修正
        assert_eq!(result[0].text, "欢迎各位参加审计署会议");
        assert_eq!(result[1].text, "今天讨论审计法的执行情况");
    }

    #[test]
    fn test_parse_correction_output_fallback_on_malformed() {
        let original = vec![
            seg("t1", 0.48, 1.66, "S01", "欢迎各位参加审计会议"),
            seg("t2", 1.66, 3.20, "S02", "今天讨论审计法执行情况"),
        ];
        // 完全无法解析的输出（缺少 ')' 字符）
        let malformed = "这是一段无法解析的输出\n\
                         没有任何格式标记\n";

        let result = parse_correction_output(malformed, &original);

        // 回退到原始段（数量一致）
        assert_eq!(result.len(), original.len(), "回退后段数应与原始一致");
        // 文本保持原始
        assert_eq!(result[0].text, "欢迎各位参加审计会议");
        assert_eq!(result[1].text, "今天讨论审计法执行情况");
    }

    #[test]
    fn test_parse_correction_output_empty_output_returns_originals() {
        let original = vec![seg("t1", 0.0, 1.0, "S01", "hello")];
        // 完全空输出
        let result = parse_correction_output("", &original);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].text, "hello");
    }

    #[test]
    fn test_parse_correction_output_fewer_lines_pads_with_originals() {
        let original = vec![
            seg("t1", 0.0, 1.0, "S01", "first"),
            seg("t2", 1.0, 2.0, "S01", "second"),
            seg("t3", 2.0, 3.0, "S01", "third"),
        ];
        // LLM 仅输出 1 行
        let llm_output = "[S01] (0.00-1.00) FIRST\n";
        let result = parse_correction_output(llm_output, &original);

        assert_eq!(result.len(), 3, "应补齐至 3 段");
        assert_eq!(result[0].text, "FIRST");
        // 后两段保留原始
        assert_eq!(result[1].text, "second");
        assert_eq!(result[2].text, "third");
    }
}
