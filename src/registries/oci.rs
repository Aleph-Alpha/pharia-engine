use oci_client::{
    Client, Reference,
    client::ClientConfig,
    errors::{OciDistributionError, OciErrorCode},
    secrets::RegistryAuth,
};
use oci_wasm::WasmClient;
use tracing::{error, warn};

use super::{Digest, DynFuture, RegistryError, SkillImage};
use crate::{logging::TracingContext, registries::SkillRegistry};

pub struct OciRegistry {
    client: WasmClient,
    registry: String,
    base_repository: Option<String>,
    username: String,
    password: String,
}

impl OciRegistry {
    pub fn new(
        registry: String,
        base_repository: Option<String>,
        username: String,
        password: String,
    ) -> Self {
        let client = Client::new(ClientConfig::default());
        let client = WasmClient::new(client);

        Self {
            client,
            registry,
            base_repository: base_repository.map(|s| s.trim_matches('/').to_owned()),
            username,
            password,
        }
    }

    fn auth(&self) -> RegistryAuth {
        RegistryAuth::Basic(self.username.clone(), self.password.clone())
    }

    fn reference(&self, name: &str, tag: impl Into<String>) -> Reference {
        let repository = if let Some(base_repository) = &self.base_repository
            && !base_repository.is_empty()
        {
            format!("{base_repository}/{name}")
        } else {
            name.to_owned()
        };
        Reference::with_tag(self.registry.clone(), repository, tag.into())
    }
}

impl SkillRegistry for OciRegistry {
    fn load_skill<'a>(
        &'a self,
        name: &'a str,
        tag: &'a str,
        tracing_context: TracingContext,
    ) -> DynFuture<'a, Result<Option<SkillImage>, RegistryError>> {
        let image = self.reference(name, tag);

        Box::pin(async move {
            // We want to match on the specific type of result.
            // If it is not found, return None, if it is a connection error, return an error
            let result = self.client.pull(&image, &self.auth()).await;
            match result {
                Ok(image) => {
                    let binary = image.layers.into_iter().next().unwrap().data;
                    let digest = if let Some(digest) = image.digest {
                        Digest::new(digest)
                    } else {
                        warn!(parent: tracing_context.span(), "Registry doesn't return digests. Fetching manually.");
                        self.fetch_digest(name, tag).await?.ok_or_else(|| {
                            RegistryError::DigestShouldExist {
                                name: name.to_owned(),
                                tag: tag.to_owned(),
                                registry: self.registry.clone(),
                            }
                        })?
                    };
                    Ok(Some(SkillImage::new(binary, digest)))
                }
                // We want to distinguish between a skill that is not there and runtime errors
                Err(e) => {
                    if anyhow_is_skill_not_found(&e) {
                        Ok(None)
                    } else {
                        error!(parent: tracing_context.span(), "Error retrieving skill from registry: {e:#}");
                        Err(RegistryError::SkillRetrievalError(e.to_string()))
                    }
                }
            }
        })
    }

    fn fetch_digest<'a>(
        &'a self,
        name: &'a str,
        tag: &'a str,
    ) -> DynFuture<'a, Result<Option<Digest>, RegistryError>> {
        Box::pin(async move {
            let result = self
                .client
                .fetch_manifest_digest(&self.reference(name, tag), &self.auth())
                .await;
            match result {
                Ok(digest) => Ok(Some(Digest::new(digest))),
                // We want to distinguish between a skill that is not there and runtime errors
                Err(e) => {
                    if is_skill_not_found(&e) {
                        Ok(None)
                    } else {
                        error!("Error retrieving digest from registry:\n{e:?}");
                        Err(RegistryError::DigestRetrievalError(e.to_string()))
                    }
                }
            }
        })
    }
}

fn anyhow_is_skill_not_found(error: &anyhow::Error) -> bool {
    let Some(error) = error.downcast_ref::<OciDistributionError>() else {
        return false;
    };

    is_skill_not_found(error)
}

fn is_skill_not_found(error: &OciDistributionError) -> bool {
    match error {
        OciDistributionError::RegistryError { envelope, .. } => envelope
            .errors
            .iter()
            .any(|e| e.code == OciErrorCode::ManifestUnknown),
        OciDistributionError::ImageManifestNotFoundError(_) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::env;

    use oci_client::{Reference, secrets::RegistryAuth};
    use oci_wasm::WasmConfig;
    use test_skills::given_rust_skill_greet_v0_2;
    use wasmtime::{Config, Engine, component::Component};

    use super::OciRegistry;
    use crate::tests::load_env;

    use crate::{logging::TracingContext, registries::SkillRegistry};

    impl OciRegistry {
        fn from_env() -> Option<Self> {
            load_env();
            let maybe_registry = env::var("TEST_REGISTRY");
            let maybe_repository = env::var("TEST_BASE_REPOSITORY");
            let maybe_username = env::var("TEST_REGISTRY_USER");
            let maybe_password = env::var("TEST_REGISTRY_PASSWORD");
            match (
                maybe_registry,
                maybe_repository,
                maybe_username,
                maybe_password,
            ) {
                (Ok(registry), Ok(repository), Ok(username), Ok(password)) => Some(
                    OciRegistry::new(registry, Some(repository), username, password),
                ),
                _ => None,
            }
        }

        async fn store_skill(&self, wasm_bytes: Vec<u8>, skill_name: &str, tag: &str) {
            let repository = if let Some(base_repository) = &self.base_repository
                && !base_repository.is_empty()
            {
                format!("{base_repository}/{skill_name}")
            } else {
                skill_name.to_owned()
            };
            let image = Reference::with_tag(self.registry.clone(), repository, tag.to_owned());
            let (config, component_layer) =
                WasmConfig::from_raw_component(wasm_bytes, None).expect("component must be valid");

            let username =
                env::var("TEST_REGISTRY_USER").expect("TEST_REGISTRY_USER variable not set");
            let password = env::var("TEST_REGISTRY_PASSWORD")
                .expect("TEST_REGISTRY_PASSWORD variable not set");
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
        let test_skill = given_rust_skill_greet_v0_2();
        let registry = OciRegistry::from_env().expect("Please configure registry, see .env.test");
        let tag = "latest";
        registry
            .store_skill(test_skill.bytes(), "greet_skill", tag)
            .await;

        // when pulled from registry
        let engine = Engine::new(Config::new().async_support(true).wasm_component_model(true))
            .expect("config must be valid");
        let skill = registry
            .load_skill("greet_skill", tag, TracingContext::dummy())
            .await
            .expect("must return okay")
            .expect("component binaries must be found");
        let component = Component::new(&engine, skill.bytes);
        let digest = registry
            .fetch_digest("greet_skill", tag)
            .await
            .unwrap()
            .unwrap();

        // then skill can be loaded
        assert!(component.is_ok());
        // Make sure the digest is loaded properly
        assert_eq!(digest, skill.digest);
    }

    #[tokio::test]
    async fn skill_not_found() {
        // given a OCI registry is available
        let registry = OciRegistry::from_env().unwrap();

        // when loading a skill that does not exist
        let bytes = registry
            .load_skill(
                "not-existing-skill",
                "not-existing-tag",
                TracingContext::dummy(),
            )
            .await
            .unwrap();

        // then skill can not be found
        assert!(bytes.is_none());
    }

    #[tokio::test]
    async fn digest_not_found() {
        // given a OCI registry is available
        let registry = OciRegistry::from_env().unwrap();

        // when loading a skill that does not exist
        let digest = registry
            .fetch_digest("not-existing-skill", "not-existing-tag")
            .await
            .unwrap();

        // then skill can not be found
        assert!(digest.is_none());
    }

    #[tokio::test]
    async fn oci_registry_not_available() {
        // given a OCI registry is not available at localhost:6000
        let registry = OciRegistry::new(
            "127.0.0.1:6000".to_owned(),
            Some("skills".to_owned()),
            "dummy-user".to_owned(),
            "dummy-password".to_owned(),
        );

        // when loading a skill that does not exist
        let bytes = registry
            .load_skill(
                "not-existing-skill",
                "not-existing-tag",
                TracingContext::dummy(),
            )
            .await;

        // then skill can not be found
        assert!(bytes.is_err());
    }

    #[test]
    fn oci_registry_sanitizes_base_repository_input() {
        let registry = OciRegistry::new(
            "127.0.0.1:6000".to_owned(),
            Some("/skills/".to_owned()),
            "dummy-user".to_owned(),
            "dummy-password".to_owned(),
        );

        assert_eq!(registry.base_repository.unwrap(), "skills");
    }

    #[test]
    fn oci_registry_handles_empty_base_repository() {
        let skill_name = "skill";
        let registry = OciRegistry::new(
            "127.0.0.1:6000".to_owned(),
            Some(String::new()),
            "dummy-user".to_owned(),
            "dummy-password".to_owned(),
        );

        let reference = registry.reference(skill_name, "tag");

        assert_eq!(reference.repository(), skill_name);
    }

    #[test]
    fn oci_registry_without_base_repository() {
        let skill_name = "skill";
        let registry = OciRegistry::new(
            "127.0.0.1:6000".to_owned(),
            None,
            "dummy-user".to_owned(),
            "dummy-password".to_owned(),
        );

        let reference = registry.reference(skill_name, "tag");

        assert_eq!(reference.repository(), skill_name);
    }
}
