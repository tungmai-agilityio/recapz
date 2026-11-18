-- 1 once the recording lives in object storage (and the local file is gone). The object key is
-- derived from the tenant id and `audio_path`, so no key is stored.
ALTER TABLE meetings ADD COLUMN audio_remote INTEGER NOT NULL DEFAULT 0;
