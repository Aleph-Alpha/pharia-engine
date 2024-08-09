use anyhow::anyhow;
use semver::Version;
use serde_json::{json, Value};
use strum::{EnumIter, IntoEnumIterator};
use wasmtime::{
    component::{Component, Linker as WasmtimeLinker},
    Config, Engine as WasmtimeEngine, OptLevel, Store,
};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};
use wit_parser::decoding::{decode, DecodedWasm};

use super::Csi;

/// Wasmtime engine that is configured with linkers for all of the supported versions of
/// our pharia/skill WIT world.
pub struct Engine {
    engine: WasmtimeEngine,
    linker: WasmtimeLinker<LinkedCtx>,
}

impl Engine {
    pub fn new() -> anyhow::Result<Self> {
        let engine = WasmtimeEngine::new(
            Config::new()
                .async_support(true)
                .cranelift_opt_level(OptLevel::SpeedAndSize)
                .wasm_component_model(true),
        )?;

        let mut linker = WasmtimeLinker::new(&engine);
        // provide host implementation of WASI interfaces required by the component with wit-bindgen
        wasmtime_wasi::add_to_linker_async(&mut linker)?;
        // Skill world from bindgen
        for version in SupportedVersion::iter() {
            match version {
                SupportedVersion::Unversioned => {
                    unversioned::Skill::add_to_linker(&mut linker, |state: &mut LinkedCtx| state)?;
                }
                SupportedVersion::V0_1 => {
                    v0_1::Skill::add_to_linker(&mut linker, |state: &mut LinkedCtx| state)?;
                }
            }
        }

        Ok(Self { engine, linker })
    }

    /// Extracts the version of the skill WIT world from the provided bytes,
    /// and links it to the appropriate version in the linker.
    pub fn instantiate_pre_skill(&self, bytes: impl AsRef<[u8]>) -> anyhow::Result<Skill> {
        let skill_version = SupportedVersion::extract(&bytes)?;
        let component = Component::new(&self.engine, bytes)?;
        let pre = self.linker.instantiate_pre(&component)?;

        match skill_version {
            SupportedVersion::Unversioned => {
                let skill = unversioned::SkillPre::new(pre)?;
                Ok(Skill::Unversioned(skill))
            }
            SupportedVersion::V0_1 => {
                let skill = v0_1::SkillPre::new(pre)?;
                Ok(Skill::V0_1(skill))
            }
        }
    }

    fn store<T>(&self, data: T) -> Store<T> {
        Store::new(&self.engine, data)
    }
}

/// Pre-initialized skills already attached to their corresponding linker.
/// Allows for as much initialization work to be done at load time as possible,
/// which can be cached across multiple invocations.
pub enum Skill {
    /// Skills targeting versions 0.1.x of the skill world
    V0_1(v0_1::SkillPre<LinkedCtx>),
    /// Skills targeting the pre-semver-released version of skill world
    Unversioned(unversioned::SkillPre<LinkedCtx>),
}

impl Skill {
    pub async fn run(
        &self,
        engine: &Engine,
        ctx: Box<dyn Csi + Send>,
        input: Value,
    ) -> anyhow::Result<Value> {
        let mut store = engine.store(LinkedCtx::new(ctx));
        match self {
            Self::V0_1(skill) => {
                let input = serde_json::to_vec(&input)?;
                let bindings = skill.instantiate_async(&mut store).await?;
                let result = bindings.call_run(store, &input).await??;
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
    /// Versions 0.1.x of the skill world
    V0_1,
    /// Pre-semver-released version of skill world
    Unversioned,
}

impl SupportedVersion {
    fn extract(wasm: impl AsRef<[u8]>) -> anyhow::Result<Self> {
        match Self::extract_pharia_skill_version(wasm)? {
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
pub(super) struct LinkedCtx {
    wasi_ctx: WasiCtx,
    resource_table: ResourceTable,
    skill_ctx: Box<dyn Csi + Send>,
}

impl LinkedCtx {
    fn new(skill_ctx: Box<dyn Csi + Send>) -> Self {
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

mod v0_1 {
    use pharia::skill::csi::{Completion, CompletionParams, FinishReason, Host};
    use wasmtime::component::bindgen;

    use crate::inference;

    use super::LinkedCtx;

    bindgen!({ world: "skill", path: "./wit/skill@0.1.0", async: true });

    #[async_trait::async_trait]
    impl Host for LinkedCtx {
        #[must_use]
        async fn complete(
            &mut self,
            model: String,
            prompt: String,
            options: Option<CompletionParams>,
        ) -> Completion {
            let mut request = inference::CompletionRequest::new(prompt, model);
            if let Some(CompletionParams {
                max_tokens,
                temperature,
                top_k,
                top_p,
            }) = options
            {
                let params = inference::CompletionParams {
                    max_tokens,
                    temperature,
                    top_k,
                    top_p,
                };
                request = request.with_params(params);
            }
            let text = self.skill_ctx.complete_text(request).await;
            Completion {
                text,
                finish_reason: FinishReason::Stop,
            }
        }
    }
}

mod unversioned {
    use pharia::skill::csi::Host;
    use wasmtime::component::bindgen;

    use crate::inference::CompletionRequest;

    use super::LinkedCtx;

    bindgen!({ world: "skill", path: "./wit/skill@unversioned", async: true });

    #[async_trait::async_trait]
    impl Host for LinkedCtx {
        #[must_use]
        async fn complete_text(&mut self, prompt: String, model: String) -> String {
            let request = CompletionRequest::new(prompt, model);
            self.skill_ctx.complete_text(request).await
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::skills::runtime::wasm::tests::CsiGreetingMock;

    use super::*;

    #[test]
    fn can_parse_module() {
        let wasm = fs::read("skills/greet_skill.wasm").unwrap();
        let version = SupportedVersion::extract_pharia_skill_version(&wasm).unwrap();
        assert_eq!(version, None);
    }

    #[test]
    fn errors_if_not_pharia_component() {
        let wasm = wat::parse_str("(component)").unwrap();
        let version = SupportedVersion::extract_pharia_skill_version(&wasm);
        assert!(version.is_err());
    }

    #[test]
    fn errors_if_not_component() {
        let wasm = wat::parse_str("(module)").unwrap();
        let version = SupportedVersion::extract_pharia_skill_version(&wasm);
        assert!(version.is_err());
    }

    #[tokio::test]
    async fn can_load_and_run_v0_1_module() {
        // Given a skill loaded by our engine
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
}
