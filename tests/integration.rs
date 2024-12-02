use std::{env, net::TcpListener, sync::OnceLock, time::Duration};

use axum::http;
use dotenvy::dotenv;
use pharia_kernel::{AppConfig, Completion, FinishReason, Kernel, OperatorConfig};
use reqwest::{header, Body};
use serde_json::json;
use test_skills::given_greet_skill_v0_2;
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
        let port = app_config.tcp_addr.port();
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
            tcp_addr: format!("127.0.0.1:{port}").parse().unwrap(),
            metrics_addr: format!("127.0.0.1:{metrics_port}").parse().unwrap(),
            inference_addr: "https://inference-api.product.pharia.com".to_owned(),
            document_index_addr: "https://document-index.product.pharia.com".to_owned(),
            authorization_addr: "https://inference-api.product.pharia.com".to_owned(),
            operator_config: OperatorConfig::local(skills),
            namespace_update_interval: Duration::from_secs(10),
            log_level: "info".to_owned(),
            open_telemetry_endpoint: None,
            use_pooling_allocator: true,
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
async fn execute_skill() {
    given_greet_skill_v0_2();
    let kernel = TestKernel::with_skills(&["greet_skill_v0_2"]).await;

    let api_token = api_token();
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
    auth_value.set_sensitive(true);
    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!("http://127.0.0.1:{}/execute_skill", kernel.port()))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value)
        .body(Body::from(
            json!({ "skill": "local/greet_skill_v0_2", "input": "Homer"}).to_string(),
        ))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = resp.text().await.unwrap();
    assert!(body.contains("Homer"));

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
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = resp.text().await.unwrap();
    assert!(body.to_lowercase().contains("kernel"));

    kernel.shutdown().await;
}

/// API Token used by tests to authenticate requests
fn api_token() -> &'static str {
    static API_TOKEN: OnceLock<String> = OnceLock::new();
    API_TOKEN.get_or_init(|| {
        drop(dotenv());
        env::var("AA_API_TOKEN").expect("AA_API_TOKEN variable not set")
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
