//! The one place an HTTP client is started.
//!
//! reqwest 0.13's `rustls-no-provider` build panics at client construction
//! unless a rustls crypto provider is installed first. The provider here is
//! ring (the `rustls` dependency enables only that feature), because reqwest's
//! own `rustls` feature would bring `aws-lc-sys`, a C/cmake build on every
//! target. Every client goes through [`builder`] or [`client`] so the install
//! cannot be forgotten.

use std::sync::Once;

fn ensure_provider() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // Err means another provider is already installed, which is fine.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// `reqwest::Client::builder()` with the TLS provider installed.
pub fn builder() -> reqwest::ClientBuilder {
    ensure_provider();
    reqwest::Client::builder()
}

/// `reqwest::Client::new()` with the TLS provider installed.
pub fn client() -> reqwest::Client {
    ensure_provider();
    reqwest::Client::new()
}
