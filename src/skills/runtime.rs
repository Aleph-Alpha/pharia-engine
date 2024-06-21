use std::future::Future;

use wasmtime::{
    component::{Component, Linker},
    Config, Engine, Store,
};
use wasmtime_wasi::{preview1::WasiP1Ctx, WasiCtxBuilder};

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

#[allow(dead_code)]
pub struct WasmRuntime {
    engine: Engine,
    linker: Linker<WasiP1Ctx>,
    component: Component,
    inference_api: InferenceApi,
}

impl WasmRuntime {
    #[allow(dead_code)]
    pub fn new(inference_api: InferenceApi) -> Self {
        let engine = Engine::new(Config::new().async_support(true)).expect("config must be valid");
        let mut linker = Linker::new(&engine);
        // provide host implementation of WASI interfaces required by the component with wit-bindgen
        wasmtime_wasi::add_to_linker_async(&mut linker).expect("linking to WASI must work");
        linker
            .instance("csi")
            .unwrap()
            .func_wrap_async(
                "complete_text",
                |_store, (_model, _prompt): (String, String)| {
                    Box::new(async move { Ok(("dummy response",)) })
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
    async fn run_greet(&mut self, name: String, api_token: String) -> String {
        let mut builder = WasiCtxBuilder::new();
        let ctx = builder.build_p1();
        let mut store = Store::new(&self.engine, ctx);
        let instance = self
            .linker
            .instantiate_async(&mut store, &self.component)
            .await
            .unwrap();
        let run = instance
            .get_typed_func::<(&str,), (String,)>(&mut store, "run")
            .unwrap();

        let (resp,) = run.call_async(&mut store, ("Pharia",)).await.unwrap();
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
        assert_eq!("Hello, Pharia", resp);
    }
}
