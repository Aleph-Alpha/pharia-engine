use std::{env, sync::OnceLock, time::Duration};

use axum::http;
use dotenvy::dotenv;
use pharia_kernel::{AppConfig, Kernel, OperatorConfig};
use reqwest::{header, Body};
use serde_json::json;
use test_skills::given_greet_skill;
use tokio::{sync::oneshot, task::JoinHandle};

struct TestKernel {
    shutdown_trigger: oneshot::Sender<()>,
    handle: JoinHandle<()>,
}

impl TestKernel {
    async fn new(app_config: AppConfig) -> Self {
        let (shutdown_trigger, shutdown_capture) = oneshot::channel::<()>();
        let shutdown_signal = async {
            shutdown_capture.await.unwrap();
        };
        // Wait for socket listener to be bound
        let kernel = Kernel::new(app_config, shutdown_signal).await.unwrap();
        let handle = tokio::spawn(async move { kernel.run().await });
        Self {
            shutdown_trigger,
            handle,
        }
    }

    async fn with_port(port: u16, skills: &[&str]) -> Self {
        let app_config = AppConfig {
            tcp_addr: format!("127.0.0.1:{port}").parse().unwrap(),
            inference_addr: "https://api.aleph-alpha.com".to_owned(),
            operator_config: OperatorConfig::local(skills),
            namespace_update_interval: Duration::from_secs(10),
            log_level: "info".to_owned(),
            open_telemetry_endpoint: None,
        };
        Self::new(app_config).await
    }

    async fn shutdown(self) {
        self.shutdown_trigger.send(()).unwrap();
        self.handle.await.unwrap();
    }
}

#[cfg_attr(not(feature = "test_inference"), ignore)]
#[tokio::test]
async fn execute_skill() {
    const PORT: u16 = 9_000;

    given_greet_skill();
    let kernel = TestKernel::with_port(PORT, &["greet_skill"]).await;

    let api_token = api_token();
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
    auth_value.set_sensitive(true);
    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!("http://127.0.0.1:{PORT}/execute_skill"))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value)
        .body(Body::from(
            json!({ "skill": "local/greet_skill", "input": "Homer"}).to_string(),
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

/// API Token used by tests to authenticate requests
fn api_token() -> &'static str {
    static API_TOKEN: OnceLock<String> = OnceLock::new();
    API_TOKEN.get_or_init(|| {
        drop(dotenv());
        env::var("AA_API_TOKEN").expect("AA_API_TOKEN variable not set")
    })
}
