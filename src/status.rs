//! Expiring Status updates, kept apart from chat history.

use crate::model::Content;
use std::{collections::HashSet, path::PathBuf};

/// Explicit audience for this publication; never inferred from an absent phone setting.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Audience {
    pub mode: AudienceMode,
    pub selected: HashSet<String>,
}

/// The three audience choices offered by the Status composer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AudienceMode {
    Contacts,
    Except,
    Only,
}

impl Audience {
    /// Filters the known, saved address book; a custom list cannot add unknown contacts.
    pub fn recipients(&self, contacts: impl IntoIterator<Item = String>) -> Vec<String> {
        let mut recipients: Vec<_> = contacts
            .into_iter()
            .filter(|id| match self.mode {
                AudienceMode::Contacts => true,
                AudienceMode::Except => !self.selected.contains(id),
                AudienceMode::Only => self.selected.contains(id),
            })
            .collect();
        recipients.sort();
        recipients.dedup();
        recipients
    }
}

/// Independent remembered selections, stored inside this account's encrypted archive.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AudienceLists {
    pub current: Option<Audience>,
    pub only: HashSet<String>,
    pub except: HashSet<String>,
}

impl AudienceLists {
    pub fn choose(&mut self, audience: Audience) {
        match audience.mode {
            AudienceMode::Only => self.only = audience.selected.clone(),
            AudienceMode::Except => self.except = audience.selected.clone(),
            AudienceMode::Contacts => {}
        }
        self.current = Some(audience);
    }
    pub fn remap(&mut self, from: &str, to: &str) {
        for selected in [&mut self.only, &mut self.except] {
            if selected.remove(from) {
                selected.insert(to.to_owned());
            }
        }
        if let Some(current) = &mut self.current
            && current.selected.remove(from)
        {
            current.selected.insert(to.to_owned());
        }
    }
    pub fn selected(&self, mode: AudienceMode) -> HashSet<String> {
        match mode {
            AudienceMode::Only => self.only.clone(),
            AudienceMode::Except => self.except.clone(),
            AudienceMode::Contacts => HashSet::new(),
        }
    }
}

/// A sorted snapshot for one open contact picker; only matching row indices change on search.
#[derive(Clone, Debug, Default)]
pub struct ContactChoices {
    pub all: std::sync::Arc<Vec<crate::model::Contact>>,
    pub keys: Vec<String>,
    pub ids: HashSet<String>,
    pub query: Option<String>,
    pub matches: std::sync::Arc<Vec<usize>>,
}

/// An unsent update. Sending failures keep this content available for correction.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub token: u64,
    pub text: String,
    pub media: Option<PathBuf>,
    pub edit_target: Option<crate::app::AttachmentId>,
    pub picture_origin: Option<crate::model::PictureOrigin>,
    pub editor: Option<Box<crate::photo::Editor>>,
    pub quality: crate::photo::Quality,
    pub undo: Vec<PathBuf>,
    pub redo: Vec<PathBuf>,
    pub background: usize,
    pub font: usize,
    pub busy: bool,
    pub error: Option<String>,
}

impl Default for Draft {
    fn default() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            token: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            text: String::new(),
            media: None,
            edit_target: None,
            picture_origin: None,
            editor: None,
            quality: crate::photo::Quality::Standard,
            undo: Vec::new(),
            redo: Vec::new(),
            background: 0,
            font: 0,
            busy: false,
            error: None,
        }
    }
}

/// Backgrounds cycled by the text editor, beginning with the reference's burgundy.
pub const BACKGROUNDS: [u32; 8] = [
    0xff762238, 0xff245c4b, 0xff5158a6, 0xff7e426c, 0xff795e25, 0xff24465e, 0xff444444, 0xffd6c7a5,
];

/// WhatsApp Status updates expire 24 hours after publication.
pub const LIFETIME: i64 = 24 * 60 * 60;
/// Bound the session's protocol data, thumbnails and temporary media.
pub const LIMIT: usize = 1024;
pub const RAW_LIMIT: usize = 256 * 1024;
/// Revocations remembered at once; past it the oldest is forgotten.
pub const REVOKE_LIMIT: usize = 4096;
/// Revocations remembered for one author, so one contact cannot evict everyone else's.
pub const REVOKE_PER_SENDER: usize = 128;
/// Largest saved metadata (audience, views) for one Status update.
pub const METADATA_LIMIT: usize = 2 * 1024 * 1024;
/// Streaming limit for one Status attachment, including unknown lengths.
pub const MEDIA_LIMIT: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Viewer {
    pub contact: String,
    pub timestamp: i64,
}

/// A received Status. Protocol keys stay in the account's worker.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    pub sender: String,
    pub name: Option<String>,
    pub timestamp: i64,
    pub content: Content,
    pub background: Option<u32>,
    pub backdrop: Option<Vec<u8>>,
}

/// A small, blurred media backdrop, prepared once outside the drawing loop.
pub fn blurred_backdrop(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() > 1024 * 1024 {
        return None;
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    let picture = reader.decode().ok()?.thumbnail(96, 96).blur(5.0);
    let mut output = std::io::Cursor::new(Vec::new());
    picture
        .write_to(&mut output, image::ImageFormat::Png)
        .ok()?;
    Some(output.into_inner())
}

impl Item {
    /// Separate video playback ids from conversation message ids.
    pub fn playback_id(&self) -> String {
        format!("status:{}:{}", self.sender, self.id)
    }

    /// A future timestamp must not extend an update's lifetime.
    pub fn alive(&self, now: i64) -> bool {
        self.timestamp <= now && now.saturating_sub(self.timestamp) < LIFETIME
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audience_filters_saved_contacts_without_expanding_custom_lists() {
        let contacts = || ["a", "b", "b"].map(str::to_owned);
        let mut audience = Audience {
            mode: AudienceMode::Only,
            selected: ["b".into(), "unknown".into()].into(),
        };
        assert_eq!(audience.recipients(contacts()), vec!["b"]);
        audience.mode = AudienceMode::Except;
        assert_eq!(audience.recipients(contacts()), vec!["a"]);
        audience.mode = AudienceMode::Contacts;
        assert_eq!(audience.recipients(contacts()), vec!["a", "b"]);
        audience.mode = AudienceMode::Only;
        audience.selected.clear();
        assert!(audience.recipients(contacts()).is_empty());
    }

    #[test]
    fn media_backdrops_are_bounded_and_bad_thumbnails_are_ignored() {
        assert!(blurred_backdrop(b"not an image").is_none());
        assert!(blurred_backdrop(&vec![0; 1024 * 1024 + 1]).is_none());
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(800, 600)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let preview = blurred_backdrop(bytes.get_ref()).unwrap();
        let image = image::load_from_memory(&preview).unwrap();
        assert!(image.width() <= 96 && image.height() <= 96);
        assert!(preview.len() < 16 * 1024);
    }

    #[test]
    fn status_lifetime_is_measured_from_publication() {
        let item = Item {
            id: "status".into(),
            sender: "sender".into(),
            name: None,
            timestamp: 100,
            content: Content::Text {
                text: "A day outside".into(),
                preview: None,
            },
            background: None,
            backdrop: None,
        };
        assert!(!item.alive(99));
        assert!(item.alive(100));
        assert!(item.alive(100 + LIFETIME - 1));
        assert!(!item.alive(100 + LIFETIME));
    }
}

/// Local story playback. Pauses never count hidden or loading time.
#[derive(Debug)]
pub struct Playback {
    pub elapsed: std::time::Duration,
    pub duration: std::time::Duration,
    pub paused: bool,
    pub started: bool,
    last: std::time::Instant,
    finished: bool,
    was_running: bool,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            elapsed: std::time::Duration::ZERO,
            duration: std::time::Duration::from_secs(5),
            paused: false,
            started: false,
            last: std::time::Instant::now(),
            finished: false,
            was_running: false,
        }
    }
}

impl Playback {
    /// The story's filled progress segment.
    pub fn fraction(&self) -> f32 {
        (self.elapsed.as_secs_f32() / self.duration.as_secs_f32().max(0.001)).clamp(0.0, 1.0)
    }

    /// Returns true once, when the story reaches its end.
    pub fn advance(&mut self, now: std::time::Instant, running: bool) -> bool {
        if running && self.was_running {
            self.elapsed += now.saturating_duration_since(self.last);
        }
        self.last = now;
        self.was_running = running;
        let ended = !self.finished && self.elapsed >= self.duration;
        self.finished |= ended;
        ended
    }
}

/// Chronological stories within each contact, then the next contact.
pub fn step<'a>(
    items: impl IntoIterator<Item = &'a Item>,
    selected: &(String, String),
    direction: i8,
) -> Option<(String, String)> {
    let items: Vec<_> = items.into_iter().collect();
    let mut authors = std::collections::HashSet::new();
    let mut order = Vec::new();
    for item in &items {
        if authors.insert(&item.sender) {
            order.extend(
                items
                    .iter()
                    .rev()
                    .filter(|other| other.sender == item.sender)
                    .map(|other| (other.sender.clone(), other.id.clone())),
            );
        }
    }
    let index = order.iter().position(|key| key == selected)?;
    let next = index.checked_add_signed(isize::from(direction.signum()))?;
    order.get(next).cloned()
}

#[cfg(test)]
mod playback_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn loading_and_pause_do_not_advance_and_completion_only_fires_once() {
        let start = Instant::now();
        let mut playback = Playback {
            last: start,
            ..Default::default()
        };
        assert!(!playback.advance(start + Duration::from_secs(10), false));
        assert_eq!(playback.elapsed, Duration::ZERO);
        assert!(!playback.advance(start + Duration::from_secs(10), true));
        assert!(!playback.advance(start + Duration::from_secs(12), true));
        assert!(!playback.advance(start + Duration::from_secs(20), false));
        assert_eq!(playback.elapsed, Duration::from_secs(2));
        assert!(!playback.advance(start + Duration::from_secs(20), true));
        assert!(playback.advance(start + Duration::from_secs(23), true));
        assert!(!playback.advance(start + Duration::from_secs(24), true));
    }
    #[test]
    fn stories_advance_chronologically_before_switching_contacts() {
        let make = |sender: &str, id: &str, at| Item {
            id: id.into(),
            sender: sender.into(),
            name: None,
            timestamp: at,
            content: Content::Text {
                text: id.into(),
                preview: None,
            },
            background: None,
            backdrop: None,
        };
        let items = vec![
            make("a", "new", 3),
            make("b", "only", 2),
            make("a", "old", 1),
        ];
        assert_eq!(
            step(&items, &("a".into(), "old".into()), 1),
            Some(("a".into(), "new".into()))
        );
        assert_eq!(
            step(&items, &("a".into(), "new".into()), 1),
            Some(("b".into(), "only".into()))
        );
        assert_eq!(
            step(&items, &("b".into(), "only".into()), -1),
            Some(("a".into(), "new".into()))
        );
        assert_eq!(step(&items, &("b".into(), "only".into()), 1), None);
    }
}
