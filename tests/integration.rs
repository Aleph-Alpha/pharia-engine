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
use dotenvy::from_filename;
use futures::StreamExt;
use opentelemetry::{SpanId, TraceId};
use pharia_engine::{AppConfig, FeatureSet, Kernel, NamespaceConfigs};
use reqwest::{Body, header};
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};
use test_skills::{
    given_chat_stream_skill, given_complete_stream_skill, given_rust_skill_doc_metadata,
    given_rust_skill_greet_v0_2, given_rust_skill_greet_v0_3, given_skill_infinite_streaming,
    given_skill_tool_invocation, given_sse_mcp_server, given_streaming_output_skill,
};

mod tracing;
use tracing::{SequentialTestGuard, exclusive_log_recorder, log_recorder};

struct TestFileRegistry {
    skills: Vec<String>,
    directory: TempDir,
    mcp_servers: Vec<String>,
}

impl TestFileRegistry {
    fn new() -> Self {
        Self {
            skills: Vec::new(),
            directory: tempdir().unwrap(),
            mcp_servers: Vec::new(),
        }
    }

    fn with_skill(&mut self, name: &str, wasm_bytes: Vec<u8>) {
        self.skills.push(name.to_owned());
        let mut file_path = self.directory.path().join(name);
        file_path.set_extension("wasm");
        std::fs::write(file_path, wasm_bytes).unwrap();
    }

    fn with_mcp_server(&mut self, url: &str) {
        self.mcp_servers.push(url.to_owned());
    }

    fn to_namespace_config(&self) -> NamespaceConfigs {
        let skills = self.skills.join("\"},{\"name\"=\"");
        let dir_path = self.directory.path().to_str().unwrap();
        // Mask directory path. It could contain characters which represent escape sequences
        let json_masked_string_with_quotes = serde_json::to_string(dir_path).unwrap();
        let mcp_servers = self.mcp_servers.join("\",\"");
        toml::from_str(&format!(
            r#"
            [local]
            path = {json_masked_string_with_quotes}
            skills = [{{"name"="{skills}"}}]
            mcp-servers = ["{mcp_servers}"]"#,
        ))
        .unwrap()
    }
}

struct TestKernel {
    kernel: Kernel,
    port: u16,
    log_recorder: &'static tracing::LogRecorder,
    _guard: SequentialTestGuard,
}

impl TestKernel {
    async fn default() -> Self {
        let app_config = Self::default_config();
        Self::new(app_config, false).await
    }

    /// Create a `TestKernel` instance that is guaranteed to be the only `TestKernel` running.
    async fn exclusive(app_config: AppConfig) -> Self {
        Self::new(app_config, true).await
    }

    async fn without_authorization() -> Self {
        let app_config = Self::default_config().with_authorization_url(None);
        Self::new(app_config, false).await
    }

    async fn with_namespaces(namespaces: NamespaceConfigs) -> Self {
        let app_config = Self::default_config().with_namespaces(namespaces);
        Self::new(app_config, false).await
    }

    async fn with_openai_inference(namespaces: NamespaceConfigs, token: &str) -> Self {
        let url = "https://api.openai.com/v1";
        let app_config = Self::default_config()
            .with_namespaces(namespaces)
            .with_openai_inference(url, token)
            .with_inference_url(None)
            .with_authorization_url(None);
        Self::new(app_config, false).await
    }

    pub fn default_config() -> AppConfig {
        let port = free_test_port();
        let metrics_port = free_test_port();
        AppConfig::default()
            .with_kernel_address(format!("127.0.0.1:{port}").parse().unwrap())
            .with_metrics_address(format!("127.0.0.1:{metrics_port}").parse().unwrap())
            .with_document_index_url("https://document-index.product.pharia.com")
            .with_inference_url(Some("https://inference-api.product.pharia.com"))
            .with_authorization_url(Some("https://pharia-iam.product.pharia.com"))
            .with_namespaces(NamespaceConfigs::default())
            .with_pharia_ai_feature_set(FeatureSet::Beta)
            .with_wasmtime_cache_size_request(Some(ByteSize::mib(512)))
            .with_wasmtime_cache_dir(Some(std::path::absolute("./.wasmtime-cache").unwrap()))
            .with_pooling_allocator(true)
    }

    async fn new(app_config: AppConfig, exclusive: bool) -> Self {
        let port = app_config.kernel_address().port();
        // Wait for socket listener to be bound
        let kernel = Kernel::new(app_config).await.unwrap();
        let (guard, log_recorder) = if exclusive {
            exclusive_log_recorder().await
        } else {
            log_recorder().await
        };

        Self {
            kernel,
            port,
            log_recorder,
            _guard: guard,
        }
    }

    async fn with_skills(skills: &[&str]) -> Self {
        Self::with_namespaces(namespace_config(
            &PathBuf::from_str("./skills").unwrap(),
            skills,
        ))
        .await
    }

    fn log_recorder(&self) -> &'static tracing::LogRecorder {
        self.log_recorder
    }

    fn port(&self) -> u16 {
        self.port
    }

    async fn shutdown(self) {
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
    let kernel = TestKernel::with_namespaces(local_skill_dir.to_namespace_config()).await;

    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!(
            "http://127.0.0.1:{}/v1/skills/local/greet/run",
            kernel.port()
        ))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value())
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

#[tokio::test]
async fn tools_can_be_listed() {
    let mcp = given_sse_mcp_server().await;
    let mut local_skill_dir: TestFileRegistry = TestFileRegistry::new();
    local_skill_dir.with_mcp_server(mcp.address());
    let kernel = TestKernel::with_namespaces(local_skill_dir.to_namespace_config()).await;

    let req_client = reqwest::Client::new();
    let resp = req_client
        .get(format!("http://127.0.0.1:{}/v1/tools/local", kernel.port()))
        .header(header::AUTHORIZATION, auth_value())
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let tools: Value = resp.json().await.unwrap();
    let expected = json!([
        {
            "name": "add",
            "description": "Add two numbers",
            "input_schema": {
                "properties": {
                    "a": {
                        "title": "A",
                        "type": "integer"
                    },
                    "b": {
                        "title": "B",
                        "type": "integer"
                    }
                },
                "required": [
                    "a",
                    "b"
                ],
                "title": "addArguments",
                "type": "object"
            },
        },
        {
            "name": "saboteur",
            "description": "I have ran out of cheese.",
            "input_schema": {
                "properties": {},
                "title": "saboteurArguments",
                "type": "object"
            },
        },
    ]);
    assert_eq!(tools, expected);
}

#[tokio::test]
async fn run_skill_with_tool_call() {
    let mcp = given_sse_mcp_server().await;

    let wasm_bytes = given_skill_tool_invocation().bytes();
    let mut local_skill_dir: TestFileRegistry = TestFileRegistry::new();
    local_skill_dir.with_skill("tool-invocation-rs", wasm_bytes);
    local_skill_dir.with_mcp_server(mcp.address());

    let kernel = TestKernel::with_namespaces(local_skill_dir.to_namespace_config()).await;

    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!(
            "http://127.0.0.1:{}/v1/skills/local/tool-invocation-rs/run",
            kernel.port()
        ))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value())
        .body(Body::from(json!({"a": 1, "b": 2}).to_string()))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = resp.text().await.unwrap();
    assert_eq!(body, "3");

    kernel.shutdown().await;
}

#[tokio::test]
async fn run_doc_metadata_skill() {
    let wasm_bytes = given_rust_skill_doc_metadata().bytes();
    let mut local_skill_dir: TestFileRegistry = TestFileRegistry::new();
    local_skill_dir.with_skill("doc_metadata", wasm_bytes);
    let kernel = TestKernel::with_namespaces(local_skill_dir.to_namespace_config()).await;

    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!(
            "http://127.0.0.1:{}/v1/skills/local/doc_metadata/run",
            kernel.port()
        ))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value())
        .body(Body::from(json!("ignore for now").to_string()))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert_eq!(status, axum::http::StatusCode::OK);
    let value: Value = resp.json().await.unwrap();
    let first_text = value["url"].as_str().unwrap();
    assert!(first_text.starts_with("https://pharia-kernel"));

    kernel.shutdown().await;
}

#[tokio::test]
async fn run_complete_stream_skill() {
    // Simulate the production environment with tracing enabled
    let wasm_bytes = given_complete_stream_skill().bytes();
    let mut local_skill_dir: TestFileRegistry = TestFileRegistry::new();
    local_skill_dir.with_skill("complete_stream", wasm_bytes);
    let kernel = TestKernel::with_namespaces(local_skill_dir.to_namespace_config()).await;

    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!(
            "http://127.0.0.1:{}/v1/skills/local/complete_stream/run",
            kernel.port()
        ))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value())
        .body(Body::from(json!("ignore for now").to_string()))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert_eq!(status, axum::http::StatusCode::OK);
    let value: Value = resp.json().await.unwrap();
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
    // Simulate the production environment with tracing enabled
    let wasm_bytes = given_chat_stream_skill().bytes();
    let mut local_skill_dir: TestFileRegistry = TestFileRegistry::new();
    local_skill_dir.with_skill("chat_stream", wasm_bytes);
    let kernel = TestKernel::with_namespaces(local_skill_dir.to_namespace_config()).await;

    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!(
            "http://127.0.0.1:{}/v1/skills/local/chat_stream/run",
            kernel.port()
        ))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value())
        .body(Body::from(
            json!({"model": "pharia-1-llm-7b-control", "query": "Homer"}).to_string(),
        ))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert_eq!(status, axum::http::StatusCode::OK);
    let value: Value = resp.json().await.unwrap();
    let values = value["events"].as_array().unwrap();
    assert_eq!(values[0], "assistant");
    let completion = values[1..values.len() - 2]
        .iter()
        .fold(String::new(), |acc, v| acc + v.as_str().unwrap());
    assert_eq!(
        completion,
        "Hello! How can I help you today? If you have any questions or need assistance, feel free \
        to ask."
    );
    assert_eq!(values[values.len() - 2], "FinishReason::Stop");
    assert_eq!(values[values.len() - 1], "prompt: 16, completion: 24");

    kernel.shutdown().await;
}

#[cfg_attr(not(feature = "test_inference"), ignore)]
#[tokio::test]
async fn completion_via_remote_csi() {
    // Simulate the production environment with tracing enabled
    let kernel = TestKernel::with_skills(&[]).await;

    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!("http://127.0.0.1:{}/csi", kernel.port()))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value())
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

#[cfg_attr(feature = "test_no_openai", ignore = "OpenAI tests disabled")]
#[tokio::test]
async fn openai_inference_backend_is_supported() {
    // Given
    let chat_skill_wasm = given_chat_stream_skill().bytes();
    let mut local_skill_dir = TestFileRegistry::new();
    local_skill_dir.with_skill("chat-stream", chat_skill_wasm);

    let token = openai_inference_token();
    let kernel =
        TestKernel::with_openai_inference(local_skill_dir.to_namespace_config(), token).await;

    // When
    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!(
            "http://127.0.0.1:{}/v1/skills/local/chat-stream/run",
            kernel.port()
        ))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .body(Body::from(
            json!({
                "model": "gpt-4o-mini",
                "query": "Who trained you?",
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

    // Then
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let value: Value = resp.json().await.unwrap();
    assert!(value["completion"].as_str().unwrap().contains("OpenAI"));

    kernel.shutdown().await;
}

#[cfg_attr(not(feature = "test_inference"), ignore)]
#[tokio::test]
async fn chat_v0_2_via_remote_csi() {
    // Simulate the production environment with tracing enabled
    let kernel = TestKernel::with_skills(&[]).await;

    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!("http://127.0.0.1:{}/csi", kernel.port()))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value())
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

    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!("http://127.0.0.1:{}/csi", kernel.port()))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value())
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

    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!("http://127.0.0.1:{}/csi", kernel.port()))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value())
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

    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!("http://127.0.0.1:{}/csi", kernel.port()))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value())
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
async fn list_mcp_servers_for_empty_namespace_returns_empty_list() {
    let kernel = TestKernel::with_skills(&[]).await;

    let req_client = reqwest::Client::new();
    let resp = req_client
        .get(format!(
            "http://127.0.0.1:{}/v1/mcp_servers/local",
            kernel.port()
        ))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value())
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = resp.bytes().await.unwrap();
    let response: Value = serde_json::from_slice::<Value>(&body).unwrap();
    assert_eq!(response, json!([]));

    kernel.shutdown().await;
}

#[tokio::test]
async fn list_tools_for_empty_namespace_returns_empty_list() {
    let kernel = TestKernel::with_skills(&[]).await;

    let req_client = reqwest::Client::new();
    let resp = req_client
        .get(format!("http://127.0.0.1:{}/v1/tools/local", kernel.port()))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value())
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = resp.bytes().await.unwrap();
    let response: Value = serde_json::from_slice::<Value>(&body).unwrap();
    assert_eq!(response, json!([]));

    kernel.shutdown().await;
}

#[tokio::test]
async fn unsupported_newer_csi_version() {
    let kernel = TestKernel::with_skills(&[]).await;

    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!("http://127.0.0.1:{}/csi", kernel.port()))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value())
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

    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!("http://127.0.0.1:{}/csi", kernel.port()))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value())
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
async fn metadata_via_remote_csi() {
    // Simulate the production environment with tracing enabled
    let kernel = TestKernel::with_skills(&[]).await;

    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!("http://127.0.0.1:{}/csi", kernel.port()))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value())
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

/// We are testing that sampling information from a traceparent header is respected.
#[tokio::test]
#[allow(clippy::unreadable_literal)]
async fn span_sampled_when_requested() {
    let app_config = TestKernel::default_config();
    let kernel = TestKernel::exclusive(app_config).await;
    let log_recorder = kernel.log_recorder();

    // When we do a request with a parent span that is sampled
    let trace_id: u128 = 0x0af7651916cd43dd8448eb211c80319c;
    let parent_span_id: u64 = 0xb7ad6b7169203331;
    let sampled: u8 = 1;
    let traceparent = format!("00-{trace_id:032x}-{parent_span_id:016x}-{sampled:02x}");

    let req_client = reqwest::Client::new();
    drop(
        req_client
            .post(format!("http://127.0.0.1:{}/health", kernel.port()))
            .header("traceparent", traceparent)
            .send()
            .await
            .unwrap(),
    );
    kernel.shutdown().await;

    // Then a span is sampled
    let spans = log_recorder.spans().into_iter().rev().collect::<Vec<_>>();
    assert_eq!(spans.len(), 1);
}

#[tokio::test]
#[allow(clippy::unreadable_literal)]
async fn span_not_sampled_when_not_requested() {
    let app_config = TestKernel::default_config();
    let kernel = TestKernel::exclusive(app_config).await;
    let log_recorder = kernel.log_recorder();

    // When we do a request with a parent span that is not sampled
    let trace_id: u128 = 0x0af7651916cd43dd8448eb211c80319c;
    let parent_span_id: u64 = 0xb7ad6b7169203331;
    let sampled: u8 = 0;
    let traceparent = format!("00-{trace_id:032x}-{parent_span_id:016x}-{sampled:02x}");

    let req_client = reqwest::Client::new();
    drop(
        req_client
            .post(format!("http://127.0.0.1:{}/health", kernel.port()))
            .header("traceparent", traceparent)
            .send()
            .await
            .unwrap(),
    );
    kernel.shutdown().await;

    // Then no span is recorded
    let spans = log_recorder.spans().into_iter().rev().collect::<Vec<_>>();
    assert_eq!(spans.len(), 0);
}

#[tokio::test]
async fn chat_content_can_be_configured_to_be_captured() {
    // Given a Kernel that is configured to capture the chat content
    let streaming_output_skill_wasm = given_streaming_output_skill().bytes();
    let mut local_skill_dir = TestFileRegistry::new();
    local_skill_dir.with_skill("streaming-output", streaming_output_skill_wasm);
    let app_config = TestKernel::default_config()
        .with_namespaces(local_skill_dir.to_namespace_config())
        .with_gen_ai_content_capture(true)
        .with_otel_sampling_ratio(1.0)
        .unwrap();
    let kernel = TestKernel::exclusive(app_config).await;
    let log_recorder = kernel.log_recorder();

    // When we execute a skill
    let req_client = reqwest::Client::new();
    let response = req_client
        .post(format!(
            "http://127.0.0.1:{}/v1/skills/local/streaming-output/message-stream",
            kernel.port()
        ))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value())
        .body(Body::from(json!("Say one word: Homer").to_string()))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    let mut stream = response.bytes_stream();
    while stream.next().await.is_some() {}

    kernel.shutdown().await;

    // Then we have a `chat` span
    let spans = log_recorder.spans().into_iter().rev().collect::<Vec<_>>();
    assert_eq!(spans.len(), 7);
    let chat_stream = spans.iter().find(|s| s.name == "chat").unwrap();

    // And the token usage is set on the span
    let input_tokens = chat_stream
        .attributes
        .iter()
        .find(|a| a.key == "gen_ai.usage.input_tokens".into())
        .unwrap();
    assert_eq!(input_tokens.value.as_str(), "19");
    let output_tokens = chat_stream
        .attributes
        .iter()
        .find(|a| a.key == "gen_ai.usage.output_tokens".into())
        .unwrap();
    assert_eq!(output_tokens.value.as_str(), "2");

    // And the input and output messages are recorded
    let input_messages = chat_stream
        .attributes
        .iter()
        .find(|a| a.key == "gen_ai.input.messages".into())
        .unwrap();
    let output_messages = chat_stream
        .attributes
        .iter()
        .find(|a| a.key == "gen_ai.output.messages".into())
        .unwrap();

    let input_messages = serde_json::from_str::<Value>(&input_messages.value.as_str()).unwrap();
    assert_eq!(
        input_messages,
        json!([{"role":"user","content":"Say one word: Homer"}]),
    );
    let output_messages = serde_json::from_str::<Value>(&output_messages.value.as_str()).unwrap();
    assert_eq!(
        output_messages,
        json!([{"content":"Homer","role":"assistant"}]),
    );
}

#[tokio::test]
#[allow(clippy::unreadable_literal)]
async fn traceparent_is_respected() {
    // Given a Kernel that is the only one running
    let streaming_output_skill_wasm = given_streaming_output_skill().bytes();
    let mut local_skill_dir = TestFileRegistry::new();
    local_skill_dir.with_skill("streaming-output", streaming_output_skill_wasm);
    let app_config =
        TestKernel::default_config().with_namespaces(local_skill_dir.to_namespace_config());
    let kernel = TestKernel::exclusive(app_config).await;
    let log_recorder = kernel.log_recorder();

    // When we execute a skill with a traceheader
    let trace_id: u128 = 0x0af7651916cd43dd8448eb211c80319c;
    let parent_span_id: u64 = 0xb7ad6b7169203331;
    let traceparent = format!("00-{trace_id:032x}-{parent_span_id:016x}-01");

    let req_client = reqwest::Client::new();
    let mut stream = req_client
        .post(format!(
            "http://127.0.0.1:{}/v1/skills/local/streaming-output/message-stream",
            kernel.port()
        ))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value())
        .header("traceparent", traceparent)
        .body(Body::from(json!("Say one word: Homer").to_string()))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap()
        .bytes_stream();

    // Consume the stream
    while stream.next().await.is_some() {}
    kernel.shutdown().await;

    // Then we should have recorded spans that all belong to the same trace id
    let spans = log_recorder.spans().into_iter().rev().collect::<Vec<_>>();
    assert_eq!(spans.len(), 7);
    let trace_ids = spans
        .iter()
        .map(|s| s.span_context.trace_id())
        .collect::<Vec<_>>();
    assert!(trace_ids.iter().all(|id| id == &TraceId::from(trace_id)));

    let outer_span = &spans[0];
    let skill_execution = &spans[1];
    let chat_stream = &spans[2];
    let load_skill = &spans[3];
    let compile = &spans[4];
    let download = &spans[5];
    let auth = &spans[6];

    assert_eq!(
        outer_span.name,
        "POST /v1/skills/{namespace}/{name}/message-stream"
    );
    assert_eq!(skill_execution.name, "skill_execution");
    assert_eq!(chat_stream.name, "chat");

    // Input and output are not set as the default config is not to capture the content
    let input_messages = chat_stream
        .attributes
        .iter()
        .find(|a| a.key == "gen_ai.input.messages".into());
    assert!(input_messages.is_none());

    let output_messages = chat_stream
        .attributes
        .iter()
        .find(|a| a.key == "gen_ai.output.messages".into());
    assert!(output_messages.is_none());

    assert_eq!(compile.name, "compile");
    assert_eq!(load_skill.name, "load_skill");
    assert_eq!(download.name, "download");
    assert_eq!(auth.name, "check_permissions");

    assert_eq!(outer_span.parent_span_id, SpanId::from(parent_span_id));
    assert_eq!(load_skill.parent_span_id, outer_span.span_context.span_id());
    assert_eq!(compile.parent_span_id, load_skill.span_context.span_id());
}

#[tokio::test]
async fn invoke_function_as_stream() {
    // Simulate the production environment with tracing enabled
    let greet_skill_wasm = given_rust_skill_greet_v0_3().bytes();
    let mut local_skill_dir = TestFileRegistry::new();
    local_skill_dir.with_skill("greet", greet_skill_wasm);
    let kernel = TestKernel::with_namespaces(local_skill_dir.to_namespace_config()).await;

    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!(
            "http://127.0.0.1:{}/v1/skills/local/greet/message-stream",
            kernel.port()
        ))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value())
        .body(Body::from(json!("Homer").to_string()))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap();

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
    // Simulate the production environment with tracing enabled
    let greet_skill_wasm = given_skill_infinite_streaming().bytes();
    let mut local_skill_dir = TestFileRegistry::new();
    local_skill_dir.with_skill("infinite", greet_skill_wasm);
    let kernel = TestKernel::with_namespaces(local_skill_dir.to_namespace_config()).await;

    let req_client = reqwest::Client::new();
    let mut stream = req_client
        .post(format!(
            "http://127.0.0.1:{}/v1/skills/local/infinite/message-stream",
            kernel.port()
        ))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value())
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

#[tokio::test]
async fn token_is_required_if_auth_url_is_set() {
    // Given a Kernel with authorization enabled
    let kernel = TestKernel::default().await;
    let req_client = reqwest::Client::new();

    // When doing a request without a token
    let resp = req_client
        .get(format!("http://127.0.0.1:{}/v1/skills", kernel.port()))
        .send()
        .await
        .unwrap();

    // Then we get a 401
    assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
    let body = resp.text().await.unwrap();
    assert_eq!(body, "Bearer token expected");
    kernel.shutdown().await;
}

#[tokio::test]
async fn token_is_checked_if_auth_url_is_set() {
    // Given a Kernel with authorization enabled
    let kernel = TestKernel::default().await;
    let req_client = reqwest::Client::new();

    // When doing a request with a bad token
    let bad_token = "bad-token";
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {bad_token}")).unwrap();
    auth_value.set_sensitive(true);
    let resp = req_client
        .get(format!("http://127.0.0.1:{}/v1/skills", kernel.port()))
        .header(header::AUTHORIZATION, auth_value)
        .send()
        .await
        .unwrap();

    // Then we get a 401
    assert_eq!(resp.status(), axum::http::StatusCode::FORBIDDEN);
    let body = resp.text().await.unwrap();
    assert_eq!(body, "Bearer token invalid");

    kernel.shutdown().await;
}

#[tokio::test]
async fn no_token_required_if_auth_url_is_not_set() {
    // Given a Kernel with authorization disabled
    let kernel = TestKernel::without_authorization().await;
    let req_client = reqwest::Client::new();

    // When doing a request without a token
    let resp = req_client
        .get(format!("http://127.0.0.1:{}/v1/skills", kernel.port()))
        .send()
        .await
        .unwrap();

    // Then we get a 200
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    kernel.shutdown().await;
}

/// Create an authenticated header value for the API token
fn auth_value() -> header::HeaderValue {
    let api_token = api_token();
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
    auth_value.set_sensitive(true);
    auth_value
}

/// API Token used by tests to authenticate requests
fn api_token() -> &'static str {
    static API_TOKEN: OnceLock<String> = OnceLock::new();
    API_TOKEN.get_or_init(|| {
        drop(from_filename(".env.test"));
        env::var("PHARIA_AI_TOKEN").expect("PHARIA_AI_TOKEN variable not set")
    })
}

/// `OpenAI` Inference API Token used by tests to authenticate requests
fn openai_inference_token() -> &'static str {
    static OPENAI_INFERENCE_TOKEN: OnceLock<String> = OnceLock::new();
    OPENAI_INFERENCE_TOKEN.get_or_init(|| {
        drop(from_filename(".env.test"));
        env::var("OPENAI_INFERENCE__TOKEN").expect("OPENAI_INFERENCE__TOKEN variable not set")
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
