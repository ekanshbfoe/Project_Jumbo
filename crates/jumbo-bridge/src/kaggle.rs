//! Minimal Kaggle API client: push a script kernel, poll its session status and keep small private
//! datasets up to date. Mirrors the requests Kaggle's own SDK (kagglesdk `KernelsApiService`,
//! `DatasetApiService`, `BlobApiService`) sends.

use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{BridgeError, Result};

pub const KAGGLE_API: &str = "https://api.kaggle.com";

const KERNELS: &str = "kernels.KernelsApiService";
const DATASETS: &str = "datasets.DatasetApiService";
const BLOBS: &str = "blobs.BlobApiService";

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
    /// `owner/kernel-slug` of kernels whose latest output this one reads under /kaggle/input.
    pub kernel_sources: Vec<String>,
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
    /// Kaggle omits enum fields that hold their first value, so a missing status means QUEUED.
    #[serde(default = "queued")]
    pub status: String,
    #[serde(default)]
    pub failure_message: Option<String>,
}

fn queued() -> String {
    "QUEUED".to_owned()
}

fn not_yet_persisted() -> String {
    "NOT_YET_PERSISTED".to_owned()
}

impl KernelStatus {
    pub fn is_finished(&self) -> bool {
        matches!(self.status.as_str(), "COMPLETE" | "ERROR" | "CANCEL_ACKNOWLEDGED")
    }
}

/// A file the kernel's latest run wrote to /kaggle/working, at a pre-signed download URL.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputFile {
    pub file_name: String,
    pub url: String,
}

/// One file of a dataset version.
#[derive(Debug, Clone, PartialEq)]
pub struct DatasetFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// Kaggle's `DatabundleVersionStatus` names, e.g. READY, FAILED, BLOBS_RECEIVED.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct DatasetStatus {
    /// Omitted when it holds the enum's first value, NOT_YET_PERSISTED.
    #[serde(default = "not_yet_persisted")]
    pub status: String,
}

impl DatasetStatus {
    pub fn is_ready(&self) -> bool {
        self.status == "READY"
    }
}

fn split_slug(slug: &str) -> Result<(&str, &str)> {
    slug.split_once('/').ok_or_else(|| BridgeError::Protocol(format!("{slug:?} is not owner/slug")))
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

    async fn call(&self, service: &str, method: &str, body: Value) -> Result<Value> {
        let req = self.http.post(format!("{}/v1/{service}/{method}", self.api)).json(&body);
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
        // Kaggle's CLI sends the accelerator flag together with the machine shape.
        let shape = kernel.machine_shape.as_deref().unwrap_or_default();
        let body = json!({
            "slug": kernel.slug,
            "newTitle": kernel.title,
            "text": kernel.source,
            "language": "python",
            "kernelType": "script",
            "isPrivate": true,
            "enableGpu": shape.starts_with("Nvidia"),
            "enableTpu": shape.starts_with("Tpu"),
            // A pinned CPU image overrides the TPU VM environment (Kaggle/kaggle-cli#1197).
            "dockerImagePinningType": shape.starts_with("Tpu").then_some("latest"),
            "enableInternet": true,
            "datasetDataSources": kernel.dataset_sources,
            "kernelDataSources": kernel.kernel_sources,
            "competitionDataSources": [],
            "modelDataSources": kernel.model_sources,
            "categoryIds": [],
            "machineShape": kernel.machine_shape,
        });
        let value = self.call(KERNELS, "SaveKernel", body).await?;
        if let Some(error) = value["error"].as_str().filter(|e| !e.is_empty()) {
            return Err(BridgeError::Status { service: "Kaggle", status: 400, body: error.to_owned() });
        }
        Ok(serde_json::from_value(value)?)
    }

    pub async fn status(&self, slug: &str) -> Result<KernelStatus> {
        let (owner, kernel) = split_slug(slug)?;
        let value =
            self.call(KERNELS, "GetKernelSessionStatus", json!({ "userName": owner, "kernelSlug": kernel })).await?;
        Ok(serde_json::from_value(value)?)
    }

    /// The dataset's processing status, or `None` if it does not exist (or is not visible to us).
    pub async fn dataset_status(&self, slug: &str) -> Result<Option<DatasetStatus>> {
        let (owner, dataset) = split_slug(slug)?;
        match self.call(DATASETS, "GetDatasetStatus", json!({ "ownerSlug": owner, "datasetSlug": dataset })).await {
            Ok(value) => Ok(Some(serde_json::from_value(value)?)),
            Err(BridgeError::Status { status: 403 | 404, .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Uploads one file to Kaggle's blob store and returns the token a dataset version refers to it by.
    async fn upload_blob(&self, file: &DatasetFile) -> Result<String> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct StartUpload {
            token: String,
            create_url: String,
        }
        let start: StartUpload = serde_json::from_value(
            self.call(
                BLOBS,
                "StartBlobUpload",
                json!({ "type": "DATASET", "name": file.name, "contentLength": file.bytes.len() }),
            )
            .await?,
        )?;
        // A pre-signed upload URL: it carries its own authorization.
        let resp = crate::http::upload_client().put(&start.create_url).body(file.bytes.clone()).send().await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            return Err(BridgeError::Status {
                service: "Kaggle upload",
                status,
                body: resp.text().await.unwrap_or_default(),
            });
        }
        Ok(start.token)
    }

    /// Creates the private dataset `owner/slug` with exactly `files`, or replaces its contents with a
    /// new version that deletes every older one (so a rotated secret does not linger in history).
    pub async fn put_private_dataset(&self, slug: &str, title: &str, files: &[DatasetFile]) -> Result<()> {
        let (owner, dataset) = split_slug(slug)?;
        let mut tokens = Vec::with_capacity(files.len());
        for file in files {
            tokens.push(json!({ "token": self.upload_blob(file).await? }));
        }
        let value = if self.dataset_status(slug).await?.is_some() {
            let body = json!({ "versionNotes": "updated by Jumbo", "deleteOldVersions": true, "files": tokens });
            self.call(
                DATASETS,
                "CreateDatasetVersion",
                json!({ "ownerSlug": owner, "datasetSlug": dataset, "body": body }),
            )
            .await?
        } else {
            self.call(
                DATASETS,
                "CreateDataset",
                json!({
                    "ownerSlug": owner,
                    "slug": dataset,
                    "title": title,
                    "licenseName": "unknown",
                    "isPrivate": true,
                    "files": tokens,
                }),
            )
            .await?
        };
        if let Some(error) = value["error"].as_str().filter(|e| !e.is_empty()) {
            return Err(BridgeError::Status { service: "Kaggle", status: 400, body: error.to_owned() });
        }
        Ok(())
    }

    /// The kernel's latest run log (stdout and stderr), as plain text.
    pub async fn output_log(&self, slug: &str) -> Result<String> {
        let (owner, kernel) = split_slug(slug)?;
        let value = self
            .call(KERNELS, "ListKernelSessionOutput", json!({ "userName": owner, "kernelSlug": kernel, "pageSize": 1 }))
            .await?;
        Ok(plain_log(value["log"].as_str().unwrap_or_default()))
    }
}

impl KaggleClient {
    /// The files the kernel's latest run left in /kaggle/working.
    pub async fn output_files(&self, slug: &str) -> Result<Vec<OutputFile>> {
        let (owner, kernel) = split_slug(slug)?;
        let mut files = Vec::new();
        let mut page: Option<String> = None;
        loop {
            let mut body = json!({ "userName": owner, "kernelSlug": kernel, "pageSize": 100 });
            if let Some(token) = &page {
                body["pageToken"] = json!(token);
            }
            let value = self.call(KERNELS, "ListKernelSessionOutput", body).await?;
            files.extend(serde_json::from_value::<Vec<OutputFile>>(value["files"].clone()).unwrap_or_default());
            match value["nextPageToken"].as_str().filter(|t| !t.is_empty()) {
                Some(token) => page = Some(token.to_owned()),
                None => return Ok(files),
            }
        }
    }

    /// Streams one output file to `dest`, through `dest.partial` so a broken download never looks finished.
    pub async fn download_output(&self, file: &OutputFile, dest: &std::path::Path) -> Result<()> {
        use futures_util::StreamExt;
        use tokio::io::AsyncWriteExt;

        // The URL is pre-signed: it carries its own authorization.
        let resp = self.http.get(&file.url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            return Err(BridgeError::Status {
                service: "Kaggle download",
                status,
                body: resp.text().await.unwrap_or_default(),
            });
        }
        let partial = dest.with_extension("partial");
        let mut out = tokio::fs::File::create(&partial).await.map_err(|e| BridgeError::io(&partial, e))?;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            out.write_all(&chunk?).await.map_err(|e| BridgeError::io(&partial, e))?;
        }
        out.sync_all().await.map_err(|e| BridgeError::io(&partial, e))?;
        drop(out);
        tokio::fs::rename(&partial, dest).await.map_err(|e| BridgeError::io(dest, e))
    }
}

/// Kaggle returns run logs as a JSON array of `{"stream_name", "time", "data"}` records.
fn plain_log(raw: &str) -> String {
    #[derive(Deserialize)]
    struct Record {
        #[serde(default)]
        data: String,
    }
    match serde_json::from_str::<Vec<Record>>(raw) {
        Ok(records) => records.into_iter().map(|r| r.data).collect(),
        Err(_) => raw.to_owned(),
    }
}

/// The last `lines` lines of a log, for error messages.
pub fn log_tail(log: &str, lines: usize) -> String {
    let all: Vec<&str> = log.trim_end().lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_status_is_the_enums_first_value() {
        let kernel: KernelStatus = serde_json::from_str("{}").expect("parse");
        assert_eq!(kernel.status, "QUEUED");
        assert!(!kernel.is_finished());
        let dataset: DatasetStatus = serde_json::from_str("{}").expect("parse");
        assert!(!dataset.is_ready());
    }

    #[test]
    fn flattens_kaggle_log_records_and_keeps_the_tail() {
        let raw = r#"[{"stream_name":"stdout","time":0.1,"data":"one\n"},{"stream_name":"stderr","time":0.2,"data":"two\nthree\n"}]"#;
        let log = plain_log(raw);
        assert_eq!(log, "one\ntwo\nthree\n");
        assert_eq!(log_tail(&log, 2), "two\nthree");
        assert_eq!(plain_log("not json"), "not json");
    }
}
