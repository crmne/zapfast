//! State and routing for the in-app image preview.

use std::path::{Path, PathBuf};

use egui::{Event, Key, Modifiers};

use crate::model::ChatId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenTarget {
    Preview,
    External,
}

/// Whether the full-window viewer can open this file.
///
/// The viewer browses the chat's album, so it takes exactly what the album
/// holds, decided by the same rule the album uses to decide what joins it: a
/// photo it can decode, or a clip it can play. A clip is not an image, so
/// `can_preview_image` alone would send it to the system player; a GIF is an
/// inline animation and is deliberately not in the album, so it is not opened
/// here either.
pub fn can_view(path: &Path) -> bool {
    crate::model::gallery_kind_for_path(path).is_some()
}

/// Whether this file is a clip, from its name alone. Used where the album item
/// is not known yet, so a clip is never handed to the image decoder.
pub fn is_video(path: &Path) -> bool {
    crate::model::gallery_kind_for_path(path) == Some(crate::model::GalleryKind::Video)
}

/// Chooses the native preview for an image rendered successfully by the
/// conversation view.
///
/// The test is `can_view`, not "is an image": the viewer opens what the album
/// holds, so a GIF, which the album leaves out, goes to the system viewer
/// instead of opening a preview that has nothing to show.
pub fn open_target(path: &Path, rendered: bool) -> OpenTarget {
    if rendered && can_view(path) {
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
        // The arrow keys stay with the modal, which needs them to walk its own
        // controls. A viewer's own step keys are the comma and the full stop.
        (false, Key::Comma) if !modifiers.any() => Some(crate::model::Action::ViewerStep(-1)),
        (false, Key::Period) if !modifiers.any() => Some(crate::model::Action::ViewerStep(1)),
        _ => None,
    }
}

/// The preview swallows keys that would type into or edit the chat behind
/// it, while leaving navigation and activation keys (Tab, Enter, Space,
/// arrows) for the preview modal's own controls.
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
            | Key::ArrowLeft
            | Key::ArrowRight
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

#[derive(Clone, Debug, PartialEq)]
pub struct PreviewState {
    /// The file being shown, or `None` for an album item whose attachment is
    /// not downloaded yet. A missing file is not replaced by another one: the
    /// viewer shows what the item is and offers it for download.
    path: Option<PathBuf>,
    zoom: f32,
    fit: bool,
    /// Scale the fitted image is drawn at, so zooming starts from what is
    /// on screen rather than from the original pixels.
    fit_scale: f32,
    /// The chat whose album the viewer browses.
    chat: ChatId,
    /// The message this picture came from.
    message: String,
}

impl PreviewState {
    const MIN_ZOOM: f32 = 0.25;
    const MAX_ZOOM: f32 = 4.0;
    const ZOOM_STEP: f32 = 1.25;

    pub fn new(path: Option<PathBuf>, chat: ChatId, message: String) -> Self {
        Self {
            path,
            chat,
            message,
            zoom: 1.0,
            fit: true,
            fit_scale: 1.0,
        }
    }

    /// The chat whose album the viewer browses.
    pub fn chat(&self) -> &str {
        &self.chat
    }

    /// The message this picture came from.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Where this picture sits in the album, when the album holds it. The
    /// position is derived, so a list that arrives late or changes cannot put
    /// the viewer on the wrong item.
    pub fn position(&self, album: &[crate::archive::ChatMedia]) -> Option<usize> {
        album.iter().position(|item| item.id == self.message)
    }

    /// Whether the viewer is showing a clip. The album item says so when the
    /// album holds this message, and the file's own name otherwise: the same
    /// rule the viewer draws with, so a clip is played and never handed to the
    /// image decoder, and the actions that would decode it are left out.
    pub fn shows_video(&self, album: &[crate::archive::ChatMedia]) -> bool {
        self.position(album)
            .and_then(|index| album.get(index))
            .map_or_else(|| self.path().is_some_and(is_video), |item| item.video)
    }

    /// Points the viewer at another item, back to fitting the window.
    pub fn show_item(&mut self, path: Option<PathBuf>, message: String) {
        self.path = path;
        self.message = message;
        self.fit = true;
        self.zoom = 1.0;
    }

    /// The file on screen, when the item has one.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
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
        self.zoom = (self.scale() * Self::ZOOM_STEP).min(Self::MAX_ZOOM);
        self.fit = false;
    }

    pub fn zoom_out(&mut self) {
        self.zoom = (self.scale() / Self::ZOOM_STEP).max(Self::MIN_ZOOM);
        self.fit = false;
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

    /// A preview of one file, with no album behind it.
    fn fixture() -> PreviewState {
        PreviewState::new(
            Some(PathBuf::from("photo.png")),
            "1@s.whatsapp.net".into(),
            "m1".into(),
        )
    }

    /// One album item, only ever matched by its id here.
    fn item(id: &str) -> crate::archive::ChatMedia {
        crate::archive::ChatMedia {
            id: id.into(),
            timestamp: 0,
            video: false,
            path: None,
            thumbnail: None,
        }
    }

    /// The rule the copy action and its shortcut share: the album item says
    /// whether a clip is on screen when the album holds this message, and the
    /// file's own name says it otherwise.
    #[test]
    fn the_album_item_says_whether_a_clip_is_on_screen() {
        let mut preview = fixture();
        assert!(!preview.shows_video(&[]), "a photo is not a clip");
        preview = PreviewState::new(
            Some(PathBuf::from("clip.mp4")),
            "1@s.whatsapp.net".into(),
            "m1".into(),
        );
        assert!(preview.shows_video(&[]), "the file's own name decides");
        // The album item decides once the album holds this message, and the
        // item is the one this message names.
        let mut clip = item("m1");
        clip.video = true;
        let album = vec![clip, item("m2"), item("m3")];
        assert!(
            preview.shows_video(&album),
            "the album says m1 is a clip, whatever the file name says"
        );
        preview.show_item(Some(PathBuf::from("clip.mp4")), "m2".into());
        assert!(
            !preview.shows_video(&album),
            "and the album says m2 is a photo"
        );
        // A message the album does not hold falls back to the file's name.
        preview.show_item(Some(PathBuf::from("clip.mp4")), "elsewhere".into());
        assert!(preview.shows_video(&album));
    }

    #[test]
    fn the_album_position_comes_from_the_message_id() {
        let mut preview = fixture();
        let album = vec![item("m1"), item("m2"), item("m3")];
        assert_eq!(preview.position(&album), Some(0));
        preview.show_item(Some(PathBuf::from("other.png")), "m3".into());
        assert_eq!(preview.position(&album), Some(2));
        // A picture that is not in the album has no position, so the viewer
        // does not claim to be somewhere it is not.
        preview.show_item(Some(PathBuf::from("other.png")), "nope".into());
        assert_eq!(preview.position(&album), None);
    }

    #[test]
    fn showing_another_item_returns_to_fitting() {
        let mut preview = fixture();
        preview.zoom_in();
        assert!(!preview.is_fit());
        preview.show_item(Some(PathBuf::from("next.png")), "m2".into());
        assert!(preview.is_fit(), "a new picture starts fitted");
        assert_eq!(preview.path(), Some(Path::new("next.png")));
        assert_eq!(preview.message(), "m2");
        // An item whose file is not here yet keeps that: the viewer does not
        // fall back to whatever was on screen.
        preview.show_item(None, "m3".into());
        assert_eq!(preview.path(), None);
        assert_eq!(preview.message(), "m3");
    }

    #[test]
    fn the_viewer_steps_with_its_own_keys() {
        assert_eq!(
            preview_action(Key::Period, Modifiers::NONE),
            Some(crate::model::Action::ViewerStep(1))
        );
        assert_eq!(
            preview_action(Key::Comma, Modifiers::NONE),
            Some(crate::model::Action::ViewerStep(-1))
        );
        // The arrows stay with the modal, which walks its own controls.
        assert_eq!(preview_action(Key::ArrowRight, Modifiers::NONE), None);
    }

    #[test]
    fn zooming_from_fit_starts_at_the_fitted_scale() {
        let mut preview = fixture();
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

    /// The viewer browses the album, so it opens what the album holds: a photo
    /// it can decode, and a clip it can play. A clip is not an image, so the
    /// image test alone would send it to the system player instead, and a GIF
    /// is an inline animation the album leaves out.
    #[test]
    fn the_viewer_opens_photos_and_clips_but_not_anything_else() {
        assert!(can_view(Path::new("photo.png")));
        assert!(can_view(Path::new("clip.mp4")));
        assert!(can_view(Path::new("holiday.MOV")));
        assert!(!can_view(Path::new("anim.gif")));
        assert!(!can_view(Path::new("sticker.webp.png.gif")));
        assert!(!can_view(Path::new("notes.pdf")));
        assert!(!can_view(Path::new("song.mp3")));
        assert!(!can_view(Path::new("no-extension")));
    }

    #[test]
    fn rendered_supported_images_route_to_the_preview() {
        assert_eq!(
            open_target(Path::new("photo.PNG"), true),
            OpenTarget::Preview
        );
        assert_eq!(
            open_target(Path::new("clip.mp4"), true),
            OpenTarget::Preview
        );
        // A GIF is an inline animation the album leaves out, so the preview has
        // nothing to show for it and it goes to the system viewer.
        assert_eq!(
            open_target(Path::new("anim.gif"), true),
            OpenTarget::External
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

    /// A clip opened before the album arrives has no item to ask, so the file
    /// name decides: handing an mp4 to the image decoder showed a load error.
    #[test]
    fn a_file_name_says_whether_it_is_a_clip() {
        assert!(is_video(Path::new("clip.mp4")));
        assert!(is_video(Path::new("holiday.MOV")));
        assert!(!is_video(Path::new("photo.png")));
        assert!(!is_video(Path::new("anim.gif")));
        assert!(!is_video(Path::new("no-extension")));
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
        let mut preview = fixture();
        assert_eq!(preview.path(), Some(Path::new("photo.png")));
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
}
