//! Delivers the worker's secrets to Kaggle without any manual step. Kaggle's API cannot attach Kaggle
//! Secrets to a kernel, so the app keeps them in a private dataset that every pushed kernel mounts
//! (`/kaggle/input/jumbo-secrets/`). Only the gist-scoped GitHub token goes there; the session key
//! never leaves the app.

use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::time::Instant;

use crate::error::{BridgeError, Result};
use crate::kaggle::{DatasetFile, KaggleClient};

pub const SECRETS_DATASET: &str = "jumbo-secrets";
pub const GIST_TOKEN_FILE: &str = "gist_token";

/// `owner/jumbo-secrets`, the dataset source a kernel lists to mount the secrets.
pub fn secrets_slug(owner: &str) -> String {
    format!("{owner}/{SECRETS_DATASET}")
}

/// Identifies what was last uploaded, so an unchanged token is not re-uploaded every session.
pub fn fingerprint(gist_token: &str) -> String {
    hex::encode(Sha256::digest(format!("{SECRETS_DATASET}\n{gist_token}")))
}

#[derive(Debug, Clone)]
pub struct SecretsTimings {
    /// How long Kaggle may take to process a new version before the kernel can mount it.
    pub ready: Duration,
    pub poll: Duration,
}

impl Default for SecretsTimings {
    fn default() -> Self {
        Self { ready: Duration::from_secs(180), poll: Duration::from_secs(3) }
    }
}

/// Makes sure `owner/jumbo-secrets` holds `gist_token` and is ready to mount. `uploaded` is the
/// fingerprint returned by the previous call; the upload is skipped when it still matches and the
/// dataset exists. Returns the fingerprint now on Kaggle.
pub async fn ensure_secrets(
    client: &KaggleClient,
    owner: &str,
    gist_token: &str,
    uploaded: Option<&str>,
    timings: &SecretsTimings,
) -> Result<String> {
    let slug = secrets_slug(owner);
    let wanted = fingerprint(gist_token);
    let status = client.dataset_status(&slug).await?;
    if status.as_ref().is_some_and(|s| s.is_ready()) && uploaded == Some(wanted.as_str()) {
        return Ok(wanted);
    }
    if status.is_none() || uploaded != Some(wanted.as_str()) {
        let file = DatasetFile { name: GIST_TOKEN_FILE.to_owned(), bytes: gist_token.as_bytes().to_vec() };
        client.put_private_dataset(&slug, "Jumbo secrets", &[file]).await?;
    }
    let deadline = Instant::now() + timings.ready;
    loop {
        match client.dataset_status(&slug).await? {
            Some(s) if s.is_ready() => return Ok(wanted),
            Some(s) if s.status == "FAILED" => {
                return Err(BridgeError::Protocol(format!("Kaggle could not process the {slug} dataset")));
            }
            _ if Instant::now() >= deadline => {
                return Err(BridgeError::Timeout(format!("{slug} was not ready within {:?}", timings.ready)));
            }
            _ => tokio::time::sleep(timings.poll).await,
        }
    }
}
