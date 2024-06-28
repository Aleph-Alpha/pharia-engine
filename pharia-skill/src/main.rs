use std::path::PathBuf;

use clap::Parser;
use mime::APPLICATION_JSON;
use oci_distribution::{client::ClientConfig, secrets::RegistryAuth, Client, Reference};
use oci_wasm::{WasmClient, WasmConfig};
use reqwest::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::json;

#[derive(Parser)]
#[clap(version)]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Parser)]
enum Command {
    /// Publish a skill to OCI registry
    Publish {
        /// Path to skill .wasm file
        skill: PathBuf,
        /// The OCI registry the skill will be published to (e.g. 'registry.gitlab.aleph-alpha.de')
        #[arg(long, short = 'R', env = "SKILL_REGISTRY")]
        registry: String,
        /// The OCI repository the skill will be published to (e.g. 'engineering/pharia-skills/skills')
        #[arg(long, short = 'r', env = "SKILL_REPOSITORY")]
        repository: String,
        /// Published skill name
        #[arg(required = false, long, short = 'n')]
        name: Option<String>,
        /// Published skill tag
        #[arg(long, short = 't', default_value = "latest")]
        tag: String,
        /// User name for OCI registry
        #[arg(long, short = 'u', env = "SKILL_REGISTRY_USER")]
        username: String,
        /// Password for OCI registry
        #[arg(long, short = 'p', env = "SKILL_REGISTRY_PASSWORD")]
        password: String,
    },
    /// Run a skill via Pharia Kernel
    Run {
        /// The Skill name
        #[arg(long, short = 'n')]
        name: String,
        /// The Skill input
        #[arg(long, short = 'i')]
        input: String,
        /// The API token
        #[arg(long, short = 'a', env = "AA_API_TOKEN")]
        token: String,
        /// The Pharia Kernel instance
        #[arg(
            long,
            short = 'l',
            default_value = "https://pharia-kernel.aleph-alpha-playground.stackit.rocks"
        )]
        url: String,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.cmd {
        Command::Publish {
            skill,
            registry,
            repository,
            name,
            tag,
            username,
            password,
        } => publish(skill, registry, repository, name, tag, username, password).await,
        Command::Run {
            name,
            input,
            token,
            url,
        } => run(name, input, token, url).await,
    }
}

async fn publish(
    skill_path: PathBuf,
    registry: String,
    repository: String,
    skill_name: Option<String>,
    tag: String,
    username: String,
    password: String,
) {
    let skill_name = skill_name.unwrap_or_else(|| {
        skill_path
            .as_path()
            .file_stem()
            .expect("Skill must be a valid file.")
            .to_str()
            .expect("Skill file name must be parsable to UTF-8.")
            .to_owned()
    });
    let repository = format!("{repository}/{skill_name}");
    let image = Reference::with_tag(registry, repository, tag);
    let (config, component_layer) = WasmConfig::from_component(skill_path, None)
        .await
        .expect("Skill must be a valid Wasm component.");

    let auth = RegistryAuth::Basic(username, password);
    let client = Client::new(ClientConfig::default());
    let client = WasmClient::new(client);

    client
        .push(&image, &auth, component_layer, config, None)
        .await
        .expect("Could not publish skill, please check command arguments.");
}

async fn run(name: String, input: String, token: String, url: String) {
    let json_payload = json!({
        "skill": name,
        "input": input
    });

    let mut auth_value = HeaderValue::from_str(&format!("Bearer {token}")).unwrap();
    auth_value.set_sensitive(true);

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{url}/execute_skill"))
        .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
        .header(AUTHORIZATION, auth_value)
        .json(&json_payload)
        .send()
        .await
        .unwrap();

    println!("{}", resp.text().await.unwrap());
}
