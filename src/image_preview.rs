//! State and routing for the in-app image preview.

use std::path::{Path, PathBuf};

use egui::{Event, Key, Modifiers};

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
pub fn preview_action(
    key: Key,
    modifiers: Modifiers,
) -> Option<crate::model::Action> {
    let command = modifiers.command || modifiers.ctrl;
    match (command, key) {
        (true, Key::Plus) | (true, Key::Equals) => {
            Some(crate::model::Action::ZoomImageIn)
        }
        (true, Key::Minus) => Some(crate::model::Action::ZoomImageOut),
        (true, Key::Num0) => Some(crate::model::Action::FitImage),
        (false, Key::Plus) | (false, Key::Equals) if !modifiers.any() => {
            Some(crate::model::Action::ZoomImageIn)
        }
        (false, Key::Minus) if !modifiers.any() => Some(crate::model::Action::ZoomImageOut),
        (false, Key::Num0) if !modifiers.any() => Some(crate::model::Action::FitImage),
        _ => None,
    }
}

/// The preview swallows input keys while it is open, so a keystroke cannot
/// reach the chat behind it.
pub fn consumes_key(key: &Event) -> bool {
    matches!(
        key,
        Event::Key { .. } | Event::Text(_) | Event::Paste(_) | Event::Copy | Event::Cut
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

/// Requests enough source pixels for the preview's current zoom level.
pub fn texture_size(canvas: egui::Vec2, fit: bool, zoom: f32) -> egui::Vec2 {
    if fit {
        canvas
    } else {
        canvas * zoom.max(0.0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreviewState {
    path: PathBuf,
    zoom: f32,
    fit: bool,
}

impl PreviewState {
    const MIN_ZOOM: f32 = 0.25;
    const MAX_ZOOM: f32 = 4.0;
    const ZOOM_STEP: f32 = 1.25;

    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            zoom: 1.0,
            fit: true,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    pub fn is_fit(&self) -> bool {
        self.fit
    }

    pub fn zoom_in(&mut self) {
        self.fit = false;
        self.zoom = (self.zoom * Self::ZOOM_STEP).min(Self::MAX_ZOOM);
    }

    pub fn zoom_out(&mut self) {
        self.fit = false;
        self.zoom = (self.zoom / Self::ZOOM_STEP).max(Self::MIN_ZOOM);
    }

    pub fn fit(&mut self) {
        self.fit = true;
        self.zoom = 1.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

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
    fn preview_keys_map_to_zoom_commands_and_ignore_text_fields() {
        use egui::{Event, Key, Modifiers};

        assert_eq!(
            preview_action(Key::Equals, Modifiers::COMMAND),
            Some(crate::model::Action::ZoomImageIn)
        );
        assert_eq!(
            preview_action(Key::Equals, Modifiers { ctrl: true, shift: true, ..Default::default() }),
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
    }

    #[test]
    fn fit_and_zoom_sizes_keep_the_aspect_ratio() {
        assert_eq!(fit_size(1600.0, 1200.0, 800.0, 700.0), (800.0, 600.0));
        assert_eq!(fit_size(320.0, 240.0, 800.0, 700.0), (320.0, 240.0));
        assert_eq!(fit_size(600.0, 1200.0, 800.0, 700.0), (350.0, 700.0));
        assert_eq!(fit_size(0.0, 0.0, 800.0, 700.0), (0.0, 0.0));
        assert_eq!(zoomed_size(320.0, 240.0, 2.0), (640.0, 480.0));
        assert_eq!(zoomed_size(320.0, 240.0, 0.25), (80.0, 60.0));
        assert_eq!(texture_size(egui::vec2(800.0, 600.0), true, 4.0), egui::vec2(800.0, 600.0));
        assert_eq!(texture_size(egui::vec2(800.0, 600.0), false, 2.0), egui::vec2(1600.0, 1200.0));
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
}
