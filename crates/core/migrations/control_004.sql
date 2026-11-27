-- Session ids are now stored as SHA-256 hashes: drop every plaintext session (members sign in once more).
-- Legacy invites get a creation time (now) so they expire after 14 days like workspace_invites.
DELETE FROM sessions;
ALTER TABLE invites ADD COLUMN created_at INTEGER NOT NULL DEFAULT 0;
UPDATE invites SET created_at = CAST(strftime('%s','now') AS INTEGER) * 1000;
-- An insert without a time (the old shape) is stamped now.
CREATE TRIGGER invites_created_at AFTER INSERT ON invites WHEN NEW.created_at = 0 BEGIN
  UPDATE invites SET created_at = CAST(strftime('%s','now') AS INTEGER) * 1000 WHERE email = NEW.email;
END;
