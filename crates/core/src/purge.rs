//! Hard delete of one meeting: audio files, then every database row about it, including search rows.
//! Used for opt-out and retention; `purge_participant` removes one person's speech everywhere.
use crate::{
    blobs::{audio_key, Blobs},
    db::tenant_dir,
};
use anyhow::{bail, Result};
use rusqlite::{Connection, OptionalExtension};
use std::{fs, io, path::Path};

/// Ends a purge transaction. After a commit the WAL is truncated, so the deleted rows do not stay
/// readable in the write-ahead log.
fn finish<T>(c: &Connection, done: Result<T>) -> Result<T> {
    c.execute_batch(if done.is_ok() { "COMMIT" } else { "ROLLBACK" })?;
    if done.is_ok() {
        scrub(c)?;
    }
    done
}

/// Folds the search indexes' delete markers into their data (an FTS5 delete only marks the old
/// terms) and truncates the WAL when no reader blocks it: the delete is already committed, so a busy
/// WAL is not an error for the caller, and the weekly vacuum truncates it later. ponytail: `optimize` rewrites each whole index; use `merge` if a
/// workspace grows large enough for a purge to feel slow.
fn scrub(c: &Connection) -> Result<()> {
    for t in ["chunks_fts", "document_chunks_fts"] {
        c.execute(&format!("INSERT INTO {t}({t}) VALUES('optimize')"), [])?;
    }
    truncate_wal(c)?;
    Ok(())
}

/// Empties the WAL; true when it did. A busy answer (another connection is reading) leaves the WAL in
/// place.
fn truncate_wal(c: &Connection) -> Result<bool> {
    let busy: i64 = c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
    Ok(busy == 0)
}

/// Rewrites the database file so free pages (where deleted text may linger) are dropped, then
/// truncates the WAL. Runs weekly from the job runner.
pub fn vacuum(c: &Connection) -> Result<()> {
    scrub(c)?;
    c.execute_batch("VACUUM")?;
    if !truncate_wal(c)? {
        bail!("the WAL could not be truncated: the database is busy");
    }
    Ok(())
}

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
    finish(c, done)
}

/// How many meetings one person spoke in, matched by the speaker's display name (case and edge
/// spaces ignored). Shown in the confirm dialog.
pub fn participant_meetings(c: &Connection, name: &str) -> Result<i64> {
    Ok(c.query_row(
        "SELECT count(DISTINCT meeting_id) FROM speakers WHERE lower(trim(display_name)) = lower(trim(?1))",
        [name],
        |r| r.get(0),
    )?)
}

/// Hard delete of everything one participant said, in every meeting. Their segments (with word
/// timings) and speaker rows go; chunks and search rows of those meetings go too, and so do the
/// report and its action items, because any of them may quote the person. Nothing readable is left
/// while the rebuild runs: an `index` and a `report` job are queued per meeting and rebuild from the
/// remaining transcript (a meeting left with no transcript stays without them). One transaction,
/// safe to repeat. Returns the meetings changed. ponytail: the recording itself is not cut;
/// removing one voice from audio needs diarised audio, add when the spec asks for it.
pub fn purge_participant(c: &Connection, name: &str) -> Result<i64> {
    if name.trim().is_empty() {
        bail!("empty participant name");
    }
    c.execute_batch("BEGIN")?;
    let done = (|| -> Result<i64> {
        let ids: Vec<String> = c
            .prepare(
                "SELECT DISTINCT meeting_id FROM speakers WHERE lower(trim(display_name)) = lower(trim(?1))",
            )?
            .query_map([name], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let who = "SELECT id FROM speakers WHERE meeting_id = ?1 AND lower(trim(display_name)) = lower(trim(?2))";
        for m in &ids {
            c.execute(
                &format!("DELETE FROM segments WHERE meeting_id = ?1 AND speaker_id IN ({who})"),
                [m, name],
            )?;
            c.execute(
                &format!("DELETE FROM speakers WHERE id IN ({who})"),
                [m, name],
            )?;
            // The chunk delete trigger removes the search rows.
            c.execute("DELETE FROM chunks WHERE meeting_id = ?1", [m])?;
            c.execute("DELETE FROM reports WHERE meeting_id = ?1", [m])?;
            c.execute("DELETE FROM action_items WHERE meeting_id = ?1", [m])?;
            c.execute(
                "DELETE FROM jobs WHERE meeting_id = ?1 AND kind IN ('index','report') AND state = 'queued'",
                [m],
            )?;
            let left: i64 = c.query_row(
                "SELECT count(*) FROM segments WHERE meeting_id = ?1",
                [m],
                |r| r.get(0),
            )?;
            if left > 0 {
                crate::jobs::enqueue(c, crate::index_job::KIND, Some(m), None)?;
                crate::jobs::enqueue(c, crate::report_job::KIND, Some(m), None)?;
                c.execute(
                    "UPDATE meetings SET status = 'reporting' WHERE id = ?1 AND status = 'ready'",
                    [m],
                )?;
            }
        }
        Ok(ids.len() as i64)
    })();
    finish(c, done)
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
    finish(c, done)
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
    fn a_purge_leaves_no_deleted_text_in_the_wal_or_the_database_file() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        seed(&c, dir.path(), "m1", "zebrafishmarker");
        seed(&c, dir.path(), "m2", "walrus");
        let db = tenant_dir(dir.path(), "t1").join("recapz.sqlite");
        let has = |p: &Path, m: &[u8]| fs::read(p).unwrap().windows(m.len()).any(|w| w == m);
        assert!(
            has(&db.with_extension("sqlite-wal"), b"zebrafishmarker")
                || has(&db, b"zebrafishmarker")
        );
        assert!(purge_meeting(&c, dir.path(), "t1", "m1", None).unwrap());
        assert_eq!(
            fs::metadata(db.with_extension("sqlite-wal")).unwrap().len(),
            0,
            "WAL emptied"
        );
        assert!(
            !has(&db, b"zebrafishmarker"),
            "text gone from the database file"
        );
        assert!(has(&db, b"walrus"), "the other meeting stays");
    }

    #[test]
    fn a_purge_still_succeeds_when_a_reader_keeps_the_wal_from_truncating() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        seed(&c, dir.path(), "m1", "zebrafish");
        let reader = open_tenant(dir.path(), "t1").unwrap();
        reader.execute_batch("BEGIN").unwrap();
        count(&reader, "SELECT count(*) FROM meetings");
        assert!(purge_meeting(&c, dir.path(), "t1", "m1", None).unwrap());
        assert_eq!(count(&c, "SELECT count(*) FROM meetings"), 0);
        assert!(vacuum(&c).is_err(), "vacuum reports a busy WAL");
    }

    #[test]
    fn vacuum_drops_deleted_text_from_free_pages() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        seed(&c, dir.path(), "m1", "zebrafishmarker");
        c.execute("DELETE FROM meetings", []).unwrap();
        vacuum(&c).unwrap();
        let db = tenant_dir(dir.path(), "t1").join("recapz.sqlite");
        assert!(!fs::read(&db)
            .unwrap()
            .windows(15)
            .any(|w| w == b"zebrafishmarker"));
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
    fn a_participants_words_leave_transcript_search_and_ask_and_the_rest_is_rebuilt() {
        use crate::{embed::HashEmbedder, index_job, jobs, search::search};
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        c.execute(
            "INSERT INTO members (id, email, role) VALUES ('o','o@x.test','admin')",
            [],
        )
        .unwrap();
        for m in ["m1", "m2", "m3"] {
            c.execute(
                "INSERT INTO meetings (id, title, platform, status, owner_id, share) VALUES (?1,'t','upload','ready','o','workspace')",
                [m],
            )
            .unwrap();
        }
        // m1 and m2: Lan says zebrafish, Minh says walrus. m3: Minh only.
        c.execute_batch(
            r#"INSERT INTO speakers (id, meeting_id, label, display_name) VALUES
               ('l1','m1','S1','Lan'),('n1','m1','S2','Minh'),('l2','m2','S1',' lan '),('n2','m2','S2','Minh'),('n3','m3','S1','Minh');
             INSERT INTO segments (meeting_id, speaker_id, start_ms, end_ms, text, words_json) VALUES
               ('m1','l1',0,1000,'zebrafish budget','["zebrafish"]'),('m1','n1',1000,2000,'walrus plan',NULL),
               ('m2','l2',0,1000,'zebrafish again',NULL),('m2','n2',1000,2000,'walrus again',NULL),
               ('m3','n3',0,1000,'walrus alone',NULL);
             INSERT INTO reports (meeting_id, summary) VALUES ('m1','zebrafish summary'),('m2','x'),('m3','y');
             INSERT INTO action_items (id, meeting_id, text) VALUES ('a1','m1','zebrafish task'),('a3','m3','keep');"#,
        )
        .unwrap();
        for m in ["m1", "m2", "m3"] {
            jobs::enqueue(&c, "index", Some(m), None).unwrap();
        }
        let drain = || {
            while let Some(job) = jobs::claim(&c, &["index"]).unwrap() {
                index_job::handle(&c, &job, &HashEmbedder).unwrap();
                jobs::finish(&c, job.id, Ok(())).unwrap();
            }
        };
        drain();
        let hits = |q: &str| search(&c, "o", q, &HashEmbedder, 10).unwrap().len();
        assert_eq!(hits("zebrafish"), 2, "seed must be searchable");
        assert_eq!(participant_meetings(&c, "LAN").unwrap(), 2);

        assert_eq!(purge_participant(&c, "lan").unwrap(), 2);

        let n = |sql: &str| count(&c, sql);
        assert_eq!(n("SELECT count(*) FROM segments WHERE text LIKE '%zebrafish%' OR words_json LIKE '%zebrafish%'"), 0, "transcript");
        assert_eq!(
            n("SELECT count(*) FROM speakers WHERE lower(trim(display_name))='lan'"),
            0
        );
        assert_eq!(
            n("SELECT count(*) FROM chunks_fts WHERE chunks_fts MATCH 'zebrafish'"),
            0,
            "FTS"
        );
        assert_eq!(
            n("SELECT count(*) FROM chunks WHERE text LIKE '%zebrafish%'"),
            0
        );
        assert_eq!(
            hits("zebrafish"),
            0,
            "Ask right after the purge, before any rebuild"
        );
        assert_eq!(
            n("SELECT count(*) FROM reports WHERE meeting_id IN ('m1','m2')"),
            0
        );
        assert_eq!(
            n("SELECT count(*) FROM action_items WHERE meeting_id = 'm1'"),
            0
        );
        assert_eq!(
            n("SELECT count(*) FROM segments"),
            3,
            "Minh's three lines stay"
        );
        assert_eq!(
            n("SELECT count(*) FROM reports WHERE meeting_id = 'm3'"),
            1,
            "untouched meeting"
        );
        assert_eq!(
            n("SELECT count(*) FROM action_items WHERE meeting_id = 'm3'"),
            1
        );
        assert_eq!(
            n("SELECT count(*) FROM jobs WHERE state='queued' AND kind='report'"),
            2
        );
        assert_eq!(
            n("SELECT count(*) FROM jobs WHERE state='queued' AND kind='index'"),
            2
        );
        assert_eq!(
            n("SELECT count(*) FROM meetings WHERE status='reporting'"),
            2
        );

        // The rebuild works from what is left and cannot bring the text back.
        drain();
        assert_eq!(
            n("SELECT count(*) FROM chunks_fts WHERE chunks_fts MATCH 'zebrafish'"),
            0
        );
        assert_eq!(hits("zebrafish"), 0);
        assert_eq!(hits("walrus"), 3, "Minh's text is searchable again");
        assert_eq!(participant_meetings(&c, "Lan").unwrap(), 0);
        assert_eq!(
            purge_participant(&c, "Lan").unwrap(),
            0,
            "repeat finds nothing"
        );
        assert!(purge_participant(&c, "  ").is_err());
    }

    #[test]
    fn a_meeting_left_without_speech_gets_no_rebuild_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        c.execute_batch(
            "INSERT INTO meetings (id, title, platform, status) VALUES ('m1','t','upload','ready');
             INSERT INTO speakers (id, meeting_id, label, display_name) VALUES ('s','m1','S1','Lan');
             INSERT INTO segments (meeting_id, speaker_id, start_ms, end_ms, text) VALUES ('m1','s',0,1,'zebrafish');",
        )
        .unwrap();
        assert_eq!(purge_participant(&c, "Lan").unwrap(), 1);
        assert_eq!(count(&c, "SELECT count(*) FROM segments"), 0);
        assert_eq!(count(&c, "SELECT count(*) FROM jobs"), 0);
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
