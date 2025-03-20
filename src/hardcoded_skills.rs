//! Contains hardcoded skills that are available in beta systems for testing.

use anyhow::bail;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::{
    csi::CsiForSkills,
    namespace_watcher::Namespace,
    skills::{AnySkillMetadata, Engine, Skill, SkillPath},
};

/// If the path designates a hardcoded skill, return it.
pub fn hardcoded_skill(path: &SkillPath) -> Option<Arc<dyn Skill>> {
    if path.namespace == Namespace::new("test-beta").unwrap() {
        match path.name.as_str() {
            "hello" => Some(Arc::new(SkillHello)),
            "saboteur" => Some(Arc::new(SkillSaboteur)),
            "tell_me_a_joke" => Some(Arc::new(SkillTellMeAJoke)),
            _ => None,
        }
    } else {
        None
    }
}

pub struct SkillHello;
pub struct SkillSaboteur;
pub struct SkillTellMeAJoke;

#[async_trait]
impl Skill for SkillHello {
    async fn metadata(
        &self,
        _engine: &Engine,
        _ctx: Box<dyn CsiForSkills + Send>,
    ) -> anyhow::Result<AnySkillMetadata> {
        Ok(AnySkillMetadata::V0)
    }

    async fn run_as_function(
        &self,
        _engine: &Engine,
        _ctx: Box<dyn CsiForSkills + Send>,
        _input: Value,
    ) -> anyhow::Result<Value> {
        bail!("I am a dummy Skill")
    }
}

#[async_trait]
impl Skill for SkillSaboteur {
    async fn metadata(
        &self,
        _engine: &Engine,
        _ctx: Box<dyn CsiForSkills + Send>,
    ) -> anyhow::Result<AnySkillMetadata> {
        Ok(AnySkillMetadata::V0)
    }

    async fn run_as_function(
        &self,
        _engine: &Engine,
        _ctx: Box<dyn CsiForSkills + Send>,
        _input: Value,
    ) -> anyhow::Result<Value> {
        bail!("I am a dummy Skill")
    }
}

#[async_trait]
impl Skill for SkillTellMeAJoke {
    async fn metadata(
        &self,
        _engine: &Engine,
        _ctx: Box<dyn CsiForSkills + Send>,
    ) -> anyhow::Result<AnySkillMetadata> {
        bail!("I am a dummy Skill")
    }

    async fn run_as_function(
        &self,
        _engine: &Engine,
        _ctx: Box<dyn CsiForSkills + Send>,
        _input: Value,
    ) -> anyhow::Result<Value> {
        bail!("I am a dummy Skill")
    }
}
