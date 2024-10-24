use std::{env, fs::File, io::Write, net::TcpListener, path::Path, time::Duration};

use assert_cmd::Command;
use pharia_kernel::{AppConfig, Kernel, OperatorConfig};
use predicates::str::contains;
use tokio::sync::oneshot;

#[test]
fn invalid_args() {
    let mut cmd = Command::cargo_bin("pharia-skill").unwrap();
    let cmd = cmd
        .arg("publish")
        .arg("-R")
        .arg("dummy-registry")
        .arg("-r")
        .arg("dummy-repo")
        .arg("-u")
        .arg("dummy_user")
        .arg("-p")
        .arg("dummy_pass")
        .arg("dummy.wasm");
    cmd.assert().failure();
}

fn wasm_file() -> &'static Path {
    let path = Path::new("./tests/test-skill.wasm");
    if !path.exists() {
        let mut file = File::create(path).unwrap();
        let content = wat::parse_str("(module)").unwrap();
        file.write_all(&content).unwrap();
    }
    path
}

#[test]
fn publish_minimal_args() {
    drop(dotenvy::dotenv());
    let path = wasm_file();
    let mut cmd = Command::cargo_bin("pharia-skill").unwrap();
    let cmd = cmd
        .arg("publish")
        .arg(path)
        .arg("-t")
        .arg("0.0.1")
        .env(
            "SKILL_REGISTRY",
            env::var("SKILL_REGISTRY").expect("SKILL_REGISTRY must be set."),
        )
        .env(
            "SKILL_REPOSITORY",
            env::var("SKILL_REPOSITORY").expect("SKILL_REPOSITORY must be set."),
        )
        .env(
            "SKILL_REGISTRY_USER",
            env::var("SKILL_REGISTRY_USER").expect("SKILL_REGISTRY_USER must be set."),
        )
        .env(
            "SKILL_REGISTRY_PASSWORD",
            env::var("SKILL_REGISTRY_PASSWORD").expect("SKILL_REGISTRY_PASSWORD must be set."),
        );
    cmd.assert().success();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_skill() {
    // given a Pharia Kernel instance
    let kernel = TestKernel::with_defaults().await;

    // when running a skill
    drop(dotenvy::dotenv());
    let mut cmd = Command::cargo_bin("pharia-skill").unwrap();
    let cmd = cmd
        .arg("run")
        .arg("-n")
        .arg("local/greet_skill")
        .arg("-i")
        .arg("Homer")
        .arg("-l")
        .arg(format!("http://127.0.0.1:{}", kernel.port()))
        .env(
            "AA_API_TOKEN",
            env::var("AA_API_TOKEN").expect("AA_API_TOKEN must be set."),
        );

    // then the output must contain the expected value
    cmd.assert().stdout(contains("Homer"));

    kernel.shutdown().await;
}

struct TestKernel {
    kernel: Kernel,
    shutdown_trigger: oneshot::Sender<()>,
    port: u16,
}

impl TestKernel {
    async fn new(app_config: AppConfig) -> Self {
        let (shutdown_trigger, shutdown_capture) = oneshot::channel::<()>();
        let shutdown_signal = async {
            shutdown_capture.await.unwrap();
        };
        let port = app_config.tcp_addr.port();
        let kernel = Kernel::new(app_config, shutdown_signal).await.unwrap();
        Self {
            kernel,
            shutdown_trigger,
            port,
        }
    }

    async fn with_defaults() -> Self {
        let port = free_test_port();
        let app_config = AppConfig {
            tcp_addr: format!("127.0.0.1:{port}").parse().unwrap(),
            inference_addr: "https://api.aleph-alpha.com".to_owned(),
            document_index_addr: "https://document-index.aleph-alpha.com".to_owned(),
            operator_config: OperatorConfig::from_toml(
                r#"
                    [namespaces.local]
                    config_url = "file://../namespace.toml"
                    registry = { type = "file", path = "../skills" }
                "#,
            )
            .unwrap(),
            namespace_update_interval: Duration::from_secs(10),
            log_level: "info".to_owned(),
            open_telemetry_endpoint: None,
            use_pooling_allocator: false,
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

/// Ask the operating system for the next free port
fn free_test_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
