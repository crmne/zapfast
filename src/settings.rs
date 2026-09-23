//! User preferences stored in JSON.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Verifying the locked-chat code costs about 20 ms, paid once per distinct
/// typed string. ponytail: fixed cost, revisit if it lags the search field.
const CHAT_LOCK_ROUNDS: std::num::NonZeroU32 = std::num::NonZeroU32::new(200_000).unwrap();

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(value: &str) -> Option<Vec<u8>> {
    value
        .len()
        .is_multiple_of(2)
        .then(|| {
            (0..value.len())
                .step_by(2)
                .map(|at| u8::from_str_radix(&value[at..at + 2], 16).ok())
                .collect()
        })
        .flatten()
}

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

/// Background colours offered by WhatsApp's wallpaper picker.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WallpaperColor {
    #[default]
    Beige,
    Cruise,
    Scandal,
    MonteCarlo,
    HawkesBlue,
    Downy,
    Seagull,
    Quartz,
    VeryLightGrey,
    Orinoco,
    Tusk,
    CapeHoney,
    Caramel,
    RoseBud,
    Bittersweet,
    RadicalRed,
    MandarinOrange,
    Flamingo,
    Buccaneer,
    BreakerBay,
    Pelorous,
    ToryBlue,
    Fiord,
    Cinder,
    Tolopea,
    Solitude,
    Canary,
    WillowBrook,
    Black,
    Nordic,
    CardinGreen,
    Tangaroa,
    Tiber,
    BlackRussian,
    Nero,
    Marshland,
    Maire,
    BlackMagic,
    CocoaBrown,
    WoodBark,
    SealBrown,
    SealBrownDarker,
    SealBrownLight,
    Cyprus,
    BlueWhale,
    BlackPearl,
    DarkTolopea,
    Woodsmoke,
    MaireTwo,
}

impl WallpaperColor {
    pub const LIGHT: [Self; 28] = [
        Self::Beige,
        Self::Cruise,
        Self::Scandal,
        Self::MonteCarlo,
        Self::HawkesBlue,
        Self::Downy,
        Self::Seagull,
        Self::Quartz,
        Self::VeryLightGrey,
        Self::Orinoco,
        Self::Tusk,
        Self::CapeHoney,
        Self::Caramel,
        Self::RoseBud,
        Self::Bittersweet,
        Self::RadicalRed,
        Self::MandarinOrange,
        Self::Flamingo,
        Self::Buccaneer,
        Self::BreakerBay,
        Self::Pelorous,
        Self::ToryBlue,
        Self::Fiord,
        Self::Cinder,
        Self::Tolopea,
        Self::Solitude,
        Self::Canary,
        Self::WillowBrook,
    ];

    pub const DARK: [Self; 21] = [
        Self::Black,
        Self::Nordic,
        Self::CardinGreen,
        Self::Tangaroa,
        Self::Tiber,
        Self::BlackRussian,
        Self::Nero,
        Self::Marshland,
        Self::Maire,
        Self::BlackMagic,
        Self::CocoaBrown,
        Self::WoodBark,
        Self::SealBrown,
        Self::SealBrownDarker,
        Self::SealBrownLight,
        Self::Cyprus,
        Self::BlueWhale,
        Self::BlackPearl,
        Self::DarkTolopea,
        Self::Woodsmoke,
        Self::MaireTwo,
    ];

    pub fn choices(dark: bool) -> &'static [Self] {
        if dark { &Self::DARK } else { &Self::LIGHT }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Beige => "Beige",
            Self::Cruise => "Cruise",
            Self::Scandal => "Scandal",
            Self::MonteCarlo => "Monte Carlo",
            Self::HawkesBlue => "Hawkes Blue",
            Self::Downy => "Downy",
            Self::Seagull => "Seagull",
            Self::Quartz => "Quartz",
            Self::VeryLightGrey => "Very light grey",
            Self::Orinoco => "Orinoco",
            Self::Tusk => "Tusk",
            Self::CapeHoney => "Cape honey",
            Self::Caramel => "Caramel",
            Self::RoseBud => "Rose bud",
            Self::Bittersweet => "Bittersweet",
            Self::RadicalRed => "Radical red",
            Self::MandarinOrange => "Mandarin Orange",
            Self::Flamingo => "Flamingo",
            Self::Buccaneer => "Buccaneer",
            Self::BreakerBay => "Breaker Bay",
            Self::Pelorous => "Pelorous",
            Self::ToryBlue => "Tory Blue",
            Self::Fiord => "Fiord",
            Self::Cinder => "Cinder",
            Self::Tolopea => "Tolopea",
            Self::Solitude => "Solitude",
            Self::Canary => "Canary",
            Self::WillowBrook => "Willow Brook",
            Self::Black => "Black",
            Self::Nordic => "Nordic",
            Self::CardinGreen => "Cardin Green",
            Self::Tangaroa => "Tangaroa",
            Self::Tiber => "Tiber",
            Self::BlackRussian => "Black Russian",
            Self::Nero => "Nero",
            Self::Marshland => "Marshland",
            Self::Maire => "Maire",
            Self::BlackMagic => "Black Magic",
            Self::CocoaBrown => "Cocoa Brown",
            Self::WoodBark => "Wood Bark",
            Self::SealBrown => "Seal Brown",
            Self::SealBrownDarker => "Seal Brown Darker",
            Self::SealBrownLight => "Seal Brown Light",
            Self::Cyprus => "Cyprus",
            Self::BlueWhale => "Blue Whale",
            Self::BlackPearl => "Black Pearl",
            Self::DarkTolopea => "Tolopea",
            Self::Woodsmoke => "Woodsmoke",
            Self::MaireTwo => "Maire 2",
        }
    }

    pub fn rgb(self) -> [u8; 3] {
        match self {
            Self::Beige => [245, 241, 235],
            Self::Cruise => [187, 228, 229],
            Self::Scandal => [174, 216, 199],
            Self::MonteCarlo => [122, 203, 165],
            Self::HawkesBlue => [203, 218, 236],
            Self::Downy => [102, 210, 213],
            Self::Seagull => [99, 189, 207],
            Self::Quartz => [214, 208, 240],
            Self::VeryLightGrey => [206, 206, 206],
            Self::Orinoco => [209, 218, 190],
            Self::Tusk => [230, 225, 177],
            Self::CapeHoney => [254, 239, 169],
            Self::Caramel => [254, 210, 151],
            Self::RoseBud => [253, 154, 155],
            Self::Bittersweet => [253, 103, 105],
            Self::RadicalRed => [251, 70, 104],
            Self::MandarinOrange => [146, 32, 64],
            Self::Flamingo => [220, 110, 79],
            Self::Buccaneer => [100, 77, 82],
            Self::BreakerBay => [81, 126, 126],
            Self::Pelorous => [49, 144, 187],
            Self::ToryBlue => [53, 85, 138],
            Self::Fiord => [85, 98, 111],
            Self::Cinder => [29, 35, 38],
            Self::Tolopea => [48, 30, 52],
            Self::Solitude => [236, 240, 241],
            Self::Canary => [255, 254, 162],
            Self::WillowBrook => [231, 232, 210],
            Self::Black => [22, 23, 23],
            Self::Nordic => [15, 36, 36],
            Self::CardinGreen => [18, 38, 31],
            Self::Tangaroa => [17, 30, 39],
            Self::Tiber => [14, 33, 37],
            Self::BlackRussian => [31, 29, 37],
            Self::Nero => [33, 33, 33],
            Self::Marshland => [31, 33, 28],
            Self::Maire => [35, 35, 27],
            Self::BlackMagic => [38, 36, 25],
            Self::CocoaBrown => [38, 31, 23],
            Self::WoodBark => [38, 23, 23],
            Self::SealBrown => [38, 15, 16],
            Self::SealBrownDarker => [25, 5, 11],
            Self::SealBrownLight => [33, 16, 12],
            Self::Cyprus => [10, 29, 37],
            Self::BlueWhale => [13, 21, 35],
            Self::BlackPearl => [13, 15, 17],
            Self::DarkTolopea => [17, 11, 18],
            Self::Woodsmoke => [30, 31, 31],
            Self::MaireTwo => [35, 35, 31],
        }
    }

    pub fn color32(self) -> egui::Color32 {
        egui::Color32::from_rgb(self.rgb()[0], self.rgb()[1], self.rgb()[2])
    }
}

/// The sound a new-message notification makes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSound {
    /// ZapFast's two-note chime, the default for one-to-one chats.
    #[default]
    Chime,
    /// ZapFast's three-note ripple, the default for groups.
    Ripple,
    /// Whatever the operating system plays for notifications.
    System,
    /// No sound.
    None,
    /// An audio file ZapFast plays itself.
    Custom(std::path::PathBuf),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeChoice,
    /// Interface language. `None` follows the operating system's locale.
    pub interface_language: Option<crate::i18n::Locale>,
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
    /// Hiding the chat list collapses it to avatars instead of removing it.
    pub collapse_chat_list: bool,
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
    /// Show the default doodle wallpaper behind conversations.
    pub show_wallpaper: bool,
    /// Colour selected in the wallpaper picker.
    pub wallpaper_color: WallpaperColor,
    /// Colour selected for the dark wallpaper picker.
    pub dark_wallpaper_color: WallpaperColor,
    /// Last open chat, restored at startup.
    pub last_chat: Option<String>,
    pub show_shortcut_hints: bool,
    /// Recently used emoji, newest first.
    pub recent_emoji: Vec<String>,
    /// Emoji reaction usage on this device, most-used first (recent breaks ties).
    pub reaction_emoji: Vec<(String, u32)>,
    /// User GIPHY API key. Empty uses the optional built-in key.
    pub giphy_key: String,
    /// Keep the app linked in the tray when the window closes.
    pub keep_running_in_background: bool,
    /// Desktop notifications while away from the chat.
    pub notifications: bool,
    /// Sound for notifications from one-to-one chats.
    pub message_sound: NotificationSound,
    /// Sound for notifications from groups.
    pub group_sound: NotificationSound,
    /// Folder for new downloads. `None` keeps them in the cache. Files
    /// already downloaded stay where they are when this changes.
    pub download_folder: Option<std::path::PathBuf>,
    /// Proxy for WhatsApp, media, and updates, such as
    /// `socks5h://127.0.0.1:9050`. Empty follows `ALL_PROXY` / `HTTPS_PROXY`.
    pub proxy: String,
    /// Ask GitHub once a day whether a newer release exists.
    pub check_for_updates: bool,
    /// Download verified updates in the background; restarting remains explicit.
    pub download_updates_automatically: bool,
    /// Prefer address-book names over public profile names.
    pub names_from_contacts: bool,
    /// Voice and audio playback speed multiplier.
    pub voice_speed: f32,
    /// Pause other apps' media while recording a voice message.
    pub pause_media_while_recording: bool,
    /// Pause other apps' media while a voice or audio message plays.
    pub pause_media_while_playing: bool,
    /// Also add saved contacts to the phone's address book.
    pub save_contacts_to_phone: bool,
    /// Legacy plaintext code, accepted once and rewritten as a verifier.
    #[serde(skip_serializing)]
    pub chat_lock_code: Option<String>,
    /// Salted PBKDF2 verifier for the local locked-chats code, `salt$hash`.
    pub chat_lock_code_hash: Option<String>,
    /// The one-time locked-chat code hint has been opened.
    pub chat_lock_hint_dismissed: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::Dark,
            interface_language: None,
            custom_theme: None,
            custom_theme_cache: None,
            system_theme_cache: None,
            zoom: 1.0,
            sidebar_width: 320.0,
            collapse_chat_list: false,
            enter_sends: true,
            send_read_receipts: true,
            send_typing: true,
            auto_download: true,
            show_sender_pictures: false,
            show_wallpaper: true,
            wallpaper_color: WallpaperColor::default(),
            dark_wallpaper_color: WallpaperColor::Black,
            last_chat: None,
            show_shortcut_hints: true,
            recent_emoji: Vec::new(),
            reaction_emoji: Vec::new(),
            giphy_key: String::new(),
            keep_running_in_background: true,
            notifications: true,
            message_sound: NotificationSound::Chime,
            group_sound: NotificationSound::Ripple,
            download_folder: None,
            proxy: String::new(),
            check_for_updates: true,
            download_updates_automatically: false,
            names_from_contacts: true,
            save_contacts_to_phone: true,
            voice_speed: 1.0,
            pause_media_while_recording: true,
            pause_media_while_playing: true,
            chat_lock_code: None,
            chat_lock_code_hash: None,
            chat_lock_hint_dismissed: false,
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
    pub fn wallpaper_color_for(&self, dark: bool) -> WallpaperColor {
        if dark {
            self.dark_wallpaper_color
        } else {
            self.wallpaper_color
        }
    }

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
            Ok(contents) => match serde_json::from_str::<Self>(&contents) {
                Ok(mut settings) => {
                    if let Some(code) = settings.chat_lock_code.take() {
                        settings.set_chat_lock_code(Some(&code));
                        if let Err(error) = settings.save(path) {
                            log::warn!("could not replace the legacy locked-chat code: {error}");
                        }
                    }
                    settings
                }
                Err(_error) => {
                    log::warn!("settings file is unreadable, using defaults");
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

    pub fn set_chat_lock_code(&mut self, code: Option<&str>) {
        self.chat_lock_code = None;
        self.chat_lock_code_hash = code
            .map(str::trim)
            .filter(|code| !code.is_empty())
            .map(Self::chat_lock_verifier);
    }

    /// Checking is deliberately slow, so callers memoize the answer.
    pub fn verifies_chat_lock_code(&self, code: &str) -> bool {
        let Some((salt, expected)) = self
            .chat_lock_code_hash
            .as_deref()
            .and_then(|stored| stored.split_once('$'))
        else {
            return false;
        };
        let (Some(salt), Some(expected)) = (unhex(salt), unhex(expected)) else {
            return false;
        };
        ring::pbkdf2::verify(
            ring::pbkdf2::PBKDF2_HMAC_SHA256,
            CHAT_LOCK_ROUNDS,
            &salt,
            code.trim().as_bytes(),
            &expected,
        )
        .is_ok()
    }

    /// `salt$hash`, both hex. Codes are short enough to be guessed offline,
    /// so the stored form is salted and slow rather than a bare digest.
    fn chat_lock_verifier(code: &str) -> String {
        let salt: [u8; 16] = ring::rand::generate(&ring::rand::SystemRandom::new())
            .expect("the system random generator is unavailable")
            .expose();
        let mut hash = [0u8; 32];
        ring::pbkdf2::derive(
            ring::pbkdf2::PBKDF2_HMAC_SHA256,
            CHAT_LOCK_ROUNDS,
            &salt,
            code.as_bytes(),
            &mut hash,
        );
        format!("{}${}", hex(&salt), hex(&hash))
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
        assert!(parsed.show_wallpaper);
        assert_eq!(parsed.wallpaper_color, WallpaperColor::Beige);
        assert!(
            !parsed.collapse_chat_list,
            "hiding the list keeps removing it until asked otherwise"
        );
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
            collapse_chat_list: true,
            interface_language: Some(crate::i18n::Locale::German),
            message_sound: NotificationSound::None,
            group_sound: NotificationSound::Custom("/sounds/ding.wav".into()),
            ..Settings::default()
        };
        settings.save(&path).expect("saves");
        assert_eq!(Settings::load(&path), settings);
        assert!(
            std::fs::read_to_string(&path)
                .expect("reads")
                .contains(r#""interface_language": "de""#),
            "the language keeps its stable serde name"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_locked_chat_verifier_is_salted_and_rejects_other_codes() {
        let mut settings = Settings::default();
        settings.set_chat_lock_code(Some(" 1234 "));
        assert!(settings.verifies_chat_lock_code("1234"));
        assert!(!settings.verifies_chat_lock_code("1235"));
        assert!(!settings.verifies_chat_lock_code(""));
        let first = settings.chat_lock_code_hash.clone();
        settings.set_chat_lock_code(Some("1234"));
        // A fresh salt every time, so the same code never stores the same value.
        assert_ne!(first, settings.chat_lock_code_hash);
        assert!(settings.verifies_chat_lock_code("1234"));
        settings.set_chat_lock_code(None);
        assert!(!settings.verifies_chat_lock_code("1234"));
    }

    #[test]
    fn legacy_locked_chat_code_is_rewritten_as_a_verifier() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"chat_lock_code":"1234"}"#).unwrap();
        let settings = Settings::load(&path);
        assert!(settings.verifies_chat_lock_code("1234"));
        // The random hex verifier may contain "1234" by chance, so check
        // fields rather than searching the file for the digits.
        let stored: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert!(stored.get("chat_lock_code").is_none());
        let verifier = stored["chat_lock_code_hash"].as_str().unwrap();
        assert_ne!(verifier.trim(), "1234");
        assert!(verifier.contains('$'));
    }

    #[test]
    fn wallpaper_palette_contains_the_official_colours() {
        assert_eq!(WallpaperColor::LIGHT.len(), 28);
        assert_eq!(WallpaperColor::DARK.len(), 21);
        assert_eq!(WallpaperColor::Beige.rgb(), [245, 241, 235]);
        assert_eq!(WallpaperColor::Black.rgb(), [22, 23, 23]);
        assert_eq!(WallpaperColor::WillowBrook.label(), "Willow Brook");
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
