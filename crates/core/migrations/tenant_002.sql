-- Calendar events pulled for one member. `auto_join` is the per-event switch on the upcoming list.
CREATE TABLE calendar_events (
  id TEXT PRIMARY KEY,
  member_id TEXT NOT NULL REFERENCES members(id) ON DELETE CASCADE,
  external_id TEXT NOT NULL,
  title TEXT NOT NULL,
  starts_at INTEGER NOT NULL,
  ends_at INTEGER NOT NULL,
  join_url TEXT,
  platform TEXT,
  auto_join INTEGER NOT NULL DEFAULT 0,
  meeting_id TEXT REFERENCES meetings(id) ON DELETE SET NULL,
  UNIQUE (member_id, external_id)
);
CREATE INDEX calendar_events_start ON calendar_events (member_id, starts_at);
