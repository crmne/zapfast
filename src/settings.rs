//! User preferences stored in JSON.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    #[default]
    Dark,
    Light,
    System,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::System => "Follow system",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeChoice,
    /// Filename of the selected local JSON palette.
    pub custom_theme: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::theme::custom::read_cached_theme",
        skip_serializing_if = "Option::is_none"
    )]
    pub custom_theme_cache: Option<crate::theme::custom::CustomTheme>,
    #[serde(
        default,
        deserialize_with = "crate::theme::custom::read_cached_theme",
        skip_serializing_if = "Option::is_none"
    )]
    pub system_theme_cache: Option<crate::theme::custom::CustomTheme>,
    /// egui zoom factor.
    pub zoom: f32,
    pub sidebar_width: f32,
    /// Whether Enter sends and Shift+Enter adds a line. Off swaps them.
    pub enter_sends: bool,
    /// Send read receipts, subject to the account privacy setting.
    pub send_read_receipts: bool,
    /// Send typing state while composing.
    pub send_typing: bool,
    /// Download attachments when they enter view instead of on click.
    #[serde(alias = "auto_download_images")]
    pub auto_download: bool,
    /// Show sender avatars outside groups too.
    pub show_sender_pictures: bool,
    /// Last open chat, restored at startup.
    pub last_chat: Option<String>,
    pub show_shortcut_hints: bool,
    /// Recently used emoji, newest first.
    pub recent_emoji: Vec<String>,
    /// User GIPHY API key. Empty uses the optional built-in key.
    pub giphy_key: String,
    /// Keep the app linked in the tray when the window closes.
    pub keep_running_in_background: bool,
    /// Desktop notifications while away from the chat.
    pub notifications: bool,
    /// Ask GitHub once a day whether a newer release exists.
    pub check_for_updates: bool,
    /// Download verified updates in the background; restarting remains explicit.
    pub download_updates_automatically: bool,
    /// Prefer address-book names over public profile names.
    pub names_from_contacts: bool,
    /// Voice and audio playback speed multiplier.
    pub voice_speed: f32,
    /// Also add saved contacts to the phone's address book.
    pub save_contacts_to_phone: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::Dark,
            custom_theme: None,
            custom_theme_cache: None,
            system_theme_cache: None,
            zoom: 1.0,
            sidebar_width: 320.0,
            enter_sends: true,
            send_read_receipts: true,
            send_typing: true,
            auto_download: true,
            show_sender_pictures: false,
            last_chat: None,
            show_shortcut_hints: true,
            recent_emoji: Vec::new(),
            giphy_key: String::new(),
            keep_running_in_background: true,
            notifications: true,
            check_for_updates: true,
            download_updates_automatically: false,
            names_from_contacts: true,
            save_contacts_to_phone: true,
            voice_speed: 1.0,
        }
    }
}

/// Optional build-time GIPHY key from `ZAPFAST_GIPHY_KEY`.
/// The previous name remains accepted for existing build setups.
pub const BUILT_IN_GIPHY_KEY: Option<&str> = match option_env!("ZAPFAST_GIPHY_KEY") {
    Some(key) if !key.is_empty() => Some(key),
    _ => option_env!("FASTSAPP_GIPHY_KEY"),
};

impl Settings {
    pub(crate) fn cached_palette(&self) -> Option<crate::theme::Palette> {
        let theme = if self.custom_theme.is_some() {
            self.custom_theme_cache.as_ref()
        } else if self.theme == ThemeChoice::System {
            self.system_theme_cache.as_ref()
        } else {
            None
        };
        theme.map(|theme| theme.palette)
    }

    /// Returns the user key, built-in key, or `None`.
    pub fn effective_giphy_key(&self) -> Option<String> {
        let own = self.giphy_key.trim();
        if !own.is_empty() {
            return Some(own.to_owned());
        }
        BUILT_IN_GIPHY_KEY
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_owned)
    }

    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => match serde_json::from_str(&contents) {
                Ok(settings) => settings,
                Err(error) => {
                    log::warn!("settings file is unreadable, using defaults: {error}");
                    Self::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                log::warn!("could not read settings: {error}");
                Self::default()
            }
        }
    }

    /// Atomically replaces the settings file through a temporary file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, contents)?;
        std::fs::rename(&temp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_and_missing_fields_are_tolerated() {
        let parsed: Settings =
            serde_json::from_str(r#"{"theme":"light","future_field":1}"#).expect("parses");
        assert_eq!(parsed.theme, ThemeChoice::Light);
        assert!(parsed.enter_sends);
        assert!(parsed.check_for_updates);
        assert!(!parsed.download_updates_automatically);
    }

    #[test]
    fn damaged_theme_cache_does_not_discard_other_settings() {
        let settings: Settings = serde_json::from_str(r#"{"custom_theme":"mine.json","custom_theme_cache":{"damaged":true},"enter_sends":false}"#).unwrap();
        assert!(!settings.enter_sends);
        assert!(settings.custom_theme_cache.is_none());
        assert_eq!(settings.custom_theme.as_deref(), Some("mine.json"));
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("zapfast-settings-{}", std::process::id()));
        let path = dir.join("settings.json");
        let settings = Settings {
            zoom: 1.25,
            enter_sends: false,
            voice_speed: 1.5,
            ..Settings::default()
        };
        settings.save(&path).expect("saves");
        assert_eq!(Settings::load(&path), settings);
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod giphy_tests {
    use super::*;

    #[test]
    fn the_users_key_wins_and_is_trimmed() {
        let settings = Settings {
            giphy_key: "  abc  ".into(),
            ..Settings::default()
        };
        assert_eq!(settings.effective_giphy_key().as_deref(), Some("abc"));
    }

    #[test]
    fn without_a_key_of_their_own_the_built_in_one_is_used() {
        let settings = Settings {
            giphy_key: "   ".into(),
            ..Settings::default()
        };
        let expected = BUILT_IN_GIPHY_KEY
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_owned);
        assert_eq!(settings.effective_giphy_key(), expected);
    }
}
