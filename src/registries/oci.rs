#[cfg(test)]
mod tests {
    use crate::registries::SkillRegistry;
    use anyhow::{Error, Result};
    use oci_distribution::{secrets::RegistryAuth, Reference};
    use oci_wasm::WasmClient;
    use std::{env, future::Future, pin::Pin};
    use wasmtime::{component::Component, Engine};

    use std::path::Path;

    use oci_distribution::{client::ClientConfig, Client};
    use oci_wasm::WasmConfig;
    use wasmtime::Config;

    pub struct OciRegistry {
        client: WasmClient,
        registry: String,
        repository: String,
    }

    impl SkillRegistry for OciRegistry {
        fn load_skill<'a>(
            &'a self,
            name: &'a str,
            engine: &'a Engine,
        ) -> Pin<Box<dyn Future<Output = Result<Option<Component>, Error>> + Send + 'a>> {
            let repository = format!("{}/{name}", self.repository);
            let tag = "latest";
            let image = Reference::with_tag(self.registry.clone(), repository, tag.to_owned());

            let username =
                env::var("SKILL_REGISTRY_USER").expect("SKILL_REGISTRY_USER variable not set");
            let password = env::var("SKILL_REGISTRY_PASSWORD")
                .expect("SKILL_REGISTRY_PASSWORD variable not set");
            let auth = RegistryAuth::Basic(username, password);

            Box::pin(async move {
                // TODO: return None if skill not found
                let image = self.client.pull(&image, &auth).await?;
                let binary = &image.layers.first().unwrap().data;
                let result = Component::from_binary(engine, binary);
                Some(result).transpose()
            })
        }
    }

    impl OciRegistry {
        fn new(repository: String, registry: String) -> Self {
            let client = Client::new(ClientConfig::default());
            let client = WasmClient::new(client);

            Self {
                client,
                registry,
                repository,
            }
        }
        fn from_env() -> Option<Self> {
            let maybe_repository = env::var("SKILL_REPOSITORY");
            let maybe_registry = env::var("SKILL_REGISTRY");
            match (maybe_repository, maybe_registry) {
                (Ok(repository), Ok(registry)) => Some(OciRegistry::new(repository, registry)),
                _ => None,
            }
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
        let registry =
            OciRegistry::from_env().expect("Please configure registry, see .env.example");
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
}
