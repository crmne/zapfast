//! Remembered window size, position and maximized state.
//!
//! Zoom already lives in [`crate::settings::Settings`]; this module keeps the
//! rest of the window state together: the [`Geometry`] value stored in the
//! settings, the per-frame [`Snapshot`] read from egui, and the viewport
//! builder applied at startup.

/// The window's geometry as remembered in [`crate::settings::Settings`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Geometry {
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub maximized: bool,
}

impl Geometry {
    /// Whether `value` is a usable window dimension.
    fn sane_size(value: f32) -> bool {
        value.is_finite() && value > 0.0 && value <= 16_384.0
    }

    /// Whether `value` is a usable window coordinate.
    fn sane_position(value: f32) -> bool {
        value.is_finite() && value.abs() <= 32_768.0
    }

    /// The remembered inner size, if both dimensions are usable.
    pub fn size(self) -> Option<[f32; 2]> {
        match (self.width, self.height) {
            (Some(width), Some(height)) if Self::sane_size(width) && Self::sane_size(height) => {
                Some([width, height])
            }
            _ => None,
        }
    }

    /// The remembered position, if both coordinates are usable.
    pub fn position(self) -> Option<[f32; 2]> {
        match (self.x, self.y) {
            (Some(x), Some(y)) if Self::sane_position(x) && Self::sane_position(y) => Some([x, y]),
            _ => None,
        }
    }
}

/// One frame's viewport values. `screen` is the content-size fallback where
/// the viewport carries no position (Wayland).
#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub inner: Option<egui::Rect>,
    pub outer: Option<egui::Rect>,
    pub screen: Option<egui::Rect>,
    pub maximized: Option<bool>,
    pub fullscreen: Option<bool>,
    pub minimized: Option<bool>,
}

impl Snapshot {
    /// Reads the current viewport values from egui.
    pub fn read(ctx: &egui::Context) -> Self {
        ctx.input(|input| {
            let viewport = input.viewport();
            Self {
                inner: viewport.inner_rect,
                outer: viewport.outer_rect,
                screen: input.raw.screen_rect,
                maximized: viewport.maximized,
                fullscreen: viewport.fullscreen,
                minimized: viewport.minimized,
            }
        })
    }

    /// Folds the snapshot into the settings. Returns whether anything
    /// changed. Skipped while minimized or fullscreen, which carry no normal
    /// geometry. The position stays untouched where the compositor owns it
    /// (Wayland), while the size is still remembered through the screen rect.
    pub fn remember(self, settings: &mut crate::settings::Settings) -> bool {
        if self.minimized == Some(true) || self.fullscreen == Some(true) {
            return false;
        }
        let mut changed = false;
        if self.maximized == Some(true) {
            if !settings.window_maximized {
                settings.window_maximized = true;
                changed = true;
            }
            return changed;
        }
        if settings.window_maximized {
            settings.window_maximized = false;
            changed = true;
        }
        let size = self
            .inner
            .map(|rect| rect.size())
            .or_else(|| self.screen.map(|rect| rect.size()));
        if let Some(size) = size
            && size.x.is_finite()
            && size.y.is_finite()
            && size.x > 0.0
            && size.y > 0.0
            && (settings
                .window_width
                .is_none_or(|w| (w - size.x).abs() > 0.5)
                || settings
                    .window_height
                    .is_none_or(|h| (h - size.y).abs() > 0.5))
        {
            settings.window_width = Some(size.x);
            settings.window_height = Some(size.y);
            changed = true;
        }
        // `with_position` places the outer frame everywhere but on macOS,
        // where it places the content; mirror that when remembering. A
        // missing position (Wayland) leaves the last one alone.
        let pos = match (self.inner, self.outer) {
            #[cfg(target_os = "macos")]
            (Some(inner), _) => Some(inner.min),
            #[cfg(target_os = "macos")]
            (None, Some(outer)) => Some(outer.min),
            #[cfg(not(target_os = "macos"))]
            (_, Some(outer)) => Some(outer.min),
            #[cfg(not(target_os = "macos"))]
            (Some(inner), None) => Some(inner.min),
            (None, None) => None,
        };
        if let Some(pos) = pos
            && pos.x.is_finite()
            && pos.y.is_finite()
            && (settings.window_x.is_none_or(|x| (x - pos.x).abs() > 0.5)
                || settings.window_y.is_none_or(|y| (y - pos.y).abs() > 0.5))
        {
            settings.window_x = Some(pos.x);
            settings.window_y = Some(pos.y);
            changed = true;
        }
        changed
    }
}

/// Applies remembered geometry to a viewport builder, falling back to
/// `default_size` without a usable remembered size.
pub fn viewport(
    builder: egui::ViewportBuilder,
    geometry: Geometry,
    default_size: [f32; 2],
) -> egui::ViewportBuilder {
    let builder = builder.with_inner_size(geometry.size().unwrap_or(default_size));
    let builder = match geometry.position() {
        Some(position) => builder.with_position(position),
        None => builder,
    };
    if geometry.maximized {
        builder.with_maximized(true)
    } else {
        builder
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    fn rects(size: [f32; 2], pos: [f32; 2]) -> (egui::Rect, egui::Rect, egui::Rect) {
        let inner =
            egui::Rect::from_min_size(egui::pos2(pos[0], pos[1]), egui::vec2(size[0], size[1]));
        let outer = egui::Rect::from_min_size(
            egui::pos2(pos[0] - 4.0, pos[1] - 4.0),
            egui::vec2(size[0] + 8.0, size[1] + 8.0),
        );
        (inner, outer, inner)
    }

    fn snapshot(
        inner: Option<egui::Rect>,
        outer: Option<egui::Rect>,
        screen: Option<egui::Rect>,
        maximized: Option<bool>,
        fullscreen: Option<bool>,
        minimized: Option<bool>,
    ) -> Snapshot {
        Snapshot {
            inner,
            outer,
            screen,
            maximized,
            fullscreen,
            minimized,
        }
    }

    #[test]
    fn the_window_geometry_defaults_to_unset_and_round_trips() {
        assert_eq!(Settings::default().window_geometry(), Geometry::default());
        assert_eq!(Geometry::default().size(), None);
        assert_eq!(Geometry::default().position(), None);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let remembered = Settings {
            zoom: 1.25,
            window_width: Some(1280.0),
            window_height: Some(800.0),
            window_x: Some(100.0),
            window_y: Some(50.0),
            window_maximized: true,
            ..Settings::default()
        };
        remembered.save(&path).unwrap();
        assert_eq!(Settings::load(&path), remembered);
        let geometry = remembered.window_geometry();
        assert_eq!(geometry.size(), Some([1280.0, 800.0]));
        assert_eq!(geometry.position(), Some([100.0, 50.0]));
        assert!(geometry.maximized);
    }

    #[test]
    fn unusable_window_geometry_is_ignored() {
        for geometry in [
            Geometry {
                width: Some(f32::NAN),
                height: Some(800.0),
                ..Geometry::default()
            },
            Geometry {
                width: Some(0.0),
                height: Some(800.0),
                ..Geometry::default()
            },
            Geometry {
                width: Some(1280.0),
                height: Some(f32::INFINITY),
                ..Geometry::default()
            },
            Geometry {
                width: Some(1280.0),
                height: None,
                ..Geometry::default()
            },
        ] {
            assert_eq!(geometry.size(), None);
        }
        for geometry in [
            Geometry {
                x: Some(f32::NAN),
                y: Some(50.0),
                ..Geometry::default()
            },
            Geometry {
                x: Some(100.0),
                y: None,
                ..Geometry::default()
            },
        ] {
            assert_eq!(geometry.position(), None);
        }
        // Files written before the window was remembered open with the
        // default size.
        let older: Settings = serde_json::from_str(r#"{"zoom":1.25}"#).unwrap();
        assert_eq!(older.window_geometry(), Geometry::default());
    }

    /// The window's size and position land in the settings, so the next
    /// window opens where this one is.
    #[test]
    fn the_window_remembers_its_size_and_position() {
        let mut settings = Settings::default();
        let (inner, outer, screen) = rects([1280.0, 800.0], [100.0, 50.0]);
        assert!(
            snapshot(Some(inner), Some(outer), Some(screen), None, None, None)
                .remember(&mut settings)
        );
        assert_eq!(settings.window_width, Some(1280.0));
        assert_eq!(settings.window_height, Some(800.0));
        #[cfg(not(target_os = "macos"))]
        assert_eq!(
            (settings.window_x, settings.window_y),
            (Some(96.0), Some(46.0))
        );
        #[cfg(target_os = "macos")]
        assert_eq!(
            (settings.window_x, settings.window_y),
            (Some(100.0), Some(50.0))
        );
        assert!(!settings.window_maximized);

        // A sub-pixel move is the window manager settling, not the reader.
        let (inner, outer, screen) = rects([1280.2, 800.2], [100.2, 50.2]);
        assert!(
            !snapshot(Some(inner), Some(outer), Some(screen), None, None, None)
                .remember(&mut settings)
        );

        // A real move is remembered.
        let (inner, outer, screen) = rects([1400.0, 900.0], [200.0, 120.0]);
        assert!(
            snapshot(Some(inner), Some(outer), Some(screen), None, None, None)
                .remember(&mut settings)
        );
        assert_eq!(settings.window_width, Some(1400.0));
    }

    /// Maximizing remembers the state without losing the normal size, and
    /// minimizing or fullscreen leaves everything alone.
    #[test]
    fn maximizing_minimizing_and_fullscreen_keep_the_normal_geometry() {
        let mut settings = Settings::default();
        let (inner, outer, screen) = rects([1280.0, 800.0], [100.0, 50.0]);
        assert!(
            snapshot(Some(inner), Some(outer), Some(screen), None, None, None)
                .remember(&mut settings)
        );

        assert!(
            snapshot(
                Some(inner),
                Some(outer),
                Some(screen),
                Some(true),
                None,
                None
            )
            .remember(&mut settings)
        );
        assert!(settings.window_maximized);
        assert_eq!(settings.window_width, Some(1280.0));

        // Restoring clears the flag and remembers the size behind it.
        assert!(
            snapshot(
                Some(inner),
                Some(outer),
                Some(screen),
                Some(false),
                None,
                None
            )
            .remember(&mut settings)
        );
        assert!(!settings.window_maximized);

        let (maximized_inner, _, _) = rects([1920.0, 1080.0], [0.0, 0.0]);
        assert!(
            !snapshot(Some(maximized_inner), None, None, None, None, Some(true))
                .remember(&mut settings)
        );
        assert!(
            !snapshot(Some(maximized_inner), None, None, None, Some(true), None)
                .remember(&mut settings)
        );
    }

    /// Without a viewport position (Wayland) the size is still remembered
    /// while the last position is kept.
    #[test]
    fn a_window_without_a_position_keeps_the_last_one() {
        let mut settings = Settings {
            window_x: Some(100.0),
            window_y: Some(50.0),
            ..Settings::default()
        };
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
        assert!(snapshot(None, None, Some(screen), None, None, None).remember(&mut settings));
        assert_eq!(settings.window_width, Some(1280.0));
        assert_eq!(settings.window_height, Some(800.0));
        assert_eq!(
            (settings.window_x, settings.window_y),
            (Some(100.0), Some(50.0))
        );
    }

    #[test]
    fn the_viewport_uses_the_remembered_geometry_or_the_default() {
        let remembered = Geometry {
            width: Some(1280.0),
            height: Some(800.0),
            x: Some(100.0),
            y: Some(50.0),
            maximized: true,
        };
        let built = viewport(
            egui::ViewportBuilder::default(),
            remembered,
            [1180.0, 780.0],
        );
        assert_eq!(built.inner_size, Some(egui::vec2(1280.0, 800.0)));
        assert_eq!(built.position, Some(egui::pos2(100.0, 50.0)));
        assert!(built.maximized.unwrap_or(false));

        let built = viewport(
            egui::ViewportBuilder::default(),
            Geometry::default(),
            [1180.0, 780.0],
        );
        assert_eq!(built.inner_size, Some(egui::vec2(1180.0, 780.0)));
        assert_eq!(built.position, None);
        assert!(!built.maximized.unwrap_or(false));
    }
}
