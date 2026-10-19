//! Minimal Kaggle API client: push a script kernel and poll its session status. Mirrors the requests
//! Kaggle's own SDK (kagglesdk `KernelsApiService`) sends.

use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{BridgeError, Result};

pub const KAGGLE_API: &str = "https://api.kaggle.com";

#[derive(Clone)]
pub enum KaggleAuth {
    /// `KAGGLE_API_TOKEN` style bearer token.
    Token(String),
    /// Username and key from `kaggle.json`.
    Key { username: String, key: String },
}

impl std::fmt::Debug for KaggleAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Token(_) => f.write_str("KaggleAuth::Token(***)"),
            Self::Key { username, .. } => write!(f, "KaggleAuth::Key({username}, ***)"),
        }
    }
}

impl KaggleAuth {
    /// Parses the contents of `kaggle.json`.
    pub fn from_kaggle_json(text: &str) -> Result<Self> {
        #[derive(Deserialize)]
        struct KaggleJson {
            username: String,
            key: String,
        }
        let k: KaggleJson = serde_json::from_str(text)?;
        Ok(Self::Key { username: k.username, key: k.key })
    }
}

/// A script kernel to (re)run on Kaggle.
#[derive(Debug, Clone, PartialEq)]
pub struct KernelPush {
    /// `owner/kernel-slug`
    pub slug: String,
    pub title: String,
    pub source: String,
    /// e.g. `TpuV5E8`
    pub machine_shape: Option<String>,
    pub dataset_sources: Vec<String>,
    pub model_sources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushResult {
    #[serde(default, rename = "ref")]
    pub reference: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub version_number: Option<u64>,
}

/// Kaggle's `KernelWorkerStatus` names: QUEUED, RUNNING, COMPLETE, ERROR, CANCEL_REQUESTED,
/// CANCEL_ACKNOWLEDGED, NEW_SCRIPT.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KernelStatus {
    pub status: String,
    #[serde(default)]
    pub failure_message: Option<String>,
}

impl KernelStatus {
    pub fn is_finished(&self) -> bool {
        matches!(self.status.as_str(), "COMPLETE" | "ERROR" | "CANCEL_ACKNOWLEDGED")
    }
}

pub struct KaggleClient {
    http: reqwest::Client,
    api: String,
    auth: KaggleAuth,
}

impl KaggleClient {
    pub fn new(http: reqwest::Client, auth: KaggleAuth) -> Self {
        Self::with_api(http, KAGGLE_API, auth)
    }

    pub fn with_api(http: reqwest::Client, api: &str, auth: KaggleAuth) -> Self {
        Self { http, api: api.trim_end_matches('/').to_owned(), auth }
    }

    async fn call(&self, method: &str, body: Value) -> Result<Value> {
        let req = self.http.post(format!("{}/v1/kernels.KernelsApiService/{method}", self.api)).json(&body);
        let req = match &self.auth {
            KaggleAuth::Token(token) => req.header(AUTHORIZATION, format!("Bearer {token}")),
            KaggleAuth::Key { username, key } => req.basic_auth(username, Some(key)),
        };
        let resp = req.send().await?;
        let status = resp.status().as_u16();
        let text = resp.text().await?;
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        // Kaggle reports some errors as 200 with a {code, message} body.
        let code = value["code"].as_u64().unwrap_or(0);
        if status >= 400 || code >= 400 {
            let message = value["message"].as_str().map_or(text, str::to_owned);
            return Err(BridgeError::Status { service: "Kaggle", status: status.max(code as u16), body: message });
        }
        Ok(value)
    }

    /// Creates or updates the kernel and queues a run of the new version.
    pub async fn push(&self, kernel: &KernelPush) -> Result<PushResult> {
        let body = json!({
            "slug": kernel.slug,
            "newTitle": kernel.title,
            "text": kernel.source,
            "language": "python",
            "kernelType": "script",
            "isPrivate": true,
            "enableGpu": false,
            "enableInternet": true,
            "datasetDataSources": kernel.dataset_sources,
            "kernelDataSources": [],
            "competitionDataSources": [],
            "modelDataSources": kernel.model_sources,
            "categoryIds": [],
            "machineShape": kernel.machine_shape,
        });
        let value = self.call("SaveKernel", body).await?;
        if let Some(error) = value["error"].as_str().filter(|e| !e.is_empty()) {
            return Err(BridgeError::Status { service: "Kaggle", status: 400, body: error.to_owned() });
        }
        Ok(serde_json::from_value(value)?)
    }

    pub async fn status(&self, slug: &str) -> Result<KernelStatus> {
        let (owner, kernel) = slug
            .split_once('/')
            .ok_or_else(|| BridgeError::Protocol(format!("kernel slug {slug:?} is not owner/slug")))?;
        let value = self.call("GetKernelSessionStatus", json!({ "userName": owner, "kernelSlug": kernel })).await?;
        Ok(serde_json::from_value(value)?)
    }
}
