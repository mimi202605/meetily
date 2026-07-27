// voiceprint_engine/commands.rs
//
// Tauri 命令：声纹注册、列表、删除、从会议抓取样本、会议匹配、手动指派

use std::path::PathBuf;
use log::{info, warn};
use tauri::{AppHandle, Manager, Runtime};
use serde::Serialize;
use uuid::Uuid;
use chrono::Utc;

use super::engine::get_engine;
use super::repository::{
    insert_voiceprint, list_voiceprints, get_voiceprint, delete_voiceprint,
    upsert_override, list_overrides_for_meeting,
    VoiceprintRecord,
};
use crate::database::models::{MeetingModel, Transcript};
use sqlx::query_as;

/// 会议声纹匹配结果
#[derive(Debug, Serialize)]
pub struct MeetingMatchResult {
    /// 已匹配的说话人列表：(speaker_id, voiceprint_name, similarity)
    pub matched: Vec<(i32, String, f32)>,
    /// 未匹配的说话人 ID 列表
    pub unmatched_speaker_ids: Vec<i32>,
}

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

/// 列出所有已注册声纹
#[tauri::command]
pub async fn voiceprint_list<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<Vec<VoiceprintDto>, String> {
    let pool = state.db_manager.pool();
    let records = list_voiceprints(pool).await?;
    Ok(records.into_iter().map(VoiceprintDto::from).collect())
}

/// 列出所有已完成说话人分离的会议（含 speaker_id 列表）
#[tauri::command]
pub async fn voiceprint_list_meetings_with_speakers<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<Vec<MeetingWithSpeakersDto>, String> {
    let pool = state.db_manager.pool();
    // 查询所有有 folder_path 的会议
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
                // DateTimeUtc is a newtype wrapping DateTime<Utc>; access inner via .0
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
    state: tauri::State<'_, crate::state::AppState>,
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

    if best_end - best_start < 3.0 {
        let combined_start = best_start;
        let mut combined_end = best_end;
        for seg in segments.iter().skip(1) {
            let s = seg.audio_start_time.unwrap_or(0.0);
            let e = seg.audio_end_time.unwrap_or(0.0);
            if s <= combined_end + 1.0 && e > combined_end {
                combined_end = e;
                if combined_end - combined_start >= 3.0 { break; }
            }
        }
        best_start = combined_start;
        best_end = combined_end;
    }

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
    let audio_path = crate::audio::retranscription::find_audio_file(std::path::Path::new(folder_path))
        .map_err(|e| format!("Audio file not found in {}: {}", folder_path, e))?;

    // 解码音频（spawn_blocking 因为 decode 不调用 block_in_place）
    let audio_path_clone = audio_path.clone();
    let decoded = tokio::task::spawn_blocking(move || crate::audio::decoder::decode_audio_file(&audio_path_clone))
        .await
        .map_err(|e| format!("Decode join error: {}", e))?
        .map_err(|e| format!("Failed to decode audio: {}", e))?;
    // to_whisper_format() always produces 16kHz mono samples
    let samples = decoded.to_whisper_format();
    // Index into the 16kHz samples, NOT the original decoded.sample_rate
    // Use f64 to match the audio_start_time/audio_end_time types from the DB.
    let sample_rate = 16000.0f64;

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
    state: tauri::State<'_, crate::state::AppState>,
    name: String,
    audio_path: String,
) -> Result<VoiceprintDto, String> {
    if name.trim().is_empty() {
        return Err("姓名不能为空".to_string());
    }

    let engine = get_engine()?;

    // 解码音频文件（spawn_blocking - decode 不调用 block_in_place）
    let path = PathBuf::from(&audio_path);
    let decoded = tokio::task::spawn_blocking(move || crate::audio::decoder::decode_audio_file(&path))
        .await
        .map_err(|e| format!("Decode join error: {}", e))?
        .map_err(|e| format!("Failed to decode audio: {}", e))?;

    if decoded.duration_seconds < 1.0 {
        return Err("音频过短，至少需要 1 秒".to_string());
    }

    let samples = decoded.to_whisper_format();

    // 提取嵌入（直接调用，内部使用 block_in_place）
    let embedding = engine.extract_embedding(&samples)?;

    let embedding_bytes = super::engine::VoiceprintEngine::serialize_embedding(&embedding);

    // 移动临时音频到永久位置
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
    state: tauri::State<'_, crate::state::AppState>,
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
    state: tauri::State<'_, crate::state::AppState>,
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
    state: tauri::State<'_, crate::state::AppState>,
    meeting_id: String,
    speaker_id: i32,
    voiceprint_id: String,
) -> Result<bool, String> {
    let pool = state.db_manager.pool();
    upsert_override(pool, &meeting_id, speaker_id, &voiceprint_id, "manual").await?;
    info!("[Voiceprint] Manual assign: meeting={} speaker={} voiceprint={}", meeting_id, speaker_id, voiceprint_id);
    Ok(true)
}

/// 对指定会议重新执行声纹识别
#[tauri::command]
pub async fn voiceprint_match_meeting<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, crate::state::AppState>,
    meeting_id: String,
) -> Result<MeetingMatchResult, String> {
    use crate::speaker_diarization_engine::commands as diarization_cmds;

    let pool = state.db_manager.pool();

    // 1. 获取会议
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
    let audio_path = crate::audio::retranscription::find_audio_file(std::path::Path::new(folder_path))
        .map_err(|e| format!("Audio file not found in {}: {}", folder_path, e))?;

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

    // 3. 解码原始音频 PCM（decode_audio_file 不使用 block_in_place，可用 spawn_blocking）
    let audio_path_clone = audio_path.clone();
    let decoded = tokio::task::spawn_blocking(move || crate::audio::decoder::decode_audio_file(&audio_path_clone))
        .await
        .map_err(|e| format!("Decode join error: {}", e))?
        .map_err(|e| format!("Failed to decode audio: {}", e))?;
    let samples = decoded.to_whisper_format();
    let sample_rate = 16000.0f32;  // to_whisper_format always returns 16kHz

    // 4. 加载所有已注册声纹
    let vp_records = list_voiceprints(pool).await?;
    if vp_records.is_empty() {
        // 没有已注册声纹，所有说话人返回为 unmatched
        let speaker_ids: Vec<i32> = speaker_segments.iter()
            .map(|s| s.speaker)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        return Ok(MeetingMatchResult {
            matched: Vec::new(),
            unmatched_speaker_ids: speaker_ids,
        });
    }
    let vp_with_embeddings: Vec<(String, String, Vec<f32>)> = vp_records.iter()
        .map(|r| {
            let emb = super::engine::VoiceprintEngine::deserialize_embedding(&r.embedding);
            (r.id.clone(), r.name.clone(), emb)
        })
        .collect();

    // 5. 按 speaker ID 分组，提取每个 segment 的嵌入并求平均
    //    extract_embedding 内部使用 block_in_place，必须直接在 async 上下文调用，不能放在 spawn_blocking 中
    let engine = get_engine()?;
    let mut cluster_embeddings: std::collections::HashMap<i32, Vec<Vec<f32>>> = std::collections::HashMap::new();
    for seg in &speaker_segments {
        let start_sample = (seg.start * sample_rate) as usize;
        let end_sample = (seg.end * sample_rate) as usize;
        if end_sample <= start_sample || end_sample > samples.len() {
            continue;
        }
        let seg_samples = samples[start_sample..end_sample].to_vec();
        match engine.extract_embedding(&seg_samples) {
            Ok(emb) => {
                cluster_embeddings.entry(seg.speaker).or_default().push(emb);
            }
            Err(e) => warn!("[Voiceprint] Failed to extract embedding for speaker {}: {}", seg.speaker, e),
        }
    }

    // 6. 计算聚类质心并匹配
    let mut matched = Vec::new();
    let mut unmatched = Vec::new();
    for (speaker_id, embs) in &cluster_embeddings {
        if embs.is_empty() { continue; }
        // 计算质心（简单算术平均）
        let dim = embs[0].len();
        let mut sum = vec![0.0f32; dim];
        for emb in embs {
            for (i, &v) in emb.iter().enumerate() {
                sum[i] += v;
            }
        }
        let avg: Vec<f32> = sum.iter().map(|v| v / embs.len() as f32).collect();
        // 重新 L2 归一化
        let norm: f32 = avg.iter().map(|v| v * v).sum::<f32>().sqrt();
        let centroid: Vec<f32> = if norm > 0.0 {
            avg.iter().map(|v| v / norm).collect()
        } else {
            avg
        };

        // 匹配
        if let Some(m) = super::engine::VoiceprintEngine::match_against(&centroid, &vp_with_embeddings, 0.6) {
            // 写入 override (source='auto')
            let _ = upsert_override(pool, &meeting_id, *speaker_id, &m.voiceprint_id, "auto").await;
            matched.push((*speaker_id, m.name, m.similarity));
        } else {
            unmatched.push(*speaker_id);
        }
    }

    // 7. 更新 transcripts 表的 speaker_name 字段
    let overrides = list_overrides_for_meeting(pool, &meeting_id).await?;
    for ov in &overrides {
        if let Some(vp) = get_voiceprint(pool, &ov.voiceprint_id).await? {
            let speaker_str = ov.speaker_id.to_string();
            sqlx::query("UPDATE transcripts SET speaker_name = ? WHERE meeting_id = ? AND speaker = ?")
                .bind(&vp.name)
                .bind(&meeting_id)
                .bind(&speaker_str)
                .execute(pool)
                .await
                .map_err(|e| format!("Failed to update speaker_name: {}", e))?;
        }
    }

    info!("[Voiceprint] Meeting {} matched: {} speakers, {} unmatched", meeting_id, matched.len(), unmatched.len());
    Ok(MeetingMatchResult { matched, unmatched_speaker_ids: unmatched })
}
