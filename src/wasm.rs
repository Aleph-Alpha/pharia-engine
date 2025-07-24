mod v0_2;
mod v0_3;

use std::{
    path::Path,
    sync::{Arc, LazyLock},
};

use async_trait::async_trait;
use engine_room::LinkerImpl;
use semver::Version;
use serde_json::Value;
use strum::{EnumIter, IntoEnumIterator};
use tokio::sync::mpsc;
use wasmtime::{
    Store,
    component::{InstancePre, Linker},
};
use wit_parser::{
    WorldKey,
    decoding::{DecodedWasm, decode},
};

use crate::{
    csi::Csi,
    logging::TracingContext,
    skill::{AnySkillManifest, BoxedCsi, Skill, SkillError, SkillEvent},
};
use thiserror::Error;
use tracing::error;

/// Wasmtime engine that is configured with linkers for all of the supported versions of
/// our pharia/skill WIT world.
pub struct Engine {
    engine: engine_room::Engine,
    linker: Linker<LinkedCtx>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new(engine_room::EngineConfig::default()).expect("Failed to create default engine")
    }
}

impl Engine {
    pub fn new(engine_config: engine_room::EngineConfig) -> anyhow::Result<Self> {
        let engine = engine_room::Engine::new(engine_config)?;
        // We currently use the same linker for multiple worlds that use CSI.
        // Shadowing allows them to be hooked up twice, but might also be a clue we might want two linkers to avoid the issue.
        let mut linker = engine.new_linker(true)?;
        // Skill world from bindgen
        SupportedSkillWorld::add_all_to_linker(&mut linker)?;

        Ok(Self { engine, linker })
    }

    /// Creates a pre-instantiation of a skill. It resolves the imports and does the linking,
    /// but it still needs to be turned into a concrete skill version implementation.
    pub fn instantiate_pre(
        &self,
        bytes: impl AsRef<[u8]>,
        tracing_context: &TracingContext,
    ) -> Result<InstancePre<LinkedCtx>, SkillLoadError> {
        let component = self.engine.new_component(bytes).map_err(|e| {
            error!(parent: tracing_context.span(), "Failed to create component: {}", e);
            SkillLoadError::ComponentError(e.to_string())
        })?;
        self.linker.instantiate_pre(&component).map_err(|e| {
            error!(parent: tracing_context.span(), "Failed to instantiate component: {}", e);
            SkillLoadError::LinkerError(e.to_string())
        })
    }

    /// Generates a store for a specific invocation.
    /// This will yield after every tick, as well as halt execution after `Self::MAX_EXECUTION_TIME`.
    fn store(&self, skill_ctx: Box<dyn Csi + Send>) -> Store<LinkedCtx> {
        self.engine.store(skill_ctx)
    }
}

/// Failures which occur when loading a skill from Web Assembly bytes.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SkillLoadError {
    #[error("Failed to pre-instantiate the skill: {0}")]
    SkillPreError(String),
    #[error("Failed to pre-instantiate the component: {0}")]
    LinkerError(String),
    #[error("Failed to instantiate the component: {0}")]
    ComponentError(String),
    #[error("Skill version {0} is no longer supported by the Kernel. Try upgrading your SDK.")]
    NoLongerSupported(String),
    #[error(
        "Skill version {0} is not supported by this Kernel installation yet. Try updating your \
        Kernel version or downgrading your SDK."
    )]
    NotSupportedYet(String),
    #[error("Error decoding Wasm component: {0}")]
    WasmDecodeError(String),
    #[error("Web Assembly is not a component.")]
    NotComponent,
    #[error("Web assembly component is not using a supported pharia:skill world.")]
    UnsupportedWorld,
}

type LinkedCtx = LinkerImpl<Box<dyn Csi + Send>>;

/// WebAssembly implementation of the Skill interface.
struct WasmSkill<T> {
    engine: Arc<Engine>,
    pre: T,
}

impl<T> WasmSkill<T> {
    pub fn new(engine: Arc<Engine>, pre: T) -> Self {
        Self { engine, pre }
    }
}

/// What we expect from WASM components that want to be executed by our runtime.
trait SkillComponent {
    fn manifest(
        &self,
        engine: &Engine,
        ctx: BoxedCsi,
        tracing_context: &TracingContext,
    ) -> impl Future<Output = Result<AnySkillManifest, SkillError>> + Send;

    fn run_as_function(
        &self,
        engine: &Engine,
        ctx: BoxedCsi,
        input: Value,
        tracing_context: &TracingContext,
    ) -> impl Future<Output = Result<Value, SkillError>> + Send;

    fn run_as_message_stream(
        &self,
        engine: &Engine,
        ctx: BoxedCsi,
        input: Value,
        sender: mpsc::Sender<SkillEvent>,
        tracing_context: &TracingContext,
    ) -> impl Future<Output = Result<(), SkillError>> + Send;
}

#[async_trait]
impl<T> Skill for WasmSkill<T>
where
    T: SkillComponent + Send + Sync,
{
    async fn manifest(
        &self,
        _ctx: BoxedCsi,
        _tracing_context: &TracingContext,
    ) -> Result<AnySkillManifest, SkillError> {
        self.pre
            .manifest(&self.engine, _ctx, _tracing_context)
            .await
    }

    async fn run_as_function(
        &self,
        ctx: BoxedCsi,
        input: Value,
        tracing_context: &TracingContext,
    ) -> Result<Value, SkillError> {
        self.pre
            .run_as_function(&self.engine, ctx, input, tracing_context)
            .await
    }

    async fn run_as_message_stream(
        &self,
        ctx: BoxedCsi,
        input: Value,
        sender: mpsc::Sender<SkillEvent>,
        tracing_context: &TracingContext,
    ) -> Result<(), SkillError> {
        self.pre
            .run_as_message_stream(&self.engine, ctx, input, sender, tracing_context)
            .await
    }
}

/// Factory for creating skills. Responsible for inspecting the skill bytes, and instantiatnig the
/// right Skill type. The skill is pre-initialized i.e. already attached to their corresponding
/// linker. This allows for as much initialization work to be done at load time as possible, which
/// can be cached across multiple invocations.
pub fn load_skill_from_wasm_bytes(
    engine: Arc<Engine>,
    bytes: impl AsRef<[u8]>,
    tracing_context: TracingContext,
) -> Result<Box<dyn Skill>, SkillLoadError> {
    let skill_world = SupportedSkillWorld::extract(&bytes)?;
    let pre = engine.instantiate_pre(&bytes, &tracing_context)?;

    match skill_world {
        SupportedSkillWorld::V0_2Function => {
            let skill_pre = v0_2::SkillPre::new(pre).map_err(|e| {
                error!(parent: tracing_context.span(), "Failed to create skill from pre-instantiation");
                SkillLoadError::SkillPreError(e.to_string())
            })?;
            let skill = WasmSkill::new(engine, skill_pre);
            Ok(Box::new(skill))
        }
        SupportedSkillWorld::V0_3Function => {
            let skill_pre = v0_3::skill::SkillPre::new(pre)
                .map_err(|e| SkillLoadError::SkillPreError(e.to_string()))?;
            let skill = WasmSkill::new(engine, skill_pre);
            Ok(Box::new(skill))
        }
        SupportedSkillWorld::V0_3MessageStream => {
            let skill_pre = v0_3::message_stream_skill::MessageStreamSkillPre::new(pre)
                .map_err(|e| SkillLoadError::SkillPreError(e.to_string()))?;
            let skill = WasmSkill::new(engine, skill_pre);
            Ok(Box::new(skill))
        }
    }
}

/// Currently supported versions of the skill world
#[derive(Debug, Clone, Copy, EnumIter, PartialEq, Eq)]
pub enum SupportedSkillWorld {
    /// Versions 0.2.x of the skill world
    V0_2Function,
    /// Versions 0.3.x of the skill world
    V0_3Function,
    /// Versions 0.3.x of the streaming-skill world
    V0_3MessageStream,
}

impl SupportedSkillWorld {
    /// Links all currently supported versions of the skill world to the engine
    fn add_all_to_linker(linker: &mut Linker<LinkedCtx>) -> anyhow::Result<()> {
        for version in Self::iter() {
            match version {
                Self::V0_2Function => {
                    v0_2::Skill::add_to_linker(linker, |state: &mut LinkedCtx| state)?;
                }
                Self::V0_3Function => {
                    v0_3::skill::Skill::add_to_linker(linker, |state: &mut LinkedCtx| state)?;
                }
                Self::V0_3MessageStream => {
                    v0_3::message_stream_skill::MessageStreamSkill::add_to_linker(
                        linker,
                        |state: &mut LinkedCtx| state,
                    )?;
                }
            }
        }

        Ok(())
    }

    fn extract(wasm: impl AsRef<[u8]>) -> Result<SupportedSkillWorld, SkillLoadError> {
        let decoded =
            decode(wasm.as_ref()).map_err(|e| SkillLoadError::WasmDecodeError(e.to_string()))?;
        let DecodedWasm::Component(resolve, ..) = decoded else {
            return Err(SkillLoadError::NotComponent);
        };

        // Decoding library should export a "root" world as the target world for the component.
        let root_world = resolve
            .worlds
            .into_iter()
            .find(|(_, world)| world.name == "root")
            .map(|(_, world)| world)
            .expect("Root world should exist");

        // Exports from the component that come from pharia:skill
        let exported_interfaces = root_world
            .exports
            .into_iter()
            // Filter to just exported interfaces
            .filter_map(|(key, _)| match key {
                WorldKey::Name(_) => None,
                WorldKey::Interface(id) => resolve.interfaces.get(id),
            })
            // Filter to interfaces with associated packages
            .filter_map(|interface| {
                interface
                    .package
                    .and_then(|package| resolve.packages.get(package))
                    .map(|package| (&package.name, interface))
            })
            // Filter to interfaces with names
            .filter_map(|(package_name, interface)| {
                interface.name.as_ref().map(|name| (package_name, name))
            })
            // Only keep interfaces from the pharia:skill package with a version
            .filter_map(|(package, name)| {
                package.version.as_ref().and_then(|version| {
                    (package.namespace == "pharia" && package.name == "skill")
                        .then_some((version, name))
                })
            });

        for (package_version, interface_name) in exported_interfaces {
            let supported_version = SupportedVersion::try_from(package_version)?;
            // export pharia:skill/skill-handler@X.X.X
            match (supported_version, interface_name.as_str()) {
                (SupportedVersion::V0_2, "skill-handler") => {
                    return Ok(SupportedSkillWorld::V0_2Function);
                }
                (SupportedVersion::V0_3, "skill-handler") => {
                    return Ok(SupportedSkillWorld::V0_3Function);
                }
                (SupportedVersion::V0_3, "message-stream") => {
                    return Ok(SupportedSkillWorld::V0_3MessageStream);
                }
                _ => {}
            }
        }

        // We didn't find an expected export
        Err(SkillLoadError::UnsupportedWorld)
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

impl TryFrom<&Version> for SupportedVersion {
    type Error = SkillLoadError;

    fn try_from(version: &Version) -> Result<Self, Self::Error> {
        match version {
            Version {
                major: 0, minor: 2, ..
            } => {
                if version <= Self::V0_2.current_supported_version() {
                    Ok(Self::V0_2)
                } else {
                    Err(SkillLoadError::NotSupportedYet(version.to_string()))
                }
            }
            Version {
                major: 0, minor: 3, ..
            } => {
                if version <= Self::V0_3.current_supported_version() {
                    Ok(Self::V0_3)
                } else {
                    Err(SkillLoadError::NotSupportedYet(version.to_string()))
                }
            }
            _ => {
                if version > Self::latest_supported_version() {
                    Err(SkillLoadError::NotSupportedYet(version.to_string()))
                } else {
                    Err(SkillLoadError::NoLongerSupported(version.to_string()))
                }
            }
        }
    }
}

impl SupportedVersion {
    /// Extracts the package version from a given WIT file.
    /// Path is used for debugging, contents should be the text contents of the WIT file.
    fn extract_wit_package_version(path: impl AsRef<Path>) -> Version {
        let mut resolve = wit_parser::Resolve::new();
        let (package_id, _) = resolve.push_path(path).expect("Invalid WIT world file");

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
                    SupportedVersion::extract_wit_package_version("./wit/skill@0.2")
                });
                &VERSION
            }
            Self::V0_3 => {
                static VERSION: LazyLock<Version> = LazyLock::new(|| {
                    SupportedVersion::extract_wit_package_version("./wit/skill@0.3")
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
}

#[cfg(test)]
pub mod tests {
    use async_trait::async_trait;
    use double_trait::Dummy;
    use serde_json::json;
    use test_skills::{
        given_chat_stream_skill, given_complete_stream_skill, given_invalid_output_skill,
        given_python_skill_greet_v0_2, given_python_skill_greet_v0_3, given_rust_skill_chat,
        given_rust_skill_complete_with_echo, given_rust_skill_explain, given_rust_skill_greet_v0_2,
        given_rust_skill_greet_v0_3, given_rust_skill_search, given_skill_tool_invocation,
        given_streaming_output_skill,
    };

    use crate::{
        csi::{
            CsiDouble, CsiError, ToolResult,
            tests::{
                ContextualCsiDouble, CsiChatStreamStub, CsiChatStub, CsiCompleteStreamStub,
                CsiCompleteWithEchoMock, CsiGreetingMock, CsiSearchMock,
            },
        },
        inference::{
            ChatEvent, CompletionEvent, Explanation, ExplanationRequest, FinishReason, TextScore,
            TokenUsage,
        },
        logging::TracingContext,
        skill_driver::SkillInvocationCtx,
        tool::{InvokeRequest, ToolDescription, ToolOutput},
    };

    use super::*;

    #[tokio::test]
    async fn python_greeting_skill() {
        let skill = given_python_skill_greet_v0_3();
        let skill_ctx = Box::new(CsiGreetingMock);
        let engine = Arc::new(Engine::default());

        let skill =
            load_skill_from_wasm_bytes(engine, skill.bytes(), TracingContext::dummy()).unwrap();
        let actual = skill
            .run_as_function(skill_ctx, json!("Homer"), &TracingContext::dummy())
            .await
            .unwrap();

        assert_eq!(actual, "Hello Homer");
    }

    #[tokio::test]
    async fn rust_greeting_skill() {
        let skill_bytes = given_rust_skill_greet_v0_2().bytes();
        let engine = Arc::new(Engine::default());

        let skill =
            load_skill_from_wasm_bytes(engine, skill_bytes, TracingContext::dummy()).unwrap();
        let actual = skill
            .run_as_function(
                Box::new(CsiGreetingMock),
                json!("Homer"),
                &TracingContext::dummy(),
            )
            .await
            .unwrap();

        assert_eq!(actual, "Hello Homer");
    }

    #[tokio::test]
    async fn explain_skill_component() {
        struct ContextualCsiStub;
        impl ContextualCsiDouble for ContextualCsiStub {
            async fn explain(
                &self,
                _requests: Vec<ExplanationRequest>,
            ) -> Result<Vec<Explanation>, CsiError> {
                Ok(vec![Explanation::new(vec![TextScore {
                    score: 0.0,
                    start: 0,
                    length: 2,
                }])])
            }
        }

        let skill_bytes = given_rust_skill_explain().bytes();
        let (send, _) = mpsc::channel(1);
        let engine = Arc::new(Engine::default());
        let ctx = Box::new(SkillInvocationCtx::new(send, ContextualCsiStub));
        let skill =
            load_skill_from_wasm_bytes(engine, skill_bytes, TracingContext::dummy()).unwrap();
        let actual = skill
            .run_as_function(
                ctx,
                json!({"prompt": "An apple a day", "target": " keeps the doctor away"}),
                &TracingContext::dummy(),
            )
            .await
            .unwrap();

        assert_eq!(actual, json!([{"start": 0, "length": 2}]));
    }

    #[tokio::test]
    async fn skill_metadata_v0_2_is_empty() {
        // Given a skill is linked againts skill package v0.2
        let test_skill = given_rust_skill_greet_v0_2();
        let engine = Arc::new(Engine::default());
        let skill = load_skill_from_wasm_bytes(engine, test_skill.bytes(), TracingContext::dummy())
            .unwrap();

        // When metadata for a skill is requested
        let metadata = skill
            .manifest(Box::new(Dummy), &TracingContext::dummy())
            .await
            .unwrap();

        // Then the metadata is the empty V0, because v0.2 had no metadata
        assert!(matches!(metadata, AnySkillManifest::V0));
    }

    #[tokio::test]
    async fn skill_metadata_invalid_output() {
        // Given a skill runtime that always returns an invalid output skill
        let skill_bytes = given_invalid_output_skill().bytes();
        let engine = Arc::new(Engine::default());
        let skill =
            load_skill_from_wasm_bytes(engine, skill_bytes, TracingContext::dummy()).unwrap();

        // When metadata for a skill is requested
        let metadata_result = skill
            .manifest(Box::new(Dummy), &TracingContext::dummy())
            .await;

        // Then the metadata gives an error
        assert!(metadata_result.is_err());
    }

    #[test]
    fn can_parse_module() {
        let wasm = given_rust_skill_greet_v0_2().bytes();
        let world = SupportedSkillWorld::extract(wasm).unwrap();
        assert_eq!(world, SupportedSkillWorld::V0_2Function);
    }

    #[test]
    fn errors_if_not_pharia_component() {
        let wasm = wat::parse_str("(component)").unwrap();
        let world = SupportedSkillWorld::extract(wasm);
        assert!(world.is_err());
    }

    #[test]
    fn errors_if_not_component() {
        let wasm = wat::parse_str("(module)").unwrap();
        let version = SupportedSkillWorld::extract(wasm);
        assert!(version.is_err());
    }

    #[tokio::test]
    async fn can_load_and_run_v0_3_module() {
        // Given a skill loaded by our engine
        let test_skill = given_rust_skill_greet_v0_3();
        let wasm = test_skill.bytes();
        let engine = Arc::new(Engine::default());
        let skill = load_skill_from_wasm_bytes(engine, wasm, TracingContext::dummy()).unwrap();
        let ctx = Box::new(CsiGreetingMock);

        // When invoked with a json string
        let input = json!("Homer");
        let result = skill
            .run_as_function(ctx, input, &TracingContext::dummy())
            .await
            .unwrap();

        // Then it returns a json string
        assert_eq!(result, json!("Hello Homer"));
    }

    #[tokio::test]
    async fn skill_requests_completion_with_echo() {
        // Given a skill loaded by our engine
        let test_skill = given_rust_skill_complete_with_echo();
        let wasm = test_skill.bytes();
        let engine = Arc::new(Engine::default());
        let skill = load_skill_from_wasm_bytes(engine, wasm, TracingContext::dummy()).unwrap();
        let ctx = Box::new(CsiCompleteWithEchoMock);

        // When invoked with a json string
        let input = json!("");
        let result = skill
            .run_as_function(ctx, input, &TracingContext::dummy())
            .await
            .unwrap();

        // Then it returns a json string
        assert_eq!(result, json!("An apple a day"));
    }

    #[tokio::test]
    async fn can_load_and_run_v0_2_module() {
        // Given a skill loaded by our engine
        let test_skills = given_rust_skill_greet_v0_2();
        let wasm = test_skills.bytes();
        let engine = Arc::new(Engine::default());
        let skill = load_skill_from_wasm_bytes(engine, wasm, TracingContext::dummy()).unwrap();
        let ctx = Box::new(CsiGreetingMock);

        // When invoked with a json string
        let input = json!("Homer");
        let result = skill
            .run_as_function(ctx, input, &TracingContext::dummy())
            .await
            .unwrap();

        // Then it returns a json string
        assert_eq!(result, json!("Hello Homer"));
    }

    #[tokio::test]
    async fn can_load_and_run_search_skill() {
        // Given a skill loaded by our engine
        let wasm = given_rust_skill_search().bytes();
        let engine = Arc::new(Engine::default());
        let skill = load_skill_from_wasm_bytes(engine, wasm, TracingContext::dummy()).unwrap();
        let ctx = Box::new(CsiSearchMock);

        // When invoked with a json string
        let content = "42";
        let input = json!(content);
        let result = skill
            .run_as_function(ctx, input, &TracingContext::dummy())
            .await
            .unwrap();

        // Then it returns a json string array
        assert_eq!(result, json!([content]));
    }

    #[tokio::test]
    async fn can_load_and_run_completion_stream_module() {
        // Given a skill loaded by our engine
        let events = vec![
            CompletionEvent::Append {
                text: "Homer".to_owned(),
                logprobs: vec![],
            },
            CompletionEvent::End {
                finish_reason: FinishReason::Stop,
            },
            CompletionEvent::Usage {
                usage: TokenUsage {
                    prompt: 1,
                    completion: 2,
                },
            },
        ];
        let test_skill = given_complete_stream_skill();
        let wasm = test_skill.bytes();
        let engine = Arc::new(Engine::default());
        let skill = load_skill_from_wasm_bytes(engine, wasm, TracingContext::dummy()).unwrap();
        let ctx = Box::new(CsiCompleteStreamStub::new(events));

        // When invoked with a json string
        let input = json!("Homer");
        let result = skill
            .run_as_function(ctx, input, &TracingContext::dummy())
            .await
            .unwrap();

        // Then it returns a json string
        assert_eq!(
            result,
            json!(["Homer", "FinishReason::Stop", "prompt: 1, completion: 2"])
        );
    }

    #[tokio::test]
    async fn can_load_and_run_chat_stream_module() {
        // Given a skill loaded by our engine
        let events = vec![
            ChatEvent::MessageBegin {
                role: "assistant".to_owned(),
            },
            ChatEvent::MessageAppend {
                content: "Homer".to_owned(),
                logprobs: vec![],
            },
            ChatEvent::MessageEnd {
                finish_reason: FinishReason::Stop,
            },
            ChatEvent::Usage {
                usage: TokenUsage {
                    prompt: 1,
                    completion: 2,
                },
            },
        ];
        let test_skill = given_chat_stream_skill();
        let wasm = test_skill.bytes();
        let engine = Arc::new(Engine::default());
        let skill = load_skill_from_wasm_bytes(engine, wasm, TracingContext::dummy()).unwrap();
        let ctx = Box::new(CsiChatStreamStub::new(events));

        // When invoked with a json string
        let result = skill
            .run_as_function(
                ctx,
                json!({"model": "pharia-1-llm-7b-control", "query": "Homer"}),
                &TracingContext::dummy(),
            )
            .await
            .unwrap();

        // Then it returns a json string
        assert_eq!(
            result["events"],
            json!([
                "assistant",
                "Homer",
                "FinishReason::Stop",
                "prompt: 1, completion: 2"
            ])
        );
    }

    #[tokio::test]
    async fn can_load_and_run_streaming_output_module() {
        // Given a skill loaded by our engine
        let events = vec![
            ChatEvent::MessageBegin {
                role: "assistant".to_owned(),
            },
            ChatEvent::MessageAppend {
                content: "Homer".to_owned(),
                logprobs: vec![],
            },
            ChatEvent::MessageEnd {
                finish_reason: FinishReason::Stop,
            },
            ChatEvent::Usage {
                usage: TokenUsage {
                    prompt: 1,
                    completion: 2,
                },
            },
        ];
        let test_skill = given_streaming_output_skill();
        let wasm = test_skill.bytes();
        let engine = Arc::new(Engine::default());
        let skill = load_skill_from_wasm_bytes(engine, wasm, TracingContext::dummy()).unwrap();
        let ctx = Box::new(CsiChatStreamStub::new(events));
        let (send, mut recv) = mpsc::channel(3);

        // When invoked with a json string
        skill
            .run_as_message_stream(ctx, json!("Homer"), send, &TracingContext::dummy())
            .await
            .unwrap();

        // Then it returns a json string
        let mut events = vec![];
        recv.recv_many(&mut events, 3).await;

        assert_eq!(
            events,
            vec![
                SkillEvent::MessageBegin,
                SkillEvent::MessageAppend {
                    text: "Homer".to_owned()
                },
                SkillEvent::MessageEnd {
                    payload: json!("FinishReason::Stop")
                }
            ]
        );
    }

    #[tokio::test]
    async fn can_load_and_run_chat_skill() {
        // Given a skill loaded by our engine
        let wasm = given_rust_skill_chat().bytes();
        let engine = Arc::new(Engine::default());
        let skill = load_skill_from_wasm_bytes(engine, wasm, TracingContext::dummy()).unwrap();
        let ctx = Box::new(CsiChatStub);

        // When invoked with a json string
        let content = "Hello, how are you?";
        let input = json!(content);
        let result = skill
            .run_as_function(ctx, input, &TracingContext::dummy())
            .await
            .unwrap();

        // Then it returns a json string array
        assert_eq!(result["content"], "dummy-content");
    }

    #[tokio::test]
    async fn can_load_and_run_v0_2_py_module() {
        // Given a skill loaded by our engine
        let skill = given_python_skill_greet_v0_2();
        let engine = Arc::new(Engine::default());
        let skill =
            load_skill_from_wasm_bytes(engine, skill.bytes(), TracingContext::dummy()).unwrap();
        let ctx = Box::new(CsiGreetingMock);

        // When invoked with a json string
        let input = json!("Homer");
        let result = skill
            .run_as_function(ctx, input, &TracingContext::dummy())
            .await
            .unwrap();

        // Then it returns a json string
        assert_eq!(result, json!("Hello Homer"));
    }

    #[tokio::test]
    async fn can_load_and_run_v0_3_py_module() {
        // Given a skill loaded by our engine
        let skill = given_python_skill_greet_v0_3();
        let engine = Arc::new(Engine::default());
        let skill =
            load_skill_from_wasm_bytes(engine, skill.bytes(), TracingContext::dummy()).unwrap();
        let ctx = Box::new(CsiGreetingMock);

        // When invoked with a json string
        let input = json!("Homer");
        let result = skill
            .run_as_function(ctx, input, &TracingContext::dummy())
            .await
            .unwrap();

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
    fn unsupported_v0_1() {
        let error = SupportedVersion::try_from(&Version::new(0, 1, 0)).unwrap_err();
        assert!(matches!(error, SkillLoadError::NoLongerSupported(..)));
    }

    #[test]
    fn valid_0_2_version() -> anyhow::Result<()> {
        let version = Version::new(0, 2, 0);
        let supported_version = SupportedVersion::try_from(&version)?;
        assert_eq!(supported_version, SupportedVersion::V0_2);
        Ok(())
    }

    #[test]
    fn invalid_0_2_version() {
        let error = SupportedVersion::try_from(&Version::new(0, 2, u64::MAX)).unwrap_err();
        assert!(matches!(error, SkillLoadError::NotSupportedYet(..)));
    }

    #[test]
    fn invalid_future_version() {
        let error =
            SupportedVersion::try_from(&Version::new(u64::MAX, u64::MAX, u64::MAX)).unwrap_err();
        assert!(matches!(error, SkillLoadError::NotSupportedYet(..)));
    }

    /// Learning test to verify nothing strange happens if a instantiated skill is invoked multiple times.
    #[tokio::test]
    async fn can_call_pre_instantiated_multiple_times() {
        let test_skill = given_rust_skill_greet_v0_2();
        let wasm = test_skill.bytes();
        let engine = Arc::new(Engine::default());
        let skill = load_skill_from_wasm_bytes(engine, wasm, TracingContext::dummy()).unwrap();
        let ctx = Box::new(CsiGreetingMock);

        // When invoked with a json string
        let input = json!("Homer");
        let first_result = skill
            .run_as_function(ctx.clone(), input.clone(), &TracingContext::dummy())
            .await
            .unwrap();
        let second_result = skill
            .run_as_function(ctx.clone(), input.clone(), &TracingContext::dummy())
            .await
            .unwrap();
        let third_result = skill
            .run_as_function(ctx.clone(), input.clone(), &TracingContext::dummy())
            .await
            .unwrap();

        // Then it returns a json string
        assert_eq!(first_result, json!("Hello Homer"));
        assert_eq!(second_result, json!("Hello Homer"));
        assert_eq!(third_result, json!("Hello Homer"));
    }

    struct CsiAddToolFake;

    #[async_trait]
    impl CsiDouble for CsiAddToolFake {
        async fn invoke_tool(&mut self, requests: Vec<InvokeRequest>) -> Vec<ToolResult> {
            requests
                .iter()
                .map(|request| {
                    let a = String::from_utf8(request.arguments[0].value.clone())
                        .unwrap()
                        .parse::<i32>()
                        .unwrap();
                    let b = String::from_utf8(request.arguments[1].value.clone())
                        .unwrap()
                        .parse::<i32>()
                        .unwrap();
                    let sum = a + b;
                    Ok(ToolOutput::from_text(sum.to_string()))
                })
                .collect()
        }

        async fn list_tools(&mut self) -> Vec<ToolDescription> {
            vec![ToolDescription::new(
                "add",
                "Add two numbers",
                json!({
                    "type": "object",
                    "properties": {
                        "a": { "type": "number" },
                        "b": { "type": "number" }
                    }
                }),
            )]
        }
    }

    #[tokio::test]
    async fn tool_invocation_skill() {
        // given a skill that calls a tool
        let skill_bytes = given_skill_tool_invocation().bytes();
        let engine = Arc::new(Engine::default());
        let skill =
            load_skill_from_wasm_bytes(engine, skill_bytes, TracingContext::dummy()).unwrap();

        // when the skill is run
        let response = skill
            .run_as_function(
                Box::new(CsiAddToolFake),
                json!({ "a": 1, "b": 2 }),
                &TracingContext::dummy(),
            )
            .await
            .unwrap();

        // then the response is equal to expected text
        let sum = response.as_number().unwrap().as_i64().unwrap();
        assert_eq!(sum, 3);
    }

    #[tokio::test]
    async fn skill_metadata_v0_3() {
        // Given a skill runtime api that always returns a v0.3 skill
        let skill_bytes = given_rust_skill_greet_v0_3().bytes();
        let engine = Arc::new(Engine::default());
        let skill =
            load_skill_from_wasm_bytes(engine, skill_bytes, TracingContext::dummy()).unwrap();

        // When metadata for a skill is requested
        let metadata = skill
            .manifest(Box::new(Dummy), &TracingContext::dummy())
            .await
            .unwrap();

        // Then the metadata is returned
        assert_eq!(metadata.description().unwrap(), "A friendly greeting skill");
        assert_eq!(
            metadata.signature().unwrap().input_schema(),
            &json!({"type": "string", "description": "The name of the person to greet"})
                .try_into()
                .unwrap()
        );
        assert_eq!(
            metadata.signature().unwrap().output_schema().unwrap(),
            &json!({"type": "string", "description": "A friendly greeting message"})
                .try_into()
                .unwrap()
        );
    }
}
