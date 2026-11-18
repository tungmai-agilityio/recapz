//! Usage and cost: speech rows per meeting, month totals over speech and language-model rows, and the
//! monthly cap that pauses cost-incurring jobs. Costs are estimates from configured rates, in
//! micro-USD (1_000_000 = $1).
use anyhow::Result;
use rusqlite::{params, Connection};

/// Records the batch transcript's cost from the stored segments (the audio length is their last end).
/// Safe to repeat: the meeting's batch row is replaced.
pub fn record_speech(
    c: &Connection,
    meeting_id: &str,
    micro_usd_per_minute: u64,
    now_ms: i64,
) -> Result<()> {
    let ms: i64 = c.query_row(
        "SELECT COALESCE(MAX(end_ms), 0) FROM segments WHERE meeting_id = ?1",
        [meeting_id],
        |r| r.get(0),
    )?;
    let secs = ms / 1000;
    let cost = secs as u64 * micro_usd_per_minute / 60;
    c.execute(
        "INSERT INTO speech_usage (meeting_id, kind, audio_seconds, cost_micro_usd, at)
         VALUES (?1, 'batch', ?2, ?3, ?4)
         ON CONFLICT (meeting_id, kind) DO UPDATE SET
           audio_seconds = excluded.audio_seconds, cost_micro_usd = excluded.cost_micro_usd, at = excluded.at",
        params![meeting_id, secs, cost as i64, now_ms],
    )?;
    Ok(())
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Totals {
    pub speech_micro_usd: i64,
    pub llm_micro_usd: i64,
}

impl Totals {
    pub fn total(&self) -> i64 {
        self.speech_micro_usd + self.llm_micro_usd
    }
}

// Condition on an `at` column (ms): inside the UTC calendar month that contains ?1.
fn in_month(col: &str) -> String {
    format!(
        "({col} >= CAST(strftime('%s', ?1 / 1000, 'unixepoch', 'start of month') AS INTEGER) * 1000
          AND {col} < CAST(strftime('%s', ?1 / 1000, 'unixepoch', 'start of month', '+1 month') AS INTEGER) * 1000)"
    )
}

pub fn month_totals(c: &Connection, now_ms: i64) -> Result<Totals> {
    let q = |table: &str| -> Result<i64> {
        Ok(c.query_row(
            &format!(
                "SELECT COALESCE(SUM(cost_micro_usd), 0) FROM {table} WHERE {}",
                in_month("at")
            ),
            [now_ms],
            |r| r.get(0),
        )?)
    };
    Ok(Totals {
        speech_micro_usd: q("speech_usage")?,
        llm_micro_usd: q("llm_usage")?,
    })
}

/// True when a cap is set (non-zero) and this month's estimated cost has reached it.
pub fn paused(c: &Connection, cap_micro_usd: u64, now_ms: i64) -> Result<bool> {
    Ok(cap_micro_usd > 0 && month_totals(c, now_ms)?.total() >= cap_micro_usd as i64)
}

#[derive(Debug, PartialEq, Eq)]
pub struct MeetingCost {
    pub meeting_id: String,
    pub title: String,
    pub speech_micro_usd: i64,
    pub llm_micro_usd: i64,
}

/// This month's costliest meetings first, only those `member_id` may see (owner, or shared with the
/// workspace). Rows without a meeting (Ask) are not listed here.
pub fn per_meeting(
    c: &Connection,
    now_ms: i64,
    limit: usize,
    member_id: &str,
) -> Result<Vec<MeetingCost>> {
    let sql = format!(
        "SELECT m.id, m.title,
                COALESCE((SELECT SUM(cost_micro_usd) FROM speech_usage s WHERE s.meeting_id = m.id AND {}), 0),
                COALESCE((SELECT SUM(cost_micro_usd) FROM llm_usage l WHERE l.meeting_id = m.id AND {}), 0)
         FROM meetings m
         WHERE m.owner_id = ?3 OR m.share = 'workspace'
         ORDER BY 3 + 4 DESC, m.id LIMIT ?2",
        in_month("s.at"),
        in_month("l.at")
    );
    let mut st = c.prepare(&sql)?;
    let rows = st
        .query_map(params![now_ms, limit as i64, member_id], |r| {
            Ok(MeetingCost {
                meeting_id: r.get(0)?,
                title: r.get(1)?,
                speech_micro_usd: r.get(2)?,
                llm_micro_usd: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .filter(|m| m.speech_micro_usd + m.llm_micro_usd > 0)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_tenant;

    // 2026-11-18 12:00 UTC and 2026-10-31 23:00 UTC in ms
    const NOV: i64 = 1_795_089_600_000;
    const OCT: i64 = 1_793_487_600_000;

    fn seed(c: &Connection) {
        for m in ["u1", "other"] {
            c.execute(
                "INSERT INTO members (id, email, role) VALUES (?1, ?1, 'member')",
                [m],
            )
            .unwrap();
        }
        for (id, end) in [("a", 3_600_000), ("b", 600_000)] {
            c.execute(
                "INSERT INTO meetings (id, title, platform, status, owner_id, share) VALUES (?1, ?1, 'upload', 'ready', ?2, 'attendees')",
                params![id, if id == "b" { "other" } else { "u1" }],
            )
            .unwrap();
            c.execute(
                "INSERT INTO segments (meeting_id, start_ms, end_ms, text) VALUES (?1, 0, ?2, 'x')",
                params![id, end],
            )
            .unwrap();
        }
    }

    #[test]
    fn speech_cost_is_audio_minutes_times_the_rate_and_a_retry_replaces_the_row() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        seed(&c);
        record_speech(&c, "a", 7_800, NOV).unwrap();
        record_speech(&c, "a", 7_800, NOV).unwrap();
        record_speech(&c, "b", 7_800, NOV).unwrap();
        let n: i64 = c
            .query_row("SELECT count(*) FROM speech_usage", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 2, "a retry does not add a second row");
        // 60 min at 7800 micro-USD a minute = $0.468; 10 min = $0.078
        let t = month_totals(&c, NOV).unwrap();
        assert_eq!(t.speech_micro_usd, 468_000 + 78_000);
    }

    #[test]
    fn totals_cover_only_the_current_month_and_both_kinds_of_usage() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        seed(&c);
        record_speech(&c, "a", 7_800, OCT).unwrap(); // last month
        record_speech(&c, "b", 7_800, NOV).unwrap();
        c.execute_batch(&format!(
            "INSERT INTO llm_usage (meeting_id, task, provider, model, cost_micro_usd, at) VALUES ('b','summary','p','m',1000,{NOV});
             INSERT INTO llm_usage (task, provider, model, cost_micro_usd, at) VALUES ('ask','p','m',500,{NOV});
             INSERT INTO llm_usage (meeting_id, task, provider, model, cost_micro_usd, at) VALUES ('a','summary','p','m',9999,{OCT});
             INSERT INTO llm_usage (task, provider, model, at) VALUES ('ask','p','m',{NOV});"
        ))
        .unwrap();
        let t = month_totals(&c, NOV).unwrap();
        assert_eq!(
            (t.speech_micro_usd, t.llm_micro_usd),
            (78_000, 1_500),
            "no cost value counts as 0"
        );
        assert_eq!(month_totals(&c, OCT).unwrap().total(), 468_000 + 9_999);
    }

    #[test]
    fn the_cap_pauses_at_the_limit_and_zero_means_off() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        seed(&c);
        record_speech(&c, "b", 7_800, NOV).unwrap(); // 78_000 so far
        assert!(!paused(&c, 0, NOV).unwrap(), "0 is no cap");
        assert!(!paused(&c, 78_001, NOV).unwrap());
        assert!(paused(&c, 78_000, NOV).unwrap(), "reaching the cap pauses");
        assert!(
            !paused(&c, 78_000, NOV + 31 * 86_400_000).unwrap(),
            "a new month starts clean"
        );
    }

    #[test]
    fn per_meeting_lists_costliest_first_and_skips_free_meetings() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        seed(&c);
        c.execute(
            "INSERT INTO meetings (id, title, platform, status) VALUES ('c','c','upload','ready')",
            [],
        )
        .unwrap();
        record_speech(&c, "a", 7_800, NOV).unwrap();
        record_speech(&c, "b", 7_800, NOV).unwrap();
        c.execute(
            &format!("INSERT INTO llm_usage (meeting_id, task, provider, model, cost_micro_usd, at) VALUES ('b','s','p','m',1000,{NOV})"),
            [],
        )
        .unwrap();
        let rows = per_meeting(&c, NOV, 10, "u1").unwrap();
        fn list(rows: &[MeetingCost]) -> Vec<(&str, i64, i64)> {
            rows.iter()
                .map(|m| (m.meeting_id.as_str(), m.speech_micro_usd, m.llm_micro_usd))
                .collect()
        }
        // "b" belongs to someone else and is not shared: its title and cost are not listed to u1
        assert_eq!(list(&rows), vec![("a", 468_000, 0)]);
        c.execute("UPDATE meetings SET share='workspace' WHERE id='b'", [])
            .unwrap();
        let rows = per_meeting(&c, NOV, 10, "u1").unwrap();
        assert_eq!(list(&rows), vec![("a", 468_000, 0), ("b", 78_000, 1_000)]);
        assert_eq!(per_meeting(&c, NOV, 1, "u1").unwrap().len(), 1);
    }
}
