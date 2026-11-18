//! Hard delete of one meeting: audio files, then every database row about it, including search rows.
//! Used for opt-out; retention and per-participant purge reuse it later.
use crate::{
    blobs::{audio_key, Blobs},
    db::tenant_dir,
};
use anyhow::{bail, Result};
use rusqlite::{Connection, OptionalExtension};
use std::{fs, io, path::Path};

/// Returns false when the meeting does not exist. Safe to repeat: the recording goes first (the
/// object in the store, then local files), so a failure partway leaves nothing readable and a
/// retry finishes the job. A recording that lives in the store with no `blobs` given is an error,
/// never a silently kept object.
pub fn purge_meeting(
    c: &Connection,
    data_dir: &Path,
    tenant_id: &str,
    meeting_id: &str,
    blobs: Option<&dyn Blobs>,
) -> Result<bool> {
    if meeting_id.is_empty()
        || !meeting_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    {
        bail!("unsafe meeting id");
    }
    let remote: Option<(Option<String>, bool)> = c
        .query_row(
            "SELECT audio_path, audio_remote FROM meetings WHERE id = ?1",
            [meeting_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((Some(rel), true)) = remote {
        let Some(b) = blobs else {
            bail!("recording is in object storage but no store is configured");
        };
        b.delete(&audio_key(tenant_id, &rel))?;
    }
    match fs::remove_dir_all(
        tenant_dir(data_dir, tenant_id)
            .join("audio")
            .join(meeting_id),
    ) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    c.execute_batch("BEGIN")?;
    let done = (|| -> Result<bool> {
        // Segments, speakers, reports, action items and chunks (with their search rows, through the
        // delete trigger) go with the meeting row; the rest have no foreign key.
        c.execute("DELETE FROM jobs WHERE meeting_id = ?1", [meeting_id])?;
        c.execute("DELETE FROM llm_usage WHERE meeting_id = ?1", [meeting_id])?;
        Ok(c.execute("DELETE FROM meetings WHERE id = ?1", [meeting_id])? > 0)
    })();
    c.execute_batch(if done.is_ok() { "COMMIT" } else { "ROLLBACK" })?;
    done
}

/// Hard delete of one member's document: the file first (a failure leaves nothing readable and a
/// retry finishes), then its row; chunks, search rows and vectors go through foreign keys and the
/// delete trigger, and queued ingest jobs are removed. False when the member does not own it.
pub fn purge_document(
    c: &Connection,
    data_dir: &Path,
    tenant_id: &str,
    member_id: &str,
    doc_id: &str,
) -> Result<bool> {
    if !crate::document::safe_id(doc_id) {
        bail!("unsafe document id");
    }
    let Some(path) = c
        .query_row(
            "SELECT path FROM documents WHERE id = ?1 AND member_id = ?2",
            [doc_id, member_id],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    else {
        return Ok(false);
    };
    match fs::remove_file(tenant_dir(data_dir, tenant_id).join(path)) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    c.execute_batch("BEGIN")?;
    let done = (|| -> Result<bool> {
        c.execute(
            "DELETE FROM jobs WHERE kind = ?1 AND payload = ?2",
            [crate::document::KIND, doc_id],
        )?;
        Ok(c.execute("DELETE FROM documents WHERE id = ?1", [doc_id])? > 0)
    })();
    c.execute_batch(if done.is_ok() { "COMMIT" } else { "ROLLBACK" })?;
    done
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_tenant;

    fn count(c: &Connection, sql: &str) -> i64 {
        c.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn seed(c: &Connection, dir: &Path, id: &str, word: &str) {
        let audio = tenant_dir(dir, "t1").join("audio").join(id);
        fs::create_dir_all(&audio).unwrap();
        fs::write(audio.join("a.wav"), b"RIFF").unwrap();
        c.execute_batch(&format!(
            "INSERT INTO meetings (id, title, platform, status, audio_path) VALUES ('{id}','t','upload','ready','audio/{id}/a.wav');
             INSERT INTO speakers (id, meeting_id, label) VALUES ('s-{id}','{id}','Speaker 1');
             INSERT INTO segments (meeting_id, speaker_id, start_ms, end_ms, text) VALUES ('{id}','s-{id}',0,1,'{word}');
             INSERT INTO reports (meeting_id, summary) VALUES ('{id}','{word}');
             INSERT INTO action_items (id, meeting_id, text) VALUES ('a-{id}','{id}','{word}');
             INSERT INTO chunks (meeting_id, start_ms, end_ms, text, embedding) VALUES ('{id}',0,1,'{word}',x'00');
             INSERT INTO llm_usage (meeting_id, task, provider, model, at) VALUES ('{id}','summary','p','m',1);
             INSERT INTO speech_usage (meeting_id, kind, audio_seconds, cost_micro_usd, at) VALUES ('{id}','batch',60,1,1);
             INSERT INTO jobs (kind, meeting_id, run_after) VALUES ('report','{id}',0);"
        ))
        .unwrap();
    }

    #[test]
    fn removes_files_rows_and_search_entries_for_that_meeting_only() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        seed(&c, dir.path(), "m1", "zebrafish");
        seed(&c, dir.path(), "m2", "walrus");
        assert_eq!(
            count(
                &c,
                "SELECT count(*) FROM chunks_fts WHERE chunks_fts MATCH 'zebrafish'"
            ),
            1,
            "seed must be searchable"
        );

        assert!(purge_meeting(&c, dir.path(), "t1", "m1", None).unwrap());

        for (table, col) in [
            ("meetings", "id"),
            ("speakers", "meeting_id"),
            ("segments", "meeting_id"),
            ("reports", "meeting_id"),
            ("action_items", "meeting_id"),
            ("chunks", "meeting_id"),
            ("llm_usage", "meeting_id"),
            ("speech_usage", "meeting_id"),
            ("jobs", "meeting_id"),
        ] {
            assert_eq!(
                count(
                    &c,
                    &format!("SELECT count(*) FROM {table} WHERE {col} = 'm1'")
                ),
                0,
                "{table}"
            );
            assert_eq!(
                count(&c, &format!("SELECT count(*) FROM {table}")),
                1,
                "{table}: the other meeting stays"
            );
        }
        assert_eq!(
            count(
                &c,
                "SELECT count(*) FROM chunks_fts WHERE chunks_fts MATCH 'zebrafish'"
            ),
            0,
            "search row removed"
        );
        assert_eq!(
            count(
                &c,
                "SELECT count(*) FROM chunks_fts WHERE chunks_fts MATCH 'walrus'"
            ),
            1
        );
        assert!(!tenant_dir(dir.path(), "t1").join("audio/m1").exists());
        assert!(tenant_dir(dir.path(), "t1").join("audio/m2/a.wav").exists());
        assert!(
            !purge_meeting(&c, dir.path(), "t1", "m1", None).unwrap(),
            "second call finds nothing"
        );
    }

    #[test]
    fn rejects_ids_that_could_escape_the_audio_folder() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        for bad in ["", "..", "../x", "a/b"] {
            assert!(
                purge_meeting(&c, dir.path(), "t1", bad, None).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_recording_in_the_store_is_deleted_there_first_and_never_kept_silently() {
        use crate::{archive_job::tests::Fake, blobs::Blobs};
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        seed(&c, dir.path(), "m1", "zebrafish");
        c.execute("UPDATE meetings SET audio_remote=1 WHERE id='m1'", [])
            .unwrap();
        let store = Fake::default();
        store.objects.lock().unwrap().extend([
            ("t1/audio/m1/a.wav".to_string(), vec![1]),
            ("t1/audio/m2/a.wav".to_string(), vec![2]),
        ]);
        // No store configured: refuse, and leave the meeting and the object alone.
        assert!(purge_meeting(&c, dir.path(), "t1", "m1", None).is_err());
        assert_eq!(count(&c, "SELECT count(*) FROM meetings"), 1);
        assert_eq!(store.objects.lock().unwrap().len(), 2);
        assert!(purge_meeting(&c, dir.path(), "t1", "m1", Some(&store as &dyn Blobs)).unwrap());
        assert_eq!(
            store
                .objects
                .lock()
                .unwrap()
                .iter()
                .map(|o| o.0.as_str())
                .collect::<Vec<_>>(),
            vec!["t1/audio/m2/a.wav"],
            "only that meeting's object goes"
        );
        assert_eq!(count(&c, "SELECT count(*) FROM meetings"), 0);
    }
}
