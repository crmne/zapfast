//! The left panel: the chat list.

use egui::{Align, Frame, Layout, Margin, Rect, Sense, Vec2, pos2, vec2};

use crate::app::App;
use crate::backend::LinkStatus;
use crate::model::{Action, Chat, ChatFilter, Contact, Dialog, Message, Page};
use crate::theme::{self, Icon, Palette};

use super::focus::{Stop, TabStop};
use super::widgets;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let panel = egui::Panel::left("chats")
        .resizable(true)
        .default_size(app.settings.sidebar_width)
        .size_range(if theme::macos_chrome(ui.ctx()) {
            (theme::traffic_light_inset(ui.ctx()) + 210.0).max(280.0)..=520.0
        } else {
            260.0..=520.0
        })
        .show_separator_line(false)
        .frame(Frame::new().fill(palette.panel).inner_margin(Margin::ZERO));
    let response = panel.show(ui, |ui| {
        header(app, ui);
        list(app, ui);
    });
    let width = response.response.rect.width();
    if (width - app.settings.sidebar_width).abs() > 1.0 {
        app.settings.sidebar_width = width;
        app.actions.push(Action::SettingsChanged);
    }
    // Separate the panel from the conversation.
    let rect = response.response.rect;
    ui.painter().vline(
        rect.right(),
        rect.y_range(),
        egui::Stroke::new(1.0, palette.outline),
    );
}

fn header(app: &mut App, ui: &mut egui::Ui) {
    if theme::macos_chrome(ui.ctx()) {
        macos_header(app, ui);
        return;
    }
    let palette = app.palette;
    Frame::new()
        .inner_margin(Margin {
            left: 14,
            right: 10,
            top: 12,
            bottom: 8,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if app.show_archived || app.locked_folder {
                    if theme::icon_button(
                        ui,
                        Icon::ArrowLeft,
                        18.0,
                        palette.secondary,
                        palette.text,
                        "Back to chats",
                    )
                    .tab_stop(Stop::Back)
                    .clicked()
                    {
                        app.show_archived = false;
                        if app.locked_folder {
                            app.actions.push(Action::CloseLockedFolder);
                        }
                    }
                    theme::text(
                        ui,
                        if app.locked_folder {
                            "Locked chats"
                        } else {
                            "Archived"
                        },
                        theme::bold(20.0),
                        palette.text,
                    );
                } else {
                    let me = app.me.clone().unwrap_or_default();
                    let name = app.me_name.clone().unwrap_or_else(|| "You".to_owned());
                    let picture = app.avatar(&me);
                    let tooltip = match &app.me_about {
                        Some(about) => format!("{name}\n{about}"),
                        None => name.clone(),
                    };
                    let response = widgets::clickable_avatar(
                        ui,
                        &palette,
                        &name,
                        &me,
                        34.0,
                        picture.as_deref(),
                        "Your profile and settings",
                    )
                    .tab_stop(Stop::Profile)
                    .on_hover_text(tooltip)
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if response.clicked() {
                        app.actions.push(Action::Open(Page::Settings));
                    }
                    ui.add_space(2.0);
                    theme::text(
                        ui,
                        crate::i18n::gettext(app.locale, "Chats"),
                        theme::bold(20.0),
                        palette.text,
                    );
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if theme::icon_button(
                        ui,
                        Icon::Settings,
                        18.0,
                        palette.secondary,
                        palette.text,
                        "Settings (Ctrl+,)",
                    )
                    .tab_stop(Stop::Settings)
                    .clicked()
                    {
                        app.actions.push(Action::Open(Page::Settings));
                    }
                    if theme::icon_button(
                        ui,
                        Icon::SquarePen,
                        18.0,
                        palette.secondary,
                        palette.text,
                        "New chat",
                    )
                    .tab_stop(Stop::NewChat)
                    .clicked()
                    {
                        app.actions
                            .push(Action::ShowDialog(crate::model::Dialog::NewChat));
                    }
                    if theme::icon_button(
                        ui,
                        Icon::PanelLeft,
                        18.0,
                        palette.secondary,
                        palette.text,
                        "Hide the chat list (Ctrl+B)",
                    )
                    .tab_stop(Stop::Sidebar)
                    .clicked()
                    {
                        app.actions.push(Action::ToggleSidebar);
                    }
                });
            });
            ui.add_space(6.0);
            let id = egui::Id::new("chat-search");
            let width = ui.available_width();
            let mut text = app.search.clone();
            let response = widgets::search_field(
                ui,
                &palette,
                id,
                &mut text,
                crate::i18n::gettext(app.locale, "Search").as_ref(),
                width,
            )
            .tab_stop(Stop::Search);
            if text != app.search {
                app.actions.push(Action::Search(text));
            }
            if app.focus_search {
                app.focus_search = false;
                response.request_focus();
            }
            filter_chips(app, ui);
        });
}

fn macos_header(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let inset = theme::traffic_light_inset(ui.ctx());
    let mut drag = ui.max_rect();
    drag.min.x += inset;
    drag.max.y = drag.min.y + 60.0;
    super::titlebar_drag(ui, drag);
    Frame::new()
        .inner_margin(Margin::symmetric(14, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_min_height(44.0);
                ui.add_space((inset - 14.0).max(0.0));
                if app.show_archived || app.locked_folder {
                    if theme::icon_button(
                        ui,
                        Icon::ArrowLeft,
                        18.0,
                        palette.secondary,
                        palette.text,
                        "Back to chats",
                    )
                    .tab_stop(Stop::Back)
                    .clicked()
                    {
                        app.show_archived = false;
                        if app.locked_folder {
                            app.actions.push(Action::CloseLockedFolder);
                        }
                    }
                    theme::text(
                        ui,
                        if app.locked_folder {
                            "Locked chats"
                        } else {
                            "Archived"
                        },
                        theme::bold(16.0),
                        palette.text,
                    );
                } else {
                    theme::text(
                        ui,
                        crate::i18n::gettext(app.locale, "Chats"),
                        theme::bold(20.0),
                        palette.text,
                    );
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if theme::icon_button(
                        ui,
                        Icon::SquarePen,
                        18.0,
                        palette.secondary,
                        palette.text,
                        "New chat (⌘N)",
                    )
                    .tab_stop(Stop::NewChat)
                    .clicked()
                    {
                        app.actions.push(Action::ShowDialog(Dialog::NewChat));
                    }
                    if theme::icon_button(
                        ui,
                        Icon::PanelLeft,
                        18.0,
                        palette.secondary,
                        palette.text,
                        "Hide the chat list (⌘B)",
                    )
                    .tab_stop(Stop::Sidebar)
                    .clicked()
                    {
                        app.actions.push(Action::ToggleSidebar);
                    }
                });
            });
            ui.add_space(6.0);
            let mut text = app.search.clone();
            let response = widgets::search_field(
                ui,
                &palette,
                egui::Id::new("chat-search"),
                &mut text,
                crate::i18n::gettext(app.locale, "Search").as_ref(),
                ui.available_width(),
            )
            .tab_stop(Stop::Search);
            if text != app.search {
                app.actions.push(Action::Search(text));
            }
            if app.focus_search {
                app.focus_search = false;
                response.request_focus();
            }
            filter_chips(app, ui);
        });
}

/// Stable main-list row id used by interaction tests.
pub fn chat_row_id(chat: &str) -> egui::Id {
    egui::Id::new(("chat-row", chat))
}

/// Stable filter-chip id used by interaction tests.
pub fn filter_chip_id(filter: ChatFilter) -> egui::Id {
    egui::Id::new(("chat-filter", filter as u8))
}

/// Filter chips under the search field. Search lists every match, so the
/// chips hide there.
fn filter_chips(app: &mut App, ui: &mut egui::Ui) {
    if !app.locked_folder_open() && !app.search.trim().is_empty() {
        return;
    }
    let palette = app.palette;
    ui.add_space(8.0);
    egui::ScrollArea::horizontal()
        .id_salt("chat-filters")
        .animated(false)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 6.0);
                for filter in ChatFilter::EVERY {
                    let count = match filter {
                        ChatFilter::All => 0,
                        _ => app.unread_chats(filter),
                    };
                    let selected = !app.locked_folder_open()
                        && !app.show_archived
                        && app.chat_filter == filter;
                    let chip = widgets::filter_chip(
                        ui,
                        &palette,
                        filter.label(app.locale).as_ref(),
                        count,
                        selected,
                    )
                    .tab_stop(match filter {
                        ChatFilter::All => Stop::All,
                        ChatFilter::Unread => Stop::Unread,
                        ChatFilter::Private => Stop::Private,
                        ChatFilter::Groups => Stop::Groups,
                        ChatFilter::Channels => Stop::Channels,
                    });
                    // Store the chip rect for interaction tests.
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(filter_chip_id(filter), chip.rect));
                    let chip = if filter == ChatFilter::Channels {
                        let now = crate::util::now();
                        let all_muted = app
                            .chats
                            .iter()
                            .filter(|chat| chat.is_channel())
                            .all(|chat| chat.muted(now));
                        chip.context_menu(|ui| {
                            let (icon, label) = if all_muted {
                                (Icon::Bell, "Unmute all channels")
                            } else {
                                (Icon::BellOff, "Mute all channels")
                            };
                            if widgets::menu_item(ui, &palette, Some(icon), label) {
                                app.actions.push(Action::MuteAllChannels(!all_muted));
                                ui.close();
                            }
                        });
                        chip
                    } else {
                        chip
                    };
                    if chip.clicked() {
                        // A second click on the active chip returns to every chat.
                        let next = if selected { ChatFilter::All } else { filter };
                        app.actions.push(Action::SetChatFilter(next));
                    }
                }
                if app.archived_count() > 0 || app.show_archived {
                    let selected = app.show_archived;
                    let chip = widgets::filter_chip(
                        ui,
                        &palette,
                        crate::i18n::gettext(app.locale, "Archived").as_ref(),
                        app.archived_unread(),
                        selected,
                    )
                    .tab_stop(Stop::Archived);
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(egui::Id::new("archived-chip"), chip.rect);
                    });
                    if chip.clicked() {
                        app.actions.push(Action::ShowArchived(!selected));
                    }
                }
                if app.locked_count() > 0 || app.locked_folder_open() {
                    let selected = app.locked_folder_open();
                    let chip = widgets::filter_chip(
                        ui,
                        &palette,
                        crate::i18n::gettext(app.locale, "Locked").as_ref(),
                        0,
                        selected,
                    )
                    .tab_stop(Stop::Locked)
                    .on_hover_text("Open locked chats with your local code");
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(egui::Id::new("locked-chip"), chip.rect));
                    if chip.clicked() {
                        app.actions.push(Action::OpenLockedFolder);
                    }
                } else {
                    ui.ctx()
                        .data_mut(|data| data.remove::<egui::Rect>(egui::Id::new("locked-chip")));
                }
                ui.add_space(4.0);
            })
        });
}

fn list(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    if app.locked_folder_open() {
        locked_list(app, ui);
        return;
    }
    if app.secret_code_matched() {
        // Typing the secret code hides every other result: the folder entry
        // is all the search reveals until it is clicked.
        locked_entry(app, ui);
        return;
    }
    if !app.search.trim().is_empty() {
        results(app, ui);
        return;
    }
    let chats: Vec<Chat> = app.visible_chats().into_iter().cloned().collect();
    if chats.is_empty() {
        let (title, body) = if app.show_archived {
            ("Nothing archived", "Archived chats appear here.")
        } else if app.chat_filter != ChatFilter::All {
            let title = match app.chat_filter {
                ChatFilter::Unread => "No unread chats",
                ChatFilter::Private => "No private chats",
                ChatFilter::Channels => "No channels",
                _ => "No groups",
            };
            (title, "Choose All to see every chat.")
        } else if app.syncing {
            ("Loading your chats", "Receiving history from your phone.")
        } else {
            (
                "No chats yet",
                "Use New chat to message a contact or yourself.",
            )
        };
        widgets::empty_state(ui, &palette, Icon::MessageCircle, title, body);
        return;
    }
    let row_height = theme::ROW_HEIGHT;
    let total = chats.len();
    let mut scroll_area = egui::ScrollArea::vertical()
        .id_salt("chat-list")
        .auto_shrink([false, false]);
    let target_row = app
        .scroll_chat_into_view
        .as_ref()
        .and_then(|target| chats.iter().position(|chat| chat.id == *target));
    if let Some(target_row) = target_row {
        let id = ui.make_persistent_id(egui::IdSalt::new("chat-list"));
        let current = egui::scroll_area::State::load(ui.ctx(), id)
            .unwrap_or_default()
            .offset
            .y;
        let offset = row_scroll_offset(
            current,
            ui.available_height(),
            target_row,
            row_height,
            ui.spacing().item_spacing.y,
        );
        scroll_area = scroll_area.vertical_scroll_offset(offset);
        app.scroll_chat_into_view = None;
    }
    scroll_area.show_rows(ui, row_height, total, |ui, range| {
        for index in range {
            let chat = &chats[index];
            // Key by chat so an open menu survives list reordering.
            let response = ui
                .push_id(("chat", &chat.id), |ui| row(app, ui, chat))
                .inner;
            // Store the row rect for interaction tests.
            ui.ctx()
                .data_mut(|data| data.insert_temp(chat_row_id(&chat.id), response.rect));
            #[cfg(test)]
            ui.ctx().data_mut(|data| {
                data.insert_temp(chat_row_id(&chat.id).with("widget"), response.id);
            });
            if response.clicked() && !app.show_archived {
                app.actions.push(Action::KeepUnread(chat.id.clone()));
            }
        }
    });
}

/// The row the secret code reveals: the only thing the search then shows.
fn locked_entry(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let count = app.locked_count();
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), theme::ROW_HEIGHT),
        Sense::click(),
    );
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter().rect_filled(rect, 0.0, palette.surface_hover);
        }
        let icon_rect =
            Rect::from_center_size(pos2(rect.left() + 38.0, rect.center().y), Vec2::splat(22.0));
        Icon::LockOpen
            .image(palette.accent, 22.0)
            .paint_at(ui, icon_rect);
        ui.painter().text(
            pos2(rect.left() + 76.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            "Locked chats",
            theme::medium(14.5),
            palette.text,
        );
        ui.painter().text(
            pos2(rect.right() - 16.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            count.to_string(),
            theme::regular(12.5),
            palette.accent,
        );
        ui.painter().hline(
            (rect.left() + 76.0)..=rect.right(),
            rect.bottom() - 0.5,
            egui::Stroke::new(1.0, palette.outline),
        );
    }
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        app.actions.push(Action::OpenLockedFolder);
    }
}

/// The authenticated folder, including archived chats and local title search.
fn locked_list(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let chats = app.visible_chats();
    if chats.is_empty() {
        widgets::empty_state(
            ui,
            &palette,
            Icon::LockOpen,
            "No locked chats",
            "Lock a chat from its context menu to hide it here.",
        );
        return;
    }
    let chats: Vec<Chat> = chats.into_iter().cloned().collect();
    egui::ScrollArea::vertical()
        .id_salt("locked-chats")
        .auto_shrink([false, false])
        .show_rows(ui, theme::ROW_HEIGHT, chats.len(), |ui, range| {
            for chat in &chats[range] {
                ui.push_id(("chat", &chat.id), |ui| row(app, ui, chat));
            }
        });
}

/// Returns the smallest offset that fully reveals a fixed-height row.
fn row_scroll_offset(
    current: f32,
    viewport_height: f32,
    row: usize,
    row_height: f32,
    spacing: f32,
) -> f32 {
    let top = row as f32 * (row_height + spacing);
    let bottom = top + row_height;
    if top < current {
        top
    } else if bottom > current + viewport_height {
        (bottom - viewport_height).max(0.0)
    } else {
        current
    }
}

/// Search results grouped into chats, messages, and contacts.
fn results(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let chats: Vec<Chat> = app.visible_chats().into_iter().cloned().collect();
    let hits: Vec<Message> = app.search_hits.clone();
    let contacts: Vec<Contact> = app.matching_contacts().into_iter().cloned().collect();
    if chats.is_empty() && hits.is_empty() && contacts.is_empty() {
        widgets::empty_state(
            ui,
            &palette,
            Icon::Search,
            "No results",
            "Try another name, number, or message text.",
        );
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("search-results")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if !chats.is_empty() {
                section(ui, &palette, "Chats");
                for chat in &chats {
                    let reveal = app.scroll_chat_into_view.as_deref() == Some(chat.id.as_str());
                    let response = ui
                        .push_id(("chat", &chat.id), |ui| row(app, ui, chat))
                        .inner;
                    if reveal {
                        response.scroll_to_me(None);
                        app.scroll_chat_into_view = None;
                    }
                }
            }
            if !hits.is_empty() {
                section(ui, &palette, "Messages");
                for hit in &hits {
                    ui.push_id(("hit", &hit.chat, &hit.id), |ui| hit_row(app, ui, hit));
                }
            }
            if !contacts.is_empty() {
                section(ui, &palette, "Contacts");
                for contact in &contacts {
                    ui.push_id(("contact", &contact.id), |ui| contact_row(app, ui, contact));
                }
            }
            ui.add_space(8.0);
        });
}

fn section(ui: &mut egui::Ui, palette: &Palette, label: &str) {
    ui.add_space(10.0);
    Frame::new()
        .inner_margin(Margin {
            left: 14,
            right: 14,
            top: 0,
            bottom: 4,
        })
        .show(ui, |ui| {
            theme::text(ui, label, theme::semibold(12.5), palette.accent);
        });
}

/// A message search result. Clicking it opens the chat at that message.
fn hit_row(app: &mut App, ui: &mut egui::Ui, hit: &Message) {
    let palette = app.palette;
    let title = match app.chat(&hit.chat) {
        Some(chat) => app.chat_title(&chat.clone()),
        None => app.display_name_or(&hit.chat, None),
    };
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), theme::ROW_HEIGHT),
        Sense::click(),
    );
    theme::reveal_focus(&response);
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter().rect_filled(rect, 0.0, palette.surface_hover);
        }
        let avatar_rect =
            Rect::from_center_size(pos2(rect.left() + 38.0, rect.center().y), Vec2::splat(48.0));
        let picture = app.avatar(&hit.chat);
        widgets::paint_avatar(
            ui,
            &palette,
            avatar_rect,
            &title,
            &hit.chat,
            picture.as_deref(),
        );
        let left = rect.left() + 76.0;
        let right = rect.right() - 14.0;
        let stamp_galley = ui.painter().layout_no_wrap(
            crate::util::chat_stamp(app.locale, hit.timestamp),
            theme::regular(11.5),
            palette.dim,
        );
        let name_top = rect.top() + 14.0;
        ui.painter().galley(
            pos2(right - stamp_galley.size().x, name_top + 1.0),
            stamp_galley.clone(),
            palette.dim,
        );
        let name_width = (right - stamp_galley.size().x - 8.0 - left).max(0.0);
        let name = widgets::line(ui, &title, theme::medium(14.5), palette.text, name_width, 1);
        name.paint(ui, pos2(left, name_top), palette.text);
        // Show the sender for group messages.
        let line_y = rect.top() + 38.0;
        let mut x = left;
        if hit.from_me {
            let who = widgets::line(
                ui,
                "You: ",
                theme::regular(13.0),
                palette.dim,
                (right - x) * 0.5,
                1,
            );
            who.paint(ui, pos2(x, line_y), palette.dim);
            x += who.size().x;
        } else if crate::model::ChatKind::from_id(&hit.chat) == crate::model::ChatKind::Group {
            let sender = app.display_name_or(&hit.sender, hit.sender_name.as_deref());
            let first = sender.split_whitespace().next().unwrap_or(&sender);
            let who = widgets::line(
                ui,
                &format!("{first}: "),
                theme::regular(13.0),
                palette.dim,
                (right - x) * 0.5,
                1,
            );
            who.paint(ui, pos2(x, line_y), palette.dim);
            x += who.size().x;
        }
        let words = widgets::line(
            ui,
            &crate::markup::plain(&app.resolve_mention_tokens(&hit.summary()), &[]),
            theme::regular(13.0),
            palette.dim,
            (right - x).max(0.0),
            1,
        );
        words.paint(ui, pos2(x, line_y), palette.dim);
        ui.painter().hline(
            left..=rect.right(),
            rect.bottom() - 0.5,
            egui::Stroke::new(1.0, palette.outline),
        );
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        app.actions.push(Action::OpenMessage {
            chat: hit.chat.clone(),
            message: hit.id.clone(),
        });
    }
}

/// A contact without a chat. Clicking starts one.
pub(super) fn contact_row(app: &mut App, ui: &mut egui::Ui, contact: &Contact) {
    let palette = app.palette;
    let name = app.display_name(&contact.id);
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), theme::ROW_HEIGHT),
        Sense::click(),
    );
    theme::reveal_focus(&response);
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter().rect_filled(rect, 0.0, palette.surface_hover);
        }
        let avatar_rect =
            Rect::from_center_size(pos2(rect.left() + 38.0, rect.center().y), Vec2::splat(48.0));
        let picture = app.avatar(&contact.id);
        widgets::paint_avatar(
            ui,
            &palette,
            avatar_rect,
            &name,
            &contact.id,
            picture.as_deref(),
        );
        let left = rect.left() + 76.0;
        let name_line = widgets::line(
            ui,
            &name,
            theme::medium(14.5),
            palette.text,
            rect.right() - 14.0 - left,
            1,
        );
        name_line.paint(ui, pos2(left, rect.top() + 14.0), palette.text);
        if let Some(phone) = crate::model::phone_of(&contact.id) {
            let phone_line = widgets::line(
                ui,
                &crate::util::phone(phone),
                theme::regular(13.0),
                palette.dim,
                rect.right() - 14.0 - left,
                1,
            );
            phone_line.paint(ui, pos2(left, rect.top() + 38.0), palette.dim);
        }
        ui.painter().hline(
            left..=rect.right(),
            rect.bottom() - 0.5,
            egui::Stroke::new(1.0, palette.outline),
        );
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        app.actions.push(Action::StartChat {
            id: contact.id.clone(),
            name,
        });
    }
}

fn row(app: &mut App, ui: &mut egui::Ui, chat: &Chat) -> egui::Response {
    let palette = app.palette;
    let title = app.chat_title(chat);
    let selected = app.open_chat.as_deref() == Some(chat.id.as_str());
    let now = crate::util::now();
    let muted = chat.muted(now);
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), theme::ROW_HEIGHT),
        Sense::click(),
    );
    theme::reveal_focus(&response);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            format!("{title}, {} unread messages", chat.unread),
        )
    });
    if ui.is_rect_visible(rect) {
        if selected {
            ui.painter().rect_filled(rect, 0.0, palette.surface_active);
        } else if response.hovered() {
            ui.painter().rect_filled(rect, 0.0, palette.surface_hover);
        }
        let avatar_rect =
            Rect::from_center_size(pos2(rect.left() + 38.0, rect.center().y), Vec2::splat(48.0));
        let picture = app.avatar(&chat.id);
        widgets::paint_avatar(
            ui,
            &palette,
            avatar_rect,
            &title,
            &chat.id,
            picture.as_deref(),
        );
        if chat.ephemeral_expiration.is_some() {
            widgets::paint_disappearing_badge(ui, &palette, avatar_rect);
        }

        let left = rect.left() + 76.0;
        let right = rect.right() - 14.0;
        let stamp = if chat.last_activity > 0 {
            crate::util::chat_stamp(app.locale, chat.last_activity)
        } else {
            String::new()
        };
        let unread = chat.unread > 0;
        let stamp_color = if unread && !muted {
            palette.accent
        } else {
            palette.dim
        };
        let stamp_galley = ui
            .painter()
            .layout_no_wrap(stamp, theme::regular(11.5), stamp_color);
        let name_top = rect.top() + 14.0;
        ui.painter().galley(
            pos2(right - stamp_galley.size().x, name_top + 1.0),
            stamp_galley.clone(),
            stamp_color,
        );
        let name_width = (right - stamp_galley.size().x - 8.0 - left).max(0.0);
        let name_font = if unread {
            theme::semibold(14.5)
        } else {
            theme::medium(14.5)
        };
        let name = widgets::line(ui, &title, name_font, palette.text, name_width, 1);
        name.paint(ui, pos2(left, name_top), palette.text);

        // Leave room for badges beside the latest-message preview.
        let mut badge_right = right;
        let line_y = rect.top() + 38.0;
        if unread {
            let width = widgets::badge(
                ui,
                &palette,
                pos2(badge_right - 10.0, line_y + 8.0),
                chat.unread,
                muted,
            );
            badge_right -= width + 6.0;
        }
        if muted {
            let icon_rect =
                Rect::from_center_size(pos2(badge_right - 8.0, line_y + 8.0), Vec2::splat(15.0));
            Icon::VolumeX
                .image(palette.dim, 15.0)
                .paint_at(ui, icon_rect);
            badge_right -= 20.0;
        }
        if chat.pinned {
            let icon_rect =
                Rect::from_center_size(pos2(badge_right - 8.0, line_y + 8.0), Vec2::splat(14.0));
            Icon::Pin.image(palette.dim, 14.0).paint_at(ui, icon_rect);
            badge_right -= 20.0;
        }
        let mut x = left;
        let typing = app.typing_in(&chat.id);
        let preview_color = if unread && !muted {
            palette.secondary
        } else {
            palette.dim
        };
        let preview = if !typing.is_empty() {
            let who = if chat.is_group() {
                format!("{} is typing…", typing[0].1.trim_start_matches('~'))
            } else {
                "typing…".to_owned()
            };
            widgets::line(
                ui,
                &who,
                theme::medium(13.0),
                palette.accent,
                badge_right - x,
                1,
            )
        } else if let Some(last) = &chat.last {
            if last.from_me {
                let tick_rect =
                    Rect::from_center_size(pos2(x + 8.0, line_y + 8.0), Vec2::splat(16.0));
                widgets::ticks(ui, &palette, tick_rect, last.status);
                x += 20.0;
            } else if chat.is_group() {
                let sender = app.display_name_or(&last.sender, last.sender_name.as_deref());
                let first = sender.split_whitespace().next().unwrap_or(&sender);
                let sender = widgets::line(
                    ui,
                    &format!("{first}: "),
                    theme::regular(13.0),
                    preview_color,
                    (badge_right - x) * 0.5,
                    1,
                );
                let width = sender.size().x;
                sender.paint(ui, pos2(x, line_y), preview_color);
                x += width;
            }
            widgets::line(
                ui,
                &crate::markup::plain(&app.resolve_mention_tokens(&last.summary), &[]),
                theme::regular(13.0),
                preview_color,
                (badge_right - x).max(0.0),
                1,
            )
        } else {
            widgets::line(ui, "", theme::regular(13.0), preview_color, 1.0, 1)
        };
        preview.paint(ui, pos2(x, line_y), preview_color);
        ui.painter().hline(
            left..=rect.right(),
            rect.bottom() - 0.5,
            egui::Stroke::new(1.0, palette.outline),
        );
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        app.actions.push(Action::OpenChat(chat.id.clone()));
    }
    let menu_palette = palette;
    let menu_width = widgets::menu_width(
        ui,
        &[
            "Mark as read",
            "Pin to top",
            "Unarchive",
            "Mute for 8 hours",
            "Mute for a week",
            "Mute indefinitely",
            "Unlock chat",
            "Copy number",
        ],
        true,
    )
    .max(190.0);
    let popup = egui::Popup::context_menu(&response)
        .width(menu_width)
        .frame(widgets::menu_frame(&menu_palette));
    #[cfg(any(test, feature = "demo"))]
    let popup = if app.open_chat_menu.as_deref() == Some(chat.id.as_str()) {
        popup
            .open_memory(Some(egui::SetOpenCommand::Bool(true)))
            .at_position(response.rect.left_top() + vec2(12.0, 8.0))
    } else {
        popup
    };
    popup.show(|ui| context_menu(app, ui, chat, &menu_palette));
    response
}

fn context_menu(app: &mut App, ui: &mut egui::Ui, chat: &Chat, palette: &Palette) {
    if chat.unread > 0 && widgets::menu_item(ui, palette, Some(Icon::CheckCheck), "Mark as read") {
        app.actions.push(Action::MarkRead(chat.id.clone()));
    }
    if widgets::menu_item(
        ui,
        palette,
        Some(if chat.pinned { Icon::PinOff } else { Icon::Pin }),
        if chat.pinned { "Unpin" } else { "Pin to top" },
    ) {
        app.actions
            .push(Action::SetPinned(chat.id.clone(), !chat.pinned));
    }
    if widgets::menu_item(
        ui,
        palette,
        Some(Icon::Archive),
        if chat.archived {
            "Unarchive"
        } else {
            "Archive"
        },
    ) {
        app.actions
            .push(Action::SetArchived(chat.id.clone(), !chat.archived));
    }
    let now = crate::util::now();
    if chat.muted(now) {
        if widgets::menu_item(ui, palette, Some(Icon::Bell), "Unmute") {
            app.actions.push(Action::SetMuted(chat.id.clone(), None));
        }
    } else {
        for (label, until) in [
            ("Mute for 8 hours", Some(now + 8 * 3600)),
            ("Mute for a week", Some(now + 7 * 86_400)),
            ("Mute indefinitely", Some(0)),
        ] {
            if widgets::menu_item(ui, palette, Some(Icon::BellOff), label) {
                app.actions.push(Action::SetMuted(chat.id.clone(), until));
            }
        }
    }
    sound_menu(app, ui, palette, chat);
    if widgets::menu_item(
        ui,
        palette,
        Some(if chat.locked {
            Icon::LockOpen
        } else {
            Icon::Lock
        }),
        if chat.locked {
            "Unlock chat"
        } else {
            "Lock chat"
        },
    ) {
        app.actions.push(if chat.locked {
            Action::SetLocked(chat.id.clone(), false)
        } else {
            Action::ShowDialog(Dialog::ConfirmLockChat(chat.id.clone()))
        });
    }
    widgets::menu_separator(ui, palette);
    if let Some(phone) = chat.phone()
        && widgets::menu_item(ui, palette, Some(Icon::Copy), "Copy number")
    {
        app.actions.push(Action::CopyText(format!("+{phone}")));
    }
    if widgets::menu_item(ui, palette, Some(Icon::Info), "Info") {
        app.actions
            .push(Action::ShowDialog(Dialog::ChatInfo(chat.id.clone())));
    }
    // Deleting reaches the phone, so it waits for a connection.
    let connected = matches!(app.link, LinkStatus::Connected);
    if widgets::menu_item_enabled(ui, palette, Some(Icon::Trash), "Delete chat", connected) {
        app.actions
            .push(Action::ShowDialog(Dialog::ConfirmDeleteChat(
                chat.id.clone(),
            )));
    }
}

/// A chat's own notification sound, overriding Settings for this chat.
fn sound_menu(app: &mut App, ui: &mut egui::Ui, palette: &Palette, chat: &Chat) {
    use crate::settings::NotificationSound;
    ui.menu_button("Notification sound", |ui| {
        let current = chat.notification_sound.clone();
        for (sound, label) in [
            (None, "Default"),
            (Some(NotificationSound::Chime), "Chime"),
            (Some(NotificationSound::Ripple), "Ripple"),
            (Some(NotificationSound::System), "System sound"),
            (Some(NotificationSound::None), "No sound"),
        ] {
            let checked = current == sound;
            if widgets::menu_item(ui, palette, checked.then_some(Icon::Check), label) {
                app.actions.push(Action::SetChatSound {
                    chat: chat.id.clone(),
                    sound,
                });
                ui.close();
            }
        }
        if let Some(NotificationSound::Custom(path)) = &current {
            let name = path.file_name().map_or_else(
                || "Custom".to_owned(),
                |name| name.to_string_lossy().into_owned(),
            );
            widgets::menu_item(ui, palette, Some(Icon::Check), &name);
        }
        if widgets::menu_item(ui, palette, None, "Choose a file…") {
            app.actions.push(Action::PickChatSound(chat.id.clone()));
            ui.close();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::AppDirs;
    use crate::settings::Settings;

    #[test]
    fn chat_context_menu_stays_compact_in_wide_windows() {
        for width in [360.0, 1180.0, 2000.0] {
            let directory = tempfile::tempdir().unwrap();
            let (mut app, _events) =
                App::headless(AppDirs::under(directory.path()), Settings::default());
            let chat = Chat::new("fixture@g.us".into(), "Fixture group".into());
            let ctx = egui::Context::default();
            app.attach(&ctx);
            let mut frame = |events| {
                let mut response = None;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            egui::Pos2::ZERO,
                            vec2(width, 800.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| response = Some(row(&mut app, ui, &chat)),
                );
                output.textures_delta.clear();
                response.unwrap()
            };
            let response = frame(vec![]);
            let position = response.rect.left_center() + vec2(20.0, 0.0);
            for pressed in [true, false] {
                frame(vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Secondary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
            }
            for _ in 0..3 {
                frame(vec![]);
            }
            let popup = response.id.with("popup");
            assert!(egui::Popup::is_id_open(&ctx, popup));
            let rect = ctx.read_response(popup).expect("chat context menu").rect;
            assert!(
                (190.0..=240.0).contains(&rect.width()),
                "a {width}-point window produced a {}-point menu",
                rect.width()
            );
        }
    }

    #[test]
    fn alt_navigation_scrolls_the_destination_chat_into_view() {
        let root = std::env::temp_dir().join(format!(
            "zapfast-chat-list-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let (mut app, _events) = App::headless(AppDirs::under(&root), Settings::default());
        let mut first = String::new();
        let mut last = String::new();
        for index in 0..24 {
            let id = format!("49170000{index:04}@s.whatsapp.net");
            let mut chat = Chat::new(id.clone(), format!("Chat {index:02}"));
            chat.last_activity = 100 - i64::from(index);
            if index == 0 {
                first.clone_from(&id);
            }
            last.clone_from(&id);
            app.chats.push(chat);
        }
        app.open_chat = Some(first);

        let ctx = egui::Context::default();
        app.attach(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(360.0, 240.0))),
            events: vec![egui::Event::Key {
                key: egui::Key::ArrowUp,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::ALT,
            }],
            ..Default::default()
        };
        let mut offset = 0.0;
        let mut output = ctx.run_ui(input, |ui| {
            super::super::keys::handle(&mut app, ui.ctx());
            let scroll_id = ui.make_persistent_id(egui::IdSalt::new("chat-list"));
            list(&mut app, ui);
            offset = egui::scroll_area::State::load(ui.ctx(), scroll_id)
                .expect("chat-list scroll state")
                .offset
                .y;
        });
        output.textures_delta.clear();

        assert!(
            app.actions.contains(&Action::OpenChat(last)),
            "Alt+Up wraps to the last visible chat"
        );
        assert!(app.scroll_chat_into_view.is_none(), "reveal was consumed");
        assert!(offset > 0.0, "the list moved down to reveal the last row");
    }
}
