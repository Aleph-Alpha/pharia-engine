use lingua::LanguageDetectorBuilder;
use tracing::trace;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    /// english
    Eng,
    /// german
    Deu,
}

impl From<Language> for lingua::Language {
    fn from(value: Language) -> Self {
        match value {
            Language::Eng => lingua::Language::English,
            Language::Deu => lingua::Language::German,
        }
    }
}

impl From<lingua::Language> for Language {
    fn from(value: lingua::Language) -> Self {
        match value {
            lingua::Language::English => Language::Eng,
            lingua::Language::German => Language::Deu,
        }
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

pub fn select_language(request: SelectLanguageRequest) -> Option<Language> {
    let languages = request
        .languages
        .iter()
        .map(|&l| l.into())
        .collect::<Vec<_>>();
    let detector = LanguageDetectorBuilder::from_languages(&languages)
        .with_minimum_relative_distance(0.5) // empirical value that makes the tests pass ;-)
        .build();
    let language = detector.detect_language_of(&request.text).map(Into::into);
    trace!(
        "select_language: text.len()={} languages={languages:?} selected_language={language:?}",
        request.text.len()
    );
    language
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn language_selected_for_english_text() {
        let text = "A little bit is better than nothing.".to_owned();
        let languages = vec![Language::Eng, Language::Deu];
        let request = SelectLanguageRequest { text, languages };
        let language = select_language(request);

        assert!(language.is_some());
        assert_eq!(language.unwrap(), Language::Eng);
    }

    #[test]
    fn language_selected_for_german_text() {
        let text = "Ich spreche Deutsch nur ein bisschen.".to_owned();
        let languages = vec![Language::Eng, Language::Deu];
        let request = SelectLanguageRequest { text, languages };
        let language = select_language(request);

        assert!(language.is_some());
        assert_eq!(language.unwrap(), Language::Deu);
    }

    #[test]
    fn no_language_selected_for_french_text() {
        let text = "Parlez-vous français?".to_owned();
        let languages = vec![Language::Eng, Language::Deu];
        let request = SelectLanguageRequest { text, languages };
        let language = select_language(request);
        assert!(language.is_none());
    }
}
