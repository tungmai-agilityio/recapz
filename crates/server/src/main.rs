use recapz_core::db;
use recapz_server::{config::Config, report_model::LlmReportModel, routes, runner};
use recapz_speech::deepgram::DeepgramTranscriber;
use std::sync::{Arc, Mutex};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    recapz_server::install_crypto_provider();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let cfg =
        Config::load(&std::env::var("RECAPZ_CONFIG").unwrap_or_else(|_| "config.toml".into()))?;
    if let Some(result) = recapz_server::cli::run(std::env::args().nth(1).as_deref(), &cfg) {
        return result;
    }
    cfg.require_speech_key()?;
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

    let store = recapz_server::storage::S3Store::from_config(
        &cfg.storage,
        tokio::runtime::Handle::current(),
    )?
    .map(Arc::new);
    if store.is_none() {
        tracing::warn!("no [storage] bucket configured: recordings stay on local disk");
    }
    let embedder = recapz_server::embedder::from_config(&cfg.embedder, &cfg.llm)?; // fail fast on a bad name
    let mail = recapz_server::mail::SmtpMailer::from_config(&cfg.mail)?.map(|m| {
        (
            Arc::new(m) as Arc<dyn recapz_server::mail::Mailer>,
            cfg.base_url.clone(),
        )
    });
    if mail.is_none() {
        tracing::warn!("no [mail] host configured: report and invite emails are off");
    }
    runner::spawn(
        cfg.data_dir.clone(),
        Arc::new(DeepgramTranscriber::new(
            &cfg.speech.base_url,
            &cfg.speech.api_key,
        )?),
        store.map(|s| s as Arc<dyn recapz_core::blobs::Blobs>),
        report,
        embedder,
        runner::Limits {
            speech_micro_usd_per_minute: cfg.speech.price_per_minute_micro_usd,
            monthly_cap_micro_usd: cfg.monthly_cap_micro_usd(),
            server_monthly_cap_micro_usd: cfg.server_monthly_cap_micro_usd(),
        },
        mail,
    );
    if cfg.google.client_id.is_empty() || cfg.secret_key.is_empty() {
        tracing::warn!("calendar sync is off: needs google.client_id and RECAPZ_SECRET_KEY");
    } else if recapz_core::google_tokens::SecretKey::from_hex(&cfg.secret_key).is_err() {
        anyhow::bail!("RECAPZ_SECRET_KEY must be 64 hex characters");
    } else {
        recapz_server::calendar_sync::spawn(
            cfg.data_dir.clone(),
            cfg.google.clone(),
            cfg.secret_key.clone(),
        );
    }
    recapz_server::notetaker_loop::startup(&cfg.data_dir)?;
    match recapz_notetaker::browser::find_chromium(
        Some(cfg.notetaker.chromium_path.as_str())
            .filter(|p| !p.is_empty())
            .map(std::path::Path::new),
    ) {
        Some(chromium) => {
            recapz_server::notetaker_loop::spawn(recapz_server::notetaker_loop::Settings {
                data_dir: cfg.data_dir.clone(),
                chromium,
                max_concurrent: cfg.max_concurrent_calls,
                max_call_minutes: cfg.max_call_minutes,
                live: Some(recapz_server::notetaker::Live {
                    base_url: cfg.speech.listen_url(),
                    api_key: cfg.speech.api_key.clone(),
                }),
            })
        }
        None => {
            tracing::warn!("the notetaker is off: Chromium not found (set notetaker.chromium_path)")
        }
    }
    let listener = tokio::net::TcpListener::bind(&cfg.bind).await?;
    tracing::info!("listening on {}", cfg.bind);
    axum::serve(listener, routes::router(cfg, Arc::new(Mutex::new(control)))).await?;
    Ok(())
}
