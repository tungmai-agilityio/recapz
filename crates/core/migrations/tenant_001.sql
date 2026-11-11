CREATE TABLE members (id TEXT PRIMARY KEY, email TEXT UNIQUE NOT NULL, name TEXT,
  role TEXT NOT NULL CHECK (role IN ('admin','member')), status TEXT NOT NULL DEFAULT 'active');

CREATE TABLE meetings (id TEXT PRIMARY KEY, title TEXT NOT NULL, platform TEXT NOT NULL,
  join_url TEXT, started_at INTEGER, ended_at INTEGER, language TEXT NOT NULL DEFAULT 'vi+en',
  status TEXT NOT NULL, owner_id TEXT REFERENCES members(id), share TEXT NOT NULL DEFAULT 'attendees',
  audio_path TEXT, audio_delete_at INTEGER, data_delete_at INTEGER, error TEXT,
  created_at INTEGER NOT NULL DEFAULT 0);

CREATE TABLE speakers (id TEXT PRIMARY KEY, meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
  label TEXT NOT NULL, member_id TEXT REFERENCES members(id), display_name TEXT);

CREATE TABLE segments (id INTEGER PRIMARY KEY, meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
  speaker_id TEXT REFERENCES speakers(id), start_ms INTEGER NOT NULL, end_ms INTEGER NOT NULL,
  text TEXT NOT NULL, words_json TEXT);

CREATE TABLE reports (meeting_id TEXT PRIMARY KEY REFERENCES meetings(id) ON DELETE CASCADE,
  summary TEXT, decisions_json TEXT, questions_json TEXT, chapters_json TEXT,
  model TEXT, prompt_version INTEGER, created_at INTEGER);

CREATE TABLE action_items (id TEXT PRIMARY KEY, meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
  text TEXT NOT NULL, owner_member_id TEXT, due_date TEXT, done INTEGER NOT NULL DEFAULT 0, source_ms INTEGER);

CREATE TABLE chunks (id INTEGER PRIMARY KEY, meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
  start_ms INTEGER, end_ms INTEGER, text TEXT NOT NULL, embedding BLOB NOT NULL);
CREATE VIRTUAL TABLE chunks_fts USING fts5(text, content='chunks', content_rowid='id',
  tokenize = 'unicode61 remove_diacritics 2');
CREATE TRIGGER chunks_ai AFTER INSERT ON chunks BEGIN
  INSERT INTO chunks_fts(rowid, text) VALUES (new.id, new.text); END;
CREATE TRIGGER chunks_ad AFTER DELETE ON chunks BEGIN
  INSERT INTO chunks_fts(chunks_fts, rowid, text) VALUES ('delete', old.id, old.text); END;
CREATE TRIGGER chunks_au AFTER UPDATE ON chunks BEGIN
  INSERT INTO chunks_fts(chunks_fts, rowid, text) VALUES ('delete', old.id, old.text);
  INSERT INTO chunks_fts(rowid, text) VALUES (new.id, new.text); END;

CREATE TABLE jobs (id INTEGER PRIMARY KEY, kind TEXT NOT NULL, meeting_id TEXT, payload TEXT,
  state TEXT NOT NULL DEFAULT 'queued', attempts INTEGER NOT NULL DEFAULT 0,
  run_after INTEGER NOT NULL, locked_until INTEGER, last_error TEXT);

CREATE TABLE llm_usage (id INTEGER PRIMARY KEY, meeting_id TEXT, task TEXT NOT NULL, provider TEXT NOT NULL,
  model TEXT NOT NULL, input_tokens INTEGER, output_tokens INTEGER, cost_micro_usd INTEGER, at INTEGER NOT NULL);

CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
