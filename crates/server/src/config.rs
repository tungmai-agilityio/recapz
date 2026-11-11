use serde::Deserialize;

#[derive(Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    pub bind: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8080".into(),
        }
    }
}

impl Config {
    /// Reads a TOML file; a missing file means defaults.
    pub fn load(path: &str) -> anyhow::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => Ok(toml::from_str(&s)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }
}
