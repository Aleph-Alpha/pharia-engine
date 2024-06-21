use std::future::Future;

use wasmtime::{
    component::{Component, Linker},
    Config, Engine, Store,
};
use wasmtime_wasi::{preview1::WasiP1Ctx, WasiCtxBuilder, WasiView};

use crate::inference::{CompleteTextParameters, InferenceApi};

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
    fn run_greet(&mut self, name: String, api_token: String)
        -> impl Future<Output = String> + Send;
}

struct InvocationCtx {
    wasi_ctx: WasiP1Ctx,
    inference_api: InferenceApi,
}

impl InvocationCtx {
    fn new(inference_api: InferenceApi) -> Self {
        let mut builder = WasiCtxBuilder::new();
        InvocationCtx {
            wasi_ctx: builder.build_p1(),
            inference_api,
        }
    }
}

impl WasiView for InvocationCtx {
    fn table(&mut self) -> &mut wasmtime_wasi::ResourceTable {
        self.wasi_ctx.table()
    }

    fn ctx(&mut self) -> &mut wasmtime_wasi::WasiCtx {
        self.wasi_ctx.ctx()
    }
}

#[allow(dead_code)]
pub struct WasmRuntime {
    engine: Engine,
    linker: Linker<InvocationCtx>,
    component: Component,
    inference_api: InferenceApi,
}

impl WasmRuntime {
    #[allow(dead_code)]
    pub fn new(inference_api: InferenceApi) -> Self {
        let engine = Engine::new(Config::new().async_support(true)).expect("config must be valid");
        let mut linker: Linker<InvocationCtx> = Linker::new(&engine);
        // provide host implementation of WASI interfaces required by the component with wit-bindgen
        wasmtime_wasi::add_to_linker_async(&mut linker).expect("linking to WASI must work");
        linker
            .instance("pharia:skill/csi")
            .unwrap()
            .func_wrap_async(
                "complete-text",
                move |mut store, (prompt, model): (String, String)| {
                    let params = CompleteTextParameters {
                        prompt,
                        model,
                        max_tokens: 10,
                    };
                    let mut inference_api_cloned = store.data_mut().inference_api.clone();
                    Box::new(async move {
                        Ok((inference_api_cloned
                            .complete_text(params, "dummy token".to_owned())
                            .await,))
                    })
                },
            )
            .unwrap();
        let component = Component::from_file(&engine, "./skills/greet_skill.wasm")
            .expect("Loading greet-skill component failed. Please run 'build-skill.sh' first.");

        Self {
            engine,
            linker,
            component,
            inference_api,
        }
    }
}

impl Runtime for WasmRuntime {
    async fn run_greet(&mut self, name: String, _api_token: String) -> String {
        let invocation_ctx = InvocationCtx::new(self.inference_api.clone());
        let mut store = Store::new(&self.engine, invocation_ctx);
        let instance = self
            .linker
            .instantiate_async(&mut store, &self.component)
            .await
            .unwrap();
        let run = instance
            .get_typed_func::<(&str,), (String,)>(&mut store, "run")
            .unwrap();

        let (resp,) = run.call_async(&mut store, (&name,)).await.unwrap();
        run.post_return_async(&mut store).await.unwrap();
        resp
    }
}

pub struct RustRuntime {
    inference_api: InferenceApi,
}

impl RustRuntime {
    pub fn new(inference_api: InferenceApi) -> Self {
        Self { inference_api }
    }
}
impl Runtime for RustRuntime {
    async fn run_greet(&mut self, name: String, api_token: String) -> String {
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
        self.inference_api.complete_text(params, api_token).await
    }
}

#[cfg(test)]
mod tests {

    use crate::{inference::tests::InferenceStub, skills::runtime::Runtime};

    use super::WasmRuntime;

    #[tokio::test]
    async fn greet_skill_component() {
        let inference = InferenceStub::new("Hello".to_owned());
        let mut runtime = WasmRuntime::new(inference.api());
        let resp = runtime
            .run_greet("name".to_owned(), "api_token".to_owned())
            .await;
        assert_eq!("Hello", resp);
    }
}
