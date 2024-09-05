use lingua::{LanguageDetector, LanguageDetectorBuilder};

pub struct LanguageSelector {
    detector: LanguageDetector,
}

impl LanguageSelector {
    pub fn new() -> Self {
        Self {
            detector: LanguageDetectorBuilder::from_all_languages().build(),
        }
    }

    fn select(&self, text: &str) -> Option<String> {
        self.detector
            .detect_language_of(text)
            .map(|l| l.iso_code_639_3().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::LanguageSelector;

    #[test]
    fn language_selected_for_english_text() {
        let text = "A little bit is better than nothing.";
        let selector = LanguageSelector::new();

        let language = selector.select(text);

        assert!(language.is_some());
        assert_eq!(language.unwrap(), "eng");
    }

    #[test]
    fn language_selected_for_german_text() {
        let text = "Ich spreche Deutsch nur ein bisschen.";
        let selector = LanguageSelector::new();

        let language = selector.select(text);

        assert!(language.is_some());
        assert_eq!(language.unwrap(), "deu");
    }

    #[test]
    fn no_language_selected_for_french_text() {
        let text = "Parlez-vous français?";
        let selector = LanguageSelector::new();

        let language = selector.select(text);

        assert!(language.is_none());
    }
}
