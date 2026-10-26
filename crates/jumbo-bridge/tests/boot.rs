//! Boot sequence against a mocked Kaggle API and the real Python worker on localhost.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use httpmock::prelude::*;
use jumbo_bridge::boot::{BootState, BootTimings, Launch, boot, reconnect};
use jumbo_bridge::crypto::AppKeys;
use jumbo_bridge::http;
use jumbo_bridge::kaggle::{KaggleAuth, KaggleClient};
use jumbo_bridge::launcher::LaunchConfig;
use jumbo_bridge::protocol::{RendezvousSpec, SessionTicket};
use jumbo_bridge::rendezvous::RendezvousReader;
use serde_json::json;

fn venv_python() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../worker/.venv");
    [dir.join("Scripts").join("python.exe"), dir.join("bin").join("python")].into_iter().find(|p| p.is_file())
}

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

fn spawn_worker(python: &Path, ticket: &SessionTicket, dir: &Path) -> KillOnDrop {
    let ticket_path = dir.join("ticket.json");
    std::fs::write(&ticket_path, serde_json::to_vec(ticket).expect("json")).expect("write");
    let port = std::net::TcpListener::bind("127.0.0.1:0").expect("bind").local_addr().expect("addr").port();
    KillOnDrop(
        Command::new(python)
            .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../worker"))
            .args(["-m", "jumbo_worker", "--tunnel", "direct", "--port", &port.to_string(), "--ticket"])
            .arg(&ticket_path)
            .arg("--data")
            .arg(dir.join("data"))
            .arg("--rendezvous")
            .arg(format!("file:{}", dir.join("rv").display()))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn"),
    )
}

fn fast() -> BootTimings {
    BootTimings {
        queue: Duration::from_secs(5),
        rendezvous: Duration::from_secs(60),
        warm: Duration::from_secs(10),
        reconnect: Duration::from_secs(10),
        retry_backoff: Duration::from_millis(50),
        poll_status: Duration::from_millis(200),
        poll_rendezvous: Duration::from_millis(200),
    }
}

fn config() -> LaunchConfig {
    LaunchConfig {
        kernel_slug: "alice/jumbo-worker".into(),
        machine_shape: Some("TpuV5E8".into()),
        model_sources: vec![],
        dataset_sources: vec![],
    }
}

fn names(states: &[BootState]) -> Vec<&'static str> {
    states
        .iter()
        .map(|s| match s {
            BootState::Pushing => "pushing",
            BootState::Queued => "queued",
            BootState::Running => "running",
            BootState::AwaitingRendezvous => "awaiting_rendezvous",
            BootState::Connecting { .. } => "connecting",
            BootState::Authenticating { .. } => "authenticating",
            BootState::Warming { .. } => "warming",
            BootState::Ready { .. } => "ready",
            BootState::Reconnecting { .. } => "reconnecting",
            BootState::Retrying { .. } => "retrying",
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn boots_through_kaggle_states_to_a_ready_worker_and_reconnects() {
    let Some(python) = venv_python() else {
        eprintln!("skipping: worker/.venv not found");
        return;
    };
    let kaggle_api = MockServer::start_async().await;
    let push = kaggle_api
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/kernels.KernelsApiService/SaveKernel")
                .json_body_includes(r#"{"machineShape":"TpuV5E8"}"#);
            then.status(200).json_body(json!({"versionNumber": 1}));
        })
        .await;
    kaggle_api
        .mock_async(|when, then| {
            when.method(POST).path("/v1/kernels.KernelsApiService/GetKernelSessionStatus");
            then.status(200).json_body(json!({"status": "RUNNING"}));
        })
        .await;
    let kaggle = KaggleClient::with_api(http::client(), &kaggle_api.base_url(), KaggleAuth::Token("t".into()));

    let keys = AppKeys::generate().expect("keys");
    let ticket = SessionTicket::new(&keys, RendezvousSpec::Gist { gist_id: "g".into() }).expect("ticket");
    let tmp = tempfile::tempdir().expect("tempdir");
    // Stands in for the kernel Kaggle would start.
    let _worker = spawn_worker(&python, &ticket, tmp.path());
    let mut reader = RendezvousReader::Folder(tmp.path().join("rv"));
    let http = http::client();
    let config = config();

    let mut states = Vec::new();
    let client =
        boot(&http, Launch::Kaggle { client: &kaggle, config: &config }, &mut reader, &keys, &ticket, &fast(), |s| {
            states.push(s)
        })
        .await
        .expect("boot");
    push.assert_async().await;
    assert_eq!(
        names(&states),
        ["pushing", "queued", "running", "awaiting_rendezvous", "connecting", "authenticating", "ready"]
    );
    let BootState::Ready { epoch, boot } = states.last().expect("ready").clone() else { panic!("not ready") };
    assert_eq!(epoch, 1);
    assert!(boot["cpus"].as_u64().unwrap_or(0) > 0, "boot facts: {boot}");

    // The same epoch answering again counts as a successful reconnect.
    let mut states = Vec::new();
    let again = reconnect(&http, &mut reader, &keys, &ticket, 1, Some((&kaggle, "alice/jumbo-worker")), &fast(), |s| {
        states.push(s)
    })
    .await
    .expect("reconnect");
    assert_eq!(names(&states), ["reconnecting", "connecting", "authenticating", "ready"]);
    again.stop().await.expect("stop");
    drop(client);
}

#[tokio::test]
async fn retries_a_crashed_boot_once_then_reports_kaggles_message_and_log() {
    let kaggle_api = MockServer::start_async().await;
    let push = kaggle_api
        .mock_async(|when, then| {
            when.method(POST).path("/v1/kernels.KernelsApiService/SaveKernel");
            then.status(200).json_body(json!({}));
        })
        .await;
    kaggle_api
        .mock_async(|when, then| {
            when.method(POST).path("/v1/kernels.KernelsApiService/GetKernelSessionStatus");
            then.status(200).json_body(json!({"status": "ERROR", "failureMessage": "No JUMBO_GIST_TOKEN"}));
        })
        .await;
    let output = kaggle_api
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/kernels.KernelsApiService/ListKernelSessionOutput")
                .json_body_includes(r#"{"userName":"alice","kernelSlug":"jumbo-worker"}"#);
            then.status(200).json_body(json!({
                "files": [],
                "log": r#"[{"stream_name":"stdout","time":1.0,"data":"jumbo: session x\n"},{"stream_name":"stderr","time":2.0,"data":"SystemExit: No JUMBO_GIST_TOKEN\n"}]"#,
            }));
        })
        .await;
    let kaggle = KaggleClient::with_api(http::client(), &kaggle_api.base_url(), KaggleAuth::Token("t".into()));
    let keys = AppKeys::generate().expect("keys");
    let ticket = SessionTicket::new(&keys, RendezvousSpec::Gist { gist_id: "g".into() }).expect("ticket");
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut reader = RendezvousReader::Folder(tmp.path().to_path_buf());
    let config = config();
    let mut states = Vec::new();
    let err = boot(
        &http::client(),
        Launch::Kaggle { client: &kaggle, config: &config },
        &mut reader,
        &keys,
        &ticket,
        &fast(),
        |s| states.push(s),
    )
    .await
    .err()
    .expect("boot fails");
    assert_eq!(names(&states), ["pushing", "queued", "retrying", "pushing", "queued"]);
    push.assert_calls_async(2).await;
    output.assert_calls_async(1).await;
    let text = err.to_string();
    assert!(text.contains("No JUMBO_GIST_TOKEN") && text.contains("SystemExit"), "{text}");
}

#[tokio::test]
async fn reconnect_gives_up_at_once_when_the_kernel_has_ended() {
    let kaggle_api = MockServer::start_async().await;
    kaggle_api
        .mock_async(|when, then| {
            when.method(POST).path("/v1/kernels.KernelsApiService/GetKernelSessionStatus");
            then.status(200).json_body(json!({"status": "COMPLETE"}));
        })
        .await;
    kaggle_api
        .mock_async(|when, then| {
            when.method(POST).path("/v1/kernels.KernelsApiService/ListKernelSessionOutput");
            then.status(200).json_body(json!({"log": "worker: deadline reached"}));
        })
        .await;
    let kaggle = KaggleClient::with_api(http::client(), &kaggle_api.base_url(), KaggleAuth::Token("t".into()));
    let keys = AppKeys::generate().expect("keys");
    let ticket = SessionTicket::new(&keys, RendezvousSpec::Gist { gist_id: "g".into() }).expect("ticket");
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut reader = RendezvousReader::Folder(tmp.path().to_path_buf());
    let started = std::time::Instant::now();
    let err = reconnect(
        &http::client(),
        &mut reader,
        &keys,
        &ticket,
        1,
        Some((&kaggle, "alice/jumbo-worker")),
        &fast(),
        |_| {},
    )
    .await
    .err()
    .expect("lost");
    assert!(matches!(err, jumbo_bridge::BridgeError::SessionLost(_)), "{err}");
    assert!(err.to_string().contains("deadline reached"), "{err}");
    assert!(started.elapsed() < fast().reconnect);
}

#[tokio::test]
async fn times_out_when_the_kernel_never_leaves_the_queue() {
    let kaggle_api = MockServer::start_async().await;
    kaggle_api
        .mock_async(|when, then| {
            when.method(POST).path("/v1/kernels.KernelsApiService/SaveKernel");
            then.status(200).json_body(json!({}));
        })
        .await;
    kaggle_api
        .mock_async(|when, then| {
            when.method(POST).path("/v1/kernels.KernelsApiService/GetKernelSessionStatus");
            then.status(200).json_body(json!({"status": "QUEUED"}));
        })
        .await;
    let kaggle = KaggleClient::with_api(http::client(), &kaggle_api.base_url(), KaggleAuth::Token("t".into()));
    let keys = AppKeys::generate().expect("keys");
    let ticket = SessionTicket::new(&keys, RendezvousSpec::Gist { gist_id: "g".into() }).expect("ticket");
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut reader = RendezvousReader::Folder(tmp.path().to_path_buf());
    let timings = BootTimings { queue: Duration::from_millis(600), ..fast() };
    let config = config();
    let err = boot(
        &http::client(),
        Launch::Kaggle { client: &kaggle, config: &config },
        &mut reader,
        &keys,
        &ticket,
        &timings,
        |_| {},
    )
    .await
    .err()
    .expect("times out");
    assert!(matches!(err, jumbo_bridge::BridgeError::Timeout(_)), "{err}");
}
