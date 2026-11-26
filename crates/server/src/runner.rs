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

/// Job kinds that call a paid provider (speech, language model, embeddings).
const PAID: [&str; 4] = [
    transcribe_job::KIND,
    report_job::KIND,
    index_job::KIND,
    document::KIND,
];

/// Estimated-cost settings the runner needs.
#[derive(Clone, Copy)]
pub struct Limits {
    pub speech_micro_usd_per_minute: u64,
    /// Default cap per workspace (a workspace may have its own); 0 means no cap.
    pub monthly_cap_micro_usd: u64,
    /// Cap on the sum over all workspaces; 0 means no cap.
    pub server_monthly_cap_micro_usd: u64,
}

/// Report jobs run only when a language model is configured; otherwise they stay queued.
fn run_pending(
    data_dir: &std::path::Path,
    tx: &dyn Transcriber,
    blobs: Option<&dyn Blobs>,
    report: Option<&LlmReportModel>,
    embedder: &dyn Embedder,
    limits: Limits,
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
    let now = db::now_ms();
    // Once per pass: past the server-wide cap, paid jobs wait in every workspace.
    let server_paused = limits.server_monthly_cap_micro_usd > 0
        && db::server_month_total(&control, data_dir, now)?
            >= limits.server_monthly_cap_micro_usd as i64;
    for tenant in db::tenant_ids(&control)? {
        let conn = db::open_tenant(data_dir, &tenant)?;
        // Past the monthly cap the jobs that cost money stay queued; archive and label still run.
        let cap = db::effective_cap(&control, &tenant, limits.monthly_cap_micro_usd)?;
        let paused = server_paused || recapz_core::usage::paused(&conn, cap, now)?;
        let kinds: Vec<&str> = kinds
            .iter()
            .copied()
            .filter(|k| !(paused && PAID.contains(k)))
            .collect();
        ran |= jobs::run_once(&conn, &kinds, |c, j| match (j.kind.as_str(), report) {
            (transcribe_job::KIND, r) => {
                transcribe_job::handle(c, j, data_dir, &tenant, tx, r.is_some())?;
                // An estimate must never fail a transcript that was stored (a retry would pay again).
                if let Some(mid) = j.meeting_id.as_deref() {
                    if let Err(e) = recapz_core::usage::record_speech(
                        c,
                        mid,
                        limits.speech_micro_usd_per_minute,
                        db::now_ms(),
                    ) {
                        tracing::warn!("speech usage not recorded: {e:#}");
                    }
                }
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

/// Deletes expired audio in every tenant; returns how many meetings were cleared.
fn retention_sweep(data_dir: &std::path::Path, blobs: Option<&dyn Blobs>) -> anyhow::Result<usize> {
    let control = db::open_control(data_dir)?;
    let mut n = 0;
    for tenant in db::tenant_ids(&control)? {
        let conn = db::open_tenant(data_dir, &tenant)?;
        n += recapz_core::retention::sweep(&conn, data_dir, &tenant, blobs, db::now_ms())?;
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
    limits: Limits,
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
        let mut last_sweep: Option<std::time::Instant> = None;
        loop {
            // Audio retention, about once an hour (and once at start).
            if last_sweep.is_none_or(|t| t.elapsed() >= Duration::from_secs(3600)) {
                last_sweep = Some(std::time::Instant::now());
                let (d, b) = (data_dir.clone(), blobs.clone());
                match tokio::task::spawn_blocking(move || retention_sweep(&d, b.as_deref())).await {
                    Ok(Ok(n)) if n > 0 => {
                        tracing::info!("retention: deleted audio of {n} meetings")
                    }
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => tracing::error!("retention sweep: {e:#}"),
                    Err(e) => tracing::error!("retention sweep panicked: {e}"),
                }
            }
            let (d, t, b, r, e) = (
                data_dir.clone(),
                tx.clone(),
                blobs.clone(),
                report.clone(),
                embedder.clone(),
            );
            match tokio::task::spawn_blocking(move || {
                run_pending(
                    &d,
                    t.as_ref(),
                    b.as_deref(),
                    r.as_deref(),
                    e.as_ref(),
                    limits,
                )
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

    const NO_LIMITS: Limits = Limits {
        speech_micro_usd_per_minute: 0,
        monthly_cap_micro_usd: 0,
        server_monthly_cap_micro_usd: 0,
    };

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
        assert!(run_pending(dir.path(), &StubTranscriber, None, None, &emb, NO_LIMITS).unwrap());
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
        assert!(run_pending(dir.path(), &StubTranscriber, None, None, &emb, NO_LIMITS).unwrap());
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
                &recapz_core::embed::HashEmbedder,
                NO_LIMITS
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
        assert!(!run_pending(dir.path(), &StubTranscriber, None, None, &emb, NO_LIMITS).unwrap());
        assert_eq!(jobs::state(&conn, id).unwrap(), Some(("queued".into(), 0)));
        assert!(run_pending(
            dir.path(),
            &StubTranscriber,
            Some(&Keep),
            None,
            &emb,
            NO_LIMITS
        )
        .unwrap());
        assert_eq!(jobs::state(&conn, id).unwrap().unwrap().0, "done");
        assert!(!wav.join("a.wav").exists());
    }

    #[test]
    fn at_the_monthly_cap_paid_jobs_stay_queued_but_archive_still_runs() {
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
        conn.execute(
            "INSERT INTO llm_usage (task, provider, model, cost_micro_usd, at) VALUES ('ask','p','m',5,?1)",
            [db::now_ms()],
        )
        .unwrap();
        let index = jobs::enqueue(&conn, index_job::KIND, Some("m1"), None).unwrap();
        let emb = recapz_core::embed::HashEmbedder;
        let cap = |n| Limits {
            speech_micro_usd_per_minute: 0,
            monthly_cap_micro_usd: n,
            server_monthly_cap_micro_usd: 0,
        };
        // spend (5) has reached the cap (5): the index job waits
        assert!(!run_pending(dir.path(), &StubTranscriber, None, None, &emb, cap(5)).unwrap());
        assert_eq!(
            jobs::state(&conn, index).unwrap(),
            Some(("queued".into(), 0))
        );
        // a recording can still be moved to storage while paused
        let archive = jobs::enqueue(&conn, archive_job::KIND, Some("m1"), None).unwrap();
        assert!(run_pending(
            dir.path(),
            &StubTranscriber,
            Some(&Keep),
            None,
            &emb,
            cap(5)
        )
        .unwrap());
        assert_eq!(jobs::state(&conn, archive).unwrap().unwrap().0, "done");
        assert_eq!(
            jobs::state(&conn, index).unwrap(),
            Some(("queued".into(), 0))
        );
        // raising the cap (or a new month) lets it run
        assert!(run_pending(dir.path(), &StubTranscriber, None, None, &emb, cap(6)).unwrap());
        assert_eq!(jobs::state(&conn, index).unwrap().unwrap().0, "done");
    }

    #[test]
    fn a_workspace_cap_pauses_only_its_workspace_and_the_server_cap_pauses_all() {
        let dir = tempfile::tempdir().unwrap();
        let control = db::open_control(dir.path()).unwrap();
        let ids: Vec<String> = ["A", "B"]
            .iter()
            .map(|n| db::create_workspace(&control, dir.path(), n, "x@x.com").unwrap())
            .collect();
        let queue = |id: &str| {
            let c = db::open_tenant(dir.path(), id).unwrap();
            c.execute(
                "INSERT INTO meetings (id, title, platform, status) VALUES ('m','t','upload','ready')",
                [],
            )
            .unwrap();
            let j = jobs::enqueue(&c, index_job::KIND, Some("m"), None).unwrap();
            c.execute(
                "INSERT INTO llm_usage (task, provider, model, cost_micro_usd, at) VALUES ('ask','p','m',5,?1)",
                [db::now_ms()],
            )
            .unwrap();
            (c, j)
        };
        let ((ca, ja), (cb, jb)) = (queue(&ids[0]), queue(&ids[1]));
        let emb = recapz_core::embed::HashEmbedder;
        let limits = |server| Limits {
            speech_micro_usd_per_minute: 0,
            monthly_cap_micro_usd: 0,
            server_monthly_cap_micro_usd: server,
        };
        let state = |c: &rusqlite::Connection, j| jobs::state(c, j).unwrap().unwrap().0;
        // only A has a cap (5, reached by its spend of 5): A waits, B runs
        db::set_workspace_cap(&control, &ids[0], 5).unwrap();
        run_pending(dir.path(), &StubTranscriber, None, None, &emb, limits(0)).unwrap();
        assert_eq!(
            (state(&ca, ja).as_str(), state(&cb, jb).as_str()),
            ("queued", "done")
        );
        // the server total (10) reached its cap: every workspace waits
        let (cb2, jb2) = {
            let j = jobs::enqueue(&cb, index_job::KIND, Some("m"), None).unwrap();
            (cb, j)
        };
        db::set_workspace_cap(&control, &ids[0], 0).unwrap();
        run_pending(dir.path(), &StubTranscriber, None, None, &emb, limits(10)).unwrap();
        assert_eq!(
            (state(&ca, ja).as_str(), state(&cb2, jb2).as_str()),
            ("queued", "queued")
        );
        // a higher server cap lets both run
        run_pending(dir.path(), &StubTranscriber, None, None, &emb, limits(11)).unwrap();
        assert_eq!(
            (state(&ca, ja).as_str(), state(&cb2, jb2).as_str()),
            ("done", "done")
        );
    }

    #[test]
    fn a_finished_transcribe_job_records_its_speech_cost_once_even_when_rerun() {
        let dir = tempfile::tempdir().unwrap();
        let control = db::open_control(dir.path()).unwrap();
        db::bootstrap_admin(&control, "boss@example.com").unwrap();
        let tenant = db::tenant_ids(&control).unwrap().remove(0);
        let conn = db::open_tenant(dir.path(), &tenant).unwrap();
        // 60 s of 16 kHz mono 16-bit silence
        let data = vec![0u8; 60 * 32_000];
        let mut w = Vec::new();
        w.extend(b"RIFF");
        w.extend((36 + data.len() as u32).to_le_bytes());
        w.extend(b"WAVEfmt ");
        for v in [16u32, 0x0001_0001, 16_000, 32_000, 0x0010_0002] {
            w.extend(v.to_le_bytes());
        }
        w.extend(b"data");
        w.extend((data.len() as u32).to_le_bytes());
        w.extend(data);
        let a = tenant_dir(dir.path(), &tenant).join("audio/m1");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::write(a.join("a.wav"), w).unwrap();
        conn.execute(
            "INSERT INTO meetings (id, title, platform, status, audio_path) VALUES ('m1','t','upload','uploaded','audio/m1/a.wav')",
            [],
        )
        .unwrap();
        let limits = Limits {
            speech_micro_usd_per_minute: 7_800,
            monthly_cap_micro_usd: 0,
            server_monthly_cap_micro_usd: 0,
        };
        let emb = recapz_core::embed::HashEmbedder;
        for _ in 0..2 {
            jobs::enqueue(&conn, transcribe_job::KIND, Some("m1"), None).unwrap();
            assert!(run_pending(dir.path(), &StubTranscriber, None, None, &emb, limits).unwrap());
        }
        let rows: Vec<(i64, i64)> = conn
            .prepare("SELECT audio_seconds, cost_micro_usd FROM speech_usage")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            rows,
            vec![(60, 7_800)],
            "one minute at the configured rate, one row"
        );
    }
}
