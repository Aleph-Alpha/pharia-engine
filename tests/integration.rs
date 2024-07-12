use std::{env, sync::OnceLock, time::Duration};

use axum::http;
use dotenvy::dotenv;
use pharia_kernel::{run, AppConfig};
use reqwest::{header, Body};
use serde_json::json;
use tokio::{sync::oneshot, task::JoinHandle};

struct Kernel {
    handle: JoinHandle<()>,
    shutdown_trigger: oneshot::Sender<()>,
}

impl Kernel {
    async fn new(app_config: AppConfig) -> Self {
        let (shutdown_trigger, shutdown_capture) = oneshot::channel::<()>();
        let shutdown_signal = async {
            shutdown_capture.await.unwrap();
        };
        let wait_for_shutdown = run(app_config, shutdown_signal).await;
        let handle = tokio::spawn(wait_for_shutdown);
        Self {
            handle,
            shutdown_trigger,
        }
    }

    async fn with_port(port: u16) -> Self {
        let app_config = AppConfig {
            tcp_addr: format!("127.0.0.1:{port}").parse().unwrap(),
            inference_addr: "https://api.aleph-alpha.com".to_owned(),
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
    let kernel = Kernel::with_port(PORT).await;

    let api_token = api_token();
    let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
    auth_value.set_sensitive(true);
    let req_client = reqwest::Client::new();
    let resp = req_client
        .post(format!("http://127.0.0.1:{PORT}/execute_skill"))
        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
        .header(header::AUTHORIZATION, auth_value)
        .body(Body::from(
            json!({ "skill": "greet_skill", "input": "Homer"}).to_string(),
        ))
        .timeout(Duration::from_secs(10))
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
