use recapz_server::{config::Config, routes};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let cfg =
        Config::load(&std::env::var("RECAPZ_CONFIG").unwrap_or_else(|_| "config.toml".into()))?;
    let listener = tokio::net::TcpListener::bind(&cfg.bind).await?;
    tracing::info!("listening on {}", cfg.bind);
    axum::serve(listener, routes::router()).await?;
    Ok(())
}
