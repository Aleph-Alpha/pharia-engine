use anyhow::anyhow;
use semver::Version;
use strum::{EnumIter, IntoEnumIterator};
use wasmtime::{
    component::{Component, Linker as WasmtimeLinker},
    Engine, Store,
};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};
use wit_parser::decoding::{decode, DecodedWasm};

use super::{provider::CachedComponent, Csi};

pub struct Linker {
    linker: WasmtimeLinker<LinkedCtx>,
}

impl Linker {
    pub fn new(engine: &Engine) -> Self {
        let mut linker = WasmtimeLinker::new(engine);
        // provide host implementation of WASI interfaces required by the component with wit-bindgen
        wasmtime_wasi::add_to_linker_async(&mut linker).expect("linking to WASI must work");
        // Skill world from bindgen
        SupportedVersion::add_all_to_linker(&mut linker).expect("linking to skill world must work");

        Self { linker }
    }

    pub async fn run_skill(
        &mut self,
        engine: &Engine,
        ctx: Box<dyn Csi + Send>,
        component: &CachedComponent,
        argument: &str,
    ) -> Result<String, anyhow::Error> {
        let invocation_ctx = LinkedCtx::new(ctx);
        let mut store = Store::new(engine, invocation_ctx);
        component
            .skill_version()
            .run_skill(
                &mut store,
                component.component(),
                &mut self.linker,
                argument,
            )
            .await
    }
}

/// Currently supported versions of the skill world
#[derive(Debug, Clone, Copy, EnumIter)]
pub enum SupportedVersion {
    /// Pre-semver-released version of skill world
    Unversioned,
}

impl SupportedVersion {
    /// Adds all supported WIT worlds to the linker.
    fn add_all_to_linker(linker: &mut WasmtimeLinker<LinkedCtx>) -> anyhow::Result<()> {
        for version in Self::iter() {
            match version {
                Self::Unversioned => {
                    unversioned::Skill::add_to_linker(linker, |state: &mut LinkedCtx| state)?;
                }
            }
        }

        Ok(())
    }

    /// Run a skill that targets a specific version of the WIT world
    async fn run_skill(
        &self,
        mut store: &mut Store<LinkedCtx>,
        component: &Component,
        linker: &mut WasmtimeLinker<LinkedCtx>,
        argument: &str,
    ) -> anyhow::Result<String> {
        match self {
            Self::Unversioned => {
                let bindings = unversioned::Skill::instantiate_async(&mut store, component, linker)
                    .await
                    .expect("failed to instantiate skill");
                bindings.call_run(store, argument).await
            }
        }
    }

    pub fn extract(wasm: impl AsRef<[u8]>) -> anyhow::Result<Self> {
        Ok(match Self::extract_pharia_skill_version(wasm)? {
            Some(_) => unreachable!(),
            None => Self::Unversioned,
        })
    }

    fn extract_pharia_skill_version(wasm: impl AsRef<[u8]>) -> anyhow::Result<Option<Version>> {
        let decoded = decode(wasm.as_ref())?;
        if let DecodedWasm::Component(resolve, ..) = decoded {
            let package_name = &resolve
                .package_names
                .keys()
                .find(|k| (k.namespace == "pharia" && k.name == "skill"))
                .ok_or_else(|| anyhow!("wasm component isn't using pharia skill"))?;
            Ok(package_name.version.clone())
        } else {
            Ok(None)
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

mod unversioned {
    use wasmtime::component::bindgen;

    use crate::inference::CompleteTextParameters;

    use super::LinkedCtx;

    bindgen!({ world: "skill", path: "./wit/skill@unversioned", async: true });

    #[async_trait::async_trait]
    impl pharia::skill::csi::Host for LinkedCtx {
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
}
