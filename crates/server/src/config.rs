use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Clone, Default)]
#[serde(default)]
pub struct Google {
    pub client_id: String,
    pub client_secret: String,
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
    pub max_tokens: u32,
    /// Micro-USD per million tokens.
    pub price_input: u64,
    pub price_output: u64,
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
    /// ggml speech model file used by the transcribe job (needs the `whisper` build feature).
    pub speech_model: PathBuf,
    /// Questions one member may ask per minute (each one is a paid model call).
    pub ask_per_minute: u32,
    /// Text embedder for Ask search: `hash` (built in). Changing it re-indexes meetings at start.
    pub embedder: String,
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
            speech_model: "./models/ggml-large-v3-turbo.bin".into(),
            ask_per_minute: 20,
            embedder: "hash".into(),
        }
    }
}

impl Config {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => Ok(toml::from_str(&s)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }
}
