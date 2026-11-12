use recapz_core::{db, embed::Embedder, index_job, jobs, transcribe_job};
use recapz_speech::transcribe::Transcriber;
use std::{path::PathBuf, sync::Arc, time::Duration};

/// Job kinds this binary can run; other kinds stay queued.
pub const KINDS: &[&str] = &[transcribe_job::KIND, index_job::KIND];

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

fn run_pending(
    data_dir: &std::path::Path,
    tx: Option<&dyn Transcriber>,
    embedder: &dyn Embedder,
) -> anyhow::Result<bool> {
    let control = db::open_control(data_dir)?;
    let mut ran = false;
    for tenant in db::tenant_ids(&control)? {
        let conn = db::open_tenant(data_dir, &tenant)?;
        ran |= jobs::run_once(&conn, KINDS, |c, j| match (j.kind.as_str(), tx) {
            (transcribe_job::KIND, Some(tx)) => transcribe_job::handle(c, j, data_dir, &tenant, tx),
            (transcribe_job::KIND, None) => anyhow::bail!("no speech model configured"),
            (index_job::KIND, _) => index_job::handle(c, j, embedder),
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
pub fn spawn(data_dir: PathBuf, model: PathBuf, embedder: Arc<dyn Embedder>) {
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
            let (d, t, e) = (data_dir.clone(), tx.clone(), embedder.clone());
            match tokio::task::spawn_blocking(move || run_pending(&d, t.as_deref(), e.as_ref()))
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
