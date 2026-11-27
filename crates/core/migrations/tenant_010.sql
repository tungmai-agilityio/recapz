-- A member's Ask history: private to them, cited moments kept as the cards the answer showed.
CREATE TABLE ask_turns (id TEXT PRIMARY KEY, member_id TEXT NOT NULL, question TEXT NOT NULL,
  answer TEXT NOT NULL, citations_json TEXT NOT NULL DEFAULT '[]', created_at INTEGER NOT NULL);
CREATE INDEX ask_turns_member ON ask_turns (member_id, created_at);
