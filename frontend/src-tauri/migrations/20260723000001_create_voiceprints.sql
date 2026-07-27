-- 已注册声纹：存储姓名 + 嵌入向量 + 样本音频路径
CREATE TABLE voiceprints (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    embedding BLOB NOT NULL,
    audio_path TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_voiceprints_name ON voiceprints(name);
