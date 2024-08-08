use anyhow::anyhow;
use semver::Version;
use serde_json::{json, Value};
use strum::{EnumIter, IntoEnumIterator};
use wasmtime::{component::Linker as WasmtimeLinker, Engine, Store};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};
use wit_parser::decoding::{decode, DecodedWasm};

use super::{provider::CachedComponent, Csi};

pub struct Linker {
    linker: WasmtimeLinker<LinkedCtx>,
}

impl Linker {
    pub fn new(engine: &Engine) -> anyhow::Result<Self> {
        let mut linker = WasmtimeLinker::new(engine);
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

        Ok(Self { linker })
    }

    pub async fn run_skill(
        &mut self,
        engine: &Engine,
        ctx: Box<dyn Csi + Send>,
        component: &CachedComponent,
        input: Value,
    ) -> anyhow::Result<Value> {
        let invocation_ctx = LinkedCtx::new(ctx);
        let mut store = Store::new(engine, invocation_ctx);
        match component.skill_version() {
            SupportedVersion::Unversioned => {
                let Some(input) = input.as_str() else {
                    return Err(anyhow!("Invalid input, string expected."));
                };
                let bindings = unversioned::Skill::instantiate_async(
                    &mut store,
                    component.component(),
                    &self.linker,
                )
                .await
                .expect("failed to instantiate skill");
                let result = bindings.call_run(store, input).await?;
                Ok(json!(result))
            }
            SupportedVersion::V0_1 => todo!(),
        }
    }
}

/// Currently supported versions of the skill world
#[derive(Debug, Clone, Copy, EnumIter)]
pub enum SupportedVersion {
    /// Versions 0.1.x of the skill world
    V0_1,
    /// Pre-semver-released version of skill world
    Unversioned,
}

impl SupportedVersion {
    pub fn extract(wasm: impl AsRef<[u8]>) -> anyhow::Result<Self> {
        match Self::extract_pharia_skill_version(wasm)? {
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
struct LinkedCtx {
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
    use pharia::skill::csi::{Completion, CompletionParams, Error, Host};
    use wasmtime::component::bindgen;

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
        ) -> Result<Completion, Error> {
            todo!()
        }
    }
}

mod unversioned {
    use pharia::skill::csi::Host;
    use wasmtime::component::bindgen;

    use crate::inference::CompleteTextParameters;

    use super::LinkedCtx;

    bindgen!({ world: "skill", path: "./wit/skill@unversioned", async: true });

    #[async_trait::async_trait]
    impl Host for LinkedCtx {
        #[must_use]
        async fn complete_text(&mut self, prompt: String, model: String) -> String {
            let params = CompleteTextParameters {
                prompt,
                model,
                max_tokens: 128,
            };
            self.skill_ctx.complete_text(params).await
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

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
}
