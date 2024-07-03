use std::{collections::HashMap, future::Future};

use anyhow::{anyhow, Error};
use wasmtime::{
    component::{bindgen, Component, Linker},
    Config, Engine, OptLevel, Store,
};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};

use crate::{
    inference::CompleteTextParameters,
    registries::{registries, SkillRegistry},
};

use super::csi::Csi;

// trappable_imports enable failure capturing from host environment (csi functions)
bindgen!({ world: "skill", async: true, trappable_imports: true });

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
        ctx: Box<dyn Csi + Send>,
    ) -> impl Future<Output = Result<String, Error>> + Send;

    fn skills(&self) -> impl Iterator<Item = &str>;
}

struct WasiInvocationCtx {
    wasi_ctx: WasiCtx,
    resource_table: ResourceTable,
    skill_ctx: Box<dyn Csi + Send>,
}

impl WasiInvocationCtx {
    fn new(skill_ctx: Box<dyn Csi + Send>) -> Self {
        let mut builder = WasiCtxBuilder::new();
        WasiInvocationCtx {
            wasi_ctx: builder.build(),
            resource_table: ResourceTable::new(),
            skill_ctx,
        }
    }
}

#[async_trait::async_trait]
impl pharia::skill::csi::Host for WasiInvocationCtx {
    #[must_use]
    async fn complete_text(&mut self, prompt: String, model: String) -> wasmtime::Result<String> {
        let params = CompleteTextParameters {
            prompt,
            model,
            max_tokens: 128,
        };
        self.skill_ctx.complete_text(params).await
    }
}

impl WasiView for WasiInvocationCtx {
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.resource_table
    }

    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi_ctx
    }
}

pub struct WasmRuntime {
    engine: Engine,
    linker: Linker<WasiInvocationCtx>,
    components: HashMap<String, Component>,
    skill_registry: Box<dyn SkillRegistry + Send>,
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
        let mut linker: Linker<WasiInvocationCtx> = Linker::new(&engine);
        // provide host implementation of WASI interfaces required by the component with wit-bindgen
        wasmtime_wasi::add_to_linker_async(&mut linker).expect("linking to WASI must work");
        // Skill world from bindgen
        Skill::add_to_linker(&mut linker, |state: &mut WasiInvocationCtx| state)
            .expect("linking to skill world must work");

        Self {
            engine,
            linker,
            components: HashMap::new(),
            skill_registry: Box::new(skill_registry),
        }
    }

    async fn load_component(&mut self, skill_name: String) -> Result<(), Error> {
        let component = self
            .skill_registry
            .load_skill(&skill_name, &self.engine)
            .await?;
        let component = component.ok_or_else(|| anyhow!("Sorry, skill {skill_name} not found."))?;
        self.components.insert(skill_name, component);
        Ok(())
    }
}

impl Runtime for WasmRuntime {
    async fn run(
        &mut self,
        skill: &str,
        name: String,
        ctx: Box<dyn Csi + Send>,
    ) -> Result<String, Error> {
        let invocation_ctx = WasiInvocationCtx::new(ctx);
        let mut store = Store::new(&self.engine, invocation_ctx);

        let component = if let Some(c) = self.components.get(skill) {
            c
        } else {
            self.load_component(skill.to_owned()).await?;
            self.components.get(skill).unwrap()
        };

        let (bindings, _) = Skill::instantiate_async(&mut store, component, &self.linker)
            .await
            .expect("failed to instantiate skill");
        bindings.call_run(&mut store, &name).await
    }

    fn skills(&self) -> impl Iterator<Item = &str> {
        self.components.keys().map(String::as_ref)
    }
}

#[cfg(test)]
pub mod tests {

    use async_trait::async_trait;
    use std::{collections::HashSet, env, fs, sync::OnceLock};

    use anyhow::{anyhow, bail, Error};
    use dotenvy::dotenv;
    use tempfile::tempdir;

    use crate::{
        inference::{tests::InferenceStub, CompleteTextParameters, Inference},
        registries::FileRegistry,
        skills::{
            csi::{Csi, SkillInvocationCtx},
            runtime::Runtime,
        },
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
            _ctx: Box<dyn Csi + Send>,
        ) -> Result<String, Error> {
            Err(anyhow!(self.err_msg.clone()))
        }

        fn skills(&self) -> impl Iterator<Item = &str> {
            std::iter::empty()
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
            mut ctx: Box<dyn Csi + Send>,
        ) -> Result<String, Error> {
            assert!(skill == "greet", "RustRuntime only supports greet skill");
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
            ctx.complete_text(params).await
        }
        fn skills(&self) -> impl Iterator<Item = &str> {
            std::iter::once("greet")
        }
    }

    /// A test double for a [`Csi`] implementation which returns an error for each method
    /// invocation.
    struct CsiSaboteur;

    #[async_trait]
    impl Csi for CsiSaboteur {
        async fn complete_text(
            &mut self,
            _params: CompleteTextParameters,
        ) -> Result<String, anyhow::Error> {
            bail!("Test error from CSI Saboteur")
        }
    }

    /// A test double for a [`Csi`] implementation which always completes with "Hello".
    struct CsiGreetingStub;

    #[async_trait]
    impl Csi for CsiGreetingStub {
        async fn complete_text(
            &mut self,
            _params: CompleteTextParameters,
        ) -> Result<String, anyhow::Error> {
            Ok("Hello".to_owned())
        }
    }

    #[tokio::test]
    async fn csi_invocation_failure() {
        let skill_ctx = Box::new(CsiSaboteur);
        let mut runtime = WasmRuntime::new();

        let resp = runtime
            .run("greet_skill", "name".to_owned(), skill_ctx)
            .await;
        assert!(resp.is_err());
    }

    #[tokio::test]
    async fn greet_skill_component() {
        let skill_ctx = Box::new(CsiGreetingStub);
        let mut runtime = WasmRuntime::new();
        let resp = runtime
            .run("greet_skill", "name".to_owned(), skill_ctx)
            .await;

        assert_eq!(resp.unwrap(), "Hello");
    }

    #[tokio::test]
    async fn errors_for_non_existing_skill() {
        let skill_ctx = Box::new(CsiGreetingStub);
        let mut runtime = WasmRuntime::new();
        let resp = runtime
            .run("non-existing-skill", "name".to_owned(), skill_ctx)
            .await;
        assert!(resp.is_err());
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
                .run("greet_skill", "name".to_owned(), skill_ctx)
                .await
                .unwrap(),
        );

        let skill_ctx = Box::new(CsiGreetingStub);
        drop(
            runtime
                .run("greet-py", "name".to_owned(), skill_ctx)
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
        let greet = runtime
            .run("greet_skill", "Homer".to_owned(), skill_ctx)
            .await;
        assert!(greet.is_ok());
    }

    static API_TOKEN: OnceLock<String> = OnceLock::new();

    /// API Token used by tests to authenticate requests
    fn api_token() -> &'static str {
        API_TOKEN.get_or_init(|| {
            drop(dotenv());
            env::var("AA_API_TOKEN").expect("AA_API_TOKEN variable not set")
        })
    }

    #[cfg_attr(not(feature = "test_inference"), ignore)]
    #[tokio::test]
    async fn all_greet_skills_are_identical() {
        let inference = Inference::new();
        let mut runtime = WasmRuntime::new();

        let skill_ctx = Box::new(SkillInvocationCtx::new(
            inference.api(),
            api_token().to_owned(),
        ));
        let rust_resp = runtime
            .run("greet_skill", "name".to_owned(), skill_ctx)
            .await
            .unwrap();

        let skill_ctx = Box::new(SkillInvocationCtx::new(
            inference.api(),
            api_token().to_owned(),
        ));
        let python_resp = runtime
            .run("greet-py", "name".to_owned(), skill_ctx)
            .await
            .unwrap();

        assert_eq!(rust_resp, python_resp);
    }
}
