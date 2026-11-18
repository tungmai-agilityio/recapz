use crate::report_model::LlmReportModel;
#[cfg(test)]
use recapz_core::db::tenant_dir;
use recapz_core::{
    archive_job, blobs::Blobs, db, document, embed::Embedder, index_job, jobs, label_job,
    report_job, transcribe_job,
};
#[cfg(test)]
use recapz_speech::transcribe::StubTranscriber;
use recapz_speech::transcribe::Transcriber;
use std::{path::PathBuf, sync::Arc, time::Duration};

/// Report jobs run only when a language model is configured; otherwise they stay queued.
fn run_pending(
    data_dir: &std::path::Path,
    tx: &dyn Transcriber,
    blobs: Option<&dyn Blobs>,
    report: Option<&LlmReportModel>,
    embedder: &dyn Embedder,
) -> anyhow::Result<bool> {
    let mut kinds = vec![
        transcribe_job::KIND,
        label_job::KIND,
        index_job::KIND,
        document::KIND,
    ];
    if report.is_some() {
        kinds.push(report_job::KIND);
    }
    if blobs.is_some() {
        kinds.push(archive_job::KIND);
    }
    let control = db::open_control(data_dir)?;
    let mut ran = false;
    for tenant in db::tenant_ids(&control)? {
        let conn = db::open_tenant(data_dir, &tenant)?;
        ran |= jobs::run_once(&conn, &kinds, |c, j| match (j.kind.as_str(), report) {
            (transcribe_job::KIND, r) => {
                transcribe_job::handle(c, j, data_dir, &tenant, tx, r.is_some())?;
                // Queued last so the recording moves only after everything that reads it was queued.
                if blobs.is_some() {
                    jobs::enqueue(c, archive_job::KIND, j.meeting_id.as_deref(), None)?;
                }
                Ok(())
            }
            (archive_job::KIND, _) => {
                archive_job::handle(c, j, data_dir, &tenant, blobs.expect("kind needs a store"))
            }
            (label_job::KIND, _) => label_job::handle(c, j, data_dir, &tenant),
            (report_job::KIND, Some(m)) => report_job::handle(c, j, m),
            (index_job::KIND, _) => index_job::handle(c, j, embedder),
            (document::KIND, _) => document::handle(c, data_dir, &tenant, j, embedder),
            _ => anyhow::bail!("no handler for {}", j.kind),
        })?;
    }
    Ok(ran)
}

/// After the configured embedder changed, queues an index job for every meeting still holding
/// vectors from the old one.
fn requeue_stale(data_dir: &std::path::Path, embedder_id: &str) -> anyhow::Result<usize> {
    let control = db::open_control(data_dir)?;
    let mut n = 0;
    for tenant in db::tenant_ids(&control)? {
        n += index_job::enqueue_stale(&db::open_tenant(data_dir, &tenant)?, embedder_id)?;
    }
    Ok(n)
}

/// Polls every tenant's queue once a second, draining while work remains.
pub fn spawn(
    data_dir: PathBuf,
    tx: Arc<dyn Transcriber>,
    blobs: Option<Arc<dyn Blobs>>,
    report: Option<Arc<LlmReportModel>>,
    embedder: Arc<dyn Embedder>,
) {
    tokio::spawn(async move {
        let (d, e) = (data_dir.clone(), embedder.clone());
        match tokio::task::spawn_blocking(move || requeue_stale(&d, e.id())).await {
            Ok(Ok(n)) if n > 0 => {
                tracing::info!("re-indexing {n} meetings for embedder {}", embedder.id())
            }
            Ok(Ok(_)) => {}
            Ok(Err(e)) => tracing::error!("re-index check: {e:#}"),
            Err(e) => tracing::error!("re-index check panicked: {e}"),
        }
        loop {
            let (d, t, b, r, e) = (
                data_dir.clone(),
                tx.clone(),
                blobs.clone(),
                report.clone(),
                embedder.clone(),
            );
            match tokio::task::spawn_blocking(move || {
                run_pending(&d, t.as_ref(), b.as_deref(), r.as_deref(), e.as_ref())
            })
            .await
            {
                Ok(Ok(true)) => continue,
                Ok(Ok(false)) => {}
                Ok(Err(e)) => tracing::error!("job runner: {e:#}"),
                Err(e) => tracing::error!("job runner panicked: {e}"),
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Counting(std::sync::atomic::AtomicUsize);
    impl Embedder for Counting {
        fn id(&self) -> &'static str {
            "counting"
        }
        fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            recapz_core::embed::HashEmbedder.embed(text)
        }
    }

    #[test]
    fn ingest_runs_and_only_a_toggled_on_document_reaches_the_embedder() {
        use std::sync::atomic::Ordering::SeqCst;
        let dir = tempfile::tempdir().unwrap();
        let control = db::open_control(dir.path()).unwrap();
        db::bootstrap_admin(&control, "boss@example.com").unwrap();
        let tenant = db::tenant_ids(&control).unwrap().remove(0);
        let conn = db::open_tenant(dir.path(), &tenant).unwrap();
        conn.execute(
            "INSERT INTO members (id, email, role) VALUES ('m1','boss@example.com','admin')",
            [],
        )
        .unwrap();
        let off = document::add(&conn, dir.path(), &tenant, "m1", "a.txt", b"alpha beta").unwrap();
        let emb = Counting(Default::default());
        assert!(run_pending(dir.path(), &StubTranscriber, None, None, &emb).unwrap());
        assert_eq!(emb.0.load(SeqCst), 0, "toggle off: nothing is embedded");
        let chunks = |id: &str| -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM document_chunks WHERE document_id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(chunks(&off), 1, "still keyword-searchable");
        assert!(document::set_llm_ok(&conn, "m1", &off, true).unwrap());
        assert!(run_pending(dir.path(), &StubTranscriber, None, None, &emb).unwrap());
        assert_eq!(emb.0.load(SeqCst), 1, "toggle on: one chunk embedded");
    }

    #[test]
    fn report_job_stays_queued_without_a_language_model() {
        let dir = tempfile::tempdir().unwrap();
        let control = db::open_control(dir.path()).unwrap();
        db::bootstrap_admin(&control, "boss@example.com").unwrap();
        let tenants = db::tenant_ids(&control).unwrap();
        assert_eq!(tenants.len(), 1);
        let conn = db::open_tenant(dir.path(), &tenants[0]).unwrap();
        let id = jobs::enqueue(&conn, report_job::KIND, Some("m1"), None).unwrap();
        assert!(
            !run_pending(
                dir.path(),
                &StubTranscriber,
                None,
                None,
                &recapz_core::embed::HashEmbedder
            )
            .unwrap(),
            "no job should run"
        );
        assert_eq!(
            jobs::state(&conn, id).unwrap(),
            Some(("queued".to_string(), 0))
        );
    }

    struct Keep;
    impl Blobs for Keep {
        fn put_file(&self, _: &str, _: &std::path::Path) -> anyhow::Result<()> {
            Ok(())
        }
        fn delete(&self, _: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn an_archive_job_waits_until_a_store_is_configured_then_runs_and_a_transcribe_queues_one() {
        let dir = tempfile::tempdir().unwrap();
        let control = db::open_control(dir.path()).unwrap();
        db::bootstrap_admin(&control, "boss@example.com").unwrap();
        let tenant = db::tenant_ids(&control).unwrap().remove(0);
        let conn = db::open_tenant(dir.path(), &tenant).unwrap();
        let wav = tenant_dir(dir.path(), &tenant).join("audio/m1");
        std::fs::create_dir_all(&wav).unwrap();
        std::fs::write(wav.join("a.wav"), b"RIFF").unwrap();
        conn.execute(
            "INSERT INTO meetings (id, title, platform, status, audio_path) VALUES ('m1','t','upload','ready','audio/m1/a.wav')",
            [],
        )
        .unwrap();
        let id = jobs::enqueue(&conn, archive_job::KIND, Some("m1"), None).unwrap();
        let emb = recapz_core::embed::HashEmbedder;
        assert!(!run_pending(dir.path(), &StubTranscriber, None, None, &emb).unwrap());
        assert_eq!(jobs::state(&conn, id).unwrap(), Some(("queued".into(), 0)));
        assert!(run_pending(dir.path(), &StubTranscriber, Some(&Keep), None, &emb).unwrap());
        assert_eq!(jobs::state(&conn, id).unwrap().unwrap().0, "done");
        assert!(!wav.join("a.wav").exists());
    }
}
