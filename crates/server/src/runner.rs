use recapz_core::{db, jobs};
use std::{path::PathBuf, time::Duration};

/// Job kinds this binary can run. Empty until the speech pipeline lands, so queued jobs wait.
pub const KINDS: &[&str] = &[];

fn run_pending(data_dir: &std::path::Path) -> anyhow::Result<bool> {
    let control = db::open_control(data_dir)?;
    let mut ran = false;
    for tenant in db::tenant_ids(&control)? {
        let conn = db::open_tenant(data_dir, &tenant)?;
        ran |= jobs::run_once(&conn, KINDS, |_, j| {
            anyhow::bail!("no handler for {}", j.kind)
        })?;
    }
    Ok(ran)
}

/// Polls every tenant's queue once a second, draining while work remains.
pub fn spawn(data_dir: PathBuf) {
    tokio::spawn(async move {
        loop {
            let d = data_dir.clone();
            match tokio::task::spawn_blocking(move || run_pending(&d)).await {
                Ok(Ok(true)) => continue,
                Ok(Ok(false)) => {}
                Ok(Err(e)) => tracing::error!("job runner: {e:#}"),
                Err(e) => tracing::error!("job runner panicked: {e}"),
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}
