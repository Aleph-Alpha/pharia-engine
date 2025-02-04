use exports::pharia::skill::skill_handler::SkillMetadata;
use pharia::skill::csi::{
    ChatParams, ChatRequest, ChatResponse, ChunkParams, ChunkRequest, Completion, CompletionParams,
    CompletionRequest, Document, DocumentPath, FinishReason, Host, IndexPath, Language, Message,
    Modality, SearchRequest, SearchResult, SelectLanguageRequest,
};
use serde_json::Value;
use wasmtime::component::bindgen;

use crate::{csi::chunking, inference, language_selection, search};

use super::LinkedCtx;

bindgen!({ world: "skill", path: "./wit/skill@0.3", async: true });

impl TryFrom<SkillMetadata> for super::SkillMetadata {
    type Error = anyhow::Error;

    fn try_from(metadata: SkillMetadata) -> Result<Self, Self::Error> {
        Ok(Self::V1(super::SkillMetadataV1 {
            description: metadata.description,
            input_schema: serde_json::from_slice::<Value>(&metadata.input_schema)?.try_into()?,
            output_schema: serde_json::from_slice::<Value>(&metadata.output_schema)?.try_into()?,
        }))
    }
}

impl From<ChatRequest> for inference::ChatRequest {
    fn from(request: ChatRequest) -> Self {
        let ChatRequest {
            model,
            messages,
            params,
        } = request;
        Self {
            model,
            messages: messages.into_iter().map(Into::into).collect(),
            params: params.into(),
        }
    }
}

impl From<ChunkParams> for chunking::ChunkParams {
    fn from(params: ChunkParams) -> Self {
        let ChunkParams { model, max_tokens } = params;
        Self { model, max_tokens }
    }
}

impl From<ChunkRequest> for chunking::ChunkRequest {
    fn from(request: ChunkRequest) -> Self {
        let ChunkRequest { text, params } = request;
        Self {
            text,
            params: params.into(),
        }
    }
}

impl Host for LinkedCtx {
    async fn chat(&mut self, requests: Vec<ChatRequest>) -> Vec<ChatResponse> {
        self.skill_ctx
            .chat(requests.into_iter().map(Into::into).collect())
            .await
            .into_iter()
            .map(Into::into)
            .collect()
    }

    async fn chunk(&mut self, requests: Vec<ChunkRequest>) -> Vec<Vec<String>> {
        self.skill_ctx
            .chunk(requests.into_iter().map(Into::into).collect())
            .await
    }

    async fn select_language(
        &mut self,
        requests: Vec<SelectLanguageRequest>,
    ) -> Vec<Option<Language>> {
        self.skill_ctx
            .select_language(requests.into_iter().map(Into::into).collect())
            .await
            .into_iter()
            .map(|r| r.map(Into::into))
            .collect()
    }

    async fn complete(&mut self, requests: Vec<CompletionRequest>) -> Vec<Completion> {
        self.skill_ctx
            .complete(requests.into_iter().map(Into::into).collect())
            .await
            .into_iter()
            .map(Into::into)
            .collect()
    }

    async fn search(&mut self, requests: Vec<SearchRequest>) -> Vec<Vec<SearchResult>> {
        self.skill_ctx
            .search(requests.into_iter().map(Into::into).collect())
            .await
            .into_iter()
            .map(|results| results.into_iter().map(Into::into).collect())
            .collect()
    }

    async fn documents(&mut self, requests: Vec<DocumentPath>) -> Vec<Document> {
        self.skill_ctx
            .documents(requests.into_iter().map(Into::into).collect())
            .await
            .into_iter()
            .map(Into::into)
            .collect()
    }

    async fn document_metadata(&mut self, requests: Vec<DocumentPath>) -> Vec<Option<Vec<u8>>> {
        self.skill_ctx
            .document_metadata(requests.into_iter().map(Into::into).collect())
            .await
            .into_iter()
            .map(|value| {
                value.map(|v| {
                    serde_json::to_vec(&v).expect("Value should have valid to_bytes repr.")
                })
            })
            .collect()
    }
}

impl From<SelectLanguageRequest> for language_selection::SelectLanguageRequest {
    fn from(request: SelectLanguageRequest) -> Self {
        let SelectLanguageRequest { text, languages } = request;
        Self {
            text,
            languages: languages.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<language_selection::Language> for Language {
    fn from(language: language_selection::Language) -> Self {
        match language {
            language_selection::Language::Eng => Language::Eng,
            language_selection::Language::Deu => Language::Deu,
        }
    }
}

impl From<SearchRequest> for search::SearchRequest {
    fn from(request: SearchRequest) -> Self {
        let SearchRequest {
            index_path,
            query,
            max_results,
            min_score,
        } = request;
        Self {
            index_path: index_path.into(),
            query,
            max_results,
            min_score,
        }
    }
}

impl From<Language> for language_selection::Language {
    fn from(language: Language) -> Self {
        match language {
            Language::Eng => language_selection::Language::Eng,
            Language::Deu => language_selection::Language::Deu,
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
        Self {
            path: document.path.into(),
            contents: document.contents.into_iter().map(Into::into).collect(),
            metadata: document
                .metadata
                .map(|v| serde_json::to_vec(&v).expect("Value should have valid to_bytes repr.")),
        }
    }
}

impl From<IndexPath> for search::IndexPath {
    fn from(index_path: IndexPath) -> Self {
        Self {
            namespace: index_path.namespace,
            collection: index_path.collection,
            index: index_path.index,
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

impl From<search::DocumentPath> for DocumentPath {
    fn from(document_path: search::DocumentPath) -> Self {
        Self {
            namespace: document_path.namespace,
            collection: document_path.collection,
            name: document_path.name,
        }
    }
}

impl From<search::SearchResult> for SearchResult {
    fn from(search_result: search::SearchResult) -> Self {
        Self {
            document_path: search_result.document_path.into(),
            content: search_result.content,
            score: search_result.score,
        }
    }
}

impl From<inference::Completion> for Completion {
    fn from(completion: inference::Completion) -> Self {
        Self {
            text: completion.text,
            finish_reason: completion.finish_reason.into(),
        }
    }
}

impl From<Message> for inference::Message {
    fn from(message: Message) -> Self {
        Self {
            role: message.role,
            content: message.content,
        }
    }
}

impl From<ChatParams> for inference::ChatParams {
    fn from(params: ChatParams) -> Self {
        Self {
            max_tokens: params.max_tokens,
            temperature: params.temperature,
            top_p: params.top_p,
            frequency_penalty: params.frequency_penalty,
            presence_penalty: params.presence_penalty,
        }
    }
}

impl From<CompletionParams> for inference::CompletionParams {
    fn from(params: CompletionParams) -> Self {
        Self {
            return_special_tokens: params.return_special_tokens,
            max_tokens: params.max_tokens,
            temperature: params.temperature,
            top_k: params.top_k,
            top_p: params.top_p,
            stop: params.stop,
        }
    }
}

impl From<CompletionRequest> for inference::CompletionRequest {
    fn from(request: CompletionRequest) -> Self {
        Self {
            prompt: request.prompt,
            model: request.model,
            params: request.params.into(),
        }
    }
}

impl From<inference::Message> for Message {
    fn from(message: inference::Message) -> Self {
        Self {
            role: message.role,
            content: message.content,
        }
    }
}

impl From<inference::ChatResponse> for ChatResponse {
    fn from(response: inference::ChatResponse) -> Self {
        Self {
            message: response.message.into(),
            finish_reason: response.finish_reason.into(),
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
    use super::*;

    #[test]
    fn forward_chat_params() {
        // Given
        let source = ChatParams {
            max_tokens: Some(10),
            temperature: Some(0.5),
            top_p: Some(0.9),
            frequency_penalty: Some(0.8),
            presence_penalty: Some(0.7),
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
                frequency_penalty: Some(0.8),
                presence_penalty: Some(0.7),
            }
        );
    }
}
