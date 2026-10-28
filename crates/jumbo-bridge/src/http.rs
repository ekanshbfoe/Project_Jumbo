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
        // A half-open connection (a tunnel that dropped mid-request) fails instead of hanging forever. The timer
        // runs from sending until the answer starts, then between reads, so it must cover sending one 16 MiB
        // worker upload chunk on a slow line.
        .read_timeout(Duration::from_secs(300))
        .user_agent(concat!("jumbo-desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("static reqwest configuration is valid")
}

/// HTTP client for one large upload (a render job of hundreds of MB to a pre-signed URL), where sending alone can
/// take longer than `client`'s read timeout.
pub fn upload_client() -> reqwest::Client {
    init_tls();
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .user_agent(concat!("jumbo-desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("static reqwest configuration is valid")
}
