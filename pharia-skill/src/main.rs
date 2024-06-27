use std::path::PathBuf;

use clap::Parser;
use oci_distribution::{client::ClientConfig, secrets::RegistryAuth, Client, Reference};
use oci_wasm::{WasmClient, WasmConfig};

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
        /// The OCI repository the skill will be published to (e.g. 'engineering/pharia-kernel/skills')
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
