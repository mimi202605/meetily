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
