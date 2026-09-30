//! Accounts: the local identities the window can show.
//!
//! WhatsApp is the account ZapFast started with. Every other network arrives
//! through its own adapter and gets its own local account id. An account id is
//! a local number, never a phone number or a protocol identifier: it names a
//! directory under the state directory and nothing else. Secrets (tokens, API
//! keys, the archive key) live in the OS keyring; `accounts.json` holds only
//! what the switcher and the welcome screen need.
//!
//! The first start after this feature adopts the pre-account `session.db` and
//! `archive.db` into account 1 following the same spirit as
//! [`crate::paths::AppDirs::adopt_previous_names`]: the files are moved, and
//! the archive's OS keyring key moves with them before the rename, because the
//! keyring identity is a digest of the archive's directory.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::model::{Chat, ChatKind};

/// Local, stable number for one account. Never a phone number or a protocol
/// id; it only names a directory under the state directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountId(pub u64);

impl AccountId {
    /// The account the app started with: the linked WhatsApp device.
    pub const WHATSAPP: Self = Self(1);

    /// The number the switcher and the directories use.
    pub fn get(self) -> u64 {
        self.0
    }
}

impl Default for AccountId {
    fn default() -> Self {
        Self::WHATSAPP
    }
}

impl std::fmt::Display for AccountId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// The network behind an account. `Line` is reserved: ZapFast ships no LINE
/// adapter, and the variant only names the network a future one would use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkKind {
    WhatsApp,
    Telegram,
    Matrix,
    Slack,
    Zulip,
    DiscordBot,
    X,
    DeltaChat,
    Line,
}

impl NetworkKind {
    /// The name shown in the switcher. Proper nouns are not translated.
    pub fn label(self) -> &'static str {
        match self {
            Self::WhatsApp => "WhatsApp",
            Self::Telegram => "Telegram",
            Self::Matrix => "Matrix",
            Self::Slack => "Slack",
            Self::Zulip => "Zulip",
            Self::DiscordBot => "Discord",
            Self::X => "X",
            Self::DeltaChat => "Delta Chat",
            Self::Line => "LINE",
        }
    }

    /// Whether ZapFast ships an adapter for this network. Line stays a
    /// reserved name with no adapter.
    pub fn supported(self) -> bool {
        matches!(
            self,
            Self::WhatsApp
                | Self::Telegram
                | Self::Matrix
                | Self::Slack
                | Self::Zulip
                | Self::DiscordBot
        )
    }
}

/// One account as the switcher and `accounts.json` know it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub id: AccountId,
    pub kind: NetworkKind,
    /// Name the switcher shows. Starts as the network's name.
    pub name: String,
}

impl Account {
    /// The linked WhatsApp device, account 1.
    pub fn whatsapp() -> Self {
        Self {
            id: AccountId::WHATSAPP,
            kind: NetworkKind::WhatsApp,
            name: NetworkKind::WhatsApp.label().to_owned(),
        }
    }

    /// A Telegram account the user is about to sign in.
    pub fn telegram(id: AccountId) -> Self {
        Self {
            id,
            kind: NetworkKind::Telegram,
            name: NetworkKind::Telegram.label().to_owned(),
        }
    }

    /// A Slack account the user is about to connect.
    pub fn slack(id: AccountId) -> Self {
        Self {
            id,
            kind: NetworkKind::Slack,
            name: NetworkKind::Slack.label().to_owned(),
        }
    }

    /// A Zulip account the user is about to sign in.
    pub fn zulip(id: AccountId) -> Self {
        Self {
            id,
            kind: NetworkKind::Zulip,
            name: NetworkKind::Zulip.label().to_owned(),
        }
    }

    /// A Discord bot account the user is about to connect.
    pub fn discord(id: AccountId) -> Self {
        Self {
            id,
            kind: NetworkKind::DiscordBot,
            name: NetworkKind::DiscordBot.label().to_owned(),
        }
    }

    /// A Matrix account the user is about to sign in.
    pub fn matrix(id: AccountId) -> Self {
        Self {
            id,
            kind: NetworkKind::Matrix,
            name: NetworkKind::Matrix.label().to_owned(),
        }
    }
}

/// A local number that no stored account uses yet.
pub fn next_id(accounts: &[Account]) -> AccountId {
    let highest = accounts
        .iter()
        .map(|account| account.id.0)
        .max()
        .unwrap_or(0);
    AccountId(highest.saturating_add(1))
}

/// Where one account's login stands. WhatsApp links this computer as a
/// companion device; every other network signs in with its own credentials.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthState {
    /// Not authorized, and not being asked for anything yet.
    SignedOut,
    /// WhatsApp: waiting for the phone to link this device.
    WhatsappLink,
    /// Telegram: a code was sent to this number. The number stays in memory,
    /// is never logged, and is never written to `accounts.json`.
    TelegramCode { phone: String },
    /// Telegram: the code was accepted and the two-step password is expected.
    TelegramPassword { phone: String },
    /// Sign-in failed. The reason is written for the window to show and never
    /// holds a code or a password.
    Failed { reason: String },
    /// Authorized and connected.
    Ready,
}

/// Loads the account list, falling back to the one WhatsApp account when the
/// file is missing or unreadable. A broken list must not brick startup; the
/// archive and session files are unaffected either way.
pub fn load(path: &Path) -> Vec<Account> {
    match std::fs::read_to_string(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => vec![Account::whatsapp()],
        Err(error) => {
            log::warn!("Could not read the account list: {error}");
            vec![Account::whatsapp()]
        }
        Ok(text) => match serde_json::from_str(&text) {
            Ok(accounts) => accounts,
            Err(error) => {
                log::warn!("Could not read the account list: {error}");
                vec![Account::whatsapp()]
            }
        },
    }
}

/// Writes the account list. Only names, networks, and local numbers live here.
pub fn save(path: &Path, accounts: &[Account]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Could not create {}", parent.display()))?;
    }
    let mut text = serde_json::to_string_pretty(accounts)?;
    text.push('\n');
    std::fs::write(path, text).with_context(|| format!("Could not write {}", path.display()))
}

/// Prepares the account list on startup and adopts the pre-account WhatsApp
/// files into account 1 on the first run after the update. `accounts.json` is
/// the marker: only its absence triggers adoption, and the steps are ordered
/// so an interruption can be retried by the next start.
pub fn adopt(dirs: &crate::paths::AppDirs) -> Result<()> {
    let list = dirs.accounts_file();
    if list.exists() {
        return Ok(());
    }
    adopt_legacy_whatsapp(dirs)?;
    save(&list, &[Account::whatsapp()])
}

fn adopt_legacy_whatsapp(dirs: &crate::paths::AppDirs) -> Result<()> {
    let pairs = [
        (
            dirs.legacy_session_db(),
            dirs.session_db(AccountId::WHATSAPP),
        ),
        (
            dirs.legacy_archive_db(),
            dirs.archive_db(AccountId::WHATSAPP),
        ),
    ];
    if pairs.iter().all(|(from, to)| !from.exists() || to.exists()) {
        return Ok(());
    }
    let account = dirs.account_dir(AccountId::WHATSAPP);
    crate::paths::create_private_dir(&account)
        .with_context(|| format!("Could not create {}", account.display()))?;
    // The archive key's keyring identity is a digest of its directory, so the
    // key moves to the new path before the file does.
    let (legacy_archive, archive) = &pairs[1];
    if legacy_archive.exists() && !archive.exists() {
        crate::archive::move_archive_key(legacy_archive, archive)?;
    }
    for (from, to) in &pairs {
        if from.exists() && !to.exists() {
            move_family(from, to)?;
        }
    }
    Ok(())
}

/// Renames a state file and its SQLite sidecars. Anything already moved stays
/// put, so a retry after an interruption finishes the rest.
fn move_family(from: &Path, to: &Path) -> Result<()> {
    std::fs::rename(from, to).with_context(|| format!("Could not move {}", from.display()))?;
    for suffix in ["-wal", "-shm"] {
        let side = |path: &Path| {
            let mut name = path.as_os_str().to_owned();
            name.push(suffix);
            PathBuf::from(name)
        };
        let side_from = side(from);
        if side_from.exists() {
            std::fs::rename(&side_from, side(to))
                .with_context(|| format!("Could not move {}", side_from.display()))?;
        }
    }
    Ok(())
}

/// What an account's chats can do. Controls read this instead of assuming
/// WhatsApp. Only `text` is acted on today; the rest describe each network for
/// the controls the adapters bring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub text: bool,
    pub files: bool,
    pub voice_notes: bool,
    pub video_notes: bool,
    pub custom_emoji: bool,
    pub reactions: ReactionStyle,
    pub gif: GifSource,
    pub stickers: StickerAccess,
    pub threads: bool,
    pub spaces: bool,
    pub typing: bool,
    pub read_receipts: bool,
    pub edit: bool,
    pub delete_for_everyone: bool,
    pub polls: bool,
    pub calls: CallAccess,
}

/// How an account reacts to a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReactionStyle {
    None,
    ReplaceOne,
    Counts,
}

/// Where a GIF search can go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GifSource {
    None,
    Giphy,
    TelegramInline,
    Attachment,
}

/// What an account accepts as a sticker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StickerAccess {
    None,
    UploadWebp,
    AccountPacks,
    GuildOnly,
    PackId,
}

/// Whether calls can be placed, logged, or neither.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallAccess {
    None,
    LogOnly,
    PlaceVoice,
}

impl Capabilities {
    /// The linked WhatsApp device, matching today's behaviour exactly.
    pub fn whatsapp(chat: &Chat) -> Self {
        let sendable =
            !chat.locked && !chat.read_only && !chat.left && chat.kind != ChatKind::Broadcast;
        Self {
            text: sendable,
            files: sendable,
            voice_notes: sendable,
            video_notes: false,
            custom_emoji: false,
            reactions: if sendable {
                ReactionStyle::ReplaceOne
            } else {
                ReactionStyle::None
            },
            gif: GifSource::Giphy,
            stickers: StickerAccess::UploadWebp,
            threads: false,
            spaces: false,
            typing: true,
            read_receipts: true,
            edit: true,
            delete_for_everyone: true,
            polls: true,
            calls: CallAccess::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(id: &str) -> Chat {
        Chat::new(crate::model::ChatId::whatsapp(id), "Fixture".to_owned())
    }

    #[test]
    fn a_missing_account_list_is_the_linked_device() {
        let directory = tempfile::tempdir().unwrap();
        let accounts = load(&directory.path().join("accounts.json"));
        assert_eq!(accounts, vec![Account::whatsapp()]);
    }

    #[test]
    fn a_broken_account_list_still_opens_whatsapp() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("accounts.json");
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(load(&path), vec![Account::whatsapp()]);
    }

    #[test]
    fn the_account_list_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("accounts.json");
        let accounts = vec![
            Account::whatsapp(),
            Account {
                id: AccountId(2),
                kind: NetworkKind::Telegram,
                name: "Telegram".to_owned(),
            },
        ];
        save(&path, &accounts).unwrap();
        assert_eq!(load(&path), accounts);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"telegram\""), "networks use stable names");
    }

    #[test]
    fn adoption_moves_the_legacy_files_and_marks_the_list() {
        let directory = tempfile::tempdir().unwrap();
        let dirs = crate::paths::AppDirs::under(directory.path());
        dirs.ensure().unwrap();
        std::fs::write(dirs.legacy_session_db(), b"session bytes").unwrap();
        std::fs::write(dirs.legacy_archive_db(), b"archive bytes").unwrap();
        std::fs::write(
            format!("{}-wal", dirs.legacy_session_db().display()),
            b"wal",
        )
        .unwrap();

        adopt(&dirs).unwrap();

        assert!(!dirs.legacy_session_db().exists());
        assert_eq!(
            std::fs::read(dirs.session_db(AccountId::WHATSAPP)).unwrap(),
            b"session bytes"
        );
        assert_eq!(
            std::fs::read(dirs.archive_db(AccountId::WHATSAPP)).unwrap(),
            b"archive bytes"
        );
        assert_eq!(
            std::fs::read(format!(
                "{}-wal",
                dirs.session_db(AccountId::WHATSAPP).display()
            ))
            .unwrap(),
            b"wal"
        );
        assert_eq!(load(&dirs.accounts_file()), vec![Account::whatsapp()]);

        // A second start changes nothing.
        adopt(&dirs).unwrap();
        assert_eq!(load(&dirs.accounts_file()), vec![Account::whatsapp()]);
    }

    #[test]
    fn an_interrupted_adoption_finishes_on_the_next_run() {
        let directory = tempfile::tempdir().unwrap();
        let dirs = crate::paths::AppDirs::under(directory.path());
        dirs.ensure().unwrap();
        std::fs::write(dirs.legacy_session_db(), b"session bytes").unwrap();
        std::fs::write(dirs.legacy_archive_db(), b"archive bytes").unwrap();
        // Simulate the first attempt having moved the session file already.
        crate::paths::create_private_dir(&dirs.account_dir(AccountId::WHATSAPP)).unwrap();
        std::fs::rename(
            dirs.legacy_session_db(),
            dirs.session_db(AccountId::WHATSAPP),
        )
        .unwrap();

        adopt(&dirs).unwrap();

        assert!(!dirs.legacy_archive_db().exists());
        assert_eq!(
            std::fs::read(dirs.archive_db(AccountId::WHATSAPP)).unwrap(),
            b"archive bytes"
        );
        assert_eq!(load(&dirs.accounts_file()), vec![Account::whatsapp()]);
    }

    #[test]
    fn adoption_without_legacy_files_only_writes_the_list() {
        let directory = tempfile::tempdir().unwrap();
        let dirs = crate::paths::AppDirs::under(directory.path());
        dirs.ensure().unwrap();

        adopt(&dirs).unwrap();

        assert_eq!(load(&dirs.accounts_file()), vec![Account::whatsapp()]);
        assert!(!dirs.account_dir(AccountId::WHATSAPP).exists());
    }

    #[test]
    fn whatsapp_capabilities_follow_the_send_gates() {
        let direct = chat("1@s.whatsapp.net");
        let caps = Capabilities::whatsapp(&direct);
        assert!(caps.text);
        assert!(caps.files);
        assert!(caps.voice_notes);
        assert_eq!(caps.reactions, ReactionStyle::ReplaceOne);
        assert!(!caps.spaces);
        assert_eq!(caps.calls, CallAccess::None);

        let mut locked = direct.clone();
        locked.locked = true;
        let caps = Capabilities::whatsapp(&locked);
        assert!(!caps.text);
        assert!(!caps.files);
        assert!(!caps.voice_notes);
        assert_eq!(caps.reactions, ReactionStyle::None);

        let newsletter = chat("1@newsletter");
        assert!(!Capabilities::whatsapp(&newsletter).text);

        let group = chat("1-2@g.us");
        let caps = Capabilities::whatsapp(&group);
        assert!(caps.text);
        assert_eq!(caps.gif, GifSource::Giphy);
        assert_eq!(caps.stickers, StickerAccess::UploadWebp);
    }
}
