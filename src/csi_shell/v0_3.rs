/// CSI Shell version 0.3
///
/// This module introduces serializable/user-facing structs which are very similar to our "internal" representations.
/// While this module may appear to contain a lot of boilerplate code, there is a good reason to not serialize our "internal" representations:
/// It allows us to keep our external interface stable while updating our "internal" representations.
/// Imagine we introduce a new version (0.4) with breaking changes in the api (e.g. a new field in `CompletionParams`).
/// If we simply serialized the internal representation, we would break clients going against the 0.3 version of the CSI shell.
use serde::Deserialize;
use serde_json::{json, Value};

use crate::csi_shell::CsiShellError;
use crate::{chunking, csi::Csi, inference, language_selection, search};

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "function")]
pub enum CsiRequest {
    Chunk {
        requests: Vec<ChunkRequest>,
    },
    SelectLanguage {
        requests: Vec<SelectLanguageRequest>,
    },
    Complete {
        requests: Vec<CompletionRequest>,
    },
    Search {
        requests: Vec<SearchRequest>,
    },
    Chat {
        requests: Vec<ChatRequest>,
    },
    Documents {
        requests: Vec<DocumentPath>,
    },
    DocumentMetadata {
        requests: Vec<DocumentPath>,
    },
    #[serde(untagged)]
    Unknown {
        function: Option<String>,
    },
}

impl CsiRequest {
    pub async fn act<C>(self, drivers: &C, auth: String) -> Result<Value, CsiShellError>
    where
        C: Csi + Sync,
    {
        let result = match self {
            CsiRequest::Chunk { requests } => drivers
                .chunk(auth, requests.into_iter().map(Into::into).collect())
                .await
                .map(|r| json!(r))?,
            CsiRequest::SelectLanguage { requests } => drivers
                .select_language(requests.into_iter().map(Into::into).collect())
                .await
                .map(|r| json!(r))?,
            CsiRequest::Complete { requests } => drivers
                .complete(auth, requests.into_iter().map(Into::into).collect())
                .await
                .map(|v| json!(v))?,
            CsiRequest::Search { requests } => drivers
                .search(auth, requests.into_iter().map(Into::into).collect())
                .await
                .map(|v| json!(v))?,
            CsiRequest::Chat { requests } => drivers
                .chat(auth, requests.into_iter().map(Into::into).collect())
                .await
                .map(|v| json!(v))?,
            CsiRequest::DocumentMetadata { requests } => drivers
                .document_metadata(auth, requests.into_iter().map(Into::into).collect())
                .await
                .map(|r| json!(r))?,
            CsiRequest::Documents { requests } => drivers
                .documents(auth, requests.into_iter().map(Into::into).collect())
                .await
                .map(|r| json!(r))?,
            CsiRequest::Unknown { function } => {
                return Err(CsiShellError::UnknownFunction(
                    function.unwrap_or_else(|| "specified".to_owned()),
                ));
            }
        };
        Ok(result)
    }
}

#[derive(Deserialize)]
pub struct DocumentPath {
    pub namespace: String,
    pub collection: String,
    pub name: String,
}

impl From<DocumentPath> for search::DocumentPath {
    fn from(value: DocumentPath) -> Self {
        let DocumentPath {
            namespace,
            collection,
            name,
        } = value;
        search::DocumentPath {
            namespace,
            collection,
            name,
        }
    }
}

#[derive(Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl From<Message> for inference::Message {
    fn from(value: Message) -> Self {
        let Message { role, content } = value;
        inference::Message { role, content }
    }
}

#[derive(Deserialize)]
pub struct ChatParams {
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub frequency_penalty: Option<f64>,
    pub presence_penalty: Option<f64>,
    pub logprobs: Logprobs,
}

impl From<ChatParams> for inference::ChatParams {
    fn from(value: ChatParams) -> Self {
        let ChatParams {
            max_tokens,
            temperature,
            top_p,
            frequency_penalty,
            presence_penalty,
            logprobs,
        } = value;
        inference::ChatParams {
            max_tokens,
            temperature,
            top_p,
            frequency_penalty,
            presence_penalty,
            logprobs: logprobs.into(),
        }
    }
}

#[derive(Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub params: ChatParams,
}

impl From<ChatRequest> for inference::ChatRequest {
    fn from(value: ChatRequest) -> Self {
        let ChatRequest {
            model,
            messages,
            params,
        } = value;
        inference::ChatRequest {
            model,
            messages: messages.into_iter().map(Into::into).collect(),
            params: params.into(),
        }
    }
}

#[derive(Deserialize)]
pub struct IndexPath {
    pub namespace: String,
    pub collection: String,
    pub index: String,
}

impl From<IndexPath> for search::IndexPath {
    fn from(value: IndexPath) -> Self {
        let IndexPath {
            namespace,
            collection,
            index,
        } = value;
        search::IndexPath {
            namespace,
            collection,
            index,
        }
    }
}

#[derive(Deserialize)]
pub struct SearchRequest {
    pub query: String,
    pub index_path: IndexPath,
    pub max_results: u32,
    pub min_score: Option<f64>,
    pub filters: Vec<Filter>,
}

impl From<SearchRequest> for search::SearchRequest {
    fn from(value: SearchRequest) -> Self {
        let SearchRequest {
            query,
            index_path,
            max_results,
            min_score,
            filters,
        } = value;
        search::SearchRequest {
            query,
            index_path: index_path.into(),
            max_results,
            min_score,
            filters: filters.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Filter {
    Without(Vec<FilterCondition>),
    WithOneOf(Vec<FilterCondition>),
    With(Vec<FilterCondition>),
}

impl From<Filter> for search::Filter {
    fn from(value: Filter) -> Self {
        match value {
            Filter::Without(items) => {
                search::Filter::Without(items.into_iter().map(Into::into).collect())
            }
            Filter::WithOneOf(items) => {
                search::Filter::WithOneOf(items.into_iter().map(Into::into).collect())
            }
            Filter::With(items) => {
                search::Filter::With(items.into_iter().map(Into::into).collect())
            }
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum FilterCondition {
    Modality(ModalityType),
    Metadata(MetadataFilter),
}

impl From<FilterCondition> for search::FilterCondition {
    fn from(value: FilterCondition) -> Self {
        match value {
            FilterCondition::Modality(modality_type) => {
                search::FilterCondition::Modality(modality_type.into())
            }
            FilterCondition::Metadata(metadata_filter) => {
                search::FilterCondition::Metadata(metadata_filter.into())
            }
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ModalityType {
    Text,
}

impl From<ModalityType> for search::ModalityType {
    fn from(value: ModalityType) -> Self {
        match value {
            ModalityType::Text => search::ModalityType::Text,
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct MetadataFilter {
    pub field: String,
    #[serde(flatten)]
    pub condition: MetadataFilterCondition,
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

#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum MetadataFilterCondition {
    GreaterThan(f64),
    GreaterThanOrEqualTo(f64),
    LessThan(f64),
    LessThanOrEqualTo(f64),
    After(String),
    AtOrAfter(String),
    Before(String),
    AtOrBefore(String),
    EqualTo(MetadataFieldValue),
    IsNull(serde_bool::True),
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
            MetadataFilterCondition::EqualTo(v) => {
                search::MetadataFilterCondition::EqualTo(v.into())
            }
            MetadataFilterCondition::IsNull(v) => search::MetadataFilterCondition::IsNull(v),
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
pub enum MetadataFieldValue {
    String(String),
    Integer(i64),
    Boolean(bool),
}

impl From<MetadataFieldValue> for search::MetadataFieldValue {
    fn from(value: MetadataFieldValue) -> Self {
        match value {
            MetadataFieldValue::String(v) => search::MetadataFieldValue::String(v),
            MetadataFieldValue::Integer(v) => search::MetadataFieldValue::Integer(v),
            MetadataFieldValue::Boolean(v) => search::MetadataFieldValue::Boolean(v),
        }
    }
}

#[derive(Deserialize)]
pub struct ChunkParams {
    pub model: String,
    pub max_tokens: u32,
    pub overlap: u32,
}

impl From<ChunkParams> for chunking::ChunkParams {
    fn from(value: ChunkParams) -> Self {
        let ChunkParams {
            model,
            max_tokens,
            overlap,
        } = value;
        chunking::ChunkParams {
            model,
            max_tokens,
            overlap,
        }
    }
}

#[derive(Deserialize)]
pub struct ChunkRequest {
    pub text: String,
    pub params: ChunkParams,
}

impl From<ChunkRequest> for chunking::ChunkRequest {
    fn from(value: ChunkRequest) -> Self {
        let ChunkRequest { text, params } = value;
        chunking::ChunkRequest {
            text,
            params: params.into(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Afr,
    Ara,
    Aze,
    Bel,
    Ben,
    Bos,
    Bul,
    Cat,
    Ces,
    Cym,
    Dan,
    Deu,
    Ell,
    Eng,
    Epo,
    Est,
    Eus,
    Fas,
    Fin,
    Fra,
    Gle,
    Guj,
    Heb,
    Hin,
    Hrv,
    Hun,
    Hye,
    Ind,
    Isl,
    Ita,
    Jpn,
    Kat,
    Kaz,
    Kor,
    Lat,
    Lav,
    Lit,
    Lug,
    Mar,
    Mkd,
    Mon,
    Mri,
    Msa,
    Nld,
    Nno,
    Nob,
    Pan,
    Pol,
    Por,
    Ron,
    Rus,
    Slk,
    Slv,
    Sna,
    Som,
    Sot,
    Spa,
    Sqi,
    Srp,
    Swa,
    Swe,
    Tam,
    Tel,
    Tgl,
    Tha,
    Tsn,
    Tso,
    Tur,
    Ukr,
    Urd,
    Vie,
    Xho,
    Yor,
    Zho,
    Zul,
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

#[derive(Deserialize)]
pub struct SelectLanguageRequest {
    pub text: String,
    pub languages: Vec<Language>,
}

impl From<SelectLanguageRequest> for language_selection::SelectLanguageRequest {
    fn from(value: SelectLanguageRequest) -> Self {
        let SelectLanguageRequest { text, languages } = value;
        language_selection::SelectLanguageRequest {
            text,
            languages: languages.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Logprobs {
    No,
    Sampled,
    Top(u8),
}

impl From<Logprobs> for inference::Logprobs {
    fn from(value: Logprobs) -> Self {
        match value {
            Logprobs::No => inference::Logprobs::No,
            Logprobs::Sampled => inference::Logprobs::Sampled,
            Logprobs::Top(n) => inference::Logprobs::Top(n),
        }
    }
}

#[derive(Deserialize)]
pub struct CompletionParams {
    pub return_special_tokens: bool,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub top_k: Option<u32>,
    pub top_p: Option<f64>,
    pub stop: Vec<String>,
    pub frequency_penalty: Option<f64>,
    pub presence_penalty: Option<f64>,
    pub logprobs: Logprobs,
}

impl From<CompletionParams> for inference::CompletionParams {
    fn from(value: CompletionParams) -> Self {
        let CompletionParams {
            return_special_tokens,
            max_tokens,
            temperature,
            top_k,
            top_p,
            frequency_penalty,
            presence_penalty,
            logprobs,
            stop,
        } = value;
        inference::CompletionParams {
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

#[derive(Deserialize)]
pub struct CompletionRequest {
    pub prompt: String,
    pub model: String,
    pub params: CompletionParams,
}

impl From<CompletionRequest> for inference::CompletionRequest {
    fn from(value: CompletionRequest) -> Self {
        let CompletionRequest {
            prompt,
            model,
            params,
        } = value;
        inference::CompletionRequest {
            prompt,
            model,
            params: params.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::csi_shell::VersionedCsiRequest;

    #[test]
    fn chunk_request() {
        let request = json!({
            "version": "0.3",
            "function": "chunk",
            "requests": [
                {
                    "text": "Hello",
                    "params": {
                        "model": "pharia-1-llm-7b-control",
                        "max_tokens": 128,
                        "overlap": 10
                    }
                },
                {
                    "text": "Hello",
                    "params": {
                        "model": "pharia-1-llm-7b-control",
                        "max_tokens": 128,
                        "overlap": 10
                    }
                }
            ]
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_3(CsiRequest::Chunk { requests })) if requests.len() == 2
        ));
    }

    #[test]
    fn select_language_request() {
        let request = json!({
            "version": "0.3",
            "function": "select_language",
            "requests": [
                {
                    "text": "Hello",
                    "languages": ["eng", "deu"]
                },
                {
                    "text": "Hello",
                    "languages": ["eng", "deu"]
                }
            ]
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_3(CsiRequest::SelectLanguage { requests })) if requests.len() == 2
        ));
    }
    #[test]
    fn complete_request() {
        let request = json!({
            "version": "0.3",
            "function": "complete",
            "requests": [
                {
                    "prompt": "Hello",
                    "model": "pharia-1-llm-7b-control",
                    "params": {
                        "return_special_tokens": true,
                        "max_tokens": 128,
                        "temperature": null,
                        "top_k": null,
                        "top_p": null,
                        "stop": [],
                        "frequency_penalty": null,
                        "presence_penalty": null,
                        "logprobs": "no"
                    }
                },
                {
                    "prompt": "Hello",
                    "model": "pharia-1-llm-7b-control",
                    "params": {
                        "return_special_tokens": true,
                        "max_tokens": 128,
                        "temperature": null,
                        "top_k": null,
                        "top_p": null,
                        "stop": [],
                        "frequency_penalty": null,
                        "presence_penalty": null,
                        "logprobs": {
                            "top": 10
                        }
                    }
                }
            ]
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_3(CsiRequest::Complete { requests })) if requests.len() == 2
        ));
    }

    #[test]
    fn search_request() {
        let request = json!({
            "version": "0.3",
            "function": "search",
            "requests": [
                {
                    "query": "Hello",
                    "index_path": {
                        "namespace": "Kernel",
                        "collection": "test",
                        "index": "asym-64"
                    },
                    "max_results": 10,
                    "min_score": null,
                    "filters": [
                        {
                            "with": [
                                {
                                    "metadata": {
                                        "field": "created",
                                        "after": "1970-07-01T14:10:11Z"
                                    }
                                }
                            ]
                        }
                    ]
                },
                {
                    "query": "Hello",
                    "index_path": {
                        "namespace": "Kernel",
                        "collection": "test",
                        "index": "asym-64"
                    },
                    "max_results": 10,
                    "min_score": null,
                    "filters": []
                }
            ]
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_3(CsiRequest::Search { requests })) if requests.len() == 2
        ));
    }

    #[test]
    fn chat_request() {
        let request = json!({
            "version": "0.3",
            "function": "chat",
            "requests": [
                {
                    "model": "pharia-1-llm-7b-control",
                    "messages": [
                        {
                            "role": "user",
                            "content": "Hello"
                        }
                    ],
                    "params": {
                        "max_tokens": 128,
                        "temperature": null,
                        "top_k": null,
                        "top_p": null,
                        "stop": [],
                        "logprobs": "no"
                    },
                }
            ]
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(matches!(
            result,
            Ok(VersionedCsiRequest::V0_3(CsiRequest::Chat { requests })) if requests.len() == 1
        ));
    }

    #[test]
    fn documents_request() {
        let request = json!({
            "version": "0.3",
            "function": "documents",
            "requests": [
                {
                    "namespace": "Kernel",
                    "collection": "test",
                    "name": "kernel-docs"
                }
            ]
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(
            matches!(result, Ok(VersionedCsiRequest::V0_3(CsiRequest::Documents { requests })) if requests.len() == 1)
        );
    }

    #[test]
    fn document_metadata_request() {
        let request = json!({
            "version": "0.3",
            "function": "document_metadata",
            "requests": [
                {
                    "namespace": "Kernel",
                    "collection": "test",
                    "name": "asym-64"
                },
                {
                    "namespace": "Kernel",
                    "collection": "test",
                    "name": "asym-64"
                }
            ]
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(
            matches!(result, Ok(VersionedCsiRequest::V0_3(CsiRequest::DocumentMetadata { requests })) if requests.len() == 2)
        );
    }

    #[test]
    fn unknown_request() {
        let request = json!({
            "version": "0.3",
            "function": "whatever",
        });

        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        assert!(
            matches!(result, Ok(VersionedCsiRequest::V0_3(CsiRequest::Unknown { function: Some(function) })) if function == "whatever")
        );
    }
}
