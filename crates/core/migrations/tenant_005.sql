-- Who was invited to a calendar event: a JSON array of {name, email, status}, one list per event row,
-- so it is only ever read through the owning member's query.
ALTER TABLE calendar_events ADD COLUMN attendees TEXT NOT NULL DEFAULT '[]';
