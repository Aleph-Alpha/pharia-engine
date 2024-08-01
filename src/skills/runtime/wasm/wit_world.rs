use strum::{EnumIter, IntoEnumIterator};
use wasmtime::{
    component::{Component, Linker},
    Store,
};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};

use super::Csi;

/// Currently supported versions of the skill world
#[derive(Debug, Clone, Copy, EnumIter)]
pub enum SupportedVersion {
    /// Pre-semver-released version of skill world
    Unversioned,
}

impl SupportedVersion {
    /// Adds all supported WIT worlds to the linker.
    pub fn add_all_to_linker(linker: &mut Linker<LinkedCtx>) -> anyhow::Result<()> {
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
    pub async fn run_skill(
        &self,
        mut store: &mut Store<LinkedCtx>,
        component: &Component,
        linker: &mut Linker<LinkedCtx>,
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
}

/// Linked against the skill by the wasm time. For the most part this gives the skill access to the
/// CSI.
pub struct LinkedCtx {
    wasi_ctx: WasiCtx,
    resource_table: ResourceTable,
    skill_ctx: Box<dyn Csi + Send>,
}

impl LinkedCtx {
    pub fn new(skill_ctx: Box<dyn Csi + Send>) -> Self {
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

    use semver::Version;
    use wit_parser::decoding::{decode, DecodedWasm};

    fn extract_pharia_skill_version(wasm: &[u8]) -> anyhow::Result<Option<Version>> {
        let decoded = decode(wasm)?;
        if let DecodedWasm::Component(resolve, ..) = decoded {
            Ok(resolve
                .package_names
                .keys()
                .find(|k| (k.namespace == "pharia" && k.name == "skill"))
                .and_then(|k| k.version.clone()))
        } else {
            Ok(None)
        }
    }

    #[test]
    fn can_parse_module() {
        let wasm = fs::read("skills/greet_skill.wasm").unwrap();
        let version = extract_pharia_skill_version(&wasm).unwrap();
        assert_eq!(version, None);
    }
}
