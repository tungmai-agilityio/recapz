//! Durable job queue in the tenant DB. A crash leaves a lock that expires, so the job is retried.
use crate::db::now_ms;
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

pub const MAX_ATTEMPTS: i64 = 3;
const LOCK_MS: i64 = 10 * 60 * 1000;

#[derive(Debug, Clone)]
pub struct Job {
    pub id: i64,
    pub kind: String,
    pub meeting_id: Option<String>,
    pub payload: Option<String>,
}

pub fn enqueue(
    c: &Connection,
    kind: &str,
    meeting_id: Option<&str>,
    payload: Option<&str>,
) -> Result<i64> {
    c.execute(
        "INSERT INTO jobs (kind, meeting_id, payload, run_after) VALUES (?1, ?2, ?3, ?4)",
        params![kind, meeting_id, payload, now_ms()],
    )?;
    Ok(c.last_insert_rowid())
}

/// Claims the oldest due job of a supported kind. Unsupported kinds stay queued.
pub fn claim(c: &Connection, kinds: &[&str]) -> Result<Option<Job>> {
    let now = now_ms();
    let mut sel = c.prepare(
        "SELECT id, kind, meeting_id, payload FROM jobs
         WHERE state IN ('queued','running') AND run_after <= ?1 AND (locked_until IS NULL OR locked_until < ?1)
         ORDER BY id",
    )?;
    let mut rows = sel.query([now])?;
    while let Some(r) = rows.next()? {
        let job = Job {
            id: r.get(0)?,
            kind: r.get(1)?,
            meeting_id: r.get(2)?,
            payload: r.get(3)?,
        };
        if !kinds.contains(&job.kind.as_str()) {
            continue;
        }
        c.execute(
            "UPDATE jobs SET state='running', attempts=attempts+1, locked_until=?2 WHERE id=?1",
            params![job.id, now + LOCK_MS],
        )?;
        return Ok(Some(job));
    }
    Ok(None)
}

pub fn finish(c: &Connection, id: i64, result: std::result::Result<(), String>) -> Result<()> {
    match result {
        Ok(()) => c.execute(
            "UPDATE jobs SET state='done', locked_until=NULL WHERE id=?1",
            [id],
        )?,
        Err(e) => {
            let attempts: i64 =
                c.query_row("SELECT attempts FROM jobs WHERE id=?1", [id], |r| r.get(0))?;
            let state = if attempts >= MAX_ATTEMPTS {
                "failed"
            } else {
                "queued"
            };
            let backoff = now_ms() + 1000 * (1 << attempts.min(10));
            c.execute(
                "UPDATE jobs SET state=?2, last_error=?3, run_after=?4, locked_until=NULL WHERE id=?1",
                params![id, state, e, backoff],
            )?;
            if state == "failed" {
                // Don't leave the meeting stuck in a working state.
                c.execute(
                    "UPDATE meetings SET status='failed', error=?2
                     WHERE id = (SELECT meeting_id FROM jobs WHERE id=?1)",
                    params![id, e.chars().take(500).collect::<String>()],
                )?;
            }
            1
        }
    };
    Ok(())
}

/// Runs at most one job; returns whether one ran.
pub fn run_once(
    c: &Connection,
    kinds: &[&str],
    handler: impl Fn(&Connection, &Job) -> Result<()>,
) -> Result<bool> {
    let Some(job) = claim(c, kinds)? else {
        return Ok(false);
    };
    let res = handler(c, &job).map_err(|e| format!("{e:#}"));
    finish(c, job.id, res)?;
    Ok(true)
}

/// Queues the meeting's latest failed job again from its own stage and clears the meeting error.
/// Returns false when the meeting has no failed job.
pub fn retry_failed(c: &Connection, meeting_id: &str) -> Result<bool> {
    let Some((id, kind)) = c
        .query_row(
            "SELECT id, kind FROM jobs WHERE meeting_id=?1 AND state='failed' ORDER BY id DESC LIMIT 1",
            [meeting_id],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?
    else {
        return Ok(false);
    };
    c.execute(
        "UPDATE jobs SET state='queued', attempts=0, run_after=?2, locked_until=NULL, last_error=NULL WHERE id=?1",
        params![id, now_ms()],
    )?;
    // Index and archive run after the report is ready, so their meeting stays ready.
    let status = match kind.as_str() {
        "transcribe" => "transcribing",
        "label" | "report" => "reporting",
        _ => "ready",
    };
    c.execute(
        "UPDATE meetings SET status=?2, error=NULL WHERE id=?1",
        params![meeting_id, status],
    )?;
    Ok(true)
}

pub fn state(c: &Connection, id: i64) -> Result<Option<(String, i64)>> {
    Ok(
        c.query_row("SELECT state, attempts FROM jobs WHERE id=?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_tenant;

    #[test]
    fn runs_retries_and_skips_unknown_kinds() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        let ok = enqueue(&c, "echo", None, Some("hi")).unwrap();
        let unknown = enqueue(&c, "later", None, None).unwrap();
        assert!(run_once(&c, &["echo"], |_, j| {
            assert_eq!(j.payload.as_deref(), Some("hi"));
            Ok(())
        })
        .unwrap());
        assert_eq!(state(&c, ok).unwrap().unwrap().0, "done");
        assert!(!run_once(&c, &["echo"], |_, _| Ok(())).unwrap());
        assert_eq!(state(&c, unknown).unwrap().unwrap().0, "queued");

        let bad = enqueue(&c, "boom", None, None).unwrap();
        run_once(&c, &["boom"], |_, _| anyhow::bail!("nope")).unwrap();
        assert_eq!(state(&c, bad).unwrap().unwrap(), ("queued".into(), 1));
    }
    #[test]
    fn retry_requeues_the_failed_stage_with_fresh_attempts_and_clears_the_error() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        c.execute(
            "INSERT INTO meetings (id, title, platform, status, error) VALUES ('m1','t','upload','failed','boom')",
            [],
        )
        .unwrap();
        assert!(!retry_failed(&c, "m1").unwrap());
        let id = enqueue(&c, "transcribe", Some("m1"), None).unwrap();
        c.execute(
            "UPDATE jobs SET state='failed', attempts=?2, last_error='boom' WHERE id=?1",
            params![id, MAX_ATTEMPTS],
        )
        .unwrap();
        assert!(retry_failed(&c, "m1").unwrap());
        assert_eq!(state(&c, id).unwrap().unwrap(), ("queued".into(), 0));
        let m: (String, Option<String>) = c
            .query_row(
                "SELECT status, error FROM meetings WHERE id='m1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(m, ("transcribing".to_string(), None));
        assert!(!retry_failed(&c, "m1").unwrap(), "nothing failed any more");
    }

    #[test]
    fn final_failure_marks_the_meeting_failed_but_a_retry_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        c.execute(
            "INSERT INTO meetings (id, title, platform, status) VALUES ('m1','t','upload','reporting')",
            [],
        )
        .unwrap();
        let status = |c: &Connection| -> (String, Option<String>) {
            c.query_row(
                "SELECT status, error FROM meetings WHERE id='m1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
        };
        let id = enqueue(&c, "boom", Some("m1"), None).unwrap();
        run_once(&c, &["boom"], |_, _| anyhow::bail!("first")).unwrap();
        assert_eq!(
            status(&c),
            ("reporting".to_string(), None),
            "retry keeps the status"
        );
        c.execute(
            "UPDATE jobs SET attempts=?2, run_after=0 WHERE id=?1",
            params![id, MAX_ATTEMPTS - 1],
        )
        .unwrap();
        run_once(&c, &["boom"], |_, _| {
            Err(anyhow::anyhow!("provider said no").context("report model"))
        })
        .unwrap();
        assert_eq!(state(&c, id).unwrap().unwrap().0, "failed");
        assert_eq!(
            status(&c),
            (
                "failed".to_string(),
                Some("report model: provider said no".to_string())
            )
        );
    }
}
