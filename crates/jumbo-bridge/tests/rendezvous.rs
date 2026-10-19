use std::path::PathBuf;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use httpmock::prelude::*;
use jumbo_bridge::crypto::AppKeys;
use jumbo_bridge::http;
use jumbo_bridge::protocol::{RendezvousRecord, record_name};
use jumbo_bridge::rendezvous::{GistReader, RendezvousReader};
use serde_json::{Value, json};

fn vectors() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../protocol/bridge-v1.vectors.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("read")).expect("parse")
}

fn app_keys(v: &Value) -> AppKeys {
    AppKeys::from_secret_bytes(B64.decode(v["app_sk"].as_str().expect("sk")).expect("b64").try_into().expect("32"))
}

/// A record exactly as the worker would publish it, built from the shared vectors.
fn record(v: &Value, epoch: u64) -> RendezvousRecord {
    RendezvousRecord {
        proto: 1,
        session_id: v["session_id"].as_str().expect("sid").into(),
        epoch,
        worker_pk: v["worker_pk"].as_str().expect("pk").into(),
        sealed: v["sealed"]["box"].as_str().expect("box").into(),
        ts: 1,
    }
}

#[test]
fn opening_a_record_yields_url_and_session_key() {
    let v = vectors();
    let sid = v["session_id"].as_str().expect("sid");
    let endpoint = record(&v, 2).open(&app_keys(&v), sid).expect("open");
    assert_eq!(endpoint.url, "https://example.trycloudflare.com");
    assert_eq!(endpoint.epoch, 2);
    assert_eq!(hex::encode(endpoint.key), v["session_key_hex"].as_str().expect("hex"));
}

#[test]
fn rejects_records_for_other_sessions_or_keys() {
    let v = vectors();
    let sid = v["session_id"].as_str().expect("sid");
    assert!(record(&v, 1).open(&app_keys(&v), "ffffffffffffffffffffffffffffffff").is_err());
    assert!(record(&v, 1).open(&AppKeys::generate().expect("keys"), sid).is_err());
}

#[tokio::test]
async fn gist_reader_uses_etags_and_finds_the_session_file() {
    let v = vectors();
    let sid = v["session_id"].as_str().expect("sid").to_owned();
    let server = MockServer::start_async().await;
    let body =
        json!({ "files": { record_name(&sid): { "content": serde_json::to_string(&record(&v, 4)).expect("json") } } });
    let unchanged = server
        .mock_async(|when, then| {
            when.method(GET).path("/gists/g1").header("if-none-match", "\"v1\"");
            then.status(304);
        })
        .await;
    let first = server
        .mock_async(|when, then| {
            when.method(GET).path("/gists/g1").header("authorization", "Bearer tok");
            then.status(200).header("etag", "\"v1\"").json_body(body.clone());
        })
        .await;

    let mut reader = GistReader::with_api(http::client(), &server.base_url(), "g1", "tok");
    assert_eq!(reader.fetch(&sid).await.expect("fetch").map(|r| r.epoch), Some(4));
    // The second poll is conditional; the cached record is reused on 304.
    assert_eq!(reader.fetch(&sid).await.expect("fetch").map(|r| r.epoch), Some(4));
    unchanged.assert_async().await;
    assert!(first.calls_async().await >= 1);
    assert!(reader.fetch("00000000000000000000000000000000").await.expect("fetch").is_none());
}

#[tokio::test]
async fn gist_errors_surface_the_status() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(GET).path("/gists/missing");
            then.status(404).body("Not Found");
        })
        .await;
    let mut reader = GistReader::with_api(http::client(), &server.base_url(), "missing", "tok");
    let err = reader.fetch("x").await.expect_err("404");
    assert!(err.to_string().contains("404"), "{err}");
}

#[tokio::test]
async fn folder_reader_reads_local_records() {
    let v = vectors();
    let sid = v["session_id"].as_str().expect("sid");
    let dir = tempfile::tempdir().expect("tempdir");
    let mut reader = RendezvousReader::Folder(dir.path().to_path_buf());
    assert!(reader.fetch(sid).await.expect("fetch").is_none());
    std::fs::write(dir.path().join(record_name(sid)), serde_json::to_string(&record(&v, 7)).expect("json"))
        .expect("write");
    assert_eq!(reader.fetch(sid).await.expect("fetch").map(|r| r.epoch), Some(7));
}
