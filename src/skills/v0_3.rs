use exports::pharia::skill::skill_handler::SkillMetadata;
use pharia::skill::csi::{
    ChatParams, ChatRequest, ChatResponse, ChunkParams, ChunkRequest, Completion, CompletionParams,
    CompletionRequest, Distribution, Document, DocumentPath, FinishReason, Host, IndexPath,
    Language, Logprob, Logprobs, Message, Modality, SearchRequest, SearchResult,
    SelectLanguageRequest,
};
use serde_json::Value;
use wasmtime::component::bindgen;

use crate::{chunking, inference, language_selection, search};

use super::LinkedCtx;

bindgen!({ world: "skill", path: "./wit/skill@0.3", async: true });

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

impl From<SelectLanguageRequest> for language_selection::SelectLanguageRequest {
    fn from(request: SelectLanguageRequest) -> Self {
        let SelectLanguageRequest { text, languages } = request;
        Self {
            text,
            languages: languages.into_iter().map(Into::into).collect(),
        }
    }
}

// Works as long as variant names match exactly
macro_rules! language_mappings {
    ($($variant:ident),*) => {
        impl From<Language> for language_selection::Language {
            fn from(language: Language) -> Self {
                match language {
                    $(Language::$variant => language_selection::Language::$variant),*
                }
            }
        }

        impl From<language_selection::Language> for Language {
            fn from(language: language_selection::Language) -> Self {
                match language {
                    $(language_selection::Language::$variant => Language::$variant),*
                }
            }
        }
    };
}

language_mappings!(
    Afr, Ara, Aze, Bel, Ben, Bos, Bul, Cat, Ces, Cym, Dan, Deu, Ell, Eng, Epo, Est, Eus, Fas, Fin,
    Fra, Gle, Guj, Heb, Hin, Hrv, Hun, Hye, Ind, Isl, Ita, Jpn, Kat, Kaz, Kor, Lat, Lav, Lit, Lug,
    Mar, Mkd, Mon, Mri, Msa, Nld, Nno, Nob, Pan, Pol, Por, Ron, Rus, Slk, Slv, Sna, Som, Sot, Spa,
    Sqi, Srp, Swa, Swe, Tam, Tel, Tgl, Tha, Tsn, Tso, Tur, Ukr, Urd, Vie, Xho, Yor, Zho, Zul
);

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

impl From<search::SearchResult> for SearchResult {
    fn from(search_result: search::SearchResult) -> Self {
        let search::SearchResult {
            document_path,
            content,
            score,
        } = search_result;
        Self {
            document_path: document_path.into(),
            content,
            score,
        }
    }
}

impl From<inference::Completion> for Completion {
    fn from(completion: inference::Completion) -> Self {
        let inference::Completion {
            text,
            finish_reason,
            logprobs,
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
        } = response;
        Self {
            message: message.into(),
            finish_reason: finish_reason.into(),
            logprobs: logprobs.into_iter().map(Into::into).collect(),
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
}
