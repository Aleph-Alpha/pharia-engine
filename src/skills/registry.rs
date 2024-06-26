use std::path::PathBuf;

use anyhow::Error;
use wasmtime::{component::Component, Engine};

pub trait SkillRegistry {
    fn load_skill(&self, name: &str, engine: &Engine) -> Result<Component, Error>;
}

pub struct FsSkillRegistry {
    skill_dir: PathBuf,
}

impl FsSkillRegistry {
    pub fn new() -> Self {
        Self::with_dir("./skills")
    }

    pub fn with_dir(skill_dir: impl Into<PathBuf>) -> Self {
        FsSkillRegistry {
            skill_dir: skill_dir.into(),
        }
    }
}
impl SkillRegistry for FsSkillRegistry {
    fn load_skill(&self, name: &str, engine: &Engine) -> Result<Component, Error> {
        let mut skill_path = self.skill_dir.join(name);
        skill_path.set_extension("wasm");
        Component::from_file(engine, skill_path)
    }
}
