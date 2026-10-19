use crate::crypto::CryptoError;

#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("{service} returned {status}: {body}")]
    Status { service: &'static str, status: u16, body: String },
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{path}: {source}")]
    Io { path: String, source: std::io::Error },
    #[error("protocol violation: {0}")]
    Protocol(String),
    #[error("timed out: {0}")]
    Timeout(String),
}

impl BridgeError {
    pub(crate) fn io(path: impl AsRef<std::path::Path>, source: std::io::Error) -> Self {
        Self::Io { path: path.as_ref().display().to_string(), source }
    }
}

pub type Result<T, E = BridgeError> = std::result::Result<T, E>;
