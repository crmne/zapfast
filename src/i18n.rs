//! Bundled gettext localization.
//!
//! Catalogs are compiled from `assets/i18n/*.po` into Rust modules at build
//! time, so there is no libintl, no runtime PO parsing, and no network access.
//! English is both the source language and the fallback for any untranslated
//! message. Only languages with a catalog are offered.

pub use fastframe_i18n::{gettext, ngettext, pgettext};
use serde::{Deserialize, Serialize};

include!(concat!(env!("OUT_DIR"), "/catalogs.rs"));

/// The interface languages ZapFast knows about.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Locale {
    #[default]
    #[serde(rename = "en")]
    English,
    #[serde(rename = "pt-BR")]
    PortugueseBrazil,
    #[serde(rename = "de")]
    German,
    #[serde(rename = "es")]
    Spanish,
    #[serde(rename = "it")]
    Italian,
    #[serde(rename = "fr")]
    French,
    #[serde(rename = "ru")]
    Russian,
    #[serde(rename = "zh-Hans")]
    ChineseSimplified,
}

impl Locale {
    /// Every locale shown in the language picker, in a stable order.
    pub const ALL: [Locale; 8] = [
        Self::English,
        Self::PortugueseBrazil,
        Self::German,
        Self::Spanish,
        Self::Italian,
        Self::French,
        Self::Russian,
        Self::ChineseSimplified,
    ];

    /// The language's own name, for the picker.
    pub fn label(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::PortugueseBrazil => "Português (Brasil)",
            Self::German => "Deutsch",
            Self::Spanish => "Español",
            Self::Italian => "Italiano",
            Self::French => "Français",
            Self::Russian => "Русский",
            Self::ChineseSimplified => "简体中文",
        }
    }

    /// Maps a system language tag (BCP 47 or POSIX) to a supported locale by
    /// its language subtag, so `pt-PT` and `pt_BR` both resolve to Portuguese.
    /// Chinese is the exception: its script decides, so a reader who asked for
    /// Traditional keeps English instead of getting the wrong characters.
    pub fn from_system(identifier: &str) -> Option<Locale> {
        fastframe_i18n::LanguageTag::parse(identifier).and_then(|tag| Self::from_tag(&tag))
    }

    fn from_tag(tag: &fastframe_i18n::LanguageTag) -> Option<Locale> {
        Some(match tag.language.as_str() {
            "en" => Self::English,
            "pt" => Self::PortugueseBrazil,
            "de" => Self::German,
            "es" => Self::Spanish,
            "it" => Self::Italian,
            "fr" => Self::French,
            "ru" => Self::Russian,
            "zh" => match (tag.script.as_deref(), tag.region.as_deref()) {
                // An explicit script decides on its own: a tag that asks for
                // Simplified and names a region where Traditional is the
                // default, such as `zh-Hans-HK`, is still Simplified. The
                // region only speaks for the tags that name no script.
                (Some("hans"), _) => Self::ChineseSimplified,
                (Some("hant"), _) => return None,
                // Windows writes the legacy `zh-CHT` region, macOS and Linux
                // the region subtag.
                (_, Some("tw" | "hk" | "mo" | "cht")) => return None,
                _ => Self::ChineseSimplified,
            },
            _ => return None,
        })
    }
}

impl fastframe_i18n::Locale for Locale {
    fn catalog(self) -> Option<&'static dyn fastframe_i18n::Translator> {
        match self {
            Self::PortugueseBrazil => Some(&pt_br::Translator),
            Self::German => Some(&de::Translator),
            Self::Spanish => Some(&es::Translator),
            Self::French => Some(&fr::Translator),
            Self::Italian => Some(&it::Translator),
            Self::Russian => Some(&ru::Translator),
            Self::ChineseSimplified => Some(&zh_hans::Translator),
            Self::English => None,
        }
    }
}

/// The first supported language the operating system prefers, falling back
/// to English.
///
/// Unit tests assert the English source strings, so the machine the suite runs
/// on must not decide their outcome: a developer with a Portuguese Brazil
/// system would otherwise see failures that the continuous integration, running
/// in English, does not.
pub fn detect() -> Locale {
    if cfg!(test) {
        return Locale::English;
    }
    fastframe_i18n::detect(Locale::from_tag).unwrap_or_default()
}

/// Resolves a stored preference: an explicit choice wins, otherwise detect.
pub fn resolve(interface_language: Option<Locale>) -> Locale {
    interface_language.unwrap_or_else(detect)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_locales_map_to_the_right_catalog() {
        assert_eq!(Locale::from_system("pt-BR"), Some(Locale::PortugueseBrazil));
        assert_eq!(Locale::from_system("pt_PT"), Some(Locale::PortugueseBrazil));
        assert_eq!(Locale::from_system("de-DE"), Some(Locale::German));
        assert_eq!(Locale::from_system("es"), Some(Locale::Spanish));
        assert_eq!(Locale::from_system("it-IT"), Some(Locale::Italian));
        assert_eq!(Locale::from_system("fr-FR"), Some(Locale::French));
        assert_eq!(Locale::from_system("ru-RU"), Some(Locale::Russian));
        assert_eq!(
            Locale::from_system("zh-Hans"),
            Some(Locale::ChineseSimplified)
        );
        assert_eq!(Locale::from_system("zh"), Some(Locale::ChineseSimplified));
        assert_eq!(
            Locale::from_system("zh_CN.GB2312"),
            Some(Locale::ChineseSimplified)
        );
        // Traditional Chinese has no catalog, so it keeps English rather than
        // showing Simplified characters to a reader who asked for another script.
        // A script the tag names itself decides, even beside a region whose
        // own default is the other one.
        assert_eq!(
            Locale::from_system("zh-Hans-TW"),
            Some(Locale::ChineseSimplified)
        );
        assert_eq!(
            Locale::from_system("zh-Hans-HK"),
            Some(Locale::ChineseSimplified)
        );
        assert_eq!(
            Locale::from_system("zh-Hans-CN"),
            Some(Locale::ChineseSimplified)
        );
        assert_eq!(Locale::from_system("zh-Hant-CN"), None);
        assert_eq!(Locale::from_system("zh-TW"), None);
        assert_eq!(Locale::from_system("zh-Hant"), None);
        assert_eq!(Locale::from_system("zh-CHT"), None);
        assert_eq!(Locale::from_system("en-US"), Some(Locale::English));
        assert_eq!(Locale::from_system("ja-JP"), None);
        assert_eq!(Locale::default(), Locale::English);
    }

    /// Every preferred language is tried in order, not only the first: a
    /// desktop that lists an unsupported language first still gets the next.
    #[test]
    fn the_first_supported_preferred_language_wins() {
        assert_eq!(
            fastframe_i18n::first_supported(["nb-NO", "de-DE", "fr-FR"], Locale::from_tag),
            Some(Locale::German)
        );
        assert_eq!(
            Locale::from_system("pt_BR.UTF-8"),
            Some(Locale::PortugueseBrazil)
        );
    }

    /// The suite asserts the English source strings, so the language of the
    /// machine that runs it must not decide whether it passes, while an
    /// explicit choice still wins over the detection.
    #[test]
    fn detection_is_english_under_test_and_a_choice_still_wins() {
        assert_eq!(detect(), Locale::English);
        assert_eq!(
            resolve(Some(Locale::PortugueseBrazil)),
            Locale::PortugueseBrazil
        );
        assert_eq!(resolve(None), Locale::English);
    }

    #[test]
    fn unknown_keys_fall_back_to_the_english_source() {
        assert_eq!(gettext(Locale::PortugueseBrazil, "Chats"), "Conversas");
        assert_eq!(gettext(Locale::English, "Chats"), "Chats");
        let missing = "A string nobody has translated";
        assert_eq!(gettext(Locale::PortugueseBrazil, missing), missing);
        assert_eq!(gettext(Locale::German, missing), missing);
    }

    #[test]
    fn german_catalog_translates_the_pilot() {
        assert_eq!(gettext(Locale::German, "Chats"), "Chats");
        assert_eq!(gettext(Locale::German, "Search"), "Suchen");
        assert_eq!(gettext(Locale::German, "Unread"), "Ungelesen");
        assert_eq!(
            gettext(Locale::German, "Type a message"),
            "Nachricht eingeben"
        );
        assert_eq!(gettext(Locale::German, "Settings"), "Einstellungen");
        assert_eq!(gettext(Locale::German, "Monday"), "Montag");
    }

    #[test]
    fn german_plural_rules_cover_singular_and_plural() {
        assert_eq!(
            ngettext(Locale::German, "{} member", "{} members", 1),
            "{} Mitglied"
        );
        assert_eq!(
            ngettext(Locale::German, "{} member", "{} members", 2),
            "{} Mitglieder"
        );
        // German treats zero as plural, unlike English.
        assert_eq!(
            ngettext(Locale::German, "{} member", "{} members", 0),
            "{} Mitglieder"
        );
    }

    #[test]
    fn remaining_pilot_catalogs_translate() {
        assert_eq!(gettext(Locale::Spanish, "Search"), "Buscar");
        assert_eq!(gettext(Locale::French, "Search"), "Rechercher");
        assert_eq!(gettext(Locale::Italian, "Search"), "Cerca");
        assert_eq!(gettext(Locale::Russian, "Search"), "Поиск");
        assert_eq!(gettext(Locale::Spanish, "Unread"), "No leídos");
        assert_eq!(gettext(Locale::French, "Settings"), "Paramètres");
        assert_eq!(
            gettext(Locale::Italian, "Type a message"),
            "Scrivi un messaggio"
        );
        assert_eq!(gettext(Locale::Russian, "Today"), "Сегодня");
    }

    #[test]
    fn russian_plural_rules_select_three_forms() {
        assert_eq!(
            ngettext(Locale::Russian, "{} member", "{} members", 1),
            "{} участник"
        );
        assert_eq!(
            ngettext(Locale::Russian, "{} member", "{} members", 3),
            "{} участника"
        );
        assert_eq!(
            ngettext(Locale::Russian, "{} member", "{} members", 5),
            "{} участников"
        );
        assert_eq!(
            ngettext(Locale::Russian, "{} member", "{} members", 21),
            "{} участник"
        );
    }

    #[test]
    fn contextual_lookups_stay_separate_from_plain_ones() {
        // The pilot catalog has no msgctxt entries, so a contextual lookup is
        // distinct from gettext but still falls back to the source.
        assert_eq!(pgettext(Locale::PortugueseBrazil, "verb", "Chats"), "Chats");
        assert_eq!(pgettext(Locale::English, "verb", "Chats"), "Chats");
        assert_eq!(gettext(Locale::PortugueseBrazil, "Chats"), "Conversas");
    }

    #[test]
    fn chinese_simplified_catalog_translates() {
        assert_eq!(gettext(Locale::ChineseSimplified, "Chats"), "聊天");
        assert_eq!(gettext(Locale::ChineseSimplified, "Search"), "搜索");
        assert_eq!(gettext(Locale::ChineseSimplified, "Settings"), "设置");
        assert_eq!(
            gettext(Locale::ChineseSimplified, "Type a message"),
            "输入消息"
        );
        assert_eq!(gettext(Locale::ChineseSimplified, "Monday"), "周一");
        assert_eq!(gettext(Locale::ChineseSimplified, "Yesterday"), "昨天");
    }

    /// Chinese has a single plural form, so one text covers every count.
    #[test]
    fn chinese_plural_rules_use_one_form() {
        for count in [0, 1, 2, 21] {
            assert_eq!(
                ngettext(Locale::ChineseSimplified, "{} member", "{} members", count),
                "{} 位成员"
            );
        }
    }

    /// The same English word with two meanings stays two different strings.
    #[test]
    fn chinese_contexts_stay_separate_from_plain_lookups() {
        assert_eq!(gettext(Locale::ChineseSimplified, "About"), "关于");
        assert_eq!(
            pgettext(Locale::ChineseSimplified, "privacy", "About"),
            "个人简介"
        );
        assert_eq!(gettext(Locale::ChineseSimplified, "Groups"), "群组");
        assert_eq!(pgettext(Locale::ChineseSimplified, "sound", "None"), "无");
    }

    #[test]
    fn portuguese_plural_rules_cover_singular_and_plural() {
        for (count, expected) in [(1, "{} membro"), (2, "{} membros"), (5, "{} membros")] {
            assert_eq!(
                ngettext(Locale::PortugueseBrazil, "{} member", "{} members", count),
                expected
            );
        }
        // Without a catalog the bare English count rule applies.
        assert_eq!(
            ngettext(Locale::English, "{} member", "{} members", 1),
            "{} member"
        );
        assert_eq!(
            ngettext(Locale::English, "{} member", "{} members", 2),
            "{} members"
        );
    }
}
