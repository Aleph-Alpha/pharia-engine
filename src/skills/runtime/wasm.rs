use anyhow::Error;
use serde_json::Value;
use wasmtime::{Config, Engine, OptLevel};

use crate::registries::{registries, SkillRegistry};

use super::{linker::Linker, provider::SkillProvider, Csi, Runtime};

pub struct WasmRuntime {
    engine: Engine,
    linker: Linker,
    skill_cache: SkillProvider,
}

impl WasmRuntime {
    pub fn new() -> Self {
        Self::with_registry(registries())
    }

    pub fn engine() -> Engine {
        Engine::new(
            Config::new()
                .async_support(true)
                .cranelift_opt_level(OptLevel::SpeedAndSize)
                .wasm_component_model(true),
        )
        .expect("config must be valid")
    }

    pub fn with_registry(skill_registry: impl SkillRegistry + Send + 'static) -> Self {
        let engine = Self::engine();
        let linker = Linker::new(&engine).expect("linking must succeed");

        Self {
            engine,
            linker,
            skill_cache: SkillProvider::new(Box::new(skill_registry)),
        }
    }
}

impl Runtime for WasmRuntime {
    async fn run(
        &mut self,
        skill_name: &str,
        input: Value,
        ctx: Box<dyn Csi + Send>,
    ) -> anyhow::Result<String> {
        let component = self.skill_cache.fetch(skill_name, &self.engine).await?;

        self.linker
            .run_skill(&self.engine, ctx, component, input)
            .await
    }

    fn skills(&self) -> impl Iterator<Item = &str> {
        self.skill_cache.skills()
    }

    fn invalidate_cached_skill(&mut self, skill: &str) -> bool {
        self.skill_cache.invalidate(skill)
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, fs};

    use crate::{inference::CompleteTextParameters, registries::FileRegistry};

    use super::*;
    use async_trait::async_trait;
    use serde_json::json;
    use tempfile::tempdir;

    #[tokio::test]
    async fn greet_skill_component() {
        let skill_ctx = Box::new(CsiGreetingStub);
        let mut runtime = WasmRuntime::new();
        let resp = runtime.run("greet_skill", json!("name"), skill_ctx).await;

        assert_eq!(resp.unwrap(), "Hello");
    }

    #[tokio::test]
    async fn errors_for_non_existing_skill() {
        let skill_ctx = Box::new(CsiGreetingStub);
        let mut runtime = WasmRuntime::new();
        let resp = runtime
            .run("non-existing-skill", json!("name"), skill_ctx)
            .await;
        assert!(resp.is_err());
    }

    #[tokio::test]
    async fn drop_non_existing_skill_from_cache() {
        // Given a WasmRuntime with no cached skills
        let mut runtime = WasmRuntime::new();

        // When removing a skill from the runtime
        let result = runtime.invalidate_cached_skill("non-cached-skill");

        // Then
        assert!(!result);
    }

    #[tokio::test]
    async fn drop_existing_skill_from_cache() {
        // Given a WasmRuntime with a cached skill
        let mut runtime = WasmRuntime::new();
        let skill_ctx = Box::new(CsiGreetingStub);
        drop(
            runtime
                .run("greet_skill", json!("name"), skill_ctx)
                .await
                .unwrap(),
        );

        // When dropping a skill from the runtime
        let result = runtime.invalidate_cached_skill("greet_skill");

        // Then the component hash map is empty
        assert_eq!(runtime.skills().count(), 0);

        // And result is a success
        assert!(result);
    }

    #[tokio::test]
    async fn no_skills_are_listed() {
        // given a fresh WasmRuntime
        let runtime = WasmRuntime::new();

        // when querying skills
        let skill_count = runtime.skills().count();

        // then an empty vec is returned
        assert_eq!(skill_count, 0);
    }

    #[tokio::test]
    async fn skills_are_listed() {
        // given a runtime with two installed skills
        let mut runtime = WasmRuntime::new();
        let skill_ctx = Box::new(CsiGreetingStub);
        drop(
            runtime
                .run("greet_skill", json!("name"), skill_ctx)
                .await
                .unwrap(),
        );

        let skill_ctx = Box::new(CsiGreetingStub);
        drop(
            runtime
                .run("greet-py", json!("name"), skill_ctx)
                .await
                .unwrap(),
        );

        // when querying skills
        let skills = runtime.skills();

        // convert to a set
        let skills: HashSet<String> = skills.map(str::to_owned).collect();
        let expected: HashSet<String> = ["greet-py".to_owned(), "greet_skill".to_owned()]
            .into_iter()
            .collect();
        assert_eq!(skills, expected);
    }

    #[tokio::test]
    async fn lazy_skill_loading() {
        // Giving and empty skill directory to the WasmRuntime
        let skill_dir = tempdir().unwrap();
        let registry = FileRegistry::with_dir(skill_dir.path());
        let mut runtime = WasmRuntime::with_registry(registry);
        let skill_ctx = Box::new(CsiGreetingStub);

        // When adding a new skill component
        let skill_path = skill_dir.path().join("greet_skill.wasm");
        fs::copy("./skills/greet_skill.wasm", skill_path).unwrap();

        // Then the skill can be invoked
        let greet = runtime.run("greet_skill", json!("Homer"), skill_ctx).await;
        assert!(greet.is_ok());
    }

    #[tokio::test]
    async fn rust_greeting_skill() {
        let skill_ctx = Box::new(CsiGreetingMock);

        let mut runtime = WasmRuntime::new();
        let actual = runtime
            .run("greet_skill", json!("Homer"), skill_ctx)
            .await
            .unwrap();

        assert_eq!(actual, "Hello Homer");
    }

    #[tokio::test]
    async fn python_greeting_skill() {
        let skill_ctx = Box::new(CsiGreetingMock);

        let mut runtime = WasmRuntime::new();
        let actual = runtime
            .run("greet-py", json!("Homer"), skill_ctx)
            .await
            .unwrap();

        assert_eq!(actual, "Hello Homer");
    }

    /// A test double for a [`Csi`] implementation which always completes with "Hello".
    struct CsiGreetingStub;

    #[async_trait]
    impl Csi for CsiGreetingStub {
        async fn complete_text(&mut self, _params: CompleteTextParameters) -> String {
            "Hello".to_owned()
        }
    }

    /// Asserts a specific prompt and model and returns a greeting message
    struct CsiGreetingMock;

    #[async_trait]
    impl Csi for CsiGreetingMock {
        async fn complete_text(&mut self, params: CompleteTextParameters) -> String {
            let expected_prompt = "### Instruction:\n\
                Provide a nice greeting for the person utilizing its given name\n\
                \n\
                ### Input:\n\
                Name: Homer\n\
                \n\
                ### Response:";

            let expected_model = "luminous-nextgen-7b";

            // Print actual parameters in case of failure
            eprintln!("{params:?}");

            if matches!(params, CompleteTextParameters{ prompt, model, max_tokens: 128 } if model == expected_model && prompt == expected_prompt)
            {
                "Hello Homer".to_owned()
            } else {
                "Mock expectation violated".to_owned()
            }
        }
    }
}
