use anyhow::{Error, Result};
use oci_distribution::{secrets::RegistryAuth, Reference};
use oci_wasm::WasmClient;
use std::{env, future::Future, path::PathBuf, pin::Pin};
use wasmtime::{component::Component, Engine};

pub trait SkillRegistry {
    fn load_skill<'a>(
        &'a self,
        name: &'a str,
        engine: &'a Engine,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Component>, Error>> + Send + 'a>>;
}

impl SkillRegistry for Box<dyn SkillRegistry> {
    fn load_skill<'a>(
        &'a self,
        name: &'a str,
        engine: &'a Engine,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Component>, Error>> + Send + 'a>> {
        self.as_ref().load_skill(name, engine)
    }
}

impl<R: SkillRegistry> SkillRegistry for Vec<R> {
    fn load_skill<'a>(
        &'a self,
        name: &'a str,
        engine: &'a Engine,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Component>, Error>> + Send + 'a>> {
        let futures: Vec<_> = self.iter().map(|r| r.load_skill(name, engine)).collect();
        Box::pin(async move {
            for fut in futures {
                if let Ok(Some(component)) = fut.await {
                    return Ok(Some(component));
                }
            }

            Ok(None)
        })
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
    ) -> Pin<Box<dyn Future<Output = Result<Option<Component>, Error>> + Send + 'a>> {
        let fut = async move {
            let mut skill_path = self.skill_dir.join(name);
            skill_path.set_extension("wasm");
            if skill_path.exists() {
                let result = Component::from_file(engine, skill_path);
                Some(result).transpose()
            } else {
                Ok(None)
            }
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
    ) -> Pin<Box<dyn Future<Output = Result<Option<Component>, Error>> + Send + 'a>> {
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
            // TODO: return None if skill not found
            let image = self.client.pull(&image, &auth).await?;
            let binary = &image.layers.first().unwrap().data;
            let result = Component::from_binary(engine, binary);
            Some(result).transpose()
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{env, future::Future, path::Path, pin::Pin};

    use anyhow::Error;
    use oci_distribution::{client::ClientConfig, secrets::RegistryAuth, Client, Reference};
    use oci_wasm::{WasmClient, WasmConfig};
    use tempfile::tempdir;
    use wasmtime::{component::Component, Config, Engine};

    use super::{FileRegistry, OciRegistry, SkillRegistry};

    struct NoneRegistry;

    impl SkillRegistry for NoneRegistry {
        fn load_skill<'a>(
            &'a self,
            name: &'a str,
            engine: &'a Engine,
        ) -> Pin<Box<dyn Future<Output = Result<Option<Component>, Error>> + Send + 'a>> {
            Box::pin(async { Ok(None) })
        }
    }

    struct SomeRegistry {
        component: Component,
    }

    impl SomeRegistry {
        fn new(component: Component) -> Self {
            Self { component }
        }
    }

    impl SkillRegistry for SomeRegistry {
        fn load_skill<'a>(
            &'a self,
            name: &'a str,
            engine: &'a Engine,
        ) -> Pin<Box<dyn Future<Output = Result<Option<Component>, Error>> + Send + 'a>> {
            Box::pin(async move { Ok(Some(self.component.clone())) })
        }
    }

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
    async fn empty_file_registry() {
        let skill_dir = tempdir().unwrap();
        let registry = FileRegistry::with_dir(skill_dir.path());
        let engine = Engine::new(Config::new().async_support(true).wasm_component_model(true))
            .expect("config must be valid");
        let result = registry.load_skill("dummy skill name", &engine).await;
        let component = result.unwrap();
        assert!(component.is_none());
    }

    #[tokio::test]
    async fn empty_skill_registries() {
        let registries = Vec::<NoneRegistry>::new();
        let engine = Engine::new(Config::new().async_support(true).wasm_component_model(true))
            .expect("config must be valid");
        let result = registries.load_skill("dummy skill name", &engine).await;
        let component = result.unwrap();
        assert!(component.is_none());
    }

    #[tokio::test]
    async fn two_empty_registries() {
        let registries = vec![NoneRegistry {}, NoneRegistry {}];
        let engine = Engine::new(Config::new().async_support(true).wasm_component_model(true))
            .expect("config must be valid");
        let result = registries.load_skill("dummy skill name", &engine).await;
        let component = result.unwrap();
        assert!(component.is_none());
    }

    #[tokio::test]
    async fn one_none_one_some_registries() {
        // given
        let engine = Engine::new(Config::new().async_support(true).wasm_component_model(true))
            .expect("config must be valid");
        let component = Component::new(&engine, &r#"(component)"#).unwrap();

        // when
        let registries: Vec<Box<dyn SkillRegistry>> = vec![
            Box::new(NoneRegistry {}),
            Box::new(SomeRegistry::new(component)),
        ];
        let result = registries.load_skill("dummy skill name", &engine).await;
        let component = result.unwrap();

        // then
        assert!(component.is_some());
    }

    #[tokio::test]
    async fn one_some_one_none_registries() {
        // given
        let engine = Engine::new(Config::new().async_support(true).wasm_component_model(true))
            .expect("config must be valid");
        let component = Component::new(&engine, &r#"(component)"#).unwrap();

        // when
        let registries: Vec<Box<dyn SkillRegistry>> = vec![
            Box::new(SomeRegistry::new(component)),
            Box::new(NoneRegistry {}),
        ];
        let result = registries.load_skill("dummy skill name", &engine).await;
        let component = result.unwrap();

        // then
        assert!(component.is_some());
    }
}
