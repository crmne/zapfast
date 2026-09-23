//! Shared avatars, fields, menus, and badges.

use std::path::Path;

use egui::{
    Align, Color32, CornerRadius, Layout, Rect, Sense, Stroke, Ui, UiBuilder, Vec2, pos2, vec2,
};

use crate::bidi;
use crate::emoji;
use crate::model::Delivery;
use crate::theme::{self, Icon, Palette};

/// Laid-out text and its color emoji placements.
pub struct Line {
    pub galley: std::sync::Arc<egui::Galley>,
    placements: Vec<String>,
    accessible_text: String,
}

impl Line {
    pub fn size(&self) -> Vec2 {
        self.galley.size()
    }

    pub fn paint(&self, ui: &Ui, pos: egui::Pos2, fallback: Color32) {
        let response = ui.interact(
            Rect::from_min_size(pos, self.size()),
            ui.id()
                .with(("painted-text", pos.x.to_bits(), pos.y.to_bits())),
            Sense::hover(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Label,
                ui.is_enabled(),
                &self.accessible_text,
            )
        });
        ui.painter().galley(pos, self.galley.clone(), fallback);
        emoji::paint(ui, &self.galley, pos, &self.placements);
    }
}

/// Lays out text within `width` and `max_rows`, with an ellipsis and color emoji.
pub fn line(
    ui: &Ui,
    text: &str,
    font: egui::FontId,
    color: Color32,
    width: f32,
    max_rows: usize,
) -> Line {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = width;
    job.wrap.max_rows = max_rows;
    // Break anywhere for single-line ellipsis; wrap multi-line text at words.
    job.wrap.break_anywhere = max_rows == 1;
    job.wrap.overflow_character = Some('…');
    let mut placements = Vec::new();
    let format = egui::TextFormat::simple(font, color);
    let single = text.lines().next().unwrap_or_default();
    emoji::append(
        &mut job,
        &mut placements,
        if max_rows == 1 { single } else { text },
        &format,
    );
    let galley = bidi::layout_job(ui, job);
    Line {
        galley,
        placements,
        accessible_text: text.to_owned(),
    }
}

/// Allocates one truncated line with color emoji.
pub fn rich_text(ui: &mut Ui, text: &str, font: egui::FontId, color: Color32) -> egui::Response {
    let width = ui.available_width().max(1.0);
    let line = line(ui, text, font, color, width, 1);
    let (rect, response) = ui.allocate_exact_size(line.size(), Sense::hover());
    if ui.is_rect_visible(rect) {
        line.paint(ui, rect.min, color);
    }
    response
}

/// Selectable version of [`rich_text`].
pub fn selectable_rich_text(
    ui: &mut Ui,
    text: &str,
    font: egui::FontId,
    color: Color32,
) -> egui::Response {
    let width = ui.available_width().max(1.0);
    let line = line(ui, text, font, color, width, 1);
    let (rect, response) = ui.allocate_exact_size(line.size(), Sense::CLICK | Sense::DRAG);
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), text));
    // Register emoji placements so copied text restores the original sequences.
    if let Some(rows) = ui.ctx().data(|data| {
        data.get_temp::<std::sync::Arc<std::sync::Mutex<Vec<crate::transcript::Row>>>>(
            egui::Id::new("copy-rows"),
        )
    }) {
        rows.lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(crate::transcript::Row {
                header: String::new(),
                body: line.galley.text().to_owned(),
                placements: line.placements.clone(),
                ..Default::default()
            });
    }
    if ui.is_rect_visible(rect) {
        egui::text_selection::LabelSelectionState::label_text_selection(
            ui,
            &response,
            rect.min,
            line.galley.clone(),
            color,
            egui::Stroke::NONE,
        );
        crate::emoji::paint(ui, &line.galley, rect.min, &line.placements);
    }
    response
}

/// An image for a local file, registered so egui's caches for it can be
/// released once it leaves the screen.
pub fn file_image(ui: &Ui, path: &Path) -> egui::Image<'static> {
    let uri = crate::util::image_uri(path);
    crate::image_cache::touch(ui.ctx(), &uri);
    egui::Image::new(uri)
}

/// Round profile picture, or id-colored initials when no picture is available.
pub fn avatar(
    ui: &mut Ui,
    palette: &Palette,
    name: &str,
    id: &str,
    size: f32,
    picture: Option<&Path>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if ui.is_rect_visible(rect) {
        paint_avatar(ui, palette, rect, name, id, picture);
    }
    response
}

/// An avatar that acts as a button. It must be created clickable: first
/// creating it for hover and then calling `interact` registers the same id
/// twice, and the unfocusable first registration makes egui drop keyboard
/// focus from it on every frame.
pub fn clickable_avatar(
    ui: &mut Ui,
    palette: &Palette,
    name: &str,
    id: &str,
    size: f32,
    picture: Option<&Path>,
    label: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    theme::reveal_focus(&response);
    theme::focus_outline(ui, response.id, rect, size / 2.0);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    if ui.is_rect_visible(rect) {
        paint_avatar(ui, palette, rect, name, id, picture);
    }
    response
}

pub fn paint_avatar(
    ui: &Ui,
    palette: &Palette,
    rect: Rect,
    name: &str,
    id: &str,
    picture: Option<&Path>,
) {
    let size = rect.width();
    let mut painted = false;
    if let Some(picture) = picture {
        let image = file_image(ui, picture)
            .fit_to_exact_size(Vec2::splat(size))
            .corner_radius(size / 2.0);
        if let Ok(egui::load::TexturePoll::Ready { .. }) =
            image.load_for_size(ui.ctx(), Vec2::splat(size))
        {
            image.paint_at(ui, rect);
            painted = true;
        }
    }
    if !painted {
        let fill = palette.avatar(crate::util::hue(id));
        ui.painter().circle_filled(rect.center(), size / 2.0, fill);
        if crate::model::ChatKind::from_id(id) == crate::model::ChatKind::Group {
            theme::paint_icon(ui, Icon::Users, rect, size * 0.5, Color32::WHITE);
        } else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                crate::util::initials(name),
                theme::semibold(size * 0.38),
                Color32::WHITE,
            );
        }
    }
}

pub fn paint_disappearing_badge(ui: &Ui, palette: &Palette, avatar: Rect) {
    let size = (avatar.width() * 0.38).clamp(14.0, 18.0);
    let rect = Rect::from_center_size(
        pos2(avatar.right() - size * 0.15, avatar.bottom() - size * 0.15),
        Vec2::splat(size),
    );
    ui.painter()
        .circle_filled(rect.center(), size * 0.58, palette.surface);
    theme::paint_icon(ui, Icon::Timer, rect, size, palette.accent);
}

/// Outgoing-message status ticks.
pub fn ticks(ui: &Ui, palette: &Palette, rect: Rect, status: Delivery) {
    let (icon, color) = match status {
        Delivery::None => return,
        Delivery::Pending => (Icon::Clock, palette.secondary),
        Delivery::Sent => (Icon::Check, palette.secondary),
        Delivery::Delivered => (Icon::CheckCheck, palette.secondary),
        Delivery::Read | Delivery::Played => (Icon::CheckCheck, palette.read),
        Delivery::Failed => (Icon::CircleAlert, palette.danger),
    };
    theme::paint_icon(ui, icon, rect, rect.height(), color);
}

/// Chat-row unread badge.
pub fn badge(ui: &Ui, palette: &Palette, at: egui::Pos2, count: u32, muted: bool) -> f32 {
    let label = if count > 99 {
        "99+".to_owned()
    } else {
        count.to_string()
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label, theme::semibold(11.0), palette.on_accent);
    let width = (galley.size().x + 12.0).max(20.0);
    let rect = Rect::from_center_size(at, vec2(width, 20.0));
    let fill = if muted { palette.dim } else { palette.accent };
    ui.painter().rect_filled(rect, 10.0, fill);
    ui.painter().galley(
        rect.center() - galley.size() / 2.0,
        galley,
        palette.on_accent,
    );
    width
}

/// Minimum width needed for menu labels.
pub fn menu_width(ui: &Ui, labels: &[&str], icons: bool) -> f32 {
    let widest = labels
        .iter()
        .map(|label| {
            ui.painter()
                .layout_no_wrap(label.to_string(), theme::regular(13.5), Color32::WHITE)
                .size()
                .x
        })
        .fold(0.0, f32::max);
    widest + if icons { 26.0 } else { 0.0 } + 20.0 + 12.0
}

pub fn menu_item(ui: &mut Ui, palette: &Palette, icon: Option<Icon>, label: &str) -> bool {
    menu_item_enabled(ui, palette, icon, label, true)
}

pub fn menu_item_enabled(
    ui: &mut Ui,
    palette: &Palette,
    icon: Option<Icon>,
    label: &str,
    enabled: bool,
) -> bool {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        vec2(width, 28.0),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    theme::reveal_focus(&response);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled && ui.is_enabled(), label)
    });
    if ui.is_rect_visible(rect) {
        if response.hovered() && enabled {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(6), palette.surface_hover);
        }
        let color = if enabled { palette.text } else { palette.dim };
        let mut x = rect.left() + 10.0;
        if let Some(icon) = icon {
            let icon_rect =
                Rect::from_center_size(pos2(x + 8.0, rect.center().y), Vec2::splat(16.0));
            icon.image(
                if enabled {
                    palette.secondary
                } else {
                    palette.dim
                },
                16.0,
            )
            .paint_at(ui, icon_rect);
            x += 26.0;
        }
        let mut job = egui::text::LayoutJob::simple_singleline(
            label.to_string(),
            theme::regular(13.5),
            color,
        );
        job.wrap = egui::text::TextWrapping {
            max_width: (rect.right() - 10.0 - x).max(0.0),
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('\u{2026}'),
        };
        let galley = crate::bidi::layout_job(ui, job);
        ui.painter().galley(
            pos2(x, rect.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
    }
    let clicked = enabled && response.clicked();
    if clicked {
        ui.close();
    }
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    clicked
}

/// A menu row that only informs: no hover, no pointer, not a button. It is
/// set apart by its smaller type and dimmed icon; the text keeps full contrast.
pub fn menu_info(ui: &mut Ui, palette: &Palette, icon: Icon, label: &str) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label));
    if !ui.is_rect_visible(rect) {
        return;
    }
    let icon_rect =
        Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), Vec2::splat(14.0));
    icon.image(palette.dim, 14.0).paint_at(ui, icon_rect);
    let mut job = egui::text::LayoutJob::simple_singleline(
        label.to_string(),
        theme::regular(12.5),
        palette.text,
    );
    job.wrap = egui::text::TextWrapping {
        max_width: (rect.right() - 10.0 - (rect.left() + 36.0)).max(0.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('\u{2026}'),
    };
    let galley = crate::bidi::layout_job(ui, job);
    ui.painter().galley(
        pos2(rect.left() + 36.0, rect.center().y - galley.size().y / 2.0),
        galley,
        palette.text,
    );
}

pub fn menu_separator(ui: &mut Ui, palette: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
    ui.painter().hline(
        rect.x_range().shrink(6.0),
        rect.center().y,
        Stroke::new(1.0, palette.outline),
    );
}

/// Shared popup-menu frame.
pub fn menu_frame(palette: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(palette.overlay)
        .stroke(Stroke::new(1.0, palette.outline))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(egui::Margin::same(6))
        .shadow(egui::epaint::Shadow {
            offset: [0, 6],
            blur: 20,
            spread: 0,
            color: palette.shadow,
        })
}

pub fn empty_state(ui: &mut Ui, palette: &Palette, icon: Icon, title: &str, body: &str) {
    ui.add_space(48.0);
    ui.vertical_centered(|ui| {
        theme::icon(ui, icon, 40.0, palette.dim);
        ui.add_space(8.0);
        theme::text(ui, title, theme::semibold(16.0), palette.text);
        ui.add_space(2.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(body)
                    .font(theme::regular(13.5))
                    .color(palette.secondary),
            )
            .wrap()
            .selectable(false),
        );
    });
}

/// Search field with icon and clear button.
pub fn search_field(
    ui: &mut Ui,
    palette: &Palette,
    id: egui::Id,
    text: &mut String,
    hint: &str,
    width: f32,
) -> egui::Response {
    let height = 34.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let has_focus = ui.memory(|memory| memory.has_focus(id));
    let fill = if has_focus {
        palette.surface_hover
    } else {
        palette.surface
    };
    ui.painter().rect_filled(rect, height / 2.0, fill);
    let icon_rect =
        Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), Vec2::splat(16.0));
    Icon::Search
        .image(palette.secondary, 16.0)
        .paint_at(ui, icon_rect);
    let field_rect = Rect::from_min_max(
        pos2(rect.left() + 34.0, rect.top() + 1.0),
        pos2(rect.right() - 30.0, rect.bottom() - 1.0),
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(field_rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    let response = child.add(
        egui::TextEdit::singleline(text)
            .id(id)
            .hint_text(
                egui::RichText::new(hint)
                    .color(palette.dim)
                    .font(theme::regular(14.0)),
            )
            .font(theme::regular(14.0))
            .text_color(palette.text)
            .frame(egui::Frame::NONE)
            .desired_width(field_rect.width())
            .vertical_align(Align::Center),
    );
    theme::focus_outline(ui, response.id, rect, height / 2.0);
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_label(hint));
    if !text.is_empty() {
        let clear_rect = Rect::from_center_size(
            pos2(rect.right() - 17.0, rect.center().y),
            Vec2::splat(24.0),
        );
        let mut clear = ui.new_child(
            UiBuilder::new()
                .max_rect(clear_rect)
                .layout(Layout::centered_and_justified(egui::Direction::LeftToRight)),
        );
        if theme::icon_button(
            &mut clear,
            Icon::X,
            15.0,
            palette.secondary,
            palette.text,
            "Clear",
        )
        .clicked()
        {
            text.clear();
            ui.memory_mut(|memory| memory.request_focus(id));
        }
    }
    response
}

/// Switch control.
/// Switch that shows its state by shape as well as colour: off is an
/// outlined track with a small grey knob, on a filled track with a larger
/// white knob carrying a check (WCAG 1.4.1).
pub fn switch(ui: &mut Ui, palette: &Palette, on: &mut bool) -> egui::Response {
    let size = vec2(40.0, 22.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool(response.id, *on);
        let fill = egui::lerp(
            egui::Rgba::from(palette.surface_active)..=egui::Rgba::from(palette.accent),
            t,
        );
        let radius = rect.height() / 2.0;
        ui.painter().rect_filled(rect, radius, Color32::from(fill));
        // The outline keeps the off track visible on a light panel.
        if t < 1.0 {
            ui.painter().rect_stroke(
                rect,
                radius,
                Stroke::new(1.5, palette.secondary.gamma_multiply(1.0 - t)),
                egui::StrokeKind::Inside,
            );
        }
        let center = pos2(
            egui::lerp(rect.left() + 11.0..=rect.right() - 11.0, t),
            rect.center().y,
        );
        let knob = egui::lerp(
            egui::Rgba::from(palette.secondary)..=egui::Rgba::from(Color32::WHITE),
            t,
        );
        ui.painter()
            .circle_filled(center, egui::lerp(6.0..=8.0, t), Color32::from(knob));
        if t > 0.5 {
            let check = Rect::from_center_size(center, Vec2::splat(12.0));
            Icon::Check
                .image(palette.accent.gamma_multiply((t - 0.5) * 2.0), 12.0)
                .paint_at(ui, check);
        }
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The author's website, linked from the credit line.
pub const AUTHOR_URL: &str = "https://paolino.me";

/// "Built with love by Carmine Paolino", with the name linking to
/// [`AUTHOR_URL`]. Returns whether the name was clicked.
pub fn credit(ui: &mut Ui, palette: &Palette, locale: crate::i18n::Locale) -> bool {
    // Translators: {name} is replaced by the author's name, shown as a link.
    let sentence = crate::i18n::gettext(locale, "Built with love by {name}");
    let (before, after) = sentence.split_once("{name}").unwrap_or((&sentence, ""));
    let mut clicked = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        theme::text(ui, "\u{2665}  ", theme::regular(13.0), palette.danger);
        theme::text(ui, before, theme::regular(13.0), palette.secondary);
        clicked = theme::link(ui, "Carmine Paolino", theme::medium(13.0), palette.link)
            .on_hover_text(AUTHOR_URL)
            .clicked();
        if !after.is_empty() {
            theme::text(ui, after, theme::regular(13.0), palette.secondary);
        }
    });
    clicked
}

/// Labeled settings row.
pub fn setting_row(
    ui: &mut Ui,
    palette: &Palette,
    label: &str,
    description: &str,
    control: impl FnOnce(&mut Ui),
) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - 260.0).max(120.0));
            rich_text(ui, label, theme::medium(14.0), palette.text);
            if !description.is_empty() {
                let description = line(
                    ui,
                    description,
                    theme::regular(12.5),
                    palette.secondary,
                    ui.available_width(),
                    usize::MAX,
                );
                let (rect, _) = ui.allocate_exact_size(description.size(), Sense::hover());
                if ui.is_rect_visible(rect) {
                    description.paint(ui, rect.min, palette.secondary);
                }
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), control);
    });
    ui.add_space(10.0);
}

pub fn paint_vertical_gradient(ui: &Ui, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(egui::Shape::mesh(mesh));
}

/// Small pill label used for date separators and pinned markers.
pub fn chip(ui: &mut Ui, palette: &Palette, label: &str) -> egui::Response {
    let galley =
        ui.painter()
            .layout_no_wrap(label.to_owned(), theme::medium(12.0), palette.secondary);
    let size = galley.size() + vec2(20.0, 10.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter()
            .rect_filled(rect, rect.height() / 2.0, palette.panel);
        ui.painter().galley(
            rect.center() - galley.size() / 2.0,
            galley,
            palette.secondary,
        );
    }
    response
}

/// Selectable pill with an optional count, used for the chat-list filters.
pub fn filter_chip(
    ui: &mut Ui,
    palette: &Palette,
    label: &str,
    count: usize,
    selected: bool,
) -> egui::Response {
    let color = if selected {
        palette.accent
    } else {
        palette.secondary
    };
    let painter = ui.painter();
    let text = painter.layout_no_wrap(label.to_owned(), theme::medium(12.5), color);
    let number =
        (count > 0).then(|| painter.layout_no_wrap(count.to_string(), theme::regular(11.5), color));
    let gap = 5.0;
    let width = text.size().x + number.as_ref().map_or(0.0, |number| gap + number.size().x);
    let (rect, response) = ui.allocate_exact_size(vec2(width + 18.0, 28.0), Sense::click());
    theme::reveal_focus(&response);
    theme::focus_outline(ui, response.id, rect, rect.height() / 2.0);
    if ui.is_rect_visible(rect) {
        let radius = rect.height() / 2.0;
        if selected {
            ui.painter()
                .rect_filled(rect, radius, palette.accent.gamma_multiply(0.18));
        } else {
            if response.hovered() {
                ui.painter().rect_filled(rect, radius, palette.surface);
            }
            ui.painter().rect_stroke(
                rect,
                radius,
                Stroke::new(1.0, palette.surface_active),
                egui::StrokeKind::Inside,
            );
        }
        let mut pos = pos2(rect.left() + 9.0, rect.center().y - text.size().y / 2.0);
        let advance = text.size().x + gap;
        ui.painter().galley(pos, text, color);
        if let Some(number) = number {
            pos.x += advance;
            pos.y = rect.center().y - number.size().y / 2.0;
            ui.painter().galley(pos, number, color);
        }
    }
    response.widget_info(|| {
        let label = if count > 0 {
            format!("{label}, {count} unread")
        } else {
            label.to_owned()
        };
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            label,
        )
    });
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The knob radius and the track outline a switch paints in one state.
    fn switch_shapes(on: bool) -> (f32, f32) {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut value = on;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            switch(ui, &crate::theme::Palette::light(), &mut value);
        });
        output.textures_delta.clear();
        let mut knob = 0.0_f32;
        let mut outline = 0.0_f32;
        for clipped in &output.shapes {
            match &clipped.shape {
                egui::Shape::Circle(circle) => knob = knob.max(circle.radius),
                egui::Shape::Rect(rect) => outline = outline.max(rect.stroke.width),
                _ => {}
            }
        }
        (knob, outline)
    }

    /// On and off differ in shape, not only in colour (WCAG 1.4.1).
    #[test]
    fn a_switch_shows_its_state_without_colour() {
        let (off_knob, off_outline) = switch_shapes(false);
        let (on_knob, on_outline) = switch_shapes(true);
        assert!(off_outline > 0.0, "the off track is outlined");
        assert_eq!(on_outline, 0.0, "the on track is filled, not outlined");
        assert!(on_knob > off_knob, "the knob grows when on");
    }
}
