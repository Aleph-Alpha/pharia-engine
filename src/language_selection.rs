use lingua::LanguageDetectorBuilder;
use serde::Serialize;
use tracing::trace;

/// ISO 639-3 labels
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    /// Afrikaans
    Afr,
    /// Arabic
    Ara,
    /// Azerbaijani
    Aze,
    /// Belarusian
    Bel,
    /// Bengali
    Ben,
    /// Bosnian
    Bos,
    /// Bulgarian
    Bul,
    /// Catalan
    Cat,
    /// Czech
    Ces,
    /// Welsh
    Cym,
    /// Danish
    Dan,
    /// German
    Deu,
    /// Greek
    Ell,
    /// English
    Eng,
    /// Esperanto
    Epo,
    /// Estonian
    Est,
    /// Basque
    Eus,
    /// Persian
    Fas,
    /// Finnish
    Fin,
    /// French
    Fra,
    /// Irish
    Gle,
    /// Gujarati
    Guj,
    /// Hebrew
    Heb,
    /// Hindi
    Hin,
    /// Croatian
    Hrv,
    /// Hungarian
    Hun,
    /// Armenian
    Hye,
    /// Indonesian
    Ind,
    /// Icelandic
    Isl,
    /// Italian
    Ita,
    /// Japanese
    Jpn,
    /// Georgian
    Kat,
    /// Kazakh
    Kaz,
    /// Korean
    Kor,
    /// Latin
    Lat,
    /// Latvian
    Lav,
    /// Lithuanian
    Lit,
    /// Ganda
    Lug,
    /// Marathi
    Mar,
    /// Macedonian
    Mkd,
    /// Mongolian
    Mon,
    /// Maori
    Mri,
    /// Malay
    Msa,
    /// Dutch
    Nld,
    /// Norwegian Nynorsk
    Nno,
    /// Norwegian Bokmål
    Nob,
    /// Punjabi
    Pan,
    /// Polish
    Pol,
    /// Portuguese
    Por,
    /// Romanian
    Ron,
    /// Russian
    Rus,
    /// Slovak
    Slk,
    /// Slovene
    Slv,
    /// Shona
    Sna,
    /// Somali
    Som,
    /// Sotho
    Sot,
    /// Spanish
    Spa,
    /// Albanian
    Sqi,
    /// Serbian
    Srp,
    /// Swahili
    Swa,
    /// Swedish
    Swe,
    /// Tamil
    Tam,
    /// Telugu
    Tel,
    /// Tagalog
    Tgl,
    /// Thai
    Tha,
    /// Tswana
    Tsn,
    /// Tsonga
    Tso,
    /// Turkish
    Tur,
    /// Ukrainian
    Ukr,
    /// Urdu
    Urd,
    /// Vietnamese
    Vie,
    /// Xhosa
    Xho,
    /// Yoruba
    Yor,
    /// Chinese
    Zho,
    /// Zulu
    Zul,
}

impl From<Language> for lingua::Language {
    fn from(value: Language) -> Self {
        match value {
            Language::Afr => lingua::Language::Afrikaans,
            Language::Ara => lingua::Language::Arabic,
            Language::Aze => lingua::Language::Azerbaijani,
            Language::Bel => lingua::Language::Belarusian,
            Language::Ben => lingua::Language::Bengali,
            Language::Bos => lingua::Language::Bosnian,
            Language::Bul => lingua::Language::Bulgarian,
            Language::Cat => lingua::Language::Catalan,
            Language::Ces => lingua::Language::Czech,
            Language::Cym => lingua::Language::Welsh,
            Language::Dan => lingua::Language::Danish,
            Language::Deu => lingua::Language::German,
            Language::Ell => lingua::Language::Greek,
            Language::Eng => lingua::Language::English,
            Language::Epo => lingua::Language::Esperanto,
            Language::Est => lingua::Language::Estonian,
            Language::Eus => lingua::Language::Basque,
            Language::Fas => lingua::Language::Persian,
            Language::Fin => lingua::Language::Finnish,
            Language::Fra => lingua::Language::French,
            Language::Gle => lingua::Language::Irish,
            Language::Guj => lingua::Language::Gujarati,
            Language::Heb => lingua::Language::Hebrew,
            Language::Hin => lingua::Language::Hindi,
            Language::Hrv => lingua::Language::Croatian,
            Language::Hun => lingua::Language::Hungarian,
            Language::Hye => lingua::Language::Armenian,
            Language::Ind => lingua::Language::Indonesian,
            Language::Isl => lingua::Language::Icelandic,
            Language::Ita => lingua::Language::Italian,
            Language::Jpn => lingua::Language::Japanese,
            Language::Kat => lingua::Language::Georgian,
            Language::Kaz => lingua::Language::Kazakh,
            Language::Kor => lingua::Language::Korean,
            Language::Lat => lingua::Language::Latin,
            Language::Lav => lingua::Language::Latvian,
            Language::Lit => lingua::Language::Lithuanian,
            Language::Lug => lingua::Language::Ganda,
            Language::Mar => lingua::Language::Marathi,
            Language::Mkd => lingua::Language::Macedonian,
            Language::Mon => lingua::Language::Mongolian,
            Language::Mri => lingua::Language::Maori,
            Language::Msa => lingua::Language::Malay,
            Language::Nld => lingua::Language::Dutch,
            Language::Nno => lingua::Language::Nynorsk,
            Language::Nob => lingua::Language::Bokmal,
            Language::Pan => lingua::Language::Punjabi,
            Language::Pol => lingua::Language::Polish,
            Language::Por => lingua::Language::Portuguese,
            Language::Ron => lingua::Language::Romanian,
            Language::Rus => lingua::Language::Russian,
            Language::Slk => lingua::Language::Slovak,
            Language::Slv => lingua::Language::Slovene,
            Language::Sna => lingua::Language::Shona,
            Language::Som => lingua::Language::Somali,
            Language::Sot => lingua::Language::Sotho,
            Language::Spa => lingua::Language::Spanish,
            Language::Sqi => lingua::Language::Albanian,
            Language::Srp => lingua::Language::Serbian,
            Language::Swa => lingua::Language::Swahili,
            Language::Swe => lingua::Language::Swedish,
            Language::Tam => lingua::Language::Tamil,
            Language::Tel => lingua::Language::Telugu,
            Language::Tgl => lingua::Language::Tagalog,
            Language::Tha => lingua::Language::Thai,
            Language::Tsn => lingua::Language::Tswana,
            Language::Tso => lingua::Language::Tsonga,
            Language::Tur => lingua::Language::Turkish,
            Language::Ukr => lingua::Language::Ukrainian,
            Language::Urd => lingua::Language::Urdu,
            Language::Vie => lingua::Language::Vietnamese,
            Language::Xho => lingua::Language::Xhosa,
            Language::Yor => lingua::Language::Yoruba,
            Language::Zho => lingua::Language::Chinese,
            Language::Zul => lingua::Language::Zulu,
        }
    }
}

impl From<lingua::Language> for Language {
    fn from(value: lingua::Language) -> Self {
        match value {
            lingua::Language::Afrikaans => Language::Afr,
            lingua::Language::Albanian => Language::Sqi,
            lingua::Language::Arabic => Language::Ara,
            lingua::Language::Armenian => Language::Hye,
            lingua::Language::Azerbaijani => Language::Aze,
            lingua::Language::Basque => Language::Eus,
            lingua::Language::Belarusian => Language::Bel,
            lingua::Language::Bengali => Language::Ben,
            lingua::Language::Bokmal => Language::Nob,
            lingua::Language::Bosnian => Language::Bos,
            lingua::Language::Bulgarian => Language::Bul,
            lingua::Language::Catalan => Language::Cat,
            lingua::Language::Chinese => Language::Zho,
            lingua::Language::Croatian => Language::Hrv,
            lingua::Language::Czech => Language::Ces,
            lingua::Language::Danish => Language::Dan,
            lingua::Language::Dutch => Language::Nld,
            lingua::Language::English => Language::Eng,
            lingua::Language::Esperanto => Language::Epo,
            lingua::Language::Estonian => Language::Est,
            lingua::Language::Finnish => Language::Fin,
            lingua::Language::French => Language::Fra,
            lingua::Language::Ganda => Language::Lug,
            lingua::Language::Georgian => Language::Kat,
            lingua::Language::German => Language::Deu,
            lingua::Language::Greek => Language::Ell,
            lingua::Language::Gujarati => Language::Guj,
            lingua::Language::Hebrew => Language::Heb,
            lingua::Language::Hindi => Language::Hin,
            lingua::Language::Hungarian => Language::Hun,
            lingua::Language::Icelandic => Language::Isl,
            lingua::Language::Indonesian => Language::Ind,
            lingua::Language::Irish => Language::Gle,
            lingua::Language::Italian => Language::Ita,
            lingua::Language::Japanese => Language::Jpn,
            lingua::Language::Kazakh => Language::Kaz,
            lingua::Language::Korean => Language::Kor,
            lingua::Language::Latin => Language::Lat,
            lingua::Language::Latvian => Language::Lav,
            lingua::Language::Lithuanian => Language::Lit,
            lingua::Language::Macedonian => Language::Mkd,
            lingua::Language::Malay => Language::Msa,
            lingua::Language::Maori => Language::Mri,
            lingua::Language::Marathi => Language::Mar,
            lingua::Language::Mongolian => Language::Mon,
            lingua::Language::Nynorsk => Language::Nno,
            lingua::Language::Persian => Language::Fas,
            lingua::Language::Polish => Language::Pol,
            lingua::Language::Portuguese => Language::Por,
            lingua::Language::Punjabi => Language::Pan,
            lingua::Language::Romanian => Language::Ron,
            lingua::Language::Russian => Language::Rus,
            lingua::Language::Serbian => Language::Srp,
            lingua::Language::Shona => Language::Sna,
            lingua::Language::Slovak => Language::Slk,
            lingua::Language::Slovene => Language::Slv,
            lingua::Language::Somali => Language::Som,
            lingua::Language::Sotho => Language::Sot,
            lingua::Language::Spanish => Language::Spa,
            lingua::Language::Swahili => Language::Swa,
            lingua::Language::Swedish => Language::Swe,
            lingua::Language::Tagalog => Language::Tgl,
            lingua::Language::Tamil => Language::Tam,
            lingua::Language::Telugu => Language::Tel,
            lingua::Language::Thai => Language::Tha,
            lingua::Language::Tsonga => Language::Tso,
            lingua::Language::Tswana => Language::Tsn,
            lingua::Language::Turkish => Language::Tur,
            lingua::Language::Ukrainian => Language::Ukr,
            lingua::Language::Urdu => Language::Urd,
            lingua::Language::Vietnamese => Language::Vie,
            lingua::Language::Welsh => Language::Cym,
            lingua::Language::Xhosa => Language::Xho,
            lingua::Language::Yoruba => Language::Yor,
            lingua::Language::Zulu => Language::Zul,
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
