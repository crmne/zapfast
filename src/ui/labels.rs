//! Label tabs, the label menu, and the label manager.
//!
//! Labels are local to this computer. A plain WhatsApp account keeps no labels
//! on the server, so nothing here reaches the phone: a label is a name, a
//! colour, and the chats that wear it.

use egui::{Color32, CornerRadius, Rect, Sense, Stroke, pos2, vec2};

use crate::app::App;
use crate::model::{Action, Chat, Dialog, Label};
use crate::theme::{self, Icon, Palette};

use super::widgets;

/// Colours offered when a label is made or edited.
pub const PRESET_COLORS: [&str; 10] = [
    "#3b82f6", "#22c55e", "#eab308", "#f97316", "#ef4444", "#ec4899", "#a855f7", "#14b8a6",
    "#64748b", "#0ea5e9",
];

const TAB_HEIGHT: f32 = 26.0;
const ROW_HEIGHT: f32 = 34.0;

/// Stable tab id used by interaction tests.
pub fn tab_id(pane: usize, label: Option<&str>) -> egui::Id {
    egui::Id::new(("label-tab", pane, label))
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

/// One tab per label, plus All, above the chat list.
///
/// A plain click shows one label in this pane. Ctrl-click opens the label
/// beside the current one, which is how the workspace splits in two.
pub fn tab_bar(app: &mut App, ui: &mut egui::Ui, pane: usize, palette: &Palette) {
    if !app.settings.labels_as_tabs || app.show_archived || app.locked_folder_open() {
        return;
    }
    let mut pick: Option<Option<String>> = None;
    let mut split: Option<Option<String>> = None;
    let mut manage = false;
    ui.add_space(6.0);
    egui::ScrollArea::horizontal()
        .id_salt(("label-tabs", pane))
        .animated(false)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 6.0);
                let active = app.panes.get(pane).and_then(|state| state.label.clone());
                let rows: Vec<(Option<String>, String, u32, Color32)> =
                    std::iter::once((None, "All".to_owned(), app.unread_total(), palette.accent))
                        .chain(app.labels.iter().map(|label| {
                            (
                                Some(label.id.clone()),
                                label.name.clone(),
                                app.label_unread(&label.id),
                                color_of(palette, &label.color_hex),
                            )
                        }))
                        .collect();
                for (id, name, count, color) in rows {
                    let selected = active == id;
                    let (response, rect) = tab(ui, palette, &name, count, color, selected);
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(tab_id(pane, id.as_deref()), rect));
                    if response.clicked() {
                        if ui.input(|input| input.modifiers.command) && !selected {
                            split = Some(id.clone());
                        } else if selected && id.is_some() {
                            // A second click on the active tab shows them all,
                            // the way the filter chips behave.
                            pick = Some(None);
                        } else {
                            pick = Some(id.clone());
                        }
                    }
                }
                if theme::icon_button(
                    ui,
                    Icon::Plus,
                    15.0,
                    palette.secondary,
                    palette.text,
                    "Manage labels",
                )
                .clicked()
                {
                    manage = true;
                }
            });
        });
    if manage {
        app.actions.push(Action::ShowDialog(Dialog::Labels));
    }
    if let Some(id) = split {
        app.actions.push(Action::OpenLabelSplit(id));
    }
    if let Some(id) = pick {
        app.actions.push(Action::SelectLabel { pane, label: id });
    }
}

/// The label filter as a menu, for when labels are not shown as tabs.
pub fn filter_menu(app: &mut App, ui: &mut egui::Ui, palette: &Palette) {
    if app.settings.labels_as_tabs || app.show_archived || app.locked_folder_open() {
        return;
    }
    // The button appears with the labels, not before them.
    if app.labels.is_empty() {
        return;
    }
    let active = app.panes.first().and_then(|state| state.label.clone());
    let name = active
        .as_ref()
        .and_then(|id| app.label(id))
        .map(|label| label.name.clone())
        .unwrap_or_else(|| "Labels".to_owned());
    let button = theme::soft_button(ui, palette, None, &name, active.is_some());
    let mut pick: Option<Option<String>> = None;
    let mut manage = false;
    egui::Popup::menu(&button)
        .frame(widgets::menu_frame(palette))
        .show(|ui| {
            if widgets::menu_item(ui, palette, Some(Icon::Check), "All chats") {
                pick = Some(None);
            }
            for label in &app.labels {
                let worn = active.as_deref() == Some(label.id.as_str());
                let icon = if worn { Some(Icon::Check) } else { None };
                if widgets::menu_item(ui, palette, icon, &label.name) {
                    pick = Some(Some(label.id.clone()));
                }
            }
            ui.separator();
            if widgets::menu_item(ui, palette, Some(Icon::Plus), "Manage labels…") {
                manage = true;
            }
        });
    if manage {
        app.actions.push(Action::ShowDialog(Dialog::Labels));
    }
    if let Some(id) = pick {
        app.actions.push(Action::SelectLabel { pane: 0, label: id });
    }
}

/// The labels a chat wears, in the chat's context menu.
pub fn chat_menu(app: &mut App, ui: &mut egui::Ui, chat: &Chat, palette: &Palette) {
    ui.separator();
    theme::text(ui, "Labels", theme::regular(11.5), palette.dim);
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
    if app.labels.len() < crate::archive::LABEL_LIMIT
        && widgets::menu_item(ui, palette, Some(Icon::Plus), "New label…")
    {
        app.actions.push(Action::ShowDialog(Dialog::Labels));
    }
}

/// The manager: create, rename, recolour and delete labels.
pub fn manager(app: &mut App, ui: &mut egui::Ui, palette: &Palette) {
    theme::text(ui, "Labels", theme::bold(18.0), palette.text);
    theme::text(
        ui,
        "Labels stay on this computer; nothing here reaches the phone.",
        theme::regular(12.5),
        palette.dim,
    );
    ui.add_space(14.0);

    ui.horizontal(|ui| {
        let field = ui.add(
            egui::TextEdit::singleline(&mut app.label_name)
                .id(name_field_id())
                .hint_text("New label")
                .desired_width(180.0)
                .font(theme::regular(14.0)),
        );
        if field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            create(app);
        }
        swatches(app, ui, palette);
        if !app.label_name.trim().is_empty()
            && theme::soft_button(ui, palette, Some(Icon::Plus), "Create", false).clicked()
        {
            create(app);
        }
    });
    ui.add_space(6.0);
    if app.labels.len() >= crate::archive::LABEL_LIMIT {
        theme::text(
            ui,
            format!(
                "You have all {} labels WhatsApp allows.",
                crate::archive::LABEL_LIMIT
            ),
            theme::regular(12.5),
            palette.warning,
        );
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
        theme::text(
            ui,
            "No labels yet. Make one above, then wear it from any chat's menu.",
            theme::regular(13.0),
            palette.dim,
        );
        return;
    }
    let mut save: Option<(String, String, String)> = None;
    let mut delete: Option<String> = None;
    for (label, count) in wearing {
        let rect = ui
            .allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click())
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
                egui::TextEdit::singleline(&mut draft.1).font(theme::regular(13.5)),
            );
            let swatch_rect =
                Rect::from_min_size(rect.left_center() + vec2(190.0, -9.0), vec2(18.0, 18.0));
            let swatch = ui.interact(swatch_rect, swatch_id(&label.id), Sense::click());
            ui.painter().rect_filled(
                swatch_rect,
                CornerRadius::same(4),
                color_of(palette, &label.color_hex),
            );
            if swatch.clicked() {
                save = Some((
                    label.id.clone(),
                    label.name.clone(),
                    next_color(&label.color_hex),
                ));
            }
            if field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                save = Some((label.id.clone(), draft.1.trim().to_owned(), label.color_hex));
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
        let worn = match count {
            1 => "1 chat".to_owned(),
            other => format!("{other} chats"),
        };
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
        }
        let trash = ui.interact(
            delete_rect,
            egui::Id::new(("label-delete", &label.id)),
            Sense::click(),
        );
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
        trash.on_hover_text("Delete this label. The chats keep everything else.");
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
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
        for hex in PRESET_COLORS {
            let selected = app.label_color.eq_ignore_ascii_case(hex);
            let (rect, response) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
            ui.ctx()
                .data_mut(|data| data.insert_temp(swatch_id(hex), rect));
            ui.painter()
                .rect_filled(rect, CornerRadius::same(4), color_of(palette, hex));
            if selected {
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

/// Draws one tab and reports its rect, for tests to click.
fn tab(
    ui: &mut egui::Ui,
    palette: &Palette,
    name: &str,
    count: u32,
    color: Color32,
    selected: bool,
) -> (egui::Response, Rect) {
    let font = if selected {
        theme::medium(13.0)
    } else {
        theme::regular(13.0)
    };
    let text_width = ui
        .painter()
        .layout_no_wrap(name.to_owned(), font.clone(), palette.text)
        .size()
        .x;
    let badge = if count > 0 {
        ui.painter()
            .layout_no_wrap(count.to_string(), theme::regular(11.0), palette.dim)
            .size()
            .x
            + 12.0
    } else {
        0.0
    };
    let size = vec2(text_width + badge + 22.0, TAB_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if selected {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS),
            palette.surface_active,
        );
        ui.painter().rect_filled(
            Rect::from_min_size(
                pos2(rect.left() + 6.0, rect.bottom() - 2.0),
                vec2(rect.width() - 12.0, 2.0),
            ),
            CornerRadius::same(1),
            color,
        );
    } else if response.hovered() {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS),
            palette.surface_hover,
        );
    }
    let text_color = if selected {
        palette.text
    } else {
        palette.secondary
    };
    let line = widgets::line(ui, name, font, text_color, text_width, 1);
    line.paint(
        ui,
        pos2(rect.left() + 11.0, rect.center().y - 8.0),
        text_color,
    );
    if count > 0 {
        let badge_rect = Rect::from_min_size(
            pos2(rect.left() + 11.0 + text_width + 4.0, rect.center().y - 8.0),
            vec2(size.x - text_width - 26.0, 16.0),
        );
        ui.painter()
            .rect_filled(badge_rect, CornerRadius::same(8), palette.surface);
        let digits = widgets::line(
            ui,
            &count.to_string(),
            theme::regular(11.0),
            palette.dim,
            badge_rect.width(),
            1,
        );
        digits.paint(
            ui,
            pos2(badge_rect.left() + 6.0, badge_rect.center().y - 7.0),
            palette.dim,
        );
    }
    let hint = if selected {
        "Showing this label".to_owned()
    } else {
        format!("Show {name}. Ctrl-click to open it beside this one")
    };
    (response.on_hover_text(hint), rect)
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

    /// Draws the tab bar once and reports where each tab ended up.
    fn tabs(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(420.0, 240.0))),
            events,
            ..Default::default()
        };
        let palette = app.palette;
        let mut output = ctx.run_ui(input, |ui| tab_bar(app, ui, 0, &palette));
        output.textures_delta.clear();
    }

    fn click(pos: egui::Pos2, modifiers: egui::Modifiers) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers,
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers,
            },
        ]
    }

    #[test]
    fn a_tab_click_picks_its_label_and_ctrl_click_opens_it_beside() {
        let root = std::env::temp_dir().join(format!("zapfast-tabs-{}", std::process::id()));
        let (mut app, _events) = App::headless(AppDirs::under(&root), Settings::default());
        app.settings.labels_as_tabs = true;
        app.labels = vec![label("label-1", "Work")];
        let ctx = egui::Context::default();
        app.attach(&ctx);
        tabs(&mut app, &ctx, Vec::new());
        let rect = ctx
            .data(|data| data.get_temp::<Rect>(tab_id(0, Some("label-1"))))
            .expect("the label tab is drawn");
        let pos = rect.center();
        tabs(&mut app, &ctx, click(pos, egui::Modifiers::NONE));
        assert!(
            app.actions.contains(&Action::SelectLabel {
                pane: 0,
                label: Some("label-1".into())
            }),
            "a plain click picks the label for this pane"
        );
        app.actions.clear();
        tabs(&mut app, &ctx, Vec::new());
        tabs(&mut app, &ctx, click(pos, egui::Modifiers::COMMAND));
        assert!(
            app.actions
                .contains(&Action::OpenLabelSplit(Some("label-1".into()))),
            "Ctrl-click opens the label beside the current one"
        );
    }

    #[test]
    fn tabs_stay_out_of_the_way_when_the_setting_is_off() {
        let root = std::env::temp_dir().join(format!("zapfast-tabs-off-{}", std::process::id()));
        let (mut app, _events) = App::headless(AppDirs::under(&root), Settings::default());
        app.labels = vec![label("label-1", "Work")];
        let ctx = egui::Context::default();
        app.attach(&ctx);
        tabs(&mut app, &ctx, Vec::new());
        assert!(
            ctx.data(|data| data.get_temp::<Rect>(tab_id(0, Some("label-1"))))
                .is_none(),
            "no tab bar without the setting"
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
}
