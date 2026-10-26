//! Session control for the Kaggle bridge: credentials, settings, boot, live events, the bridge
//! self-test and stop. Secrets live in the OS credential store (see `credentials`); settings in the
//! app config dir.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use jumbo_bridge::boot::{BootState, BootTimings, Launch, boot, reconnect};
use jumbo_bridge::crypto::AppKeys;
use jumbo_bridge::http;
use jumbo_bridge::kaggle::{KaggleAuth, KaggleClient};
use jumbo_bridge::launcher::LaunchConfig;
use jumbo_bridge::protocol::{RendezvousSpec, SessionTicket};
use jumbo_bridge::rendezvous::{GistReader, RendezvousReader};
use jumbo_bridge::secrets::{SecretsTimings, ensure_secrets, secrets_slug};
use jumbo_bridge::worker::WorkerClient;
use jumbo_core::fsx;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::credentials::{self, Keyring, Storage};

const KAGGLE_SECRET: &str = "kaggle";
const GITHUB_SECRET: &str = "github-gist";

type CmdResult<T> = Result<T, String>;

fn msg(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn config_dir(app: &AppHandle) -> CmdResult<PathBuf> {
    app.path().app_config_dir().map_err(msg)
}

fn secret(app: &AppHandle, name: &str) -> Option<String> {
    credentials::get(&Keyring, &config_dir(app).ok()?, name)
}

fn set_secret(app: &AppHandle, name: &str, value: &str) -> CmdResult<()> {
    credentials::set(&Keyring, &config_dir(app)?, name, value).map(drop).map_err(msg)
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BridgeMode {
    /// TPU v5e-8 on Kaggle: production.
    #[default]
    KaggleTpu,
    /// A CPU Kaggle machine: tests the whole cloud path without spending TPU quota.
    KaggleCpu,
    /// A worker started on this PC: tests the app and protocol without Kaggle or a tunnel.
    Local,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BridgeSettings {
    pub mode: BridgeMode,
    /// `owner/kernel-slug`
    pub kernel_slug: String,
    pub gist_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CredentialStatus {
    kaggle: bool,
    github: bool,
    storage: Storage,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum SessionView {
    Idle,
    Booting { boot: BootState, mode: BridgeMode, since_ms: u64 },
    Ready { epoch: u64, facts: Value, mode: BridgeMode, since_ms: u64 },
    Reconnecting { from_epoch: u64 },
    Failed { error: String },
    Stopped,
}

#[derive(Debug, Clone, Serialize)]
pub struct BridgeTestReport {
    bytes: u64,
    upload_ms: u128,
    job_ms: u128,
    download_ms: u128,
    verified: bool,
}

/// Live bridge session, shared by commands and the background boot/event tasks.
pub struct BridgeState {
    view: Mutex<SessionView>,
    client: tokio::sync::Mutex<Option<Arc<WorkerClient>>>,
    task: Mutex<Option<JoinHandle<()>>>,
    local_worker: Mutex<Option<Child>>,
}

impl Default for BridgeState {
    fn default() -> Self {
        Self {
            view: Mutex::new(SessionView::Idle),
            client: tokio::sync::Mutex::new(None),
            task: Mutex::new(None),
            local_worker: Mutex::new(None),
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn set_view(app: &AppHandle, view: SessionView) {
    if let Some(state) = app.try_state::<BridgeState>()
        && let Ok(mut current) = state.view.lock()
    {
        *current = view.clone();
    }
    let _ = app.emit("bridge://state", view);
}

fn settings_path(app: &AppHandle) -> CmdResult<PathBuf> {
    Ok(config_dir(app)?.join("bridge.json"))
}

fn load_settings(app: &AppHandle) -> CmdResult<BridgeSettings> {
    let path = settings_path(app)?;
    Ok(std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default())
}

#[tauri::command]
pub fn bridge_settings(app: AppHandle) -> CmdResult<BridgeSettings> {
    load_settings(&app)
}

#[tauri::command]
pub fn save_bridge_settings(app: AppHandle, settings: BridgeSettings) -> CmdResult<()> {
    let path = settings_path(&app)?;
    fsx::atomic_write_json(&path, &settings).map_err(msg)
}

#[tauri::command]
pub fn credential_status(app: AppHandle) -> CmdResult<CredentialStatus> {
    Ok(CredentialStatus {
        kaggle: secret(&app, KAGGLE_SECRET).is_some(),
        github: secret(&app, GITHUB_SECRET).is_some(),
        storage: credentials::storage(&config_dir(&app)?),
    })
}

/// Stores the username and key from a kaggle.json file in the OS credential store.
#[tauri::command]
pub fn import_kaggle_json(app: AppHandle, path: String) -> CmdResult<()> {
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
    KaggleAuth::from_kaggle_json(&text).map_err(|e| format!("not a valid kaggle.json: {e}"))?;
    set_secret(&app, KAGGLE_SECRET, &text)
}

/// Stores a Kaggle API token (the newer alternative to kaggle.json).
#[tauri::command]
pub fn set_kaggle_token(app: AppHandle, token: String) -> CmdResult<()> {
    let token = token.trim();
    if token.is_empty() {
        return Err("token is empty".into());
    }
    set_secret(&app, KAGGLE_SECRET, &json!({ "token": token }).to_string())
}

/// Stores the GitHub token (classic, `gist` scope only) used to read the rendezvous gist.
#[tauri::command]
pub fn set_github_token(app: AppHandle, token: String) -> CmdResult<()> {
    let token = token.trim();
    if token.is_empty() {
        return Err("token is empty".into());
    }
    set_secret(&app, GITHUB_SECRET, token)
}

fn kaggle_auth(app: &AppHandle) -> CmdResult<KaggleAuth> {
    let raw = secret(app, KAGGLE_SECRET).ok_or("add your Kaggle credentials first")?;
    let value: Value = serde_json::from_str(&raw).map_err(msg)?;
    match value["token"].as_str() {
        Some(token) => Ok(KaggleAuth::Token(token.to_owned())),
        None => KaggleAuth::from_kaggle_json(&raw).map_err(msg),
    }
}

/// What the pushed kernel needs from the private `jumbo-secrets` dataset.
struct KaggleSecrets {
    owner: String,
    gist_token: String,
}

/// Fingerprint of the secrets last uploaded, so an unchanged token is not re-uploaded each session.
fn secrets_fingerprint_path(app: &AppHandle) -> CmdResult<PathBuf> {
    Ok(config_dir(app)?.join("kaggle_secrets.json"))
}

async fn upload_secrets(app: &AppHandle, client: &KaggleClient, secrets: &KaggleSecrets) -> CmdResult<()> {
    let path = secrets_fingerprint_path(app)?;
    let uploaded: Option<String> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["fingerprint"].as_str().map(str::to_owned));
    let fingerprint =
        ensure_secrets(client, &secrets.owner, &secrets.gist_token, uploaded.as_deref(), &SecretsTimings::default())
            .await
            .map_err(msg)?;
    if uploaded.as_deref() != Some(fingerprint.as_str()) {
        fsx::atomic_write_json(&path, &json!({ "fingerprint": fingerprint })).map_err(msg)?;
    }
    Ok(())
}

#[tauri::command]
pub fn session_view(state: State<'_, BridgeState>) -> CmdResult<SessionView> {
    Ok(state.view.lock().map_err(msg)?.clone())
}

/// Development only: the worker checked out next to this app, run from its own venv.
fn local_worker_dir() -> PathBuf {
    std::env::var_os("JUMBO_WORKER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../worker"))
}

fn spawn_local_worker(ticket: &SessionTicket, rendezvous: &std::path::Path) -> CmdResult<Child> {
    let dir = local_worker_dir();
    let python = [dir.join(".venv/Scripts/python.exe"), dir.join(".venv/bin/python")]
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| format!("no worker venv in {} (python -m venv .venv && pip install -e .)", dir.display()))?;
    let home = rendezvous.parent().unwrap_or(rendezvous).to_path_buf();
    let ticket_path = home.join("ticket.json");
    std::fs::create_dir_all(&home).map_err(msg)?;
    std::fs::write(&ticket_path, serde_json::to_vec(ticket).map_err(msg)?).map_err(msg)?;
    let port = std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map_err(msg)?.port();
    // Kept next to the ticket so a worker that dies during boot can be diagnosed.
    let log = std::fs::File::create(home.join("worker.log")).map_err(msg)?;
    let log_err = log.try_clone().map_err(msg)?;
    Command::new(python)
        .current_dir(&dir)
        .args(["-m", "jumbo_worker", "--tunnel", "direct", "--port", &port.to_string(), "--ticket"])
        .arg(&ticket_path)
        .arg("--data")
        .arg(home.join("data"))
        .arg("--rendezvous")
        .arg(format!("file:{}", rendezvous.display()))
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err))
        .spawn()
        .map_err(|e| format!("could not start the local worker: {e}"))
}

/// Starts a session in the background; progress arrives as `bridge://state` events.
#[tauri::command]
pub async fn start_session(app: AppHandle, state: State<'_, BridgeState>) -> CmdResult<()> {
    if matches!(*state.view.lock().map_err(msg)?, SessionView::Booting { .. } | SessionView::Ready { .. }) {
        return Err("a session is already running".into());
    }
    let settings = load_settings(&app)?;
    let keys = AppKeys::generate().map_err(msg)?;
    let gist_id =
        if settings.mode == BridgeMode::Local { "local".to_owned() } else { settings.gist_id.trim().to_owned() };
    if gist_id.is_empty() {
        return Err("set the rendezvous gist id in Bridge settings".into());
    }
    let ticket = SessionTicket::new(&keys, RendezvousSpec::Gist { gist_id: gist_id.clone() }).map_err(msg)?;
    let http = http::client();

    let mut log_hint = String::new();
    let (reader, kaggle) = match settings.mode {
        BridgeMode::Local => {
            let rv = std::env::temp_dir().join(format!("jumbo-local-{}", ticket.session_id)).join("rv");
            let child = spawn_local_worker(&ticket, &rv)?;
            *state.local_worker.lock().map_err(msg)? = Some(child);
            if let Some(home) = rv.parent() {
                log_hint = format!(" (worker log: {})", home.join("worker.log").display());
            }
            (RendezvousReader::Folder(rv), None)
        }
        BridgeMode::KaggleTpu | BridgeMode::KaggleCpu => {
            if !settings.kernel_slug.contains('/') {
                return Err("set the Kaggle kernel as owner/kernel-slug in Bridge settings".into());
            }
            let token = secret(&app, GITHUB_SECRET).ok_or("add the GitHub gist token first")?;
            let kernel_slug = settings.kernel_slug.trim().to_owned();
            let owner = kernel_slug.split_once('/').map_or("", |(owner, _)| owner).to_owned();
            let config = LaunchConfig {
                machine_shape: (settings.mode == BridgeMode::KaggleTpu).then(|| "TpuV5E8".to_owned()),
                model_sources: vec![],
                // The worker reads its gist token from here (Kaggle's API cannot attach Kaggle Secrets).
                dataset_sources: vec![secrets_slug(&owner)],
                kernel_slug,
            };
            let reader = RendezvousReader::Gist(GistReader::new(http.clone(), &gist_id, &token));
            let client = KaggleClient::new(http.clone(), kaggle_auth(&app)?);
            (reader, Some((client, config, KaggleSecrets { owner, gist_token: token })))
        }
    };

    let mode = settings.mode;
    let since = now_ms();
    set_view(&app, SessionView::Booting { boot: BootState::Pushing, mode, since_ms: since });
    let task_app = app.clone();
    let task = tauri::async_runtime::spawn(async move {
        let app = task_app;
        let mut reader = reader;
        let timings = BootTimings::default();
        let on_state = |s: BootState| set_view(&app, SessionView::Booting { boot: s, mode, since_ms: since });
        if let Some((client, _, secrets)) = &kaggle
            && let Err(e) = upload_secrets(&app, client, secrets).await
        {
            return set_view(
                &app,
                SessionView::Failed { error: format!("could not prepare the secrets dataset: {e}") },
            );
        }
        let launch = match &kaggle {
            Some((client, config, _)) => Launch::Kaggle { client, config },
            None => Launch::External,
        };
        let mut client = match boot(&http, launch, &mut reader, &keys, &ticket, &timings, on_state).await {
            Ok(client) => client,
            Err(e) => return set_view(&app, SessionView::Failed { error: format!("{e}{log_hint}") }),
        };
        // Keep the session: forward worker events, and follow the worker to new epochs if the tunnel drops.
        loop {
            let endpoint = client.endpoint().clone();
            set_view(
                &app,
                SessionView::Ready { epoch: endpoint.epoch, facts: endpoint.boot.clone(), mode, since_ms: since },
            );
            let shared = Arc::new(client);
            if let Some(state) = app.try_state::<BridgeState>() {
                *state.client.lock().await = Some(shared.clone());
            }
            if let Ok(mut events) = shared.events().await {
                while let Some(event) = events.recv().await {
                    let stopped = event["type"] == "session.stopped";
                    let _ = app.emit("bridge://event", event);
                    if stopped {
                        return set_view(&app, SessionView::Stopped);
                    }
                }
            }
            let on_state = |s: BootState| {
                if let BootState::Reconnecting { from_epoch } = s {
                    set_view(&app, SessionView::Reconnecting { from_epoch });
                }
            };
            // Watching the kernel too means a finished session is reported at once, not after a timeout.
            let kernel = kaggle.as_ref().map(|(client, config, _)| (client, config.kernel_slug.as_str()));
            client =
                match reconnect(&http, &mut reader, &keys, &ticket, endpoint.epoch, kernel, &timings, on_state).await {
                    Ok(next) => next,
                    Err(e) => return set_view(&app, SessionView::Failed { error: e.to_string() }),
                };
        }
    });
    *state.task.lock().map_err(msg)? = Some(task);
    Ok(())
}

pub(crate) async fn current_client(state: &BridgeState) -> CmdResult<Arc<WorkerClient>> {
    state.client.lock().await.clone().ok_or_else(|| "no worker is connected".to_owned())
}

/// Round-trips 8 MiB through the worker (upload, echo job, download) and verifies it byte for byte.
#[tauri::command]
pub async fn run_bridge_test(state: State<'_, BridgeState>) -> CmdResult<BridgeTestReport> {
    let client = current_client(&state).await?;
    let dir = std::env::temp_dir().join("jumbo-bridge-test");
    std::fs::create_dir_all(&dir).map_err(msg)?;
    let input = dir.join("probe.bin");
    let mut seed = now_ms();
    let data: Vec<u8> = (0..8 * 1024 * 1024)
        .map(|_| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 56) as u8
        })
        .collect();
    std::fs::write(&input, &data).map_err(msg)?;

    let t = Instant::now();
    let hash = client.upload_file(&input, |_, _| {}).await.map_err(msg)?;
    let upload_ms = t.elapsed().as_millis();
    let t = Instant::now();
    let job_id = client.submit_job("echo", &hash, json!({ "step_s": 0.05 })).await.map_err(msg)?;
    let output = loop {
        let job = client.job(&job_id).await.map_err(msg)?;
        match job.status.as_str() {
            "done" => break job.output.ok_or("echo finished without output")?,
            "failed" => return Err(job.error.unwrap_or_else(|| "echo failed".into())),
            _ => tokio::time::sleep(std::time::Duration::from_millis(200)).await,
        }
    };
    let job_ms = t.elapsed().as_millis();
    let t = Instant::now();
    let out = dir.join("echo.bin");
    let _ = std::fs::remove_file(&out);
    client.download(&output, &out).await.map_err(msg)?;
    let download_ms = t.elapsed().as_millis();
    let verified = std::fs::read(&out).map_err(msg)? == data;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(BridgeTestReport { bytes: data.len() as u64, upload_ms, job_ms, download_ms, verified })
}

/// The worker's model pool snapshot; live updates follow as `pool.status` events.
#[tauri::command]
pub async fn pool_status(state: State<'_, BridgeState>) -> CmdResult<Value> {
    current_client(&state).await?.pool().await.map_err(msg)
}

/// Asks the worker to drain and exit (ending the Kaggle session), then resets the session.
#[tauri::command]
pub async fn stop_session(app: AppHandle, state: State<'_, BridgeState>) -> CmdResult<()> {
    let client = state.client.lock().await.take();
    let stopped = match client {
        Some(client) => client.stop().await.map_err(msg),
        None => Ok(()),
    };
    if let Some(task) = state.task.lock().map_err(msg)?.take() {
        task.abort();
    }
    if let Some(mut child) = state.local_worker.lock().map_err(msg)?.take() {
        // Give the local worker a moment to exit on its own after the stop request.
        std::thread::sleep(std::time::Duration::from_millis(500));
        let _ = child.kill();
    }
    set_view(&app, SessionView::Stopped);
    stopped
}
