//! `transcribe` job: decode the meeting audio, recognize speech, store segments, mark the meeting ready.
use crate::{db::tenant_dir, jobs::Job};
use anyhow::{anyhow, Context, Result};
use recapz_speech::{decode::decode_to_16k_mono, transcribe::Transcriber};
use rusqlite::{params, Connection};
use std::path::Path;

pub const KIND: &str = "transcribe";

/// Safe to retry: segments are replaced, not appended.
pub fn handle(
    c: &Connection,
    job: &Job,
    data_dir: &Path,
    tenant_id: &str,
    tx: &dyn Transcriber,
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
        c.execute(
            "UPDATE meetings SET status='ready', error=NULL WHERE id=?1",
            [mid],
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
            handle(&c, &job, dir.path(), "t1", &StubTranscriber).unwrap();
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
    }

    #[test]
    fn undecodable_audio_fails_and_leaves_no_segments() {
        let (dir, c, job) = setup(Some(b"junk"));
        assert!(handle(&c, &job, dir.path(), "t1", &StubTranscriber).is_err());
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
