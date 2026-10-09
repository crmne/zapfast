//! The left panel: the chat list.

use egui::{Align, Frame, Key, Layout, Margin, Rect, Sense, Vec2, pos2, vec2};

use crate::app::App;
use crate::backend::LinkStatus;
use crate::model::{Action, Chat, ChatFilter, Contact, Dialog, Message, Page};
use crate::theme::{self, Icon, Palette};

use super::focus::{Stop, TabStop};
use super::labels;
use super::widgets;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    search_keyboard(app, ui);
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
    // Separate the panel from the conversation, as the header's line does.
    widgets::paint_edge_beside(ui, &palette, response.response.rect);
}

/// The chat list across the whole window, when the window is too narrow to
/// hold it beside a conversation. Opening a chat replaces it.
pub fn full_show(app: &mut App, ui: &mut egui::Ui) {
    search_keyboard(app, ui);
    let palette = app.palette;
    egui::CentralPanel::default()
        .frame(Frame::new().fill(palette.panel).inner_margin(Margin::ZERO))
        .show(ui, |ui| {
            header(app, ui);
            list(app, ui);
        });
}

/// Walks matching chats while the global search field keeps keyboard focus.
/// Enter leaves search and opens the reached chat ready for typing. A
/// disabled list (beneath a narrow window's sliding chat) leaves the keys
/// to the view coming in.
fn search_keyboard(app: &mut App, ui: &egui::Ui) {
    let field = egui::Id::new("chat-search");
    if !ui.is_enabled()
        || app.search.trim().is_empty()
        || app.locked_folder_open()
        || app.secret_code_matched()
        || !ui.memory(|memory| memory.has_focus(field))
    {
        return;
    }
    let (down, up, enter) = ui.input_mut(|input| {
        (
            super::keys::take_plain(input, Key::ArrowDown),
            super::keys::take_plain(input, Key::ArrowUp),
            super::keys::take_plain(input, Key::Enter),
        )
    });
    if !down && !up && !enter {
        return;
    }
    let chats: Vec<_> = app
        .visible_chats()
        .into_iter()
        .map(|chat| chat.id.clone())
        .collect();
    if chats.is_empty() {
        app.search_selected = None;
        return;
    }
    let before = app.search_selected.clone();
    let mut index = before
        .as_ref()
        .and_then(|selected| chats.iter().position(|chat| chat == selected));
    if down {
        index = Some(index.map_or(0, |index| (index + 1).min(chats.len() - 1)));
    }
    if up {
        index = index.map(|index| index.saturating_sub(1));
    }
    app.search_selected = index.map(|index| chats[index].clone());
    if app.search_selected != before {
        let selected = app.search_selected.clone();
        app.scroll_chat_into_view = selected;
    }
    if enter {
        let chat = app
            .search_selected
            .clone()
            .unwrap_or_else(|| chats[0].clone());
        ui.memory_mut(|memory| memory.surrender_focus(field));
        app.actions.push(Action::Search(String::new()));
        app.actions.push(Action::OpenChat(chat));
    }
}

fn header(app: &mut App, ui: &mut egui::Ui) {
    if theme::macos_chrome(ui.ctx()) {
        macos_header(app, ui);
        return;
    }
    let palette = app.palette;
    // The same margins as the conversation header beside it, and a row as
    // tall as its own, so the two titles share a centre line. The right
    // margin matches the left, so the search field and the chips end where
    // the rows' timestamps do.
    Frame::new()
        .inner_margin(Margin {
            left: 14,
            right: 14,
            top: 8,
            bottom: 8,
        })
        .show(ui, |ui| {
            let row = ui.allocate_ui_with_layout(
                vec2(ui.available_width(), super::conversation::HEADER_ROW),
                Layout::left_to_right(Align::Center),
                |ui| {
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
                        // Our avatar opens the account switcher, with the
                        // profile and settings below the accounts.
                        super::accounts::avatar_button(app, ui, 34.0).tab_stop(Stop::Profile);
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
                            if app.page == Page::Settings {
                                palette.accent
                            } else {
                                palette.secondary
                            },
                            palette.text,
                            // Same as the avatar: the label says what the click
                            // does now, not what it opened.
                            if app.page == Page::Settings {
                                "Close settings (Ctrl+,)"
                            } else {
                                "Settings (Ctrl+,)"
                            },
                        )
                        .tab_stop(Stop::Settings)
                        .clicked()
                        {
                            app.actions.push(Action::ToggleSettings);
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
                        if !super::narrow(ui.ctx())
                            && theme::icon_button(
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
                },
            );
            ui.ctx()
                .data_mut(|data| data.insert_temp(header_row_id(), row.response.rect));
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
            // A disabled field cannot take the keyboard; the request waits.
            if app.focus_search && ui.is_enabled() {
                app.focus_search = false;
                response.request_focus();
            }
            filter_chips(app, ui);
        });
}

/// Where the chat list header's first row was laid out, for layout tests.
pub(crate) fn header_row_id() -> egui::Id {
    egui::Id::new("chat-list-header-row")
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
            let row = ui.horizontal(|ui| {
                ui.set_min_height(super::conversation::HEADER_ROW);
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
                    // Our avatar opens the account switcher here as on the
                    // other platforms, after the traffic lights' inset.
                    super::accounts::avatar_button(app, ui, 30.0).tab_stop(Stop::Profile);
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
                    if !super::narrow(ui.ctx())
                        && theme::icon_button(
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
            ui.ctx()
                .data_mut(|data| data.insert_temp(header_row_id(), row.response.rect));
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
            // A disabled field cannot take the keyboard; the request waits.
            if app.focus_search && ui.is_enabled() {
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
/// Width of the fade over the filter chips' right edge.
pub(super) const CHIP_FADE: f32 = 16.0;

fn filter_chips(app: &mut App, ui: &mut egui::Ui) {
    if !app.locked_folder_open() && !app.search.trim().is_empty() {
        return;
    }
    let palette = app.palette;
    ui.add_space(8.0);
    let output = egui::ScrollArea::horizontal()
        .id_salt("chat-filters")
        // A floating bar would cover the chips; the edge fade shows the row scrolls.
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
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
                        && app.label_filter.is_none()
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
                        ChatFilter::Favorites => Stop::Favorites,
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
    // Chips cut off at the edge fade into the panel, which says the row
    // scrolls on.
    let hidden = output.content_size.x - output.state.offset.x - output.inner_rect.width();
    if hidden > 0.5 {
        widgets::fade_right(ui, output.inner_rect, CHIP_FADE, palette.panel);
    }
    labels::chip_row(app, ui, &palette);
}

fn list(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    // Taken here so a send seen while the list is hidden cannot move it later.
    let to_top = std::mem::take(&mut app.scroll_chats_to_top);
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
        // The favorites title is translated, so it is bound here: the tuple
        // below borrows it for this frame, and every other arm stays a literal.
        let favorites_title;
        let (title, body) = if app.show_archived {
            ("Nothing archived", "Archived chats appear here.")
        } else if app.chat_filter != ChatFilter::All {
            let title = match app.chat_filter {
                ChatFilter::Unread => "No unread chats",
                ChatFilter::Private => "No private chats",
                ChatFilter::Channels => "No channels",
                ChatFilter::Favorites => {
                    favorites_title = crate::i18n::gettext(app.locale, "No favorites yet");
                    favorites_title.as_ref()
                }
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
    // Rows touch: one clickable surface from top to bottom, no gaps or rules.
    ui.spacing_mut().item_spacing.y = 0.0;
    let row_height = theme::ROW_HEIGHT;
    let total = chats.len();
    let mut scroll_area = egui::ScrollArea::vertical()
        .id_salt("chat-list")
        .auto_shrink([false, false]);
    if to_top {
        scroll_area = scroll_area.vertical_scroll_offset(0.0);
    }
    let target_row = app
        .scroll_chat_into_view
        .as_ref()
        .and_then(|target| chats.iter().position(|chat| chat.id == *target));
    // The copies a narrow window's slide draws have widget ids of their own:
    // they show the real list's offset instead of keeping one.
    let copy = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(slide_copy_id()))
        .is_some();
    if copy
        && let Some(offset) = ui
            .ctx()
            .data(|data| data.get_temp::<f32>(shared_offset_id()))
    {
        scroll_area = scroll_area.vertical_scroll_offset(offset);
    }
    // A disabled list leaves the request and any carried scroll to the list
    // drawn for real.
    if let Some(target_row) = target_row.filter(|_| ui.is_enabled()) {
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
    // A scroll gesture that began over the list stays with it (#274).
    let carried = if ui.is_enabled() {
        app.scroll_route.take(crate::app::ScrollPane::Chats)
    } else {
        0.0
    };
    let output = scroll_area.show_rows(ui, row_height, total, |ui, range| {
        if carried != 0.0 {
            ui.scroll_with_delta_animation(
                vec2(0.0, carried),
                egui::style::ScrollAnimation::none(),
            );
        }
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
    if !copy {
        ui.ctx()
            .data_mut(|data| data.insert_temp(shared_offset_id(), output.state.offset.y));
    }
    app.scroll_route
        .place(crate::app::ScrollPane::Chats, output.inner_rect);
    #[cfg(test)]
    ui.ctx()
        .data_mut(|data| data.insert_temp(list_offset_id(), output.state.offset.y));
}

/// Set while a narrow window's slide draws copies of the list.
pub(crate) fn slide_copy_id() -> egui::Id {
    egui::Id::new("chat-list-slide-copy")
}

/// The real list's scroll offset, for the copies a slide draws.
fn shared_offset_id() -> egui::Id {
    egui::Id::new("chat-list-real-offset")
}

/// Where the chat list's scroll offset is kept for tests.
#[cfg(test)]
pub(crate) fn list_offset_id() -> egui::Id {
    egui::Id::new("chat-list-offset")
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
            widgets::row_highlight(ui, &palette, rect, palette.surface_hover);
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
    ui.spacing_mut().item_spacing.y = 0.0;
    let mut scroll_area = egui::ScrollArea::vertical()
        .id_salt("locked-chats")
        .auto_shrink([false, false]);
    let target_row = app
        .scroll_chat_into_view
        .as_ref()
        .and_then(|target| chats.iter().position(|chat| chat.id == *target));
    if let Some(target_row) = target_row {
        let id = ui.make_persistent_id(egui::IdSalt::new("locked-chats"));
        let current = egui::scroll_area::State::load(ui.ctx(), id)
            .unwrap_or_default()
            .offset
            .y;
        let offset = row_scroll_offset(
            current,
            ui.available_height(),
            target_row,
            theme::ROW_HEIGHT,
            ui.spacing().item_spacing.y,
        );
        scroll_area = scroll_area.vertical_scroll_offset(offset);
        app.scroll_chat_into_view = None;
    }
    scroll_area.show_rows(ui, theme::ROW_HEIGHT, chats.len(), |ui, range| {
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
    let needle = crate::util::search_key(app.search.trim());
    let offer_self = !needle.is_empty()
        && app.offers_self(&needle)
        && !chats
            .iter()
            .any(|chat| app.me.as_deref() == Some(chat.id.as_str()));
    if chats.is_empty() && hits.is_empty() && contacts.is_empty() && !offer_self {
        widgets::empty_state(
            ui,
            &palette,
            Icon::Search,
            "No results",
            "Try another name, number, or message text.",
        );
        return;
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical()
        .id_salt("search-results")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if !chats.is_empty() {
                section(ui, &palette, "Chats");
                for chat in &chats {
                    let reveal = ui.is_enabled()
                        && app.scroll_chat_into_view.as_deref() == Some(chat.id.as_str());
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
            if !contacts.is_empty() || offer_self {
                section(ui, &palette, "Contacts");
                if offer_self {
                    ui.push_id("self", |ui| self_row(app, ui));
                }
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
            bottom: 10,
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
            widgets::row_highlight(ui, &palette, rect, palette.surface_hover);
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
            let first = app.short_name(&hit.sender, &sender);
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
            &app.preview_line(&hit.summary(), hit),
            theme::regular(13.0),
            palette.dim,
            (right - x).max(0.0),
            1,
        );
        words.paint(ui, pos2(x, line_y), palette.dim);
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
    let name = app.display_name(&contact.id);
    let detail = crate::model::phone_of(&contact.id).map(crate::util::phone);
    if person_row(app, ui, &contact.id, &name, detail.as_deref()).clicked() {
        app.actions.push(Action::StartChat {
            id: contact.id.clone(),
            name,
        });
    }
}

/// Offers the chat with ourselves, as "Name (You)" over "Message yourself".
pub(super) fn self_row(app: &mut App, ui: &mut egui::Ui) {
    let Some(me) = app.me.clone() else {
        return;
    };
    let name = app.self_title();
    let detail = crate::i18n::gettext(app.locale, "Message yourself");
    if person_row(app, ui, &me, &name, Some(detail.as_ref())).clicked() {
        app.actions.push(Action::MessageYourself);
    }
}

fn person_row(
    app: &mut App,
    ui: &mut egui::Ui,
    id: &str,
    name: &str,
    detail: Option<&str>,
) -> egui::Response {
    let palette = app.palette;
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), theme::ROW_HEIGHT),
        Sense::click(),
    );
    theme::reveal_focus(&response);
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            widgets::row_highlight(ui, &palette, rect, palette.surface_hover);
        }
        let avatar_rect =
            Rect::from_center_size(pos2(rect.left() + 38.0, rect.center().y), Vec2::splat(48.0));
        let picture = app.avatar(id);
        widgets::paint_avatar(ui, &palette, avatar_rect, name, id, picture.as_deref());
        let left = rect.left() + 76.0;
        let name_line = widgets::line(
            ui,
            name,
            theme::medium(14.5),
            palette.text,
            rect.right() - 14.0 - left,
            1,
        );
        name_line.paint(ui, pos2(left, rect.top() + 14.0), palette.text);
        if let Some(detail) = detail {
            let phone_line = widgets::line(
                ui,
                detail,
                theme::regular(13.0),
                palette.dim,
                rect.right() - 14.0 - left,
                1,
            );
            phone_line.paint(ui, pos2(left, rect.top() + 38.0), palette.dim);
        }
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn row(app: &mut App, ui: &mut egui::Ui, chat: &Chat) -> egui::Response {
    let palette = app.palette;
    let title = app.chat_title(chat);
    // While searching, the result reached with the arrows is the selection;
    // before any arrow press it stays the open chat, as a click leaves it.
    let selected = app
        .search_selected
        .as_ref()
        .filter(|_| !app.search.trim().is_empty())
        .or(app.open_chat.as_ref())
        .is_some_and(|selected| *selected == chat.id);
    let now = crate::util::now();
    let muted = chat.muted(now);
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), theme::ROW_HEIGHT),
        Sense::click(),
    );
    theme::reveal_focus(&response);
    // The preview area and the whole last message, when the row cuts it short.
    let mut full_preview: Option<(Rect, String, String)> = None;
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            unread_announcement(&title, chat),
        )
    });
    if ui.is_rect_visible(rect) {
        if selected {
            widgets::row_highlight(ui, &palette, rect, palette.surface_active);
        } else if response.hovered() {
            widgets::row_highlight(ui, &palette, rect, palette.surface_hover);
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
        let unread = chat.looks_unread();
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
            let width = widgets::unread_indicator(
                ui,
                &palette,
                pos2(badge_right - 10.0, line_y + 8.0),
                chat.unread,
                chat.marked_unread,
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
        } else if let Some(draft) = app.draft_preview(&chat.id) {
            // Unsent text waits here as in WhatsApp, marked in the accent.
            let label = format!("{} ", crate::i18n::gettext(app.locale, "Draft:"));
            let label = widgets::line(
                ui,
                &label,
                theme::medium(13.0),
                palette.accent,
                (badge_right - x) * 0.5,
                1,
            );
            let width = label.size().x;
            label.paint(ui, pos2(x, line_y), palette.accent);
            x += width;
            widgets::line(
                ui,
                &draft,
                theme::regular(13.0),
                preview_color,
                (badge_right - x).max(0.0),
                1,
            )
        } else if let Some(last) = &chat.last {
            let mut prefix = String::new();
            if last.from_me {
                let tick_rect =
                    Rect::from_center_size(pos2(x + 8.0, line_y + 8.0), Vec2::splat(16.0));
                widgets::ticks(ui, &palette, tick_rect, last.status);
                x += 20.0;
            } else if chat.is_group() {
                let sender = app.display_name_or(&last.sender, last.sender_name.as_deref());
                let first = app.short_name(&last.sender, &sender);
                prefix = format!("{first}: ");
                let sender = widgets::line(
                    ui,
                    &prefix,
                    theme::regular(13.0),
                    preview_color,
                    (badge_right - x) * 0.5,
                    1,
                );
                let width = sender.size().x;
                sender.paint(ui, pos2(x, line_y), preview_color);
                x += width;
            }
            let words = widgets::line(
                ui,
                &crate::markup::plain(&app.resolve_mention_tokens(&last.summary), &[]),
                theme::regular(13.0),
                preview_color,
                (badge_right - x).max(0.0),
                1,
            );
            // The row shows one line: offer the whole message when that line
            // was cut short or the message has more lines than it.
            if words.galley.elided || last.full.trim_end() != last.summary {
                let area = Rect::from_min_max(
                    pos2(left, line_y - 4.0),
                    pos2(badge_right, line_y + words.size().y.max(16.0) + 4.0),
                );
                full_preview = Some((area, prefix, last.full.clone()));
            }
            words
        } else {
            widgets::line(ui, "", theme::regular(13.0), preview_color, 1.0, 1)
        };
        preview.paint(ui, pos2(x, line_y), preview_color);
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if let Some((area, prefix, full)) = full_preview {
        full_preview_tooltip(app, ui, &chat.id, area, &prefix, &full);
    }
    if response.clicked() {
        app.actions.push(Action::OpenChat(chat.id.clone()));
    }
    let menu_palette = palette;
    // The favorite item is translated, so its width counts in the reader's
    // language.
    let favorite_label = if chat.favorite {
        crate::i18n::gettext(app.locale, "Remove from favorites")
    } else {
        crate::i18n::gettext(app.locale, "Add to favorites")
    };
    let menu_width = widgets::menu_width(
        ui,
        &[
            "Mark as read",
            "Mark as unread",
            "Pin to top",
            favorite_label.as_ref(),
            "Unarchive",
            &crate::i18n::gettext(app.locale, "Leave group"),
            &crate::i18n::gettext(app.locale, "Leave channel"),
            "Mute for 8 hours",
            "Mute for a week",
            "Mute indefinitely",
            "Unlock chat",
            "Copy number",
        ],
        true,
    )
    // Submenu rows also carry a chevron.
    .max(
        widgets::menu_width(
            ui,
            &[
                "Notification sound",
                &crate::i18n::gettext(app.locale, "Labels"),
            ],
            true,
        ) + 24.0,
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

/// The widest a chat row's full-message tooltip grows before it wraps.
const FULL_PREVIEW_WIDTH: f32 = 360.0;
/// Lines a full-message tooltip shows before it ends in an ellipsis.
const FULL_PREVIEW_ROWS: usize = 12;
/// Characters of a message the tooltip lays out: far more than its rows
/// hold, so a very long message costs no more than a long one.
const FULL_PREVIEW_CHARS: usize = 2_000;

/// Where a chat row's latest-message preview sits, for hover tests.
pub fn preview_id(chat: &str) -> egui::Id {
    egui::Id::new(("chat-preview", chat))
}

/// Shows the whole last message while the pointer rests on a chat row's
/// cut-short preview, as WhatsApp Web does. It keeps out of the way of an
/// open menu and of a drag.
fn full_preview_tooltip(
    app: &App,
    ui: &egui::Ui,
    chat: &str,
    area: Rect,
    prefix: &str,
    full: &str,
) {
    let hover = ui.interact(area, preview_id(chat), Sense::hover());
    if egui::Popup::is_any_open(ui.ctx()) || ui.ctx().dragged_id().is_some() {
        return;
    }
    egui::Tooltip::for_enabled(&hover)
        .width(FULL_PREVIEW_WIDTH)
        .show(|ui| {
            let full: String = full.chars().take(FULL_PREVIEW_CHARS).collect();
            let text = format!(
                "{prefix}{}",
                crate::markup::plain(&app.resolve_mention_tokens(&full), &[])
            );
            let color = ui.visuals().text_color();
            let line = widgets::line(
                ui,
                text.trim_end(),
                theme::regular(13.0),
                color,
                FULL_PREVIEW_WIDTH,
                FULL_PREVIEW_ROWS,
            );
            let (rect, _) = ui.allocate_exact_size(line.size(), Sense::hover());
            line.paint(ui, rect.min, color);
        });
}

/// Width of the chat list when it is collapsed to avatars.
const COMPACT_WIDTH: f32 = 60.0;
/// Height of one avatar cell in the collapsed list.
const COMPACT_CELL: f32 = 52.0;
/// Avatar size inside a collapsed cell.
const COMPACT_AVATAR: f32 = 36.0;
/// Height of the rail's top row, level with the conversation header.
const COMPACT_HEADER: f32 = 60.0;

/// Where one avatar in the collapsed chat list was drawn.
pub fn compact_chat_id(chat: &str) -> egui::Id {
    egui::Id::new(("chat-rail", chat))
}

/// Width of the collapsed list. On macOS it also clears the traffic lights,
/// which sit over its top row as they sit over the full list's header.
pub fn compact_width(ctx: &egui::Context) -> f32 {
    COMPACT_WIDTH.max(theme::traffic_light_inset(ctx))
}

/// The chat list collapsed to avatars: the list is out of the way, but every
/// chat is still one click away.
pub fn compact_show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    // Its own panel id: sharing the full list's would hand that resizable
    // panel this narrow width, and it would save it as the list's width.
    let panel = egui::Panel::left("chat-rail")
        .resizable(false)
        .exact_size(compact_width(ui.ctx()))
        .show_separator_line(false)
        .frame(Frame::new().fill(palette.panel).inner_margin(Margin::ZERO));
    let response = panel.show(ui, |ui| {
        let inset = theme::traffic_light_inset(ui.ctx());
        if inset > 0.0 {
            // The traffic lights take the top row; the button goes below.
            let (strip, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), COMPACT_HEADER), Sense::hover());
            super::titlebar_drag(ui, strip);
        }
        ui.allocate_ui_with_layout(
            vec2(
                ui.available_width(),
                if inset > 0.0 { 44.0 } else { COMPACT_HEADER },
            ),
            Layout::centered_and_justified(egui::Direction::TopDown),
            |ui| {
                if theme::icon_button(
                    ui,
                    Icon::PanelLeft,
                    18.0,
                    palette.secondary,
                    palette.text,
                    &super::keys::label("Show the chat list (Ctrl+B)"),
                )
                .tab_stop(Stop::Sidebar)
                .clicked()
                {
                    app.actions.push(Action::ToggleSidebar);
                }
            },
        );
        compact_list(app, ui);
    });
    // Separate the rail from the conversation, exactly as the full list does.
    widgets::paint_edge_beside(ui, &palette, response.response.rect);
}

/// The avatars: the chats the full list would show right now, under the same
/// filter, search, archive, and locked-folder state.
fn compact_list(app: &mut App, ui: &mut egui::Ui) {
    let to_top = std::mem::take(&mut app.scroll_chats_to_top);
    if !app.locked_folder_open() && app.secret_code_matched() {
        // As in the full list, the secret code reveals only the way in.
        compact_locked_entry(app, ui);
        return;
    }
    let chats: Vec<Chat> = app.visible_chats().into_iter().cloned().collect();
    let mut scroll_area = egui::ScrollArea::vertical()
        .id_salt("chat-rail")
        .auto_shrink([false, false]);
    if to_top {
        scroll_area = scroll_area.vertical_scroll_offset(0.0);
    }
    // Alt+Up/Down reveals the chat it opens here too.
    let target_row = app
        .scroll_chat_into_view
        .as_ref()
        .and_then(|target| chats.iter().position(|chat| chat.id == *target));
    if let Some(target_row) = target_row {
        let id = ui.make_persistent_id(egui::IdSalt::new("chat-rail"));
        let current = egui::scroll_area::State::load(ui.ctx(), id)
            .unwrap_or_default()
            .offset
            .y;
        let offset = row_scroll_offset(
            current,
            ui.available_height(),
            target_row,
            COMPACT_CELL,
            ui.spacing().item_spacing.y,
        );
        scroll_area = scroll_area.vertical_scroll_offset(offset);
        app.scroll_chat_into_view = None;
    }
    let carried = app.scroll_route.take(crate::app::ScrollPane::Chats);
    // Only the avatars on screen are laid out, however many chats there are.
    let output = scroll_area.show_rows(ui, COMPACT_CELL, chats.len(), |ui, range| {
        if carried != 0.0 {
            ui.scroll_with_delta_animation(
                vec2(0.0, carried),
                egui::style::ScrollAnimation::none(),
            );
        }
        for chat in &chats[range] {
            let response = ui
                .push_id(("chat", &chat.id), |ui| compact_row(app, ui, chat))
                .inner;
            if response.clicked() && !app.locked_folder_open() && !app.show_archived {
                app.actions.push(Action::KeepUnread(chat.id.clone()));
            }
        }
    });
    app.scroll_route
        .place(crate::app::ScrollPane::Chats, output.inner_rect);
}

/// The collapsed form of the entry the secret code reveals.
fn compact_locked_entry(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), COMPACT_CELL), Sense::click());
    theme::reveal_focus(&response);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Locked chats")
    });
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter().rect_filled(
                Rect::from_center_size(rect.center(), Vec2::splat(COMPACT_CELL - 4.0)),
                egui::CornerRadius::same(theme::RADIUS),
                palette.surface_hover,
            );
        }
        Icon::LockOpen
            .image(palette.accent, 22.0)
            .paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(22.0)));
    }
    ui.ctx()
        .data_mut(|data| data.insert_temp(compact_locked_id(), response.rect));
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Locked chats")
        .clicked()
    {
        app.actions.push(Action::OpenLockedFolder);
    }
}

/// Where the collapsed locked-chats entry was drawn.
fn compact_locked_id() -> egui::Id {
    egui::Id::new("chat-rail-locked")
}

/// One avatar in the collapsed chat list: a click opens the chat, hovering
/// names it, and unread chats carry the same badge as a full row.
fn compact_row(app: &mut App, ui: &mut egui::Ui, chat: &Chat) -> egui::Response {
    let palette = app.palette;
    let title = app.chat_title(chat);
    let selected = app.open_chat.as_deref() == Some(chat.id.as_str());
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), COMPACT_CELL), Sense::click());
    theme::reveal_focus(&response);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            unread_announcement(&title, chat),
        )
    });
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter().rect_filled(
                Rect::from_center_size(rect.center(), Vec2::splat(COMPACT_CELL - 4.0)),
                egui::CornerRadius::same(theme::RADIUS),
                palette.surface_hover,
            );
        }
        let avatar_rect = Rect::from_center_size(rect.center(), Vec2::splat(COMPACT_AVATAR));
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
        if selected {
            // A short accent bar stands in for the selected row's background.
            ui.painter().rect_filled(
                Rect::from_min_size(
                    pos2(rect.left() + 1.0, rect.center().y - 11.0),
                    vec2(3.0, 22.0),
                ),
                1.5,
                palette.accent,
            );
        }
        if chat.looks_unread() {
            // Top right, clear of the disappearing-messages timer in the
            // bottom right corner. A muted chat's badge is dimmed, as in the
            // full row.
            widgets::unread_indicator(
                ui,
                &palette,
                compact_badge_center(avatar_rect),
                chat.unread,
                chat.marked_unread,
                chat.muted(crate::util::now()),
            );
        }
    }
    ui.ctx()
        .data_mut(|data| data.insert_temp(compact_chat_id(&chat.id), response.rect));
    let response = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(title);
    if response.clicked() {
        app.actions.push(Action::OpenChat(chat.id.clone()));
    }
    response
}

/// Centre of the unread badge on a collapsed avatar. A "99+" badge, the
/// widest, still ends inside the rail.
fn compact_badge_center(avatar: Rect) -> egui::Pos2 {
    pos2(avatar.right() - 6.0, avatar.top() + 2.0)
}

fn context_menu(app: &mut App, ui: &mut egui::Ui, chat: &Chat, palette: &Palette) {
    if chat.looks_unread()
        && widgets::menu_item(ui, palette, Some(Icon::CheckCheck), "Mark as read")
    {
        app.actions.push(Action::MarkRead(chat.id.clone()));
    }
    if !chat.looks_unread()
        && widgets::menu_item(ui, palette, Some(Icon::MessageCircle), "Mark as unread")
    {
        app.actions.push(Action::MarkUnread(chat.id.clone()));
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
    // Channels cannot be favorites, as on the phone.
    if !chat.is_channel() {
        // Bound before the call so the translated text outlives the borrow.
        let favorite_label = if chat.favorite {
            crate::i18n::gettext(app.locale, "Remove from favorites")
        } else {
            crate::i18n::gettext(app.locale, "Add to favorites")
        };
        if widgets::menu_item(ui, palette, Some(Icon::Heart), favorite_label.as_ref()) {
            app.actions
                .push(Action::SetFavorite(chat.id.clone(), !chat.favorite));
        }
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
    // Bound before the call so the translated text outlives the borrow.
    let leave_label = if chat.is_channel() {
        crate::i18n::gettext(app.locale, "Leave channel")
    } else {
        crate::i18n::gettext(app.locale, "Leave group")
    };
    if chat.can_leave(&app.our_ids())
        && widgets::menu_item(ui, palette, Some(Icon::LogOut), leave_label.as_ref())
    {
        app.actions
            .push(Action::ShowDialog(Dialog::ConfirmLeaveGroup(
                chat.id.clone(),
            )));
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
    labels::chat_menu(app, ui, chat, palette);
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

/// What a screen reader reads out for a chat's unread state. A chat marked
/// unread by hand has no count to read, so it must not announce zero.
fn unread_announcement(title: &str, chat: &Chat) -> String {
    if chat.marked_unread && chat.unread == 0 {
        format!("{title}, unread")
    } else {
        format!("{title}, {} unread messages", chat.unread)
    }
}

/// A chat's own notification sound, overriding Settings for this chat.
fn sound_menu(app: &mut App, ui: &mut egui::Ui, palette: &Palette, chat: &Chat) {
    use crate::settings::NotificationSound;
    let current = chat.notification_sound.clone();
    let sounds = [
        (None, "Default"),
        (Some(NotificationSound::Receive), "Pidgin"),
        (Some(NotificationSound::Alert), "Pidgin alert"),
        (Some(NotificationSound::System), "System sound"),
        (Some(NotificationSound::None), "No sound"),
    ];
    let custom = match &current {
        Some(NotificationSound::Custom(path)) => Some(path.file_name().map_or_else(
            || "Custom".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        )),
        _ => None,
    };
    let choose = "Choose a file…";
    let items: Vec<&str> = sounds
        .iter()
        .map(|(_, label)| *label)
        .chain(custom.as_deref())
        .chain([choose])
        .collect();
    widgets::submenu(
        ui,
        palette,
        Icon::Volume2,
        "Notification sound",
        &items,
        |ui| {
            for (sound, label) in sounds {
                let checked = current == sound;
                if widgets::menu_item(ui, palette, checked.then_some(Icon::Check), label) {
                    app.actions.push(Action::SetChatSound {
                        chat: chat.id.clone(),
                        sound,
                    });
                    ui.close();
                }
            }
            if let Some(name) = &custom {
                widgets::menu_item(ui, palette, Some(Icon::Check), name);
            }
            if widgets::menu_item(ui, palette, None, choose) {
                app.actions.push(Action::PickChatSound(chat.id.clone()));
                ui.close();
            }
        },
    );
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

    #[test]
    fn a_sent_message_scrolls_the_chat_list_to_the_top() {
        let (_directory, mut app, ids, ctx) = rail_app(24);
        let frame = |app: &mut App| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(360.0, 240.0))),
                    ..Default::default()
                },
                |ui| list(app, ui),
            );
            output.textures_delta.clear();
            ctx.data(|data| data.get_temp::<f32>(list_offset_id()))
                .expect("chat-list offset")
        };
        app.scroll_chat_into_view = ids.last().cloned();
        assert!(frame(&mut app) > 0.0, "the list starts scrolled down");

        app.scroll_chats_to_top = true;
        assert_eq!(frame(&mut app), 0.0, "the list is back at the top");
        assert!(!app.scroll_chats_to_top, "the request was consumed");
        assert_eq!(frame(&mut app), 0.0, "the list stays at the top");
    }

    #[test]
    fn a_send_while_search_results_show_does_not_move_the_list_later() {
        let (_directory, mut app, ids, ctx) = rail_app(24);
        let frame = |app: &mut App| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(360.0, 240.0))),
                    ..Default::default()
                },
                |ui| list(app, ui),
            );
            output.textures_delta.clear();
            ctx.data(|data| data.get_temp::<f32>(list_offset_id()))
                .expect("chat-list offset")
        };
        app.scroll_chat_into_view = ids.last().cloned();
        let scrolled = frame(&mut app);
        assert!(scrolled > 0.0, "the list starts scrolled down");

        app.search = "Chat".into();
        app.scroll_chats_to_top = true;
        frame(&mut app);
        assert!(!app.scroll_chats_to_top, "the request was dropped");
        app.search.clear();
        assert_eq!(frame(&mut app), scrolled, "the list keeps its place");
    }

    #[test]
    fn numbered_shortcuts_reveal_locked_chats_and_consume_the_scroll_request() {
        let (_directory, mut app, ids, ctx) = rail_app(24);
        for chat in &mut app.chats {
            chat.locked = true;
        }
        app.page = Page::Chats;
        app.settings.set_chat_lock_code(Some("fixture-code"));
        app.actions
            .push(Action::UnlockLockedFolder("fixture-code".into()));
        app.background_frame(&ctx);
        assert!(app.locked_folder_open());

        let mut frame = |key: Option<Key>| {
            let mut offset = 0.0;
            let mut height = 0.0;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(360.0, 240.0))),
                    events: key
                        .into_iter()
                        .map(|key| egui::Event::Key {
                            key,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::COMMAND,
                        })
                        .collect(),
                    ..Default::default()
                },
                |ui| {
                    super::super::keys::handle(&mut app, ui.ctx());
                    let scroll_id = ui.make_persistent_id(egui::IdSalt::new("locked-chats"));
                    height = ui.available_height();
                    list(&mut app, ui);
                    offset = egui::scroll_area::State::load(ui.ctx(), scroll_id)
                        .expect("locked-chats scroll state")
                        .offset
                        .y;
                },
            );
            output.textures_delta.clear();
            (offset, height)
        };
        assert_eq!(frame(None).0, 0.0);
        let (offset, height) = frame(Some(Key::Num9));
        assert!(offset > 0.0, "the ninth locked chat starts off screen");
        assert!(8.0 * theme::ROW_HEIGHT >= offset);
        assert!(9.0 * theme::ROW_HEIGHT <= offset + height + 0.5);
        let (offset, _) = frame(Some(Key::Num1));
        assert_eq!(offset, 0.0, "navigating back reveals the first row");
        assert!(app.actions.contains(&Action::OpenChat(ids[8].clone())));
        assert!(app.actions.contains(&Action::OpenChat(ids[0].clone())));
        assert!(app.scroll_chat_into_view.is_none(), "reveal was consumed");
    }

    #[test]
    fn the_collapsed_list_is_narrow_and_opens_a_chat_on_click() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _events) =
            App::headless(AppDirs::under(directory.path()), Settings::default());
        let mut first = Chat::new("491700000001@s.whatsapp.net".into(), "Alice".into());
        first.unread = 3;
        first.last_activity = 20;
        let second = Chat::new("491700000002@s.whatsapp.net".into(), "Bob".into());
        app.chats.push(first.clone());
        app.chats.push(second.clone());
        app.open_chat = Some(second.id.clone());
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut frame = |events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(360.0, 400.0))),
                    events,
                    ..Default::default()
                },
                |ui| compact_show(&mut app, ui),
            );
            output.textures_delta.clear();
        };
        frame(vec![]);
        let rect = ctx
            .data(|data| data.get_temp::<Rect>(compact_chat_id(&first.id)))
            .expect("the rail draws every visible chat");
        assert!(
            ctx.data(|data| data.get_temp::<Rect>(compact_chat_id(&second.id)))
                .is_some(),
            "the rail draws the second chat too"
        );
        assert!(
            (rect.width() - compact_width(&ctx)).abs() < 1.0,
            "an avatar cell is one panel wide, not {} points",
            rect.width()
        );
        let position = rect.center();
        for pressed in [true, false] {
            frame(vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        assert!(
            app.actions.contains(&Action::OpenChat(first.id.clone())),
            "clicking an avatar opens that chat: {:?}",
            app.actions
        );
    }

    #[test]
    fn hovering_the_chip_rows_draws_no_scroll_bar_over_the_chips() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _events) =
            App::headless(AppDirs::under(directory.path()), Settings::default());
        app.labels = (0..8)
            .map(|index| crate::model::Label {
                id: index.to_string(),
                name: format!("Label {index}"),
                color_hex: "#25d366".into(),
                created_at: 0,
            })
            .collect();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        // Narrow enough that both rows scroll.
        let mut frame = |events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(220.0, 400.0))),
                    events,
                    ..Default::default()
                },
                |ui| filter_chips(&mut app, ui),
            );
            output.textures_delta.clear();
            output.shapes
        };
        frame(vec![]);
        let chip = ctx
            .data(|data| data.get_temp::<Rect>(filter_chip_id(ChatFilter::All)))
            .expect("the filter chips are drawn");
        let labels = ctx
            .data(|data| data.get_temp::<Rect>(super::labels::chip_row_id()))
            .expect("the label chips are drawn");
        for (row, pointer) in [("filter", chip.center()), ("label", labels.center())] {
            let mut shapes = Vec::new();
            for _ in 0..5 {
                shapes = frame(vec![egui::Event::PointerMoved(pointer)]);
            }
            // A scroll bar handle is a thin rectangle; chips are taller.
            fn thin_rects(shape: &egui::Shape, bars: &mut Vec<Rect>) {
                match shape {
                    egui::Shape::Vec(shapes) => {
                        shapes.iter().for_each(|shape| thin_rects(shape, bars));
                    }
                    egui::Shape::Rect(rect)
                        if rect.rect.height() < 12.0 && rect.rect.width() > 12.0 =>
                    {
                        bars.push(rect.rect);
                    }
                    _ => {}
                }
            }
            let mut bars = Vec::new();
            for clipped in &shapes {
                thin_rects(&clipped.shape, &mut bars);
            }
            assert!(
                bars.is_empty(),
                "the {row} row drew a scroll bar at {bars:?}"
            );
        }
    }

    /// An app with `count` chats, newest first, and a context to draw it in.
    fn rail_app(count: usize) -> (tempfile::TempDir, App, Vec<String>, egui::Context) {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _events) =
            App::headless(AppDirs::under(directory.path()), Settings::default());
        let mut ids = Vec::new();
        for index in 0..count {
            let id = format!("49170000{index:04}@s.whatsapp.net");
            let mut chat = Chat::new(id.clone(), format!("Chat {index:03}"));
            chat.last_activity = 10_000 - index as i64;
            app.chats.push(chat);
            ids.push(id);
        }
        let ctx = egui::Context::default();
        app.attach(&ctx);
        (directory, app, ids, ctx)
    }

    fn rail_frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 400.0))),
                events,
                ..Default::default()
            },
            |ui| {
                super::super::keys::handle(app, ui.ctx());
                compact_show(app, ui);
            },
        );
        output.textures_delta.clear();
    }

    #[test]
    fn the_collapsed_list_lays_out_only_the_avatars_on_screen() {
        let (_directory, mut app, ids, ctx) = rail_app(2_000);
        rail_frame(&mut app, &ctx, vec![]);
        let drawn = ids
            .iter()
            .filter(|id| ctx.data(|data| data.get_temp::<Rect>(compact_chat_id(id)).is_some()))
            .count();
        assert!(drawn > 0, "the first avatars are drawn");
        assert!(
            drawn < 20,
            "a 400-point window has room for a few avatars, yet {drawn} were laid out"
        );
    }

    #[test]
    fn a_sent_message_scrolls_the_collapsed_list_to_the_top() {
        let (_directory, mut app, ids, ctx) = rail_app(40);
        let laid_out = |id: &str| {
            ctx.data(|data| data.get_temp::<Rect>(compact_chat_id(id)))
                .is_some()
        };
        app.scroll_chat_into_view = Some(ids[39].clone());
        rail_frame(&mut app, &ctx, vec![]);
        rail_frame(&mut app, &ctx, vec![]);
        assert!(laid_out(&ids[39]), "the rail starts scrolled to the end");

        app.scroll_chats_to_top = true;
        // Row rects stay in memory until replaced; forget the old ones.
        ctx.data_mut(|data| {
            data.remove::<Rect>(compact_chat_id(&ids[0]));
            data.remove::<Rect>(compact_chat_id(&ids[39]));
        });
        rail_frame(&mut app, &ctx, vec![]);
        assert!(!app.scroll_chats_to_top, "the request was consumed");
        assert!(laid_out(&ids[0]), "the first avatar is back on screen");
        assert!(!laid_out(&ids[39]), "the last avatar left the screen");
    }

    #[test]
    fn alt_navigation_scrolls_the_collapsed_list_too() {
        let (_directory, mut app, ids, ctx) = rail_app(40);
        app.open_chat = Some(ids[0].clone());
        rail_frame(&mut app, &ctx, vec![]);
        assert!(
            ctx.data(|data| data.get_temp::<Rect>(compact_chat_id(&ids[39])))
                .is_none(),
            "the last avatar starts off screen"
        );
        let alt_up = egui::Event::Key {
            key: egui::Key::ArrowUp,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::ALT,
        };
        rail_frame(&mut app, &ctx, vec![alt_up]);
        assert!(app.actions.contains(&Action::OpenChat(ids[39].clone())));
        assert!(app.scroll_chat_into_view.is_none(), "reveal was consumed");
        rail_frame(&mut app, &ctx, vec![]);
        let rect = ctx
            .data(|data| data.get_temp::<Rect>(compact_chat_id(&ids[39])))
            .expect("the destination avatar is laid out");
        assert!(
            rect.bottom() <= 400.0 + 0.5,
            "the destination avatar is on screen: {rect:?}"
        );
    }

    #[test]
    fn the_secret_code_shows_only_the_way_into_the_locked_folder() {
        let (_directory, mut app, ids, ctx) = rail_app(3);
        app.chats[0].locked = true;
        // A code that also matches an ordinary chat's number.
        app.settings.set_chat_lock_code(Some("0001"));
        app.search = "0001".into();
        rail_frame(&mut app, &ctx, vec![]);
        for id in &ids {
            assert!(
                ctx.data(|data| data.get_temp::<Rect>(compact_chat_id(id)))
                    .is_none(),
                "no chat shows while the code is typed"
            );
        }
        let entry = ctx
            .data(|data| data.get_temp::<Rect>(compact_locked_id()))
            .expect("the locked entry is drawn");
        let position = entry.center();
        for pressed in [true, false] {
            rail_frame(
                &mut app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(app.actions.contains(&Action::OpenLockedFolder));
    }

    #[test]
    fn collapsing_and_expanding_keeps_the_saved_list_width() {
        let (_directory, mut app, _ids, ctx) = rail_app(3);
        app.settings.sidebar_width = 400.0;
        let frame = |app: &mut App, collapsed: bool| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 600.0))),
                    ..Default::default()
                },
                |ui| {
                    if collapsed {
                        compact_show(app, ui);
                    } else {
                        show(app, ui);
                    }
                },
            );
            output.textures_delta.clear();
        };
        frame(&mut app, false);
        frame(&mut app, true);
        frame(&mut app, false);
        assert!(
            (app.settings.sidebar_width - 400.0).abs() < 1.0,
            "the full list came back {} points wide",
            app.settings.sidebar_width
        );
        assert!(!app.actions.contains(&Action::SettingsChanged));
    }

    #[test]
    fn the_widest_badge_stays_inside_the_collapsed_list() {
        let (_directory, _app, _ids, ctx) = rail_app(0);
        let rail = Rect::from_min_size(egui::Pos2::ZERO, vec2(COMPACT_WIDTH, COMPACT_CELL));
        let avatar = Rect::from_center_size(rail.center(), Vec2::splat(COMPACT_AVATAR));
        let mut width = 0.0;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let palette = Palette::dark();
            width = widgets::badge(ui, &palette, compact_badge_center(avatar), 120, true);
        });
        output.textures_delta.clear();
        let center = compact_badge_center(avatar);
        assert!(center.x + width / 2.0 <= rail.right(), "{width} too wide");
        assert!(center.y - 10.0 >= rail.top(), "the badge fits its cell");
    }
}
