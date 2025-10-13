mod contextual;
mod raw;

use async_trait::async_trait;
use derive_more::{Constructor, From};
use serde_json::Value;

pub use contextual::{ContextualCsi, InvocationContext};
pub use raw::{CsiDrivers, CsiError, RawCsi};

use crate::{
    chunking::{Chunk, ChunkRequest},
    inference::{
        ChatEvent, ChatEventV2, ChatRequest, ChatResponse, ChatResponseV2, Completion,
        CompletionEvent, CompletionRequest, Explanation, ExplanationRequest,
    },
    language_selection::{Language, SelectLanguageRequest},
    search::{Document, DocumentPath, SearchRequest, SearchResult},
    tool::{InvokeRequest, ToolDescription, ToolOutput},
};

#[cfg(test)]
use double_trait::double;

/// `CompletionStreamId` is a unique identifier for a completion stream.
#[derive(Debug, Clone, Constructor, Copy, From, PartialEq, Eq, Hash)]
pub struct CompletionStreamId(usize);
/// `ChatStreamId` is a unique identifier for a chat stream.
#[derive(Debug, Clone, Constructor, Copy, From, PartialEq, Eq, Hash)]
pub struct ChatStreamId(usize);

/// Cognitive System Interface (CSI) as consumed by Skill developers. In particular some accidental
/// complexity has been stripped away, by implementations due to removing accidental errors from the
/// interface. It also assumes all authentication and authorization is handled behind the scenes.
/// This is the CSI as passed to user defined code in WASM.
#[async_trait]
#[cfg_attr(test, double(CsiDouble))]
pub trait Csi {
    async fn explain(&mut self, requests: Vec<ExplanationRequest>) -> Vec<Explanation>;
    async fn complete(&mut self, requests: Vec<CompletionRequest>) -> Vec<Completion>;
    async fn completion_stream_new(&mut self, request: CompletionRequest) -> CompletionStreamId;
    async fn completion_stream_next(&mut self, id: &CompletionStreamId) -> Option<CompletionEvent>;
    async fn completion_stream_drop(&mut self, id: CompletionStreamId);
    async fn chunk(&mut self, requests: Vec<ChunkRequest>) -> Vec<Vec<Chunk>>;
    async fn select_language(
        &mut self,
        requests: Vec<SelectLanguageRequest>,
    ) -> Vec<Option<Language>>;
    async fn chat(&mut self, requests: Vec<ChatRequest>) -> Vec<ChatResponse>;
    async fn chat_v2(&mut self, requests: Vec<ChatRequest>) -> Vec<ChatResponseV2>;
    async fn chat_stream_new(&mut self, request: ChatRequest) -> ChatStreamId;
    async fn chat_stream_new_v2(&mut self, request: ChatRequest) -> ChatStreamId;
    async fn chat_stream_next(&mut self, id: &ChatStreamId) -> Option<ChatEvent>;
    async fn chat_stream_next_v2(&mut self, id: &ChatStreamId) -> Option<ChatEventV2>;
    async fn chat_stream_drop(&mut self, id: ChatStreamId);
    async fn search(&mut self, requests: Vec<SearchRequest>) -> Vec<Vec<SearchResult>>;
    async fn document_metadata(&mut self, document_paths: Vec<DocumentPath>) -> Vec<Option<Value>>;
    async fn documents(&mut self, document_paths: Vec<DocumentPath>) -> Vec<Document>;
    async fn invoke_tool(&mut self, request: Vec<InvokeRequest>) -> Vec<ToolResult>;
    async fn list_tools(&mut self) -> Vec<ToolDescription>;
}

/// While the [`crate::csi::RawCsi`] knows about the [`crate::tool::ToolError`] enum, the CSI does
/// not. For all it cares, all errors are represented as error messages that can be exposed to the
/// Skill. In all other CSI methods, we do not have an error concept. In the error case, we stop
/// Skill execution, as we assume that there is nothing the Skill could do to recover.
/// For tool calls however, there are many scenarios, especially when a tool is invoked by a model,
/// in which an error message could help the model to recover. Examples are badly formatted tool
/// input or a misspelled tool name.
pub type ToolResult = Result<ToolOutput, String>;

#[cfg(test)]
pub mod tests {
    pub use contextual::ContextualCsiDouble;
    pub use raw::RawCsiDouble;
    use std::collections::HashMap;

    use crate::{
        inference::{
            AssistantMessage, AssistantMessageV2, ChatEvent, CompletionEvent, FinishReason,
            TokenUsage,
        },
        search::TextCursor,
    };

    use super::*;

    pub struct CsiCompleteStreamStub {
        current_id: usize,
        events: Vec<CompletionEvent>,
        streams: HashMap<CompletionStreamId, Vec<CompletionEvent>>,
    }

    impl CsiCompleteStreamStub {
        pub fn new(mut events: Vec<CompletionEvent>) -> Self {
            events.reverse();
            Self {
                current_id: 0,
                events,
                streams: HashMap::new(),
            }
        }
    }

    #[async_trait]
    impl CsiDouble for CsiCompleteStreamStub {
        async fn completion_stream_new(
            &mut self,
            _request: CompletionRequest,
        ) -> CompletionStreamId {
            let id = CompletionStreamId::new(self.current_id);
            self.streams.insert(id, self.events.clone());
            self.current_id += 1;
            id
        }

        async fn completion_stream_next(
            &mut self,
            id: &CompletionStreamId,
        ) -> Option<CompletionEvent> {
            self.streams.get_mut(id)?.pop()
        }

        async fn completion_stream_drop(&mut self, id: CompletionStreamId) {
            self.streams.remove(&id);
        }
    }

    pub struct CsiChatStreamStub {
        current_id: usize,
        events: Vec<ChatEvent>,
        streams: HashMap<ChatStreamId, Vec<ChatEvent>>,
    }

    impl CsiChatStreamStub {
        pub fn new(mut events: Vec<ChatEvent>) -> Self {
            events.reverse();
            Self {
                current_id: 0,
                events,
                streams: HashMap::new(),
            }
        }
    }

    #[async_trait]
    impl CsiDouble for CsiChatStreamStub {
        async fn chat_stream_new(&mut self, _request: ChatRequest) -> ChatStreamId {
            let id = ChatStreamId::new(self.current_id);
            self.streams.insert(id, self.events.clone());
            self.current_id += 1;
            id
        }

        async fn chat_stream_next(&mut self, id: &ChatStreamId) -> Option<ChatEvent> {
            self.streams.get_mut(id)?.pop()
        }

        async fn chat_stream_drop(&mut self, id: ChatStreamId) {
            self.streams.remove(&id);
        }
    }

    /// Assert that the echo parameter is set to true and return the prompt as the completion.
    pub struct CsiCompleteWithEchoMock;

    #[async_trait]
    impl CsiDouble for CsiCompleteWithEchoMock {
        async fn complete(&mut self, requests: Vec<CompletionRequest>) -> Vec<Completion> {
            requests
                .into_iter()
                .map(|request| {
                    assert!(
                        request.params.echo,
                        "`CsiCompleteWithEchoMock` requires the `echo` parameter to be set to true"
                    );
                    Completion::from_text(request.prompt)
                })
                .collect()
        }
    }

    /// Asserts a specific prompt and model and returns a greeting message
    #[derive(Clone)]
    pub struct CsiGreetingMock;

    impl CsiGreetingMock {
        fn complete_text(request: CompletionRequest) -> Completion {
            let expected_prompt = "<|begin_of_text|><|start_header_id|>system<|end_header_id|>

Cutting Knowledge Date: December 2023
Today Date: 23 Jul 2024

You are a helpful assistant.<|eot_id|><|start_header_id|>user<|end_header_id|>

Provide a nice greeting for the person named: Homer<|eot_id|><|start_header_id|>assistant<|end_header_id|>";

            let expected_model = "pharia-1-llm-7b-control";

            // Print actual parameters in case of failure
            eprintln!("{request:?}");

            if matches!(request, CompletionRequest{ prompt, model, ..} if model == expected_model && prompt == expected_prompt)
            {
                Completion::from_text("Hello Homer")
            } else {
                Completion::from_text("Mock expectation violated")
            }
        }
    }

    #[async_trait]
    impl CsiDouble for CsiGreetingMock {
        async fn complete(&mut self, requests: Vec<CompletionRequest>) -> Vec<Completion> {
            requests.into_iter().map(Self::complete_text).collect()
        }
    }

    /// Always return a hardcoded dummy response
    pub struct CsiChatStub;

    #[async_trait]
    impl CsiDouble for CsiChatStub {
        async fn chat(&mut self, requests: Vec<ChatRequest>) -> Vec<ChatResponse> {
            requests
                .iter()
                .map(|_| ChatResponse {
                    message: AssistantMessage::dummy(),
                    finish_reason: FinishReason::Stop,
                    logprobs: vec![],
                    usage: TokenUsage {
                        prompt: 0,
                        completion: 0,
                    },
                })
                .collect()
        }
    }

    pub struct CsiChatStubV2;

    #[async_trait]
    impl CsiDouble for CsiChatStubV2 {
        async fn chat_v2(&mut self, requests: Vec<ChatRequest>) -> Vec<ChatResponseV2> {
            requests
                .iter()
                .map(|_| ChatResponseV2 {
                    message: AssistantMessageV2::dummy(),
                    finish_reason: FinishReason::Stop,
                    logprobs: vec![],
                    usage: TokenUsage {
                        prompt: 0,
                        completion: 0,
                    },
                })
                .collect()
        }
    }

    /// Return the content of the query as a search result
    pub struct CsiSearchMock;

    #[async_trait]
    impl CsiDouble for CsiSearchMock {
        async fn search(&mut self, requests: Vec<SearchRequest>) -> Vec<Vec<SearchResult>> {
            requests
                .into_iter()
                .map(|request| {
                    let document_path = DocumentPath {
                        namespace: "aleph-alpha".to_owned(),
                        collection: "test-collection".to_owned(),
                        name: "small".to_owned(),
                    };
                    vec![SearchResult {
                        document_path,
                        content: request.query,
                        score: 1.0,
                        start: TextCursor {
                            item: 0,
                            position: 0,
                        },
                        end: TextCursor {
                            item: 0,
                            position: 0,
                        },
                    }]
                })
                .collect()
        }

        async fn documents(&mut self, _requests: Vec<DocumentPath>) -> Vec<Document> {
            vec![Document::dummy()]
        }
    }
}
