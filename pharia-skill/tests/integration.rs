use std::{env, fs::File, io::Write, path::Path};

use assert_cmd::Command;
use pharia_kernel::{run, AppConfig, OperatorConfig};
use predicates::str::contains;
use tokio::{sync::oneshot, task::JoinHandle};

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
    const PORT: u16 = 9_000;
    let kernel = Kernel::with_port(PORT).await;

    // when running a skill
    let mut cmd = Command::cargo_bin("pharia-skill").unwrap();
    let cmd = cmd
        .arg("run")
        .arg("-n")
        .arg("greet_skill")
        .arg("-i")
        .arg("Homer")
        .arg("-l")
        .arg(format!("http://127.0.0.1:{PORT}"))
        .env(
            "AA_API_TOKEN",
            env::var("AA_API_TOKEN").expect("AA_API_TOKEN must be set."),
        );

    // then the output must contain the expected value
    cmd.assert().stdout(contains("Homer"));

    kernel.shutdown().await;
}

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
            operator_config: OperatorConfig::from_file("../config.toml")
                .expect("Configuration must be valid."),
        };
        Self::new(app_config).await
    }

    async fn shutdown(self) {
        self.shutdown_trigger.send(()).unwrap();
        self.handle.await.unwrap();
    }
}
