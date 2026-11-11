use recapz_core::db;
use recapz_server::{config::Config, routes, runner};
use std::sync::{Arc, Mutex};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let cfg =
        Config::load(&std::env::var("RECAPZ_CONFIG").unwrap_or_else(|_| "config.toml".into()))?;

    let control = db::open_control(&cfg.data_dir)?;
    if !cfg.bootstrap_admin_email.is_empty() {
        db::bootstrap_admin(&control, &cfg.bootstrap_admin_email)?;
    }
    for t in db::tenant_ids(&control)? {
        db::open_tenant(&cfg.data_dir, &t)?; // run migrations on startup
    }
    let control = Arc::new(Mutex::new(control));
    runner::spawn(cfg.data_dir.clone(), control.clone());

    let listener = tokio::net::TcpListener::bind(&cfg.bind).await?;
    tracing::info!("listening on {}", cfg.bind);
    axum::serve(listener, routes::router(cfg, control)).await?;
    Ok(())
}
