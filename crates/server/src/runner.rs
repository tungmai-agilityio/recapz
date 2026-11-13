use crate::report_model::LlmReportModel;
use recapz_core::{db, embed::Embedder, index_job, jobs, label_job, report_job, transcribe_job};
use recapz_speech::transcribe::Transcriber;
use std::{path::PathBuf, sync::Arc, time::Duration};

/// Loads the recognizer once. Without the `whisper` feature or a model file, transcribe jobs fail
/// with a clear error (and retry) instead of storing made-up text.
fn load_transcriber(model: &std::path::Path) -> Option<Box<dyn Transcriber>> {
    #[cfg(feature = "whisper")]
    if model.is_file() {
        return recapz_speech::transcribe::WhisperTranscriber::load(model)
            .map(|t| Box::new(t) as Box<dyn Transcriber>)
            .map_err(|e| tracing::error!("speech model: {e:#}"))
            .ok();
    }
    let _ = model;
    None
}

/// Report jobs run only when a language model is configured; otherwise they stay queued.
fn run_pending(
    data_dir: &std::path::Path,
    tx: Option<&dyn Transcriber>,
    report: Option<&LlmReportModel>,
    embedder: &dyn Embedder,
) -> anyhow::Result<bool> {
    let kinds: &[&str] = if report.is_some() {
        &[
            transcribe_job::KIND,
            label_job::KIND,
            index_job::KIND,
            report_job::KIND,
        ]
    } else {
        &[transcribe_job::KIND, label_job::KIND, index_job::KIND]
    };
    let control = db::open_control(data_dir)?;
    let mut ran = false;
    for tenant in db::tenant_ids(&control)? {
        let conn = db::open_tenant(data_dir, &tenant)?;
        ran |= jobs::run_once(&conn, kinds, |c, j| match (j.kind.as_str(), tx, report) {
            (transcribe_job::KIND, Some(tx), r) => {
                transcribe_job::handle(c, j, data_dir, &tenant, tx, r.is_some())
            }
            (transcribe_job::KIND, None, _) => anyhow::bail!("no speech model configured"),
            (label_job::KIND, _, _) => label_job::handle(c, j, data_dir, &tenant),
            (report_job::KIND, _, Some(m)) => report_job::handle(c, j, m),
            (index_job::KIND, _, _) => index_job::handle(c, j, embedder),
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
    model: PathBuf,
    report: Option<Arc<LlmReportModel>>,
    embedder: Arc<dyn Embedder>,
) {
    tokio::spawn(async move {
        let tx: Option<std::sync::Arc<dyn Transcriber>> = load_transcriber(&model).map(Into::into);
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
            let (d, t, r, e) = (
                data_dir.clone(),
                tx.clone(),
                report.clone(),
                embedder.clone(),
            );
            match tokio::task::spawn_blocking(move || {
                run_pending(&d, t.as_deref(), r.as_deref(), e.as_ref())
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
            !run_pending(dir.path(), None, None, &recapz_core::embed::HashEmbedder).unwrap(),
            "no job should run"
        );
        assert_eq!(
            jobs::state(&conn, id).unwrap(),
            Some(("queued".to_string(), 0))
        );
    }
}
