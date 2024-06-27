use super::SkillRegistry;
use anyhow::{Error, Result};
use std::{future::Future, path::PathBuf, pin::Pin};
use wasmtime::{component::Component, Engine};

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
