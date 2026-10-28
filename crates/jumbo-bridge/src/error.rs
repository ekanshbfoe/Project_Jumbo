use crate::crypto::CryptoError;

#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("{service} returned {status}: {}", readable_body(service, body))]
    Status { service: &'static str, status: u16, body: String },
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{path}: {source}")]
    Io { path: String, source: std::io::Error },
    #[error("protocol violation: {0}")]
    Protocol(String),
    #[error("timed out: {0}")]
    Timeout(String),
    /// The Kaggle kernel ended before or after the worker came up. `log` is the tail of its output.
    #[error("{message}{}", if log.is_empty() { String::new() } else { format!("\n--- Kaggle log (last lines) ---\n{log}") })]
    KernelFailed { message: String, log: String },
    /// A connected worker is gone and did not come back.
    #[error("session lost: {0}")]
    SessionLost(String),
}

/// The longest response body an error message quotes.
const MAX_BODY: usize = 300;

/// A response body fit for an error message. An HTML page (a Cloudflare error page when the tunnel is down, a
/// proxy's error page) becomes its title and a plain explanation instead of the whole page.
fn readable_body(service: &str, body: &str) -> String {
    let lower = body.to_ascii_lowercase();
    if lower.contains("<html") || lower.contains("<!doctype") {
        let title = lower
            .find("<title>")
            .and_then(|start| {
                let rest = &body[start + 7..];
                rest.to_ascii_lowercase()
                    .find("</title>")
                    .map(|end| rest[..end].split_whitespace().collect::<Vec<_>>().join(" "))
            })
            .filter(|t| !t.is_empty());
        let hint = if service == "worker" {
            "lost the connection to the Kaggle worker (its tunnel is down or moving)"
        } else {
            "an error page instead of an answer"
        };
        return match title {
            Some(title) => format!("{hint} [{title}]"),
            None => hint.to_owned(),
        };
    }
    let body = body.trim();
    match body.char_indices().nth(MAX_BODY) {
        Some((cut, _)) => format!("{}…", &body[..cut]),
        None => body.to_owned(),
    }
}

impl BridgeError {
    pub(crate) fn io(path: impl AsRef<std::path::Path>, source: std::io::Error) -> Self {
        Self::Io { path: path.as_ref().display().to_string(), source }
    }
}

pub type Result<T, E = BridgeError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_error_pages_become_a_short_message() {
        let page = "<!DOCTYPE html><html><head><title>Cloudflare Tunnel error | example.trycloudflare.com</title></head><body>...</body></html>";
        let e = BridgeError::Status { service: "worker", status: 530, body: page.into() };
        assert_eq!(
            e.to_string(),
            "worker returned 530: lost the connection to the Kaggle worker (its tunnel is down or moving) [Cloudflare Tunnel error | example.trycloudflare.com]"
        );
        let long = BridgeError::Status { service: "Kaggle", status: 500, body: "x".repeat(1000) };
        assert!(long.to_string().chars().count() < 340);
        let plain = BridgeError::Status { service: "Kaggle", status: 404, body: "not found".into() };
        assert_eq!(plain.to_string(), "Kaggle returned 404: not found");
    }
}
