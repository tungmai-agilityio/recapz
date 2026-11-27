-- Set when the notetaker recorded a call without posting its chat notice (`require_chat_notice = false`); the report warns.
ALTER TABLE meetings ADD COLUMN no_chat_notice INTEGER NOT NULL DEFAULT 0;
