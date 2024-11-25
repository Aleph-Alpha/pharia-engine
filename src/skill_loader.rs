use std::sync::Arc;

use anyhow::{anyhow, Context};
use tokio::task::spawn_blocking;

use crate::registries::{SkillImage, SkillRegistry};
use crate::skills::{Engine, Skill, SkillPath};

use std::collections::HashMap;

/// Owns access to the registries and provides ready-to-use skills and skill digests.
pub struct SkillLoader {
    engine: Arc<Engine>,
    registries: HashMap<String, Box<dyn SkillRegistry + Send + Sync>>,
}

impl SkillLoader {
    pub fn new(
        engine: Arc<Engine>,
        registries: HashMap<String, Box<dyn SkillRegistry + Send + Sync>>,
    ) -> Self {
        Self { engine, registries }
    }

    /// Load a skill from the registry and build it to a `Skill`
    pub async fn fetch(
        &self,
        skill_path: &SkillPath,
        tag: &str,
    ) -> anyhow::Result<(Skill, String)> {
        let registry = self
            .registries
            .get(&skill_path.namespace)
            .expect("If skill exists, so must the namespace it resides in.");

        let skill_bytes = registry.load_skill(&skill_path.name, tag).await?;
        let SkillImage { bytes, digest } = skill_bytes
            .ok_or_else(|| anyhow!("Skill {skill_path} configured but not loadable."))?;
        let engine = self.engine.clone();
        let skill = spawn_blocking(move || Skill::new(engine.as_ref(), bytes))
            .await
            .expect("Spawned linking thread must run to completion without being poisoned.")
            .with_context(|| format!("Failed to initialize {skill_path}."))?;
        Ok((skill, digest))
    }

    /// Fetch the digest for a skill from the registry
    pub async fn fetch_digest(
        &self,
        skill_path: &SkillPath,
        tag: &str,
    ) -> anyhow::Result<Option<String>> {
        let registry = self
            .registries
            .get(&skill_path.namespace)
            .expect("If skill exists, so must the namespace it resides in.");
        registry.fetch_digest(&skill_path.name, tag).await
    }
}
