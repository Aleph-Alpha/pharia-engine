use crate::registries::SkillRegistry;
use oci_client::{
    client::ClientConfig,
    errors::{OciDistributionError, OciErrorCode},
    secrets::RegistryAuth,
    Client, Reference,
};
use oci_wasm::WasmClient;
use tracing::error;

use super::DynFuture;

pub struct OciRegistry {
    client: WasmClient,
    registry: String,
    repository: String,
    username: String,
    password: String,
}

impl SkillRegistry for OciRegistry {
    fn load_skill<'a>(
        &'a self,
        name: &'a str,
        tag: &'a str,
    ) -> DynFuture<'a, anyhow::Result<Option<Vec<u8>>>> {
        let repository = format!("{}/{name}", self.repository);
        let image = Reference::with_tag(self.registry.clone(), repository, tag.to_owned());
        let auth = RegistryAuth::Basic(self.username.clone(), self.password.clone());

        Box::pin(async move {
            // TODO: we want to match on the specific type of result.
            // If it is not found, return None, if it is a connection error, return an error
            let result = self.client.pull(&image, &auth).await;
            match result {
                Ok(image) => {
                    let binary = image.layers.into_iter().next().unwrap().data;
                    Ok(Some(binary))
                }
                // We want to distinguish between a skill that is not there and runtime errors
                Err(e) => {
                    if is_skill_not_found(&e) {
                        Ok(None)
                    } else {
                        error!("Error retrieving skill from registry: {e}");
                        Err(e)
                    }
                }
            }
        })
    }
}

impl OciRegistry {
    pub fn new(repository: String, registry: String, username: String, password: String) -> Self {
        let client = Client::new(ClientConfig::default());
        let client = WasmClient::new(client);

        Self {
            client,
            registry,
            repository,
            username,
            password,
        }
    }
}

fn is_skill_not_found(error: &anyhow::Error) -> bool {
    let Some(error) = error.downcast_ref::<OciDistributionError>() else {
        return false;
    };

    if let OciDistributionError::RegistryError { envelope, .. } = error {
        envelope
            .errors
            .iter()
            .any(|e| e.code == OciErrorCode::ManifestUnknown)
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use std::{env, path::Path};

    use dotenvy::dotenv;
    use oci_client::{secrets::RegistryAuth, Reference};
    use oci_wasm::WasmConfig;
    use wasmtime::{component::Component, Config, Engine};

    use super::OciRegistry;

    use crate::registries::SkillRegistry;

    impl OciRegistry {
        fn from_env() -> Option<Self> {
            let maybe_repository = env::var("SKILL_REPOSITORY");
            let maybe_registry = env::var("SKILL_REGISTRY");
            let maybe_username = env::var("SKILL_REGISTRY_USER");
            let maybe_password = env::var("SKILL_REGISTRY_PASSWORD");
            match (
                maybe_repository,
                maybe_registry,
                maybe_username,
                maybe_password,
            ) {
                (Ok(repository), Ok(registry), Ok(username), Ok(password)) => {
                    Some(OciRegistry::new(repository, registry, username, password))
                }
                _ => None,
            }
        }

        async fn store_skill(&self, path: impl AsRef<Path>, skill_name: &str, tag: &str) {
            let repository = format!("{}/{skill_name}", self.repository);
            let image = Reference::with_tag(self.registry.clone(), repository, tag.to_owned());
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
        let tag = "latest";
        registry
            .store_skill("./skills/greet_skill.wasm", "greet_skill", tag)
            .await;

        // when pulled from registry
        let engine = Engine::new(Config::new().async_support(true).wasm_component_model(true))
            .expect("config must be valid");
        let bytes = registry
            .load_skill("greet_skill", tag)
            .await
            .expect("must return okay")
            .expect("component binaries must be found");
        let component = Component::new(&engine, bytes);

        // then skill can be loaded
        assert!(component.is_ok());
    }

    #[tokio::test]
    async fn skill_not_found() {
        // given a OCI registry is available
        drop(dotenv());
        let registry = OciRegistry::from_env().unwrap();

        // when loading a skill that does not exist
        let bytes = registry
            .load_skill("not-existing-skill", "not-existing-tag")
            .await
            .unwrap();

        // then skill can not be found
        assert!(bytes.is_none());
    }

    #[tokio::test]
    async fn oci_registry_not_available() {
        // given a OCI registry is not available at localhost:6000
        let registry = OciRegistry::new(
            "127.0.0.1:6000".to_owned(),
            "skills".to_owned(),
            "dummy-user".to_owned(),
            "dummy-password".to_owned(),
        );

        // when loading a skill that does not exist
        let bytes = registry
            .load_skill("not-existing-skill", "not-existing-tag")
            .await;

        // then skill can not be found
        assert!(bytes.is_err());
    }
}
