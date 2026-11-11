use recapz_core::{db, jobs};
use std::{path::PathBuf, sync::Arc, time::Duration};

/// Job kinds this binary can run. Empty until the speech pipeline lands, so queued jobs wait.
pub const KINDS: &[&str] = &[];

pub fn spawn(data_dir: PathBuf, control: Arc<std::sync::Mutex<rusqlite::Connection>>) {
    tokio::spawn(async move {
        loop {
            let (d, c) = (data_dir.clone(), control.clone());
            let res = tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
                let tenants = db::tenant_ids(&c.lock().unwrap())?;
                let mut ran = false;
                for t in tenants {
                    let conn = db::open_tenant(&d, &t)?;
                    ran |= jobs::run_once(&conn, KINDS, |_, j| {
                        anyhow::bail!("no handler for {}", j.kind)
                    })?;
                }
                Ok(ran)
            })
            .await;
            match res {
                Ok(Ok(true)) => continue,
                Ok(Ok(false)) => {}
                Ok(Err(e)) => tracing::error!("job runner: {e:#}"),
                Err(e) => tracing::error!("job runner panicked: {e}"),
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}
