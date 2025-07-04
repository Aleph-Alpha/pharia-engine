use crate::{csi::Csi, logging::TracingContext, namespace_watcher::Namespace};
use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use std::fmt;
use tokio::sync::mpsc;
use utoipa::ToSchema;

#[cfg(test)]
use double_trait::double;

/// Unique identifier for a Skill.
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

/// A Skill encapsulates business logic to solve a particular problem.
///
/// It interacts with the world through the Cognitive System Interface (CSI).
/// While we support skills that are written in WASM, there are also other implementations
/// like [`crate::hardcoded_skills::hardcoded_skill`] that are also supported.
#[async_trait]
#[cfg_attr(test, double(SkillDouble))]
pub trait Skill: Send + Sync {
    async fn manifest(
        &self,
        ctx: Box<dyn Csi + Send>,
        tracing_context: &TracingContext,
    ) -> Result<AnySkillManifest, SkillError>;

    async fn run_as_function(
        &self,
        ctx: Box<dyn Csi + Send>,
        input: Value,
        tracing_context: &TracingContext,
    ) -> Result<Value, SkillError>;

    async fn run_as_message_stream(
        &self,
        ctx: Box<dyn Csi + Send>,
        input: Value,
        sender: mpsc::Sender<SkillEvent>,
        tracing_context: &TracingContext,
    ) -> Result<(), SkillError>;
}

/// An event emitted by the business logic living in the skill. As opposed to an event emitted by
/// executing the skill. The difference is that the former can not contain runtime errors, and is
/// not checked yet for invalid state transitions. Example of invalid state transition would be
/// starting a message with end rather than start.
#[derive(Debug, PartialEq, Eq)]
pub enum SkillEvent {
    /// Send at the beginning of each message, currently carries no information. May be used in the
    /// future to communicate the role. Can also be useful to the UI to communicate that its about
    /// time to start rendering that speech bubble.
    MessageBegin,
    /// Send at the end of each message. Can carry an arbitrary payload, to make messages more of a
    /// dropin for classical functions. Might be refined in the future. We anticipate the stop
    /// reason to be very useful for end applications. We also introduce end messages to keep the
    /// door open for multiple messages in a stream.
    MessageEnd { payload: Value },
    /// Append the internal string to the current message
    MessageAppend { text: String },
    /// A bug caused by the skill code, there the object passed in the payload is not valid JSON.
    InvalidBytesInPayload { message: String },
}

#[derive(Debug)]
pub enum SkillError {
    Any(anyhow::Error),
    /// Skills are still responsible for parsing the input bytes. Our SDK emits a specific error if
    /// the input is not interpretable.
    InvalidInput(String),
    /// Could be a bug in the skill code, an invalid input, which is not catched by the signature,
    /// or an unmet precondition. E.g. a model not existing, or a collection not available. It
    /// should always map to a logic error.
    UserCode(String),
    InvalidOutput(String),
    /// E.g. if instantiating the skill fails.
    RuntimeError(anyhow::Error),
    /// Returned if a function is invoked as a message stream
    IsFunction,
    /// Returned if a message stream is invoked as a function
    IsMessageStream,
}

#[derive(Debug, Clone)]
pub enum AnySkillManifest {
    /// Earliest skill versions do not contain metadata
    V0,
    V0_3(SkillMetadataV0_3),
}

impl AnySkillManifest {
    pub fn description(&self) -> Option<&str> {
        match self {
            Self::V0 => None,
            Self::V0_3(metadata) => metadata.description.as_deref(),
        }
    }

    pub fn signature(&self) -> Option<&Signature> {
        match self {
            Self::V0 => None,
            Self::V0_3(metadata) => Some(&metadata.signature),
        }
    }

    pub fn version(&self) -> Option<&'static str> {
        match self {
            Self::V0 => None,
            Self::V0_3(_) => Some("0.3"),
        }
    }

    pub fn skill_type_name(&self) -> &'static str {
        match self {
            Self::V0 => "function",
            Self::V0_3(metadata) => metadata.signature.skill_type_name(),
        }
    }
}

/// Metadata for a skill at wit version 0.3
#[derive(Debug, Clone)]
pub struct SkillMetadataV0_3 {
    pub description: Option<String>,
    pub signature: Signature,
}

#[derive(Debug, thiserror::Error)]
pub enum MetadataError {
    #[error("Invalid JSON Schema")]
    InvalidJsonSchema,
}

/// Validated to be valid JSON Schema
#[derive(ToSchema, Serialize, Debug, PartialEq, Eq, Clone)]
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

/// Describes the signature of a skill. The signature is the contract of how the skill can be
/// invoked and what its result are.
#[derive(Debug, Clone)]
pub enum Signature {
    Function {
        input_schema: JsonSchema,
        output_schema: JsonSchema,
    },
    MessageStream {
        input_schema: JsonSchema,
    },
}

impl Signature {
    pub fn input_schema(&self) -> &JsonSchema {
        match self {
            Self::MessageStream { input_schema } | Self::Function { input_schema, .. } => {
                input_schema
            }
        }
    }

    pub fn output_schema(&self) -> Option<&JsonSchema> {
        match self {
            Self::Function { output_schema, .. } => Some(output_schema),
            Self::MessageStream { .. } => None,
        }
    }

    pub fn skill_type_name(&self) -> &'static str {
        match self {
            Self::Function { .. } => "function",
            Self::MessageStream { .. } => "message_stream",
        }
    }
}

#[cfg(test)]
pub mod tests {
    use fake::{Fake as _, Faker};
    use serde_json::json;

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
}
