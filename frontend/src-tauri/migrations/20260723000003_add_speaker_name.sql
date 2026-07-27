-- 声纹匹配或手动指派的说话人姓名（覆盖默认"说话人 N"显示）
ALTER TABLE transcripts ADD COLUMN speaker_name TEXT;
