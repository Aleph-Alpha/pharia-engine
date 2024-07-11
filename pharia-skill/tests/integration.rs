use std::{env, fs::File, io::Write, net::SocketAddr, path::Path, thread::sleep, time::Duration};

use assert_cmd::Command;
use pharia_kernel::{run, AppConfig};
use predicates::str::contains;
use tokio::sync::oneshot::{self};

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

#[tokio::test]
async fn run_skill() {
    // given a Pharia Kernel instance
    let (send, recv) = oneshot::channel();
    let shutdown_signal = async {
        recv.await.unwrap();
    };
    let config = AppConfig::from_env();
    let address = config.tcp_addr;
    let handle = tokio::spawn(async {
        run(config, shutdown_signal).await;
    });
    wait_until_kernel_ready(address).await;

    // when running a skill
    let mut cmd = Command::cargo_bin("pharia-skill").unwrap();
    let cmd = cmd
        .arg("run")
        .arg("-n")
        .arg("greet_skill")
        .arg("-i")
        .arg("Homer")
        .arg("-l")
        .arg(format!("http://{address}"))
        .env(
            "AA_API_TOKEN",
            env::var("AA_API_TOKEN").expect("AA_API_TOKEN must be set."),
        );

    // then the output must contain the expected value
    cmd.assert().stdout(contains("Homer"));

    send.send(()).unwrap();
    handle.await.unwrap();
}

async fn wait_until_kernel_ready(address: SocketAddr) {
    let url = format!("http://{address}/healthcheck");
    for _ in 0..10 {
        if let Ok(resp) = reqwest::get(&url).await {
            if let Ok(body) = resp.text().await {
                if "ok".eq(&body) {
                    return;
                }
            }
        }
        sleep(Duration::from_secs(1));
    }
    panic!("Kernel is not ready after waiting for 10 seconds.")
}
