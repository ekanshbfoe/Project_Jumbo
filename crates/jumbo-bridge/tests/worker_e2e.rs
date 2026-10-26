//! End-to-end over the real Python worker on localhost: rendezvous → sealed URL → signed API →
//! chunked upload → echo job with streamed events → verified download → stop. Everything the Kaggle
//! path does except the cloudflared hop. Skips (with a message) if worker/.venv is not set up.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use jumbo_bridge::crypto::AppKeys;
use jumbo_bridge::protocol::{RendezvousSpec, SessionTicket};
use jumbo_bridge::rendezvous::RendezvousReader;
use jumbo_bridge::worker::{WorkerClient, hash_file};
use jumbo_bridge::{BridgeError, http};
use serde_json::json;

fn worker_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../worker")
}

fn venv_python() -> Option<PathBuf> {
    let dir = worker_dir().join(".venv");
    [dir.join("Scripts").join("python.exe"), dir.join("bin").join("python")].into_iter().find(|p| p.is_file())
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").expect("bind").local_addr().expect("addr").port()
}

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

fn spawn_worker(python: &Path, ticket: &Path, data: &Path, rv: &Path, port: u16) -> KillOnDrop {
    let child = Command::new(python)
        .current_dir(worker_dir())
        .args(["-m", "jumbo_worker", "--ticket"])
        .arg(ticket)
        .args(["--port", &port.to_string(), "--tunnel", "direct", "--data"])
        .arg(data)
        .arg("--rendezvous")
        .arg(format!("file:{}", rv.display()))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn worker");
    KillOnDrop(child)
}

#[tokio::test(flavor = "multi_thread")]
async fn round_trips_a_file_through_the_real_worker() {
    let Some(python) = venv_python() else {
        eprintln!("skipping: worker/.venv not found (python -m venv worker/.venv && pip install -e worker[dev])");
        return;
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let keys = AppKeys::generate().expect("keys");
    let ticket = SessionTicket::new(&keys, RendezvousSpec::Gist { gist_id: "unused-locally".into() }).expect("ticket");
    let ticket_path = tmp.path().join("ticket.json");
    std::fs::write(&ticket_path, serde_json::to_vec(&ticket).expect("json")).expect("write ticket");
    let rv = tmp.path().join("rv");
    let mut worker = spawn_worker(&python, &ticket_path, &tmp.path().join("data"), &rv, free_port());

    // Rendezvous: wait for the record, then open the sealed URL.
    let mut reader = RendezvousReader::Folder(rv);
    let record = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            if let Some(r) = reader.fetch(&ticket.session_id).await.expect("fetch") {
                return r;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("worker published a rendezvous record");
    let endpoint = record.open(&keys, &ticket.session_id).expect("open record");
    assert_eq!(endpoint.epoch, 1);
    let client = WorkerClient::new(http::client(), endpoint);
    assert!(client.health().await);
    assert_eq!(client.session().await.expect("session")["session_id"], ticket.session_id.as_str());
    let pool = client.pool().await.expect("pool");
    assert_eq!(pool["enabled"], true, "the worker runs a model pool: {pool}");
    assert!(pool["hbm_ceiling"].as_f64().is_some_and(|c| c > 0.5));

    // A request signed with the wrong key is refused.
    let mut forged = client.endpoint().clone();
    forged.key = [0u8; 32];
    let err = WorkerClient::new(http::client(), forged).session().await.expect_err("forged");
    assert!(matches!(err, BridgeError::Status { status: 401, .. }), "{err}");

    // 40 MB in three chunks.
    let input = tmp.path().join("chapter.bin");
    let data: Vec<u8> = (0..40_000_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
    std::fs::write(&input, &data).expect("write input");
    let mut chunks = 0;
    let hash = client.upload_file(&input, |_, _| chunks += 1).await.expect("upload");
    assert_eq!(chunks, 3);
    assert_eq!(hash, hash_file(&input).await.expect("hash"));
    // Uploading again is a no-op.
    let mut again = 0;
    client.upload_file(&input, |_, _| again += 1).await.expect("re-upload");
    assert_eq!(again, 1);

    let mut events = client.events().await.expect("events");
    assert_eq!(events.recv().await.expect("hello")["type"], "hello");
    let job_id = client.submit_job("echo", &hash, json!({ "step_s": 0.02 })).await.expect("submit");
    let mut progress = 0;
    let output = loop {
        let event = tokio::time::timeout(Duration::from_secs(30), events.recv()).await.expect("event").expect("open");
        match event["type"].as_str() {
            Some("job.progress") if event["job_id"] == job_id.as_str() => progress += 1,
            Some("job.done") if event["job_id"] == job_id.as_str() => {
                break event["output"].as_str().expect("output").to_owned();
            }
            Some("job.failed") => panic!("job failed: {event}"),
            _ => {}
        }
    };
    assert_eq!(progress, 5);
    assert_eq!(client.job(&job_id).await.expect("status").status, "done");

    let out = tmp.path().join("out.bin");
    client.download(&output, &out).await.expect("download");
    assert_eq!(std::fs::read(&out).expect("read"), data);

    client.stop().await.expect("stop");
    let exited = tokio::task::spawn_blocking(move || {
        for _ in 0..100 {
            if let Ok(Some(status)) = worker.0.try_wait() {
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    })
    .await
    .expect("join");
    assert!(exited.is_some(), "worker did not exit after stop");
}
