use anyhow::{Error, Result};
use futures::{stream::FuturesOrdered, StreamExt};
use oci::OciRegistry;
use std::{future::Future, pin::Pin};

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

type DynFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait SkillRegistry {
    /// can return either the binary or WAT text format of a Wasm component
    fn load_skill<'a>(&'a self, _name: &'a str) -> DynFuture<'a, Result<Option<Vec<u8>>, Error>>;
}

impl SkillRegistry for Box<dyn SkillRegistry + Send> {
    fn load_skill<'a>(&'a self, name: &'a str) -> DynFuture<'a, Result<Option<Vec<u8>>, Error>> {
        self.as_ref().load_skill(name)
    }
}

impl<R: SkillRegistry> SkillRegistry for Vec<R> {
    fn load_skill<'a>(&'a self, name: &'a str) -> DynFuture<'a, Result<Option<Vec<u8>>, Error>> {
        // Collect all the futures into an ordered stream that will run the futures concurrently,
        // but will return the results in the order they were added.
        let mut futures = self
            .iter()
            .map(|r| r.load_skill(name))
            .collect::<FuturesOrdered<_>>();
        Box::pin(async move {
            while let Some(result) = futures.next().await {
                // We found the component bytes. Otherwise, we continue to the next registry.
                // Bubble up any errors we find as well.
                if let Some(bytes) = result? {
                    return Ok(Some(bytes));
                }
            }
            // We didn't find it anywhere
            Ok(None)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use anyhow::{anyhow, Error};
    use tempfile::tempdir;

    use super::{DynFuture, FileRegistry, SkillRegistry};

    impl SkillRegistry for HashMap<String, Vec<u8>> {
        fn load_skill<'a>(
            &'a self,
            name: &'a str,
        ) -> DynFuture<'a, Result<Option<Vec<u8>>, Error>> {
            if let Some(bytes) = self.get(name) {
                Box::pin(async move { Ok(Some(bytes.clone())) })
            } else {
                Box::pin(async { Ok(None) })
            }
        }
    }

    struct SaboteurRegistry;

    impl SkillRegistry for SaboteurRegistry {
        fn load_skill<'a>(
            &'a self,
            _name: &'a str,
        ) -> DynFuture<'a, Result<Option<Vec<u8>>, Error>> {
            Box::pin(async move { Err(anyhow!("out-of-cheese-error")) })
        }
    }

    #[tokio::test]
    async fn empty_file_registry() {
        let skill_dir = tempdir().unwrap();
        let registry = FileRegistry::with_dir(skill_dir.path());
        let result = registry.load_skill("dummy skill name").await;
        let bytes = result.unwrap();
        assert!(bytes.is_none());
    }

    #[tokio::test]
    async fn empty_skill_registries() {
        let registries = HashMap::new();
        let result = registries.load_skill("dummy skill name").await;
        let bytes = result.unwrap();
        assert!(bytes.is_none());
    }

    #[tokio::test]
    async fn two_empty_registries() {
        let registries = vec![HashMap::new(), HashMap::new()];
        let result = registries.load_skill("dummy skill name").await;
        let bytes = result.unwrap();
        assert!(bytes.is_none());
    }

    #[tokio::test]
    async fn find_skill_in_second_registry() {
        // given
        let registries: Vec<Box<dyn SkillRegistry + Send>> = vec![
            Box::new(HashMap::new()),
            Box::new(HashMap::from([(
                "dummy skill name".to_owned(),
                b"(component)".to_vec(),
            )])),
        ];

        // when
        let result = registries.load_skill("dummy skill name").await;
        let bytes = result.unwrap().unwrap();

        // then
        assert_eq!(bytes, b"(component)");
    }

    #[tokio::test]
    async fn find_skill_in_first_registry() {
        // given
        let registries: Vec<Box<dyn SkillRegistry + Send>> = vec![
            Box::new(HashMap::from([(
                "dummy skill name".to_owned(),
                b"(component)".to_vec(),
            )])),
            Box::new(HashMap::new()),
        ];

        // when
        let result = registries.load_skill("dummy skill name").await;
        let bytes = result.unwrap().unwrap();

        // then
        assert_eq!(bytes, b"(component)");
    }

    #[tokio::test]
    async fn first_fails_and_second_succeeds() {
        // given
        let registries: Vec<Box<dyn SkillRegistry + Send>> = vec![
            Box::new(SaboteurRegistry),
            Box::new(HashMap::from([(
                "dummy skill name".to_owned(),
                b"(component)".to_vec(),
            )])),
        ];

        // when
        let result = registries.load_skill("dummy skill name").await;

        // then
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn second_one_fails() {
        let registries: Vec<Box<dyn SkillRegistry + Send>> = vec![
            Box::new(HashMap::from([(
                "dummy skill name".to_owned(),
                b"(component)".to_vec(),
            )])),
            Box::new(SaboteurRegistry),
        ];

        // when
        let result = registries.load_skill("dummy skill name").await;

        // then
        assert!(result.is_ok());
    }
}
