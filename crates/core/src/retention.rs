//! Audio retention: past the workspace's retention period a finished meeting's audio is deleted
//! (the object in the store, or the local file). Transcript, report and search rows stay.
//! Transcript retention, when set, hard deletes the whole meeting past its period.
use crate::{
    blobs::{audio_key, Blobs},
    db::tenant_dir,
    purge::purge_meeting,
    settings,
};
use anyhow::Result;
use rusqlite::Connection;
use std::{fs, io, path::Path};

const DAY_MS: i64 = 86_400_000;

/// Deletes the audio of meetings older than the retention period; returns how many. Safe to repeat:
/// the object goes first and the row is cleared after, so a failure leaves the audio listed and the
/// next sweep finishes. A remote recording with no store configured is skipped, never forgotten.
pub fn sweep(
    c: &Connection,
    data_dir: &Path,
    tenant_id: &str,
    blobs: Option<&dyn Blobs>,
    now_ms: i64,
) -> Result<usize> {
    let days = settings::audio_retention_days(c)?;
    if days == 0 {
        return Ok(0);
    }
    let cutoff = now_ms - i64::from(days) * DAY_MS;
    // Only finished meetings: one being recorded or processed still needs its audio.
    let old: Vec<(String, String, bool)> = c
        .prepare(
            "SELECT id, audio_path, audio_remote FROM meetings
             WHERE audio_path IS NOT NULL AND status IN ('ready','failed')
               AND COALESCE(ended_at, created_at) < ?1",
        )?
        .query_map([cutoff], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;
    let mut n = 0;
    for (id, rel, remote) in old {
        if remote {
            let Some(b) = blobs else { continue };
            b.delete(&audio_key(tenant_id, &rel))?;
        }
        match fs::remove_file(tenant_dir(data_dir, tenant_id).join(&rel)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
        c.execute(
            "UPDATE meetings SET audio_path=NULL, audio_remote=0 WHERE id=?1",
            [&id],
        )?;
        n += 1;
    }
    Ok(n)
}

/// Hard deletes finished meetings older than the transcript retention period (audio, transcript,
/// report, action items and search rows, through `purge_meeting`); returns how many. 0 days keeps
/// everything. A remote recording with no store configured is skipped, never forgotten.
pub fn sweep_meetings(
    c: &Connection,
    data_dir: &Path,
    tenant_id: &str,
    blobs: Option<&dyn Blobs>,
    now_ms: i64,
) -> Result<usize> {
    let days = settings::transcript_retention_days(c)?;
    if days == 0 {
        return Ok(0);
    }
    let cutoff = now_ms - i64::from(days) * DAY_MS;
    let old: Vec<(String, bool)> = c
        .prepare(
            "SELECT id, COALESCE(audio_remote, 0) FROM meetings
             WHERE status IN ('ready','failed') AND COALESCE(ended_at, created_at) < ?1",
        )?
        .query_map([cutoff], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut n = 0;
    for (id, remote) in old {
        if remote && blobs.is_none() {
            continue;
        }
        if purge_meeting(c, data_dir, tenant_id, &id, blobs)? {
            n += 1;
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{archive_job::tests::Fake, db::open_tenant};

    #[test]
    fn only_old_finished_audio_goes_object_or_file_and_the_transcript_stays() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        let day = DAY_MS;
        let now = 200 * day;
        // (id, status, ended days ago, remote)
        for (id, status, ago, remote) in [
            ("old-local", "ready", 100, false),
            ("old-remote", "failed", 91, true),
            ("fresh", "ready", 89, false),
            ("busy", "transcribing", 500, false),
        ] {
            let a = tenant_dir(dir.path(), "t1").join(format!("audio/{id}"));
            fs::create_dir_all(&a).unwrap();
            if !remote {
                fs::write(a.join("r.wav"), b"x").unwrap();
            }
            c.execute(
                "INSERT INTO meetings (id, title, platform, status, audio_path, audio_remote, ended_at, created_at)
                 VALUES (?1,'t','meet',?2,?3,?4,?5,0)",
                rusqlite::params![id, status, format!("audio/{id}/r.wav"), remote, now - ago * day],
            )
            .unwrap();
            c.execute(
                "INSERT INTO segments (meeting_id, start_ms, end_ms, text) VALUES (?1,0,1,'kept')",
                [id],
            )
            .unwrap();
        }
        let store = Fake::default();
        store.objects.lock().unwrap().extend([
            ("t1/audio/old-remote/r.wav".to_string(), vec![1]),
            ("t1/audio/other/r.wav".to_string(), vec![2]),
        ]);
        // remote audio with no store configured is skipped, not forgotten
        assert_eq!(sweep(&c, dir.path(), "t1", None, now).unwrap(), 1);
        assert_eq!(sweep(&c, dir.path(), "t1", Some(&store), now).unwrap(), 1);
        let left: Vec<String> = c
            .prepare("SELECT id FROM meetings WHERE audio_path IS NOT NULL ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(left, vec!["busy", "fresh"]);
        assert!(!tenant_dir(dir.path(), "t1")
            .join("audio/old-local/r.wav")
            .exists());
        assert!(tenant_dir(dir.path(), "t1")
            .join("audio/fresh/r.wav")
            .exists());
        assert_eq!(
            store
                .objects
                .lock()
                .unwrap()
                .iter()
                .map(|o| o.0.as_str())
                .collect::<Vec<_>>(),
            vec!["t1/audio/other/r.wav"]
        );
        let segs: i64 = c
            .query_row("SELECT count(*) FROM segments", [], |r| r.get(0))
            .unwrap();
        assert_eq!(segs, 4, "transcripts stay");
        // keep-forever switch
        settings::set_audio_retention_days(&c, 0).unwrap();
        assert_eq!(sweep(&c, dir.path(), "t1", None, now * 10).unwrap(), 0);
    }

    #[test]
    fn transcript_sweep_hard_deletes_only_old_finished_meetings_with_everything_attached() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        let now = 500 * DAY_MS;
        // (id, status, ended days ago)
        for (id, status, ago) in [
            ("old", "ready", 91),
            ("old-failed", "failed", 400),
            ("fresh", "ready", 89),
            ("busy", "transcribing", 500),
        ] {
            let a = tenant_dir(dir.path(), "t1").join(format!("audio/{id}"));
            fs::create_dir_all(&a).unwrap();
            fs::write(a.join("r.wav"), b"x").unwrap();
            c.execute(
                "INSERT INTO meetings (id, title, platform, status, audio_path, audio_remote, ended_at, created_at)
                 VALUES (?1,'t','meet',?2,?3,0,?4,0)",
                rusqlite::params![id, status, format!("audio/{id}/r.wav"), now - ago * DAY_MS],
            )
            .unwrap();
            c.execute(
                "INSERT INTO segments (meeting_id, start_ms, end_ms, text) VALUES (?1,0,1,'words')",
                [id],
            )
            .unwrap();
        }
        // keep (the default) deletes nothing, however old
        assert_eq!(sweep_meetings(&c, dir.path(), "t1", None, now).unwrap(), 0);
        settings::set_transcript_retention_days(&c, 90).unwrap();
        assert_eq!(sweep_meetings(&c, dir.path(), "t1", None, now).unwrap(), 2);
        let left: Vec<String> = c
            .prepare("SELECT id FROM meetings ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(left, vec!["busy", "fresh"]);
        let segs: i64 = c
            .query_row("SELECT count(*) FROM segments", [], |r| r.get(0))
            .unwrap();
        assert_eq!(segs, 2, "transcripts of deleted meetings are gone");
        for gone in ["old", "old-failed"] {
            assert!(!tenant_dir(dir.path(), "t1")
                .join("audio")
                .join(gone)
                .exists());
        }
        assert!(tenant_dir(dir.path(), "t1")
            .join("audio/fresh/r.wav")
            .exists());
        assert_eq!(sweep_meetings(&c, dir.path(), "t1", None, now).unwrap(), 0);
    }
}
