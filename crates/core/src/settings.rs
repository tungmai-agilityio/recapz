//! Workspace settings kept in the tenant `settings` table.
use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};

/// Whether new calendar events with a join link start with the join switch on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinRule {
    /// The notetaker joins only events a member switched on. The default: it never joins unasked.
    Manual,
    /// New events with a link start switched on; a member can still switch one off.
    Auto,
}

impl JoinRule {
    pub fn as_str(self) -> &'static str {
        match self {
            JoinRule::Manual => "manual",
            JoinRule::Auto => "auto",
        }
    }
}

const JOIN_RULE: &str = "join_rule";

pub fn join_rule(c: &Connection) -> Result<JoinRule> {
    let v: Option<String> = c
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [JOIN_RULE],
            |r| r.get(0),
        )
        .optional()?;
    // An unknown stored value falls back to the safe default.
    Ok(if v.as_deref() == Some("auto") {
        JoinRule::Auto
    } else {
        JoinRule::Manual
    })
}

pub fn set_join_rule(c: &Connection, rule: &str) -> Result<()> {
    if !["manual", "auto"].contains(&rule) {
        bail!("unknown join rule");
    }
    c.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![JOIN_RULE, rule],
    )?;
    Ok(())
}

const DEFAULT_SHARE: &str = "default_share";

/// Who can see a new meeting besides its owner: `owner`, `attendees` (the default) or `workspace`.
/// A database trigger applies it to every new meeting.
pub fn default_share(c: &Connection) -> Result<String> {
    text(c, DEFAULT_SHARE, "attendees")
}

pub fn set_default_share(c: &Connection, share: &str) -> Result<()> {
    if !["owner", "attendees", "workspace"].contains(&share) {
        bail!("unknown share");
    }
    c.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![DEFAULT_SHARE, share],
    )?;
    Ok(())
}

const NOTETAKER_NAME: &str = "notetaker_name";
const JOIN_MESSAGE: &str = "join_message";
pub const DEFAULT_NOTETAKER_NAME: &str = "Recapz notetaker";
/// Default text the notetaker posts; the words "opt out" must stay, members are told to type them.
pub const DEFAULT_JOIN_MESSAGE: &str =
    "Recapz notetaker is recording this call and transcribing it live. \
Type 'opt out' to make it leave and delete the recording.";

fn text(c: &Connection, key: &str, default: &str) -> Result<String> {
    let v: Option<String> = c
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(v.unwrap_or_else(|| default.to_owned()))
}

pub fn notetaker_name(c: &Connection) -> Result<String> {
    text(c, NOTETAKER_NAME, DEFAULT_NOTETAKER_NAME)
}

pub fn join_message(c: &Connection) -> Result<String> {
    text(c, JOIN_MESSAGE, DEFAULT_JOIN_MESSAGE)
}

/// Checks all three values, then saves them together. The outer error is storage, the inner one is
/// the message for a bad value; nothing is saved when a value is bad.
pub fn set_join_settings(
    c: &Connection,
    rule: &str,
    name: &str,
    message: &str,
) -> Result<std::result::Result<(), &'static str>> {
    if !["manual", "auto"].contains(&rule) {
        return Ok(Err("join rule must be manual or auto"));
    }
    if !(1..=60).contains(&name.chars().count()) {
        return Ok(Err("notetaker name must be 1 to 60 characters"));
    }
    if message.chars().count() > 500 {
        return Ok(Err("join message must be at most 500 characters"));
    }
    if !message.to_lowercase().contains("opt out") {
        return Ok(Err("join message must contain the words \"opt out\""));
    }
    set_join_rule(c, rule)?;
    for (k, v) in [(NOTETAKER_NAME, name), (JOIN_MESSAGE, message)] {
        c.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![k, v],
        )?;
    }
    Ok(Ok(()))
}

const AUDIO_RETENTION_DAYS: &str = "audio_retention_days";
pub const DEFAULT_AUDIO_RETENTION_DAYS: u32 = 90;

/// Days a finished recording's audio is kept; 0 keeps it forever. Transcripts and reports are not
/// affected. A missing or unreadable value means the default.
pub fn audio_retention_days(c: &Connection) -> Result<u32> {
    let v: Option<String> = c
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [AUDIO_RETENTION_DAYS],
            |r| r.get(0),
        )
        .optional()?;
    Ok(v.and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_AUDIO_RETENTION_DAYS))
}

pub fn set_audio_retention_days(c: &Connection, days: u32) -> Result<()> {
    if days > 3650 {
        bail!("retention is at most 3650 days");
    }
    c.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![AUDIO_RETENTION_DAYS, days.to_string()],
    )?;
    Ok(())
}

const TRANSCRIPT_RETENTION_DAYS: &str = "transcript_retention_days";
/// The periods an admin can pick for transcripts and reports: 90 days, one year, or 0 for keep.
pub const TRANSCRIPT_RETENTION_CHOICES: [u32; 3] = [0, 90, 365];

/// Days a finished meeting (transcript, report, search rows and audio) is kept before it is hard
/// deleted; 0 keeps it forever. The default and any missing or unreadable value mean keep, so
/// nothing is ever deleted unasked.
pub fn transcript_retention_days(c: &Connection) -> Result<u32> {
    let v: Option<String> = c
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [TRANSCRIPT_RETENTION_DAYS],
            |r| r.get(0),
        )
        .optional()?;
    Ok(v.and_then(|v| v.parse().ok())
        .filter(|d| TRANSCRIPT_RETENTION_CHOICES.contains(d))
        .unwrap_or(0))
}

pub fn set_transcript_retention_days(c: &Connection, days: u32) -> Result<()> {
    if !TRANSCRIPT_RETENTION_CHOICES.contains(&days) {
        bail!("transcript retention is 90, 365 or 0 days");
    }
    c.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![TRANSCRIPT_RETENTION_DAYS, days.to_string()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_tenant;

    #[test]
    fn defaults_to_manual_and_round_trips_and_rejects_junk() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        assert_eq!(join_rule(&c).unwrap(), JoinRule::Manual);
        set_join_rule(&c, "auto").unwrap();
        assert_eq!(join_rule(&c).unwrap(), JoinRule::Auto);
        set_join_rule(&c, "manual").unwrap();
        assert_eq!(join_rule(&c).unwrap(), JoinRule::Manual);
        assert!(set_join_rule(&c, "everyone").is_err());
        assert_eq!(
            join_rule(&c).unwrap(),
            JoinRule::Manual,
            "a rejected value changes nothing"
        );
        c.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('join_rule','bogus')",
            [],
        )
        .unwrap();
        assert_eq!(
            join_rule(&c).unwrap(),
            JoinRule::Manual,
            "unknown stored value is treated as manual"
        );
        assert_eq!(
            (JoinRule::Auto.as_str(), JoinRule::Manual.as_str()),
            ("auto", "manual")
        );
    }

    #[test]
    fn join_text_defaults_round_trips_and_a_bad_value_saves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        assert_eq!(notetaker_name(&c).unwrap(), "Recapz notetaker");
        assert_eq!(join_message(&c).unwrap(), DEFAULT_JOIN_MESSAGE);
        assert_eq!(
            set_join_settings(&c, "auto", "Bot", "Say opt out to leave").unwrap(),
            Ok(())
        );
        assert_eq!(notetaker_name(&c).unwrap(), "Bot");
        assert_eq!(join_message(&c).unwrap(), "Say opt out to leave");
        assert_eq!(join_rule(&c).unwrap(), JoinRule::Auto);
        let bad = set_join_settings(&c, "manual", "Bot2", "no way out").unwrap();
        assert_eq!(bad, Err("join message must contain the words \"opt out\""));
        assert_eq!(notetaker_name(&c).unwrap(), "Bot");
        assert_eq!(join_rule(&c).unwrap(), JoinRule::Auto);
    }

    #[test]
    fn audio_retention_defaults_to_90_days_round_trips_and_rejects_absurd_values() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        assert_eq!(audio_retention_days(&c).unwrap(), 90);
        set_audio_retention_days(&c, 30).unwrap();
        assert_eq!(audio_retention_days(&c).unwrap(), 30);
        set_audio_retention_days(&c, 0).unwrap();
        assert_eq!(
            audio_retention_days(&c).unwrap(),
            0,
            "0 keeps audio forever"
        );
        assert!(set_audio_retention_days(&c, 3651).is_err());
        c.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('audio_retention_days','x')",
            [],
        )
        .unwrap();
        assert_eq!(
            audio_retention_days(&c).unwrap(),
            90,
            "junk means the default"
        );
    }

    #[test]
    fn transcript_retention_defaults_to_keep_and_only_takes_the_offered_periods() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        assert_eq!(transcript_retention_days(&c).unwrap(), 0);
        set_transcript_retention_days(&c, 90).unwrap();
        assert_eq!(transcript_retention_days(&c).unwrap(), 90);
        set_transcript_retention_days(&c, 365).unwrap();
        assert_eq!(transcript_retention_days(&c).unwrap(), 365);
        assert!(set_transcript_retention_days(&c, 30).is_err());
        assert_eq!(transcript_retention_days(&c).unwrap(), 365);
        c.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('transcript_retention_days','7')",
            [],
        )
        .unwrap();
        assert_eq!(transcript_retention_days(&c).unwrap(), 0, "junk means keep");
    }
}
