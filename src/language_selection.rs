use derive_more::{Constructor, Deref, From, Into};
use lingua::LanguageDetectorBuilder;
use thiserror::Error;

use crate::{context_event, logging::TracingContext};

#[derive(Error, Debug, PartialEq, Eq)]
pub enum LanguageError {
    #[error("the language '{0}' is unsupported")]
    Unsupported(String),
}

#[derive(Constructor, Debug, Deref, From, Into, Eq, PartialEq)]
pub struct Language(pub String);

impl TryFrom<Language> for lingua::Language {
    type Error = LanguageError;

    fn try_from(value: Language) -> Result<Self, Self::Error> {
        let language = match value.as_ref() {
            "afr" => lingua::Language::Afrikaans,
            "ara" => lingua::Language::Arabic,
            "aze" => lingua::Language::Azerbaijani,
            "bel" => lingua::Language::Belarusian,
            "ben" => lingua::Language::Bengali,
            "bos" => lingua::Language::Bosnian,
            "bul" => lingua::Language::Bulgarian,
            "cat" => lingua::Language::Catalan,
            "ces" => lingua::Language::Czech,
            "cym" => lingua::Language::Welsh,
            "dan" => lingua::Language::Danish,
            "deu" => lingua::Language::German,
            "ell" => lingua::Language::Greek,
            "eng" => lingua::Language::English,
            "epo" => lingua::Language::Esperanto,
            "est" => lingua::Language::Estonian,
            "eus" => lingua::Language::Basque,
            "fas" => lingua::Language::Persian,
            "fin" => lingua::Language::Finnish,
            "fra" => lingua::Language::French,
            "gle" => lingua::Language::Irish,
            "guj" => lingua::Language::Gujarati,
            "heb" => lingua::Language::Hebrew,
            "hin" => lingua::Language::Hindi,
            "hrv" => lingua::Language::Croatian,
            "hun" => lingua::Language::Hungarian,
            "hye" => lingua::Language::Armenian,
            "ind" => lingua::Language::Indonesian,
            "isl" => lingua::Language::Icelandic,
            "ita" => lingua::Language::Italian,
            "jpn" => lingua::Language::Japanese,
            "kat" => lingua::Language::Georgian,
            "kaz" => lingua::Language::Kazakh,
            "kor" => lingua::Language::Korean,
            "lat" => lingua::Language::Latin,
            "lav" => lingua::Language::Latvian,
            "lit" => lingua::Language::Lithuanian,
            "lug" => lingua::Language::Ganda,
            "mar" => lingua::Language::Marathi,
            "mkd" => lingua::Language::Macedonian,
            "mon" => lingua::Language::Mongolian,
            "mri" => lingua::Language::Maori,
            "msa" => lingua::Language::Malay,
            "nld" => lingua::Language::Dutch,
            "nno" => lingua::Language::Nynorsk,
            "nob" => lingua::Language::Bokmal,
            "pan" => lingua::Language::Punjabi,
            "pol" => lingua::Language::Polish,
            "por" => lingua::Language::Portuguese,
            "ron" => lingua::Language::Romanian,
            "rus" => lingua::Language::Russian,
            "slk" => lingua::Language::Slovak,
            "slv" => lingua::Language::Slovene,
            "sna" => lingua::Language::Shona,
            "som" => lingua::Language::Somali,
            "sot" => lingua::Language::Sotho,
            "spa" => lingua::Language::Spanish,
            "sqi" => lingua::Language::Albanian,
            "srp" => lingua::Language::Serbian,
            "swa" => lingua::Language::Swahili,
            "swe" => lingua::Language::Swedish,
            "tam" => lingua::Language::Tamil,
            "tel" => lingua::Language::Telugu,
            "tgl" => lingua::Language::Tagalog,
            "tha" => lingua::Language::Thai,
            "tsn" => lingua::Language::Tswana,
            "tso" => lingua::Language::Tsonga,
            "tur" => lingua::Language::Turkish,
            "ukr" => lingua::Language::Ukrainian,
            "urd" => lingua::Language::Urdu,
            "vie" => lingua::Language::Vietnamese,
            "xho" => lingua::Language::Xhosa,
            "yor" => lingua::Language::Yoruba,
            "zho" => lingua::Language::Chinese,
            "zul" => lingua::Language::Zulu,
            _ => return Err(LanguageError::Unsupported(value.0)),
        };

        Ok(language)
    }
}

impl From<lingua::Language> for Language {
    fn from(value: lingua::Language) -> Self {
        Language(value.iso_code_639_3().to_string())
    }
}

pub struct SelectLanguageRequest {
    pub text: String,
    pub languages: Vec<Language>,
}

impl SelectLanguageRequest {
    pub fn new(text: String, languages: Vec<Language>) -> Self {
        Self { text, languages }
    }
}

pub fn select_language(
    request: SelectLanguageRequest,
    tracing_context: TracingContext,
) -> Result<Option<Language>, LanguageError> {
    let languages = request
        .languages
        .into_iter()
        .map(TryInto::try_into)
        .collect::<Result<Vec<_>, _>>()?;
    let detector = LanguageDetectorBuilder::from_languages(&languages)
        .with_minimum_relative_distance(0.5) // empirical value that makes the tests pass ;-)
        .build();
    let language = detector.detect_language_of(&request.text).map(Into::into);
    context_event!(context: tracing_context, level: Level::INFO, message="selected language {language:?}");
    Ok(language)
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn language_selected_for_english_text() {
        let text = "A little bit is better than nothing.".to_owned();
        let languages = vec![Language("eng".to_owned()), Language("deu".to_owned())];
        let request = SelectLanguageRequest { text, languages };
        let language = select_language(request, TracingContext::dummy()).unwrap();

        assert!(language.is_some());
        assert_eq!(language.unwrap(), Language("eng".to_owned()));
    }

    #[test]
    fn language_selected_for_german_text() {
        let text = "Ich spreche Deutsch nur ein bisschen.".to_owned();
        let languages = vec![Language("eng".to_owned()), Language("deu".to_owned())];
        let request = SelectLanguageRequest { text, languages };
        let language = select_language(request, TracingContext::dummy()).unwrap();

        assert!(language.is_some());
        assert_eq!(language.unwrap(), Language("deu".to_owned()));
    }

    #[test]
    fn no_language_selected_for_french_text() {
        let text = "Parlez-vous français?".to_owned();
        let languages = vec![Language("eng".to_owned()), Language("deu".to_owned())];
        let request = SelectLanguageRequest { text, languages };
        let language = select_language(request, TracingContext::dummy()).unwrap();
        assert!(language.is_none());
    }

    #[test]
    fn unsupported_language() {
        let text = "A little bit is better than nothing.".to_owned();
        let languages = vec![Language("foo".to_owned()), Language("deu".to_owned())];
        let request = SelectLanguageRequest { text, languages };
        let error = select_language(request, TracingContext::dummy()).unwrap_err();

        assert_eq!(error, LanguageError::Unsupported("foo".to_owned()));
    }
}
