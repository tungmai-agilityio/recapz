use recapz_core::{db, embed::HashEmbedder, index_job, jobs, transcribe_job};
use recapz_speech::transcribe::Transcriber;
use std::{path::PathBuf, time::Duration};

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

fn run_pending(data_dir: &std::path::Path, tx: Option<&dyn Transcriber>) -> anyhow::Result<bool> {
    let control = db::open_control(data_dir)?;
    let mut ran = false;
    for tenant in db::tenant_ids(&control)? {
        let conn = db::open_tenant(data_dir, &tenant)?;
        ran |= jobs::run_once(&conn, KINDS, |c, j| match (j.kind.as_str(), tx) {
            (transcribe_job::KIND, Some(tx)) => transcribe_job::handle(c, j, data_dir, &tenant, tx),
            (transcribe_job::KIND, None) => anyhow::bail!("no speech model configured"),
            (index_job::KIND, _) => index_job::handle(c, j, &HashEmbedder),
            _ => anyhow::bail!("no handler for {}", j.kind),
        })?;
    }
    Ok(ran)
}

/// Polls every tenant's queue once a second, draining while work remains.
pub fn spawn(data_dir: PathBuf, model: PathBuf) {
    tokio::spawn(async move {
        let tx: Option<std::sync::Arc<dyn Transcriber>> = load_transcriber(&model).map(Into::into);
        loop {
            let (d, t) = (data_dir.clone(), tx.clone());
            match tokio::task::spawn_blocking(move || run_pending(&d, t.as_deref())).await {
                Ok(Ok(true)) => continue,
                Ok(Ok(false)) => {}
                Ok(Err(e)) => tracing::error!("job runner: {e:#}"),
                Err(e) => tracing::error!("job runner panicked: {e}"),
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}
