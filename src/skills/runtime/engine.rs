use std::time::{Duration, Instant};

use anyhow::anyhow;
use semver::Version;
use serde_json::{json, Value};
use strum::{EnumIter, IntoEnumIterator};
use wasmtime::{
    component::{Component, Linker as WasmtimeLinker},
    Config, Engine as WasmtimeEngine, OptLevel, Store, UpdateDeadline,
};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};
use wit_parser::decoding::{decode, DecodedWasm};

use super::CsiForSkills;

/// Wasmtime engine that is configured with linkers for all of the supported versions of
/// our pharia/skill WIT world.
pub struct Engine {
    inner: WasmtimeEngine,
    linker: WasmtimeLinker<LinkedCtx>,
}

impl Engine {
    /// How long to wait before incrementing the epoch counter.
    const EPOCH_INTERVAL: Duration = Duration::from_millis(100);
    /// Maximum skill execution time before we cancel the execution.
    /// Currently set to 10 minutes as an upper bound.
    const MAX_EXECUTION_TIME: Duration = Duration::from_secs(60 * 10);

    pub fn new() -> anyhow::Result<Self> {
        let engine = WasmtimeEngine::new(
            Config::new()
                .async_support(true)
                .cranelift_opt_level(OptLevel::SpeedAndSize)
                // Allows for cooperative timeslicing in async mode
                .epoch_interruption(true)
                .wasm_component_model(true),
        )?;

        // We only need a weak reference to pass to the loop.
        let engine_ref = engine.weak();

        // Increment epoch counter so that running skills have to yield
        // Uses a real thread to make sure this doesn't get blocked in
        // the async runtime by a skill that doesn't yield.
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Self::EPOCH_INTERVAL);
                // If the engine is still alive, increment the epoch counter.
                // Otherwise stop the thread.
                let Some(engine) = engine_ref.upgrade() else {
                    break;
                };
                engine.increment_epoch();
            }
        });

        let mut linker = WasmtimeLinker::new(&engine);
        // provide host implementation of WASI interfaces required by the component with wit-bindgen
        wasmtime_wasi::add_to_linker_async(&mut linker)?;
        // Skill world from bindgen
        for version in SupportedVersion::iter() {
            match version {
                SupportedVersion::V0_2 => {
                    v0_2::Skill::add_to_linker(&mut linker, |state: &mut LinkedCtx| state)?;
                }
                SupportedVersion::V0_1 => {
                    v0_1::Skill::add_to_linker(&mut linker, |state: &mut LinkedCtx| state)?;
                }
                SupportedVersion::Unversioned => {
                    unversioned::Skill::add_to_linker(&mut linker, |state: &mut LinkedCtx| state)?;
                }
            }
        }

        Ok(Self {
            inner: engine,
            linker,
        })
    }

    /// Extracts the version of the skill WIT world from the provided bytes,
    /// and links it to the appropriate version in the linker.
    pub fn instantiate_pre_skill(&self, bytes: impl AsRef<[u8]>) -> anyhow::Result<Skill> {
        let skill_version = SupportedVersion::extract(&bytes)?;
        let component = Component::new(&self.inner, bytes)?;
        let pre = self.linker.instantiate_pre(&component)?;

        match skill_version {
            SupportedVersion::V0_2 => {
                let skill = v0_2::SkillPre::new(pre)?;
                Ok(Skill::V0_2(skill))
            }
            SupportedVersion::V0_1 => {
                let skill = v0_1::SkillPre::new(pre)?;
                Ok(Skill::V0_1(skill))
            }
            SupportedVersion::Unversioned => {
                let skill = unversioned::SkillPre::new(pre)?;
                Ok(Skill::Unversioned(skill))
            }
        }
    }

    fn store<T>(&self, data: T) -> Store<T> {
        let mut store = Store::new(&self.inner, data);
        // Check after the next tick
        store.set_epoch_deadline(1);
        // Once the deadline is reached, the callback will be called.
        // If the skill hasn't been running for more than 10 minutes, it will yield
        // and be allowed to run for one more tick.
        // If it has been running for more than 10 minutes, it will trap and return an error.
        let start = Instant::now();
        store.epoch_deadline_callback(move |_| {
            let now = Instant::now();
            if now - start < Self::MAX_EXECUTION_TIME {
                Ok(UpdateDeadline::Yield(1))
            } else {
                Err(anyhow!("Maximum skill execution time reached."))
            }
        });
        store
    }
}

/// Pre-initialized skills already attached to their corresponding linker.
/// Allows for as much initialization work to be done at load time as possible,
/// which can be cached across multiple invocations.
pub enum Skill {
    /// Skills targeting versions 0.2.x of the skill world
    V0_2(v0_2::SkillPre<LinkedCtx>),
    /// Skills targeting versions 0.1.x of the skill world
    V0_1(v0_1::SkillPre<LinkedCtx>),
    /// Skills targeting the pre-semver-released version of skill world
    Unversioned(unversioned::SkillPre<LinkedCtx>),
}

impl Skill {
    pub async fn run(
        &self,
        engine: &Engine,
        ctx: Box<dyn CsiForSkills + Send>,
        input: Value,
    ) -> anyhow::Result<Value> {
        let mut store = engine.store(LinkedCtx::new(ctx));
        match self {
            Self::V0_2(skill) => {
                let input = serde_json::to_vec(&input)?;
                let bindings = skill.instantiate_async(&mut store).await?;
                let result = bindings
                    .pharia_skill_skill_handler()
                    .call_run(store, &input)
                    .await?;
                let result = match result {
                    Ok(result) => result,
                    Err(e) => match e {
                        v0_2::exports::pharia::skill::skill_handler::Error::Internal(e) => {
                            tracing::error!("Failed to run skill, internal skill error:\n{e}");
                            return Err(anyhow!("Internal skill error:\n{e}"));
                        }
                        v0_2::exports::pharia::skill::skill_handler::Error::InvalidInput(e) => {
                            tracing::error!("Failed to run skill, invalid input:\n{e}");
                            return Err(anyhow!("Invalid input:\n{e}"));
                        }
                    },
                };
                Ok(serde_json::from_slice(&result)?)
            }
            Self::V0_1(skill) => {
                let input = serde_json::to_vec(&input)?;
                let bindings = skill.instantiate_async(&mut store).await?;
                let result = bindings
                    .pharia_skill_skill_handler()
                    .call_run(store, &input)
                    .await?;
                let result = match result {
                    Ok(result) => result,
                    Err(e) => match e {
                        v0_1::exports::pharia::skill::skill_handler::Error::Internal(e) => {
                            tracing::error!("Failed to run skill, internal skill error:\n{e}");
                            return Err(anyhow!("Internal skill error:\n{e}"));
                        }
                        v0_1::exports::pharia::skill::skill_handler::Error::InvalidInput(e) => {
                            tracing::error!("Failed to run skill, invalid input:\n{e}");
                            return Err(anyhow!("Invalid input:\n{e}"));
                        }
                    },
                };
                Ok(serde_json::from_slice(&result)?)
            }
            Self::Unversioned(skill) => {
                let Some(input) = input.as_str() else {
                    return Err(anyhow!("Invalid input, string expected."));
                };
                let bindings = skill.instantiate_async(&mut store).await?;
                let result = bindings.call_run(store, input).await?;
                Ok(json!(result))
            }
        }
    }
}

/// Currently supported versions of the skill world
#[derive(Debug, Clone, Copy, EnumIter)]
enum SupportedVersion {
    /// Versions 0.2.x of the skill world
    V0_2,
    /// Versions 0.1.x of the skill world
    V0_1,
    /// Pre-semver-released version of skill world
    Unversioned,
}

impl SupportedVersion {
    fn extract(wasm: impl AsRef<[u8]>) -> anyhow::Result<Self> {
        match Self::extract_pharia_skill_version(wasm)? {
            Some(Version {
                major: 0, minor: 2, ..
            }) => Ok(Self::V0_2),
            Some(Version {
                major: 0, minor: 1, ..
            }) => Ok(Self::V0_1),
            None => Ok(Self::Unversioned),
            Some(_) => Err(anyhow!("Unsupported Pharia Skill version.")),
        }
    }

    fn extract_pharia_skill_version(wasm: impl AsRef<[u8]>) -> anyhow::Result<Option<Version>> {
        let decoded = decode(wasm.as_ref())?;
        if let DecodedWasm::Component(resolve, ..) = decoded {
            let package_name = &resolve
                .package_names
                .keys()
                .find(|k| (k.namespace == "pharia" && k.name == "skill"))
                .ok_or_else(|| anyhow!("Wasm component isn't using Pharia Skill."))?;
            Ok(package_name.version.clone())
        } else {
            Err(anyhow!("Wasm isn't a component."))
        }
    }
}

/// Linked against the skill by the wasm time. For the most part this gives the skill access to the
/// CSI.
pub struct LinkedCtx {
    wasi_ctx: WasiCtx,
    resource_table: ResourceTable,
    skill_ctx: Box<dyn CsiForSkills + Send>,
}

impl LinkedCtx {
    fn new(skill_ctx: Box<dyn CsiForSkills + Send>) -> Self {
        let mut builder = WasiCtxBuilder::new();
        LinkedCtx {
            wasi_ctx: builder.build(),
            resource_table: ResourceTable::new(),
            skill_ctx,
        }
    }
}

impl WasiView for LinkedCtx {
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.resource_table
    }

    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi_ctx
    }
}

mod v0_2 {
    use pharia::skill::csi::{
        ChunkParams, Completion, CompletionParams, CompletionRequest, FinishReason, Host, Language,
    };
    use wasmtime::component::bindgen;

    use crate::{csi::ChunkRequest, inference, language_selection};

    use super::LinkedCtx;

    bindgen!({ world: "skill", path: "./wit/skill@0.2", async: true });

    #[async_trait::async_trait]
    impl Host for LinkedCtx {
        #[must_use]
        async fn complete(
            &mut self,
            model: String,
            prompt: String,
            options: CompletionParams,
        ) -> Completion {
            let CompletionParams {
                max_tokens,
                temperature,
                top_k,
                top_p,
                stop,
            } = options;
            let params = inference::CompletionParams {
                max_tokens,
                temperature,
                top_k,
                top_p,
                stop,
            };
            let request = inference::CompletionRequest::new(prompt, model).with_params(params);
            self.skill_ctx.complete_text(request).await.into()
        }

        async fn complete_all(&mut self, requests: Vec<CompletionRequest>) -> Vec<Completion> {
            let requests = requests
                .into_iter()
                .map(|r| {
                    let CompletionParams {
                        max_tokens,
                        temperature,
                        top_k,
                        top_p,
                        stop,
                    } = r.params;
                    inference::CompletionRequest {
                        prompt: r.prompt,
                        model: r.model,
                        params: inference::CompletionParams {
                            max_tokens,
                            temperature,
                            top_k,
                            top_p,
                            stop,
                        },
                    }
                })
                .collect();

            self.skill_ctx
                .complete_all(requests)
                .await
                .into_iter()
                .map(|c| Completion {
                    text: c.text,
                    finish_reason: match c.finish_reason {
                        inference::FinishReason::Stop => FinishReason::Stop,
                        inference::FinishReason::Length => FinishReason::Length,
                        inference::FinishReason::ContentFilter => FinishReason::ContentFilter,
                    },
                })
                .collect()
        }

        async fn chunk(&mut self, text: String, params: ChunkParams) -> Vec<String> {
            let ChunkParams { model, max_tokens } = params;
            let request = ChunkRequest::new(text, model, max_tokens);
            self.skill_ctx.chunk(request).await
        }

        async fn select_language(
            &mut self,
            text: String,
            languages: Vec<Language>,
        ) -> Option<Language> {
            let languages = languages
                .iter()
                .map(|l| match l {
                    Language::Eng => language_selection::Language::Eng,
                    Language::Deu => language_selection::Language::Deu,
                })
                .collect::<Vec<_>>();
            self.skill_ctx
                .select_language(text, languages)
                .await
                .map(|l| match l {
                    language_selection::Language::Eng => Language::Eng,
                    language_selection::Language::Deu => Language::Deu,
                })
        }
    }

    impl From<inference::Completion> for Completion {
        fn from(completion: inference::Completion) -> Self {
            Self {
                text: completion.text,
                finish_reason: completion.finish_reason.into(),
            }
        }
    }

    impl From<inference::FinishReason> for FinishReason {
        fn from(finish_reason: inference::FinishReason) -> Self {
            match finish_reason {
                inference::FinishReason::Stop => Self::Stop,
                inference::FinishReason::Length => Self::Length,
                inference::FinishReason::ContentFilter => Self::ContentFilter,
            }
        }
    }
}

mod v0_1 {
    use pharia::skill::csi::{Completion, CompletionParams, FinishReason, Host};
    use wasmtime::component::bindgen;

    use crate::inference;

    use super::LinkedCtx;

    bindgen!({ world: "skill", path: "./wit/skill@0.1", async: true });

    #[async_trait::async_trait]
    impl Host for LinkedCtx {
        #[must_use]
        async fn complete(
            &mut self,
            model: String,
            prompt: String,
            options: Option<CompletionParams>,
        ) -> Completion {
            let params = if let Some(CompletionParams {
                max_tokens,
                temperature,
                top_k,
                top_p,
            }) = options
            {
                inference::CompletionParams {
                    max_tokens: max_tokens.or(Some(128)),
                    temperature,
                    top_k,
                    top_p,
                    ..Default::default()
                }
            } else {
                inference::CompletionParams {
                    max_tokens: Some(128),
                    ..Default::default()
                }
            };
            let request = inference::CompletionRequest::new(prompt, model).with_params(params);
            self.skill_ctx.complete_text(request).await.into()
        }
    }

    impl From<inference::Completion> for Completion {
        fn from(completion: inference::Completion) -> Self {
            Self {
                text: completion.text,
                finish_reason: completion.finish_reason.into(),
            }
        }
    }

    impl From<inference::FinishReason> for FinishReason {
        fn from(finish_reason: inference::FinishReason) -> Self {
            match finish_reason {
                inference::FinishReason::Stop => Self::Stop,
                inference::FinishReason::Length => Self::Length,
                inference::FinishReason::ContentFilter => Self::ContentFilter,
            }
        }
    }
}

mod unversioned {
    use pharia::skill::csi::Host;
    use wasmtime::component::bindgen;

    use crate::inference::{CompletionParams, CompletionRequest};

    use super::LinkedCtx;

    bindgen!({ world: "skill", path: "./wit/skill@unversioned", async: true });

    #[async_trait::async_trait]
    impl Host for LinkedCtx {
        #[must_use]
        async fn complete_text(&mut self, prompt: String, model: String) -> String {
            let request = CompletionRequest::new(prompt, model).with_params(CompletionParams {
                max_tokens: Some(128),
                ..Default::default()
            });
            self.skill_ctx.complete_text(request).await.text
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use test_skills::{
        given_greet_py, given_greet_py_v0_2, given_greet_skill, given_greet_skill_v0_1,
        given_greet_skill_v0_2,
    };
    use tokio::sync::oneshot;
    use v0_2::pharia::skill::csi::{CompletionParams, CompletionRequest, Host, Language};

    use crate::{
        csi::{tests::dummy_csi_apis, CsiDrivers},
        inference::{self, tests::InferenceStub},
        skills::{actor::SkillInvocationCtx, runtime::wasm::tests::CsiGreetingMock},
        tests::api_token,
    };

    use super::*;

    #[test]
    fn can_parse_module() {
        given_greet_skill();
        let wasm = fs::read("skills/greet_skill.wasm").unwrap();
        let version = SupportedVersion::extract_pharia_skill_version(wasm).unwrap();
        assert_eq!(version, None);
    }

    #[test]
    fn errors_if_not_pharia_component() {
        let wasm = wat::parse_str("(component)").unwrap();
        let version = SupportedVersion::extract_pharia_skill_version(wasm);
        assert!(version.is_err());
    }

    #[test]
    fn errors_if_not_component() {
        let wasm = wat::parse_str("(module)").unwrap();
        let version = SupportedVersion::extract_pharia_skill_version(wasm);
        assert!(version.is_err());
    }

    #[tokio::test]
    async fn complete_all_completion_requests_in_respective_order() {
        // Given a linked context
        let inference_stub = InferenceStub::new(|r| Ok(inference::Completion::from_text(r.prompt)));
        let csi_apis = CsiDrivers {
            inference: inference_stub.api(),
            ..dummy_csi_apis()
        };
        let (send_rt_err, _) = oneshot::channel();
        let skill_ctx = Box::new(SkillInvocationCtx::new(
            send_rt_err,
            csi_apis,
            api_token().to_owned(),
            None,
        ));
        let mut ctx = LinkedCtx::new(skill_ctx);

        // When requesting multiple completions
        let completion_req_1 = CompletionRequest {
            model: "dummy_model".to_owned(),
            prompt: "1st_request".to_owned(),
            params: CompletionParams {
                max_tokens: None,
                temperature: None,
                top_k: None,
                top_p: None,
                stop: vec![],
            },
        };

        let completion_req_2 = CompletionRequest {
            prompt: "2nd request".to_owned(),
            ..completion_req_1.clone()
        };

        let completions = ctx
            .complete_all(vec![completion_req_1, completion_req_2])
            .await;

        // Then the completion must have the same order as the respective requests
        assert_eq!(completions.len(), 2);
        assert!(completions.first().unwrap().text.contains("1st"));
        assert!(completions.get(1).unwrap().text.contains("2nd"));
    }

    #[tokio::test]
    async fn language_selection_from_csi() {
        // Given a linked context
        let (send_rt_err, _) = oneshot::channel();
        let skill_ctx = Box::new(SkillInvocationCtx::new(
            send_rt_err,
            dummy_csi_apis(),
            api_token().to_owned(),
            None,
        ));
        let mut ctx = LinkedCtx::new(skill_ctx);

        // When selecting a language based on the provided text
        let text = "This is a sentence written in German language.";
        let language = ctx
            .select_language(text.to_owned(), vec![Language::Eng, Language::Deu])
            .await;

        // Then English is selected as the language
        assert!(language.is_some());
        assert_eq!(language.unwrap(), Language::Eng);
    }

    #[tokio::test]
    async fn can_load_and_run_v0_1_module() {
        // Given a skill loaded by our engine
        given_greet_skill_v0_1();
        let wasm = fs::read("skills/greet_skill_v0_1.wasm").unwrap();
        let engine = Engine::new().unwrap();
        let skill = engine.instantiate_pre_skill(wasm).unwrap();
        let ctx = Box::new(CsiGreetingMock);

        // When invoked with a json string
        let input = json!("Homer");
        let result = skill.run(&engine, ctx, input).await.unwrap();

        // Then it returns a json string
        assert_eq!(result, json!("Hello Homer"));
    }

    #[tokio::test]
    async fn can_load_and_run_v0_2_module() {
        // Given a skill loaded by our engine
        given_greet_skill_v0_2();
        let wasm = fs::read("skills/greet_skill_v0_2.wasm").unwrap();
        let engine = Engine::new().unwrap();
        let skill = engine.instantiate_pre_skill(wasm).unwrap();
        let ctx = Box::new(CsiGreetingMock);

        // When invoked with a json string
        let input = json!("Homer");
        let result = skill.run(&engine, ctx, input).await.unwrap();

        // Then it returns a json string
        assert_eq!(result, json!("Hello Homer"));
    }

    #[tokio::test]
    async fn can_load_and_run_unversioned_py_module() {
        // Given a skill loaded by our engine
        given_greet_py();
        let wasm = fs::read("skills/greet-py.wasm").unwrap();
        let engine = Engine::new().unwrap();
        let skill = engine.instantiate_pre_skill(wasm).unwrap();
        let ctx = Box::new(CsiGreetingMock);

        // When invoked with a json string
        let input = json!("Homer");
        let result = skill.run(&engine, ctx, input).await.unwrap();

        // Then it returns a json string
        assert_eq!(result, json!("Hello Homer"));
    }

    #[tokio::test]
    async fn can_load_and_run_v0_2_py_module() {
        // Given a skill loaded by our engine
        given_greet_py_v0_2();
        let wasm = fs::read("skills/greet-py-v0_2.wasm").unwrap();
        let engine = Engine::new().unwrap();
        let skill = engine.instantiate_pre_skill(wasm).unwrap();
        let ctx = Box::new(CsiGreetingMock);

        // When invoked with a json string
        let input = json!("Homer");
        let result = skill.run(&engine, ctx, input).await.unwrap();

        // Then it returns a json string
        assert_eq!(result, json!("Hello Homer"));
    }
}
