use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Clone, Default)]
#[serde(default)]
pub struct Google {
    pub client_id: String,
    pub client_secret: String,
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
