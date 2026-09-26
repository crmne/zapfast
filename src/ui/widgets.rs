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
    let single = text.lines().next().unwrap_or_default();
    if max_rows == 1 && single.chars().any(bidi::is_strong_rtl) {
        return rtl_line(ui, text, single, font, color, width);
    }
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = width;
    job.wrap.max_rows = max_rows;
    // Break anywhere for single-line ellipsis; wrap multi-line text at words.
    job.wrap.break_anywhere = max_rows == 1;
    job.wrap.overflow_character = Some('…');
    let mut placements = Vec::new();
    let format = egui::TextFormat::simple(font, color);
    emoji::append(
        ui,
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

/// One line of text holding right-to-left script, cut to `width` by its
/// logical end. egui cuts a line in the order it lays glyphs out, which for
/// Arabic and Hebrew is already the visual one, so its ellipsis replaced a
/// letter mid-line and the reordered line overflowed its width (#72). The
/// longest start of the text that fits beside an ellipsis is laid out
/// whole instead, so the ellipsis ends the text as a reader expects.
fn rtl_line(
    ui: &Ui,
    text: &str,
    single: &str,
    font: egui::FontId,
    color: Color32,
    width: f32,
) -> Line {
    let format = egui::TextFormat::simple(font, color);
    let layout = |shown: &str| {
        let mut job = egui::text::LayoutJob::default();
        job.wrap.max_width = f32::INFINITY;
        job.wrap.max_rows = 1;
        let mut placements = Vec::new();
        emoji::append(ui, &mut job, &mut placements, shown, &format);
        (bidi::layout_job(ui, job), placements)
    };
    let (mut galley, mut placements) = layout(single);
    if galley.size().x > width {
        let ends: Vec<usize> = single
            .char_indices()
            .map(|(at, _)| at)
            .skip(1)
            .chain(std::iter::once(single.len()))
            .collect();
        // Longest prefix, in characters, that fits with the ellipsis.
        let (mut low, mut high) = (0, ends.len());
        let mut best = layout("…");
        while low < high {
            let middle = (low + high).div_ceil(2);
            let shown = format!("{}…", single[..ends[middle - 1]].trim_end());
            let candidate = layout(&shown);
            if candidate.0.size().x <= width {
                best = candidate;
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        (galley, placements) = best;
    }
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

/// Empty unread reminder: the same round as a count badge, with no digit.
pub fn unread_dot(ui: &Ui, palette: &Palette, at: egui::Pos2, muted: bool) -> f32 {
    let fill = if muted { palette.dim } else { palette.accent };
    ui.painter().circle_filled(at, 5.0, fill);
    10.0
}

/// The unread mark beside a chat: a numbered badge while messages are pending,
/// an empty dot for a chat marked unread by hand, nothing otherwise.
pub fn unread_indicator(
    ui: &Ui,
    palette: &Palette,
    at: egui::Pos2,
    count: u32,
    marked: bool,
    muted: bool,
) -> f32 {
    if count > 0 {
        badge(ui, palette, at, count, muted)
    } else if marked {
        unread_dot(ui, palette, at, muted)
    } else {
        0.0
    }
}

/// The height of a dialog's scrolling body: it grows with the content until
/// the whole dialog takes the window's height less a margin, up to a cap, and
/// then scrolls. Poll results set the measure; give the scroll area this as
/// both its maximum and its minimum scrolled height, so a modal does not
/// shrink it to the space below its first, smaller position.
pub fn dialog_scroll_height(ui: &Ui) -> f32 {
    // What the dialog has laid out above the scroll area: its title, and
    // anything else that stays in place.
    let above = (ui.cursor().top() - ui.min_rect().top()).max(0.0);
    (ui.ctx().content_rect().height() - 158.0 - above)
        .min(592.0 - above)
        .max(100.0)
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
    // Text padding, then `menu_frame`'s margin and its 1-point stroke on each
    // side: without the stroke the widest label lost its last letters.
    widest + if icons { 26.0 } else { 0.0 } + 20.0 + 12.0 + 2.0
}

/// A context-menu entry that opens a submenu, drawn like the plain entries
/// beside it, with a chevron.
pub fn submenu<R>(
    ui: &mut Ui,
    palette: &Palette,
    icon: Icon,
    label: &str,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> Option<egui::InnerResponse<R>> {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 28.0), Sense::click());
    theme::reveal_focus(&response);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    let open = egui::Popup::is_id_open(
        ui.ctx(),
        egui::containers::menu::SubMenu::id_from_widget_id(response.id),
    );
    if ui.is_rect_visible(rect) {
        if response.hovered() || open {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(6), palette.surface_hover);
        }
        let icon_rect =
            Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), Vec2::splat(16.0));
        icon.image(palette.secondary, 16.0).paint_at(ui, icon_rect);
        let chevron = Rect::from_center_size(
            pos2(rect.right() - 16.0, rect.center().y),
            Vec2::splat(14.0),
        );
        Icon::ChevronRight
            .image(palette.secondary, 14.0)
            .paint_at(ui, chevron);
        let x = rect.left() + 36.0;
        let mut job = egui::text::LayoutJob::simple_singleline(
            label.to_string(),
            theme::regular(13.5),
            palette.text,
        );
        job.wrap = egui::text::TextWrapping {
            max_width: (chevron.left() - 6.0 - x).max(0.0),
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('\u{2026}'),
        };
        let galley = crate::bidi::layout_job(ui, job);
        ui.painter().galley(
            pos2(x, rect.center().y - galley.size().y / 2.0),
            galley,
            palette.text,
        );
    }
    egui::containers::menu::SubMenu::new().show(ui, &response, add_contents)
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

pub fn menu_separator(ui: &mut Ui, palette: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
    ui.painter().hline(
        rect.x_range().shrink(6.0),
        rect.center().y,
        Stroke::new(1.0, palette.outline),
    );
}

/// Shows `frame` raised like a message bubble: the same soft lift shadow
/// below it and the faint raised edge along its top, for surfaces that sit
/// on the chat (the composer, and the strips above it).
pub fn raised<R>(
    ui: &mut Ui,
    palette: &Palette,
    frame: egui::Frame,
    add: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    // Reserved before the frame paints, so the edge lies under its fill.
    let edge_at = ui.painter().add(egui::Shape::Noop);
    let (fill, radius) = (frame.fill, frame.corner_radius);
    let shown = frame.shadow(palette.bubble_shadow()).show(ui, add);
    ui.painter().set(
        edge_at,
        egui::Shape::rect_filled(
            shown
                .response
                .rect
                .translate(vec2(0.0, -theme::RAISED_EDGE)),
            radius,
            palette.raised_edge(fill),
        ),
    );
    shown
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
    let format = egui::TextFormat::simple(theme::regular(14.0), palette.text);
    let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, wrap: f32| {
        bidi::layout_field(ui, buffer.as_str(), &format, wrap)
    };
    let align = if bidi::base_rtl(text) {
        Align::RIGHT
    } else {
        Align::LEFT
    };
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
            .vertical_align(Align::Center)
            .horizontal_align(align)
            .layouter(&mut layouter),
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

/// The hover or selection behind a chat-list row: a rounded card inset from
/// the list's edges rather than a full-width band.
pub fn row_highlight(ui: &Ui, palette: &Palette, rect: Rect, color: Color32) {
    let card = rect.shrink2(vec2(8.0, 2.0));
    let radius = CornerRadius::same(theme::RADIUS + 2);
    // Raised like a message bubble, only more gently: the list sits on a
    // flat panel, and a hovered row should not jump out.
    let mut shadow = palette.bubble_shadow();
    shadow.color = shadow.color.gamma_multiply(0.6);
    ui.painter().add(shadow.as_shape(card, radius));
    ui.painter().rect_filled(
        card.translate(vec2(0.0, -theme::RAISED_EDGE)),
        radius,
        palette.raised_edge(color),
    );
    ui.painter().rect_filled(card, radius, color);
}

/// A soft shadow cast downward from `edge`, for a bar that content scrolls
/// under, beneath a hairline of the bar's raised edge. One gradient quad.
pub fn paint_shadow_below(ui: &Ui, palette: &Palette, left: f32, right: f32, edge: f32) {
    let height = 9.0;
    let dark = palette.lift_shadow().gamma_multiply(0.64);
    ui.painter().rect_filled(
        Rect::from_min_max(pos2(left, edge - theme::RAISED_EDGE), pos2(right, edge)),
        0.0,
        palette.raised_edge(palette.panel),
    );
    let mut mesh = egui::Mesh::default();
    let rect = Rect::from_min_max(pos2(left, edge), pos2(right, edge + height));
    mesh.colored_vertex(rect.left_top(), dark);
    mesh.colored_vertex(rect.right_top(), dark);
    mesh.colored_vertex(rect.right_bottom(), Color32::TRANSPARENT);
    mesh.colored_vertex(rect.left_bottom(), Color32::TRANSPARENT);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(egui::Shape::mesh(mesh));
}

/// The side a message bubble's tail points to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// Corner radius of a message bubble.
pub const BUBBLE_RADIUS: u8 = 10;
/// How far a bubble's tail reaches out from its side, and down from its top.
const TAIL_WIDTH: f32 = 8.0;
const TAIL_HEIGHT: f32 = 11.0;

/// The corners of a message bubble: the one its tail leaves is square.
pub fn bubble_corners(tail: Option<Side>) -> CornerRadius {
    let mut corners = CornerRadius::same(BUBBLE_RADIUS);
    match tail {
        Some(Side::Left) => corners.nw = 0,
        Some(Side::Right) => corners.ne = 0,
        None => {}
    }
    corners
}

/// A message bubble's backdrop: its soft shadow, its fill, and on the first
/// message of a run a small tail at the top corner toward the sender, as the
/// phone draws it. A handful of vertices; bubbles off screen are culled
/// before tessellation.
pub fn bubble_shape(
    palette: &Palette,
    rect: Rect,
    fill: Color32,
    tail: Option<Side>,
) -> egui::Shape {
    let corners = bubble_corners(tail);
    let shadow = palette.bubble_shadow();
    let mut shapes = vec![egui::Shape::Rect(shadow.as_shape(rect, corners))];
    // The tail overlaps the bubble by a few points so no seam shows where
    // the two anti-aliased edges meet.
    let tail_points = tail.map(|side| {
        let (edge, out, into): (f32, f32, f32) = match side {
            Side::Left => (rect.left(), -TAIL_WIDTH, 3.0),
            Side::Right => (rect.right(), TAIL_WIDTH, -3.0),
        };
        let top = rect.top();
        let reach = TAIL_HEIGHT * (TAIL_WIDTH + into.abs()) / TAIL_WIDTH;
        let mut points = vec![
            pos2(edge + into, top),
            pos2(edge + into, top + reach),
            pos2(edge + out, top),
        ];
        // Clockwise winding either way.
        if side == Side::Right {
            points.reverse();
        }
        points
    });
    if let Some(points) = &tail_points {
        let offset = vec2(shadow.offset[0].into(), shadow.offset[1].into());
        let points: Vec<_> = points.iter().map(|point| *point + offset).collect();
        shapes.push(soft_triangle(&points, shadow.color, shadow.blur.into()));
    }
    // The raised edge: the same outline a hair higher, under the fill, so
    // only its top shows, thinning out down the rounded corners.
    let edge = palette.raised_edge(fill);
    let lift = vec2(0.0, -theme::RAISED_EDGE);
    shapes.push(egui::Shape::rect_filled(
        rect.translate(lift),
        corners,
        edge,
    ));
    if let Some(points) = &tail_points {
        shapes.push(egui::Shape::convex_polygon(
            points.iter().map(|point| *point + lift).collect(),
            edge,
            Stroke::NONE,
        ));
    }
    shapes.push(egui::Shape::rect_filled(rect, corners, fill));
    if let Some(points) = tail_points {
        shapes.push(egui::Shape::convex_polygon(points, fill, Stroke::NONE));
    }
    egui::Shape::Vec(shapes)
}

/// A triangle in `color` that fades out over `blur` points around its
/// edges, like the blurred shadow of the bubble it belongs to: a sharp one
/// showed as a grey wedge beneath the tail's tip. Six vertices.
fn soft_triangle(points: &[egui::Pos2], color: Color32, blur: f32) -> egui::Shape {
    let centre = (points
        .iter()
        .fold(Vec2::ZERO, |sum, point| sum + point.to_vec2())
        / points.len() as f32)
        .to_pos2();
    let mut mesh = egui::Mesh::default();
    for point in points {
        let out = (*point - centre).normalized();
        mesh.colored_vertex(*point - out * blur / 4.0, color);
        mesh.colored_vertex(*point + out * blur / 2.0, Color32::TRANSPARENT);
    }
    let count = points.len() as u32;
    // Inner vertices are even, their faded partners odd.
    mesh.add_triangle(0, 2, 4);
    for index in 0..count {
        let next = (index + 1) % count;
        mesh.add_triangle(2 * index, 2 * index + 1, 2 * next + 1);
        mesh.add_triangle(2 * index, 2 * next + 1, 2 * next);
    }
    egui::Shape::mesh(mesh)
}

/// Small pill label used for date separators and pinned markers.
pub fn chip(ui: &mut Ui, palette: &Palette, label: &str) -> egui::Response {
    let galley =
        ui.painter()
            .layout_no_wrap(label.to_owned(), theme::medium(12.0), palette.secondary);
    let size = galley.size() + vec2(20.0, 10.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        let radius = CornerRadius::from(rect.height() / 2.0);
        ui.painter()
            .add(palette.bubble_shadow().as_shape(rect, radius));
        ui.painter().rect_filled(
            rect.translate(vec2(0.0, -theme::RAISED_EDGE)),
            radius,
            palette.raised_edge(palette.panel),
        );
        ui.painter().rect_filled(rect, radius, palette.panel);
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
    dotted_chip(ui, palette, None, label, count, selected)
}

/// A filter chip led by a coloured dot, for filters the user named and
/// coloured, such as labels.
pub fn dotted_chip(
    ui: &mut Ui,
    palette: &Palette,
    dot: Option<Color32>,
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
    let dot_width = if dot.is_some() { 8.0 + gap } else { 0.0 };
    let width =
        dot_width + text.size().x + number.as_ref().map_or(0.0, |number| gap + number.size().x);
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
        if let Some(dot) = dot {
            ui.painter()
                .circle_filled(pos2(rect.left() + 13.0, rect.center().y), 4.0, dot);
        }
        let mut pos = pos2(
            rect.left() + 9.0 + dot_width,
            rect.center().y - text.size().y / 2.0,
        );
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

    /// A raised frame lies on its edge: one point higher, under its fill, in
    /// the palette's raised-edge colour, with the bubble's lift shadow.
    /// A line of Arabic cut to its width ends with the ellipsis where the
    /// text ends (its left, in a right-to-left line), inside the width, with
    /// whole words before it (#72).
    #[test]
    fn a_cut_right_to_left_line_stays_in_its_width_and_ends_in_an_ellipsis() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            for text in [
                "المنصب بتاعك مش هو اللي بيحدد قيمتك في الحياة يا صاحبي",
                "39: المنصب بتاعك مش هو اللي بيحدد قيمتك في الحياة",
            ] {
                let line = line(
                    ui,
                    text,
                    egui::FontId::proportional(13.0),
                    Color32::WHITE,
                    150.0,
                    1,
                );
                let galley = &line.galley;
                assert_eq!(galley.rows.len(), 1, "{text}");
                assert!(galley.size().x <= 150.0, "{text}: {}", galley.size().x);
                let shown = galley.text();
                assert!(shown.ends_with('…'), "{shown}");
                let kept = shown.trim_end_matches('…');
                assert!(text.starts_with(kept), "the start is kept: {shown}");
                // The ellipsis is drawn leftmost: the text ends on the left.
                let row = &galley.rows[0];
                let mark = row.glyphs.iter().find(|glyph| glyph.chr == '…').unwrap();
                assert!(
                    row.glyphs
                        .iter()
                        .filter(|glyph| bidi::is_strong_rtl(glyph.chr))
                        .all(|glyph| glyph.pos.x > mark.pos.x),
                    "{shown}"
                );
                assert_eq!(line.accessible_text, text);
            }
            // A line that fits is left alone.
            let short = line(
                ui,
                "مرحبا",
                egui::FontId::proportional(13.0),
                Color32::WHITE,
                150.0,
                1,
            );
            assert_eq!(short.galley.text(), "مرحبا");
        });
        output.textures_delta.clear();
    }

    #[test]
    fn a_raised_frame_draws_its_edge_under_its_fill() {
        let palette = Palette::dark();
        let ctx = egui::Context::default();
        let mut rect = Rect::NOTHING;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            rect = raised(
                ui,
                &palette,
                egui::Frame::new().fill(palette.surface).inner_margin(8),
                |ui| ui.label("x"),
            )
            .response
            .rect;
        });
        output.textures_delta.clear();
        // In paint order, looking inside grouped shapes (a frame groups its
        // shadow with its fill).
        let mut fills: Vec<(Rect, Color32)> = Vec::new();
        let mut pending: Vec<&egui::Shape> = output
            .shapes
            .iter()
            .rev()
            .map(|clipped| &clipped.shape)
            .collect();
        while let Some(shape) = pending.pop() {
            match shape {
                egui::Shape::Vec(shapes) => pending.extend(shapes.iter().rev()),
                egui::Shape::Rect(shape) => fills.push((shape.rect, shape.fill)),
                _ => {}
            }
        }
        let edge = fills
            .iter()
            .position(|(at, fill)| {
                *at == rect.translate(vec2(0.0, -theme::RAISED_EDGE))
                    && *fill == palette.raised_edge(palette.surface)
            })
            .expect("the edge is drawn");
        let body = fills
            .iter()
            .position(|(at, fill)| *at == rect && *fill == palette.surface)
            .expect("the frame is drawn");
        assert!(edge < body, "the edge lies under the fill");
    }

    #[test]
    fn a_tail_squares_its_corner_and_points_toward_the_sender() {
        assert_eq!(bubble_corners(None), CornerRadius::same(BUBBLE_RADIUS));
        let left = bubble_corners(Some(Side::Left));
        assert_eq!((left.nw, left.ne), (0, BUBBLE_RADIUS));
        let right = bubble_corners(Some(Side::Right));
        assert_eq!((right.nw, right.ne), (BUBBLE_RADIUS, 0));

        let rect = Rect::from_min_size(pos2(100.0, 50.0), vec2(200.0, 40.0));
        let palette = Palette::light();
        for (side, outside) in [(Side::Left, 92.0), (Side::Right, 308.0)] {
            let egui::Shape::Vec(shapes) =
                bubble_shape(&palette, rect, palette.bubble_out, Some(side))
            else {
                panic!("a bubble is a list of shapes");
            };
            // The tail's tip reaches out of the bubble at its top edge. The
            // tail is the last filled triangle, over its shadow and edge.
            let tip = shapes
                .iter()
                .rev()
                .find_map(|shape| match shape {
                    egui::Shape::Path(path) if path.fill == palette.bubble_out => Some(path),
                    _ => None,
                })
                .into_iter()
                .flat_map(|path| path.points.iter())
                .find(|point| !rect.contains(**point));
            assert_eq!(tip.copied(), Some(pos2(outside, rect.top())));
        }
        // Later messages of a run draw no tail.
        let egui::Shape::Vec(shapes) = bubble_shape(&palette, rect, palette.bubble_in, None) else {
            panic!("a bubble is a list of shapes");
        };
        assert!(
            !shapes
                .iter()
                .any(|shape| matches!(shape, egui::Shape::Path(_)))
        );
    }

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
