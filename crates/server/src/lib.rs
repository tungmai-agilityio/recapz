pub mod calendar_sync;
pub mod cli;
pub mod config;
pub mod dev_call;
pub mod embedder;
pub mod google_calendar;
pub mod live;
pub mod notetaker;
pub mod notetaker_loop;
pub mod report_model;
pub mod routes;
pub mod runner;
pub mod storage;

/// Select the TLS crypto provider once per process. The dependency tree enables both `ring` and
/// `aws-lc-rs`, so rustls cannot choose and panics on the first secure connection (the speech
/// WebSocket) unless one is installed.
pub fn install_crypto_provider() {
    // Err only means a provider is already installed, which is fine.
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[cfg(test)]
mod crypto_provider_tests {
    #[test]
    fn tls_client_config_builds_after_install() {
        super::install_crypto_provider();
        super::install_crypto_provider(); // twice is harmless
        let cfg = rustls::ClientConfig::builder()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
        assert!(cfg.alpn_protocols.is_empty());
    }
}
