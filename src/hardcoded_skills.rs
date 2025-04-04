//! Contains hardcoded skills that are available in beta systems for testing.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::{
    csi::CsiForSkills,
    inference::{ChatEvent, ChatParams, ChatRequest, Message},
    namespace_watcher::Namespace,
    skills::{AnySkillManifest, Engine, Skill, SkillError, SkillEvent, SkillPath},
};

/// If the path designates a hardcoded skill, return it.
pub fn hardcoded_skill(path: &SkillPath) -> Option<Arc<dyn Skill>> {
    if path.namespace == Namespace::new("test-beta").unwrap() {
        match path.name.as_str() {
            "hello" => Some(Arc::new(SkillHello)),
            "saboteur" => Some(Arc::new(SkillSaboteur)),
            "tell_me_a_joke" => Some(Arc::new(SkillTellMeAJoke)),
            _ => None,
        }
    } else {
        None
    }
}

pub struct SkillHello;
pub struct SkillSaboteur;
pub struct SkillTellMeAJoke;

#[async_trait]
impl Skill for SkillHello {
    async fn manifest(
        &self,
        _engine: &Engine,
        _ctx: Box<dyn CsiForSkills + Send>,
    ) -> Result<AnySkillManifest, SkillError> {
        Ok(AnySkillManifest::V0)
    }

    async fn run_as_function(
        &self,
        _engine: &Engine,
        _ctx: Box<dyn CsiForSkills + Send>,
        _input: Value,
    ) -> Result<Value, SkillError> {
        Err(SkillError::UserCode("I am a dummy Skill".to_owned()))
    }

    async fn run_as_message_stream(
        &self,
        _engine: &Engine,
        _ctx: Box<dyn CsiForSkills + Send>,
        _input: Value,
        sender: mpsc::Sender<SkillEvent>,
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
        _engine: &Engine,
        _ctx: Box<dyn CsiForSkills + Send>,
    ) -> Result<AnySkillManifest, SkillError> {
        Ok(AnySkillManifest::V0)
    }

    async fn run_as_function(
        &self,
        _engine: &Engine,
        _ctx: Box<dyn CsiForSkills + Send>,
        _input: Value,
    ) -> Result<Value, SkillError> {
        Err(SkillError::IsMessageStream)
    }

    async fn run_as_message_stream(
        &self,
        _engine: &Engine,
        _ctx: Box<dyn CsiForSkills + Send>,
        _input: Value,
        _sender: mpsc::Sender<SkillEvent>,
    ) -> Result<(), SkillError> {
        Err(SkillError::UserCode("Skill is a saboteur".to_owned()))
    }
}

#[async_trait]
impl Skill for SkillTellMeAJoke {
    async fn manifest(
        &self,
        _engine: &Engine,
        _ctx: Box<dyn CsiForSkills + Send>,
    ) -> Result<AnySkillManifest, SkillError> {
        Err(SkillError::UserCode("I am a dummy Skill".to_owned()))
    }

    async fn run_as_function(
        &self,
        _engine: &Engine,
        _ctx: Box<dyn CsiForSkills + Send>,
        _input: Value,
    ) -> Result<Value, SkillError> {
        Err(SkillError::UserCode("I am a dummy Skill".to_owned()))
    }

    async fn run_as_message_stream(
        &self,
        _engine: &Engine,
        mut ctx: Box<dyn CsiForSkills + Send>,
        _input: Value,
        sender: mpsc::Sender<SkillEvent>,
    ) -> Result<(), SkillError> {
        let request = ChatRequest {
            model: "llama-3.1-8b-instruct".to_owned(),
            messages: vec![Message {
                role: "user".to_owned(),
                content: "Tell me a joke!".to_owned(),
            }],
            params: ChatParams {
                max_tokens: Some(300),
                temperature: Some(0.3),
                top_p: None,
                frequency_penalty: None,
                presence_penalty: None,
                logprobs: crate::inference::Logprobs::No,
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
                ChatEvent::Usage { .. } => None,
            };
            if let Some(event) = event {
                drop(sender.send(event).await);
            }
        }
        ctx.chat_stream_drop(stream_id).await;
        Ok(())
    }
}
