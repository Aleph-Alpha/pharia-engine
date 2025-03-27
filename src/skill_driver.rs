use std::{collections::HashMap, future::pending, sync::Arc};

use anyhow::anyhow;
use async_trait::async_trait;
use serde_json::Value;
use tokio::{
    select,
    sync::{mpsc, oneshot},
};

use crate::{
    chunking::{Chunk, ChunkRequest},
    csi::{ChatStreamId, CompletionStreamId, Csi, CsiForSkills},
    inference::{
        ChatEvent, ChatRequest, ChatResponse, Completion, CompletionEvent, CompletionRequest,
        Explanation, ExplanationRequest,
    },
    language_selection::{Language, SelectLanguageRequest},
    search::{Document, DocumentPath, SearchRequest, SearchResult},
    skill_runtime::{SkillExecutionError, StreamEvent},
    skills::{AnySkillManifest, Engine, Skill},
};

pub struct SkillDriver {
    /// Used to execute skills. We will share the engine with multiple running skills, and skill
    /// provider to convert bytes into executable skills.
    engine: Arc<Engine>,
}

impl SkillDriver {
    pub fn new(engine: Arc<Engine>) -> Self {
        Self { engine }
    }

    pub async fn run_message_stream(
        &self,
        skill: Arc<dyn Skill>,
        input: Value,
        csi: impl Csi + Send + Sync + 'static,
        api_token: String,
        sender: mpsc::Sender<StreamEvent>,
    ) -> Result<(), SkillExecutionError> {
        let (send_rt_err, recv_rt_err) = oneshot::channel();
        let csi_for_skills = Box::new(SkillInvocationCtx::new(send_rt_err, csi, api_token));

        let result = select! {
            result = skill.run_as_message_stream(&self.engine, csi_for_skills, input, sender.clone()) => result.map_err(Into::into),
            Ok(error) = recv_rt_err => Err(SkillExecutionError::RuntimeError(error))
        };

        if let Err(err) = &result {
            sender
                .send(StreamEvent::Error(err.to_string()))
                .await
                .unwrap();
        }

        result
    }

    pub async fn run_function(
        &self,
        skill: Arc<dyn Skill>,
        input: Value,
        csi_apis: impl Csi + Send + Sync + 'static,
        api_token: String,
    ) -> Result<Value, SkillExecutionError> {
        let (send_rt_err, recv_rt_err) = oneshot::channel();
        let csi_for_skills = Box::new(SkillInvocationCtx::new(send_rt_err, csi_apis, api_token));
        select! {
            result = skill.run_as_function(&self.engine, csi_for_skills, input) => result.map_err(Into::into),
            // An error occurred during skill execution.
            Ok(error) = recv_rt_err => Err(SkillExecutionError::RuntimeError(error))
        }
    }

    pub async fn metadata(
        &self,
        skill: Arc<dyn Skill>,
    ) -> Result<AnySkillManifest, SkillExecutionError> {
        let (send_rt_err, recv_rt_err) = oneshot::channel();
        let ctx = Box::new(SkillMetadataCtx::new(send_rt_err));
        select! {
            result = skill.manifest(self.engine.as_ref(), ctx) => result.map_err(Into::into),
            // An error occurred during skill execution.
            Ok(_error) = recv_rt_err => Err(SkillExecutionError::CsiUseFromMetadata)
        }
    }
}

/// Implementation of [`Csi`] provided to skills. It is responsible for forwarding the function
/// calls to csi, to the respective drivers and forwarding runtime errors directly to the actor
/// so the User defined code must not worry about accidental complexity.
pub struct SkillInvocationCtx<C> {
    /// This is used to send any runtime error (as opposed to logic error) back to the actor, so it
    /// can drop the future invoking the skill, and report the error appropriately to user and
    /// operator.
    send_rt_error: Option<oneshot::Sender<anyhow::Error>>,
    csi_apis: C,
    // How the user authenticates with us
    api_token: String,
    /// ID counter for stored streams.
    current_stream_id: usize,
    /// Currently running chat streams. We store them here so that we can easier cancel the running
    /// skill if there is an error in the stream. This is much harder to do if we use the normal `ResourceTable`.
    chat_streams: HashMap<ChatStreamId, mpsc::Receiver<anyhow::Result<ChatEvent>>>,
    /// Currently running completion streams. We store them here so that we can easier cancel the running
    /// skill if there is an error in the stream. This is much harder to do if we use the normal `ResourceTable`.
    completion_streams:
        HashMap<CompletionStreamId, mpsc::Receiver<anyhow::Result<CompletionEvent>>>,
}

impl<C> SkillInvocationCtx<C> {
    pub fn new(
        send_rt_err: oneshot::Sender<anyhow::Error>,
        csi_apis: C,
        api_token: String,
    ) -> Self {
        SkillInvocationCtx {
            send_rt_error: Some(send_rt_err),
            csi_apis,
            api_token,
            current_stream_id: 0,
            chat_streams: HashMap::new(),
            completion_streams: HashMap::new(),
        }
    }

    fn next_stream_id<Id>(&mut self) -> Id
    where
        Id: From<usize>,
    {
        self.current_stream_id += 1;
        self.current_stream_id.into()
    }

    /// Never return, we did report the error via the send error channel.
    async fn send_error<T>(&mut self, error: anyhow::Error) -> T {
        self.send_rt_error
            .take()
            .expect("Only one error must be send during skill invocation")
            .send(error)
            .unwrap();
        pending().await
    }
}

#[async_trait]
impl<C> CsiForSkills for SkillInvocationCtx<C>
where
    C: Csi + Send + Sync,
{
    async fn explain(&mut self, requests: Vec<ExplanationRequest>) -> Vec<Explanation> {
        match self
            .csi_apis
            .explain(self.api_token.clone(), requests)
            .await
        {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn complete(&mut self, requests: Vec<CompletionRequest>) -> Vec<Completion> {
        match self
            .csi_apis
            .complete(self.api_token.clone(), requests)
            .await
        {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn completion_stream_new(&mut self, request: CompletionRequest) -> CompletionStreamId {
        let id = self.next_stream_id();
        let recv = self
            .csi_apis
            .completion_stream(self.api_token.clone(), request)
            .await;
        self.completion_streams.insert(id, recv);
        id
    }

    async fn completion_stream_next(&mut self, id: &CompletionStreamId) -> Option<CompletionEvent> {
        let event = self
            .completion_streams
            .get_mut(id)
            .expect("Stream not found")
            .recv()
            .await
            .transpose();
        match event {
            Ok(event) => event,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn completion_stream_drop(&mut self, id: CompletionStreamId) {
        self.completion_streams.remove(&id);
    }

    async fn chat(&mut self, requests: Vec<ChatRequest>) -> Vec<ChatResponse> {
        match self.csi_apis.chat(self.api_token.clone(), requests).await {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn chat_stream_new(&mut self, request: ChatRequest) -> ChatStreamId {
        let id = self.next_stream_id();
        let recv = self
            .csi_apis
            .chat_stream(self.api_token.clone(), request)
            .await;
        self.chat_streams.insert(id, recv);
        id
    }

    async fn chat_stream_next(&mut self, id: &ChatStreamId) -> Option<ChatEvent> {
        let event = self
            .chat_streams
            .get_mut(id)
            .expect("Stream not found")
            .recv()
            .await
            .transpose();
        match event {
            Ok(event) => event,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn chat_stream_drop(&mut self, id: ChatStreamId) {
        self.chat_streams.remove(&id);
    }

    async fn chunk(&mut self, requests: Vec<ChunkRequest>) -> Vec<Vec<Chunk>> {
        match self.csi_apis.chunk(self.api_token.clone(), requests).await {
            Ok(chunks) => chunks,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn select_language(
        &mut self,
        requests: Vec<SelectLanguageRequest>,
    ) -> Vec<Option<Language>> {
        match self.csi_apis.select_language(requests).await {
            Ok(language) => language,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn search(&mut self, requests: Vec<SearchRequest>) -> Vec<Vec<SearchResult>> {
        match self.csi_apis.search(self.api_token.clone(), requests).await {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn documents(&mut self, requests: Vec<DocumentPath>) -> Vec<Document> {
        match self
            .csi_apis
            .documents(self.api_token.clone(), requests)
            .await
        {
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }

    async fn document_metadata(&mut self, requests: Vec<DocumentPath>) -> Vec<Option<Value>> {
        match self
            .csi_apis
            .document_metadata(self.api_token.clone(), requests)
            .await
        {
            // We know there will always be exactly one element in the vector
            Ok(value) => value,
            Err(error) => self.send_error(error).await,
        }
    }
}

/// We know that skill metadata will not invoke any csi functions, but still need to provide an
/// implementation of `CsiForSkills` We do not want to panic, as someone could build a component
/// that uses the csi functions inside the metadata function. Therefore, we always send a runtime
/// error which will lead to skill suspension.
struct SkillMetadataCtx {
    send_rt_err: Option<oneshot::Sender<anyhow::Error>>,
}

impl SkillMetadataCtx {
    pub fn new(send_rt_err: oneshot::Sender<anyhow::Error>) -> Self {
        Self {
            send_rt_err: Some(send_rt_err),
        }
    }

    async fn send_error<T>(&mut self) -> T {
        self.send_rt_err
            .take()
            .expect("Only one error must be send during skill invocation")
            .send(anyhow!(
                "This message will be translated and thus never seen by a user"
            ))
            .unwrap();
        pending().await
    }
}

#[async_trait]
impl CsiForSkills for SkillMetadataCtx {
    async fn explain(&mut self, _requests: Vec<ExplanationRequest>) -> Vec<Explanation> {
        self.send_error().await
    }

    async fn complete(&mut self, _requests: Vec<CompletionRequest>) -> Vec<Completion> {
        self.send_error().await
    }

    async fn completion_stream_new(&mut self, _request: CompletionRequest) -> CompletionStreamId {
        self.send_error().await
    }

    async fn completion_stream_next(
        &mut self,
        _id: &CompletionStreamId,
    ) -> Option<CompletionEvent> {
        self.send_error().await
    }

    async fn completion_stream_drop(&mut self, _id: CompletionStreamId) {
        self.send_error::<()>().await;
    }

    async fn chunk(&mut self, _requests: Vec<ChunkRequest>) -> Vec<Vec<Chunk>> {
        self.send_error().await
    }

    async fn select_language(
        &mut self,
        _requests: Vec<SelectLanguageRequest>,
    ) -> Vec<Option<Language>> {
        self.send_error().await
    }

    async fn chat(&mut self, _requests: Vec<ChatRequest>) -> Vec<ChatResponse> {
        self.send_error().await
    }

    async fn chat_stream_new(&mut self, _request: ChatRequest) -> ChatStreamId {
        self.send_error().await
    }

    async fn chat_stream_next(&mut self, _id: &ChatStreamId) -> Option<ChatEvent> {
        self.send_error().await
    }

    async fn chat_stream_drop(&mut self, _id: ChatStreamId) {
        self.send_error::<()>().await;
    }

    async fn search(&mut self, _requests: Vec<SearchRequest>) -> Vec<Vec<SearchResult>> {
        self.send_error().await
    }

    async fn document_metadata(&mut self, _requests: Vec<DocumentPath>) -> Vec<Option<Value>> {
        self.send_error().await
    }

    async fn documents(&mut self, _requests: Vec<DocumentPath>) -> Vec<Document> {
        self.send_error().await
    }
}

#[cfg(test)]
mod test {
    use std::sync::Mutex;

    use super::*;
    use crate::{
        chunking::ChunkParams,
        csi::tests::{CsiDummy, CsiSaboteur, StubCsi},
        inference::{
            ChatParams, CompletionParams, FinishReason, Granularity, Logprobs, Message, TextScore,
            TokenUsage,
        },
        skills::SkillError,
    };
    use anyhow::anyhow;
    use serde_json::json;

    #[tokio::test]
    async fn chunk() {
        // Given a skill invocation context with a stub tokenizer provider
        let (send, _) = oneshot::channel();
        let mut csi = StubCsi::empty();
        csi.set_chunking(|r| {
            Ok(r.into_iter()
                .map(|_| {
                    vec![Chunk {
                        text: "my_chunk".to_owned(),
                        byte_offset: 0,
                        character_offset: None,
                    }]
                })
                .collect())
        });

        let mut invocation_ctx = SkillInvocationCtx::new(send, csi, "dummy token".to_owned());

        // When chunking a short text
        let model = "Pharia-1-LLM-7B-control".to_owned();
        let max_tokens = 10;
        let request = ChunkRequest {
            text: "Greet".to_owned(),
            params: ChunkParams {
                model,
                max_tokens,
                overlap: 0,
            },
            character_offsets: false,
        };
        let chunks = invocation_ctx.chunk(vec![request]).await;

        // Then a single chunk is returned
        assert_eq!(
            chunks[0],
            vec![Chunk {
                text: "my_chunk".to_owned(),
                byte_offset: 0,
                character_offset: None,
            }]
        );
    }

    #[tokio::test]
    async fn receive_error_if_chunk_failed() {
        // Given a skill invocation context with a saboteur tokenizer provider
        let (send, recv) = oneshot::channel();
        let mut csi = StubCsi::empty();
        csi.set_chunking(|_| Err(anyhow!("Failed to load tokenizer")));
        let mut invocation_ctx = SkillInvocationCtx::new(send, csi, "dummy token".to_owned());

        // When chunking a short text
        let model = "Pharia-1-LLM-7B-control".to_owned();
        let max_tokens = 10;
        let request = ChunkRequest {
            text: "Greet".to_owned(),
            params: ChunkParams {
                model,
                max_tokens,
                overlap: 0,
            },
            character_offsets: false,
        };
        let error = select! {
            error = recv => error.unwrap(),
            _ = invocation_ctx.chunk(vec![request])  => unreachable!(),
        };

        // Then receive the error from saboteur tokenizer provider
        assert_eq!(error.to_string(), "Failed to load tokenizer");
    }

    #[tokio::test]
    async fn skill_invocation_ctx_stream_management() {
        let (send, _) = oneshot::channel();
        let completion = Completion {
            text: "text".to_owned(),
            finish_reason: FinishReason::Stop,
            logprobs: vec![],
            usage: TokenUsage {
                prompt: 2,
                completion: 2,
            },
        };
        let resp = completion.clone();
        let csi = StubCsi::with_completion(move |_| resp.clone());
        let mut ctx = SkillInvocationCtx::new(send, csi, "dummy".to_owned());
        let request = CompletionRequest::new("prompt", "model");

        let stream_id = ctx.completion_stream_new(request).await;
        let mut events = vec![];
        while let Some(event) = ctx.completion_stream_next(&stream_id).await {
            events.push(event);
        }
        ctx.completion_stream_drop(stream_id).await;

        assert_eq!(
            events,
            vec![
                CompletionEvent::Append {
                    text: completion.text,
                    logprobs: completion.logprobs
                },
                CompletionEvent::End {
                    finish_reason: completion.finish_reason
                },
                CompletionEvent::Usage {
                    usage: completion.usage
                }
            ]
        );
        assert!(ctx.completion_streams.is_empty());
    }

    #[tokio::test]
    async fn skill_invocation_ctx_chat_stream_management() {
        let (send, _) = oneshot::channel();
        let response = ChatResponse {
            message: Message {
                role: "assistant".to_owned(),
                content: "Hello".to_owned(),
            },
            finish_reason: FinishReason::Stop,
            logprobs: vec![],
            usage: TokenUsage {
                prompt: 1,
                completion: 1,
            },
        };
        let stub_response = response.clone();
        let csi = StubCsi::with_chat(move |_| stub_response.clone());
        let mut ctx = SkillInvocationCtx::new(send, csi, "dummy".to_owned());
        let request = ChatRequest {
            model: "model".to_owned(),
            messages: vec![],
            params: ChatParams::default(),
        };

        let stream_id = ctx.chat_stream_new(request).await;
        let mut events = vec![];
        while let Some(event) = ctx.chat_stream_next(&stream_id).await {
            events.push(event);
        }
        ctx.chat_stream_drop(stream_id).await;

        assert_eq!(
            events,
            vec![
                ChatEvent::MessageBegin {
                    role: response.message.role,
                },
                ChatEvent::MessageAppend {
                    content: response.message.content,
                    logprobs: response.logprobs,
                },
                ChatEvent::MessageEnd {
                    finish_reason: response.finish_reason
                },
                ChatEvent::Usage {
                    usage: response.usage
                }
            ]
        );
        assert!(ctx.completion_streams.is_empty());
    }

    #[tokio::test]
    async fn csi_usage_from_metadata_leads_to_suspension() {
        // Given a skill runtime that always returns a skill that uses the csi from the metadata function
        struct CsiFromMetadataSkill;
        #[async_trait]
        impl Skill for CsiFromMetadataSkill {
            async fn manifest(
                &self,
                _engine: &Engine,
                mut ctx: Box<dyn CsiForSkills + Send>,
            ) -> Result<AnySkillManifest, SkillError> {
                ctx.select_language(vec![SelectLanguageRequest {
                    text: "Hello, good sir!".to_owned(),
                    languages: Vec::new(),
                }])
                .await;
                unreachable!(
                    "The test should never reach this point, as its execution shoud be suspendend"
                )
            }

            async fn run_as_function(
                &self,
                _engine: &Engine,
                _ctx: Box<dyn CsiForSkills + Send>,
                _input: Value,
            ) -> Result<Value, SkillError> {
                unreachable!("This won't be invoked during the test")
            }

            async fn run_as_message_stream(
                &self,
                _engine: &Engine,
                _ctx: Box<dyn CsiForSkills + Send>,
                _input: Value,
                _sender: mpsc::Sender<StreamEvent>,
            ) -> Result<(), SkillError> {
                unreachable!("This won't be invoked during the test")
            }
        }

        let engine = Arc::new(Engine::new(false).unwrap());
        let skill_driver = SkillDriver::new(engine);

        // When metadata for a skill is requested
        let metadata = skill_driver.metadata(Arc::new(CsiFromMetadataSkill)).await;

        // Then the metadata is None
        assert_eq!(
            metadata.unwrap_err().to_string(),
            "The metadata function of the invoked skill is bugged. It is forbidden to invoke any \
            CSI functions from the metadata function, yet the skill does precisely this."
        );
    }

    #[tokio::test]
    async fn forward_explain_response_from_csi() {
        struct SkillDoubleUsingExplain;

        #[async_trait]
        impl Skill for SkillDoubleUsingExplain {
            async fn run_as_function(
                &self,
                _engine: &Engine,
                mut ctx: Box<dyn CsiForSkills + Send>,
                _input: Value,
            ) -> Result<Value, SkillError> {
                let explanation = ctx
                    .explain(vec![ExplanationRequest {
                        prompt: "An apple a day".to_owned(),
                        target: " keeps the doctor away".to_owned(),
                        model: "test-model-name".to_owned(),
                        granularity: Granularity::Auto,
                    }])
                    .await
                    .pop()
                    .unwrap();
                let output = explanation
                    .into_iter()
                    .map(|text_score| json!({"start": text_score.start, "length": text_score.length}))
                    .collect::<Vec<_>>();
                Ok(json!(output))
            }

            async fn manifest(
                &self,
                _engine: &Engine,
                _ctx: Box<dyn CsiForSkills + Send>,
            ) -> Result<AnySkillManifest, SkillError> {
                panic!("Dummy metadata implementation of SkillDoubleUsingExplain")
            }

            async fn run_as_message_stream(
                &self,
                _engine: &Engine,
                _ctx: Box<dyn CsiForSkills + Send>,
                _input: Value,
                _sender: mpsc::Sender<StreamEvent>,
            ) -> Result<(), SkillError> {
                panic!("Dummy message stream implementation of SkillDoubleUsingExplain")
            }
        }

        let engine = Arc::new(Engine::new(false).unwrap());
        let csi = StubCsi::with_explain(|_| {
            Explanation::new(vec![TextScore {
                score: 0.0,
                start: 0,
                length: 2,
            }])
        });

        let runtime = SkillDriver::new(engine);
        let resp = runtime
            .run_function(
                Arc::new(SkillDoubleUsingExplain),
                json!({"prompt": "An apple a day", "target": " keeps the doctor away"}),
                csi,
                "dummy token".to_owned(),
            )
            .await;

        drop(runtime);

        assert_eq!(resp.unwrap(), json!([{"start": 0, "length": 2}]));
    }

    #[tokio::test]
    async fn should_forward_csi_errors() {
        // Given a skill using csi and a csi that fails
        // Note we are using a skill which actually invokes the csi
        let skill = Arc::new(SkillGreetCompletion);
        let engine = Arc::new(Engine::new(false).unwrap());
        let driver = SkillDriver::new(engine);

        // When trying to generate a greeting for Homer using the greet skill
        let result = driver
            .run_function(
                skill,
                json!("Homer"),
                CsiSaboteur,
                "TOKEN_NOT_REQUIRED".to_owned(),
            )
            .await;

        // Then
        let expectet_error_msg = "The skill could not be executed to completion, something in our \
            runtime is currently \nunavailable or misconfigured. You should try again later, if \
            the situation persists you \nmay want to contact the operaters. Original error:\n\n\
            Test error";
        assert_eq!(result.unwrap_err().to_string(), expectet_error_msg);
    }

    #[tokio::test]
    async fn should_stop_execution_after_message_stream_emits_error() {
        // Given a skill that emits an error
        struct SaboteurSpy {
            // This will be set to true if skill code is executed after error
            executed_code_after_error: Mutex<bool>,
        }

        #[async_trait]
        impl Skill for SaboteurSpy {
            async fn run_as_function(
                &self,
                _engine: &Engine,
                _ctx: Box<dyn CsiForSkills + Send>,
                _input: Value,
            ) -> Result<Value, SkillError> {
                panic!("This function should not be called");
            }

            async fn manifest(
                &self,
                _engine: &Engine,
                _ctx: Box<dyn CsiForSkills + Send>,
            ) -> Result<AnySkillManifest, SkillError> {
                panic!("This function should not be called");
            }

            async fn run_as_message_stream(
                &self,
                _engine: &Engine,
                _ctx: Box<dyn CsiForSkills + Send>,
                _input: Value,
                sender: mpsc::Sender<StreamEvent>,
            ) -> Result<(), SkillError> {
                sender
                    .send(StreamEvent::Error("Test error".to_owned()))
                    .await
                    .unwrap();
                // Set boolean to true if the code is executed after the error. We test for this in
                // our assertion
                *(self.executed_code_after_error.lock().unwrap()) = true;
                Ok(())
            }
        }

        let engine = Arc::new(Engine::new(false).unwrap());
        let driver = SkillDriver::new(engine);

        // When
        let skill = Arc::new(SaboteurSpy {
            executed_code_after_error: Mutex::new(false),
        });
        let (send, mut recv) = mpsc::channel(1);
        driver
            .run_message_stream(
                skill.clone(),
                json!({}),
                CsiDummy,
                "Dummy Token".to_owned(),
                send,
            )
            .await
            .unwrap();

        // Then
        assert_eq!(
            StreamEvent::Error("Test error".to_owned()),
            recv.recv().await.unwrap()
        );
        // the boolean should still be false.
        assert!(!*skill.executed_code_after_error.lock().unwrap());
    }

    /// A test double for a skill. It invokes the csi with a prompt and returns the result.
    struct SkillGreetCompletion;

    #[async_trait]
    impl Skill for SkillGreetCompletion {
        async fn run_as_function(
            &self,
            _engine: &Engine,
            mut ctx: Box<dyn CsiForSkills + Send>,
            _input: Value,
        ) -> Result<Value, SkillError> {
            let mut completions = ctx
                .complete(vec![CompletionRequest {
                    prompt: "Hello".to_owned(),
                    model: "test-model-name".to_owned(),
                    params: CompletionParams {
                        max_tokens: Some(10),
                        temperature: Some(0.5),
                        top_p: Some(1.0),
                        presence_penalty: Some(0.0),
                        frequency_penalty: Some(0.0),
                        stop: Vec::new(),
                        return_special_tokens: true,
                        top_k: None,
                        logprobs: Logprobs::No,
                    },
                }])
                .await;
            let completion = completions.pop().unwrap().text;
            Ok(json!(completion))
        }

        async fn manifest(
            &self,
            _engine: &Engine,
            _ctx: Box<dyn CsiForSkills + Send>,
        ) -> Result<AnySkillManifest, SkillError> {
            panic!("Dummy metadata implementation of Skill Greet Completion")
        }

        async fn run_as_message_stream(
            &self,
            _engine: &Engine,
            _ctx: Box<dyn CsiForSkills + Send>,
            _input: Value,
            _sender: mpsc::Sender<StreamEvent>,
        ) -> Result<(), SkillError> {
            Err(SkillError::IsFunction)
        }
    }
}
