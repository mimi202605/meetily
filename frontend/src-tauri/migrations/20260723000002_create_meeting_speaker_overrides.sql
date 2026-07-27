-- 会议内说话人 ID → 已注册声纹的映射（含自动匹配和手动指派）
CREATE TABLE meeting_speaker_overrides (
    meeting_id TEXT NOT NULL,
    speaker_id INTEGER NOT NULL,
    voiceprint_id TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT 'manual',
    PRIMARY KEY (meeting_id, speaker_id),
    FOREIGN KEY (voiceprint_id) REFERENCES voiceprints(id) ON DELETE CASCADE
);
