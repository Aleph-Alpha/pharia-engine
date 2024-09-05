use lingua::LanguageDetectorBuilder;

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

pub fn select_language(text: &str, languages: &[Language]) -> Option<Language> {
    let languages = languages.iter().map(|&l| l.into()).collect::<Vec<_>>();
    let detector = LanguageDetectorBuilder::from_languages(&languages)
        .with_minimum_relative_distance(0.5) // empirical value that makes the tests pass ;-)
        .build();
    detector.detect_language_of(text).map(Into::into)
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn language_selected_for_english_text() {
        let text = "A little bit is better than nothing.";

        let language = select_language(text, &[Language::Eng, Language::Deu]);

        assert!(language.is_some());
        assert_eq!(language.unwrap(), Language::Eng);
    }

    #[test]
    fn language_selected_for_german_text() {
        let text = "Ich spreche Deutsch nur ein bisschen.";
        let language = select_language(text, &[Language::Eng, Language::Deu]);

        assert!(language.is_some());
        assert_eq!(language.unwrap(), Language::Deu);
    }

    #[test]
    fn no_language_selected_for_french_text() {
        let text = "Parlez-vous français?";
        let language = select_language(text, &[Language::Eng, Language::Deu]);
        assert!(language.is_none());
    }
}
