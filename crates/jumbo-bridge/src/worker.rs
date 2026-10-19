//! Signed client for one worker epoch: session control, resumable blob transfer, jobs and events.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use reqwest::Method;
use reqwest::header::{AUTHORIZATION, RANGE};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::crypto::auth_header;
use crate::error::{BridgeError, Result};
use crate::protocol::WorkerEndpoint;

/// Upload chunk size: well under the quick tunnel's 100 MB request limit and the worker's 32 MiB cap.
pub const CHUNK: usize = 16 * 1024 * 1024;
const PING_EVERY: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct JobStatus {
    pub id: String,
    pub kind: String,
    pub input: String,
    pub status: String,
    pub progress: f64,
    pub output: Option<String>,
    pub error: Option<String>,
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// blake3 of a file, streamed.
pub async fn hash_file(path: &Path) -> Result<String> {
    let mut file = tokio::fs::File::open(path).await.map_err(|e| BridgeError::io(path, e))?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf).await.map_err(|e| BridgeError::io(path, e))?;
        if n == 0 {
            return Ok(hasher.finalize().to_hex().to_string());
        }
        hasher.update(&buf[..n]);
    }
}

pub struct WorkerClient {
    http: reqwest::Client,
    endpoint: WorkerEndpoint,
}

impl WorkerClient {
    pub fn new(http: reqwest::Client, endpoint: WorkerEndpoint) -> Self {
        Self { http, endpoint }
    }

    pub fn endpoint(&self) -> &WorkerEndpoint {
        &self.endpoint
    }

    fn sign(&self, method: &Method, path_and_query: &str, body: &[u8]) -> String {
        auth_header(&self.endpoint.session_id, &self.endpoint.key, method.as_str(), path_and_query, unix_now(), body)
    }

    async fn send(&self, method: Method, path_and_query: &str, body: Vec<u8>) -> Result<reqwest::Response> {
        let auth = self.sign(&method, path_and_query, &body);
        let resp = self
            .http
            .request(method, format!("{}{path_and_query}", self.endpoint.url))
            .header(AUTHORIZATION, auth)
            .body(body)
            .send()
            .await?;
        if resp.status().is_success() {
            return Ok(resp);
        }
        Err(BridgeError::Status {
            service: "worker",
            status: resp.status().as_u16(),
            body: resp.text().await.unwrap_or_default(),
        })
    }

    async fn json<T: DeserializeOwned>(&self, method: Method, path_and_query: &str, body: Value) -> Result<T> {
        let bytes = if body.is_null() { Vec::new() } else { serde_json::to_vec(&body)? };
        Ok(self.send(method, path_and_query, bytes).await?.json().await?)
    }

    /// Unauthenticated liveness check through the tunnel.
    pub async fn health(&self) -> bool {
        match self.http.get(format!("{}/health", self.endpoint.url)).timeout(Duration::from_secs(10)).send().await {
            Ok(r) => r.json::<Value>().await.is_ok_and(|v| v["ok"] == true),
            Err(_) => false,
        }
    }

    pub async fn session(&self) -> Result<Value> {
        self.json(Method::GET, "/v1/session", Value::Null).await
    }

    /// Asks the worker to drain and exit.
    pub async fn stop(&self) -> Result<()> {
        self.json::<Value>(Method::POST, "/v1/session/stop", Value::Null).await.map(|_| ())
    }

    pub async fn missing(&self, hashes: &[String]) -> Result<Vec<String>> {
        let resp: Value = self.json(Method::POST, "/v1/blobs/missing", json!({ "hashes": hashes })).await?;
        Ok(serde_json::from_value(resp["missing"].clone())?)
    }

    /// Uploads a file unless the worker already has it, resuming any partial upload.
    /// `progress(sent, total)` is called after each chunk. Returns the blob's hash.
    pub async fn upload_file(&self, path: &Path, mut progress: impl FnMut(u64, u64)) -> Result<String> {
        let hash = hash_file(path).await?;
        let total = tokio::fs::metadata(path).await.map_err(|e| BridgeError::io(path, e))?.len();
        if self.missing(std::slice::from_ref(&hash)).await?.is_empty() {
            progress(total, total);
            return Ok(hash);
        }
        let status: Value = self.json(Method::GET, &format!("/v1/blobs/{hash}/status"), Value::Null).await?;
        let mut offset = status["received"].as_u64().unwrap_or(0);
        let mut file = tokio::fs::File::open(path).await.map_err(|e| BridgeError::io(path, e))?;
        file.seek(std::io::SeekFrom::Start(offset)).await.map_err(|e| BridgeError::io(path, e))?;
        let mut buf = vec![0u8; CHUNK];
        while offset < total {
            let mut filled = 0;
            while filled < CHUNK && offset + (filled as u64) < total {
                let n = file.read(&mut buf[filled..]).await.map_err(|e| BridgeError::io(path, e))?;
                if n == 0 {
                    break;
                }
                filled += n;
            }
            self.send(Method::PUT, &format!("/v1/blobs/{hash}?offset={offset}&total={total}"), buf[..filled].to_vec())
                .await?;
            offset += filled as u64;
            progress(offset, total);
        }
        Ok(hash)
    }

    /// Downloads a blob to `dest`, resuming from `dest.partial` and verifying the hash before the file
    /// appears at `dest`.
    pub async fn download(&self, hash: &str, dest: &Path) -> Result<()> {
        let partial = dest.with_extension("partial");
        let have = tokio::fs::metadata(&partial).await.map_or(0, |m| m.len());
        let path = format!("/v1/blobs/{hash}");
        let mut req = self
            .http
            .get(format!("{}{path}", self.endpoint.url))
            .header(AUTHORIZATION, self.sign(&Method::GET, &path, b""));
        if have > 0 {
            req = req.header(RANGE, format!("bytes={have}-"));
        }
        let resp = req.send().await?;
        let status = resp.status().as_u16();
        if !(status == 200 || status == 206) {
            return Err(BridgeError::Status { service: "worker", status, body: resp.text().await.unwrap_or_default() });
        }
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(status == 206)
            .write(true)
            .truncate(status == 200)
            .open(&partial)
            .await
            .map_err(|e| BridgeError::io(&partial, e))?;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            file.write_all(&chunk?).await.map_err(|e| BridgeError::io(&partial, e))?;
        }
        file.sync_all().await.map_err(|e| BridgeError::io(&partial, e))?;
        drop(file);
        if hash_file(&partial).await? != hash {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(BridgeError::Protocol(format!("downloaded blob {hash} failed verification")));
        }
        tokio::fs::rename(&partial, dest).await.map_err(|e| BridgeError::io(dest, e))
    }

    pub async fn submit_job(&self, kind: &str, input: &str, params: Value) -> Result<String> {
        let resp: Value =
            self.json(Method::POST, "/v1/jobs", json!({ "kind": kind, "input": input, "params": params })).await?;
        resp["job_id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| BridgeError::Protocol("job submit returned no id".into()))
    }

    pub async fn job(&self, job_id: &str) -> Result<JobStatus> {
        self.json(Method::GET, &format!("/v1/jobs/{job_id}"), Value::Null).await
    }

    /// Opens the signed event stream. Events arrive on the returned channel, which closes when the
    /// connection drops; the caller decides whether to reconnect (possibly to a new epoch).
    pub async fn events(&self) -> Result<mpsc::Receiver<Value>> {
        crate::http::init_tls();
        let ws_url = format!("{}/v1/events", self.endpoint.url.replacen("http", "ws", 1));
        let mut request =
            ws_url.into_client_request().map_err(|e| BridgeError::Protocol(format!("bad event stream URL: {e}")))?;
        let auth = self.sign(&Method::GET, "/v1/events", b"");
        request
            .headers_mut()
            .insert(AUTHORIZATION, auth.parse().map_err(|_| BridgeError::Protocol("unencodable auth header".into()))?);
        let (socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| BridgeError::Protocol(format!("event stream refused: {e}")))?;
        let (mut sink, mut stream) = socket.split();
        let (tx, rx) = mpsc::channel(256);
        tokio::spawn(async move {
            let mut ping = tokio::time::interval(PING_EVERY);
            ping.tick().await;
            loop {
                tokio::select! {
                    msg = stream.next() => match msg {
                        Some(Ok(Message::Text(text))) => {
                            if let Ok(event) = serde_json::from_str::<Value>(&text)
                                && tx.send(event).await.is_err()
                            {
                                break;
                            }
                        }
                        Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                        Some(Ok(_)) => {}
                    },
                    _ = ping.tick() => {
                        // Counts as activity for the worker's idle timer and keeps the tunnel warm.
                        if sink.send(Message::text(r#"{"type":"ping"}"#)).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Ok(rx)
    }
}
