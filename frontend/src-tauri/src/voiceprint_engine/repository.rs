use sqlx::sqlite::SqlitePool;
use sqlx::FromRow;
use log::info;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct VoiceprintRecord {
    pub id: String,
    pub name: String,
    pub embedding: Vec<u8>,
    pub audio_path: String,
    pub created_at: String,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
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
    // manual overrides take precedence: don't overwrite manual with auto
    sqlx::query(
        "INSERT INTO meeting_speaker_overrides (meeting_id, speaker_id, voiceprint_id, source)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(meeting_id, speaker_id) DO UPDATE SET
            voiceprint_id = excluded.voiceprint_id,
            source = excluded.source
         WHERE meeting_speaker_overrides.source != 'manual' OR excluded.source = 'manual'",
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
