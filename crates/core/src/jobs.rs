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
            )?
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
    let res = handler(c, &job).map_err(|e| e.to_string());
    finish(c, job.id, res)?;
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
}
