use std::time::Duration;

use httpmock::prelude::*;
use jumbo_bridge::http;
use jumbo_bridge::kaggle::{DatasetFile, KaggleAuth, KaggleClient};
use jumbo_bridge::secrets::{SecretsTimings, ensure_secrets, fingerprint, secrets_slug};
use serde_json::json;

const STATUS: &str = "/v1/datasets.DatasetApiService/GetDatasetStatus";
const START_UPLOAD: &str = "/v1/blobs.BlobApiService/StartBlobUpload";
const CREATE: &str = "/v1/datasets.DatasetApiService/CreateDataset";
const NEW_VERSION: &str = "/v1/datasets.DatasetApiService/CreateDatasetVersion";

fn client(server: &MockServer) -> KaggleClient {
    KaggleClient::with_api(http::client(), &server.base_url(), KaggleAuth::Token("tok".into()))
}

fn fast() -> SecretsTimings {
    SecretsTimings { ready: Duration::from_secs(2), poll: Duration::from_millis(20) }
}

/// The blob upload handshake: Kaggle hands out a pre-signed URL, the bytes go there, and the token
/// names the upload in the dataset request.
async fn mock_upload<'a>(server: &'a MockServer, body: &'static str) -> (httpmock::Mock<'a>, httpmock::Mock<'a>) {
    let start = server
        .mock_async(|when, then| {
            when.method(POST).path(START_UPLOAD).header("authorization", "Bearer tok").json_body_includes(format!(
                r#"{{"type":"DATASET","name":"gist_token","contentLength":{}}}"#,
                body.len()
            ));
            then.status(200).json_body(json!({"token": "blob-1", "createUrl": server.url("/upload/blob-1")}));
        })
        .await;
    let put = server
        .mock_async(|when, then| {
            when.method(PUT).path("/upload/blob-1").body(body);
            then.status(200);
        })
        .await;
    (start, put)
}

#[tokio::test]
async fn creates_a_private_dataset_when_none_exists() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(POST).path(STATUS);
            then.status(404).json_body(json!({"code": 404, "message": "Not found"}));
        })
        .await;
    let (start, put) = mock_upload(&server, "ghp_abc").await;
    let create = server
        .mock_async(|when, then| {
            when.method(POST).path(CREATE).json_body_includes(
                r#"{"ownerSlug":"alice","slug":"jumbo-secrets","isPrivate":true,"files":[{"token":"blob-1"}]}"#,
            );
            then.status(200).json_body(json!({"ref": "alice/jumbo-secrets", "status": "ok"}));
        })
        .await;

    let file = DatasetFile { name: "gist_token".into(), bytes: b"ghp_abc".to_vec() };
    client(&server).put_private_dataset("alice/jumbo-secrets", "Jumbo secrets", &[file]).await.expect("put");
    start.assert_async().await;
    put.assert_async().await;
    create.assert_async().await;
}

#[tokio::test]
async fn replaces_an_existing_dataset_and_deletes_old_versions() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(POST).path(STATUS).json_body(json!({"ownerSlug": "alice", "datasetSlug": "jumbo-secrets"}));
            then.status(200).json_body(json!({"status": "READY"}));
        })
        .await;
    mock_upload(&server, "ghp_new").await;
    let version = server
        .mock_async(|when, then| {
            when.method(POST).path(NEW_VERSION).json_body_includes(
                r#"{"ownerSlug":"alice","datasetSlug":"jumbo-secrets","body":{"deleteOldVersions":true,"files":[{"token":"blob-1"}]}}"#,
            );
            then.status(200).json_body(json!({"status": "ok"}));
        })
        .await;

    let got = ensure_secrets(&client(&server), "alice", "ghp_new", Some(&fingerprint("ghp_old")), &fast())
        .await
        .expect("ensure");
    assert_eq!(got, fingerprint("ghp_new"));
    version.assert_async().await;
}

#[tokio::test]
async fn skips_the_upload_when_the_token_is_unchanged() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(POST).path(STATUS);
            then.status(200).json_body(json!({"status": "READY"}));
        })
        .await;
    let start = server
        .mock_async(|when, then| {
            when.method(POST).path(START_UPLOAD);
            then.status(500);
        })
        .await;

    let same = fingerprint("ghp_abc");
    let got = ensure_secrets(&client(&server), "alice", "ghp_abc", Some(&same), &fast()).await.expect("ensure");
    assert_eq!(got, same);
    start.assert_calls_async(0).await;
}

#[tokio::test]
async fn surfaces_upload_errors() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(POST).path(STATUS);
            then.status(404);
        })
        .await;
    server
        .mock_async(|when, then| {
            when.method(POST).path(START_UPLOAD);
            then.status(403).json_body(json!({"code": 403, "message": "Permission 'datasets.create' was denied"}));
        })
        .await;
    let err = ensure_secrets(&client(&server), "alice", "ghp_abc", None, &fast()).await.expect_err("denied");
    assert!(err.to_string().contains("denied"), "{err}");
}

#[test]
fn slug_and_fingerprint_are_stable() {
    assert_eq!(secrets_slug("alice"), "alice/jumbo-secrets");
    assert_eq!(fingerprint("a"), fingerprint("a"));
    assert_ne!(fingerprint("a"), fingerprint("b"));
    assert!(!fingerprint("ghp_abc").contains("ghp_abc"));
}
