use std::{env, net::TcpListener, sync::OnceLock, time::Duration};

use axum::http;
use dotenvy::dotenv;
use pharia_kernel::{AppConfig, Completion, FinishReason, Kernel, NamespaceConfigs};
use reqwest::{header, Body};
use serde_json::{json, Value};
use test_skills::{given_doc_metadata_skill, given_greet_skill_v0_2, given_search_skill};
use tokio::sync::oneshot;

struct TestKernel {
    shutdown_trigger: oneshot::Sender<()>,
    kernel: Kernel,
    port: u16,
}

impl TestKernel {
    async fn new(app_config: AppConfig) -> Self {
        let (shutdown_trigger, shutdown_capture) = oneshot::channel::<()>();
        let shutdown_signal = async {
            shutdown_capture.await.unwrap();
        };
        let port = app_config.kernel_address.port();
        // Wait for socket listener to be bound
        let kernel = Kernel::new(app_config, shutdown_signal).await.unwrap();

        Self {
            shutdown_trigger,
            kernel,
            port,
        }
    }

    async fn with_skills(skills: &[&str]) -> Self {
        let port = free_test_port();
        let metrics_port = free_test_port();
        let app_config = AppConfig {
            kernel_address: format!("127.0.0.1:{port}").parse().unwrap(),
            metrics_address: format!("127.0.0.1:{metrics_port}").parse().unwrap(),
            namespaces: NamespaceConfigs::local(skills),
            use_pooling_allocator: true,
            ..AppConfig::default()
        };
        Self::new(app_config).await
    }

    fn port(&self) -> u16 {
        self.port
    }

    async fn shutdown(self) {
        self.shutdown_trigger.send(()).unwrap();
        self.kernel.wait_for_shutdown().await;
    }
}

#[cfg_attr(not(feature = "test_inference"), ignore)]
#[tokio::test]
async fn run_skill() {
    given_greet_skill_v0_2();
    let kernel = TestKernel::with_skills(&["greet_skill_v0_2"]).await;

    let api_token = api_token();
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
    auth_value.set_sensitive(true);
    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!(
            "http://127.0.0.1:{}/v1/skills/local/greet_skill_v0_2/run",
            kernel.port()
        ))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value)
        .body(Body::from(json!("Homer").to_string()))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = resp.text().await.unwrap();
    assert!(body.contains("Homer"));

    kernel.shutdown().await;
}

#[cfg_attr(not(feature = "test_document_index"), ignore)]
#[tokio::test]
async fn run_search_skill() {
    given_search_skill();
    let kernel = TestKernel::with_skills(&["search_skill"]).await;

    let api_token = api_token();
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
    auth_value.set_sensitive(true);
    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!(
            "http://127.0.0.1:{}/v1/skills/local/search_skill/run",
            kernel.port()
        ))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value)
        .body(Body::from(json!("What is the Pharia Kernel?").to_string()))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let value: Value = resp.json().await.unwrap();
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(value.is_array());
    assert!(!value.as_array().unwrap().is_empty());
    let first_text = value[0].clone().to_string();
    assert!(first_text.to_ascii_lowercase().contains("kernel"));

    kernel.shutdown().await;
}

#[tokio::test]
async fn run_doc_metadata_skill() {
    given_doc_metadata_skill();
    let kernel = TestKernel::with_skills(&["doc_metadata_skill"]).await;

    let api_token = api_token();
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
    auth_value.set_sensitive(true);
    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!(
            "http://127.0.0.1:{}/v1/skills/local/doc_metadata_skill/run",
            kernel.port()
        ))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value)
        .body(Body::from(json!("ignore for now").to_string()))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let value: Value = resp.json().await.unwrap();
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(value.is_array());
    assert!(!value.as_array().unwrap().is_empty());
    let first_text = value[0]["url"].as_str().unwrap();
    assert!(first_text.starts_with("https://pharia-kernel"));

    kernel.shutdown().await;
}

#[cfg_attr(not(feature = "test_inference"), ignore)]
#[tokio::test]
async fn completion_via_remote_csi() {
    let kernel = TestKernel::with_skills(&[]).await;

    let api_token = api_token();
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
    auth_value.set_sensitive(true);
    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!("http://127.0.0.1:{}/csi", kernel.port()))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value)
        .body(Body::from(
            json!({
                "version": "0.2",
                "function": "complete",
                "prompt": "<|begin_of_text|><|start_header_id|>system<|end_header_id|>

You are a helpful assistant.<|eot_id|><|start_header_id|>user<|end_header_id|>

Say hello to Homer<|eot_id|><|start_header_id|>assistant<|end_header_id|>",
                "model": "pharia-1-llm-7b-control",
                "params": {
                    "max_tokens": 64,
                    "temperature": null,
                    "top_k": null,
                    "top_p": null,
                    "stop": []
                }
            })
            .to_string(),
        ))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = resp.bytes().await.unwrap();
    let completion = serde_json::from_slice::<Completion>(&body).unwrap();
    assert!(completion.text.contains("Homer"));
    assert!(matches!(completion.finish_reason, FinishReason::Stop));

    kernel.shutdown().await;
}

#[tokio::test]
async fn unsupported_csi_function() {
    let kernel = TestKernel::with_skills(&[]).await;

    let api_token = api_token();
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
    auth_value.set_sensitive(true);
    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!("http://127.0.0.1:{}/csi", kernel.port()))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value)
        .body(Body::from(
            json!({"version": "0.2", "function": "foo"}).to_string(),
        ))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = resp.bytes().await.unwrap();
    let error = serde_json::from_slice::<String>(&body).unwrap();
    assert!(error.contains("not supported by this Kernel installation yet"));

    kernel.shutdown().await;
}

#[tokio::test]
async fn unsupported_old_csi_version() {
    let kernel = TestKernel::with_skills(&[]).await;

    let api_token = api_token();
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
    auth_value.set_sensitive(true);
    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!("http://127.0.0.1:{}/csi", kernel.port()))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value)
        .body(Body::from(json!({"version": "0.1"}).to_string()))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = resp.bytes().await.unwrap();
    let error = serde_json::from_slice::<String>(&body).unwrap();
    assert!(error.contains("no longer supported"));

    kernel.shutdown().await;
}

#[tokio::test]
async fn error_for_lack_of_version() {
    let kernel = TestKernel::with_skills(&[]).await;

    let api_token = api_token();
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
    auth_value.set_sensitive(true);
    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!("http://127.0.0.1:{}/csi", kernel.port()))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value)
        .body(Body::from(json!({}).to_string()))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = resp.bytes().await.unwrap();
    let error = serde_json::from_slice::<String>(&body).unwrap();
    assert!(error.contains("version is required"));

    kernel.shutdown().await;
}

#[tokio::test]
async fn unsupported_newer_csi_version() {
    let kernel = TestKernel::with_skills(&[]).await;

    let api_token = api_token();
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
    auth_value.set_sensitive(true);
    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!("http://127.0.0.1:{}/csi", kernel.port()))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value)
        .body(Body::from(
            json!({"version": u32::MAX.to_string() }).to_string(),
        ))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = resp.bytes().await.unwrap();
    let error = serde_json::from_slice::<String>(&body).unwrap();
    assert!(error.contains("not supported by this Kernel installation yet"));

    kernel.shutdown().await;
}

#[tokio::test]
async fn unsupported_newer_minor_csi_version() {
    let kernel = TestKernel::with_skills(&[]).await;

    let api_token = api_token();
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
    auth_value.set_sensitive(true);
    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!("http://127.0.0.1:{}/csi", kernel.port()))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value)
        .body(Body::from(
            json!({"version": format!("0.{}", u32::MAX) }).to_string(),
        ))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = resp.bytes().await.unwrap();
    let error = serde_json::from_slice::<String>(&body).unwrap();
    assert!(error.contains("not supported by this Kernel installation yet"));

    kernel.shutdown().await;
}

#[cfg_attr(not(feature = "test_document_index"), ignore)]
#[tokio::test]
async fn search_via_remote_csi() {
    let kernel = TestKernel::with_skills(&[]).await;

    let api_token = api_token();
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
    auth_value.set_sensitive(true);
    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!("http://127.0.0.1:{}/csi", kernel.port()))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value)
        .body(Body::from(
            json!({
                "version": "0.2",
                "function": "search",
                "index_path": {
                    "namespace": "Kernel",
                    "collection": "test",
                    "index": "asym-64",
                },
                "query":"What is the Pharia Kernel?",
                "max_results":10,
                "min_score":0.1,
            })
            .to_string(),
        ))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body = resp.text().await.unwrap();
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(body.to_lowercase().contains("kernel"));

    kernel.shutdown().await;
}

#[cfg_attr(not(feature = "test_document_index"), ignore)]
#[tokio::test]
async fn metadata_via_remote_csi() {
    let kernel = TestKernel::with_skills(&[]).await;

    let api_token = api_token();
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
    auth_value.set_sensitive(true);
    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!("http://127.0.0.1:{}/csi", kernel.port()))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value)
        .body(Body::from(
            json!({
                "version": "0.2",
                "function": "document_metadata",
                "document_path": {
                    "namespace": "Kernel",
                    "collection": "test",
                    "name": "kernel-docs",
                },
            })
            .to_string(),
        ))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    assert!(body[0]["url"]
        .as_str()
        .unwrap()
        .starts_with("https://pharia-kernel"));

    kernel.shutdown().await;
}

/// API Token used by tests to authenticate requests
fn api_token() -> &'static str {
    static API_TOKEN: OnceLock<String> = OnceLock::new();
    API_TOKEN.get_or_init(|| {
        drop(dotenv());
        env::var("PHARIA_AI_TOKEN").expect("PHARIA_AI_TOKEN variable not set")
    })
}

/// Ask the operating system for the next free port
fn free_test_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
