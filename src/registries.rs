use std::{future::Future, pin::Pin};

mod file;
mod oci;

pub use file::FileRegistry;
pub use oci::OciRegistry;

type DynFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Used to check if a skill image has changed
#[derive(Debug, Eq, PartialEq)]
pub struct Digest(String);

/// Contains the bytes necessary to instantiate a Skill, as well as the
/// digest at the time of the pull associated with these bytes.
pub struct SkillImage {
    /// Can be either the binary or WAT text format of a Wasm component
    pub bytes: Vec<u8>,
    /// Digest associated with these bytes from the registry. We can use
    /// this to do a cheaper comparison if the backing bytes have changed.
    pub digest: Digest,
}

impl SkillImage {
    pub fn new(bytes: Vec<u8>, digest: Digest) -> Self {
        Self { bytes, digest }
    }
}

pub trait SkillRegistry {
    fn load_skill<'a>(
        &'a self,
        name: &'a str,
        tag: &'a str,
    ) -> DynFuture<'a, anyhow::Result<Option<SkillImage>>>;

    /// Retrieve the current digest value for the name and tag
    fn fetch_digest<'a>(
        &'a self,
        name: &'a str,
        tag: &'a str,
    ) -> DynFuture<'a, anyhow::Result<Option<Digest>>>;
}

impl SkillRegistry for Box<dyn SkillRegistry + Send + Sync> {
    fn load_skill<'a>(
        &'a self,
        name: &'a str,
        tag: &'a str,
    ) -> DynFuture<'a, anyhow::Result<Option<SkillImage>>> {
        self.as_ref().load_skill(name, tag)
    }

    fn fetch_digest<'a>(
        &'a self,
        name: &'a str,
        tag: &'a str,
    ) -> DynFuture<'a, anyhow::Result<Option<Digest>>> {
        self.as_ref().fetch_digest(name, tag)
    }
}

#[cfg(test)]
pub mod tests {
    use std::collections::HashMap;

    use futures::future::{pending, ready};
    use tempfile::tempdir;

    use crate::registries::FileRegistry;

    use super::{Digest, DynFuture, SkillImage, SkillRegistry};

    impl SkillRegistry for HashMap<String, Vec<u8>> {
        fn load_skill<'a>(
            &'a self,
            name: &'a str,
            tag: &'a str,
        ) -> DynFuture<'a, anyhow::Result<Option<SkillImage>>> {
            if let Some(bytes) = self.get(name) {
                Box::pin(
                    async move { Ok(Some(SkillImage::new(bytes.clone(), Digest(tag.to_owned())))) },
                )
            } else {
                Box::pin(async { Ok(None) })
            }
        }

        fn fetch_digest<'a>(
            &'a self,
            _name: &'a str,
            tag: &'a str,
        ) -> DynFuture<'a, anyhow::Result<Option<Digest>>> {
            Box::pin(async { Ok(Some(Digest(tag.to_owned()))) })
        }
    }

    pub struct NeverResolvingRegistry;

    impl SkillRegistry for NeverResolvingRegistry {
        fn load_skill<'a>(
            &'a self,
            _name: &'a str,
            _tag: &'a str,
        ) -> DynFuture<'a, anyhow::Result<Option<SkillImage>>> {
            Box::pin(pending::<anyhow::Result<Option<SkillImage>>>())
        }
        fn fetch_digest<'a>(
            &'a self,
            _name: &'a str,
            _tag: &'a str,
        ) -> DynFuture<'a, anyhow::Result<Option<Digest>>> {
            Box::pin(pending::<anyhow::Result<Option<Digest>>>())
        }
    }

    pub struct ReadyRegistry;

    impl SkillRegistry for ReadyRegistry {
        fn load_skill<'a>(
            &'a self,
            _name: &'a str,
            _tag: &'a str,
        ) -> DynFuture<'a, anyhow::Result<Option<SkillImage>>> {
            Box::pin(ready(Ok(None)))
        }
        fn fetch_digest<'a>(
            &'a self,
            _name: &'a str,
            _tag: &'a str,
        ) -> DynFuture<'a, anyhow::Result<Option<Digest>>> {
            Box::pin(ready(Ok(None)))
        }
    }

    #[tokio::test]
    async fn empty_file_registry() {
        let skill_dir = tempdir().unwrap();
        let registry = FileRegistry::with_dir(skill_dir.path());
        let result = registry.load_skill("dummy skill name", "dummy tag").await;
        let bytes = result.unwrap();
        assert!(bytes.is_none());
    }

    #[tokio::test]
    async fn empty_skill_registries() {
        let registries = HashMap::new();
        let result = registries.load_skill("dummy skill name", "dummy tag").await;
        let bytes = result.unwrap();
        assert!(bytes.is_none());
    }
}
