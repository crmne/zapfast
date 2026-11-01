//! New chat's New group steps: choosing the members, then naming the group.
//! The group info panel's Add members dialog is the choosing step alone.
//!
//! The draft lives in `Account::new_group`; this view edits only its name
//! and the dialog's search, and pushes `Action`s for everything else.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Rect, Sense, pos2, vec2};

use crate::app::App;
use crate::model::Action;
use crate::new_group::Stage;
use crate::theme::{self, Icon};

/// The id the new group's photo is drawn under before it has a chat.
const PREVIEW_ID: &str = "new-group@g.us";
const ROW_HEIGHT: f32 = 52.0;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    match app.new_group.as_ref().map(|draft| draft.stage) {
        Some(Stage::Choosing) => choosing(app, ui),
        Some(Stage::Naming) => naming(app, ui),
        None => {}
    }
}

/// The id of the rect a member row was drawn in, for tests.
#[cfg(any(test, feature = "demo"))]
pub fn member_row_id(id: &str) -> egui::Id {
    egui::Id::new(("new-group-member", id))
}

/// The dialog's heading, with Back before it and Close after it.
fn header(app: &mut App, ui: &mut egui::Ui, label: &str) {
    let palette = app.palette;
    let back = crate::i18n::gettext(app.locale, "Back");
    ui.horizontal(|ui| {
        if theme::icon_button(
            ui,
            Icon::ArrowLeft,
            16.0,
            palette.secondary,
            palette.text,
            &back,
        )
        .clicked()
        {
            app.actions.push(Action::NewGroupBack);
        }
        theme::text(ui, label, theme::bold(18.0), palette.text);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if theme::icon_button(
                ui,
                Icon::X,
                16.0,
                palette.secondary,
                palette.text,
                &crate::i18n::gettext(app.locale, "Close"),
            )
            .clicked()
            {
                app.actions.push(Action::CloseDialog);
            }
        });
    });
    ui.add_space(4.0);
}

fn choosing(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let adding = app
        .new_group
        .as_ref()
        .is_some_and(|draft| draft.adding.is_some());
    let busy = app.new_group.as_ref().is_some_and(|draft| draft.creating);
    let heading = if adding {
        crate::i18n::gettext(app.locale, "Add members")
    } else {
        crate::i18n::gettext(app.locale, "Add group members")
    };
    header(app, ui, &heading);
    let members = app
        .new_group
        .as_ref()
        .map(|draft| draft.members.clone())
        .unwrap_or_default();
    if !members.is_empty() {
        chips(app, ui, &members);
        ui.add_space(2.0);
    }
    let width = ui.available_width();
    let hint = crate::i18n::gettext(app.locale, "Search contacts");
    let search = super::widgets::search_field(
        ui,
        &palette,
        egui::Id::new("new-chat-search"),
        &mut app.new_chat_search,
        &hint,
        width,
    );
    if ui.memory(|memory| memory.focused().is_none()) {
        search.request_focus();
    }
    ui.add_space(4.0);
    let needle = crate::util::search_key(app.new_chat_search.trim());
    let mut contacts = super::dialogs::contact_choices(app, &needle);
    if let Some(draft) = app.new_group.as_ref() {
        // Adding offers only people outside the group.
        contacts.retain(|contact| draft.offers(&contact.id));
    }
    let max_height = (ui.ctx().content_rect().height() - 300.0).clamp(ROW_HEIGHT * 3.0, 420.0);
    egui::ScrollArea::vertical()
        .id_salt("new-group-contacts")
        .max_height(max_height)
        .auto_shrink([false, true])
        .show_rows(ui, ROW_HEIGHT, contacts.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for contact in &contacts[range] {
                member_row(app, ui, &contact.id, members.contains(&contact.id));
            }
        });
    if contacts.is_empty() {
        ui.add_space(8.0);
        theme::text(
            ui,
            crate::i18n::gettext(app.locale, "No matching contacts"),
            theme::regular(13.0),
            palette.secondary,
        );
    }
    ui.add_space(8.0);
    let ready = app.new_group.as_ref().is_some_and(|draft| {
        if adding {
            draft.can_add()
        } else {
            draft.can_advance()
        }
    });
    ui.horizontal(|ui| {
        if busy {
            theme::spinner(ui, 16.0, palette.accent);
        }
        theme::text(
            ui,
            crate::i18n::ngettext(
                app.locale,
                "{count} selected",
                "{count} selected",
                members.len() as u32,
            )
            .replace("{count}", &members.len().to_string()),
            theme::regular(13.5),
            palette.secondary,
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let next = if adding {
                crate::i18n::pgettext(app.locale, "add group members", "Add")
            } else {
                crate::i18n::gettext(app.locale, "Next")
            };
            let pick = crate::i18n::gettext(app.locale, "Choose at least one person");
            let clicked = ui
                .add_enabled_ui(ready, |ui| {
                    theme::pill_button(ui, &palette, &next, true).on_disabled_hover_text(pick)
                })
                .inner
                .clicked();
            if clicked && ready {
                app.actions.push(if adding {
                    Action::AddGroupMembers
                } else {
                    Action::NewGroupNext
                });
            }
        });
    });
}

/// The people picked so far, each a chip that lets them go when clicked.
fn chips(app: &mut App, ui: &mut egui::Ui, members: &[String]) {
    let palette = app.palette;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        for id in members {
            let name = app.display_name(id);
            let picture = app.avatar(id);
            let label = crate::i18n::gettext(app.locale, "Remove {name}").replace("{name}", &name);
            let line = super::widgets::line(ui, &name, theme::medium(13.0), palette.text, 140.0, 1);
            let avatar = 22.0;
            let size = vec2(4.0 + avatar + 6.0 + line.size().x + 6.0 + 12.0 + 8.0, 30.0);
            let (rect, response) = ui.allocate_exact_size(size, Sense::click());
            theme::reveal_focus(&response);
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
            if ui.is_rect_visible(rect) {
                let fill = if response.hovered() || response.has_focus() {
                    palette.surface_hover
                } else {
                    palette.surface
                };
                ui.painter().rect_filled(rect, CornerRadius::same(15), fill);
                let face = Rect::from_center_size(
                    pos2(rect.left() + 4.0 + avatar / 2.0, rect.center().y),
                    egui::Vec2::splat(avatar),
                );
                super::widgets::paint_avatar(ui, &palette, face, &name, id, picture.as_deref());
                line.paint(
                    ui,
                    pos2(face.right() + 6.0, rect.center().y - line.size().y / 2.0),
                    palette.text,
                );
                let cross = Rect::from_center_size(
                    pos2(rect.right() - 8.0 - 6.0, rect.center().y),
                    egui::Vec2::splat(12.0),
                );
                theme::paint_icon(ui, Icon::X, cross, 12.0, palette.secondary);
            }
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(&label)
                .clicked()
            {
                app.actions.push(Action::ToggleNewGroupMember(id.clone()));
            }
        }
    });
}

/// One contact with its check box. The row and the box both pick.
fn member_row(app: &mut App, ui: &mut egui::Ui, id: &str, picked: bool) {
    let palette = app.palette;
    let name = app.display_name(id);
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    theme::reveal_focus(&response);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            picked,
            &name,
        )
    });
    #[cfg(any(test, feature = "demo"))]
    ui.ctx()
        .data_mut(|data| data.insert_temp(member_row_id(id), rect));
    let mut toggled = false;
    if ui.is_rect_visible(rect) {
        if response.hovered() || picked {
            super::widgets::dialog_row_highlight(ui, rect, palette.surface_hover);
        }
        let check = Rect::from_center_size(
            pos2(rect.right() - 18.0, rect.center().y),
            egui::Vec2::splat(20.0),
        );
        toggled = super::widgets::round_check(ui, &palette, check, id, picked, true, &name);
        let avatar = Rect::from_center_size(
            pos2(rect.left() + 23.0, rect.center().y),
            egui::Vec2::splat(38.0),
        );
        let picture = app.avatar(id);
        super::widgets::paint_avatar(ui, &palette, avatar, &name, id, picture.as_deref());
        let left = rect.left() + 50.0;
        let width = rect.width() - 90.0;
        let detail = crate::model::phone_of(id).map(crate::util::phone);
        let line = super::widgets::line(ui, &name, theme::medium(14.5), palette.text, width, 1);
        match detail.filter(|phone| *phone != name) {
            Some(phone) => {
                let phone =
                    super::widgets::line(ui, &phone, theme::regular(12.5), palette.dim, width, 1);
                line.paint(ui, pos2(left, rect.top() + 8.0), palette.text);
                phone.paint(ui, pos2(left, rect.top() + 29.0), palette.dim);
            }
            None => line.paint(
                ui,
                pos2(left, rect.center().y - line.size().y / 2.0),
                palette.text,
            ),
        }
    }
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
        || toggled
    {
        app.actions
            .push(Action::ToggleNewGroupMember(id.to_owned()));
    }
}

fn naming(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let heading = crate::i18n::gettext(app.locale, "New group");
    header(app, ui, &heading);
    let Some(draft) = app.new_group.clone() else {
        return;
    };
    ui.add_space(4.0);
    // The photo, optional: the group dialog's picking and cropping.
    ui.vertical_centered(|ui| {
        super::widgets::avatar(
            ui,
            &palette,
            draft.subject(),
            PREVIEW_ID,
            96.0,
            draft.picture.as_deref(),
        );
    });
    ui.add_space(2.0);
    let add = crate::i18n::gettext(app.locale, "Add group photo");
    let change = crate::i18n::gettext(app.locale, "Change photo");
    let remove = crate::i18n::gettext(app.locale, "Remove photo");
    let labels: Vec<&str> = if draft.picture.is_some() {
        vec![change.as_ref(), remove.as_ref()]
    } else {
        vec![add.as_ref()]
    };
    let spacing = ui.spacing().item_spacing.x;
    let row: f32 = labels
        .iter()
        .map(|label| theme::soft_button_width(ui, label, true) + spacing)
        .sum::<f32>()
        - spacing;
    ui.horizontal(|ui| {
        ui.add_space(((ui.available_width() - row) / 2.0).max(0.0));
        ui.add_enabled_ui(!draft.creating, |ui| {
            if draft.picture.is_some() {
                if theme::soft_button(ui, &palette, Some(Icon::Image), &change, false).clicked() {
                    app.actions.push(Action::PickNewGroupPicture);
                }
                if theme::soft_button(ui, &palette, Some(Icon::Trash), &remove, false).clicked() {
                    app.actions.push(Action::RemoveNewGroupPicture);
                }
            } else if theme::soft_button(ui, &palette, Some(Icon::Image), &add, false).clicked() {
                app.actions.push(Action::PickNewGroupPicture);
            }
        });
    });
    ui.add_space(8.0);

    let hint = crate::i18n::gettext(app.locale, "Group name (required)");
    let format = egui::TextFormat::simple(theme::regular(16.0), palette.text);
    let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
        crate::bidi::layout_field(ui, text.as_str(), &format, wrap)
    };
    let mut name = draft.name.clone();
    let align = if crate::bidi::base_rtl(&name) {
        Align::RIGHT
    } else {
        Align::LEFT
    };
    let typed = name.chars().count();
    let field = Frame::new()
        .fill(palette.surface)
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let counter = super::widgets::line(
                    ui,
                    &format!("{typed}/{}", crate::model::GROUP_NAME_LIMIT),
                    theme::regular(12.0),
                    palette.dim,
                    80.0,
                    1,
                );
                let width = (ui.available_width() - counter.size().x - 8.0).max(40.0);
                let response = ui.add_enabled(
                    !draft.creating,
                    egui::TextEdit::singleline(&mut name)
                        .id(name_field_id())
                        .hint_text(
                            egui::RichText::new(hint.as_ref())
                                .color(palette.dim)
                                .font(theme::regular(16.0)),
                        )
                        .font(theme::regular(16.0))
                        .text_color(palette.text)
                        .frame(Frame::NONE)
                        .char_limit(crate::model::GROUP_NAME_LIMIT)
                        .desired_width(width)
                        .horizontal_align(align)
                        .layouter(&mut layouter),
                );
                let (spot, _) = ui.allocate_exact_size(counter.size(), Sense::hover());
                counter.paint(ui, spot.min, palette.dim);
                response
            })
            .inner
        });
    theme::focus_outline(
        ui,
        field.inner.id,
        field.response.rect,
        f32::from(theme::RADIUS),
    );
    if !draft.creating && ui.memory(|memory| memory.focused().is_none()) {
        field.inner.request_focus();
    }
    let submitted =
        field.inner.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
    if let Some(edited) = app.new_group.as_mut() {
        edited.name = name;
    }
    ui.add_space(8.0);

    let names: Vec<String> = draft
        .members
        .iter()
        .map(|id| app.display_name(id))
        .collect();
    theme::text(
        ui,
        crate::i18n::ngettext(
            app.locale,
            "{count} member besides you",
            "{count} members besides you",
            names.len() as u32,
        )
        .replace("{count}", &names.len().to_string()),
        theme::medium(13.5),
        palette.text,
    );
    let list = super::widgets::line(
        ui,
        &names.join(", "),
        theme::regular(13.0),
        palette.secondary,
        ui.available_width(),
        2,
    );
    let (spot, _) = ui.allocate_exact_size(list.size(), Sense::hover());
    list.paint(ui, spot.min, palette.secondary);
    ui.add_space(12.0);

    let ready = app
        .new_group
        .as_ref()
        .is_some_and(crate::new_group::Draft::can_create);
    ui.horizontal(|ui| {
        if draft.creating {
            theme::spinner(ui, 16.0, palette.accent);
            theme::text(
                ui,
                crate::i18n::gettext(app.locale, "Creating the group…"),
                theme::regular(12.5),
                palette.secondary,
            );
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let create = crate::i18n::gettext(app.locale, "Create");
            let needs_name = crate::i18n::gettext(app.locale, "Enter a name for the group.");
            let clicked = ui
                .add_enabled_ui(ready, |ui| {
                    theme::pill_button(ui, &palette, &create, true)
                        .on_disabled_hover_text(needs_name)
                })
                .inner
                .clicked();
            if (clicked || submitted) && ready {
                app.actions.push(Action::CreateGroup);
            }
            let back = crate::i18n::gettext(app.locale, "Back");
            if ui
                .add_enabled_ui(!draft.creating, |ui| {
                    theme::pill_button(ui, &palette, &back, false)
                })
                .inner
                .clicked()
            {
                app.actions.push(Action::NewGroupBack);
            }
        });
    });
}

/// The new group's name field.
pub fn name_field_id() -> egui::Id {
    egui::Id::new("new-group-name")
}
