//! `transcribe` job: decode the meeting audio, recognize speech, store segments, then either queue the
//! `report` job or mark the meeting ready.
use crate::{db::tenant_dir, jobs::Job};
use anyhow::{anyhow, Context, Result};
use recapz_speech::{decode::decode_to_16k_mono, transcribe::Transcriber};
use rusqlite::{params, Connection};
use std::path::Path;

pub const KIND: &str = "transcribe";

/// With `then_report`, the meeting moves to `reporting` and a `report` job is queued in the same
/// transaction; otherwise it is ready. Safe to retry: segments are replaced, not appended.
pub fn handle(
    c: &Connection,
    job: &Job,
    data_dir: &Path,
    tenant_id: &str,
    tx: &dyn Transcriber,
    then_report: bool,
) -> Result<()> {
    let mid = job
        .meeting_id
        .as_deref()
        .ok_or_else(|| anyhow!("job has no meeting"))?;
    let (rel, lang): (String, String) = c
        .query_row(
            "SELECT audio_path, language FROM meetings WHERE id=?1 AND audio_path IS NOT NULL",
            [mid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .context("meeting has no audio")?;
    c.execute(
        "UPDATE meetings SET status='transcribing' WHERE id=?1",
        [mid],
    )?;

    let samples = decode_to_16k_mono(&tenant_dir(data_dir, tenant_id).join(rel))?;
    let segments = tx.transcribe(&samples, &lang)?;

    c.execute_batch("BEGIN")?;
    let stored = (|| -> Result<()> {
        c.execute("DELETE FROM segments WHERE meeting_id=?1", [mid])?;
        for s in &segments {
            c.execute(
                "INSERT INTO segments (meeting_id, start_ms, end_ms, text) VALUES (?1, ?2, ?3, ?4)",
                params![mid, s.start_ms, s.end_ms, s.text],
            )?;
        }
        // Names come before the report and the index so both read the labelled transcript: jobs run
        // oldest first. Only a call recording has the samples file.
        if tenant_dir(data_dir, tenant_id)
            .join(format!("audio/{mid}/speakers.jsonl"))
            .exists()
        {
            crate::jobs::enqueue(c, crate::label_job::KIND, Some(mid), None)?;
        }
        if then_report {
            c.execute(
                "UPDATE meetings SET status='reporting', error=NULL WHERE id=?1",
                [mid],
            )?;
            crate::jobs::enqueue(c, crate::report_job::KIND, Some(mid), None)?;
        } else {
            c.execute(
                "UPDATE meetings SET status='ready', error=NULL WHERE id=?1",
                [mid],
            )?;
        }
        // Ask search is built from the stored segments, in its own retryable job.
        c.execute(
            "INSERT INTO jobs (kind, meeting_id, run_after)
             SELECT ?1, ?2, ?3 WHERE NOT EXISTS
               (SELECT 1 FROM jobs WHERE kind=?1 AND meeting_id=?2 AND state='queued')",
            params![crate::index_job::KIND, mid, crate::db::now_ms()],
        )?;
        Ok(())
    })();
    c.execute_batch(if stored.is_ok() { "COMMIT" } else { "ROLLBACK" })?;
    stored
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::open_tenant, jobs};
    use recapz_speech::transcribe::StubTranscriber;

    fn wav_1s() -> Vec<u8> {
        let data = vec![0u8; 32_000];
        let mut w = Vec::new();
        w.extend(b"RIFF");
        w.extend((36 + data.len() as u32).to_le_bytes());
        w.extend(b"WAVEfmt ");
        w.extend(16u32.to_le_bytes());
        w.extend(1u16.to_le_bytes());
        w.extend(1u16.to_le_bytes());
        w.extend(16_000u32.to_le_bytes());
        w.extend(32_000u32.to_le_bytes());
        w.extend(2u16.to_le_bytes());
        w.extend(16u16.to_le_bytes());
        w.extend(b"data");
        w.extend((data.len() as u32).to_le_bytes());
        w.extend(data);
        w
    }

    fn setup(audio: Option<&[u8]>) -> (tempfile::TempDir, Connection, Job) {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        let a = tenant_dir(dir.path(), "t1").join("audio/m1");
        std::fs::create_dir_all(&a).unwrap();
        if let Some(b) = audio {
            std::fs::write(a.join("a.wav"), b).unwrap();
        }
        c.execute(
            "INSERT INTO meetings (id, title, platform, status, audio_path) VALUES ('m1','t','upload','uploaded','audio/m1/a.wav')",
            [],
        )
        .unwrap();
        jobs::enqueue(&c, KIND, Some("m1"), None).unwrap();
        let job = jobs::claim(&c, &[KIND]).unwrap().unwrap();
        (dir, c, job)
    }

    #[test]
    fn writes_segments_and_marks_ready_even_when_rerun() {
        let wav = wav_1s();
        let (dir, c, job) = setup(Some(&wav));
        for _ in 0..2 {
            handle(&c, &job, dir.path(), "t1", &StubTranscriber, false).unwrap();
        }
        let rows: Vec<(i64, i64, String)> = c
            .prepare("SELECT start_ms, end_ms, text FROM segments WHERE meeting_id='m1' ORDER BY start_ms")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows.len(), 2, "rerun must not duplicate segments");
        assert_eq!((rows[0].0, rows[1].1), (0, 1000));
        let st: String = c
            .query_row("SELECT status FROM meetings WHERE id='m1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(st, "ready");
        let queued: i64 = c
            .query_row(
                "SELECT count(*) FROM jobs WHERE kind='index' AND meeting_id='m1' AND state='queued'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(queued, 1, "one index job, even after a rerun");
    }

    #[test]
    fn then_report_queues_report_job_and_is_not_ready_yet() {
        let wav = wav_1s();
        let (dir, c, job) = setup(Some(&wav));
        for _ in 0..2 {
            handle(&c, &job, dir.path(), "t1", &StubTranscriber, true).unwrap();
        }
        let st: String = c
            .query_row("SELECT status FROM meetings WHERE id='m1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(st, "reporting");
        // A retried transcribe queues a second report job; the report job itself is idempotent.
        let n: i64 = c
            .query_row(
                "SELECT count(*) FROM jobs WHERE kind='report' AND meeting_id='m1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 2);
    }

    #[test]
    fn a_call_recording_queues_label_before_report_and_index_an_upload_queues_none() {
        let wav = wav_1s();
        let (dir, c, job) = setup(Some(&wav));
        handle(&c, &job, dir.path(), "t1", &StubTranscriber, true).unwrap();
        let kinds = |c: &Connection| -> Vec<String> {
            c.prepare("SELECT kind FROM jobs WHERE state='queued' ORDER BY id")
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert_eq!(
            kinds(&c),
            vec!["report", "index"],
            "an upload has no samples"
        );

        std::fs::write(
            tenant_dir(dir.path(), "t1").join("audio/m1/speakers.jsonl"),
            "",
        )
        .unwrap();
        c.execute("DELETE FROM jobs WHERE state='queued'", [])
            .unwrap();
        handle(&c, &job, dir.path(), "t1", &StubTranscriber, true).unwrap();
        assert_eq!(kinds(&c), vec!["label", "report", "index"]);
    }

    #[test]
    fn undecodable_audio_fails_and_leaves_no_segments() {
        let (dir, c, job) = setup(Some(b"junk"));
        assert!(handle(&c, &job, dir.path(), "t1", &StubTranscriber, false).is_err());
        let n: i64 = c
            .query_row("SELECT count(*) FROM segments", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
        let st: String = c
            .query_row("SELECT status FROM meetings WHERE id='m1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(st, "transcribing");
    }
}
