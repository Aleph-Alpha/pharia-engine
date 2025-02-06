mod v0_2;
mod v0_3;

use std::{
    fmt,
    path::Path,
    sync::LazyLock,
    time::{Duration, Instant},
};

use anyhow::anyhow;
use semver::Version;
use serde::Serialize;
use serde_json::Value;
use strum::{EnumIter, IntoEnumIterator};
use tracing::info;
use utoipa::ToSchema;
use wasmtime::{
    component::{Component, InstancePre, Linker as WasmtimeLinker},
    Config, Engine as WasmtimeEngine, InstanceAllocationStrategy, Memory, MemoryType, OptLevel,
    Store, UpdateDeadline,
};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};
use wit_parser::decoding::{decode, DecodedWasm};

use crate::{csi::CsiForSkills, namespace_watcher::Namespace};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(fake::Dummy))]
pub struct SkillPath {
    pub namespace: Namespace,
    #[cfg_attr(test, dummy(faker = "fake::faker::company::en::Buzzword()"))]
    pub name: String,
}

impl SkillPath {
    pub fn new(namespace: Namespace, name: impl Into<String>) -> Self {
        Self {
            namespace,
            name: name.into(),
        }
    }
}
impl fmt::Display for SkillPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.namespace, self.name)
    }
}

#[derive(ToSchema, Serialize, Debug)]
#[serde(tag = "version")]
pub enum SkillMetadata {
    #[serde(rename = "1")]
    V1(SkillMetadataV1),
}

#[derive(Debug, thiserror::Error)]
pub enum MetadataError {
    #[error("Invalid JSON Schema")]
    InvalidJsonSchema,
}

/// Validated to be valid JSON Schema
#[derive(ToSchema, Serialize, Debug, PartialEq, Eq)]
#[serde(transparent)]
pub struct JsonSchema(Value);

impl TryFrom<Value> for JsonSchema {
    type Error = MetadataError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        if jsonschema::meta::is_valid(&value) {
            Ok(Self(value))
        } else {
            Err(MetadataError::InvalidJsonSchema)
        }
    }
}

#[derive(ToSchema, Serialize, Debug)]
pub struct SkillMetadataV1 {
    pub description: Option<String>,
    pub input_schema: JsonSchema,
    pub output_schema: JsonSchema,
}

#[derive(Debug, thiserror::Error, Clone)]
pub enum SkillError {
    #[error("Failed to pre-instantiate the skill: {0}")]
    SkillPreError(String),
    #[error("Failed to pre-instantiate the component: {0}")]
    LinkerError(String),
    #[error("Failed to instantiate the component: {0}")]
    ComponentError(String),
    #[error("Skill version is missing.")]
    MissingVersion,
    #[error("Skill version {0} is no longer supported by the Kernel. Try upgrading your SDK.")]
    NoLongerSupported(Version),
    #[error("Skill version {0} is not supported by this Kernel installation yet. Try updating your Kernel version or downgrading your SDK.")]
    NotSupportedYet(Version),
    #[error("Error decoding Wasm component: {0}")]
    WasmDecodeError(String),
    #[error("Wasm isn't a component.")]
    NotComponent,
    #[error("Wasm component isn't using Pharia Skill.")]
    NotPhariaSkill,
}

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

    pub fn new(use_pooling_allocator: bool) -> anyhow::Result<Self> {
        let mut config = Config::new();
        config
            .async_support(true)
            .cranelift_opt_level(OptLevel::SpeedAndSize)
            // Allows for cooperative timeslicing in async mode
            .epoch_interruption(true)
            .wasm_component_model(true);

        if use_pooling_allocator && pooling_allocator_is_supported() {
            // For more information on Pooling Allocation, as well as all of possible configuration,
            // read the wasmtime docs: https://docs.rs/wasmtime/latest/wasmtime/struct.PoolingAllocationConfig.html
            config.allocation_strategy(InstanceAllocationStrategy::pooling());
        }

        let engine = WasmtimeEngine::new(&config)?;

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
        SupportedVersion::add_all_to_linker(&mut linker)?;

        Ok(Self {
            inner: engine,
            linker,
        })
    }

    /// Creates a pre-instantiation of a skill. It resolves the imports and does the linking,
    /// but it still needs to be turned into a concrete skill version implementation.
    pub fn instantiate_pre(
        &self,
        bytes: impl AsRef<[u8]>,
    ) -> Result<InstancePre<LinkedCtx>, SkillError> {
        let component = Component::new(&self.inner, bytes)
            .map_err(|e| SkillError::ComponentError(e.to_string()))?;
        self.linker
            .instantiate_pre(&component)
            .map_err(|e| SkillError::LinkerError(e.to_string()))
    }

    /// Generates a store for a specific invocation.
    /// This will yield after every tick, as well as halt execution after `Self::MAX_EXECUTION_TIME`.
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
    /// Skills targeting versions 0.3.x of the skill world
    V0_3(v0_3::SkillPre<LinkedCtx>),
}

impl Skill {
    /// Extracts the version of the skill WIT world from the provided bytes,
    /// and links it to the appropriate version in the linker.
    pub fn new(engine: &Engine, bytes: impl AsRef<[u8]>) -> Result<Self, SkillError> {
        let skill_version = SupportedVersion::extract(&bytes)?;
        let pre = engine.instantiate_pre(&bytes)?;

        match skill_version {
            SupportedVersion::V0_2 => {
                let skill = v0_2::SkillPre::new(pre)
                    .map_err(|e| SkillError::SkillPreError(e.to_string()))?;
                Ok(Skill::V0_2(skill))
            }
            SupportedVersion::V0_3 => {
                let skill = v0_3::SkillPre::new(pre)
                    .map_err(|e| SkillError::SkillPreError(e.to_string()))?;
                Ok(Skill::V0_3(skill))
            }
        }
    }

    pub async fn metadata(
        &self,
        engine: &Engine,
        ctx: Box<dyn CsiForSkills + Send>,
    ) -> anyhow::Result<Option<SkillMetadata>> {
        match self {
            Self::V0_2(_) => Ok(None),
            Self::V0_3(skill) => {
                let mut store = engine.store(LinkedCtx::new(ctx));
                let bindings = skill.instantiate_async(&mut store).await?;
                let metadata = bindings
                    .pharia_skill_skill_handler()
                    .call_metadata(store)
                    .await?;
                Some(metadata.try_into()).transpose()
            }
        }
    }

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
            Self::V0_3(skill) => {
                let input = serde_json::to_vec(&input)?;
                let bindings = skill.instantiate_async(&mut store).await?;
                let result = bindings
                    .pharia_skill_skill_handler()
                    .call_run(store, &input)
                    .await?;
                let result = match result {
                    Ok(result) => result,
                    Err(e) => match e {
                        v0_3::exports::pharia::skill::skill_handler::Error::Internal(e) => {
                            tracing::error!("Failed to run skill, internal skill error:\n{e}");
                            return Err(anyhow!("Internal skill error:\n{e}"));
                        }
                        v0_3::exports::pharia::skill::skill_handler::Error::InvalidInput(e) => {
                            tracing::error!("Failed to run skill, invalid input:\n{e}");
                            return Err(anyhow!("Invalid input:\n{e}"));
                        }
                    },
                };
                Ok(serde_json::from_slice(&result)?)
            }
        }
    }
}

/// Currently supported versions of the skill world
#[derive(Debug, Clone, Copy, EnumIter, PartialEq, Eq)]
pub enum SupportedVersion {
    /// Versions 0.2.x of the skill world
    V0_2,
    /// Versions 0.3.x of the skill world
    V0_3,
}

impl SupportedVersion {
    /// Links all currently supported versions of the skill world to the engine
    fn add_all_to_linker(linker: &mut WasmtimeLinker<LinkedCtx>) -> anyhow::Result<()> {
        for version in Self::iter() {
            match version {
                Self::V0_2 => {
                    v0_2::Skill::add_to_linker(linker, |state: &mut LinkedCtx| state)?;
                }
                Self::V0_3 => {
                    v0_3::Skill::add_to_linker(linker, |state: &mut LinkedCtx| state)?;
                }
            }
        }

        Ok(())
    }

    fn extract(wasm: impl AsRef<[u8]>) -> Result<Self, SkillError> {
        let version = Self::extract_pharia_skill_version(wasm)?;
        Self::validate_version(version)
    }

    fn extract_pharia_skill_version(wasm: impl AsRef<[u8]>) -> Result<Option<Version>, SkillError> {
        let decoded =
            decode(wasm.as_ref()).map_err(|e| SkillError::WasmDecodeError(e.to_string()))?;
        if let DecodedWasm::Component(resolve, ..) = decoded {
            let package_name = &resolve
                .package_names
                .keys()
                .find(|k| (k.namespace == "pharia" && k.name == "skill"))
                .ok_or_else(|| SkillError::NotPhariaSkill)?;
            Ok(package_name.version.clone())
        } else {
            Err(SkillError::NotComponent)
        }
    }

    /// Extracts the package version from a given WIT file.
    /// Path is used for debugging, contents should be the text contents of the WIT file.
    fn extract_wit_package_version(path: impl AsRef<Path>, contents: &str) -> Version {
        let mut resolve = wit_parser::Resolve::new();
        let package_id = resolve
            .push_str(path, contents)
            .expect("Invalid WIT world file");

        resolve
            .packages
            .get(package_id)
            .expect("Package should exist.")
            .name
            .version
            .as_ref()
            .expect("Version should be specified.")
            .clone()
    }

    /// Current latest supported version of a given release line
    fn current_supported_version(self) -> &'static Version {
        match self {
            Self::V0_2 => {
                static VERSION: LazyLock<Version> = LazyLock::new(|| {
                    SupportedVersion::extract_wit_package_version(
                        "./wit/skill@0.2/skill.wit",
                        include_str!("../wit/skill@0.2/skill.wit"),
                    )
                });
                &VERSION
            }
            Self::V0_3 => {
                static VERSION: LazyLock<Version> = LazyLock::new(|| {
                    SupportedVersion::extract_wit_package_version(
                        "./wit/skill@0.3/skill.wit",
                        include_str!("../wit/skill@0.3/skill.wit"),
                    )
                });
                &VERSION
            }
        }
    }

    /// Latest supported version for all supported versions
    pub fn latest_supported_version() -> &'static Version {
        Self::iter()
            .map(SupportedVersion::current_supported_version)
            .max()
            .expect("At least one version.")
    }

    /// Check if a given version is valid
    fn validate_version(version: Option<Version>) -> Result<Self, SkillError> {
        let Some(version) = version else {
            return Err(SkillError::MissingVersion);
        };

        match version {
            Version {
                major: 0, minor: 2, ..
            } => {
                if &version <= Self::V0_2.current_supported_version() {
                    Ok(Self::V0_2)
                } else {
                    Err(SkillError::NotSupportedYet(version))
                }
            }
            Version {
                major: 0, minor: 3, ..
            } => {
                if &version <= Self::V0_3.current_supported_version() {
                    Ok(Self::V0_3)
                } else {
                    Err(SkillError::NotSupportedYet(version))
                }
            }
            _ => {
                if &version > Self::latest_supported_version() {
                    Err(SkillError::NotSupportedYet(version))
                } else {
                    Err(SkillError::NoLongerSupported(version))
                }
            }
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

/// The pooling allocator is tailor made for our use case, so
/// try to use it when we can. The main cost of the pooling allocator, however,
/// is the virtual memory required to run it. Not all systems support the same
/// amount of virtual memory, for example some aarch64 and riscv64 configuration
/// only support 39 bits of virtual address space.
///
/// The pooling allocator, by default, will request 1000 linear memories each
/// sized at 6G per linear memory. This is 6T of virtual memory which ends up
/// being about 42 bits of the address space. This exceeds the 39 bit limit of
/// some systems, so there the pooling allocator will fail by default.
///
/// This function attempts to dynamically determine the hint for the pooling
/// allocator. This returns `true` if the pooling allocator should be used
/// by default, or `false` otherwise.
///
/// The method for testing this is to allocate a 0-sized 64-bit linear memory
/// with a maximum size that's N bits large where we force all memories to be
/// static. This should attempt to acquire N bits of the virtual address space.
/// If successful that should mean that the pooling allocator is OK to use, but
/// if it fails then the pooling allocator is not used and the normal mmap-based
/// implementation is used instead.
///
/// Based on [`wasmtime serve`](https://github.com/bytecodealliance/wasmtime/blob/c42f925f3ab966e8446a807ea3cb59e3251aea5c/src/commands/serve.rs#L641) and [[`spin`](https://github.com/fermyon/spin/blob/2a9bf7c57eda9aa42152f016373d3105170b164b/crates/core/src/lib.rs#L157) implementations
fn pooling_allocator_is_supported() -> bool {
    const BITS_TO_TEST: u32 = 42;
    static USE_POOLING: LazyLock<bool> = LazyLock::new(|| {
        let mut config = Config::new();
        config.wasm_memory64(true);
        config.memory_reservation(1 << BITS_TO_TEST);
        let Ok(engine) = WasmtimeEngine::new(&config) else {
            info!("unable to create an engine to test the pooling allocator, disabling pooling allocation");
            return false;
        };
        let mut store = Store::new(&engine, ());
        // NB: the maximum size is in wasm pages to take out the 16-bits of wasm
        // page size here from the maximum size.
        let ty = MemoryType::new64(0, Some(1 << (BITS_TO_TEST - 16)));
        Memory::new(&mut store, ty).inspect_err(|_| {
            info!("Pooling allocation not supported on this system. Falling back to mmap-based implementation.");
        }).is_ok()
    });
    *USE_POOLING
}

#[cfg(test)]
mod tests {
    use std::fs;

    use fake::{Fake, Faker};
    use serde_json::json;
    use test_skills::{
        given_chat_skill, given_greet_py_v0_2, given_greet_skill_v0_2, given_greet_skill_v0_3,
        given_search_skill,
    };
    use tokio::sync::oneshot;
    use v0_2::pharia::skill::csi::{Host, Language};

    use crate::{
        csi::tests::{CsiGreetingMock, DummyCsi},
        skill_runtime::SkillInvocationCtx,
        tests::api_token,
    };

    use super::*;

    impl SkillPath {
        pub fn dummy() -> Self {
            Faker.fake()
        }

        pub fn local(name: impl Into<String>) -> Self {
            let namespace = Namespace::new("local").unwrap();
            Self {
                namespace,
                name: name.into(),
            }
        }
    }

    impl JsonSchema {
        pub fn dummy() -> Self {
            let schema = json!(
                {
                    "properties": {
                        "topic": {
                            "title": "Topic",
                            "type": "string"
                        }
                    },
                    "required": ["topic"],
                    "title": "Input",
                    "type": "object"
                }
            );
            Self(schema)
        }
    }

    #[test]
    fn can_parse_module() {
        given_greet_skill_v0_2();
        let wasm = fs::read("skills/greet_skill_v0_2.wasm").unwrap();
        let version = SupportedVersion::extract_pharia_skill_version(wasm)
            .unwrap()
            .unwrap();
        assert_eq!(version, Version::new(0, 2, 10));
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

    #[test]
    fn validate_metaschema() {
        let schema = json!({
            "properties": {
                "topic": {
                    "title": "Topic",
                    "type": "string"
                }
            },
            "required": ["topic"],
            "title": "Input",
            "type": "object"
        });
        assert!(JsonSchema::try_from(schema).is_ok());
    }

    #[test]
    fn validate_invalid_schema() {
        let schema = json!("invalid");
        assert!(matches!(
            JsonSchema::try_from(schema).unwrap_err(),
            MetadataError::InvalidJsonSchema
        ));
    }

    #[tokio::test]
    async fn language_selection_from_csi() {
        // Given a linked context
        let (send_rt_err, _) = oneshot::channel();
        let skill_ctx = Box::new(SkillInvocationCtx::new(
            send_rt_err,
            DummyCsi,
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
    async fn can_load_and_run_v0_3_module() {
        // Given a skill loaded by our engine
        let test_skill = given_greet_skill_v0_3();
        let wasm = test_skill.bytes();
        let engine = Engine::new(false).unwrap();
        let skill = Skill::new(&engine, wasm).unwrap();
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
        let engine = Engine::new(false).unwrap();
        let skill = Skill::new(&engine, wasm).unwrap();
        let ctx = Box::new(CsiGreetingMock);

        // When invoked with a json string
        let input = json!("Homer");
        let result = skill.run(&engine, ctx, input).await.unwrap();

        // Then it returns a json string
        assert_eq!(result, json!("Hello Homer"));
    }

    #[tokio::test]
    async fn can_load_and_run_search_skill() {
        // Given a skill loaded by our engine
        given_search_skill();
        let wasm = fs::read("skills/search_skill.wasm").unwrap();
        let engine = Engine::new(false).unwrap();
        let skill = Skill::new(&engine, wasm).unwrap();
        let ctx = Box::new(CsiGreetingMock);

        // When invoked with a json string
        let content = "42";
        let input = json!(content);
        let result = skill.run(&engine, ctx, input).await.unwrap();

        // Then it returns a json string array
        assert_eq!(result, json!([content]));
    }

    #[tokio::test]
    async fn can_load_and_run_chat_skill() {
        // Given a skill loaded by our engine
        given_chat_skill();
        let wasm = fs::read("skills/chat_skill.wasm").unwrap();
        let engine = Engine::new(false).unwrap();
        let skill = Skill::new(&engine, wasm).unwrap();
        let ctx = Box::new(CsiGreetingMock);

        // When invoked with a json string
        let content = "Hello, how are you?";
        let input = json!(content);
        let result = skill.run(&engine, ctx, input).await.unwrap();

        // Then it returns a json string array
        assert_eq!(result["content"], "dummy-content");
    }

    #[tokio::test]
    async fn can_load_and_run_v0_2_py_module() {
        // Given a skill loaded by our engine
        given_greet_py_v0_2();
        let wasm = fs::read("skills/greet-py-v0_2.wasm").unwrap();
        let engine = Engine::new(false).unwrap();
        let skill = Skill::new(&engine, wasm).unwrap();
        let ctx = Box::new(CsiGreetingMock);

        // When invoked with a json string
        let input = json!("Homer");
        let result = skill.run(&engine, ctx, input).await.unwrap();

        // Then it returns a json string
        assert_eq!(result, json!("Hello Homer"));
    }

    #[test]
    fn can_parse_latest_wit_world_version() {
        assert_eq!(
            SupportedVersion::V0_2.current_supported_version(),
            &Version::new(0, 2, 10)
        );
    }

    #[test]
    fn latest_supported_version() {
        assert_eq!(
            SupportedVersion::V0_3.current_supported_version(),
            SupportedVersion::latest_supported_version(),
        );
    }

    #[test]
    fn unsupported_unversioned() {
        let error = SupportedVersion::validate_version(None).unwrap_err();
        assert!(matches!(error, SkillError::MissingVersion));
    }

    #[test]
    fn unsupported_v0_1() {
        let error = SupportedVersion::validate_version(Some(Version::new(0, 1, 0))).unwrap_err();
        assert!(matches!(error, SkillError::NoLongerSupported(..)));
    }

    #[test]
    fn valid_0_2_version() -> anyhow::Result<()> {
        let version = Some(Version::new(0, 2, 0));
        let supported_version = SupportedVersion::validate_version(version)?;
        assert_eq!(supported_version, SupportedVersion::V0_2);
        Ok(())
    }

    #[test]
    fn invalid_0_2_version() {
        let error =
            SupportedVersion::validate_version(Some(Version::new(0, 2, u64::MAX))).unwrap_err();
        assert!(matches!(error, SkillError::NotSupportedYet(..)));
    }

    #[test]
    fn invalid_future_version() {
        let error =
            SupportedVersion::validate_version(Some(Version::new(u64::MAX, u64::MAX, u64::MAX)))
                .unwrap_err();
        assert!(matches!(error, SkillError::NotSupportedYet(..)));
    }
}
