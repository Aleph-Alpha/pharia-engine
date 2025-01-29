use serde_json::Value;
use std::sync::Arc;

use crate::{
    csi::CsiForSkills,
    engine::{Engine, SkillPath},
    skill_store::SkillStoreApi,
    skills::actor::ExecuteSkillError,
};

pub struct WasmRuntime {
    /// Used to execute skills. We will share the engine with multiple running skills, and skill
    /// provider to convert bytes into executable skills.
    engine: Arc<Engine>,
    skill_store_api: SkillStoreApi,
}

impl WasmRuntime {
    pub fn new(engine: Arc<Engine>, skill_store_api: SkillStoreApi) -> Self {
        Self {
            engine,
            skill_store_api,
        }
    }

    pub async fn run(
        &self,
        skill_path: &SkillPath,
        input: Value,
        ctx: Box<dyn CsiForSkills + Send>,
    ) -> Result<Value, ExecuteSkillError> {
        let skill = self
            .skill_store_api
            .fetch(skill_path.to_owned())
            .await
            .map_err(ExecuteSkillError::Other)?;
        // Unwrap Skill, raise error if it is not existing
        let skill = skill.ok_or(ExecuteSkillError::SkillDoesNotExist)?;
        skill
            .run(&self.engine, ctx, input)
            .await
            .map_err(ExecuteSkillError::Other)
    }
}

#[cfg(test)]
pub mod tests {
    use std::{sync::Arc, time::Duration};

    use serde_json::json;
    use test_skills::{given_greet_py_v0_2, given_greet_skill_v0_2};

    use crate::{
        csi::tests::{CsiCompleteStub, CsiCounter, CsiGreetingMock},
        inference::Completion,
        namespace_watcher::Namespace,
        skill_loader::{ConfiguredSkill, SkillLoader},
        skill_store::SkillStore,
    };

    use super::*;

    #[tokio::test]
    async fn greet_skill_component() {
        given_greet_skill_v0_2();
        let skill_path = SkillPath::local("greet_skill_v0_2");
        let skill = ConfiguredSkill::from_path(&skill_path);
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_loader =
            SkillLoader::with_file_registry(engine.clone(), skill_path.namespace.clone()).api();

        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(10));
        skill_store.api().upsert(skill).await;
        let runtime = WasmRuntime::new(engine, skill_store.api());
        let skill_ctx = Box::new(CsiCompleteStub::new(|_| Completion::from_text("Hello")));
        let resp = runtime.run(&skill_path, json!("name"), skill_ctx).await;

        drop(runtime);
        skill_store.wait_for_shutdown().await;

        assert_eq!(resp.unwrap(), "Hello");
    }

    #[tokio::test]
    async fn errors_for_non_existing_skill() {
        let engine = Arc::new(Engine::new(false).unwrap());
        let namespace = Namespace::new("local").unwrap();
        let skill_loader = SkillLoader::with_file_registry(engine.clone(), namespace).api();
        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(10));
        let runtime = WasmRuntime::new(engine, skill_store.api());
        let skill_ctx = Box::new(CsiCompleteStub::new(|_| Completion::from_text("")));
        let resp = runtime
            .run(&SkillPath::dummy(), json!("name"), skill_ctx)
            .await;

        drop(runtime);
        skill_store.wait_for_shutdown().await;

        assert!(resp.is_err());
    }

    #[tokio::test]
    async fn rust_greeting_skill() {
        given_greet_skill_v0_2();
        let skill_ctx = Box::new(CsiGreetingMock);
        let skill_path = SkillPath::local("greet_skill_v0_2");
        let skill = ConfiguredSkill::from_path(&skill_path);
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_loader =
            SkillLoader::with_file_registry(engine.clone(), skill_path.namespace.clone()).api();
        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(10));
        skill_store.api().upsert(skill).await;
        let runtime = WasmRuntime::new(engine, skill_store.api());

        let actual = runtime
            .run(&skill_path, json!("Homer"), skill_ctx)
            .await
            .unwrap();

        drop(runtime);
        skill_store.wait_for_shutdown().await;

        assert_eq!(actual, "Hello Homer");
    }

    #[tokio::test]
    async fn python_greeting_skill() {
        given_greet_py_v0_2();
        let skill_ctx = Box::new(CsiGreetingMock);
        let skill_path = SkillPath::local("greet-py-v0_2");
        let skill = ConfiguredSkill::from_path(&skill_path);
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_loader =
            SkillLoader::with_file_registry(engine.clone(), skill_path.namespace.clone()).api();
        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(10));
        skill_store.api().upsert(skill).await;
        let runtime = WasmRuntime::new(engine, skill_store.api());

        let actual = runtime
            .run(&skill_path, json!("Homer"), skill_ctx)
            .await
            .unwrap();

        drop(runtime);
        skill_store.wait_for_shutdown().await;

        assert_eq!(actual, "Hello Homer");
    }

    #[tokio::test]
    async fn can_call_pre_instantiated_multiple_times() {
        given_greet_skill_v0_2();
        let skill_ctx = Box::new(CsiCounter::new());
        let skill_path = SkillPath::local("greet_skill_v0_2");
        let skill = ConfiguredSkill::from_path(&skill_path);
        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_loader =
            SkillLoader::with_file_registry(engine.clone(), skill_path.namespace.clone()).api();
        let skill_store = SkillStore::new(skill_loader, Duration::from_secs(10));
        skill_store.api().upsert(skill).await;
        let runtime = WasmRuntime::new(engine, skill_store.api());
        for i in 1..10 {
            let resp = runtime
                .run(&skill_path, json!("Homer"), skill_ctx.clone())
                .await
                .unwrap();
            assert_eq!(resp, json!(i.to_string()));
        }

        drop(runtime);
        skill_store.wait_for_shutdown().await;
    }
}
