use anyhow::{Error, Result};
use futures::{stream::FuturesOrdered, StreamExt};
use oci::OciRegistry;
use std::{future::Future, pin::Pin};
use wasmtime::{component::Component, Engine};

mod file;
mod oci;

pub use self::file::FileRegistry;

pub fn registries() -> Vec<Box<dyn SkillRegistry + Send>> {
    let mut registries: Vec<Box<dyn SkillRegistry + Send>> = vec![Box::new(FileRegistry::new())];
    if let Some(oci_registry) = OciRegistry::from_env() {
        registries.push(Box::new(oci_registry));
    }
    registries
}

pub trait SkillRegistry {
    fn load_skill<'a>(
        &'a self,
        name: &'a str,
        engine: &'a Engine,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Component>, Error>> + Send + 'a>> {
        let fut = self.load_skill_new(name);
        Box::pin(async move {
            let binary = fut.await?;
            if let Some(binary) = binary {
                Some(Component::new(engine, binary)).transpose()
            } else {
                Ok(None)
            }
        })
    }

    /// can return either the binary or WAT text format of a Wasm component
    fn load_skill_new<'a>(
        &'a self,
        _name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Vec<u8>>, Error>> + Send + 'a>> {
        todo!()
    }
}

impl SkillRegistry for Box<dyn SkillRegistry + Send> {
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
        // Collect all the futures into an ordered stream that will run the futures concurrently,
        // but will return the results in the order they were added.
        let mut futures = self
            .iter()
            .map(|r| r.load_skill(name, engine))
            .collect::<FuturesOrdered<_>>();
        Box::pin(async move {
            while let Some(result) = futures.next().await {
                // We found the component. Otherwise, we continue to the next registry.
                // Bubble up any errors we find as well.
                if let Some(component) = result? {
                    return Ok(Some(component));
                }
            }
            // We didn't find it anywhere
            Ok(None)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{future::Future, pin::Pin};

    use anyhow::{anyhow, Error};
    use tempfile::tempdir;
    use wasmtime::{component::Component, Engine};

    use crate::skills::WasmRuntime;

    use super::{FileRegistry, SkillRegistry};

    struct NoneRegistry;

    impl SkillRegistry for NoneRegistry {
        fn load_skill_new<'a>(
            &'a self,
            _name: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<Option<Vec<u8>>, Error>> + Send + 'a>> {
            Box::pin(async { Ok(None) })
        }
    }

    struct SomeRegistry {
        /// this could be binary or WAT
        bytes: Vec<u8>,
    }

    impl SomeRegistry {
        fn new(bytes: Vec<u8>) -> Self {
            Self { bytes }
        }
    }

    impl SkillRegistry for SomeRegistry {
        fn load_skill_new<'a>(
            &'a self,
            _name: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<Option<Vec<u8>>, Error>> + Send + 'a>> {
            Box::pin(async move { Ok(Some(self.bytes.clone())) })
        }
    }

    struct SaboteurRegistry;

    impl SkillRegistry for SaboteurRegistry {
        fn load_skill<'a>(
            &'a self,
            _name: &'a str,
            _engine: &'a Engine,
        ) -> Pin<Box<dyn Future<Output = Result<Option<Component>, Error>> + Send + 'a>> {
            Box::pin(async move { Err(anyhow!("out-of-cheese-error")) })
        }
    }

    #[tokio::test]
    async fn empty_file_registry() {
        let skill_dir = tempdir().unwrap();
        let registry = FileRegistry::with_dir(skill_dir.path());
        let engine = WasmRuntime::engine();
        let result = registry.load_skill("dummy skill name", &engine).await;
        let component = result.unwrap();
        assert!(component.is_none());
    }

    #[tokio::test]
    async fn empty_skill_registries() {
        let registries = Vec::<NoneRegistry>::new();
        let engine = WasmRuntime::engine();
        let result = registries.load_skill("dummy skill name", &engine).await;
        let component = result.unwrap();
        assert!(component.is_none());
    }

    #[tokio::test]
    async fn two_empty_registries() {
        let registries = vec![NoneRegistry {}, NoneRegistry {}];
        let engine = WasmRuntime::engine();
        let result = registries.load_skill("dummy skill name", &engine).await;
        let component = result.unwrap();
        assert!(component.is_none());
    }

    #[tokio::test]
    async fn one_none_one_some_registries() {
        // given
        let engine = WasmRuntime::engine();

        // when
        let registries: Vec<Box<dyn SkillRegistry + Send>> = vec![
            Box::new(NoneRegistry {}),
            Box::new(SomeRegistry::new(b"(component)".to_vec())),
        ];
        let result = registries.load_skill("dummy skill name", &engine).await;
        let component = result.unwrap();

        // then
        assert!(component.is_some());
    }

    #[tokio::test]
    async fn one_some_one_none_registries() {
        // given
        let engine = WasmRuntime::engine();

        // when
        let registries: Vec<Box<dyn SkillRegistry + Send>> = vec![
            Box::new(SomeRegistry::new(b"(component)".to_vec())),
            Box::new(NoneRegistry {}),
        ];
        let result = registries.load_skill("dummy skill name", &engine).await;
        let component = result.unwrap();

        // then
        assert!(component.is_some());
    }

    #[tokio::test]
    async fn first_fails_and_second_succeeds() {
        // given
        let engine = WasmRuntime::engine();
        let registries: Vec<Box<dyn SkillRegistry + Send>> = vec![
            Box::new(SaboteurRegistry),
            Box::new(SomeRegistry::new(b"(component)".to_vec())),
        ];

        // when
        let result = registries.load_skill("dummy skill name", &engine).await;

        // then
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn second_one_fails() {
        let engine = WasmRuntime::engine();
        let registries: Vec<Box<dyn SkillRegistry + Send>> = vec![
            Box::new(SomeRegistry::new(b"(component)".to_vec())),
            Box::new(SaboteurRegistry),
        ];

        // when
        let result = registries.load_skill("dummy skill name", &engine).await;

        // then
        assert!(result.is_ok());
    }
}
