//! Picks the text embedder named in config. Ask and the index job must use the same one.
use recapz_core::embed::{Embedder, HashEmbedder};
use std::sync::Arc;

/// `hash` is the built-in word-overlap baseline. A real multilingual model goes here as another
/// arm (behind a cargo feature if it pulls a runtime), with its own `Embedder::id`.
pub fn from_config(name: &str) -> anyhow::Result<Arc<dyn Embedder>> {
    match name {
        "hash" => Ok(Arc::new(HashEmbedder)),
        other => anyhow::bail!("unknown embedder {other:?}; this build has: hash"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_name_loads_and_unknown_name_is_an_error() {
        assert_eq!(from_config("hash").unwrap().id(), "hash-256");
        let e = from_config("minilm").err().unwrap().to_string();
        assert_eq!(e, "unknown embedder \"minilm\"; this build has: hash");
    }
}
