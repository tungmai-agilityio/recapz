-- Member documents for Ask: owned by one member and visible to that member only. `chunks` belongs to
-- meetings (meeting_id NOT NULL), so documents get their own tables. A vector row exists only for a
-- document whose llm_ok is 1 ("May be sent to the language model"); the rest are keyword-only.
CREATE TABLE documents (
  id TEXT PRIMARY KEY,
  member_id TEXT NOT NULL REFERENCES members(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('md','txt')),
  size_bytes INTEGER NOT NULL,
  path TEXT NOT NULL,
  llm_ok INTEGER NOT NULL DEFAULT 0 CHECK (llm_ok IN (0,1)),
  created_at INTEGER NOT NULL
);
CREATE INDEX documents_member ON documents (member_id);
CREATE TABLE document_chunks (
  id INTEGER PRIMARY KEY,
  document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
  seq INTEGER NOT NULL,
  text TEXT NOT NULL
);
CREATE INDEX document_chunks_doc ON document_chunks (document_id);
CREATE VIRTUAL TABLE document_chunks_fts USING fts5(text, content='document_chunks', content_rowid='id',
  tokenize = 'unicode61 remove_diacritics 2');
CREATE TRIGGER document_chunks_ai AFTER INSERT ON document_chunks BEGIN
  INSERT INTO document_chunks_fts(rowid, text) VALUES (new.id, new.text); END;
CREATE TRIGGER document_chunks_ad AFTER DELETE ON document_chunks BEGIN
  INSERT INTO document_chunks_fts(document_chunks_fts, rowid, text) VALUES ('delete', old.id, old.text); END;
CREATE TABLE document_vectors (
  chunk_id INTEGER PRIMARY KEY REFERENCES document_chunks(id) ON DELETE CASCADE,
  document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
  embedder TEXT NOT NULL,
  embedding BLOB NOT NULL
);
CREATE INDEX document_vectors_doc ON document_vectors (document_id);
