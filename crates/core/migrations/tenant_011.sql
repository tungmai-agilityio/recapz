-- A new meeting takes the workspace's `default_share` setting when an admin has set one; without it the
-- column default ('attendees') stays. One trigger covers every creation path (calendar sync, record,
-- upload, notetaker join, dev live).
CREATE TRIGGER meetings_default_share AFTER INSERT ON meetings
WHEN EXISTS (SELECT 1 FROM settings WHERE key = 'default_share')
BEGIN
  UPDATE meetings SET share = (SELECT value FROM settings WHERE key = 'default_share') WHERE id = NEW.id;
END;
