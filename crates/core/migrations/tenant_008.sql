-- Estimated speech cost per meeting (audio seconds from the finished transcript; the rate is config).
-- llm_usage already holds the language-model side. One batch row per meeting: a retried transcribe job replaces it.
CREATE TABLE speech_usage (
  id INTEGER PRIMARY KEY,
  meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('batch','stream')),
  audio_seconds INTEGER NOT NULL,
  cost_micro_usd INTEGER NOT NULL,
  at INTEGER NOT NULL,
  UNIQUE (meeting_id, kind)
);
