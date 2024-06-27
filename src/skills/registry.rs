use anyhow::{anyhow, bail, Error, Result};
use oci_distribution::{secrets::RegistryAuth, Reference};
use oci_wasm::WasmClient;
use std::{env, future::Future, path::PathBuf, pin::Pin};
use wasmtime::{component::Component, Engine};

pub trait SkillRegistry {
    fn load_skill<'a>(
        &'a self,
        name: &'a str,
        engine: &'a Engine,
    ) -> Pin<Box<dyn Future<Output = Result<Component, Error>> + Send + 'a>>;
}

impl<R> SkillRegistry for Vec<R> {
    fn load_skill<'a>(
        &'a self,
        name: &'a str,
        engine: &'a Engine,
    ) -> Pin<Box<dyn Future<Output = Result<Component, Error>> + Send + 'a>> {
        Box::pin(async move { bail!("skill not found in registry") })
    }
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

pub struct OciRegistry {
    client: WasmClient,
}

impl SkillRegistry for OciRegistry {
    fn load_skill<'a>(
        &'a self,
        name: &'a str,
        engine: &'a Engine,
    ) -> Pin<Box<dyn Future<Output = Result<Component, Error>> + Send + 'a>> {
        let registry = "registry.gitlab.aleph-alpha.de";
        let repository = format!("engineering/pharia-kernel/skills/{name}");
        let tag = "latest";
        let image = Reference::with_tag(registry.to_owned(), repository, tag.to_owned());

        let username =
            env::var("SKILL_REGISTRY_USER").expect("SKILL_REGISTRY_USER variable not set");
        let password =
            env::var("SKILL_REGISTRY_PASSWORD").expect("SKILL_REGISTRY_PASSWORD variable not set");
        let auth = RegistryAuth::Basic(username, password);

        Box::pin(async move {
            let image = self.client.pull(&image, &auth).await?;
            let binary = &image.layers.first().unwrap().data;
            Component::from_binary(engine, binary)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{env, path::Path};

    use oci_distribution::{client::ClientConfig, secrets::RegistryAuth, Client, Reference};
    use oci_wasm::{WasmClient, WasmConfig};
    use wasmtime::{Config, Engine};

    use super::{OciRegistry, SkillRegistry};

    impl OciRegistry {
        fn new() -> Self {
            let client = Client::new(ClientConfig::default());
            let client = WasmClient::new(client);

            Self { client }
        }

        async fn store_skill(&self, path: impl AsRef<Path>, skill_name: &str) {
            let registry = "registry.gitlab.aleph-alpha.de";
            let repository = format!("engineering/pharia-kernel/skills/{skill_name}");
            let tag = "latest";
            let image = Reference::with_tag(registry.to_owned(), repository, tag.to_owned());
            let (config, component_layer) = WasmConfig::from_component(path, None)
                .await
                .expect("component must be valid");

            let username =
                env::var("SKILL_REGISTRY_USER").expect("SKILL_REGISTRY_USER variable not set");
            let password = env::var("SKILL_REGISTRY_PASSWORD")
                .expect("SKILL_REGISTRY_PASSWORD variable not set");
            let auth = RegistryAuth::Basic(username, password);

            self.client
                .push(&image, &auth, component_layer, config, None)
                .await
                .expect("must be able to push component");
        }
    }

    #[tokio::test]
    async fn oci_push_and_pull_skill() {
        // given skill in local directory is pushed to registry
        drop(dotenvy::dotenv());
        let registry = OciRegistry::new();
        registry
            .store_skill("./skills/greet_skill.wasm", "greet_skill")
            .await;

        // when pulled from registry
        let engine = Engine::new(Config::new().async_support(true).wasm_component_model(true))
            .expect("config must be valid");
        let component = registry.load_skill("greet_skill", &engine).await;

        // then skill can be loaded
        assert!(component.is_ok());
    }

    #[tokio::test]
    async fn empty_skill_registries() {
        let registries = Vec::<Box<dyn SkillRegistry>>::new();
        let engine = Engine::new(Config::new().async_support(true).wasm_component_model(true))
            .expect("config must be valid");
        let component = registries.load_skill("dummy skill name", &engine).await;
        assert!(component.is_err());
    }
}
