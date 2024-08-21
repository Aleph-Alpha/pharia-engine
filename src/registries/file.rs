use super::{DynFuture, SkillRegistry};

use anyhow::anyhow;
use std::{fs, path::PathBuf};
use url::Url;

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

    pub fn with_url(raw_url: &str) -> anyhow::Result<Self> {
        let url = Url::parse(raw_url)?;
        match url.scheme() {
            "file" => {
                // remove leading "file://"
                let file_path = &raw_url[7..];
                Ok(Self::with_dir(file_path))
            }
            scheme => Err(anyhow!("Unsupported URL scheme: {scheme}")),
        }
    }
}

impl SkillRegistry for FileRegistry {
    fn load_skill<'a>(&'a self, name: &'a str) -> DynFuture<'a, anyhow::Result<Option<Vec<u8>>>> {
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
