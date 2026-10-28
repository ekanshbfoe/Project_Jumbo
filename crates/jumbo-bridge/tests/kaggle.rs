use httpmock::prelude::*;
use jumbo_bridge::http;
use jumbo_bridge::kaggle::{KaggleAuth, KaggleClient, KernelPush};
use serde_json::json;

fn push() -> KernelPush {
    KernelPush {
        slug: "alice/jumbo-worker".into(),
        title: "jumbo-worker".into(),
        source: "print('hi')".into(),
        machine_shape: Some("TpuV5E8".into()),
        dataset_sources: vec![],
        model_sources: vec!["alice/qwen/transformers/72b-int8/1".into()],
        kernel_sources: vec!["alice/jumbo-render-c1".into()],
    }
}

#[test]
fn parses_kaggle_json_and_never_prints_the_key() {
    let auth = KaggleAuth::from_kaggle_json(r#"{"username":"alice","key":"s3cret"}"#).expect("parse");
    assert_eq!(format!("{auth:?}"), "KaggleAuth::Key(alice, ***)");
}

#[tokio::test]
async fn push_sends_the_sdk_request_with_basic_auth() {
    let server = MockServer::start_async().await;
    let save = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/kernels.KernelsApiService/SaveKernel")
                // base64("alice:s3cret")
                .header("authorization", "Basic YWxpY2U6czNjcmV0")
                .json_body_includes(r#"{"slug":"alice/jumbo-worker","machineShape":"TpuV5E8","enableTpu":true,"enableGpu":false,"dockerImagePinningType":"latest","enableInternet":true,"kernelType":"script","isPrivate":true}"#)
                .json_body_includes(r#"{"modelDataSources":["alice/qwen/transformers/72b-int8/1"]}"#)
                .json_body_includes(r#"{"kernelDataSources":["alice/jumbo-render-c1"]}"#);
            then.status(200).json_body(json!({"ref": "/code/alice/jumbo-worker", "url": "https://www.kaggle.com/code/alice/jumbo-worker", "versionNumber": 3}));
        })
        .await;
    let auth = KaggleAuth::Key { username: "alice".into(), key: "s3cret".into() };
    let client = KaggleClient::with_api(http::client(), &server.base_url(), auth);
    let result = client.push(&push()).await.expect("push");
    assert_eq!(result.version_number, Some(3));
    save.assert_async().await;
}

#[tokio::test]
async fn push_surfaces_kaggle_errors() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/kernels.KernelsApiService/SaveKernel");
            then.status(200).json_body(json!({"error": "Weekly TPU quota exceeded"}));
        })
        .await;
    let client = KaggleClient::with_api(http::client(), &server.base_url(), KaggleAuth::Token("t".into()));
    let err = client.push(&push()).await.expect_err("quota");
    assert!(err.to_string().contains("Weekly TPU quota exceeded"), "{err}");
}

#[tokio::test]
async fn status_reads_the_worker_status_with_a_bearer_token() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/kernels.KernelsApiService/GetKernelSessionStatus")
                .header("authorization", "Bearer tok")
                .json_body(json!({"userName": "alice", "kernelSlug": "jumbo-worker"}));
            then.status(200).json_body(json!({"status": "RUNNING"}));
        })
        .await;
    let client = KaggleClient::with_api(http::client(), &server.base_url(), KaggleAuth::Token("tok".into()));
    let status = client.status("alice/jumbo-worker").await.expect("status");
    assert_eq!(status.status, "RUNNING");
    assert!(!status.is_finished());
}

#[tokio::test]
async fn http_errors_carry_kaggles_message() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/kernels.KernelsApiService/GetKernelSessionStatus");
            then.status(403).json_body(json!({"code": 403, "message": "Permission 'kernels.get' was denied"}));
        })
        .await;
    let client = KaggleClient::with_api(http::client(), &server.base_url(), KaggleAuth::Token("t".into()));
    let err = client.status("alice/x").await.expect_err("403");
    assert!(err.to_string().contains("403") && err.to_string().contains("denied"), "{err}");
}

#[tokio::test]
async fn output_files_are_listed_across_pages_and_downloaded() {
    let server = MockServer::start_async().await;
    let url = server.url("/signed/chunk1.mp4");
    server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/kernels.KernelsApiService/ListKernelSessionOutput")
                .json_body(json!({"userName": "alice", "kernelSlug": "jumbo-render-c1", "pageSize": 100}));
            then.status(200).json_body(
                json!({"files": [{"fileName": "chunk1.mp4", "url": url}], "log": "", "nextPageToken": "p2"}),
            );
        })
        .await;
    server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/kernels.KernelsApiService/ListKernelSessionOutput").json_body(
                json!({"userName": "alice", "kernelSlug": "jumbo-render-c1", "pageSize": 100, "pageToken": "p2"}),
            );
            then.status(200)
                .json_body(json!({"files": [{"fileName": "log.txt", "url": server.url("/signed/log.txt")}]}));
        })
        .await;
    server
        .mock_async(|when, then| {
            when.method(GET).path("/signed/chunk1.mp4");
            then.status(200).body("MP4 BYTES");
        })
        .await;
    let client = KaggleClient::with_api(http::client(), &server.base_url(), KaggleAuth::Token("tok".into()));
    let files = client.output_files("alice/jumbo-render-c1").await.expect("files");
    assert_eq!(files.iter().map(|f| f.file_name.as_str()).collect::<Vec<_>>(), ["chunk1.mp4", "log.txt"]);
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dir.path().join("chunk1.mp4");
    client.download_output(&files[0], &dest).await.expect("download");
    assert_eq!(std::fs::read(&dest).expect("file"), b"MP4 BYTES");
    assert!(!dir.path().join("chunk1.partial").exists());
}
