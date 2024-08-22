use serde_json::Value;

use crate::skills::SkillPath;

use super::{engine::Engine, provider::SkillProvider, Csi, Runtime};

pub struct WasmRuntime {
    engine: Engine,
    provider: SkillProvider,
}

impl WasmRuntime {
    pub fn with_provider(skill_provider: SkillProvider) -> Self {
        Self {
            engine: Engine::new().expect("engine creation failed"),
            provider: skill_provider,
        }
    }
}

impl Runtime for WasmRuntime {
    async fn run(
        &mut self,
        skill_path: &SkillPath,
        input: Value,
        ctx: Box<dyn Csi + Send>,
    ) -> anyhow::Result<Value> {
        let skill = self.provider.fetch(skill_path, &self.engine).await?;
        skill.run(&self.engine, ctx, input).await
    }

    fn add_skill(&mut self, skill: SkillPath) {
        self.provider.add_skill(skill);
    }

    fn remove_skill(&mut self, skill: &SkillPath) {
        self.provider.remove_skill(skill);
    }

    fn skills(&self) -> impl Iterator<Item = SkillPath> {
        self.provider.skills()
    }

    fn loaded_skills(&self) -> impl Iterator<Item = String> {
        self.provider.loaded_skills()
    }

    fn invalidate_cached_skill(&mut self, skill: &SkillPath) -> bool {
        self.provider.invalidate(skill)
    }
}

#[cfg(test)]
pub mod tests {
    use std::{
        collections::HashSet,
        fs,
        sync::{Arc, Mutex},
    };

    use crate::{
        configuration_observer::OperatorConfig,
        inference::{Completion, CompletionRequest},
    };

    use super::*;
    use async_trait::async_trait;
    use serde_json::json;
    use tempfile::tempdir;

    impl WasmRuntime {
        pub fn local() -> Self {
            let namespaces = OperatorConfig::local().namespaces;
            let provider = SkillProvider::new(&namespaces);
            Self::with_provider(provider)
        }
    }

    #[tokio::test]
    async fn greet_skill_component() {
        let skill_ctx = Box::new(CsiGreetingStub);
        let mut runtime = WasmRuntime::local();
        let skill_path = SkillPath::new("local", "greet_skill");
        runtime.add_skill(skill_path.clone());
        let resp = runtime.run(&skill_path, json!("name"), skill_ctx).await;

        assert_eq!(resp.unwrap(), "Hello");
    }

    #[tokio::test]
    async fn errors_for_non_existing_skill() {
        let skill_ctx = Box::new(CsiGreetingStub);
        let mut runtime = WasmRuntime::local();
        let resp = runtime
            .run(&SkillPath::dummy(), json!("name"), skill_ctx)
            .await;
        assert!(resp.is_err());
    }

    #[tokio::test]
    async fn drop_non_existing_skill_from_cache() {
        // Given a WasmRuntime with no cached skills
        let mut runtime = WasmRuntime::local();

        // When removing a skill from the runtime
        let result = runtime.invalidate_cached_skill(&SkillPath::from_str("non-cached-skill"));

        // Then
        assert!(!result);
    }

    #[tokio::test]
    async fn drop_existing_skill_from_cache() {
        // Given a WasmRuntime with a cached skill
        let mut runtime = WasmRuntime::local();
        let skill_path = SkillPath::new("local", "greet_skill");
        runtime.add_skill(skill_path.clone());
        let skill_ctx = Box::new(CsiGreetingStub);
        drop(
            runtime
                .run(&skill_path, json!("name"), skill_ctx)
                .await
                .unwrap(),
        );

        // When dropping a skill from the runtime
        let result = runtime.invalidate_cached_skill(&skill_path);

        // Then the component hash map is empty
        assert_eq!(runtime.loaded_skills().count(), 0);

        // And result is a success
        assert!(result);
    }

    #[tokio::test]
    async fn no_skills_are_listed() {
        // given a fresh WasmRuntime
        let runtime = WasmRuntime::local();

        // when querying skills
        let skill_count = runtime.loaded_skills().count();

        // then an empty vec is returned
        assert_eq!(skill_count, 0);
    }

    #[tokio::test]
    async fn skills_are_listed() {
        // given a runtime with two installed skills
        let mut runtime = WasmRuntime::local();
        let skill_path_rs = SkillPath::new("local", "greet_skill");
        runtime.add_skill(skill_path_rs.clone());
        let skill_ctx = Box::new(CsiGreetingStub);
        drop(
            runtime
                .run(&skill_path_rs, json!("name"), skill_ctx)
                .await
                .unwrap(),
        );

        let skill_ctx = Box::new(CsiGreetingStub);
        let skill_path_py = SkillPath::new("local", "greet-py");
        runtime.add_skill(skill_path_py.clone());

        drop(
            runtime
                .run(&skill_path_py, json!("name"), skill_ctx)
                .await
                .unwrap(),
        );

        // when querying skills
        let skills = runtime.loaded_skills();

        // convert to a set
        let skills: HashSet<String> = skills.collect();
        let expected: HashSet<String> = [skill_path_py.to_string(), skill_path_rs.to_string()]
            .into_iter()
            .collect();
        assert_eq!(skills, expected);
    }

    #[tokio::test]
    async fn lazy_skill_loading() {
        // Giving and empty skill directory to the WasmRuntime
        let skill_dir = tempdir().unwrap();

        let mut runtime = WasmRuntime::local();
        let skill_path = SkillPath::new("local", "greet_skill");
        runtime.add_skill(skill_path.clone());
        let skill_ctx = Box::new(CsiGreetingStub);

        // When adding a new skill component
        let skill_file = skill_dir.path().join("greet_skill.wasm");
        fs::copy("./skills/greet_skill.wasm", skill_file).unwrap();

        // Then the skill can be invoked
        let greet = runtime.run(&skill_path, json!("Homer"), skill_ctx).await;
        assert!(greet.is_ok());
    }

    #[tokio::test]
    async fn rust_greeting_skill() {
        let skill_ctx = Box::new(CsiGreetingMock);

        let mut runtime = WasmRuntime::local();
        let skill_path = SkillPath::new("local", "greet_skill");
        runtime.add_skill(skill_path.clone());

        let actual = runtime
            .run(&skill_path, json!("Homer"), skill_ctx)
            .await
            .unwrap();

        assert_eq!(actual, "Hello Homer");
    }

    #[tokio::test]
    async fn python_greeting_skill() {
        let skill_ctx = Box::new(CsiGreetingMock);

        let mut runtime = WasmRuntime::local();
        let skill_path = SkillPath::new("local", "greet_skill");
        runtime.add_skill(skill_path.clone());

        let actual = runtime
            .run(&skill_path, json!("Homer"), skill_ctx)
            .await
            .unwrap();

        assert_eq!(actual, "Hello Homer");
    }

    #[tokio::test]
    async fn can_call_preinstantiated_multiple_times() {
        let skill_ctx = Box::new(CsiCounter::new());
        let mut runtime = WasmRuntime::local();
        let skill_path = SkillPath::new("local", "greet_skill");
        runtime.add_skill(skill_path.clone());
        for i in 1..10 {
            let resp = runtime
                .run(&skill_path, json!("Homer"), skill_ctx.clone())
                .await
                .unwrap();
            assert_eq!(resp, json!(i.to_string()));
        }
    }

    /// A test double for a [`Csi`] implementation which always completes with "Hello".
    pub struct CsiGreetingStub;

    #[async_trait]
    impl Csi for CsiGreetingStub {
        async fn complete_text(&mut self, _request: CompletionRequest) -> Completion {
            Completion::from_text("Hello")
        }
    }

    /// Asserts a specific prompt and model and returns a greeting message
    pub struct CsiGreetingMock;

    #[async_trait]
    impl Csi for CsiGreetingMock {
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
    impl Csi for CsiCounter {
        async fn complete_text(&mut self, _params: CompletionRequest) -> Completion {
            let mut counter = self.counter.lock().unwrap();
            *counter += 1;
            Completion::from_text(counter.to_string())
        }
    }
}
