//! State and routing for the in-app image preview.

use std::path::{Path, PathBuf};

use egui::{Event, Key, Modifiers, Vec2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenTarget {
    Preview,
    External,
}

/// Chooses the native preview for an image rendered successfully by the conversation view.
pub fn open_target(path: &Path, rendered: bool) -> OpenTarget {
    if rendered && crate::safety::can_preview_image(path) {
        OpenTarget::Preview
    } else {
        OpenTarget::External
    }
}

/// Keyboard command the preview handles while it owns the window.
pub fn preview_action(key: Key, modifiers: Modifiers) -> Option<crate::model::Action> {
    let command = modifiers.command || modifiers.ctrl;
    match (command, key) {
        (true, Key::Plus) | (true, Key::Equals) => Some(crate::model::Action::ZoomImageIn),
        (true, Key::Minus) => Some(crate::model::Action::ZoomImageOut),
        (true, Key::Num0) => Some(crate::model::Action::FitImage),
        (false, Key::Plus) | (false, Key::Equals) if !modifiers.any() => {
            Some(crate::model::Action::ZoomImageIn)
        }
        (false, Key::Minus) if !modifiers.any() => Some(crate::model::Action::ZoomImageOut),
        (false, Key::Num0) if !modifiers.any() => Some(crate::model::Action::FitImage),
        (false, Key::ArrowLeft) if !modifiers.any() => Some(crate::model::Action::PreviousImage),
        (false, Key::ArrowRight) if !modifiers.any() => Some(crate::model::Action::NextImage),
        _ => None,
    }
}

/// Image slots retain their position even while a download is pending.
fn image_slots(message: &crate::model::Message) -> usize {
    match &message.content {
        crate::model::Content::Image { .. } => 1,
        crate::model::Content::Interactive {
            card: Some(card), ..
        } => 1 + card.carousel.len(),
        _ => 0,
    }
}

/// Downloaded path for a photo or an interactive-card image slot.
fn image_path(message: &crate::model::Message, slot: usize) -> Option<&Path> {
    use crate::model::Content;
    let media = match &message.content {
        Content::Image { media, .. } if slot == 0 => Some(media),
        Content::Interactive {
            card: Some(card), ..
        } => {
            if slot == 0 {
                card.image.as_ref()
            } else {
                card.carousel.get(slot - 1)?.image.as_ref()
            }
        }
        _ => None,
    }?;
    media.path.as_deref()
}

#[derive(Clone, Debug, PartialEq)]
struct Cursor {
    message: String,
    index: usize,
    slot: usize,
}

/// The preview swallows keys that would type into or edit the chat behind
/// it, while leaving navigation and activation keys (Tab, Enter, Space,
/// Up/Down) for the preview modal's own controls.
pub fn consumes_key(key: &Event) -> bool {
    match key {
        Event::Text(_) | Event::Paste(_) | Event::Copy | Event::Cut => true,
        Event::Key { key, .. } => !is_modal_navigation(*key),
        _ => false,
    }
}

/// Keys the preview modal needs for focus traversal, activation, and
/// scrolling its own controls.
fn is_modal_navigation(key: Key) -> bool {
    matches!(
        key,
        Key::Tab
            | Key::Enter
            | Key::Space
            | Key::ArrowUp
            | Key::ArrowDown
            | Key::Home
            | Key::End
            | Key::PageUp
            | Key::PageDown
    )
}

/// Fitted image size for a canvas, keeping the aspect ratio and never
/// enlarging past the original pixels. `(0, 0)` original sizes get `(0, 0)`.
pub fn fit_size(width: f32, height: f32, canvas_width: f32, canvas_height: f32) -> (f32, f32) {
    if width <= 0.0 || height <= 0.0 {
        return (0.0, 0.0);
    }
    let scale = (canvas_width / width).min(canvas_height / height).min(1.0);
    (width * scale, height * scale)
}

/// Image size for the zoom level, relative to the original pixels.
pub fn zoomed_size(width: f32, height: f32, zoom: f32) -> (f32, f32) {
    (width * zoom, height * zoom)
}

/// Scroll offset that brings the picture point at `from` to `to` once the
/// picture is resized from `old` to `new`, both points relative to the
/// viewport's top left. With `from == to` the pixel under the pointer stays
/// under it while zooming. The preview centres the picture in content of
/// `viewport.max(size)`, so that is the layout inverted here; the result is
/// clamped to the range the scroll area allows, which keeps a picture
/// narrower or shorter than the viewport centred on that axis.
pub fn anchored_offset(
    viewport: Vec2,
    old: Vec2,
    new: Vec2,
    offset: Vec2,
    from: Vec2,
    to: Vec2,
) -> Vec2 {
    let axis = |d: usize| {
        let origin = |size: f32| (viewport[d].max(size) - size) / 2.0;
        let fraction = if old[d] > 0.0 {
            (offset[d] + from[d] - origin(old[d])) / old[d]
        } else {
            0.5
        };
        let limit = viewport[d].max(new[d]) - viewport[d];
        (origin(new[d]) + fraction * new[d] - to[d]).clamp(0.0, limit)
    };
    vec2(axis(0), axis(1))
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreviewState {
    path: PathBuf,
    chat: Option<String>,
    cursor: Option<Cursor>,
    zoom: f32,
    fit: bool,
    /// Scale the fitted image is drawn at, so zooming starts from what is
    /// on screen rather than from the original pixels.
    fit_scale: f32,
}

impl PreviewState {
    const MIN_ZOOM: f32 = 0.25;
    const MAX_ZOOM: f32 = 4.0;
    pub const ZOOM_STEP: f32 = 1.25;

    /// Opens a fitted preview without conversation navigation.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            chat: None,
            cursor: None,
            zoom: 1.0,
            fit: true,
            fit_scale: 1.0,
        }
    }

    /// Local file currently displayed by the preview.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Opening the preview does not inspect the conversation or count photos.
    pub fn in_conversation(path: PathBuf, chat: Option<String>) -> Self {
        Self {
            chat,
            ..Self::new(path)
        }
    }

    /// Conversation whose loaded messages supply navigation candidates.
    pub fn chat(&self) -> Option<&str> {
        self.chat.as_deref()
    }

    /// Whether the preview has a conversation to browse.
    pub fn can_navigate(&self) -> bool {
        self.chat.is_some()
    }

    /// Moves to the next available photo in either direction, stopping at the ends.
    pub fn navigate(&mut self, messages: &[crate::model::Message], forward: bool) {
        self.navigate_with(messages, forward, |path| path.is_file());
    }

    /// Resolve the anchor only on demand. Its index is reused until paging or
    /// deletion moves it; message identity also distinguishes repeated paths.
    fn navigate_with(
        &mut self,
        messages: &[crate::model::Message],
        forward: bool,
        mut available: impl FnMut(&Path) -> bool,
    ) {
        let cached = self.cursor.as_ref().filter(|cursor| {
            messages.get(cursor.index).is_some_and(|message| {
                message.id == cursor.message
                    && image_path(message, cursor.slot) == Some(self.path())
            })
        });
        let find = |same_message: bool| {
            messages.iter().enumerate().find_map(|(index, message)| {
                if same_message
                    && self
                        .cursor
                        .as_ref()
                        .is_some_and(|cursor| cursor.message != message.id)
                {
                    return None;
                }
                (0..image_slots(message))
                    .find(|&slot| image_path(message, slot) == Some(self.path()))
                    .map(|slot| (index, slot))
            })
        };
        let anchor = cached
            .map(|cursor| (cursor.index, cursor.slot))
            .or_else(|| find(true))
            .or_else(|| find(false));
        let (mut index, mut slot) = if let Some((index, slot)) = anchor {
            self.cursor = Some(Cursor {
                message: messages[index].id.clone(),
                index,
                slot,
            });
            (index, slot)
        } else if let Some(cursor) = &self.cursor {
            // Treat the old position as a gap: the next message now occupies
            // its index, while the previous message is immediately before it.
            (cursor.index.min(messages.len()), 0)
        } else {
            return;
        };
        let mut advance = anchor.is_some() || !forward;
        loop {
            if !advance {
                advance = true;
            } else if forward {
                slot += 1;
                if slot >= image_slots(&messages[index]) {
                    index += 1;
                    slot = 0;
                }
                if index >= messages.len() {
                    return;
                }
            } else if slot > 0 {
                slot -= 1;
            } else {
                if index == 0 {
                    return;
                }
                index -= 1;
                slot = image_slots(&messages[index]).saturating_sub(1);
            }
            if index >= messages.len() {
                return;
            }
            if let Some(path) = image_path(&messages[index], slot)
                && crate::safety::can_preview_image(path)
                && available(path)
            {
                self.path = path.to_owned();
                self.cursor = Some(Cursor {
                    message: messages[index].id.clone(),
                    index,
                    slot,
                });
                self.fit();
                self.fit_scale = 1.0;
                return;
            }
        }
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    pub fn is_fit(&self) -> bool {
        self.fit
    }

    /// The scale on screen: the fitted scale while fitting, else the zoom.
    pub fn scale(&self) -> f32 {
        if self.fit { self.fit_scale } else { self.zoom }
    }

    /// Records the scale the view fitted the image at.
    pub fn set_fit_scale(&mut self, scale: f32) {
        if scale.is_finite() && scale > 0.0 {
            self.fit_scale = scale;
        }
    }

    pub fn zoom_in(&mut self) {
        self.zoom_by(Self::ZOOM_STEP);
    }

    pub fn zoom_out(&mut self) {
        self.zoom_by(1.0 / Self::ZOOM_STEP);
    }

    /// Scales what is on screen by `factor` within the zoom limits. The
    /// limits never reverse the direction: a picture fitted below the
    /// minimum does not grow when zoomed out, so it stays fitted.
    pub fn zoom_by(&mut self, factor: f32) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let scale = self.scale();
        let zoom = if factor > 1.0 {
            (scale * factor).min(Self::MAX_ZOOM).max(scale)
        } else {
            (scale * factor).max(Self::MIN_ZOOM).min(scale)
        };
        if zoom != scale {
            self.zoom = zoom;
            self.fit = false;
        }
    }

    pub fn fit(&mut self) {
        self.fit = true;
        self.zoom = 1.0;
    }

    /// Shows the original pixels at 100%.
    pub fn actual_size(&mut self) {
        self.fit = false;
        self.zoom = 1.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn photo(id: &str, path: &str) -> crate::model::Message {
        use crate::model::{Content, Delivery, Media, MediaState, Message};
        Message {
            id: id.into(),
            chat: "fixture".into(),
            sender: "fixture".into(),
            sender_name: None,
            from_me: false,
            timestamp: 0,
            history_order: None,
            content: Content::Image {
                caption: None,
                media: Media {
                    mime: "image/png".into(),
                    size: 0,
                    width: None,
                    height: None,
                    path: Some(path.into()),
                    state: MediaState::Idle,
                },
            },
            status: Delivery::None,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: Vec::new(),
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        }
    }

    #[test]
    fn navigation_checks_only_candidates_until_the_first_available_photo() {
        let messages = vec![
            photo("a", "a.png"),
            photo("missing", "missing.png"),
            photo("b", "b.png"),
            photo("unvisited", "unvisited.png"),
        ];
        let mut preview = PreviewState::in_conversation("a.png".into(), Some("fixture".into()));
        assert!(
            preview.cursor.is_none(),
            "opening does not resolve the anchor"
        );
        let mut checked = Vec::new();
        preview.navigate_with(&messages, true, |path| {
            checked.push(path.to_owned());
            path != Path::new("missing.png")
        });
        assert_eq!(
            checked,
            [PathBuf::from("missing.png"), PathBuf::from("b.png")]
        );
        assert_eq!(preview.path(), Path::new("b.png"));
        preview.navigate_with(&messages, false, |path| path != Path::new("missing.png"));
        assert_eq!(preview.path(), Path::new("a.png"));
    }

    #[test]
    fn navigation_relocates_after_paging_and_sees_new_downloads_and_messages() {
        let mut messages = vec![
            photo("a", "same.png"),
            photo("b", "b.png"),
            photo("c", "same.png"),
        ];
        let mut preview = PreviewState::in_conversation("b.png".into(), Some("fixture".into()));
        preview.navigate_with(&messages, true, |_| true);
        assert_eq!(preview.cursor.as_ref().unwrap().message, "c");
        messages.insert(0, photo("older", "older.png"));
        preview.navigate_with(&messages, false, |_| true);
        assert_eq!(preview.path(), Path::new("b.png"));
        preview.navigate_with(&messages, true, |_| false);
        assert_eq!(preview.path(), Path::new("b.png"));
        preview.navigate_with(&messages, true, |_| true);
        assert_eq!(preview.cursor.as_ref().unwrap().message, "c");
        preview.navigate_with(&messages, true, |_| true);
        messages.push(photo("new", "new.png"));
        preview.navigate_with(&messages, true, |_| true);
        assert_eq!(preview.path(), Path::new("new.png"));
    }

    #[test]
    fn navigation_recovers_after_the_cursor_message_is_deleted() {
        for forward in [false, true] {
            let mut messages = vec![
                photo("before", "before.png"),
                photo("remaining", "same.png"),
                photo("after", "after.png"),
                photo("deleted", "same.png"),
            ];
            let mut preview =
                PreviewState::in_conversation("after.png".into(), Some("fixture".into()));
            preview.navigate_with(&messages, true, |_| true);
            assert_eq!(preview.cursor.as_ref().unwrap().message, "deleted");
            messages.pop();
            preview.navigate_with(&messages, forward, |_| true);
            let expected = if forward { "after" } else { "before" };
            assert_eq!(preview.path(), Path::new(&format!("{expected}.png")));
            assert_eq!(preview.cursor.as_ref().unwrap().message, expected);
        }
    }

    #[test]
    fn navigation_recovers_after_deletion_of_a_unique_image_path() {
        for deleted_index in 0..3 {
            for forward in [false, true] {
                let mut messages = vec![
                    photo("a", "a.png"),
                    photo("b", "b.png"),
                    photo("c", "c.png"),
                ];
                let path = image_path(&messages[deleted_index], 0).unwrap().to_owned();
                let mut preview =
                    PreviewState::in_conversation(path.clone(), Some("fixture".into()));
                // Resolve the cursor without leaving the chosen image.
                preview.navigate_with(&messages, forward, |_| false);
                messages.remove(deleted_index);
                let expected = if forward {
                    messages.get(deleted_index)
                } else {
                    deleted_index
                        .checked_sub(1)
                        .and_then(|index| messages.get(index))
                };
                preview.navigate_with(&messages, forward, |_| true);
                let expected_path = expected
                    .and_then(|message| image_path(message, 0))
                    .unwrap_or(&path)
                    .to_owned();
                assert_eq!(preview.path(), expected_path);
                messages.clear();
                preview.navigate_with(&messages, forward, |_| true);
                assert_eq!(preview.path(), expected_path);
            }
        }
    }

    #[test]
    fn zooming_from_fit_starts_at_the_fitted_scale() {
        let mut preview = PreviewState::new(PathBuf::from("photo.png"));
        preview.set_fit_scale(0.4);
        preview.zoom_in();
        assert!(!preview.is_fit());
        assert!((preview.zoom() - 0.5).abs() < 1e-6);
        preview.actual_size();
        assert_eq!(preview.zoom(), 1.0);
        preview.fit();
        preview.zoom_out();
        assert!((preview.zoom() - 0.32).abs() < 1e-6);
    }

    #[test]
    fn rendered_supported_images_route_to_the_preview() {
        assert_eq!(
            open_target(Path::new("photo.PNG"), true),
            OpenTarget::Preview
        );
        assert_eq!(
            open_target(Path::new("photo.heic"), true),
            OpenTarget::External
        );
        assert_eq!(
            open_target(Path::new("photo.png"), false),
            OpenTarget::External
        );
    }

    #[test]
    fn preview_keys_map_to_zoom_commands_including_shifted_equals() {
        use egui::{Key, Modifiers};

        assert_eq!(
            preview_action(Key::Equals, Modifiers::COMMAND),
            Some(crate::model::Action::ZoomImageIn)
        );
        assert_eq!(
            preview_action(
                Key::Equals,
                Modifiers {
                    ctrl: true,
                    shift: true,
                    ..Default::default()
                },
            ),
            Some(crate::model::Action::ZoomImageIn)
        );
        assert_eq!(
            preview_action(Key::Minus, Modifiers::NONE),
            Some(crate::model::Action::ZoomImageOut)
        );
        assert_eq!(
            preview_action(Key::Num0, Modifiers::COMMAND),
            Some(crate::model::Action::FitImage)
        );
        assert_eq!(preview_action(Key::Escape, Modifiers::NONE), None);
    }

    #[test]
    fn arrows_navigate_only_without_modifiers_and_are_consumed() {
        for (key, action) in [
            (Key::ArrowLeft, crate::model::Action::PreviousImage),
            (Key::ArrowRight, crate::model::Action::NextImage),
        ] {
            assert_eq!(preview_action(key, Modifiers::NONE), Some(action));
            for modifiers in [
                Modifiers::CTRL,
                Modifiers::ALT,
                Modifiers::SHIFT,
                Modifiers::COMMAND,
            ] {
                assert_eq!(preview_action(key, modifiers), None);
            }
            assert!(consumes_key(&Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE
            }));
        }
    }

    #[test]
    fn the_preview_swallows_chat_input_keys() {
        use egui::{Event, Key, Modifiers};

        assert!(consumes_key(&Event::Text("a".into())));
        assert!(consumes_key(&Event::Copy));
        assert!(consumes_key(&Event::Cut));
        assert!(consumes_key(&Event::Paste("a".into())));
        assert!(consumes_key(&Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }));
        assert!(!consumes_key(&Event::PointerMoved(egui::pos2(1.0, 2.0))));
        for key in [
            Key::Tab,
            Key::Enter,
            Key::Space,
            Key::ArrowDown,
            Key::ArrowUp,
        ] {
            assert!(
                !consumes_key(&Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }),
                "modal navigation key {key:?} must reach the preview"
            );
        }
    }

    #[test]
    fn fit_and_zoom_sizes_keep_the_aspect_ratio() {
        assert_eq!(fit_size(1600.0, 1200.0, 800.0, 700.0), (800.0, 600.0));
        assert_eq!(fit_size(320.0, 240.0, 800.0, 700.0), (320.0, 240.0));
        assert_eq!(fit_size(600.0, 1200.0, 800.0, 700.0), (350.0, 700.0));
        assert_eq!(fit_size(0.0, 0.0, 800.0, 700.0), (0.0, 0.0));
        assert_eq!(zoomed_size(320.0, 240.0, 2.0), (640.0, 480.0));
        assert_eq!(zoomed_size(320.0, 240.0, 0.25), (80.0, 60.0));
    }

    #[test]
    fn preview_starts_fitted_and_zoom_has_sensible_limits() {
        let mut preview = PreviewState::new(PathBuf::from("photo.png"));
        assert_eq!(preview.path(), Path::new("photo.png"));
        assert!(preview.is_fit());

        preview.zoom_in();
        assert!(!preview.is_fit());
        assert_eq!(preview.zoom(), 1.25);
        for _ in 0..20 {
            preview.zoom_in();
        }
        assert_eq!(preview.zoom(), 4.0);
        for _ in 0..40 {
            preview.zoom_out();
        }
        assert_eq!(preview.zoom(), 0.25);

        preview.fit();
        assert!(preview.is_fit());
        assert_eq!(preview.zoom(), 1.0);
    }

    #[test]
    fn zooming_at_the_limits_never_goes_the_wrong_way() {
        let mut preview = PreviewState::new(PathBuf::from("photo.png"));
        preview.set_fit_scale(0.1);
        preview.zoom_by(0.8);
        assert_eq!(preview.scale(), 0.1, "a tiny fit must not jump up");
        assert!(preview.is_fit());
        preview.zoom_by(1.1);
        assert!((preview.zoom() - 0.11).abs() < 1e-6);

        preview.actual_size();
        preview.zoom_by(3.9);
        preview.zoom_by(1.1);
        assert_eq!(preview.zoom(), 4.0);
        preview.zoom_by(0.001);
        assert_eq!(preview.zoom(), 0.25);
    }

    #[test]
    fn a_picture_smaller_than_the_viewport_stays_centred() {
        let offset = anchored_offset(
            vec2(800.0, 600.0),
            vec2(1600.0, 1200.0),
            vec2(400.0, 300.0),
            vec2(500.0, 400.0),
            vec2(10.0, 20.0),
            vec2(10.0, 20.0),
        );
        assert_eq!(offset, Vec2::ZERO);
    }
}
