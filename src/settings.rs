//! User preferences stored in JSON.

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Verifying the locked-chat code costs about 20 ms, paid once per distinct
/// typed string. ponytail: fixed cost, revisit if it lags the search field.
const CHAT_LOCK_ROUNDS: std::num::NonZeroU32 = std::num::NonZeroU32::new(200_000).unwrap();

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn unhex(value: &str) -> Option<Vec<u8>> {
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

/// The typeface the interface is drawn with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FontChoice {
    /// The platform's own interface font.
    #[default]
    System,
    /// The bundled Inter, the same on every machine.
    Inter,
}

impl FontChoice {
    pub const ALL: [FontChoice; 2] = [Self::System, Self::Inter];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Inter => "Inter",
        }
    }
}

/// Background colours offered by WhatsApp's wallpaper picker, after the
/// active theme's own chat colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WallpaperColor {
    /// The active palette's `chat` colour, so a custom theme sets the wallpaper.
    #[default]
    Theme,
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
    pub const LIGHT: [Self; 29] = [
        Self::Theme,
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

    pub const DARK: [Self; 22] = [
        Self::Theme,
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

    /// The colour's English name. The interface translates [`Self::Theme`]'s.
    pub fn label(self) -> &'static str {
        match self {
            Self::Theme => "Theme",
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

    /// A fixed colour's value; `None` for [`Self::Theme`], which follows the
    /// palette.
    pub fn rgb(self) -> Option<[u8; 3]> {
        Some(match self {
            Self::Theme => return None,
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
        })
    }

    /// The colour drawn under `palette`: [`Self::Theme`] is its chat colour,
    /// read at draw time so a theme switch or Omarchy reload shows at once.
    pub fn color32(self, palette: &crate::theme::Palette) -> egui::Color32 {
        match self.rgb() {
            Some([r, g, b]) => egui::Color32::from_rgb(r, g, b),
            None => palette.chat,
        }
    }
}

/// How long ZapFast may go without input before the app lock locks it:
/// WhatsApp Web's three choices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoLock {
    OneMinute,
    #[default]
    FifteenMinutes,
    OneHour,
}

impl AutoLock {
    pub const ALL: [AutoLock; 3] = [Self::OneMinute, Self::FifteenMinutes, Self::OneHour];

    pub fn duration(self) -> std::time::Duration {
        std::time::Duration::from_secs(match self {
            Self::OneMinute => 60,
            Self::FifteenMinutes => 15 * 60,
            Self::OneHour => 60 * 60,
        })
    }

    pub fn label(self, locale: crate::i18n::Locale) -> std::borrow::Cow<'static, str> {
        match self {
            Self::OneMinute => crate::i18n::gettext(locale, "After 1 minute"),
            Self::FifteenMinutes => crate::i18n::gettext(locale, "After 15 minutes"),
            Self::OneHour => crate::i18n::gettext(locale, "After 1 hour"),
        }
    }
}

/// The sound a new-message notification makes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSound {
    /// Pidgin's message sound, the default for new messages.
    #[default]
    #[serde(alias = "chime")]
    Receive,
    /// Pidgin's alert sound, the default for mentions and replies to us.
    #[serde(alias = "ripple")]
    Alert,
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
    /// [`SETTINGS_VERSION`] when written; missing, and so 0, in older files.
    #[serde(default)]
    pub version: u32,
    pub theme: ThemeChoice,
    /// The interface's typeface.
    pub font: FontChoice,
    /// Interface language. `None` follows the operating system's locale.
    pub interface_language: Option<crate::i18n::Locale>,
    /// Filename of the selected local JSON palette.
    pub custom_theme: Option<String>,
    #[serde(
        default,
        deserialize_with = "fastframe_theme::read_cached_theme",
        skip_serializing_if = "Option::is_none"
    )]
    pub custom_theme_cache: Option<crate::theme::CustomTheme>,
    #[serde(
        default,
        deserialize_with = "fastframe_theme::read_cached_theme",
        skip_serializing_if = "Option::is_none"
    )]
    pub system_theme_cache: Option<crate::theme::CustomTheme>,
    /// egui zoom factor.
    pub zoom: f32,
    pub sidebar_width: f32,
    /// Width of the search pane beside the open chat.
    pub search_pane_width: f32,
    /// Whether Enter sends. Off, Enter adds a line and Ctrl+Enter (Cmd+Enter
    /// on macOS) sends.
    pub enter_sends: bool,
    /// Send read receipts, subject to the account privacy setting.
    pub send_read_receipts: bool,
    /// Send typing state while composing.
    pub send_typing: bool,
    /// Download attachments when they enter view instead of on click.
    #[serde(alias = "auto_download_images")]
    pub auto_download: bool,
    /// Show the default doodle wallpaper behind conversations.
    pub show_wallpaper: bool,
    /// Colour selected in the wallpaper picker.
    pub wallpaper_color: WallpaperColor,
    /// Colour selected for the dark wallpaper picker.
    pub dark_wallpaper_color: WallpaperColor,
    /// ZapFast's own copy of the chosen wallpaper image, drawn in place of the
    /// colour and doodles in light and dark mode alike.
    pub wallpaper_image: Option<std::path::PathBuf>,
    /// Last open chat, restored at startup.
    pub last_chat: Option<String>,
    /// The hint bar under the composer, hidden with its × and shown again
    /// from the Keyboard shortcuts dialog.
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
    /// Sound for new messages, in one-to-one chats and groups alike.
    pub message_sound: NotificationSound,
    /// Sound for group messages that mention us or reply to one of ours, as
    /// Pidgin alerts when someone says your name in a chat.
    pub mention_sound: NotificationSound,
    /// Play the message sound for ordinary group messages. Mentions and
    /// replies to us sound either way.
    pub group_sounds: bool,
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
    /// Voice and audio playback speed multiplier.
    pub voice_speed: f32,
    /// Pause other apps' media while recording, or while a voice message,
    /// audio, or video plays with sound.
    pub pause_other_media: bool,
    /// The two switches `pause_other_media` replaced, read once and folded
    /// into it by [`Settings::load`].
    #[serde(skip_serializing)]
    pub pause_media_while_recording: Option<bool>,
    #[serde(skip_serializing)]
    pub pause_media_while_playing: Option<bool>,
    /// The last choice of the new-contact dialog's "Save to phone" box, which
    /// starts the next one and applies when a contact is renamed.
    pub save_contacts_to_phone: bool,
    /// Legacy plaintext code, accepted once and rewritten as a verifier.
    #[serde(skip_serializing)]
    pub chat_lock_code: Option<String>,
    /// Salted PBKDF2 verifier for the local locked-chats code, `salt$hash`.
    pub chat_lock_code_hash: Option<String>,
    /// The one-time locked-chat code hint has been opened.
    pub chat_lock_hint_dismissed: bool,
    /// Salted, slow verifier of the app lock password
    /// ([`crate::app_lock::verifier`]); `None` leaves the app lock off.
    pub app_lock_hash: Option<String>,
    /// How long ZapFast may go unused before the app lock locks it.
    pub app_lock_after: AutoLock,
    /// Last window size in points, remembered across restarts. `None` uses
    /// the default size.
    pub window_width: Option<f32>,
    /// Last window height in points. See [`Self::window_width`].
    pub window_height: Option<f32>,
    /// Last window position in points, as `ViewportBuilder::with_position`
    /// takes it. `None` lets the window manager place the window, which is
    /// also the case on Wayland where the position is not known.
    pub window_x: Option<f32>,
    /// Last window vertical position. See [`Self::window_x`].
    pub window_y: Option<f32>,
    /// Whether the window was maximized when it last closed.
    pub window_maximized: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            theme: ThemeChoice::Dark,
            font: FontChoice::System,
            interface_language: None,
            custom_theme: None,
            custom_theme_cache: None,
            system_theme_cache: None,
            zoom: 1.0,
            sidebar_width: 320.0,
            search_pane_width: 380.0,
            enter_sends: true,
            send_read_receipts: true,
            send_typing: true,
            auto_download: true,
            show_wallpaper: true,
            wallpaper_color: WallpaperColor::Theme,
            dark_wallpaper_color: WallpaperColor::Theme,
            wallpaper_image: None,
            last_chat: None,
            show_shortcut_hints: true,
            recent_emoji: Vec::new(),
            reaction_emoji: Vec::new(),
            giphy_key: String::new(),
            keep_running_in_background: true,
            notifications: true,
            message_sound: NotificationSound::Receive,
            mention_sound: NotificationSound::Alert,
            group_sounds: true,
            download_folder: None,
            proxy: String::new(),
            check_for_updates: true,
            download_updates_automatically: false,
            save_contacts_to_phone: true,
            voice_speed: 1.0,
            pause_other_media: true,
            pause_media_while_recording: None,
            pause_media_while_playing: None,
            chat_lock_code: None,
            chat_lock_code_hash: None,
            chat_lock_hint_dismissed: false,
            app_lock_hash: None,
            app_lock_after: AutoLock::default(),
            window_width: None,
            window_height: None,
            window_x: None,
            window_y: None,
            window_maximized: false,
        }
    }
}

/// The settings file's format. Files without a version predate it and are
/// migrated once by [`Settings::load`]:
///
/// 1. The wallpaper colours that were the defaults (Beige, and Black in dark
///    mode) become [`WallpaperColor::Theme`], the new default.
pub const SETTINGS_VERSION: u32 = 1;

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

    /// The wallpaper colour for `palette`, with Theme resolved against it.
    pub fn wallpaper_background(&self, palette: &crate::theme::Palette) -> egui::Color32 {
        self.wallpaper_color_for(palette.dark).color32(palette)
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

    /// The remembered window geometry for the next window.
    pub fn window_geometry(&self) -> crate::window::Geometry {
        crate::window::Geometry {
            width: self.window_width,
            height: self.window_height,
            x: self.window_x,
            y: self.window_y,
            maximized: self.window_maximized,
        }
    }

    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => match serde_json::from_str::<Self>(&contents) {
                Ok(mut settings) => {
                    settings.migrate();
                    settings.fold_legacy_media_pause();
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

    /// Brings a file written before [`SETTINGS_VERSION`] up to date. Each
    /// step runs once: the next save records the version, so a colour chosen
    /// again afterwards is kept.
    fn migrate(&mut self) {
        if self.version < 1 {
            // Old files store every field, so a default colour cannot be told
            // from a chosen one; the old defaults move to the new one.
            if self.wallpaper_color == WallpaperColor::Beige {
                self.wallpaper_color = WallpaperColor::Theme;
            }
            if self.dark_wallpaper_color == WallpaperColor::Black {
                self.dark_wallpaper_color = WallpaperColor::Theme;
            }
        }
        self.version = self.version.max(SETTINGS_VERSION);
    }

    /// Folds the former recording and playback switches into
    /// `pause_other_media`. A file that still has them was last written by a
    /// version without the merged switch, so they win: media keeps pausing
    /// only if neither was turned off, which never pauses music for someone
    /// who asked it not to be. The next save drops them.
    fn fold_legacy_media_pause(&mut self) {
        let recording = self.pause_media_while_recording.take();
        let playing = self.pause_media_while_playing.take();
        if recording.is_some() || playing.is_some() {
            self.pause_other_media = recording.unwrap_or(true) && playing.unwrap_or(true);
        }
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

/// Preferences that belong to one WhatsApp account.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountSettings {
    pub send_read_receipts: bool,
    pub send_typing: bool,
    #[serde(alias = "auto_download_images")]
    pub auto_download: bool,
    pub last_chat: Option<String>,
    pub notifications: bool,
    pub save_contacts_to_phone: bool,
    /// This account's copy of the chosen chat wallpaper image.
    pub wallpaper_image: Option<std::path::PathBuf>,
}

impl Default for AccountSettings {
    fn default() -> Self {
        Self {
            send_read_receipts: true,
            send_typing: true,
            auto_download: true,
            last_chat: None,
            notifications: true,
            save_contacts_to_phone: true,
            wallpaper_image: None,
        }
    }
}

impl AccountSettings {
    pub fn from_legacy(settings: &Settings) -> Self {
        Self {
            send_read_receipts: settings.send_read_receipts,
            send_typing: settings.send_typing,
            auto_download: settings.auto_download,
            last_chat: settings.last_chat.clone(),
            notifications: settings.notifications,
            save_contacts_to_phone: settings.save_contacts_to_phone,
            wallpaper_image: settings.wallpaper_image.clone(),
        }
    }

    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_else(|error| {
                log::warn!("account settings are unreadable, using the defaults: {error}");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

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

/// Which accounts exist on this computer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccountRoster {
    pub next_id: u32,
    pub active: String,
    pub order: Vec<String>,
}

impl Default for AccountRoster {
    fn default() -> Self {
        Self {
            next_id: 2,
            active: "1".into(),
            order: vec!["1".into()],
        }
    }
}

impl AccountRoster {
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let mut roster = match std::fs::read_to_string(path) {
            Ok(contents) => serde_json::from_str(&contents).map_err(std::io::Error::other)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => return Err(error),
        };
        roster.sanitize();
        Ok(roster)
    }

    /// Drops ids that cannot be folder names, so a hand-edited file cannot
    /// point state or a later delete at an arbitrary path. Keeps the first
    /// spelling of a repeated id so two workers never share one archive.
    pub fn sanitize(&mut self) {
        let mut seen = HashSet::new();
        self.order
            .retain(|id| crate::model::AccountId::is_safe(id) && seen.insert(id.clone()));
        if !self.order.iter().any(|id| id == &self.active) {
            self.active = self.order.first().cloned().unwrap_or_else(|| "1".into());
        }
        let highest = self
            .order
            .iter()
            .filter_map(|id| id.parse::<u32>().ok())
            .max()
            .unwrap_or(1);
        self.next_id = self.next_id.max(highest + 1).max(2);
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, contents)?;
        std::fs::rename(&temp, path)
    }

    pub fn allocate(&mut self) -> crate::model::AccountId {
        let id = crate::model::AccountId(self.next_id.to_string());
        self.next_id += 1;
        self.order.push(id.0.clone());
        self.active = id.0.clone();
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn earlier_bundled_sound_names_still_load() {
        let settings: Settings =
            serde_json::from_str(r#"{"message_sound":"chime","mention_sound":"ripple"}"#).unwrap();
        assert_eq!(settings.message_sound, NotificationSound::Receive);
        assert_eq!(settings.mention_sound, NotificationSound::Alert);
    }

    #[test]
    fn the_former_group_sound_gives_way_to_mentions_and_quiet_groups() {
        // Group sound used to play for every group message; it is dropped,
        // and groups follow the message sound until they are silenced.
        let settings: Settings =
            serde_json::from_str(r#"{"message_sound":"system","group_sound":"none"}"#).unwrap();
        assert_eq!(settings.message_sound, NotificationSound::System);
        assert_eq!(settings.mention_sound, NotificationSound::Alert);
        assert!(settings.group_sounds);
    }

    /// Settings written before the font could be chosen draw with the
    /// platform's; the choice is saved by name.
    #[test]
    fn the_font_defaults_to_the_platform_and_is_saved_by_name() {
        let older: Settings = serde_json::from_str(r#"{"theme":"light"}"#).unwrap();
        assert_eq!(older.font, FontChoice::System);
        let chosen: Settings = serde_json::from_str(r#"{"font":"inter"}"#).unwrap();
        assert_eq!(chosen.font, FontChoice::Inter);
        let saved = serde_json::to_value(&chosen).unwrap();
        assert_eq!(saved["font"], "inter");
    }

    #[test]
    fn unknown_and_missing_fields_are_tolerated() {
        let parsed: Settings =
            serde_json::from_str(r#"{"theme":"light","future_field":1}"#).expect("parses");
        assert_eq!(parsed.theme, ThemeChoice::Light);
        assert!(parsed.enter_sends);
        assert!(parsed.check_for_updates);
        assert!(!parsed.download_updates_automatically);
        assert!(parsed.show_wallpaper);
        assert_eq!(parsed.wallpaper_color, WallpaperColor::Theme);
        assert!(parsed.pause_other_media);
        assert_eq!(parsed.window_width, None);
        assert_eq!(parsed.window_height, None);
        assert_eq!(parsed.window_x, None);
        assert_eq!(parsed.window_y, None);
        assert!(!parsed.window_maximized);
    }

    #[test]
    fn the_window_geometry_defaults_to_unset() {
        // Files written before the window was remembered open with the
        // default size; the full round-trip lives in `window::tests`.
        let settings = Settings::default();
        assert_eq!(
            settings.window_geometry(),
            crate::window::Geometry::default()
        );
        let older: Settings = serde_json::from_str(r#"{"zoom":1.25}"#).unwrap();
        assert_eq!(older.window_geometry(), crate::window::Geometry::default());
    }

    fn load_from(contents: &str) -> (Settings, serde_json::Value) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, contents).unwrap();
        let settings = Settings::load(&path);
        settings.save(&path).unwrap();
        let stored = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        (settings, stored)
    }

    #[test]
    fn removed_settings_are_ignored_and_dropped_on_save() {
        let (settings, stored) = load_from(
            r#"{"enter_sends":false,"label_chips":true,"show_sender_pictures":true,
                "names_from_contacts":false,"collapse_chat_list":false,
                "save_contacts_to_phone":false}"#,
        );
        assert!(!settings.enter_sends, "the rest of the file still loads");
        assert!(
            !settings.save_contacts_to_phone,
            "the old switch starts the new-contact box"
        );
        for key in [
            "label_chips",
            "show_sender_pictures",
            "names_from_contacts",
            "collapse_chat_list",
            "pause_media_while_recording",
            "pause_media_while_playing",
        ] {
            assert!(stored.get(key).is_none(), "{key} is dropped on save");
        }
        assert_eq!(stored["pause_other_media"], true);
    }

    #[test]
    fn the_two_media_pause_switches_merge_into_one() {
        let merged = |contents: &str| {
            let (settings, stored) = load_from(contents);
            assert!(stored.get("pause_media_while_recording").is_none());
            assert!(stored.get("pause_media_while_playing").is_none());
            settings.pause_other_media
        };
        assert!(merged(
            r#"{"pause_media_while_recording":true,"pause_media_while_playing":true}"#
        ));
        assert!(!merged(
            r#"{"pause_media_while_recording":false,"pause_media_while_playing":true}"#
        ));
        assert!(!merged(
            r#"{"pause_media_while_recording":true,"pause_media_while_playing":false}"#
        ));
        assert!(!merged(r#"{"pause_media_while_playing":false}"#));
        assert!(merged(r#"{"pause_media_while_recording":true}"#));
        assert!(merged("{}"), "on by default");
        assert!(!merged(r#"{"pause_other_media":false}"#));
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
            pause_other_media: false,
            interface_language: Some(crate::i18n::Locale::German),
            message_sound: NotificationSound::None,
            mention_sound: NotificationSound::Custom("/sounds/ding.wav".into()),
            group_sounds: false,
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
        // WhatsApp's 28 light and 21 dark colours, after the theme's own.
        assert_eq!(WallpaperColor::LIGHT.len(), 29);
        assert_eq!(WallpaperColor::DARK.len(), 22);
        assert_eq!(WallpaperColor::LIGHT[0], WallpaperColor::Theme);
        assert_eq!(WallpaperColor::DARK[0], WallpaperColor::Theme);
        assert_eq!(WallpaperColor::Beige.rgb(), Some([245, 241, 235]));
        assert_eq!(WallpaperColor::Black.rgb(), Some([22, 23, 23]));
        assert_eq!(WallpaperColor::WillowBrook.label(), "Willow Brook");
    }

    #[test]
    fn the_theme_wallpaper_is_the_palettes_chat_colour() {
        use crate::theme::Palette;
        let settings = Settings::default();
        assert_eq!(settings.wallpaper_color, WallpaperColor::Theme);
        assert_eq!(settings.dark_wallpaper_color, WallpaperColor::Theme);
        let light = Palette::light();
        let dark = Palette::dark();
        assert_eq!(settings.wallpaper_background(&light), light.chat);
        assert_eq!(settings.wallpaper_background(&dark), dark.chat);
        let mut custom = Palette::dark();
        custom.chat = egui::Color32::from_rgb(30, 30, 46);
        assert_eq!(settings.wallpaper_background(&custom), custom.chat);
        // A fixed colour ignores the palette.
        let chosen = Settings {
            dark_wallpaper_color: WallpaperColor::Nordic,
            ..Settings::default()
        };
        assert_eq!(
            chosen.wallpaper_background(&custom),
            egui::Color32::from_rgb(15, 36, 36)
        );
        assert_eq!(chosen.wallpaper_background(&light), light.chat);
    }

    #[test]
    fn the_old_default_wallpaper_colours_become_the_theme_once() {
        let (settings, stored) =
            load_from(r#"{"wallpaper_color":"beige","dark_wallpaper_color":"black"}"#);
        assert_eq!(settings.wallpaper_color, WallpaperColor::Theme);
        assert_eq!(settings.dark_wallpaper_color, WallpaperColor::Theme);
        assert_eq!(stored["version"], SETTINGS_VERSION);
        assert_eq!(stored["wallpaper_color"], "theme");

        // Chosen colours survive the migration.
        let (settings, _) =
            load_from(r#"{"wallpaper_color":"cruise","dark_wallpaper_color":"nordic"}"#);
        assert_eq!(settings.wallpaper_color, WallpaperColor::Cruise);
        assert_eq!(settings.dark_wallpaper_color, WallpaperColor::Nordic);
        let (settings, _) =
            load_from(r#"{"wallpaper_color":"beige","dark_wallpaper_color":"tiber"}"#);
        assert_eq!(settings.wallpaper_color, WallpaperColor::Theme);
        assert_eq!(settings.dark_wallpaper_color, WallpaperColor::Tiber);

        // Once migrated, choosing Beige or Black again sticks.
        let (settings, _) =
            load_from(r#"{"version":1,"wallpaper_color":"beige","dark_wallpaper_color":"black"}"#);
        assert_eq!(settings.wallpaper_color, WallpaperColor::Beige);
        assert_eq!(settings.dark_wallpaper_color, WallpaperColor::Black);

        // A current file round-trips unchanged, image path included.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let chosen = Settings {
            wallpaper_color: WallpaperColor::Beige,
            dark_wallpaper_color: WallpaperColor::Black,
            wallpaper_image: Some(dir.path().join("wallpaper.png")),
            ..Settings::default()
        };
        chosen.save(&path).unwrap();
        assert_eq!(Settings::load(&path), chosen);
        assert_eq!(
            Settings::load(&path),
            chosen,
            "loading again changes nothing"
        );
    }

    #[test]
    fn a_hand_edited_account_id_cannot_leave_the_accounts_folder() {
        use crate::model::AccountId;
        assert!(AccountId::is_safe("1"));
        assert!(AccountId::is_safe("12"));
        assert!(!AccountId::is_safe(""));
        assert!(!AccountId::is_safe("0"));
        assert!(!AccountId::is_safe("01"));
        assert!(!AccountId::is_safe(".."));
        assert!(!AccountId::is_safe("/tmp"));
        assert!(!AccountId::is_safe("1/../etc"));
    }

    #[test]
    fn a_roster_drops_unsafe_ids() {
        let mut roster = AccountRoster {
            next_id: 4,
            active: "../etc".into(),
            order: vec!["1".into(), "..".into(), "/tmp".into()],
        };
        roster.sanitize();
        assert_eq!(roster.order, ["1".to_string()]);
        assert_eq!(roster.active, "1");
        assert!(roster.next_id > 1);
    }

    #[test]
    fn a_roster_keeps_the_first_copy_of_a_repeated_id() {
        let mut roster = AccountRoster {
            next_id: 3,
            active: "1".into(),
            order: vec!["1".into(), "1".into(), "2".into()],
        };
        roster.sanitize();
        assert_eq!(roster.order, ["1".to_string(), "2".to_string()]);
    }

    #[test]
    fn a_broken_roster_file_does_not_become_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        std::fs::write(&path, "{").unwrap();
        assert!(AccountRoster::load(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{");
    }

    #[test]
    fn allocating_an_account_advances_the_roster() {
        let mut roster = AccountRoster::default();
        assert_eq!(roster.order, ["1".to_string()]);
        let second = roster.allocate();
        assert_eq!(second.as_str(), "2");
        assert_eq!(roster.next_id, 3);
        assert_eq!(roster.active, "2");
        assert_eq!(roster.order, ["1".to_string(), "2".to_string()]);
    }

    #[test]
    fn a_missing_roster_file_starts_with_the_first_account() {
        let dir = tempfile::tempdir().unwrap();
        let roster = AccountRoster::load(&dir.path().join("accounts.json")).unwrap();
        assert_eq!(roster, AccountRoster::default());
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
