//! Contains hardcoded skills that are available in beta systems for testing.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::{
    csi::CsiForSkills,
    inference::{CompletionEvent, CompletionParams, CompletionRequest},
    namespace_watcher::Namespace,
    skill_runtime::StreamEvent,
    skills::{AnySkillManifest, Engine, Skill, SkillError, SkillPath},
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
        sender: mpsc::Sender<StreamEvent>,
    ) -> Result<(), SkillError> {
        sender.send(StreamEvent::MessageBegin).await.unwrap();
        for c in "Hello".chars() {
            sender
                .send(StreamEvent::MessageAppend {
                    text: c.to_string(),
                })
                .await
                .unwrap();
        }
        sender
            .send(StreamEvent::MessageEnd {
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
        _sender: mpsc::Sender<StreamEvent>,
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
        sender: mpsc::Sender<StreamEvent>,
    ) -> Result<(), SkillError> {
        let prompt = "<|begin_of_text|><|start_header_id|>user<|end_header_id|>\n\
                            \n\
                        Tell me a joke!<|eot_id|><|start_header_id|>assistant<|end_header_id|>\n\
                        "
        .to_owned();
        let params = CompletionParams {
            return_special_tokens: false,
            max_tokens: Some(300),
            temperature: Some(0.3),
            top_k: None,
            top_p: None,
            stop: vec![],
            frequency_penalty: None,
            presence_penalty: None,
            logprobs: crate::inference::Logprobs::No,
        };
        let request = CompletionRequest {
            prompt,
            model: "llama-3.1-8b-instruct".to_owned(),
            params,
        };
        let stream_id = ctx.completion_stream_new(request).await;
        while let Some(event) = ctx.completion_stream_next(&stream_id).await {
            if let CompletionEvent::Append { text, .. } = event {
                sender
                    .send(StreamEvent::MessageAppend { text })
                    .await
                    .unwrap();
            }
        }
        ctx.completion_stream_drop(stream_id).await;
        Ok(())
    }
}
