// hotword_correction/commands.rs
//
// Tauri 命令：热词 CRUD + 调用 LLM 修正转写文本中的专有名词。
//
// 模式借鉴 summary::commands::api_process_transcript：LLM 配置由前端传入，
// 通过 tauri 事件通知前端修正进度。

use log::{error as log_error, info as log_info, warn as log_warn};
use sqlx::query_as;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

use crate::database::models::Transcript;
use crate::hotword_correction::repository::HotwordRepository;
use crate::hotword_correction::{build_correction_prompt, parse_correction_output, Hotword};
use crate::state::AppState;
use crate::summary::llm_client::{generate_summary, LLMProvider};
use crate::transcript_postprocess::ProcessableSegment;

/// 修正指令（system prompt）：聚焦专有名词修正，禁止改写语义。
const CORRECTION_SYSTEM_PROMPT: &str = "你是一位会议转写校对助手。你的任务是根据提供的热词列表，\
修正转写文本中识别错误的专有名词（人名、机构名、法规名、专业术语等）。\n\n\
严格规则：\n\
1. 只修正专有名词，不得改写普通词汇或句子结构；\n\
2. 保持原文语义不变；\n\
3. 严格保持输入的 [Sxx] (start-end) text 行格式输出，每行一段；\n\
4. 不要输出多余说明、注释或 markdown 代码块标记；\n\
5. 段数与输入保持一致，不要合并或拆分。";

/// 列出热词。
///
/// - `scope = None`：查询全部热词
/// - `scope = Some(meeting_id)`：查询该会议专属 + 全局热词
#[tauri::command]
pub async fn get_hotwords(
    state: State<'_, AppState>,
    scope: Option<String>,
) -> Result<Vec<Hotword>, String> {
    let pool = state.db_manager.pool().clone();
    let repo = HotwordRepository::new(pool);
    repo.list(scope.as_deref()).await
}

/// 新增热词。`scope` 默认 "global"。
#[tauri::command]
pub async fn add_hotword(
    state: State<'_, AppState>,
    word: String,
    category: Option<String>,
    scope: Option<String>,
) -> Result<String, String> {
    let word = word.trim().to_string();
    if word.is_empty() {
        return Err("hotword word must not be empty".to_string());
    }
    let final_scope = scope
        .map(|s| {
            let trimmed = s.trim().to_string();
            if trimmed.is_empty() {
                "global".to_string()
            } else {
                trimmed
            }
        })
        .unwrap_or_else(|| "global".to_string());

    let pool = state.db_manager.pool().clone();
    let repo = HotwordRepository::new(pool);
    repo.add(&word, category.as_deref(), &final_scope).await
}

/// 按 ID 删除热词。
#[tauri::command]
pub async fn delete_hotword(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let pool = state.db_manager.pool().clone();
    let repo = HotwordRepository::new(pool);
    repo.delete(&id).await
}

/// 调用 LLM 修正指定会议的转写文本中的专有名词。
///
/// 流程：
/// 1. emit "hotword-correction-start"
/// 2. 加载热词（全局 + 该会议）
/// 3. 如果无热词 → emit error, return Err
/// 4. 查询 DB 的 transcripts
/// 5. 转换为 ProcessableSegment
/// 6. 构建 prompt（system_prompt = 修正指令，user_prompt = build_correction_prompt）
/// 7. 调用 generate_summary
/// 8. 解析输出
/// 9. emit "hotword-correction-complete"
/// 10. 返回修正后的 segments
#[tauri::command]
pub async fn correct_transcript_with_hotwords<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    meeting_id: String,
    model_provider: String,
    model_name: String,
    api_key: Option<String>,
    ollama_endpoint: Option<String>,
    custom_openai_endpoint: Option<String>,
) -> Result<Vec<ProcessableSegment>, String> {
    log_info!(
        "correct_transcript_with_hotwords called: meeting_id={}, provider={}, model={}",
        &meeting_id,
        &model_provider,
        &model_name
    );

    // 1. 通知前端：修正开始
    let _ = app.emit(
        "hotword-correction-start",
        serde_json::json!({ "meeting_id": &meeting_id }),
    );

    // 2. 加载热词（全局 + 该会议）
    let pool = state.db_manager.pool().clone();
    let repo = HotwordRepository::new(pool.clone());
    let hotwords = repo.list(Some(&meeting_id)).await?;

    // 3. 无热词 → 报错
    if hotwords.is_empty() {
        let err_msg = format!(
            "No hotwords configured for meeting {} (add global or meeting-scoped hotwords first)",
            &meeting_id
        );
        log_warn!("{}", err_msg);
        let _ = app.emit(
            "hotword-correction-error",
            serde_json::json!({
                "meeting_id": &meeting_id,
                "error": &err_msg,
            }),
        );
        return Err(err_msg);
    }

    let hotword_strings: Vec<String> = hotwords.iter().map(|h| h.word.clone()).collect();

    // 4. 查询 DB 的 transcripts
    let transcripts: Vec<Transcript> = query_as::<_, Transcript>(
        "SELECT * FROM transcripts WHERE meeting_id = ? ORDER BY audio_start_time",
    )
    .bind(&meeting_id)
    .fetch_all(&pool)
    .await
    .map_err(|e| {
        let msg = format!("DB error: {}", e);
        log_error!("{}", msg);
        let _ = app.emit(
            "hotword-correction-error",
            serde_json::json!({
                "meeting_id": &meeting_id,
                "error": &msg,
            }),
        );
        msg
    })?;

    if transcripts.is_empty() {
        let err_msg = format!("No transcripts found for meeting {}", &meeting_id);
        log_warn!("{}", err_msg);
        let _ = app.emit(
            "hotword-correction-error",
            serde_json::json!({
                "meeting_id": &meeting_id,
                "error": &err_msg,
            }),
        );
        return Err(err_msg);
    }

    // 5. 转换为 ProcessableSegment
    let segments: Vec<ProcessableSegment> = transcripts
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

    // 6. 构建 prompt
    let user_prompt = build_correction_prompt(&segments, &hotword_strings);

    // 7. 调用 LLM
    let provider = LLMProvider::from_str(&model_provider)
        .map_err(|e| {
            let msg = format!("Invalid model_provider '{}': {}", &model_provider, e);
            log_error!("{}", msg);
            let _ = app.emit(
                "hotword-correction-error",
                serde_json::json!({
                    "meeting_id": &meeting_id,
                    "error": &msg,
                }),
            );
            msg
        })?;

    let final_api_key = api_key.unwrap_or_default();
    let client = reqwest::Client::new();

    // 获取 app_data_dir（BuiltInAI provider 需要）
    let app_data_dir = app.path().app_data_dir().ok();

    log_info!(
        "Calling LLM for hotword correction: meeting_id={}, segments={}, hotwords={}",
        &meeting_id,
        segments.len(),
        hotword_strings.len()
    );

    let llm_result = generate_summary(
        &client,
        &provider,
        &model_name,
        &final_api_key,
        CORRECTION_SYSTEM_PROMPT,
        &user_prompt,
        ollama_endpoint.as_deref(),
        custom_openai_endpoint.as_deref(),
        None,    // max_tokens
        None,    // temperature
        None,    // top_p
        app_data_dir.as_ref(),
        None,    // cancellation_token
    )
    .await;

    let raw_output = match llm_result {
        Ok(text) => text,
        Err(e) => {
            let msg = format!("LLM call failed: {}", e);
            log_error!("{}", msg);
            let _ = app.emit(
                "hotword-correction-error",
                serde_json::json!({
                    "meeting_id": &meeting_id,
                    "error": &msg,
                }),
            );
            return Err(msg);
        }
    };

    // 8. 解析输出
    let corrected = parse_correction_output(&raw_output, &segments);

    log_info!(
        "Hotword correction completed: meeting_id={}, segments_in={}, segments_out={}",
        &meeting_id,
        segments.len(),
        corrected.len()
    );

    // 9. 通知前端：修正完成
    let _ = app.emit(
        "hotword-correction-complete",
        serde_json::json!({
            "meeting_id": &meeting_id,
            "segments": &corrected,
        }),
    );

    // 10. 返回修正后的 segments
    Ok(corrected)
}
