use pharia::skill::csi::{
    ChatParams, ChatResponse, ChunkParams, Completion, CompletionParams, CompletionRequest,
    Document, DocumentPath, FinishReason, Host, IndexPath, Language, Message, Modality, Role,
    SearchResult,
};
use wasmtime::component::bindgen;

use crate::{
    csi::ChunkRequest,
    inference,
    language_selection::{self, SelectLanguageRequest},
    search::{self, SearchRequest},
};

use super::LinkedCtx;

bindgen!({ world: "skill", path: "./wit/skill@0.2", async: true });

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
        let ChunkParams { model, max_tokens } = params;
        let request = ChunkRequest::new(text, model, max_tokens);
        self.skill_ctx.chunk(vec![request]).await.remove(0)
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

impl From<language_selection::Language> for Language {
    fn from(language: language_selection::Language) -> Self {
        match language {
            language_selection::Language::Deu => Language::Deu,
            language_selection::Language::Eng => Language::Eng,
            _ => unreachable!("Language not allowed as input"),
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

impl From<IndexPath> for search::IndexPath {
    fn from(index_path: IndexPath) -> Self {
        Self {
            namespace: index_path.namespace,
            collection: index_path.collection,
            index: index_path.index,
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

impl From<search::DocumentPath> for DocumentPath {
    fn from(document_path: search::DocumentPath) -> Self {
        Self {
            namespace: document_path.namespace,
            collection: document_path.collection,
            name: document_path.name,
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
        Self {
            path: document.path.into(),
            contents: document.contents.into_iter().map(Into::into).collect(),
            metadata: document
                .metadata
                .map(|v| serde_json::to_vec(&v).expect("Value should have valid to_bytes repr.")),
        }
    }
}

impl From<CompletionParams> for inference::CompletionParams {
    fn from(params: CompletionParams) -> Self {
        Self {
            return_special_tokens: false,
            max_tokens: params.max_tokens,
            temperature: params.temperature,
            top_k: params.top_k,
            top_p: params.top_p,
            stop: params.stop,
            frequency_penalty: None,
            presence_penalty: None,
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
            role: message.role.into(),
            content: message.content,
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
        Self {
            role: message.role.into(),
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
    use std::vec;

    use super::*;

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
            }
        );
    }
}
