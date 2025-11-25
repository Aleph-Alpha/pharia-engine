//! Contains hardcoded skills with defined behavior.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::{
    inference::{AssistantMessage, ChatEvent, ChatParams, ChatRequest, Message},
    logging::TracingContext,
    namespace_watcher::Namespace,
    skill::{
        AnySkillManifest, BoxedCsi, JsonSchema, Signature, Skill, SkillError, SkillEvent,
        SkillMetadataV0_3, SkillPath,
    },
    tool::{Argument, InvokeRequest},
};

/// Hardcoded skills are provided in beta systems for testing.
/// If the path designates a hardcoded skill, return it.
pub fn hardcoded_skill(path: &SkillPath) -> Option<Arc<dyn Skill>> {
    if path.namespace == Namespace::new("test-beta").unwrap() {
        match path.name.as_str() {
            "hello" => Some(Arc::new(SkillHello)),
            "saboteur" => Some(Arc::new(SkillSaboteur)),
            "tell_me_a_joke" => Some(Arc::new(SkillTellMeAJoke)),
            "tool_caller" => Some(Arc::new(SkillToolCaller)),
            _ => None,
        }
    } else {
        None
    }
}

pub struct SkillHello;
pub struct SkillSaboteur;
pub struct SkillTellMeAJoke;

/// A `message_stream` skill that invokes a tool.
pub struct SkillToolCaller;

#[async_trait]
impl Skill for SkillToolCaller {
    async fn manifest(
        &self,
        _ctx: BoxedCsi,
        _tracing_context: &TracingContext,
    ) -> Result<AnySkillManifest, SkillError> {
        Ok(AnySkillManifest::V0)
    }

    async fn run_as_function(
        &self,
        _ctx: BoxedCsi,
        _input: Value,
        _tracing_context: &TracingContext,
    ) -> Result<Value, SkillError> {
        Err(SkillError::UserCode(
            "I am a `message_stream` skill. Use the `message_stream` endpoint to invoke me."
                .to_owned(),
        ))
    }

    async fn run_as_message_stream(
        &self,
        mut ctx: BoxedCsi,
        _input: Value,
        sender: mpsc::Sender<SkillEvent>,
        _tracing_context: &TracingContext,
    ) -> Result<(), SkillError> {
        let result = ctx
            .invoke_tool(vec![InvokeRequest {
                name: "add".to_owned(),
                arguments: vec![
                    Argument {
                        name: "a".to_owned(),
                        value: json!(1).to_string().into_bytes(),
                    },
                    Argument {
                        name: "b".to_owned(),
                        value: json!(2).to_string().into_bytes(),
                    },
                ],
            }])
            .await
            .into_iter()
            .nth(0)
            .ok_or(SkillError::UserCode("Tool invocation failed".to_owned()))?
            .map_err(|e| SkillError::UserCode(e.clone()))?;

        sender.send(SkillEvent::MessageBegin).await.unwrap();
        sender
            .send(SkillEvent::MessageAppend {
                text: "The result of calling the add tool for a=1 and b=2 is: ".to_owned()
                    + &result.text(),
            })
            .await
            .unwrap();
        sender
            .send(SkillEvent::MessageEnd {
                payload: json!(null),
            })
            .await
            .unwrap();
        Ok(())
    }
}

#[async_trait]
impl Skill for SkillHello {
    async fn manifest(
        &self,
        _ctx: BoxedCsi,
        _tracing_context: &TracingContext,
    ) -> Result<AnySkillManifest, SkillError> {
        Ok(AnySkillManifest::V0)
    }

    async fn run_as_function(
        &self,
        _ctx: BoxedCsi,
        _input: Value,
        _tracing_context: &TracingContext,
    ) -> Result<Value, SkillError> {
        Err(SkillError::UserCode("I am a dummy Skill".to_owned()))
    }

    async fn run_as_message_stream(
        &self,
        _ctx: BoxedCsi,
        _input: Value,
        sender: mpsc::Sender<SkillEvent>,
        _tracing_context: &TracingContext,
    ) -> Result<(), SkillError> {
        sender.send(SkillEvent::MessageBegin).await.unwrap();

        for c in "Hello".chars() {
            sender
                .send(SkillEvent::MessageAppend {
                    text: c.to_string(),
                })
                .await
                .unwrap();
        }

        sender
            .send(SkillEvent::MessageEnd {
                payload: json!(null),
            })
            .await
            .unwrap();
        Ok(())
    }
}

#[async_trait]
impl Skill for SkillSaboteur {
    async fn manifest(
        &self,
        _ctx: BoxedCsi,
        _tracing_context: &TracingContext,
    ) -> Result<AnySkillManifest, SkillError> {
        Ok(AnySkillManifest::V0)
    }

    async fn run_as_function(
        &self,
        _ctx: BoxedCsi,
        _input: Value,
        _tracing_context: &TracingContext,
    ) -> Result<Value, SkillError> {
        Err(SkillError::IsMessageStream)
    }

    async fn run_as_message_stream(
        &self,
        _ctx: BoxedCsi,
        _input: Value,
        _sender: mpsc::Sender<SkillEvent>,
        _tracing_context: &TracingContext,
    ) -> Result<(), SkillError> {
        Err(SkillError::UserCode("Skill is a saboteur".to_owned()))
    }
}

#[async_trait]
impl Skill for SkillTellMeAJoke {
    async fn manifest(
        &self,
        _ctx: BoxedCsi,
        _tracing_context: &TracingContext,
    ) -> Result<AnySkillManifest, SkillError> {
        Err(SkillError::UserCode("I am a dummy Skill".to_owned()))
    }

    async fn run_as_function(
        &self,
        _ctx: BoxedCsi,
        _input: Value,
        _tracing_context: &TracingContext,
    ) -> Result<Value, SkillError> {
        Err(SkillError::UserCode("I am a dummy Skill".to_owned()))
    }

    async fn run_as_message_stream(
        &self,
        mut ctx: BoxedCsi,
        _input: Value,
        sender: mpsc::Sender<SkillEvent>,
        _tracing_context: &TracingContext,
    ) -> Result<(), SkillError> {
        let request = ChatRequest {
            model: "llama-3.1-8b-instruct".to_owned(),
            messages: vec![Message::Other {
                role: "user".to_owned(),
                content: "Tell me a joke!".to_owned(),
            }],
            params: ChatParams {
                max_tokens: Some(300),
                max_completion_tokens: None,
                temperature: Some(0.3),
                top_p: None,
                frequency_penalty: None,
                presence_penalty: None,
                logprobs: crate::inference::Logprobs::No,
                tools: None,
                tool_choice: None,
                parallel_tool_calls: None,
                response_format: None,
                reasoning_effort: None,
            },
        };
        let stream_id = ctx.chat_stream_new(request).await;
        while let Some(event) = ctx.chat_stream_next(&stream_id).await {
            let event = match event {
                ChatEvent::MessageBegin { .. } => Some(SkillEvent::MessageBegin),
                ChatEvent::MessageAppend { content, .. } => {
                    Some(SkillEvent::MessageAppend { text: content })
                }
                ChatEvent::MessageEnd { finish_reason } => Some(SkillEvent::MessageEnd {
                    payload: json!(format!("{finish_reason:?}")),
                }),
                ChatEvent::Usage { .. } | ChatEvent::ToolCall { .. } => None,
            };
            if let Some(event) = event {
                drop(sender.send(event).await);
            }
        }
        ctx.chat_stream_drop(stream_id).await;
        Ok(())
    }
}

/// Since the chat skill is part of our public API, we decouple it from the inference types by
/// introducing our own message type. This means even if the inference types change, the skill still
/// exposes the same interface.
#[derive(Deserialize)]
struct SkillChatMessage {
    role: String,
    content: String,
}

impl SkillChatMessage {
    fn into_message(self) -> anyhow::Result<Message> {
        match self.role.as_str() {
            "system" => Ok(Message::system(self.content)),
            "user" => Ok(Message::user(self.content)),
            "assistant" => Ok(Message::Assistant(AssistantMessage {
                content: Some(self.content),
                tool_calls: None,
            })),
            _ => Err(anyhow::anyhow!("Invalid role: {}", self.role)),
        }
    }
}

#[derive(Deserialize)]
struct SkillChatInput {
    messages: Vec<SkillChatMessage>,
}

pub struct SkillChat {
    model: String,
    system_prompt: String,
}

impl SkillChat {
    pub fn new(model: impl Into<String>, system_prompt: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            system_prompt: system_prompt.into(),
        }
    }
}

#[async_trait]
impl Skill for SkillChat {
    async fn manifest(
        &self,
        _ctx: BoxedCsi,
        _tracing_context: &TracingContext,
    ) -> Result<AnySkillManifest, SkillError> {
        Ok(AnySkillManifest::V0_3(SkillMetadataV0_3 {
            description: Some("A chat skill".to_owned()),
            signature: Signature::MessageStream {
                input_schema: JsonSchema::try_from(json!(
                    {
                        "properties": {
                            "messages": {
                                "title": "Messages",
                                "type": "array"
                            }
                        },
                        "required": ["messages"],
                        "title": "SkillInput",
                        "type": "object"
                    }
                ))
                .unwrap(),
            },
        }))
    }

    async fn run_as_function(
        &self,
        _ctx: BoxedCsi,
        _input: Value,
        _tracing_context: &TracingContext,
    ) -> Result<Value, SkillError> {
        Err(SkillError::IsMessageStream)
    }

    async fn run_as_message_stream(
        &self,
        mut ctx: BoxedCsi,
        input: Value,
        sender: mpsc::Sender<SkillEvent>,
        _tracing_context: &TracingContext,
    ) -> Result<(), SkillError> {
        let result = serde_json::from_value::<SkillChatInput>(input);

        if let Err(e) = result {
            return Err(SkillError::InvalidInput(e.to_string()));
        }

        let messages = result
            .unwrap()
            .messages
            .into_iter()
            .map(SkillChatMessage::into_message)
            .collect::<anyhow::Result<Vec<_>>>()
            .map_err(|e| SkillError::InvalidInput(e.to_string()))?;

        let mut all_messages = vec![Message::Other {
            role: "system".to_owned(),
            content: self.system_prompt.clone(),
        }];
        all_messages.extend(messages);

        let request = ChatRequest {
            model: self.model.clone(),
            messages: all_messages,
            params: ChatParams {
                max_tokens: None,
                max_completion_tokens: None,
                temperature: None,
                top_p: None,
                frequency_penalty: None,
                presence_penalty: None,
                logprobs: crate::inference::Logprobs::No,
                tools: None,
                tool_choice: None,
                parallel_tool_calls: None,
                response_format: None,
                reasoning_effort: None,
            },
        };

        let stream_id = ctx.chat_stream_new(request).await;
        while let Some(event) = ctx.chat_stream_next(&stream_id).await {
            let event = match event {
                ChatEvent::MessageBegin { .. } => Some(SkillEvent::MessageBegin),
                ChatEvent::MessageAppend { content, .. } => {
                    Some(SkillEvent::MessageAppend { text: content })
                }
                ChatEvent::MessageEnd { finish_reason } => Some(SkillEvent::MessageEnd {
                    payload: json!(format!("{finish_reason:?}")),
                }),
                ChatEvent::Usage { .. } | ChatEvent::ToolCall { .. } => None,
            };
            if let Some(event) = event {
                drop(sender.send(event).await);
            }
        }
        ctx.chat_stream_drop(stream_id).await;
        Ok(())
    }
}
