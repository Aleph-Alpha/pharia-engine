use super::{DynFuture, SkillImage, SkillRegistry};

use std::{fs, path::PathBuf};

pub struct FileRegistry {
    skill_dir: PathBuf,
}

impl FileRegistry {
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
        _tag: &'a str,
    ) -> DynFuture<'a, anyhow::Result<Option<SkillImage>>> {
        let fut = async move {
            let mut skill_path = self.skill_dir.join(name);
            skill_path.set_extension("wasm");
            let maybe_binary = if skill_path.exists() {
                let binary = fs::read(skill_path)?;
                Some(SkillImage::new(binary))
            } else {
                None
            };
            Ok(maybe_binary)
        };
        Box::pin(fut)
    }
}
