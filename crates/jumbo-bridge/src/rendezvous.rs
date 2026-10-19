//! Reading the worker's rendezvous record: a private GitHub gist in production, a folder locally.

use std::path::PathBuf;

use reqwest::StatusCode;
use reqwest::header::{ACCEPT, AUTHORIZATION, ETAG, IF_NONE_MATCH, USER_AGENT};
use serde_json::Value;

use crate::error::{BridgeError, Result};
use crate::protocol::{RendezvousRecord, record_name};

pub const GITHUB_API: &str = "https://api.github.com";

/// Polls one gist. Conditional requests (ETag) keep frequent polling within GitHub's rate limit:
/// unchanged answers come back as 304 and do not count against it.
pub struct GistReader {
    client: reqwest::Client,
    url: String,
    token: String,
    etag: Option<String>,
    files: Value,
}

impl GistReader {
    pub fn new(client: reqwest::Client, gist_id: &str, token: &str) -> Self {
        Self::with_api(client, GITHUB_API, gist_id, token)
    }

    pub fn with_api(client: reqwest::Client, api: &str, gist_id: &str, token: &str) -> Self {
        Self {
            client,
            url: format!("{}/gists/{gist_id}", api.trim_end_matches('/')),
            token: token.to_owned(),
            etag: None,
            files: Value::Null,
        }
    }

    async fn refresh(&mut self) -> Result<()> {
        let mut req = self
            .client
            .get(&self.url)
            .header(AUTHORIZATION, format!("Bearer {}", self.token))
            .header(ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header(USER_AGENT, "jumbo-desktop");
        if let Some(etag) = &self.etag {
            req = req.header(IF_NONE_MATCH, etag);
        }
        let resp = req.send().await?;
        match resp.status() {
            StatusCode::NOT_MODIFIED => Ok(()),
            s if s.is_success() => {
                self.etag = resp.headers().get(ETAG).and_then(|v| v.to_str().ok()).map(str::to_owned);
                let gist: Value = resp.json().await?;
                self.files = gist["files"].clone();
                Ok(())
            }
            s => Err(BridgeError::Status {
                service: "GitHub",
                status: s.as_u16(),
                body: resp.text().await.unwrap_or_default(),
            }),
        }
    }

    pub async fn fetch(&mut self, session_id: &str) -> Result<Option<RendezvousRecord>> {
        self.refresh().await?;
        let file = &self.files[record_name(session_id)];
        if file.is_null() {
            return Ok(None);
        }
        // Large files are truncated in the gist listing; records are tiny, but follow raw_url if needed.
        let content = if file["truncated"].as_bool() == Some(true) {
            let raw = file["raw_url"].as_str().ok_or_else(|| BridgeError::Protocol("truncated gist file".into()))?;
            self.client.get(raw).header(USER_AGENT, "jumbo-desktop").send().await?.text().await?
        } else {
            file["content"].as_str().unwrap_or_default().to_owned()
        };
        Ok(Some(serde_json::from_str(&content)?))
    }
}

pub enum RendezvousReader {
    Gist(GistReader),
    /// Development: the worker was started with `--rendezvous file:<dir>`.
    Folder(PathBuf),
}

impl RendezvousReader {
    pub async fn fetch(&mut self, session_id: &str) -> Result<Option<RendezvousRecord>> {
        match self {
            Self::Gist(gist) => gist.fetch(session_id).await,
            Self::Folder(dir) => {
                let path = dir.join(record_name(session_id));
                match tokio::fs::read_to_string(&path).await {
                    Ok(text) => Ok(Some(serde_json::from_str(&text)?)),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(e) => Err(BridgeError::io(&path, e)),
                }
            }
        }
    }
}
