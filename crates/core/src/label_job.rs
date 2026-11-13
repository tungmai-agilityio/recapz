//! `label` job: names the people in a call recording. It reads the active-speaker samples the
//! notetaker saved next to the audio and gives each transcript segment its most frequent speaker.
//! The names are participant data: they are stored in the tenant database and never logged, and no
//! error text here contains one.
use crate::{db::tenant_dir, jobs::Job, speakers};
use anyhow::{anyhow, Result};
use rusqlite::{params, Connection};
use std::path::Path;

pub const KIND: &str = "label";

/// Safe to retry: the meeting's speakers and every segment's speaker are replaced, not added to.
/// A meeting without samples (an upload, or a call where nobody was marked) is left as it is.
pub fn handle(c: &Connection, job: &Job, data_dir: &Path, tenant_id: &str) -> Result<()> {
    let mid = job
        .meeting_id
        .as_deref()
        .ok_or_else(|| anyhow!("job has no meeting"))?;
    let path = tenant_dir(data_dir, tenant_id).join(format!("audio/{mid}/speakers.jsonl"));
    let samples = match std::fs::read_to_string(path) {
        Ok(t) => speakers::parse_jsonl(&t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(anyhow!(
                "could not read the speaker samples: {:?}",
                e.kind()
            ))
        }
    };
    let segments: Vec<(i64, i64, i64)> = c
        .prepare("SELECT id, start_ms, end_ms FROM segments WHERE meeting_id=?1")?
        .query_map([mid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;

    c.execute_batch("BEGIN")?;
    let done = (|| -> Result<()> {
        c.execute(
            "UPDATE segments SET speaker_id=NULL WHERE meeting_id=?1",
            [mid],
        )?;
        c.execute("DELETE FROM speakers WHERE meeting_id=?1", [mid])?;
        let mut ids: Vec<(String, String)> = Vec::new(); // (name, speaker id)
        for (seg, start, end) in segments {
            let Some(name) = speakers::speaker_for(&samples, start, end) else {
                continue;
            };
            let id = match ids.iter().find(|(n, _)| n == name) {
                Some((_, id)) => id.clone(),
                None => {
                    let id = uuid::Uuid::now_v7().to_string();
                    c.execute(
                        "INSERT INTO speakers (id, meeting_id, label, display_name) VALUES (?1, ?2, ?3, ?3)",
                        params![id, mid, name],
                    )?;
                    ids.push((name.to_string(), id.clone()));
                    id
                }
            };
            c.execute(
                "UPDATE segments SET speaker_id=?2 WHERE id=?1",
                params![seg, id],
            )?;
        }
        Ok(())
    })();
    c.execute_batch(if done.is_ok() { "COMMIT" } else { "ROLLBACK" })?;
    done
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::open_tenant, jobs};

    fn setup(jsonl: Option<&str>) -> (tempfile::TempDir, Connection, Job) {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        let a = tenant_dir(dir.path(), "t1").join("audio/m1");
        std::fs::create_dir_all(&a).unwrap();
        if let Some(t) = jsonl {
            std::fs::write(a.join("speakers.jsonl"), t).unwrap();
        }
        c.execute(
            "INSERT INTO meetings (id, title, platform, status) VALUES ('m1','t','meet','transcribing')",
            [],
        )
        .unwrap();
        for (s, e) in [(0, 1000), (1000, 2000), (5000, 6000)] {
            c.execute(
                "INSERT INTO segments (meeting_id, start_ms, end_ms, text) VALUES ('m1', ?1, ?2, 'x')",
                [s, e],
            )
            .unwrap();
        }
        jobs::enqueue(&c, KIND, Some("m1"), None).unwrap();
        let job = jobs::claim(&c, &[KIND]).unwrap().unwrap();
        (dir, c, job)
    }

    fn names(c: &Connection) -> Vec<Option<String>> {
        c.prepare(
            "SELECT p.display_name FROM segments g LEFT JOIN speakers p ON p.id=g.speaker_id ORDER BY g.start_ms",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    }

    fn count(c: &Connection, sql: &str) -> i64 {
        c.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn names_each_segment_from_its_samples_and_a_rerun_adds_nothing() {
        // 0..1000: Ann x3, Bo x1. 1000..2000 (inclusive start): Bo x2. 5000..6000: nobody.
        let j = "{\"at_ms\":0,\"speaker\":\"Ann\"}\n{\"at_ms\":250,\"speaker\":\"Ann\"}\n\
                 {\"at_ms\":500,\"speaker\":\"Ann\"}\n{\"at_ms\":750,\"speaker\":\"Bo\"}\n\
                 {\"at_ms\":1500,\"speaker\":\"Bo\"}\n{\"at_ms\":1750,\"speaker\":\"Bo\"}\n";
        let (dir, c, job) = setup(Some(j));
        for _ in 0..2 {
            handle(&c, &job, dir.path(), "t1").unwrap();
        }
        assert_eq!(names(&c), vec![Some("Ann".into()), Some("Bo".into()), None]);
        assert_eq!(
            count(&c, "SELECT count(*) FROM speakers"),
            2,
            "one row per name, even after a rerun"
        );
        assert_eq!(
            count(&c, "SELECT count(DISTINCT speaker_id) FROM segments"),
            2
        );
    }

    #[test]
    fn no_samples_file_leaves_the_meeting_alone() {
        let (dir, c, job) = setup(None);
        handle(&c, &job, dir.path(), "t1").unwrap();
        assert_eq!(count(&c, "SELECT count(*) FROM speakers"), 0);
        assert_eq!(names(&c), vec![None, None, None]);
    }

    #[test]
    fn a_job_without_a_meeting_fails() {
        let (dir, c, mut job) = setup(None);
        job.meeting_id = None;
        assert!(handle(&c, &job, dir.path(), "t1").is_err());
    }
}
