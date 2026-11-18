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
}
