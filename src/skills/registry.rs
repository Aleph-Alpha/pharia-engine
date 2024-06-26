use anyhow::Error;
use oci_distribution::{client::ClientConfig, secrets::RegistryAuth, Client, Reference};
use std::{env, future::Future, path::PathBuf, pin::Pin};
use wasmtime::{component::Component, Engine};

pub struct CombinedRegistry {
    file_registry: FileRegistry,
}

impl CombinedRegistry {
    pub fn new() -> Self {
        Self {
            file_registry: FileRegistry::new(),
        }
    }
}

impl SkillRegistry for CombinedRegistry {
    fn load_skill<'a>(
        &'a self,
        name: &'a str,
        engine: &'a Engine,
    ) -> Pin<Box<dyn Future<Output = Result<Component, Error>> + Send + 'a>> {
        self.file_registry.load_skill(name, engine)
    }
}

pub trait SkillRegistry {
    fn load_skill<'a>(
        &'a self,
        name: &'a str,
        engine: &'a Engine,
    ) -> Pin<Box<dyn Future<Output = Result<Component, Error>> + Send + 'a>>;
}

pub struct FileRegistry {
    skill_dir: PathBuf,
}

impl FileRegistry {
    pub fn new() -> Self {
        Self::with_dir("./skills")
    }

    pub fn with_dir(skill_dir: impl Into<PathBuf>) -> Self {
        FileRegistry {
            skill_dir: skill_dir.into(),
        }
    }
}

impl SkillRegistry for FileRegistry {
    fn load_skill<'a>(
        &'a self,
        name: &'a str,
        engine: &'a Engine,
    ) -> Pin<Box<dyn Future<Output = Result<Component, Error>> + Send + 'a>> {
        let fut = async move {
            let mut skill_path = self.skill_dir.join(name);
            skill_path.set_extension("wasm");
            Component::from_file(engine, skill_path)
        };
        Box::pin(fut)
    }
}

pub struct OciRegistry {}

impl SkillRegistry for OciRegistry {
    fn load_skill<'a>(
        &'a self,
        name: &'a str,
        engine: &'a Engine,
    ) -> Pin<Box<dyn Future<Output = Result<Component, Error>> + Send + 'a>> {
        let protocol = oci_distribution::client::ClientProtocol::Https;
        let config = ClientConfig {
            protocol,
            ..Default::default()
        };
        let client = Client::new(config);

        let registry = "registry.gitlab.aleph-alpha.de".to_owned();
        let tag = "v1".to_owned();
        let reference = Reference::with_tag(
            registry,
            format!("engineering/pharia-kernel/skills/{name}"),
            tag,
        );
        dbg!(reference.whole());

        let username =
            env::var("SKILL_REGISTRY_USER").expect("SKILL_REGISTRY_USER variable not set");
        let password =
            env::var("SKILL_REGISTRY_PASSWORD").expect("SKILL_REGISTRY_PASSWORD variable not set");

        let auth = RegistryAuth::Basic(username, password);

        Box::pin(async move {
            let image = client
                .pull(
                    &reference,
                    &auth,
                    vec!["application/vnd.wasm.content.layer.v1+wasm"],
                )
                .await?;
            let binary = &image.layers.first().unwrap().data;
            Component::from_binary(engine, binary)
        })
    }
}

#[cfg(test)]
mod tests {
    use wasmtime::{Config, Engine};

    use super::{OciRegistry, SkillRegistry};

    #[tokio::test]
    async fn oci_skill_is_loaded() {
        drop(dotenvy::dotenv());
        let registry = OciRegistry {};
        let engine = Engine::new(Config::new().async_support(true).wasm_component_model(true))
            .expect("config must be valid");
        let component = registry.load_skill("greet-py", &engine).await;

        assert!(component.is_ok());
    }
}
