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
    #[serde(rename = "zh-Hant")]
    ChineseTraditional,
    #[serde(rename = "tr")]
    Turkish,
}

impl Locale {
    /// Every locale shown in the language picker, in a stable order.
    pub const ALL: [Locale; 10] = [
        Self::English,
        Self::PortugueseBrazil,
        Self::German,
        Self::Spanish,
        Self::Italian,
        Self::French,
        Self::Russian,
        Self::ChineseSimplified,
        Self::ChineseTraditional,
        Self::Turkish,
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
            Self::ChineseTraditional => "繁體中文",
            Self::Turkish => "Türkçe",
        }
    }

    /// Maps a system language tag (BCP 47 or POSIX) to a supported locale by
    /// its language subtag, so `pt-PT` and `pt_BR` both resolve to Portuguese.
    /// Chinese is the exception: its script decides, so a reader who asked for
    /// Traditional does not get Simplified characters and the other way round.
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
            "tr" => Self::Turkish,
            "zh" => match (tag.script.as_deref(), tag.region.as_deref()) {
                // An explicit script decides on its own: a tag that asks for
                // Simplified and names a region where Traditional is the
                // default, such as `zh-Hans-HK`, is still Simplified. The
                // region only speaks for the tags that name no script.
                (Some("hans"), _) => Self::ChineseSimplified,
                (Some("hant"), _) => Self::ChineseTraditional,
                // Windows writes the legacy `zh-CHT` region, macOS and Linux
                // the region subtag.
                (_, Some("tw" | "hk" | "mo" | "cht")) => Self::ChineseTraditional,
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
            Self::ChineseTraditional => Some(&zh_hant::Translator),
            Self::Turkish => Some(&tr::Translator),
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
        // A script the tag names itself decides, even beside a region whose own
        // default is the other one.
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
        assert_eq!(
            Locale::from_system("zh-Hant-CN"),
            Some(Locale::ChineseTraditional)
        );
        assert_eq!(
            Locale::from_system("zh-TW"),
            Some(Locale::ChineseTraditional)
        );
        assert_eq!(
            Locale::from_system("zh-Hant"),
            Some(Locale::ChineseTraditional)
        );
        assert_eq!(
            Locale::from_system("zh-HK"),
            Some(Locale::ChineseTraditional)
        );
        assert_eq!(
            Locale::from_system("zh-MO"),
            Some(Locale::ChineseTraditional)
        );
        assert_eq!(
            Locale::from_system("zh-CHT"),
            Some(Locale::ChineseTraditional)
        );
        // The POSIX form macOS and Linux report for a Traditional system.
        assert_eq!(
            Locale::from_system("zh_TW.UTF-8"),
            Some(Locale::ChineseTraditional)
        );
        assert_eq!(Locale::from_system("tr-TR"), Some(Locale::Turkish));
        assert_eq!(Locale::from_system("tr"), Some(Locale::Turkish));
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
        assert_eq!(Locale::from_system("tr_TR.UTF-8"), Some(Locale::Turkish));
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
    fn message_selection_labels_use_the_portuguese_catalog() {
        assert_eq!(
            gettext(Locale::PortugueseBrazil, "Select messages"),
            "Selecionar mensagens"
        );
        assert_eq!(
            gettext(
                Locale::PortugueseBrazil,
                "Select message from {sender}, {time}: {summary}"
            ),
            "Selecionar mensagem de {sender} ({time}): {summary}"
        );
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
        assert_eq!(
            pgettext(Locale::ChineseSimplified, "profile", "About"),
            "个人简介"
        );
        assert_eq!(gettext(Locale::ChineseSimplified, "Groups"), "群组");
        assert_eq!(pgettext(Locale::ChineseSimplified, "sound", "None"), "无");
    }

    #[test]
    fn chinese_traditional_catalog_translates() {
        assert_eq!(gettext(Locale::ChineseTraditional, "Chats"), "對話");
        assert_eq!(gettext(Locale::ChineseTraditional, "Search"), "搜尋");
        assert_eq!(gettext(Locale::ChineseTraditional, "Settings"), "設定");
        assert_eq!(
            gettext(Locale::ChineseTraditional, "Type a message"),
            "輸入訊息"
        );
        assert_eq!(gettext(Locale::ChineseTraditional, "Monday"), "週一");
        assert_eq!(gettext(Locale::ChineseTraditional, "Yesterday"), "昨天");
    }

    /// Chinese has a single plural form, so one text covers every count.
    #[test]
    fn traditional_chinese_plural_rules_use_one_form() {
        for count in [0, 1, 2, 21] {
            assert_eq!(
                ngettext(Locale::ChineseTraditional, "{} member", "{} members", count),
                "{} 位成員"
            );
        }
    }

    /// The same English word with two meanings stays two different strings.
    #[test]
    fn traditional_chinese_contexts_stay_separate_from_plain_lookups() {
        assert_eq!(gettext(Locale::ChineseTraditional, "About"), "關於");
        assert_eq!(
            pgettext(Locale::ChineseTraditional, "privacy", "About"),
            "個人簡介"
        );
        assert_eq!(gettext(Locale::ChineseTraditional, "Groups"), "群組");
        assert_eq!(pgettext(Locale::ChineseTraditional, "sound", "None"), "無");
        // The wallpaper section's "Theme" is a colour choice, not the
        // interface theme.
        assert_eq!(gettext(Locale::ChineseTraditional, "Theme"), "主題");
        assert_eq!(
            pgettext(Locale::ChineseTraditional, "wallpaper colour", "Theme"),
            "桌布主題"
        );
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

    #[test]
    fn turkish_catalog_translates() {
        assert_eq!(gettext(Locale::Turkish, "Chats"), "Sohbetler");
        assert_eq!(gettext(Locale::Turkish, "Search"), "Ara");
        assert_eq!(gettext(Locale::Turkish, "Settings"), "Ayarlar");
        assert_eq!(
            gettext(Locale::Turkish, "Type a message"),
            "Bir mesaj yazın"
        );
        assert_eq!(gettext(Locale::Turkish, "Monday"), "Pazartesi");
        assert_eq!(gettext(Locale::Turkish, "Yesterday"), "Dün");
    }

    #[test]
    fn turkish_plural_rules_cover_singular_and_plural() {
        for (count, expected) in [(1, "{} üye"), (2, "{} üye"), (5, "{} üye")] {
            assert_eq!(
                ngettext(Locale::Turkish, "{} member", "{} members", count),
                expected
            );
        }
    }

    #[test]
    fn turkish_contexts_stay_separate_from_plain_lookups() {
        assert_eq!(gettext(Locale::Turkish, "About"), "Hakkımda");
        assert_eq!(pgettext(Locale::Turkish, "privacy", "About"), "Hakkımda");
        assert_eq!(gettext(Locale::Turkish, "Groups"), "Gruplar");
        assert_eq!(pgettext(Locale::Turkish, "sound", "None"), "Yok");
        assert_eq!(gettext(Locale::Turkish, "None"), "None");
    }
}
