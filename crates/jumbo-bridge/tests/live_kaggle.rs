//! Live end-to-end test against real Kaggle, GitHub and Cloudflare. Ignored by default because it
//! spends Kaggle quota and needs credentials:
//!
//!   KAGGLE_USERNAME=… KAGGLE_API_TOKEN=KGAT_… JUMBO_GIST_TOKEN=… JUMBO_GIST_ID=… \
//!   [JUMBO_MACHINE_SHAPE=TpuV5E8] cargo test -p jumbo-bridge --test live_kaggle -- --ignored --nocapture
//!
//! It runs exactly what the desktop app's Launch does: refresh the secrets dataset, push the kernel,
//! follow it to Ready through the tunnel and the gist, round-trip 8 MiB through the echo job, stop.

use std::time::{Duration, Instant};

use jumbo_bridge::boot::{BootTimings, Launch, boot};
use jumbo_bridge::crypto::AppKeys;
use jumbo_bridge::http;
use jumbo_bridge::kaggle::{KaggleAuth, KaggleClient};
use jumbo_bridge::launcher::LaunchConfig;
use jumbo_bridge::protocol::{RendezvousSpec, SessionTicket, record_name};
use jumbo_bridge::rendezvous::{GistReader, RendezvousReader};
use jumbo_bridge::secrets::{SecretsTimings, ensure_secrets, secrets_slug};
use serde_json::json;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name} to run the live test"))
}

fn elapsed(start: Instant) -> String {
    format!("{:>6.1}s", start.elapsed().as_secs_f64())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "spends Kaggle quota; needs credentials (see the module docs)"]
async fn launch_to_ready_and_round_trip_through_kaggle() {
    let owner = env("KAGGLE_USERNAME");
    let gist_token = env("JUMBO_GIST_TOKEN");
    let gist_id = env("JUMBO_GIST_ID");
    let machine_shape = std::env::var("JUMBO_MACHINE_SHAPE").ok().filter(|s| !s.is_empty());
    let http = http::client();
    let kaggle = KaggleClient::new(http.clone(), KaggleAuth::Token(env("KAGGLE_API_TOKEN")));
    let start = Instant::now();

    ensure_secrets(&kaggle, &owner, &gist_token, None, &SecretsTimings::default()).await.expect("secrets dataset");
    println!("{} secrets dataset {} ready", elapsed(start), secrets_slug(&owner));

    let keys = AppKeys::generate().expect("keys");
    let ticket = SessionTicket::new(&keys, RendezvousSpec::Gist { gist_id: gist_id.clone() }).expect("ticket");
    let config = LaunchConfig {
        kernel_slug: format!("{owner}/jumbo-worker"),
        machine_shape: machine_shape.clone(),
        model_sources: vec![],
        dataset_sources: vec![secrets_slug(&owner)],
    };
    println!("{} session {} on {}", elapsed(start), ticket.session_id, machine_shape.as_deref().unwrap_or("CPU"));

    let mut reader = RendezvousReader::Gist(GistReader::new(http.clone(), &gist_id, &gist_token));
    let timings = BootTimings::default();
    let client =
        boot(&http, Launch::Kaggle { client: &kaggle, config: &config }, &mut reader, &keys, &ticket, &timings, |s| {
            println!("{} {}", elapsed(start), serde_json::to_string(&s).unwrap_or_default());
        })
        .await
        .unwrap_or_else(|e| panic!("boot failed: {e}"));
    println!("{} worker facts: {}", elapsed(start), client.endpoint().boot);

    // Same round trip as the app's "Run bridge test".
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("probe.bin");
    let data: Vec<u8> = (0..8 * 1024 * 1024u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
    std::fs::write(&input, &data).expect("write probe");
    let t = Instant::now();
    let hash = client.upload_file(&input, |_, _| {}).await.expect("upload");
    println!("{} uploaded 8 MiB in {:.1}s", elapsed(start), t.elapsed().as_secs_f64());
    let job_id = client.submit_job("echo", &hash, json!({ "step_s": 0.05 })).await.expect("submit");
    let output = loop {
        let job = client.job(&job_id).await.expect("job status");
        match job.status.as_str() {
            "done" => break job.output.expect("echo output"),
            "failed" => panic!("echo failed: {:?}", job.error),
            _ => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    };
    let out = dir.path().join("echo.bin");
    let t = Instant::now();
    client.download(&output, &out).await.expect("download");
    println!("{} downloaded in {:.1}s", elapsed(start), t.elapsed().as_secs_f64());
    assert_eq!(std::fs::read(&out).expect("read echo"), data, "echo must return the same bytes");

    // A signed request with a wrong key must be refused even though the URL is known.
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("clock").as_secs();
    let forged = jumbo_bridge::crypto::sign_request(&[7u8; 32], "GET", "/v1/session", now, b"");
    let resp = http
        .get(format!("{}/v1/session", client.endpoint().url))
        .header("authorization", format!("Jumbo {}.{now}.{forged}", ticket.session_id))
        .send()
        .await
        .expect("forged request");
    assert_eq!(resp.status().as_u16(), 401, "unsigned access must be refused");

    client.stop().await.expect("stop");
    println!("{} stop requested", elapsed(start));
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        let status = kaggle.status(&config.kernel_slug).await.expect("status");
        if status.is_finished() {
            println!("{} kernel finished: {}", elapsed(start), status.status);
            break;
        }
        assert!(Instant::now() < deadline, "kernel still {} five minutes after stop", status.status);
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
    // The worker removes its rendezvous record when it stops.
    let mut reader = RendezvousReader::Gist(GistReader::new(http.clone(), &gist_id, &gist_token));
    assert!(
        reader.fetch(&ticket.session_id).await.expect("gist").is_none(),
        "{} left behind",
        record_name(&ticket.session_id)
    );
}
