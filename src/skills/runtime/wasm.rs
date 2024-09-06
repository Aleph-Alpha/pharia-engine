use serde_json::Value;
use std::sync::Arc;

use crate::skills::{actor::ExecuteSkillError, SkillPath};

use super::{
    engine::Engine,
    provider::{SkillProvider, SkillProviderApi},
    CsiForSkills, Runtime,
};

pub struct WasmRuntime {
    /// Used to execute skills. We will share the engine with multiple running skills, and skill
    /// provider to convert bytes into executable skills.
    engine: Arc<Engine>,
    provider: SkillProvider,
    skill_provider_api: SkillProviderApi,
}

impl WasmRuntime {
    pub fn with_provider(
        skill_provider: SkillProvider,
        skill_provider_api: SkillProviderApi,
    ) -> Self {
        Self {
            engine: Arc::new(Engine::new().expect("engine creation failed")),
            provider: skill_provider,
            skill_provider_api,
        }
    }
}

impl Runtime for WasmRuntime {
    async fn run(
        &mut self,
        skill_path: &SkillPath,
        input: Value,
        ctx: Box<dyn CsiForSkills + Send>,
    ) -> Result<Value, ExecuteSkillError> {
        let skill = self
            .skill_provider_api
            .fetch(skill_path.to_owned(), self.engine.clone())
            .await
            .map_err(ExecuteSkillError::Other)?;
        // Unwrap Skill, raise error if it is not existing
        let skill = skill.ok_or(ExecuteSkillError::SkillDoesNotExist)?;
        skill
            .run(&self.engine, ctx, input)
            .await
            .map_err(ExecuteSkillError::Other)
    }

    fn upsert_skill(&mut self, skill: SkillPath, tag: Option<String>) {
        self.provider.upsert_skill(&skill, tag);
    }

    fn remove_skill(&mut self, skill: &SkillPath) {
        self.provider.remove_skill(skill);
    }

    fn skills(&self) -> impl Iterator<Item = &SkillPath> {
        self.provider.skills()
    }

    fn invalidate_cached_skill(&mut self, skill: &SkillPath) -> bool {
        self.provider.invalidate(skill)
    }

    fn mark_namespace_as_invalid(&mut self, namespace: String, e: anyhow::Error) {
        self.provider.add_invalid_namespace(namespace, e);
    }

    fn mark_namespace_as_valid(&mut self, namespace: &str) {
        self.provider.remove_invalid_namespace(namespace);
    }
}

#[cfg(test)]
pub mod tests {
    use std::{
        fs,
        sync::{Arc, Mutex},
    };

    use crate::{
        configuration_observer::OperatorConfig,
        csi::ChunkRequest,
        inference::{Completion, CompletionRequest},
        language_selection::{select_language, Language},
        skills::runtime::SkillProviderActorHandle,
    };

    use super::*;
    use async_trait::async_trait;
    use serde_json::json;
    use tempfile::tempdir;

    impl WasmRuntime {
        pub fn local(skill_provider_api: SkillProviderApi) -> Self {
            let namespaces = OperatorConfig::local().namespaces;
            let provider = SkillProvider::new(&namespaces);
            Self::with_provider(provider, skill_provider_api)
        }
    }

    #[tokio::test]
    async fn greet_skill_component() {
        let skill_path = SkillPath::new("local", "greet_skill");
        let skill_provider = SkillProviderActorHandle::new(&OperatorConfig::local().namespaces);
        skill_provider.api().upsert(skill_path.clone(), None).await;

        let mut runtime = WasmRuntime::local(skill_provider.api());
        let skill_ctx = Box::new(CsiCompleteStub::new(|_| Completion::from_text("Hello")));
        runtime.upsert_skill(skill_path.clone(), None);
        let resp = runtime.run(&skill_path, json!("name"), skill_ctx).await;

        drop(runtime);
        skill_provider.wait_for_shutdown().await;

        assert_eq!(resp.unwrap(), "Hello");
    }

    #[tokio::test]
    async fn errors_for_non_existing_skill() {
        let skill_provider = SkillProviderActorHandle::new(&OperatorConfig::local().namespaces);
        let mut runtime = WasmRuntime::local(skill_provider.api());
        let skill_ctx = Box::new(CsiCompleteStub::new(|_| Completion::from_text("")));
        let resp = runtime
            .run(&SkillPath::dummy(), json!("name"), skill_ctx)
            .await;

        drop(runtime);
        skill_provider.wait_for_shutdown().await;

        assert!(resp.is_err());
    }

    #[tokio::test]
    async fn drop_non_existing_skill_from_cache() {
        // Given a WasmRuntime with no cached skills
        let skill_provider = SkillProviderActorHandle::new(&OperatorConfig::local().namespaces);
        let mut runtime = WasmRuntime::local(skill_provider.api());

        // When removing a skill from the runtime
        let result = runtime.invalidate_cached_skill(&SkillPath::from_str("non-cached-skill"));

        drop(runtime);
        skill_provider.wait_for_shutdown().await;

        // Then
        assert!(!result);
    }

    #[tokio::test]
    async fn drop_existing_skill_from_cache() {
        // Given a WasmRuntime with a cached skill
        let skill_path = SkillPath::new("local", "greet_skill");
        let skill_provider = SkillProviderActorHandle::new(&OperatorConfig::local().namespaces);
        skill_provider.api().upsert(skill_path.clone(), None).await;
        let mut runtime = WasmRuntime::local(skill_provider.api());
        runtime.upsert_skill(skill_path.clone(), None);
        let skill_ctx = Box::new(CsiCompleteStub::new(|_| Completion::from_text("")));
        drop(
            runtime
                .run(&skill_path, json!("name"), skill_ctx)
                .await
                .unwrap(),
        );

        // When dropping a skill from the runtime
        let had_been_in_cache = skill_provider.api().invalidate_cache(skill_path).await;
        let loaded_skill_count = skill_provider.api().list_cached().await.len();

        drop(runtime);
        skill_provider.wait_for_shutdown().await;

        // Then the component hash map is empty
        assert_eq!(loaded_skill_count, 0);

        // And it had actually been cached before
        assert!(had_been_in_cache);
    }

    #[tokio::test]
    async fn lazy_skill_loading() {
        // Giving and empty skill directory to the WasmRuntime
        let skill_dir = tempdir().unwrap();
        let skill_path = SkillPath::new("local", "greet_skill");

        let skill_provider = SkillProviderActorHandle::new(&OperatorConfig::local().namespaces);
        skill_provider.api().upsert(skill_path.clone(), None).await;
        let mut runtime = WasmRuntime::local(skill_provider.api());
        runtime.upsert_skill(skill_path.clone(), None);
        let skill_ctx = Box::new(CsiCompleteStub::new(|_| Completion::from_text("")));

        // When adding a new skill component
        let skill_file = skill_dir.path().join("greet_skill.wasm");
        fs::copy("./skills/greet_skill.wasm", skill_file).unwrap();

        // Then the skill can be invoked
        let greet = runtime.run(&skill_path, json!("Homer"), skill_ctx).await;

        drop(runtime);
        skill_provider.wait_for_shutdown().await;

        assert!(greet.is_ok());
    }

    #[tokio::test]
    async fn rust_greeting_skill() {
        let skill_ctx = Box::new(CsiGreetingMock);
        let skill_path = SkillPath::new("local", "greet_skill");
        let skill_provider = SkillProviderActorHandle::new(&OperatorConfig::local().namespaces);
        skill_provider.api().upsert(skill_path.clone(), None).await;
        let mut runtime = WasmRuntime::local(skill_provider.api());

        let actual = runtime
            .run(&skill_path, json!("Homer"), skill_ctx)
            .await
            .unwrap();

        drop(runtime);
        skill_provider.wait_for_shutdown().await;

        assert_eq!(actual, "Hello Homer");
    }

    #[tokio::test]
    async fn python_greeting_skill() {
        let skill_ctx = Box::new(CsiGreetingMock);
        let skill_path = SkillPath::new("local", "greet_skill");
        let skill_provider = SkillProviderActorHandle::new(&OperatorConfig::local().namespaces);
        skill_provider.api().upsert(skill_path.clone(), None).await;
        let mut runtime = WasmRuntime::local(skill_provider.api());

        let actual = runtime
            .run(&skill_path, json!("Homer"), skill_ctx)
            .await
            .unwrap();

        drop(runtime);
        skill_provider.wait_for_shutdown().await;

        assert_eq!(actual, "Hello Homer");
    }

    #[tokio::test]
    async fn can_call_preinstantiated_multiple_times() {
        let skill_ctx = Box::new(CsiCounter::new());
        let skill_path = SkillPath::new("local", "greet_skill");
        let skill_provider = SkillProviderActorHandle::new(&OperatorConfig::local().namespaces);
        skill_provider.api().upsert(skill_path.clone(), None).await;
        let mut runtime = WasmRuntime::local(skill_provider.api());
        for i in 1..10 {
            let resp = runtime
                .run(&skill_path, json!("Homer"), skill_ctx.clone())
                .await
                .unwrap();
            assert_eq!(resp, json!(i.to_string()));
        }

        drop(runtime);
        skill_provider.wait_for_shutdown().await;
    }

    /// A test double for a [`Csi`] implementation which always completes with the provided function.
    pub struct CsiCompleteStub {
        complete: Box<dyn FnMut(CompletionRequest) -> Completion + Send>,
    }

    impl CsiCompleteStub {
        pub fn new(complete: impl FnMut(CompletionRequest) -> Completion + Send + 'static) -> Self {
            Self {
                complete: Box::new(complete),
            }
        }
    }

    #[async_trait]
    impl CsiForSkills for CsiCompleteStub {
        async fn complete_text(&mut self, request: CompletionRequest) -> Completion {
            (self.complete)(request)
        }

        async fn complete_all(&mut self, _requests: Vec<CompletionRequest>) -> Vec<Completion> {
            unimplemented!()
        }

        async fn chunk(&mut self, request: ChunkRequest) -> Vec<String> {
            vec![request.text]
        }

        async fn select_language(
            &mut self,
            text: String,
            languages: Vec<Language>,
        ) -> Option<Language> {
            select_language(&text, &languages)
        }
    }

    /// Asserts a specific prompt and model and returns a greeting message
    pub struct CsiGreetingMock;

    #[async_trait]
    impl CsiForSkills for CsiGreetingMock {
        async fn complete_text(&mut self, request: CompletionRequest) -> Completion {
            let expected_prompt = "<|begin_of_text|><|start_header_id|>system<|end_header_id|>

Cutting Knowledge Date: December 2023
Today Date: 23 Jul 2024

You are a helpful assistant.<|eot_id|><|start_header_id|>user<|end_header_id|>

Provide a nice greeting for the person named: Homer<|eot_id|><|start_header_id|>assistant<|end_header_id|>";

            let expected_model = "llama-3.1-8b-instruct";

            // Print actual parameters in case of failure
            eprintln!("{request:?}");

            if matches!(request, CompletionRequest{ prompt, model, ..} if model == expected_model && prompt == expected_prompt)
            {
                Completion::from_text("Hello Homer")
            } else {
                Completion::from_text("Mock expectation violated")
            }
        }

        async fn complete_all(&mut self, _requests: Vec<CompletionRequest>) -> Vec<Completion> {
            unimplemented!()
        }

        async fn chunk(&mut self, request: ChunkRequest) -> Vec<String> {
            vec![request.text]
        }

        async fn select_language(
            &mut self,
            text: String,
            languages: Vec<Language>,
        ) -> Option<Language> {
            select_language(&text, &languages)
        }
    }

    #[derive(Default, Clone)]
    struct CsiCounter {
        counter: Arc<Mutex<u32>>,
    }

    impl CsiCounter {
        fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl CsiForSkills for CsiCounter {
        async fn complete_text(&mut self, _params: CompletionRequest) -> Completion {
            let mut counter = self.counter.lock().unwrap();
            *counter += 1;
            Completion::from_text(counter.to_string())
        }

        async fn complete_all(&mut self, _requests: Vec<CompletionRequest>) -> Vec<Completion> {
            unimplemented!()
        }

        async fn chunk(&mut self, request: ChunkRequest) -> Vec<String> {
            vec![request.text]
        }

        async fn select_language(
            &mut self,
            text: String,
            languages: Vec<Language>,
        ) -> Option<Language> {
            select_language(&text, &languages)
        }
    }
}
