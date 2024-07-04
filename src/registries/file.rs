use super::SkillRegistry;
use anyhow::{Error, Result};
use std::{fs, future::Future, path::PathBuf, pin::Pin};

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
    fn load_skill_new<'a>(
        &'a self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Vec<u8>>, Error>> + Send + 'a>> {
        let fut = async move {
            let mut skill_path = self.skill_dir.join(name);
            skill_path.set_extension("wasm");
            let maybe_binary = if skill_path.exists() {
                let binary = fs::read(skill_path)?;
                Some(binary)
            } else {
                None
            };
            Ok(maybe_binary)
        };
        Box::pin(fut)
    }
}
