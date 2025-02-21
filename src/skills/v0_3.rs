use exports::pharia::skill::skill_handler::SkillMetadata;
use pharia::skill::{
    chunking::{
        ChunkParams, ChunkRequest, ChunkWithOffset, ChunkWithOffsetRequest, Host as ChunkingHost,
    },
    document_index::{
        Document, DocumentPath, Host as DocumentIndexHost, IndexPath, MetadataFieldValue,
        MetadataFilter, MetadataFilterCondition, Modality, SearchFilter, SearchRequest,
        SearchResult, TextCursor,
    },
    inference::{
        ChatParams, ChatRequest, ChatResponse, Completion, CompletionParams, CompletionRequest,
        Distribution, ExplanationRequest, FinishReason, Granularity, Host as InferenceHost,
        Logprob, Logprobs, Message, TextScore, TokenUsage,
    },
    language::{Host as LanguageHost, SelectLanguageRequest},
};
use serde_json::Value;
use wasmtime::component::bindgen;

use crate::{chunking, inference, language_selection, search};

use super::LinkedCtx;

bindgen!({ world: "skill", path: "./wit/skill@0.3", async: true });

impl ChunkingHost for LinkedCtx {
    async fn chunk(&mut self, requests: Vec<ChunkRequest>) -> Vec<Vec<String>> {
        self.skill_ctx
            .chunk(requests.into_iter().map(Into::into).collect())
            .await
            .into_iter()
            .map(|response| response.into_iter().map(|chunk| chunk.text).collect())
            .collect()
    }

    async fn chunks_with_offsets(
        &mut self,
        requests: Vec<ChunkWithOffsetRequest>,
    ) -> Vec<Vec<ChunkWithOffset>> {
        self.skill_ctx
            .chunk(requests.into_iter().map(Into::into).collect())
            .await
            .into_iter()
            .map(|response| response.into_iter().map(Into::into).collect())
            .collect()
    }
}

impl DocumentIndexHost for LinkedCtx {
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

    async fn search(&mut self, requests: Vec<SearchRequest>) -> Vec<Vec<SearchResult>> {
        self.skill_ctx
            .search(requests.into_iter().map(Into::into).collect())
            .await
            .into_iter()
            .map(|results| results.into_iter().map(Into::into).collect())
            .collect()
    }
}

impl InferenceHost for LinkedCtx {
    async fn explain(&mut self, requests: Vec<ExplanationRequest>) -> Vec<Vec<TextScore>> {
        self.skill_ctx
            .explain(requests.into_iter().map(Into::into).collect())
            .await
            .into_iter()
            .map(|scores| scores.into_iter().map(Into::into).collect())
            .collect()
    }
    async fn chat(&mut self, requests: Vec<ChatRequest>) -> Vec<ChatResponse> {
        self.skill_ctx
            .chat(requests.into_iter().map(Into::into).collect())
            .await
            .into_iter()
            .map(Into::into)
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
}

impl LanguageHost for LinkedCtx {
    async fn select_language(
        &mut self,
        requests: Vec<SelectLanguageRequest>,
    ) -> Vec<Option<String>> {
        self.skill_ctx
            .select_language(requests.into_iter().map(Into::into).collect())
            .await
            .into_iter()
            .map(|r| r.map(Into::into))
            .collect()
    }
}

impl From<inference::TextScore> for TextScore {
    fn from(score: inference::TextScore) -> Self {
        let inference::TextScore {
            start,
            length,
            score,
        } = score;
        Self {
            start,
            length,
            score,
        }
    }
}

impl From<Granularity> for inference::Granularity {
    fn from(granularity: Granularity) -> Self {
        match granularity {
            Granularity::Auto => inference::Granularity::Auto,
            Granularity::Word => inference::Granularity::Word,
            Granularity::Sentence => inference::Granularity::Sentence,
            Granularity::Paragraph => inference::Granularity::Paragraph,
        }
    }
}

impl From<ExplanationRequest> for inference::ExplanationRequest {
    fn from(request: ExplanationRequest) -> Self {
        let ExplanationRequest {
            prompt,
            target,
            model,
            granularity,
        } = request;
        Self {
            prompt,
            target,
            model,
            granularity: granularity.into(),
        }
    }
}

impl TryFrom<SkillMetadata> for super::SkillMetadata {
    type Error = anyhow::Error;

    fn try_from(metadata: SkillMetadata) -> Result<Self, Self::Error> {
        let SkillMetadata {
            description,
            input_schema,
            output_schema,
        } = metadata;
        Ok(Self::V1(super::SkillMetadataV1 {
            description,
            input_schema: serde_json::from_slice::<Value>(&input_schema)?.try_into()?,
            output_schema: serde_json::from_slice::<Value>(&output_schema)?.try_into()?,
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
        let ChunkParams {
            model,
            max_tokens,
            overlap,
        } = params;
        Self {
            model,
            max_tokens,
            overlap,
        }
    }
}

impl From<ChunkRequest> for chunking::ChunkRequest {
    fn from(request: ChunkRequest) -> Self {
        let ChunkRequest { text, params } = request;
        Self {
            text,
            params: params.into(),
            character_offsets: false,
        }
    }
}

impl From<ChunkWithOffsetRequest> for chunking::ChunkRequest {
    fn from(request: ChunkWithOffsetRequest) -> Self {
        let ChunkWithOffsetRequest {
            text,
            params,
            character_offsets,
        } = request;
        Self {
            text,
            params: params.into(),
            character_offsets,
        }
    }
}

impl From<chunking::Chunk> for ChunkWithOffset {
    fn from(source: chunking::Chunk) -> Self {
        let chunking::Chunk {
            text,
            byte_offset,
            character_offset,
        } = source;
        Self {
            text,
            byte_offset,
            character_offset,
        }
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

impl From<SearchRequest> for search::SearchRequest {
    fn from(request: SearchRequest) -> Self {
        let SearchRequest {
            index_path,
            query,
            max_results,
            min_score,
            filters,
        } = request;
        Self {
            index_path: index_path.into(),
            query,
            max_results,
            min_score,
            filters: filters.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<SearchFilter> for search::Filter {
    fn from(value: SearchFilter) -> Self {
        match value {
            SearchFilter::Without(metadata_filters) => search::Filter::Without(
                metadata_filters
                    .into_iter()
                    .map(|f| search::FilterCondition::Metadata(f.into()))
                    .collect(),
            ),
            SearchFilter::WithOneOf(metadata_filters) => search::Filter::WithOneOf(
                metadata_filters
                    .into_iter()
                    .map(|f| search::FilterCondition::Metadata(f.into()))
                    .collect(),
            ),
            SearchFilter::WithAll(metadata_filters) => search::Filter::With(
                metadata_filters
                    .into_iter()
                    .map(|f| search::FilterCondition::Metadata(f.into()))
                    .collect(),
            ),
        }
    }
}

impl From<MetadataFilter> for search::MetadataFilter {
    fn from(value: MetadataFilter) -> Self {
        let MetadataFilter { field, condition } = value;
        Self {
            field,
            condition: condition.into(),
        }
    }
}

impl From<MetadataFilterCondition> for search::MetadataFilterCondition {
    fn from(value: MetadataFilterCondition) -> Self {
        match value {
            MetadataFilterCondition::GreaterThan(v) => {
                search::MetadataFilterCondition::GreaterThan(v)
            }
            MetadataFilterCondition::GreaterThanOrEqualTo(v) => {
                search::MetadataFilterCondition::GreaterThanOrEqualTo(v)
            }
            MetadataFilterCondition::LessThan(v) => search::MetadataFilterCondition::LessThan(v),
            MetadataFilterCondition::LessThanOrEqualTo(v) => {
                search::MetadataFilterCondition::LessThanOrEqualTo(v)
            }
            MetadataFilterCondition::After(v) => search::MetadataFilterCondition::After(v),
            MetadataFilterCondition::AtOrAfter(v) => search::MetadataFilterCondition::AtOrAfter(v),
            MetadataFilterCondition::Before(v) => search::MetadataFilterCondition::Before(v),
            MetadataFilterCondition::AtOrBefore(v) => {
                search::MetadataFilterCondition::AtOrBefore(v)
            }
            MetadataFilterCondition::EqualTo(metadata_field_value) => {
                search::MetadataFilterCondition::EqualTo(metadata_field_value.into())
            }
            MetadataFilterCondition::IsNull => {
                search::MetadataFilterCondition::IsNull(serde_bool::True)
            }
        }
    }
}

impl From<MetadataFieldValue> for search::MetadataFieldValue {
    fn from(value: MetadataFieldValue) -> Self {
        match value {
            MetadataFieldValue::StringType(v) => search::MetadataFieldValue::String(v),
            MetadataFieldValue::IntegerType(v) => search::MetadataFieldValue::Integer(v),
            MetadataFieldValue::BooleanType(v) => search::MetadataFieldValue::Boolean(v),
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

impl From<DocumentPath> for search::DocumentPath {
    fn from(document_path: DocumentPath) -> Self {
        let DocumentPath {
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

impl From<search::TextCursor> for TextCursor {
    fn from(cursor: search::TextCursor) -> Self {
        let search::TextCursor { item, position } = cursor;
        Self { item, position }
    }
}

impl From<search::SearchResult> for SearchResult {
    fn from(search_result: search::SearchResult) -> Self {
        let search::SearchResult {
            document_path,
            content,
            score,
            start,
            end,
        } = search_result;
        Self {
            document_path: document_path.into(),
            content,
            score,
            start: start.into(),
            end: end.into(),
        }
    }
}

impl From<inference::Completion> for Completion {
    fn from(completion: inference::Completion) -> Self {
        let inference::Completion {
            text,
            finish_reason,
            logprobs,
            usage,
        } = completion;
        Self {
            text,
            finish_reason: finish_reason.into(),
            logprobs: logprobs.into_iter().map(Into::into).collect(),
            usage: usage.into(),
        }
    }
}

impl From<inference::TokenUsage> for TokenUsage {
    fn from(usage: inference::TokenUsage) -> Self {
        let inference::TokenUsage { prompt, completion } = usage;
        Self { prompt, completion }
    }
}

impl From<Message> for inference::Message {
    fn from(message: Message) -> Self {
        let Message { role, content } = message;
        Self { role, content }
    }
}

impl From<ChatParams> for inference::ChatParams {
    fn from(params: ChatParams) -> Self {
        let ChatParams {
            max_tokens,
            temperature,
            top_p,
            frequency_penalty,
            presence_penalty,
            logprobs,
        } = params;
        Self {
            max_tokens,
            temperature,
            top_p,
            frequency_penalty,
            presence_penalty,
            logprobs: logprobs.into(),
        }
    }
}

impl From<Logprobs> for inference::Logprobs {
    fn from(logprobs: Logprobs) -> Self {
        match logprobs {
            Logprobs::No => inference::Logprobs::No,
            Logprobs::Sampled => inference::Logprobs::Sampled,
            Logprobs::Top(n) => inference::Logprobs::Top(n),
        }
    }
}

impl From<CompletionParams> for inference::CompletionParams {
    fn from(params: CompletionParams) -> Self {
        let CompletionParams {
            return_special_tokens,
            max_tokens,
            temperature,
            top_k,
            top_p,
            stop,
            frequency_penalty,
            presence_penalty,
            logprobs,
        } = params;

        Self {
            return_special_tokens,
            max_tokens,
            temperature,
            top_k,
            top_p,
            stop,
            frequency_penalty,
            presence_penalty,
            logprobs: logprobs.into(),
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

impl From<inference::Message> for Message {
    fn from(message: inference::Message) -> Self {
        let inference::Message { role, content } = message;
        Self { role, content }
    }
}

impl From<inference::ChatResponse> for ChatResponse {
    fn from(response: inference::ChatResponse) -> Self {
        let inference::ChatResponse {
            message,
            finish_reason,
            logprobs,
            usage,
        } = response;
        Self {
            message: message.into(),
            finish_reason: finish_reason.into(),
            logprobs: logprobs.into_iter().map(Into::into).collect(),
            usage: usage.into(),
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

impl From<inference::Distribution> for Distribution {
    fn from(logprob: inference::Distribution) -> Self {
        let inference::Distribution { sampled, top } = logprob;
        Self {
            sampled: sampled.into(),
            top: top.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<inference::Logprob> for Logprob {
    fn from(top_logprob: inference::Logprob) -> Self {
        let inference::Logprob { token, logprob } = top_logprob;
        Self { token, logprob }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_explain_request() {
        // Given
        let source = ExplanationRequest {
            prompt: "Hello, world!".to_string(),
            target: "world".to_string(),
            model: "model".to_string(),
            granularity: Granularity::Auto,
        };

        // When
        let result: inference::ExplanationRequest = source.into();

        // Then
        assert_eq!(result.prompt, "Hello, world!".to_string());
        assert_eq!(result.target, "world".to_string());
        assert_eq!(result.model, "model".to_string());
        assert_eq!(result.granularity, inference::Granularity::Auto);
    }

    #[test]
    fn forward_chunk_params() {
        // Given
        let model = "model";
        let max_tokens = 10;
        let overlap = 5;
        let text = "text";

        let source = ChunkRequest {
            text: text.to_owned(),
            params: ChunkParams {
                model: model.to_owned(),
                max_tokens,
                overlap,
            },
        };

        // When
        let result: chunking::ChunkRequest = source.into();

        // Then
        assert_eq!(
            result,
            chunking::ChunkRequest {
                text: text.to_owned(),
                params: chunking::ChunkParams {
                    model: model.to_owned(),
                    max_tokens,
                    overlap,
                },
                character_offsets: false,
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
            frequency_penalty: Some(0.8),
            presence_penalty: Some(0.7),
            logprobs: Logprobs::Top(2),
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
                logprobs: inference::Logprobs::Top(2),
            }
        );
    }

    #[test]
    fn forward_logprobs() {
        // Given
        let token: Vec<u8> = "Hello".to_string().into();
        let source = inference::ChatResponse {
            usage: inference::TokenUsage {
                prompt: 4,
                completion: 1,
            },
            message: inference::Message {
                role: "user".to_string(),
                content: "Hello, world!".to_string(),
            },
            finish_reason: inference::FinishReason::Stop,
            logprobs: vec![inference::Distribution {
                sampled: inference::Logprob {
                    token: token.clone(),
                    logprob: 0.0,
                },
                top: vec![inference::Logprob {
                    token: token.clone(),
                    logprob: 0.0,
                }],
            }],
        };

        // When
        let result: ChatResponse = source.into();

        // Then
        assert_eq!(result.logprobs.len(), 1);
        assert_eq!(result.logprobs[0].sampled.token, token);

        assert_eq!(result.logprobs[0].top.len(), 1);
        assert_eq!(result.logprobs[0].top[0].token, token);
    }

    #[test]
    fn forward_token_usage_chat() {
        // Given
        let source = inference::ChatResponse {
            usage: inference::TokenUsage {
                prompt: 4,
                completion: 1,
            },
            message: inference::Message {
                role: "user".to_string(),
                content: "Hello, world!".to_string(),
            },
            finish_reason: inference::FinishReason::Stop,
            logprobs: vec![],
        };

        // When
        let result: ChatResponse = source.into();

        // Then
        assert_eq!(result.usage.prompt, 4);
        assert_eq!(result.usage.completion, 1);
    }

    #[test]
    fn forward_token_usage_completion() {
        // Given
        let source = inference::Completion {
            text: "Hello, world!".to_string(),
            finish_reason: inference::FinishReason::Stop,
            logprobs: vec![],
            usage: inference::TokenUsage {
                prompt: 4,
                completion: 1,
            },
        };

        // When
        let result: Completion = source.into();

        // Then
        assert_eq!(result.usage.prompt, 4);
        assert_eq!(result.usage.completion, 1);
    }
}
