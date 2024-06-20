use std::future::Future;

use wasmtime::{component::Linker, Config, Engine, Store};

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
    fn run_greet(
        &self,
        name: String,
        api_token: String,
        inference_api: &mut InferenceApi,
    ) -> impl Future<Output = String> + Send;
}

#[allow(dead_code)]
pub struct WasmRuntime {
    engine: Engine,
    store: Store<()>,
    linker: Linker<()>,
}

impl WasmRuntime {
    #[allow(dead_code)]
    pub fn new() -> Self {
        let engine = Engine::new(Config::new().async_support(true)).expect("config must be valid");
        let store = Store::new(&engine, ());
        let mut linker = Linker::<()>::new(&engine);
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

        Self {
            engine,
            store,
            linker,
        }
    }
}

pub struct RustRuntime {}

impl RustRuntime {
    pub fn new() -> Self {
        Self {}
    }
}
impl Runtime for RustRuntime {
    async fn run_greet(
        &self,
        name: String,
        api_token: String,
        inference_api: &mut InferenceApi,
    ) -> String {
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
        inference_api.complete_text(params, api_token).await
    }
}

#[cfg(test)]
mod tests {
    use wasmtime::component::Component;

    use super::WasmRuntime;

    #[test]
    fn greet_skill_component() {
        let runtime = WasmRuntime::new();
        Component::from_file(&runtime.engine, "./skills/greet_skill.wasm")
            .expect("Loading greet-skill component failed. Please run 'build-skill.sh' first.");
    }
}
