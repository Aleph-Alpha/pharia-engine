use crate::logging::TracingContext;

use super::{Digest, DynFuture, RegistryError, SkillImage, SkillRegistry};

use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

pub struct FileRegistry {
    skill_dir: PathBuf,
}

impl FileRegistry {
    pub fn with_dir(skill_dir: impl Into<PathBuf>) -> Self {
        FileRegistry {
            skill_dir: skill_dir.into(),
        }
    }

    fn skill_path(&self, name: &str) -> PathBuf {
        let mut skill_path = self.skill_dir.join(name);
        skill_path.set_extension("wasm");
        skill_path
    }

    fn skill_digest(skill_path: &Path) -> anyhow::Result<Digest> {
        let digest = skill_path
            .metadata()?
            .modified()?
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_millis()
            .to_string();
        Ok(Digest::new(digest))
    }
}

impl SkillRegistry for FileRegistry {
    fn load_skill<'a>(
        &'a self,
        name: &'a str,
        _tag: &'a str,
        _tracing_context: TracingContext,
    ) -> DynFuture<'a, Result<Option<SkillImage>, RegistryError>> {
        Box::pin(async move {
            let skill_path = self.skill_path(name);
            if skill_path.exists() {
                let binary = fs::read(&skill_path)
                    .map_err(|e| RegistryError::SkillRetrievalError(e.to_string()))?;
                let digest = Self::skill_digest(&skill_path)
                    .map_err(|e| RegistryError::DigestRetrievalError(e.to_string()))?;
                Ok(Some(SkillImage::new(binary, digest)))
            } else {
                Ok(None)
            }
        })
    }

    fn fetch_digest<'a>(
        &'a self,
        name: &'a str,
        _tag: &'a str,
    ) -> DynFuture<'a, Result<Option<Digest>, RegistryError>> {
        Box::pin(async move {
            let skill_path = self.skill_path(name);
            if skill_path.exists() {
                Ok(Some(Self::skill_digest(&skill_path).map_err(|e| {
                    RegistryError::DigestRetrievalError(e.to_string())
                })?))
            } else {
                Ok(None)
            }
        })
    }
}

#[cfg(test)]
mod test {
    use std::{fs::File, time::Duration};

    use super::*;
    use tempfile::tempdir;
    use test_skills::given_rust_skill_greet_v0_3;
    use tokio::time::sleep;

    #[tokio::test]
    async fn change_digest_if_file_is_modified() {
        // Given a file `my_skill.wasm` containing a skill in a directory
        let any_skill_bytes = b"DUMMY SKILL BYTES";
        // Any skill bytes do, as long as they are different
        let different_skill_bytes = b"DIFFERENT DUMMY SKILL BYTES";
        let skill_dir = tempdir().unwrap();
        let file_path = skill_dir.path().join("my_skill.wasm");
        fs::write(&file_path, any_skill_bytes).unwrap();
        eprintln!(
            "{:?}",
            File::open(&file_path)
                .unwrap()
                .metadata()
                .unwrap()
                .modified()
                .unwrap()
        );

        // When fetching a digest before and after modifying the file
        let registry = FileRegistry::with_dir(skill_dir.path());
        let original_digest = registry.fetch_digest("my_skill", "latest").await.unwrap();
        // Wait for at least one millisecond before changing the file. Otherwise we might actually
        // get the digest and change the file in under one millisecond, and with some bad timing not
        // see the change, because the digest is rounded to milliseconds.
        //
        // One millisecond should be enough, but seems not te be on my desktop if testing on a
        // Ubuntu in WSL. 2 milliseconds seem to be doing the trick reliably, though.
        sleep(Duration::from_millis(2)).await;
        fs::write(&file_path, different_skill_bytes).unwrap();
        eprintln!(
            "{:?}",
            File::open(file_path)
                .unwrap()
                .metadata()
                .unwrap()
                .modified()
                .unwrap()
        );

        // Not sure why we need this sleep. `fs::write` is flushing, but the metainformation seems
        // to not be updated immediately.
        let new_digest = registry.fetch_digest("my_skill", "latest").await.unwrap();

        // Then the digest should change
        assert_ne!(original_digest, new_digest);
    }

    #[tokio::test]
    async fn load_skill() {
        // Given a file `my_skill.wasm` containing a skill in a directory
        let any_skill_bytes = given_rust_skill_greet_v0_3().bytes();
        let skill_dir = tempdir().unwrap();
        fs::write(skill_dir.path().join("my_skill.wasm"), &any_skill_bytes).unwrap();

        // When loading a skill
        let registry = FileRegistry::with_dir(skill_dir.path());
        let skill_image = registry
            .load_skill("my_skill", "latest", TracingContext::dummy())
            .await
            .unwrap()
            .unwrap();

        // Then the skill bytes are identical with the file contents
        assert_eq!(skill_image.bytes, any_skill_bytes);
    }

    #[tokio::test]
    async fn fetch_digest_yields_same_digest_as_skill_image() {
        // Given a file `my_skill.wasm` containing a skill in a directory
        let any_skill_bytes = given_rust_skill_greet_v0_3().bytes();
        let skill_dir = tempdir().unwrap();
        fs::write(skill_dir.path().join("my_skill.wasm"), &any_skill_bytes).unwrap();

        // When fetching a digest and loading a skill
        let registry = FileRegistry::with_dir(skill_dir.path());
        let skill_image = registry
            .load_skill("my_skill", "latest", TracingContext::dummy())
            .await
            .unwrap()
            .unwrap();
        let digest = registry
            .fetch_digest("my_skill", "latest")
            .await
            .unwrap()
            .unwrap();

        // Then the fetched digest is identical with the one returned from the image
        assert_eq!(skill_image.digest, digest);
    }
}
