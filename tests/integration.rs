use std::{
    env,
    net::TcpListener,
    path::{Path, PathBuf},
    str::FromStr,
    sync::OnceLock,
    time::Duration,
};

use axum::http;
use bytesize::ByteSize;
use dotenvy::dotenv;
use futures::StreamExt;
use opentelemetry::trace::TracerProvider;
use opentelemetry_sdk::trace::{SdkTracerProvider, Tracer};
use pharia_kernel::{AppConfig, FeatureSet, Kernel, NamespaceConfigs};
use reqwest::{Body, header};
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};
use test_skills::{
    given_chat_stream_skill, given_complete_stream_skill, given_rust_skill_doc_metadata,
    given_rust_skill_greet_v0_2, given_rust_skill_greet_v0_3, given_rust_skill_search,
    given_skill_infinite_streaming, given_streaming_output_skill,
};
use tokio::sync::oneshot;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

struct TestFileRegistry {
    skills: Vec<String>,
    directory: TempDir,
}

impl TestFileRegistry {
    fn new() -> Self {
        Self {
            skills: Vec::new(),
            directory: tempdir().unwrap(),
        }
    }

    fn with_skill(&mut self, name: &str, wasm_bytes: Vec<u8>) {
        self.skills.push(name.to_owned());
        let mut file_path = self.directory.path().join(name);
        file_path.set_extension("wasm");
        std::fs::write(file_path, wasm_bytes).unwrap();
    }

    fn to_namespace_config(&self) -> NamespaceConfigs {
        let skills = self.skills.join("\"},{\"name\"=\"");
        let dir_path = self.directory.path().to_str().unwrap();
        // Mask directory path. It could contain characters which represent escape sequences
        let json_masked_string_with_quotes = serde_json::to_string(dir_path).unwrap();
        toml::from_str(&format!(
            r#"
            [local]
            path = {json_masked_string_with_quotes}
            skills = [{{"name"="{skills}"}}]"#,
        ))
        .unwrap()
    }
}

struct TestKernel {
    shutdown_trigger: oneshot::Sender<()>,
    kernel: Kernel,
    port: u16,
}

impl TestKernel {
    async fn new(namespaces: NamespaceConfigs) -> Self {
        let (shutdown_trigger, shutdown_capture) = oneshot::channel::<()>();
        let shutdown_signal = async {
            shutdown_capture.await.unwrap();
        };
        let port = free_test_port();
        let metrics_port = free_test_port();
        let app_config = AppConfig::default()
            .with_kernel_address(format!("127.0.0.1:{port}").parse().unwrap())
            .with_metrics_address(format!("127.0.0.1:{metrics_port}").parse().unwrap())
            .with_namespaces(namespaces)
            .with_pharia_ai_feature_set(FeatureSet::Beta)
            .with_wasmtime_cache_size_request(Some(ByteSize::mib(512)))
            .with_wasmtime_cache_dir(Some(std::path::absolute("./.wasmtime-cache").unwrap()))
            .with_pooling_allocator(true);
        let port = app_config.kernel_address().port();
        // Wait for socket listener to be bound
        let kernel = Kernel::new(app_config, shutdown_signal).await.unwrap();

        Self {
            shutdown_trigger,
            kernel,
            port,
        }
    }

    async fn with_skills(skills: &[&str]) -> Self {
        Self::new(namespace_config(
            &PathBuf::from_str("./skills").unwrap(),
            skills,
        ))
        .await
    }

    fn port(&self) -> u16 {
        self.port
    }

    async fn shutdown(self) {
        self.shutdown_trigger.send(()).unwrap();
        self.kernel.wait_for_shutdown().await;
    }
}

fn namespace_config(dir_path: &Path, skills: &[&str]) -> NamespaceConfigs {
    let skills = skills.join("\"},{\"name\"=\"");
    let dir_path = dir_path.to_str().unwrap();
    toml::from_str(&format!(
        r#"
        [local]
        path = "{dir_path}"
        skills = [{{"name"="{skills}"}}]"#,
    ))
    .unwrap()
}

#[cfg_attr(not(feature = "test_inference"), ignore)]
#[tokio::test]
async fn run_skill() {
    let greet_skill_wasm = given_rust_skill_greet_v0_2().bytes();
    let mut local_skill_dir = TestFileRegistry::new();
    local_skill_dir.with_skill("greet", greet_skill_wasm);
    let kernel = TestKernel::new(local_skill_dir.to_namespace_config()).await;

    let api_token = api_token();
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
    auth_value.set_sensitive(true);
    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!(
            "http://127.0.0.1:{}/v1/skills/local/greet/run",
            kernel.port()
        ))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value)
        .body(Body::from(json!("Homer").to_string()))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap();

    eprintln!("{resp:?}");
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = resp.text().await.unwrap();
    assert!(body.contains("Homer"));

    kernel.shutdown().await;
}

#[cfg_attr(not(feature = "test_document_index"), ignore)]
#[tokio::test]
async fn run_search_skill() {
    let wasm_bytes = given_rust_skill_search().bytes();
    let mut local_skill_dir: TestFileRegistry = TestFileRegistry::new();
    local_skill_dir.with_skill("search", wasm_bytes);
    let kernel = TestKernel::new(local_skill_dir.to_namespace_config()).await;

    let api_token = api_token();
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
    auth_value.set_sensitive(true);
    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!(
            "http://127.0.0.1:{}/v1/skills/local/search/run",
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
    let wasm_bytes = given_rust_skill_doc_metadata().bytes();
    let mut local_skill_dir: TestFileRegistry = TestFileRegistry::new();
    local_skill_dir.with_skill("doc_metadata", wasm_bytes);
    let kernel = TestKernel::new(local_skill_dir.to_namespace_config()).await;

    let api_token = api_token();
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
    auth_value.set_sensitive(true);
    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!(
            "http://127.0.0.1:{}/v1/skills/local/doc_metadata/run",
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
    let first_text = value["url"].as_str().unwrap();
    assert!(first_text.starts_with("https://pharia-kernel"));

    kernel.shutdown().await;
}

#[tokio::test]
async fn run_complete_stream_skill() {
    let wasm_bytes = given_complete_stream_skill().bytes();
    let mut local_skill_dir: TestFileRegistry = TestFileRegistry::new();
    local_skill_dir.with_skill("complete_stream", wasm_bytes);
    let kernel = TestKernel::new(local_skill_dir.to_namespace_config()).await;

    let api_token = api_token();
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
    auth_value.set_sensitive(true);
    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!(
            "http://127.0.0.1:{}/v1/skills/local/complete_stream/run",
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
    assert_eq!(
        value,
        json!([
            " \n\n Hello there! How are you doing today?<|endoftext|>",
            "FinishReason::Stop",
            "prompt: 64, completion: 13",
        ])
    );

    kernel.shutdown().await;
}

#[tokio::test]
async fn run_chat_stream_skill() {
    let wasm_bytes = given_chat_stream_skill().bytes();
    let mut local_skill_dir: TestFileRegistry = TestFileRegistry::new();
    local_skill_dir.with_skill("chat_stream", wasm_bytes);
    let kernel = TestKernel::new(local_skill_dir.to_namespace_config()).await;

    let api_token = api_token();
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
    auth_value.set_sensitive(true);
    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!(
            "http://127.0.0.1:{}/v1/skills/local/chat_stream/run",
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
    let values = value.as_array().unwrap();
    assert_eq!(values[0], "assistant");
    let completion = values[1..values.len() - 2]
        .iter()
        .fold(String::new(), |acc, v| acc + v.as_str().unwrap());
    assert_eq!(
        completion,
        "I'm here to help you with any questions or assistance you need. Please feel free to ask \
        anything, and I'll do my best to provide helpful information or guidance."
    );
    assert_eq!(values[values.len() - 2], "FinishReason::Stop");
    assert_eq!(values[values.len() - 1], "prompt: 17, completion: 37");

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
    let completion = serde_json::from_slice::<Value>(&body).unwrap();
    assert!(completion["text"].as_str().unwrap().contains("Homer"));
    assert_eq!(completion["finish_reason"], "stop");

    kernel.shutdown().await;
}

#[cfg_attr(not(feature = "test_inference"), ignore)]
#[tokio::test]
async fn chat_v0_2_via_remote_csi() {
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
                "function": "chat",
                "messages": [{"role": "User", "content": "Say hello to Homer"}],
                "model": "pharia-1-llm-7b-control",
                "params": {
                    "max_tokens": 64,
                    "temperature": null,
                    "top_p": null,
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
    let completion = serde_json::from_slice::<Value>(&body).unwrap();
    assert!(
        completion["message"]["role"]
            .as_str()
            .unwrap()
            .contains("assistant")
    );
    assert!(matches!(
        completion["finish_reason"].as_str().unwrap(),
        "stop"
    ));

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
    assert!(
        body["url"]
            .as_str()
            .unwrap()
            .starts_with("https://pharia-kernel")
    );

    kernel.shutdown().await;
}

fn tracing_subscriber(tracer: Tracer) -> impl tracing::Subscriber {
    let layer = tracing_opentelemetry::layer().with_tracer(tracer);
    tracing_subscriber::registry()
        .with(EnvFilter::from_str("info").unwrap())
        .with(layer)
}

#[tokio::test]
async fn invoke_message_stream_skill_with_tracing() {
    let provider = SdkTracerProvider::builder().build();
    let tracer = provider.tracer("test");
    tracing_subscriber(tracer).init();

    let streaming_output_skill_wasm = given_streaming_output_skill().bytes();
    let mut local_skill_dir = TestFileRegistry::new();
    local_skill_dir.with_skill("streaming-output", streaming_output_skill_wasm);
    let kernel = TestKernel::new(local_skill_dir.to_namespace_config()).await;

    let api_token = api_token();
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
    auth_value.set_sensitive(true);
    let req_client = reqwest::Client::new();
    let mut stream = req_client
        .post(format!(
            "http://127.0.0.1:{}/v1/skills/local/streaming-output/message-stream",
            kernel.port()
        ))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value)
        .body(Body::from(json!("Say one word: Homer").to_string()))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap()
        .bytes_stream();

    let mut events = vec![];
    while let Some(event) = stream.next().await {
        events.push(event.unwrap());
    }
    assert_eq!(events[0], "event: message\ndata: {\"type\":\"begin\"}\n\n");
    assert_eq!(
        events[1],
        "event: message\ndata: {\"type\":\"append\",\"text\":\"Homer\"}\n\n"
    );
    kernel.shutdown().await;
}

#[tokio::test]
async fn invoke_function_as_stream() {
    let greet_skill_wasm = given_rust_skill_greet_v0_3().bytes();
    let mut local_skill_dir = TestFileRegistry::new();
    local_skill_dir.with_skill("greet", greet_skill_wasm);
    let kernel = TestKernel::new(local_skill_dir.to_namespace_config()).await;

    let api_token = api_token();
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
    auth_value.set_sensitive(true);
    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!(
            "http://127.0.0.1:{}/v1/skills/local/greet/message-stream",
            kernel.port()
        ))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value)
        .body(Body::from(json!("Homer").to_string()))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap();

    eprintln!("{resp:?}");
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = resp.text().await.unwrap();
    assert_eq!(
        body,
        "event: error\n\
        data: {\"message\":\"The skill is designed to be executed as a function. Please invoke it \
        via the /run endpoint.\"}\n\n"
    );

    kernel.shutdown().await;
}

#[tokio::test]
async fn test_message_stream_canceled_during_the_skill_execution() {
    let greet_skill_wasm = given_skill_infinite_streaming().bytes();
    let mut local_skill_dir = TestFileRegistry::new();
    local_skill_dir.with_skill("infinite", greet_skill_wasm);
    let kernel = TestKernel::new(local_skill_dir.to_namespace_config()).await;

    let api_token = api_token();
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
    auth_value.set_sensitive(true);

    let req_client = reqwest::Client::new();
    let mut stream = req_client
        .post(format!(
            "http://127.0.0.1:{}/v1/skills/local/infinite/message-stream",
            kernel.port()
        ))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value)
        .body(Body::from(json!("").to_string()))
        .send()
        .await
        .unwrap()
        .bytes_stream();

    // Pull at least once from the stream so that it is sent
    drop(stream.next().await.unwrap());
    drop(stream);
    drop(req_client);
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
