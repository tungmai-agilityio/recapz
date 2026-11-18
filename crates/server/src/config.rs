use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Clone)]
#[serde(default)]
pub struct Google {
    pub client_id: String,
    pub client_secret: String,
    /// Endpoint overrides, so tests can point the OAuth calls at a local mock.
    pub auth_url: String,
    pub token_url: String,
    pub userinfo_url: String,
    pub revoke_url: String,
    pub events_url: String,
}

impl Default for Google {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            client_secret: String::new(),
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            userinfo_url: "https://openidconnect.googleapis.com/v1/userinfo".into(),
            revoke_url: "https://oauth2.googleapis.com/revoke".into(),
            events_url: "https://www.googleapis.com/calendar/v3/calendars/primary/events".into(),
        }
    }
}

/// One language-model route used for every report task. Empty `provider` turns reports off.
#[derive(Deserialize, Clone, Default)]
#[serde(default)]
pub struct Llm {
    /// `anthropic` or `openai` (any OpenAI-compatible server).
    pub provider: String,
    /// Required for `openai`; optional override for `anthropic`.
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// Used when `embedder = "hosted"`: model for `POST {base}/embeddings`.
    pub embedding_model: String,
    /// Endpoint for embeddings; empty uses `base_url`. The `api_key` above is reused.
    pub embedding_base_url: String,
    pub max_tokens: u32,
    /// Micro-USD per million tokens.
    pub price_input: u64,
    pub price_output: u64,
}

/// Settings for the browser notetaker.
#[derive(Deserialize, Clone, Default)]
#[serde(default)]
pub struct Notetaker {
    /// Chromium or Chrome binary; empty searches the usual locations.
    pub chromium_path: String,
}

#[derive(Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    pub data_dir: PathBuf,
    pub base_url: String,
    pub bind: String,
    /// Creates the first workspace and invites this email as admin.
    pub bootstrap_admin_email: String,
    /// Enables POST /auth/dev-login (local development only).
    pub dev_login: bool,
    pub google: Google,
    pub llm: Llm,
    pub notetaker: Notetaker,
    /// ggml speech model file used by the transcribe job (needs the `whisper` build feature).
    pub speech_model: PathBuf,
    /// Questions one member may ask per minute (each one is a paid model call).
    pub ask_per_minute: u32,
    /// Text embedder for Ask search: `hash` (built in) or `hosted` (see `[llm] embedding_model`). Changing it re-indexes meetings at start.
    pub embedder: String,
    /// Calls the notetaker may be in at once (each runs a browser next to the speech model).
    pub max_concurrent_calls: usize,
    /// Hard cap on one call; the notetaker leaves and transcribes what it has.
    pub max_call_minutes: u32,
    /// 64 hex characters that encrypt stored Google refresh tokens. Never read from the config
    /// file: `Config::load` takes it from `RECAPZ_SECRET_KEY`. Empty turns calendar connect off.
    #[serde(skip)]
    pub secret_key: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: "./data".into(),
            base_url: "http://localhost:8080".into(),
            bind: "127.0.0.1:8080".into(),
            bootstrap_admin_email: String::new(),
            dev_login: false,
            google: Google::default(),
            llm: Llm::default(),
            notetaker: Notetaker::default(),
            speech_model: "./models/ggml-large-v3-turbo.bin".into(),
            ask_per_minute: 20,
            embedder: "hash".into(),
            max_concurrent_calls: 2,
            max_call_minutes: 240,
            secret_key: String::new(),
        }
    }
}

impl Config {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let mut cfg = match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(e.into()),
        };
        cfg.secret_key = std::env::var("RECAPZ_SECRET_KEY").unwrap_or_default();
        Ok(cfg)
    }
}
