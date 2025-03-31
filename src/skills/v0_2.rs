use async_trait::async_trait;
use pharia::skill::csi::{
    ChatParams, ChatResponse, ChunkParams, Completion, CompletionParams, CompletionRequest,
    Document, DocumentPath, FinishReason, Host, IndexPath, Language, Message, Modality, Role,
    SearchResult,
};
use serde_json::Value;
use tokio::sync::mpsc;
use wasmtime::component::bindgen;

use crate::{
    chunking,
    csi::CsiForSkills,
    inference,
    language_selection::{self, SelectLanguageRequest},
    search::{self, SearchRequest},
};

use super::{AnySkillManifest, Engine, LinkedCtx, SkillError, SkillEvent};

bindgen!({ world: "skill", path: "./wit/skill@0.2", async: true });

#[async_trait]
impl super::Skill for SkillPre<LinkedCtx> {
    async fn manifest(
        &self,
        _engine: &Engine,
        _ctx: Box<dyn CsiForSkills + Send>,
    ) -> Result<AnySkillManifest, SkillError> {
        Ok(AnySkillManifest::V0)
    }

    async fn run_as_function(
        &self,
        engine: &Engine,
        ctx: Box<dyn CsiForSkills + Send>,
        input: Value,
    ) -> Result<Value, SkillError> {
        let mut store = engine.store(LinkedCtx::new(ctx));
        let input = serde_json::to_vec(&input).expect("Json is always serializable");
        let bindings = self.instantiate_async(&mut store).await.map_err(|e| {
            tracing::error!("Failed to instantiate skill: {}", e);
            SkillError::RuntimeError(e)
        })?;
        let result = bindings
            .pharia_skill_skill_handler()
            .call_run(store, &input)
            .await
            .map_err(SkillError::RuntimeError)?;
        let result = match result {
            Ok(result) => result,
            Err(e) => match e {
                exports::pharia::skill::skill_handler::Error::Internal(e) => {
                    return Err(SkillError::UserCode(e.to_string()));
                }
                exports::pharia::skill::skill_handler::Error::InvalidInput(e) => {
                    return Err(SkillError::InvalidInput(e.to_string()));
                }
            },
        };
        match serde_json::from_slice(&result) {
            Ok(result) => Ok(result),
            Err(e) => {
                // An operator might want to know that there is a buggy skill deployed.
                tracing::warn!("A skill returned invalid output: {}", e);
                Err(SkillError::InvalidOutput(e.to_string()))
            }
        }
    }

    async fn run_as_message_stream(
        &self,
        _engine: &Engine,
        _ctx: Box<dyn CsiForSkills + Send>,
        _input: Value,
        _sender: mpsc::Sender<SkillEvent>,
    ) -> Result<(), SkillError> {
        Err(SkillError::IsFunction)
    }
}

impl Host for LinkedCtx {
    #[must_use]
    async fn complete(
        &mut self,
        model: String,
        prompt: String,
        options: CompletionParams,
    ) -> Completion {
        let request = inference::CompletionRequest::new(prompt, model).with_params(options.into());
        self.skill_ctx
            .complete(vec![request])
            .await
            .remove(0)
            .into()
    }

    async fn complete_return_special_tokens(
        &mut self,
        model: String,
        prompt: String,
        options: CompletionParams,
    ) -> Completion {
        let CompletionParams {
            max_tokens,
            temperature,
            top_k,
            top_p,
            stop,
        } = options;
        let params = inference::CompletionParams {
            return_special_tokens: true,
            max_tokens,
            temperature,
            top_k,
            top_p,
            stop,
            frequency_penalty: None,
            presence_penalty: None,
            logprobs: inference::Logprobs::No,
        };
        let request = inference::CompletionRequest::new(prompt, model).with_params(params);
        self.skill_ctx
            .complete(vec![request])
            .await
            .remove(0)
            .into()
    }

    async fn chat(
        &mut self,
        model: String,
        messages: Vec<Message>,
        params: ChatParams,
    ) -> ChatResponse {
        let request = inference::ChatRequest {
            model,
            messages: messages.into_iter().map(Into::into).collect(),
            params: params.into(),
        };
        self.skill_ctx.chat(vec![request]).await.remove(0).into()
    }

    async fn chunk(&mut self, text: String, params: ChunkParams) -> Vec<String> {
        let request = chunking::ChunkRequest {
            text,
            params: params.into(),
            character_offsets: false,
        };
        self.skill_ctx
            .chunk(vec![request])
            .await
            .remove(0)
            .into_iter()
            .map(|chunk| chunk.text)
            .collect()
    }

    async fn select_language(
        &mut self,
        text: String,
        languages: Vec<Language>,
    ) -> Option<Language> {
        let languages = languages.into_iter().map(Into::into).collect::<Vec<_>>();
        let request = SelectLanguageRequest::new(text, languages);
        self.skill_ctx
            .select_language(vec![request])
            .await
            .remove(0)
            .map(Into::into)
    }

    async fn complete_all(&mut self, requests: Vec<CompletionRequest>) -> Vec<Completion> {
        let requests = requests.into_iter().map(Into::into).collect();

        self.skill_ctx
            .complete(requests)
            .await
            .into_iter()
            .map(Into::into)
            .collect()
    }

    async fn search(
        &mut self,
        index_path: IndexPath,
        query: String,
        max_results: u32,
        min_score: Option<f64>,
    ) -> Vec<SearchResult> {
        let request = SearchRequest {
            index_path: index_path.into(),
            query,
            max_results,
            min_score,
            filters: Vec::new(),
        };
        self.skill_ctx
            .search(vec![request])
            .await
            .remove(0)
            .into_iter()
            .map(Into::into)
            .collect()
    }

    async fn documents(&mut self, requests: Vec<DocumentPath>) -> Vec<Document> {
        let requests = requests.into_iter().map(Into::into).collect();
        self.skill_ctx
            .documents(requests)
            .await
            .into_iter()
            .map(Into::into)
            .collect()
    }

    async fn document_metadata(&mut self, document_path: DocumentPath) -> Option<Vec<u8>> {
        self.skill_ctx
            .document_metadata(vec![document_path.into()])
            .await
            .remove(0) // we know there will be exactly one document returned
            .map(|value| {
                serde_json::to_vec(&value).expect("Value should have valid to_bytes repr.")
            })
    }
}

impl From<ChunkParams> for chunking::ChunkParams {
    fn from(value: ChunkParams) -> Self {
        let ChunkParams { model, max_tokens } = value;
        Self {
            model,
            max_tokens,
            overlap: 0,
        }
    }
}

impl From<language_selection::Language> for Language {
    fn from(language: language_selection::Language) -> Self {
        match language {
            language_selection::Language(s) if s == "deu" => Language::Deu,
            language_selection::Language(s) if s == "eng" => Language::Eng,
            _ => unreachable!("Language not allowed as input"),
        }
    }
}

impl From<Language> for language_selection::Language {
    fn from(language: Language) -> Self {
        match language {
            Language::Eng => language_selection::Language::new("eng".to_owned()),
            Language::Deu => language_selection::Language::new("deu".to_owned()),
        }
    }
}

impl From<IndexPath> for search::IndexPath {
    fn from(index_path: IndexPath) -> Self {
        let IndexPath {
            namespace,
            collection,
            index,
        } = index_path;
        Self {
            namespace,
            collection,
            index,
        }
    }
}

impl From<search::SearchResult> for SearchResult {
    fn from(search_result: search::SearchResult) -> Self {
        let search::SearchResult {
            document_path,
            content,
            score,
            start: _,
            end: _,
        } = search_result;
        Self {
            document_path: document_path.into(),
            content,
            score,
        }
    }
}

impl From<search::DocumentPath> for DocumentPath {
    fn from(document_path: search::DocumentPath) -> Self {
        let search::DocumentPath {
            namespace,
            collection,
            name,
        } = document_path;
        Self {
            namespace,
            collection,
            name,
        }
    }
}

impl From<DocumentPath> for search::DocumentPath {
    fn from(document_path: DocumentPath) -> Self {
        Self {
            namespace: document_path.namespace,
            collection: document_path.collection,
            name: document_path.name,
        }
    }
}

impl From<search::Modality> for Modality {
    fn from(modality: search::Modality) -> Self {
        match modality {
            search::Modality::Text { text } => Modality::Text(text),
            search::Modality::Image { .. } => Modality::Image,
        }
    }
}

impl From<search::Document> for Document {
    fn from(document: search::Document) -> Self {
        let search::Document {
            path,
            contents,
            metadata,
        } = document;
        Self {
            path: path.into(),
            contents: contents.into_iter().map(Into::into).collect(),
            metadata: metadata
                .map(|v| serde_json::to_vec(&v).expect("Value should have valid to_bytes repr.")),
        }
    }
}

impl From<CompletionParams> for inference::CompletionParams {
    fn from(params: CompletionParams) -> Self {
        let CompletionParams {
            max_tokens,
            temperature,
            top_k,
            top_p,
            stop,
        } = params;
        Self {
            max_tokens,
            temperature,
            top_k,
            top_p,
            stop,
            return_special_tokens: false,
            frequency_penalty: None,
            presence_penalty: None,
            logprobs: inference::Logprobs::No,
        }
    }
}

impl From<CompletionRequest> for inference::CompletionRequest {
    fn from(request: CompletionRequest) -> Self {
        let CompletionRequest {
            prompt,
            model,
            params,
        } = request;
        Self {
            prompt,
            model,
            params: params.into(),
        }
    }
}

impl From<inference::Completion> for Completion {
    fn from(completion: inference::Completion) -> Self {
        let inference::Completion {
            text,
            finish_reason,
            logprobs: _,
            usage: _,
        } = completion;
        Self {
            text,
            finish_reason: finish_reason.into(),
        }
    }
}

impl From<Message> for inference::Message {
    fn from(message: Message) -> Self {
        let Message { role, content } = message;
        Self {
            role: role.into(),
            content,
        }
    }
}

impl From<ChatParams> for inference::ChatParams {
    fn from(params: ChatParams) -> Self {
        let ChatParams {
            max_tokens,
            temperature,
            top_p,
        } = params;
        Self {
            max_tokens,
            temperature,
            top_p,
            frequency_penalty: None,
            presence_penalty: None,
            logprobs: inference::Logprobs::No,
        }
    }
}

impl From<Role> for String {
    fn from(role: Role) -> Self {
        match role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System => "system",
        }
        .to_owned()
    }
}

impl From<String> for Role {
    fn from(role: String) -> Self {
        match role.as_str() {
            "user" => Self::User,
            "system" => Self::System,
            // An unsupported role can happen if the api scheduler introduces more roles in the future.
            // It is unclear what to pass to the skill in this case, as it only knows three roles.
            // We could terminate skill execution, but as we know that this will be a reply from the model,
            // returning assistant seems like a sensible fallback.
            _ => Self::Assistant,
        }
    }
}

impl From<inference::Message> for Message {
    fn from(message: inference::Message) -> Self {
        let inference::Message { role, content } = message;
        Self {
            role: role.into(),
            content,
        }
    }
}

impl From<inference::ChatResponse> for ChatResponse {
    fn from(response: inference::ChatResponse) -> Self {
        let inference::ChatResponse {
            message,
            finish_reason,
            logprobs: _,
            usage: _,
        } = response;
        Self {
            message: message.into(),
            finish_reason: finish_reason.into(),
        }
    }
}

impl From<inference::FinishReason> for FinishReason {
    fn from(finish_reason: inference::FinishReason) -> Self {
        match finish_reason {
            inference::FinishReason::Stop => Self::Stop,
            inference::FinishReason::Length => Self::Length,
            inference::FinishReason::ContentFilter => Self::ContentFilter,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::vec;

    use super::*;

    #[test]
    fn forward_chunk_params() {
        // Given
        let model = "model";
        let max_tokens = 10;

        let source = ChunkParams {
            model: model.to_owned(),
            max_tokens,
        };

        // When
        let result: chunking::ChunkParams = source.into();

        // Then
        assert_eq!(
            result,
            chunking::ChunkParams {
                model: model.to_owned(),
                max_tokens,
                overlap: 0,
            }
        );
    }

    #[test]
    fn forward_chat_params() {
        // Given
        let source = ChatParams {
            max_tokens: Some(10),
            temperature: Some(0.5),
            top_p: Some(0.9),
        };

        // When
        let result: inference::ChatParams = source.into();

        // Then
        assert_eq!(
            result,
            inference::ChatParams {
                max_tokens: Some(10),
                temperature: Some(0.5),
                top_p: Some(0.9),
                frequency_penalty: None,
                presence_penalty: None,
                logprobs: inference::Logprobs::No,
            }
        );
    }

    #[test]
    fn forward_completion_params() {
        // Given
        let source = CompletionParams {
            max_tokens: Some(10),
            temperature: Some(0.5),
            top_k: Some(5),
            top_p: Some(0.9),
            stop: vec!["stop".to_string()],
        };

        // When
        let result: inference::CompletionParams = source.into();

        // Then
        assert_eq!(
            result,
            inference::CompletionParams {
                return_special_tokens: false,
                max_tokens: Some(10),
                temperature: Some(0.5),
                top_k: Some(5),
                top_p: Some(0.9),
                stop: vec!["stop".to_string()],
                frequency_penalty: None,
                presence_penalty: None,
                logprobs: inference::Logprobs::No,
            }
        );
    }
}
