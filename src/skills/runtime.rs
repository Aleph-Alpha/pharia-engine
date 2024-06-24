use std::{collections::HashMap, future::Future};

use anyhow::{anyhow, Error};
use wasmtime::{
    component::{bindgen, Component, Linker},
    Config, Engine, Store,
};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};

use crate::inference::{CompleteTextParameters, InferenceApi};

bindgen!({ world: "skill", async: true });

pub trait Runtime {
    // We are returning a Future explicitly here instead of using the `async` syntax. This has the
    // following reason: The async syntax is ambiguous with regards to whether or not the Future is
    // `Send`. The Rust compiler figures out the lifetime and `Send`ness of the future implicitly
    // via type inference. Yet this for example can never work across crate bounds, and sometimes
    // hits its limits even within a crate. To give an example:
    //
    // `fn async f() -> i32` could be a shortcut for both `fn f() -> impl Future<Output=i32>` **or**
    // `fn f() -> impl Future<Output=i32> + Send`. It is also ambiguous over lifetime and `Sync`ness
    // of the future, but we do not need these traits here.
    fn run(
        &mut self,
        skill: &str,
        name: String,
        api_token: String,
        inference_api: InferenceApi,
    ) -> impl Future<Output = Result<String, Error>> + Send;
}

struct InvocationCtx {
    wasi_ctx: WasiCtx,
    resource_table: ResourceTable,
    inference_api: InferenceApi,
    api_token: String,
}

impl InvocationCtx {
    fn new(inference_api: InferenceApi, api_token: String) -> Self {
        let mut builder = WasiCtxBuilder::new();
        InvocationCtx {
            wasi_ctx: builder.build(),
            resource_table: ResourceTable::new(),
            inference_api,
            api_token,
        }
    }
}

#[async_trait::async_trait]
impl pharia::skill::csi::Host for InvocationCtx {
    #[must_use]
    async fn complete_text(&mut self, prompt: String, model: String) -> String {
        let params = CompleteTextParameters {
            prompt,
            model,
            max_tokens: 10,
        };
        let api_token = self.api_token.clone();
        self.inference_api.complete_text(params, api_token).await
    }
}

impl WasiView for InvocationCtx {
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.resource_table
    }

    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi_ctx
    }
}

pub struct WasmRuntime {
    engine: Engine,
    linker: Linker<InvocationCtx>,
    components: HashMap<String, Component>,
}

impl WasmRuntime {
    pub fn new() -> Self {
        let engine = Engine::new(Config::new().async_support(true).wasm_component_model(true))
            .expect("config must be valid");
        let mut linker: Linker<InvocationCtx> = Linker::new(&engine);
        // provide host implementation of WASI interfaces required by the component with wit-bindgen
        wasmtime_wasi::add_to_linker_async(&mut linker).expect("linking to WASI must work");
        // Skill world from bindgen
        Skill::add_to_linker(&mut linker, |state: &mut InvocationCtx| state)
            .expect("linking to skill world must work");

        let component = Component::from_file(&engine, "./skills/greet_skill.wasm")
            .expect("Loading greet-skill component failed. Please run 'build-skill.sh' first.");

        let mut components = HashMap::new();
        components.insert("greet".to_owned(), component);

        Self {
            engine,
            linker,
            components,
        }
    }
}

impl Runtime for WasmRuntime {
    async fn run(
        &mut self,
        skill: &str,
        name: String,
        api_token: String,
        inference_api: InferenceApi,
    ) -> Result<String, Error> {
        let invocation_ctx = InvocationCtx::new(inference_api, api_token);
        let mut store = Store::new(&self.engine, invocation_ctx);

        let greet_component = self
            .components
            .get(skill)
            .ok_or_else(|| anyhow!("skill not installed"))?;

        let (bindings, _) = Skill::instantiate_async(&mut store, greet_component, &self.linker)
            .await
            .expect("failed to instantiate skill");
        Ok(bindings
            .call_run(&mut store, &name)
            .await
            .expect("need error handling for this"))
    }
}

#[cfg(test)]
pub mod tests {

    use anyhow::{anyhow, Error};

    use crate::{
        inference::{tests::InferenceStub, CompleteTextParameters, InferenceApi},
        skills::runtime::Runtime,
    };

    use super::WasmRuntime;

    pub struct SaboteurRuntime {
        err_msg: String,
    }

    impl SaboteurRuntime {
        pub fn new(err_msg: String) -> Self {
            Self { err_msg }
        }
    }

    impl Runtime for SaboteurRuntime {
        async fn run(
            &mut self,
            _skill: &str,
            _name: String,
            _api_token: String,
            _inference_api: InferenceApi,
        ) -> Result<String, Error> {
            Err(anyhow!(self.err_msg.to_owned()))
        }
    }

    pub struct RustRuntime {}

    impl RustRuntime {
        pub fn new() -> Self {
            Self {}
        }
    }
    impl Runtime for RustRuntime {
        async fn run(
            &mut self,
            skill: &str,
            name: String,
            api_token: String,
            mut inference_api: InferenceApi,
        ) -> Result<String, Error> {
            if skill != "greet" {
                panic!("RustRuntime only supports greet skill")
            }
            let prompt = format!(
                "### Instruction:
                Provide a nice greeting for the person utilizing its given name

                ### Input:
                Name: {name}

                ### Response:"
            );
            let params = CompleteTextParameters {
                prompt,
                model: "luminous-nextgen-7b".to_owned(),
                max_tokens: 10,
            };
            Ok(inference_api.complete_text(params, api_token).await)
        }
    }

    #[tokio::test]
    async fn greet_skill_component() {
        let inference = InferenceStub::new("Hello".to_owned());
        let mut runtime = WasmRuntime::new();
        let resp = runtime
            .run(
                "greet",
                "name".to_owned(),
                "api_token".to_owned(),
                inference.api(),
            )
            .await;

        assert_eq!(resp.unwrap(), "Hello");
    }
    #[tokio::test]
    async fn errors_for_non_existing_skill() {
        let inference = InferenceStub::new("Hello".to_owned());
        let mut runtime = WasmRuntime::new();
        let resp = runtime
            .run(
                "non-existing-skill",
                "name".to_owned(),
                "dummy-token".to_owned(),
                inference.api(),
            )
            .await;
        assert!(resp.is_err());
    }
}
