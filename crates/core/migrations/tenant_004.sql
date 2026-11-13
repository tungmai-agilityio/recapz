-- Google refresh tokens, one per member, encrypted at rest (key comes from the environment).
CREATE TABLE google_tokens (
  member_id TEXT PRIMARY KEY REFERENCES members(id) ON DELETE CASCADE,
  refresh_token BLOB NOT NULL,
  nonce BLOB NOT NULL,
  scopes TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
