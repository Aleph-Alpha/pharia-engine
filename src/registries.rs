use std::{future::Future, pin::Pin};

mod file;
mod oci;

pub use file::FileRegistry;
pub use oci::OciRegistry;

type DynFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Contains the bytes necessary to instantiate a Skill, as well as the
/// digest at the time of the pull associated with these bytes.
pub struct SkillImage {
    /// Can be either the binary or WAT text format of a Wasm component
    pub bytes: Vec<u8>,
    /// Digest associated with these bytes from the registry. We can use
    /// this to do a cheaper comparison if the backing bytes have changed.
    pub digest: String,
}

impl SkillImage {
    pub fn new(bytes: Vec<u8>, digest: impl Into<String>) -> Self {
        Self {
            bytes,
            digest: digest.into(),
        }
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
    ) -> DynFuture<'a, anyhow::Result<Option<String>>>;
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
    ) -> DynFuture<'a, anyhow::Result<Option<String>>> {
        self.as_ref().fetch_digest(name, tag)
    }
}

#[cfg(test)]
pub mod tests {
    use std::collections::HashMap;

    use tempfile::tempdir;

    use crate::registries::FileRegistry;

    use super::{DynFuture, SkillImage, SkillRegistry};

    impl SkillRegistry for HashMap<String, Vec<u8>> {
        fn load_skill<'a>(
            &'a self,
            name: &'a str,
            tag: &'a str,
        ) -> DynFuture<'a, anyhow::Result<Option<SkillImage>>> {
            if let Some(bytes) = self.get(name) {
                Box::pin(async move { Ok(Some(SkillImage::new(bytes.clone(), tag))) })
            } else {
                Box::pin(async { Ok(None) })
            }
        }

        fn fetch_digest<'a>(
            &'a self,
            _name: &'a str,
            tag: &'a str,
        ) -> DynFuture<'a, anyhow::Result<Option<String>>> {
            Box::pin(async { Ok(Some(tag.to_owned())) })
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
