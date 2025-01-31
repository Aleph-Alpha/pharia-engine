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
        Ok(Digest(digest))
    }
}

impl SkillRegistry for FileRegistry {
    fn load_skill<'a>(
        &'a self,
        name: &'a str,
        _tag: &'a str,
    ) -> DynFuture<'a, Result<Option<SkillImage>, RegistryError>> {
        let fut = async move {
            let skill_path = self.skill_path(name);
            if skill_path.exists() {
                let binary = fs::read(&skill_path).map_err(anyhow::Error::from)?;
                let digest = Self::skill_digest(&skill_path)?;
                Ok(Some(SkillImage::new(binary, digest)))
            } else {
                Ok(None)
            }
        };
        Box::pin(fut)
    }

    fn fetch_digest<'a>(
        &'a self,
        name: &'a str,
        _tag: &'a str,
    ) -> DynFuture<'a, Result<Option<Digest>, RegistryError>> {
        Box::pin(async move {
            let skill_path = self.skill_path(name);
            if skill_path.exists() {
                Ok(Some(Self::skill_digest(&skill_path)?))
            } else {
                Ok(None)
            }
        })
    }
}
