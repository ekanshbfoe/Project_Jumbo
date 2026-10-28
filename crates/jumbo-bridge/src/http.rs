//! One place to build HTTP clients, so every client uses the same TLS setup.

use std::sync::Once;
use std::time::Duration;

static TLS: Once = Once::new();

/// Installs the ring crypto provider for rustls (once per process).
pub fn init_tls() {
    TLS.call_once(|| {
        // Another component may have installed a provider already; either is fine.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// HTTP client for the GitHub, Kaggle and worker APIs.
pub fn client() -> reqwest::Client {
    init_tls();
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        // A half-open connection (a tunnel that dropped mid-request) fails after a minute without data instead of
        // hanging forever; it is the time between reads, so long uploads and downloads are not cut short.
        .read_timeout(Duration::from_secs(60))
        .user_agent(concat!("jumbo-desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("static reqwest configuration is valid")
}
