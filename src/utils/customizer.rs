use std::path::PathBuf;

use crate::app::App;
use crate::settings::{NotificationSound, ThemeChoice};

/// Developer utility for configuring client themes, zoom, wallpapers, and behaviors.
pub struct Customizer<'a> {
    app: &'a mut App,
}

impl<'a> Customizer<'a> {
    pub fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    pub fn theme(&mut self, choice: ThemeChoice) -> &mut Self {
        self.app.settings.theme = choice;
        self
    }

    pub fn dark_mode(&mut self, enabled: bool) -> &mut Self {
        self.app.settings.theme = if enabled {
            ThemeChoice::Dark
        } else {
            ThemeChoice::Light
        };
        self
    }

    pub fn custom_theme(&mut self, theme_name: impl Into<String>) -> &mut Self {
        self.app.settings.custom_theme = Some(theme_name.into());
        self
    }

    pub fn zoom(&mut self, zoom: f32) -> &mut Self {
        self.app.settings.zoom = zoom.clamp(0.5, 3.0);
        self
    }

    pub fn enter_sends(&mut self, enabled: bool) -> &mut Self {
        self.app.settings.enter_sends = enabled;
        self
    }

    pub fn auto_download(&mut self, enabled: bool) -> &mut Self {
        self.app.settings.auto_download = enabled;
        self
    }

    pub fn wallpaper_doodles(&mut self, enabled: bool) -> &mut Self {
        self.app.settings.show_wallpaper = enabled;
        self
    }

    pub fn wallpaper_image(&mut self, path: impl Into<PathBuf>) -> &mut Self {
        self.app.settings.wallpaper_image = Some(path.into());
        self
    }

    pub fn clear_wallpaper_image(&mut self) -> &mut Self {
        self.app.settings.wallpaper_image = None;
        self
    }

    pub fn message_sound(&mut self, sound: NotificationSound) -> &mut Self {
        self.app.settings.message_sound = sound;
        self
    }

    pub fn mention_sound(&mut self, sound: NotificationSound) -> &mut Self {
        self.app.settings.mention_sound = sound;
        self
    }

    pub fn apply(&mut self) {
        self.app.mark_settings_dirty();
    }
}
