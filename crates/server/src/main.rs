use recapz_core::db;
use recapz_server::{config::Config, report_model::LlmReportModel, routes, runner};
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
    for tenant in db::tenant_ids(&control)? {
        db::open_tenant(&cfg.data_dir, &tenant)?; // run migrations on startup
    }
    let report =
        LlmReportModel::from_config(&cfg.llm, tokio::runtime::Handle::current())?.map(Arc::new);
    if report.is_none() {
        tracing::warn!(
            "no [llm] provider configured: meetings stop after transcription, no reports"
        );
    }

    let embedder = recapz_server::embedder::from_config(&cfg.embedder)?; // fail fast on a bad name
    runner::spawn(
        cfg.data_dir.clone(),
        cfg.speech_model.clone(),
        report,
        embedder,
    );
    let listener = tokio::net::TcpListener::bind(&cfg.bind).await?;
    tracing::info!("listening on {}", cfg.bind);
    axum::serve(listener, routes::router(cfg, Arc::new(Mutex::new(control)))).await?;
    Ok(())
}
