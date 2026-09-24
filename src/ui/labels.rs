//! Label chips, the label menu, and the label manager.
//!
//! Labels are local to this computer: a name, a colour, and the chats that
//! wear them. They are not WhatsApp Business labels or WhatsApp lists, and
//! nothing here reaches the phone or the protocol.

use std::borrow::Cow;

use egui::{Color32, CornerRadius, Rect, Sense, Stroke, pos2, vec2};

use crate::app::App;
use crate::i18n::{Locale, gettext, ngettext, pgettext};
use crate::model::{Action, Chat, Dialog, Label};
use crate::theme::{self, Icon, Palette};

use super::focus::{Stop, TabStop};
use super::widgets;

/// Colours offered when a label is made or edited.
pub const PRESET_COLORS: [&str; 10] = [
    "#3b82f6", "#22c55e", "#eab308", "#f97316", "#ef4444", "#ec4899", "#a855f7", "#14b8a6",
    "#64748b", "#0ea5e9",
];

const ROW_HEIGHT: f32 = 34.0;

/// Stable label-chip id used by interaction tests.
pub fn chip_id(label: &str) -> egui::Id {
    egui::Id::new(("label-chip", label))
}

/// Stable id for the Labels menu chip, used by interaction tests.
pub fn menu_chip_id() -> egui::Id {
    egui::Id::new("label-menu-chip")
}

/// Stable id for the manager's name field.
pub fn name_field_id() -> egui::Id {
    egui::Id::new("label-name-field")
}

/// Stable id for one row of the manager.
pub fn label_row_id(id: &str) -> egui::Id {
    egui::Id::new(("label-row", id))
}

/// Stable id for a colour swatch, in the manager and in a row being edited.
pub fn swatch_id(hex: &str) -> egui::Id {
    egui::Id::new(("label-swatch", hex))
}

/// Reads a `#rrggbb` colour, falling back to the theme accent.
pub fn color_of(palette: &Palette, hex: &str) -> Color32 {
    let digits = hex.trim_start_matches('#');
    if digits.len() != 6 {
        return palette.accent;
    }
    let channel = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
    match (channel(0), channel(2), channel(4)) {
        (Some(red), Some(green), Some(blue)) => Color32::from_rgb(red, green, blue),
        _ => palette.accent,
    }
}

/// The spoken name of a preset colour, for screen readers.
fn color_name(locale: Locale, hex: &str) -> Cow<'static, str> {
    match hex {
        "#3b82f6" => pgettext(locale, "label colour", "Blue"),
        "#22c55e" => pgettext(locale, "label colour", "Green"),
        "#eab308" => pgettext(locale, "label colour", "Yellow"),
        "#f97316" => pgettext(locale, "label colour", "Orange"),
        "#ef4444" => pgettext(locale, "label colour", "Red"),
        "#ec4899" => pgettext(locale, "label colour", "Pink"),
        "#a855f7" => pgettext(locale, "label colour", "Purple"),
        "#14b8a6" => pgettext(locale, "label colour", "Teal"),
        "#64748b" => pgettext(locale, "label colour", "Grey"),
        "#0ea5e9" => pgettext(locale, "label colour", "Sky blue"),
        _ => pgettext(locale, "label colour", "Colour"),
    }
}

/// The label the chip rows show as chosen. Chips read as unchosen in the
/// archive and the locked folder, which the label filter does not reach.
fn active_label(app: &App) -> Option<String> {
    if app.show_archived || app.locked_folder_open() {
        return None;
    }
    app.label_filter.clone()
}

/// A row of label chips under the built-in ones, with
/// [`Settings::label_chips`](crate::settings::Settings::label_chips) on.
///
/// The row has no All chip of its own: a label is one more choice beside the
/// built-in chips, so picking it lets go of them, picking one of them lets go
/// of it, and All above shows every chat again.
pub fn chip_row(app: &mut App, ui: &mut egui::Ui, palette: &Palette) {
    if !app.settings.label_chips {
        return;
    }
    ui.add_space(2.0);
    egui::ScrollArea::horizontal()
        .id_salt("label-chips")
        .animated(false)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 6.0);
                label_chips(app, ui, palette);
                ui.add_space(4.0);
            });
        });
}

fn label_chips(app: &mut App, ui: &mut egui::Ui, palette: &Palette) {
    let active = active_label(app);
    let rows: Vec<(String, String, usize, Color32)> = app
        .labels
        .iter()
        .map(|label| {
            (
                label.id.clone(),
                label.name.clone(),
                app.label_unread(&label.id),
                color_of(palette, &label.color_hex),
            )
        })
        .collect();
    let mut pick: Option<Option<String>> = None;
    for (index, (id, name, count, color)) in rows.into_iter().enumerate() {
        let selected = active.as_deref() == Some(id.as_str());
        let chip = widgets::dotted_chip(ui, palette, Some(color), &name, count, selected)
            .tab_stop(Stop::Label(u8::try_from(index).unwrap_or(u8::MAX)));
        ui.ctx()
            .data_mut(|data| data.insert_temp(chip_id(&id), chip.rect));
        if chip.clicked() {
            // A second click on the active chip returns to every chat, the
            // way the other chips behave.
            pick = Some((!selected).then_some(id));
        }
    }
    let locale = app.locale;
    if theme::icon_button(
        ui,
        Icon::Plus,
        15.0,
        palette.secondary,
        palette.text,
        &gettext(locale, "Manage labels"),
    )
    .tab_stop(Stop::ManageLabels)
    .clicked()
    {
        app.actions.push(Action::ShowDialog(Dialog::Labels));
    }
    if let Some(label) = pick {
        app.actions.push(Action::SelectLabel(label));
    }
}

/// One chip before the built-in chips that opens every label in a menu,
/// when labels do not have a row of their own. It shows the chosen label's
/// name and colour, so a narrowed list always says why.
pub fn menu_chip(app: &mut App, ui: &mut egui::Ui, palette: &Palette) {
    // The chip appears with the labels, not before them.
    if app.settings.label_chips || app.labels.is_empty() {
        return;
    }
    let locale = app.locale;
    let active = active_label(app).and_then(|id| app.label(&id).cloned());
    let name = active.as_ref().map_or_else(
        || gettext(locale, "Labels").into_owned(),
        |label| label.name.clone(),
    );
    let dot = active
        .as_ref()
        .map(|label| color_of(palette, &label.color_hex));
    let chip =
        widgets::dotted_chip(ui, palette, dot, &name, 0, active.is_some()).tab_stop(Stop::Labels);
    ui.ctx()
        .data_mut(|data| data.insert_temp(menu_chip_id(), chip.rect));
    let mut pick: Option<Option<String>> = None;
    let mut manage = false;
    egui::Popup::menu(&chip)
        .frame(widgets::menu_frame(palette))
        .show(|ui| {
            let icon = |on: bool| on.then_some(Icon::Check);
            if widgets::menu_item(
                ui,
                palette,
                icon(active.is_none()),
                &gettext(locale, "All chats"),
            ) {
                pick = Some(None);
            }
            for label in &app.labels {
                let worn = active.as_ref().is_some_and(|active| active.id == label.id);
                if widgets::menu_item(ui, palette, icon(worn), &label.name) {
                    pick = Some(Some(label.id.clone()));
                }
            }
            widgets::menu_separator(ui, palette);
            if widgets::menu_item(
                ui,
                palette,
                Some(Icon::Pencil),
                &gettext(locale, "Manage labels…"),
            ) {
                manage = true;
            }
        });
    if manage {
        app.actions.push(Action::ShowDialog(Dialog::Labels));
    }
    if let Some(label) = pick {
        app.actions.push(Action::SelectLabel(label));
    }
}

/// The labels a chat wears, as a submenu of the chat's context menu.
pub fn chat_menu(app: &mut App, ui: &mut egui::Ui, chat: &Chat, palette: &Palette) {
    let locale = app.locale;
    widgets::submenu(ui, palette, Icon::Tag, &gettext(locale, "Labels"), |ui| {
        for label in &app.labels {
            let worn = app.chat_wears(chat, &label.id);
            let icon = if worn { Some(Icon::Check) } else { None };
            if widgets::menu_item(ui, palette, icon, &label.name) {
                let mut next: Vec<String> = chat.labels.clone();
                if worn {
                    next.retain(|id| id != &label.id);
                } else {
                    next.push(label.id.clone());
                }
                app.actions.push(Action::SetChatLabels {
                    chat: chat.id.clone(),
                    labels: next,
                });
            }
        }
        if !app.labels.is_empty() {
            widgets::menu_separator(ui, palette);
        }
        if widgets::menu_item(
            ui,
            palette,
            Some(Icon::Pencil),
            &gettext(locale, "Manage labels…"),
        ) {
            app.actions.push(Action::ShowDialog(Dialog::Labels));
        }
    });
}

/// The manager: create, rename, recolour and delete labels.
pub fn manager(app: &mut App, ui: &mut egui::Ui, palette: &Palette) {
    let locale = app.locale;
    ui.horizontal(|ui| {
        theme::text(
            ui,
            gettext(locale, "Labels"),
            theme::bold(18.0),
            palette.text,
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::icon_button(
                ui,
                Icon::X,
                16.0,
                palette.secondary,
                palette.text,
                &gettext(locale, "Close"),
            )
            .clicked()
            {
                app.actions.push(Action::CloseDialog);
            }
        });
    });
    theme::paragraph(
        ui,
        gettext(
            locale,
            "Labels stay on this computer. They do not reach your phone or anyone you chat with.",
        ),
        theme::regular(12.5),
        palette.dim,
    );
    ui.add_space(14.0);

    let full = app.labels.len() >= crate::archive::LABEL_LIMIT;
    if full {
        theme::paragraph(
            ui,
            gettext(locale, "You have {limit} labels, the most ZapFast keeps.")
                .replace("{limit}", &crate::archive::LABEL_LIMIT.to_string()),
            theme::regular(12.5),
            palette.warning,
        );
    } else {
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut app.label_name)
                    .id(name_field_id())
                    .hint_text(gettext(locale, "New label"))
                    .char_limit(crate::archive::NAME_LIMIT)
                    .desired_width(160.0)
                    .font(theme::regular(14.0)),
            );
            if field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                create(app);
            }
            swatches(app, ui, palette);
            if !app.label_name.trim().is_empty()
                && theme::soft_button(
                    ui,
                    palette,
                    Some(Icon::Plus),
                    &gettext(locale, "Create"),
                    false,
                )
                .clicked()
            {
                create(app);
            }
        });
    }

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(6.0);
    let wearing: Vec<(Label, usize)> = app
        .labels
        .iter()
        .map(|label| {
            let count = app
                .chats
                .iter()
                .filter(|chat| app.chat_wears(chat, &label.id))
                .count();
            (label.clone(), count)
        })
        .collect();
    if wearing.is_empty() {
        theme::paragraph(
            ui,
            gettext(
                locale,
                "No labels yet. Create one above, then add chats to it from their right-click menu.",
            ),
            theme::regular(13.0),
            palette.dim,
        );
        return;
    }
    let mut save: Option<(String, String, String)> = None;
    let mut delete: Option<String> = None;
    for (label, count) in wearing {
        let rect = ui
            .allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover())
            .0;
        ui.ctx()
            .data_mut(|data| data.insert_temp(label_row_id(&label.id), rect));
        let editing = app
            .label_editing
            .as_ref()
            .is_some_and(|(id, _)| id == &label.id);
        if editing {
            let mut draft = app.label_editing.take().expect("editing a label");
            let field = ui.put(
                Rect::from_min_size(rect.left_center() + vec2(0.0, -12.0), vec2(180.0, 24.0)),
                egui::TextEdit::singleline(&mut draft.1)
                    .id(rename_field_id(&label.id))
                    .char_limit(crate::archive::NAME_LIMIT)
                    .font(theme::regular(13.5)),
            );
            let swatch_rect =
                Rect::from_min_size(rect.left_center() + vec2(190.0, -9.0), vec2(18.0, 18.0));
            let swatch = ui.interact(swatch_rect, swatch_id(&label.id), Sense::click());
            swatch.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    gettext(locale, "Next colour"),
                )
            });
            ui.painter().rect_filled(
                swatch_rect,
                CornerRadius::same(4),
                color_of(palette, &label.color_hex),
            );
            let typed = draft.1.trim().to_owned();
            let name = if typed.is_empty() {
                label.name.clone()
            } else {
                typed
            };
            if swatch.clicked() {
                // A new colour keeps the row open, with what is typed so far.
                save = Some((label.id.clone(), name, next_color(&label.color_hex)));
                app.label_editing = Some(draft);
            } else if field.lost_focus() {
                // Enter saves; Escape or a click elsewhere keeps the old name.
                if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    save = Some((label.id.clone(), name, label.color_hex.clone()));
                }
            } else {
                app.label_editing = Some(draft);
            }
            continue;
        }
        let swatch_rect =
            Rect::from_min_size(rect.left_center() + vec2(0.0, -7.0), vec2(14.0, 14.0));
        ui.painter().rect_filled(
            swatch_rect,
            CornerRadius::same(4),
            color_of(palette, &label.color_hex),
        );
        let name = widgets::line(
            ui,
            &label.name,
            theme::medium(13.5),
            palette.text,
            (rect.width() - 140.0).max(40.0),
            1,
        );
        name.paint(
            ui,
            pos2(rect.left() + 24.0, rect.center().y - 8.0),
            palette.text,
        );
        let worn =
            ngettext(locale, "{} chat", "{} chats", count as u32).replace("{}", &count.to_string());
        let worn_width = ui
            .painter()
            .layout_no_wrap(worn.clone(), theme::regular(12.0), palette.dim)
            .size()
            .x;
        let worn_line = widgets::line(ui, &worn, theme::regular(12.0), palette.dim, worn_width, 1);
        worn_line.paint(
            ui,
            pos2(rect.right() - 62.0 - worn_width, rect.center().y - 7.0),
            palette.dim,
        );
        let edit_rect =
            Rect::from_min_size(rect.right_center() + vec2(-54.0, -11.0), vec2(24.0, 22.0));
        let delete_rect =
            Rect::from_min_size(rect.right_center() + vec2(-28.0, -11.0), vec2(24.0, 22.0));
        let edit = ui.interact(
            edit_rect,
            egui::Id::new(("label-edit", &label.id)),
            Sense::click(),
        );
        let rename = gettext(locale, "Rename or recolour");
        edit.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, rename.as_ref())
        });
        theme::paint_icon(
            ui,
            Icon::Pencil,
            edit_rect,
            14.0,
            if edit.hovered() {
                palette.text
            } else {
                palette.secondary
            },
        );
        if edit.clicked() {
            app.label_editing = Some((label.id.clone(), label.name.clone()));
            ui.memory_mut(|memory| memory.request_focus(rename_field_id(&label.id)));
        }
        edit.on_hover_text(rename.as_ref());
        let trash = ui.interact(
            delete_rect,
            egui::Id::new(("label-delete", &label.id)),
            Sense::click(),
        );
        let remove = gettext(locale, "Delete this label. Its chats keep their messages.");
        trash.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, remove.as_ref())
        });
        theme::paint_icon(
            ui,
            Icon::Trash,
            delete_rect,
            14.0,
            if trash.hovered() {
                palette.danger
            } else {
                palette.secondary
            },
        );
        if trash.clicked() {
            delete = Some(label.id.clone());
        }
        trash.on_hover_text(remove.as_ref());
    }
    if let Some((id, name, color_hex)) = save {
        app.actions.push(Action::UpdateLabel {
            id,
            name,
            color_hex,
        });
    }
    if let Some(id) = delete {
        app.actions.push(Action::DeleteLabel(id));
    }
}

/// Stable id for the field that renames one label.
fn rename_field_id(id: &str) -> egui::Id {
    egui::Id::new(("label-rename", id))
}

/// Sends the label the create row describes.
fn create(app: &mut App) {
    let name = app.label_name.trim().to_owned();
    if name.is_empty() {
        return;
    }
    let color_hex = app.label_color.clone();
    app.actions.push(Action::CreateLabel { name, color_hex });
}

/// The next preset colour after this one, so a click always changes something.
fn next_color(hex: &str) -> String {
    match PRESET_COLORS
        .iter()
        .position(|preset| preset.eq_ignore_ascii_case(hex))
    {
        Some(at) => PRESET_COLORS[(at + 1) % PRESET_COLORS.len()].to_owned(),
        // A colour that is not one of the presets steps onto the first one.
        None => PRESET_COLORS[0].to_owned(),
    }
}

fn swatches(app: &mut App, ui: &mut egui::Ui, palette: &Palette) {
    let locale = app.locale;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
        for hex in PRESET_COLORS {
            let selected = app.label_color.eq_ignore_ascii_case(hex);
            let (rect, response) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::RadioButton,
                    true,
                    selected,
                    color_name(locale, hex),
                )
            });
            theme::reveal_focus(&response);
            ui.ctx()
                .data_mut(|data| data.insert_temp(swatch_id(hex), rect));
            ui.painter()
                .rect_filled(rect, CornerRadius::same(4), color_of(palette, hex));
            if selected || response.has_focus() {
                ui.painter().rect_stroke(
                    rect.expand(1.5),
                    CornerRadius::same(5),
                    Stroke::new(1.5, palette.text),
                    egui::StrokeKind::Outside,
                );
            }
            if response.clicked() {
                app.label_color = hex.to_owned();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::paths::AppDirs;
    use crate::settings::Settings;

    fn label(id: &str, name: &str) -> Label {
        Label {
            id: id.to_owned(),
            name: name.to_owned(),
            color_hex: "#22c55e".to_owned(),
            created_at: 1,
        }
    }

    /// Draws the label chips once, so their rects are recorded.
    fn draw(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(420.0, 240.0))),
            events,
            ..Default::default()
        };
        let palette = app.palette;
        let mut output = ctx.run_ui(input, |ui| {
            ui.horizontal(|ui| menu_chip(app, ui, &palette));
            chip_row(app, ui, &palette);
        });
        output.textures_delta.clear();
    }

    fn click(pos: egui::Pos2) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    #[test]
    fn a_label_chip_picks_its_label_and_a_second_click_shows_all() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _events) =
            App::headless(AppDirs::under(directory.path()), Settings::default());
        app.settings.label_chips = true;
        app.labels = vec![label("label-1", "Work")];
        let ctx = egui::Context::default();
        app.attach(&ctx);
        draw(&mut app, &ctx, Vec::new());
        let rect = ctx
            .data(|data| data.get_temp::<Rect>(chip_id("label-1")))
            .expect("the label chip is drawn");
        let pos = rect.center();
        draw(&mut app, &ctx, click(pos));
        assert!(
            app.actions
                .contains(&Action::SelectLabel(Some("label-1".into()))),
            "a click picks the label"
        );
        app.actions.clear();
        app.label_filter = Some("label-1".into());
        draw(&mut app, &ctx, Vec::new());
        draw(&mut app, &ctx, click(pos));
        assert!(
            app.actions.contains(&Action::SelectLabel(None)),
            "a second click on the active chip shows every chat"
        );
    }

    #[test]
    fn labels_share_one_menu_chip_unless_they_are_chips() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _events) =
            App::headless(AppDirs::under(directory.path()), Settings::default());
        let ctx = egui::Context::default();
        app.attach(&ctx);
        draw(&mut app, &ctx, Vec::new());
        assert!(
            ctx.data(|data| data.get_temp::<Rect>(menu_chip_id()))
                .is_none(),
            "no Labels chip before there is a label"
        );
        app.labels = vec![label("label-1", "Work")];
        draw(&mut app, &ctx, Vec::new());
        assert!(
            ctx.data(|data| data.get_temp::<Rect>(chip_id("label-1")))
                .is_none(),
            "no chip per label without the setting"
        );
        assert!(
            ctx.data(|data| data.get_temp::<Rect>(menu_chip_id()))
                .is_some(),
            "one Labels chip holds them"
        );
    }

    #[test]
    fn a_colour_falls_back_when_it_is_not_hex() {
        let palette = Palette::dark();
        assert_eq!(
            color_of(&palette, "#22c55e"),
            Color32::from_rgb(34, 197, 94)
        );
        assert_eq!(color_of(&palette, "22c55e"), Color32::from_rgb(34, 197, 94));
        assert_eq!(color_of(&palette, "green"), palette.accent);
        assert_eq!(color_of(&palette, "#12345"), palette.accent);
    }

    #[test]
    fn a_click_walks_the_preset_colours() {
        assert_eq!(next_color(PRESET_COLORS[0]), PRESET_COLORS[1]);
        assert_eq!(
            next_color(PRESET_COLORS[PRESET_COLORS.len() - 1]),
            PRESET_COLORS[0]
        );
        assert_eq!(next_color("#ffffff"), PRESET_COLORS[0]);
    }

    #[test]
    fn every_preset_colour_has_a_name() {
        for hex in PRESET_COLORS {
            assert_ne!(color_name(Locale::English, hex), "Colour", "{hex}");
        }
    }
}
