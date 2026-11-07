use egui::{pos2, vec2, Color32, CornerRadius, Rect, Sense, Vec2};
use crate::theme::Icon;

use crate::theme::{self, Palette};
use crate::ui::widgets::{floating_menu_colors, menu_card_frame};

/// Represents a single item entry in a context menu or dropdown.
pub struct MenuItem {
    pub icon: Option<Icon>,
    pub label: String,
    pub danger: bool,
    pub disabled: bool,
}

impl MenuItem {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            icon: None,
            label: label.into(),
            danger: false,
            disabled: false,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn danger(mut self, danger: bool) -> Self {
        self.danger = danger;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

pub enum MenuEntry {
    Item(MenuItem),
    Separator,
}

/// Declarative builder for menus and floating cards with native WhatsApp styling.
pub struct MenuBuilder {
    entries: Vec<MenuEntry>,
    width: f32,
    row_height: f32,
}

impl Default for MenuBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl MenuBuilder {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            width: 200.0,
            row_height: 36.0,
        }
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn item(mut self, icon: impl Into<Option<Icon>>, label: impl Into<String>) -> Self {
        self.entries.push(MenuEntry::Item(MenuItem {
            icon: icon.into(),
            label: label.into(),
            danger: false,
            disabled: false,
        }));
        self
    }

    pub fn item_danger(mut self, icon: impl Into<Option<Icon>>, label: impl Into<String>) -> Self {
        self.entries.push(MenuEntry::Item(MenuItem {
            icon: icon.into(),
            label: label.into(),
            danger: true,
            disabled: false,
        }));
        self
    }

    pub fn item_disabled(mut self, icon: impl Into<Option<Icon>>, label: impl Into<String>) -> Self {
        self.entries.push(MenuEntry::Item(MenuItem {
            icon: icon.into(),
            label: label.into(),
            danger: false,
            disabled: true,
        }));
        self
    }

    pub fn separator(mut self) -> Self {
        self.entries.push(MenuEntry::Separator);
        self
    }

    /// Renders the menu items and returns the index of the clicked item.
    pub fn show(&self, ui: &mut egui::Ui, palette: &Palette) -> Option<usize> {
        let mut clicked_index = None;
        menu_card_frame(palette).show(ui, |ui| {
            ui.set_width(self.width);
            let mut item_counter = 0;
            for entry in &self.entries {
                match entry {
                    MenuEntry::Separator => {
                        ui.add_space(4.0);
                        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                        ui.painter().rect_filled(rect, CornerRadius::ZERO, palette.outline);
                        ui.add_space(4.0);
                    }
                    MenuEntry::Item(item) => {
                        let current_idx = item_counter;
                        item_counter += 1;
                        let enabled = !item.disabled;
                        let (rect, response) = ui.allocate_exact_size(
                            vec2(ui.available_width(), self.row_height),
                            Sense::click(),
                        );
                        if enabled && response.clicked() {
                            clicked_index = Some(current_idx);
                        }
                        if !ui.is_rect_visible(rect) {
                            continue;
                        }
                        let (_, _, hover_fill) = floating_menu_colors(palette);
                        if response.hovered() && enabled {
                            ui.painter().rect_filled(rect, CornerRadius::same(8), hover_fill);
                        }
                        let text_color = if !enabled {
                            palette.dim
                        } else if item.danger {
                            palette.danger
                        } else {
                            palette.text
                        };

                        let text_start_x = if let Some(icon) = item.icon {
                            let icon_color = if !enabled {
                                palette.dim
                            } else if item.danger {
                                palette.danger
                            } else if palette.dark {
                                Color32::from_gray(235)
                            } else {
                                palette.text
                            };
                            let icon_rect = Rect::from_center_size(
                                pos2(rect.left() + 20.0, rect.center().y),
                                Vec2::splat(18.0),
                            );
                            icon.image(icon_color, 18.0).paint_at(ui, icon_rect);
                            rect.left() + 42.0
                        } else {
                            rect.left() + 16.0
                        };

                        let mut job = egui::text::LayoutJob::simple_singleline(
                            item.label.clone(),
                            theme::regular(14.5),
                            text_color,
                        );
                        job.wrap = egui::text::TextWrapping {
                            max_width: (rect.right() - 12.0 - text_start_x).max(0.0),
                            max_rows: 1,
                            break_anywhere: true,
                            overflow_character: Some('\u{2026}'),
                        };
                        let galley = crate::bidi::layout_job(ui, job);
                        ui.painter().galley(
                            pos2(text_start_x, rect.center().y - galley.size().y * 0.5),
                            galley,
                            text_color,
                        );
                    }
                }
            }
        });
        clicked_index
    }
}
