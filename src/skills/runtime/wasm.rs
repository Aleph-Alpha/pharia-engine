use serde_json::Value;
use std::sync::Arc;

use crate::{
    skill_store::SkillStoreApi,
    skills::{actor::ExecuteSkillError, SkillPath},
};

use super::{engine::Engine, CsiForSkills};

pub struct WasmRuntime {
    /// Used to execute skills. We will share the engine with multiple running skills, and skill
    /// provider to convert bytes into executable skills.
    engine: Arc<Engine>,
    skill_provider_api: SkillStoreApi,
}

impl WasmRuntime {
    pub fn new(engine: Arc<Engine>, skill_provider_api: SkillStoreApi) -> Self {
        Self {
            engine,
            skill_provider_api,
        }
    }

    pub async fn run(
        &self,
        skill_path: &SkillPath,
        input: Value,
        ctx: Box<dyn CsiForSkills + Send>,
    ) -> Result<Value, ExecuteSkillError> {
        let skill = self
            .skill_provider_api
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
    use std::sync::{Arc, Mutex};

    use crate::{
        csi::ChunkRequest,
        inference::{ChatRequest, ChatResponse, Completion, CompletionRequest},
        language_selection::{select_language, Language, SelectLanguageRequest},
        namespace_watcher::OperatorConfig,
        search::{DocumentPath, SearchRequest, SearchResult},
        skill_store::SkillStore,
    };

    use super::*;
    use async_trait::async_trait;
    use serde_json::json;
    use test_skills::{given_greet_py, given_greet_skill};

    #[tokio::test]
    async fn greet_skill_component() {
        given_greet_skill();
        let skill_path = SkillPath::new("local", "greet_skill");
        let engine = Arc::new(Engine::new().unwrap());
        let skill_provider = SkillStore::new(
            engine.clone(),
            &OperatorConfig::local(&["greet_skill"]).namespaces,
        );
        skill_provider.api().upsert(skill_path.clone(), None).await;
        let runtime = WasmRuntime::new(engine, skill_provider.api());
        let skill_ctx = Box::new(CsiCompleteStub::new(|_| Completion::from_text("Hello")));
        let resp = runtime.run(&skill_path, json!("name"), skill_ctx).await;

        drop(runtime);
        skill_provider.wait_for_shutdown().await;

        assert_eq!(resp.unwrap(), "Hello");
    }

    #[tokio::test]
    async fn errors_for_non_existing_skill() {
        let engine = Arc::new(Engine::new().unwrap());
        let skill_provider =
            SkillStore::new(engine.clone(), &OperatorConfig::local(&[]).namespaces);
        let runtime = WasmRuntime::new(engine, skill_provider.api());
        let skill_ctx = Box::new(CsiCompleteStub::new(|_| Completion::from_text("")));
        let resp = runtime
            .run(&SkillPath::dummy(), json!("name"), skill_ctx)
            .await;

        drop(runtime);
        skill_provider.wait_for_shutdown().await;

        assert!(resp.is_err());
    }

    #[tokio::test]
    async fn rust_greeting_skill() {
        given_greet_skill();
        let skill_ctx = Box::new(CsiGreetingMock);
        let skill_path = SkillPath::new("local", "greet_skill");
        let engine = Arc::new(Engine::new().unwrap());
        let skill_provider = SkillStore::new(
            engine.clone(),
            &OperatorConfig::local(&["greet_skill"]).namespaces,
        );
        skill_provider.api().upsert(skill_path.clone(), None).await;
        let runtime = WasmRuntime::new(engine, skill_provider.api());

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
        given_greet_py();
        let skill_ctx = Box::new(CsiGreetingMock);
        let skill_path = SkillPath::new("local", "greet-py");
        let engine = Arc::new(Engine::new().unwrap());
        let skill_provider = SkillStore::new(
            engine.clone(),
            &OperatorConfig::local(&["greet-py"]).namespaces,
        );
        skill_provider.api().upsert(skill_path.clone(), None).await;
        let runtime = WasmRuntime::new(engine, skill_provider.api());

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
        given_greet_skill();
        let skill_ctx = Box::new(CsiCounter::new());
        let skill_path = SkillPath::new("local", "greet_skill");
        let engine = Arc::new(Engine::new().unwrap());
        let skill_provider = SkillStore::new(
            engine.clone(),
            &OperatorConfig::local(&["greet_skill"]).namespaces,
        );
        skill_provider.api().upsert(skill_path.clone(), None).await;
        let runtime = WasmRuntime::new(engine, skill_provider.api());
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

        async fn select_language(&mut self, request: SelectLanguageRequest) -> Option<Language> {
            select_language(request)
        }

        async fn search(&mut self, _request: SearchRequest) -> Vec<SearchResult> {
            unimplemented!()
        }

        async fn chat(&mut self, _request: ChatRequest) -> ChatResponse {
            unimplemented!()
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

        async fn select_language(&mut self, request: SelectLanguageRequest) -> Option<Language> {
            select_language(request)
        }

        async fn search(&mut self, request: SearchRequest) -> Vec<SearchResult> {
            let document_path = DocumentPath {
                namespace: "aleph-alpha".to_owned(),
                collection: "test-collection".to_owned(),
                name: "small".to_owned(),
            };
            vec![SearchResult {
                document_path,
                content: request.query,
                score: 1.0,
            }]
        }

        async fn chat(&mut self, _request: ChatRequest) -> ChatResponse {
            unimplemented!()
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

        async fn select_language(&mut self, request: SelectLanguageRequest) -> Option<Language> {
            select_language(request)
        }

        async fn search(&mut self, _request: SearchRequest) -> Vec<SearchResult> {
            unimplemented!()
        }

        async fn chat(&mut self, _request: ChatRequest) -> ChatResponse {
            unimplemented!()
        }
    }
}
