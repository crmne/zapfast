//! The open chat: its header, the messages, and the composer.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::{
    Align, Align2, Color32, CornerRadius, Frame, Key, KeyboardShortcut, Layout, Margin, Modifiers,
    Rect, Sense, Stroke, Vec2, pos2, vec2,
};

use crate::animation;
use crate::app::{App, Conversation};
use crate::markup;
use crate::model::{
    Action, Chat, ChatId, Content, Delivery, Dialog, LinkPreview, Media, MediaState, Message,
    PickerTab,
};
use crate::theme::{self, Icon, Palette};
use crate::wallpaper;

use super::focus::{Stop, TabStop};
use super::widgets;

/// Group-message avatar size.
const SENDER_AVATAR: f32 = 28.0;
const BODY_SIZE: f32 = 14.5;
/// Footer label on an outgoing message that failed to send.
const NOT_SENT: &str = "Not sent";
const NOT_SENT_HINT: &str =
    "This message could not be sent, and ZapFast will not retry it. Send it again yourself.";

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let Some(chat) = app.current_chat().cloned() else {
        super::standalone_header(app, ui);
        if theme::macos_chrome(ui.ctx()) {
            super::banner(app, ui);
        }
        empty(app, ui);
        return;
    };
    if app.settings.show_wallpaper {
        wallpaper::paint(ui, app.settings.wallpaper_color_for(app.palette.dark));
    }
    header(app, ui, &chat);
    chat_search(app, ui);
    if theme::macos_chrome(ui.ctx()) {
        super::banner(app, ui);
    }
    composer(app, ui, &chat);
    messages(app, ui, &chat);
}

/// Search within the open chat: the query, how many matches it has, and the
/// buttons that walk them. Escape closes it, Enter goes forward.
fn chat_search(app: &mut App, ui: &mut egui::Ui) {
    if !app.chat_search_open {
        return;
    }
    let palette = app.palette;
    egui::Panel::top("chat-search")
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(palette.panel)
                .inner_margin(Margin::symmetric(14, 6)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_min_height(HEADER_ROW);
                let controls = 150.0;
                let width = (ui.available_width() - controls).max(120.0);
                let id = egui::Id::new("chat-search-in-chat");
                let mut text = app.chat_search.clone();
                let response = widgets::search_field(
                    ui,
                    &palette,
                    id,
                    &mut text,
                    "Search in this chat",
                    width,
                );
                if text != app.chat_search {
                    app.actions.push(Action::ChatSearch(text));
                }
                if app.chat_search_focus {
                    app.chat_search_focus = false;
                    response.request_focus();
                }
                let total = app.chat_search_hits.len();
                let position = if total == 0 {
                    0
                } else {
                    app.chat_search_index + 1
                };
                ui.label(
                    egui::RichText::new(format!("{position} / {total}")).color(palette.secondary),
                );
                if theme::icon_button(
                    ui,
                    Icon::ChevronUp,
                    16.0,
                    palette.secondary,
                    palette.text,
                    "Previous match (Shift+Enter)",
                )
                .clicked()
                {
                    app.actions.push(Action::StepChatSearch(-1));
                }
                if theme::icon_button(
                    ui,
                    Icon::ChevronDown,
                    16.0,
                    palette.secondary,
                    palette.text,
                    "Next match (Enter)",
                )
                .clicked()
                {
                    app.actions.push(Action::StepChatSearch(1));
                }
            });
        });
}

fn empty(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let rect = ui.max_rect();
    let center = rect.center() - vec2(0.0, 30.0);
    theme::logo(
        ui,
        center - vec2(0.0, 60.0),
        72.0,
        palette.surface,
        palette.dim,
    );
    ui.painter().text(
        center,
        Align2::CENTER_CENTER,
        "ZapFast",
        theme::bold(24.0),
        palette.text,
    );
    ui.painter().text(
        center + vec2(0.0, 30.0),
        Align2::CENTER_CENTER,
        if app.chats.is_empty() {
            "Your chats appear on the left as they load."
        } else {
            "Select a chat on the left."
        },
        theme::regular(14.0),
        palette.secondary,
    );
    if app.settings.show_shortcut_hints {
        ui.painter().text(
            center + vec2(0.0, 56.0),
            Align2::CENTER_CENTER,
            super::keys::label(
                crate::i18n::gettext(app.locale, "Ctrl+K to search · ? for keyboard shortcuts")
                    .as_ref(),
            ),
            theme::regular(12.5),
            palette.dim,
        );
    }
}

fn header(app: &mut App, ui: &mut egui::Ui, chat: &Chat) {
    let palette = app.palette;
    let title = app.chat_title(chat);
    // The collapsed list has its own show button and clears the traffic
    // lights itself; only a fully hidden list leaves both to the header.
    let sidebar_hidden = app.sidebar_mode() == crate::model::SidebarDisplayMode::Hidden;
    egui::Panel::top("chat-header")
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(palette.panel)
                .inner_margin(Margin::symmetric(14, 8)),
        )
        .show(ui, |ui| {
            if theme::macos_chrome(ui.ctx()) {
                let mut drag = ui.max_rect();
                if sidebar_hidden {
                    drag.min.x += theme::traffic_light_inset(ui.ctx());
                }
                super::titlebar_drag(ui, drag);
            }
            ui.horizontal(|ui| {
                // Give both rows a fixed height so their contents align.
                ui.set_min_height(HEADER_ROW);
                if sidebar_hidden && theme::macos_chrome(ui.ctx()) {
                    ui.add_space((theme::traffic_light_inset(ui.ctx()) - 14.0).max(0.0));
                }
                if sidebar_hidden
                    && theme::icon_button(
                        ui,
                        Icon::PanelLeft,
                        18.0,
                        palette.secondary,
                        palette.text,
                        "Show the chat list (Ctrl+B)",
                    )
                    .tab_stop(Stop::Sidebar)
                    .clicked()
                {
                    app.actions.push(Action::ToggleSidebar);
                }
                let picture = app.avatar(&chat.id);
                let (subtitle, color) = subtitle(app, chat);
                let right_controls = 52.0;
                // Treat the avatar, name, and subtitle as one info button.
                let block = ui
                    .scope(|ui| {
                        // Fix the child height before centering its contents.
                        ui.allocate_ui_with_layout(
                            vec2(
                                (ui.available_width() - right_controls).max(80.0),
                                HEADER_ROW,
                            ),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                let avatar_response = widgets::avatar(
                                    ui,
                                    &palette,
                                    &title,
                                    &chat.id,
                                    40.0,
                                    picture.as_deref(),
                                );
                                if chat.ephemeral_expiration.is_some() {
                                    widgets::paint_disappearing_badge(
                                        ui,
                                        &palette,
                                        avatar_response.rect,
                                    );
                                }
                                ui.add_space(4.0);
                                ui.vertical(|ui| {
                                    let width = (ui.available_width() - right_controls).max(80.0);
                                    ui.set_max_width(width);
                                    if subtitle.is_empty() {
                                        // Center the name on the avatar.
                                        ui.allocate_ui_with_layout(
                                            vec2(width, 40.0),
                                            Layout::left_to_right(Align::Center),
                                            |ui| {
                                                widgets::rich_text(
                                                    ui,
                                                    &title,
                                                    theme::semibold(17.0),
                                                    palette.text,
                                                );
                                            },
                                        );
                                    } else {
                                        // Align the name and subtitle with the avatar edges.
                                        ui.allocate_ui_with_layout(
                                            vec2(width, 40.0),
                                            Layout::top_down(Align::Min),
                                            |ui| {
                                                widgets::rich_text(
                                                    ui,
                                                    &title,
                                                    theme::semibold(15.0),
                                                    palette.text,
                                                );
                                                ui.with_layout(
                                                    Layout::bottom_up(Align::Min),
                                                    |ui| {
                                                        widgets::rich_text(
                                                            ui,
                                                            &subtitle,
                                                            theme::regular(12.5),
                                                            color,
                                                        );
                                                    },
                                                );
                                            },
                                        );
                                    }
                                });
                            },
                        );
                    })
                    .response;
                let block = ui
                    .interact(block.rect, ui.id().with("chat-header-info"), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if block.clicked() {
                    app.actions
                        .push(Action::ShowDialog(Dialog::ChatInfo(chat.id.clone())));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let more = theme::icon_button(
                        ui,
                        Icon::Ellipsis,
                        18.0,
                        palette.secondary,
                        palette.text,
                        "More",
                    );
                    let width = widgets::menu_width(
                        ui,
                        &[
                            "Info",
                            "Pin to top",
                            "Unarchive",
                            "Copy number",
                            "Close chat",
                        ],
                        true,
                    );
                    egui::Popup::menu(&more)
                        .width(width)
                        .frame(widgets::menu_frame(&palette))
                        .show(|ui| {
                            if widgets::menu_item(ui, &palette, Some(Icon::Info), "Info") {
                                app.actions
                                    .push(Action::ShowDialog(Dialog::ChatInfo(chat.id.clone())));
                            }
                            if widgets::menu_item(
                                ui,
                                &palette,
                                Some(if chat.pinned { Icon::PinOff } else { Icon::Pin }),
                                if chat.pinned { "Unpin" } else { "Pin to top" },
                            ) {
                                app.actions
                                    .push(Action::SetPinned(chat.id.clone(), !chat.pinned));
                            }
                            if widgets::menu_item(
                                ui,
                                &palette,
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
                            widgets::menu_separator(ui, &palette);
                            if let Some(phone) = chat.phone()
                                && widgets::menu_item(ui, &palette, Some(Icon::Copy), "Copy number")
                            {
                                app.actions.push(Action::CopyText(format!("+{phone}")));
                            }
                            if widgets::menu_item(ui, &palette, Some(Icon::X), "Close chat") {
                                app.actions.push(Action::CloseChat);
                            }
                        });
                });
            });
        });
}

/// Chat-header subtitle.
fn subtitle(app: &App, chat: &Chat) -> (String, Color32) {
    let palette = app.palette;
    let typing = app.typing_in(&chat.id);
    if !typing.is_empty() {
        let text = if chat.is_group() {
            let names: Vec<&str> = typing.iter().map(|(_, name)| name.as_str()).collect();
            match names.as_slice() {
                [] => String::new(),
                [one] => format!("{one} is typing…"),
                [rest @ .., last] => format!("{} and {last} are typing…", rest.join(", ")),
            }
        } else {
            "typing…".to_owned()
        };
        return (text, palette.accent);
    }
    if chat.is_group() {
        let names = app.participant_names(chat);
        return (
            if names.is_empty() {
                "Group".to_owned()
            } else {
                names
            },
            palette.secondary,
        );
    }
    if let Some(presence) = app.presence.get(&chat.id) {
        if presence.online {
            return ("online".to_owned(), palette.accent);
        }
        if let Some(seen) = presence.last_seen {
            return (
                format!(
                    "last seen {}",
                    crate::util::chat_stamp(app.locale, seen).to_lowercase()
                ),
                palette.secondary,
            );
        }
    }
    match chat.phone() {
        Some(phone) if !app.is_saved_contact(&chat.id) => {
            (crate::util::phone(phone), palette.secondary)
        }
        _ => (String::new(), palette.secondary),
    }
}

/// Byte position of a freshly typed standalone trigger immediately before
/// the text cursor. Colons inside times and URLs, and `@` inside addresses,
/// remain ordinary text.
fn standalone_trigger(text: &str, cursor: usize, trigger: char) -> Option<usize> {
    let cursor = text
        .char_indices()
        .nth(cursor)
        .map_or(text.len(), |(at, _)| at);
    let (at, found) = text[..cursor].char_indices().next_back()?;
    if found != trigger {
        return None;
    }
    (at == 0
        || text[..at]
            .chars()
            .next_back()
            .is_some_and(|character| !character.is_alphanumeric()))
    .then_some(at)
}

/// Active mention query from its `@` through the current text cursor.
fn active_mention(text: &str, start: Option<usize>, cursor: usize) -> Option<(usize, &str)> {
    let start = start?;
    let end = text
        .char_indices()
        .nth(cursor)
        .map_or(text.len(), |(at, _)| at);
    let query = text.get(start.checked_add(1)?..end)?;
    (!query.contains(['@', '\n'])).then_some((end, query))
}

/// Active emoji query from its `:` through the current text cursor. Spaces
/// and punctuation end autocomplete without changing what the user typed.
fn active_emoji(text: &str, start: Option<usize>, cursor: usize) -> Option<(usize, &str)> {
    let start = start?;
    let after = start.checked_add(1)?;
    if text.get(start..after) != Some(":") {
        return None;
    }
    let end = text
        .char_indices()
        .nth(cursor)
        .map_or(text.len(), |(at, _)| at);
    let query = text.get(after..end)?;
    query
        .chars()
        .all(|character| character.is_alphanumeric() || matches!(character, '_' | '-' | '+'))
        .then_some((end, query))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EmojiSuggestion {
    emoji: &'static str,
    shortcode: String,
    name: &'static str,
}

fn emoji_match_score(emoji: &emojis::Emoji, query: &str) -> Option<u8> {
    let name = emoji.name();
    let shortcodes = emoji.shortcodes();
    if shortcodes.clone().any(|code| code == query) {
        Some(0)
    } else if shortcodes.clone().any(|code| code.starts_with(query)) {
        Some(1)
    } else if name == query {
        Some(2)
    } else if name.starts_with(query)
        || name
            .split([' ', '-', '_'])
            .any(|word| word.starts_with(query))
    {
        Some(3)
    } else if shortcodes.clone().any(|code| code.contains(query)) {
        Some(4)
    } else if name.contains(query) {
        Some(5)
    } else {
        None
    }
}

fn emoji_suggestion(emoji: &'static emojis::Emoji) -> EmojiSuggestion {
    let shortcode = emoji
        .shortcode()
        .map_or_else(|| emoji.name().replace([' ', '-'], "_"), str::to_owned);
    EmojiSuggestion {
        emoji: emoji.as_str(),
        shortcode: format!(":{shortcode}:"),
        name: emoji.name(),
    }
}

/// Completions for `:query`. `emojis::iter` yields each emoji once, and a
/// skin-tone-capable one such as 👍 reports `Some(SkinTone::Default)`.
fn emoji_candidates(app: &App, query: &str) -> Vec<EmojiSuggestion> {
    const LIMIT: usize = 6;
    let query = query.to_lowercase();
    let mut seen = HashSet::new();
    if query.is_empty() {
        let recent = app
            .settings
            .recent_emoji
            .iter()
            .filter_map(|emoji| emojis::get(emoji))
            .chain(emojis::iter())
            .filter(|emoji| seen.insert(emoji.as_str()))
            .take(LIMIT)
            .map(emoji_suggestion)
            .collect();
        return recent;
    }

    let mut found: Vec<_> = emojis::iter()
        .enumerate()
        .filter_map(|(order, emoji)| {
            emoji_match_score(emoji, &query).map(|score| (score, order, emoji))
        })
        .collect();
    found.sort_by_key(|(score, order, _)| (*score, *order));
    found
        .into_iter()
        .filter(|(_, _, emoji)| seen.insert(emoji.as_str()))
        .take(LIMIT)
        .map(|(_, _, emoji)| emoji_suggestion(emoji))
        .collect()
}

fn take_plain_key(ui: &mut egui::Ui, key: Key) -> bool {
    ui.input_mut(|input| {
        let mut taken = false;
        input.events.retain(|event| {
            if taken {
                return true;
            }
            let matches = matches!(
                event,
                egui::Event::Key {
                    key: found,
                    pressed: true,
                    modifiers,
                    ..
                } if *found == key && *modifiers == Modifiers::NONE
            );
            taken |= matches;
            !matches
        });
        taken
    })
}

/// Slack-style emoji suggestions above the composer. The composer keeps
/// focus, so ordinary typing continues refining the query.
fn emoji_suggestions(app: &mut App, ui: &mut egui::Ui, field: egui::Id) {
    let cursor = egui::TextEdit::load_state(ui.ctx(), field)
        .and_then(|state| state.cursor.char_range())
        .map(|range| range.primary.index.0)
        .unwrap_or_else(|| app.composer.chars().count());
    let Some((end, query)) = active_emoji(&app.composer, app.emoji_start, cursor) else {
        app.emoji_start = None;
        return;
    };
    let start = app.emoji_start.expect("checked above");
    let candidates = emoji_candidates(app, query);
    if candidates.is_empty() {
        return;
    }

    let down = take_plain_key(ui, Key::ArrowDown);
    let up = take_plain_key(ui, Key::ArrowUp);
    if down {
        app.emoji_selected = (app.emoji_selected + 1) % candidates.len();
    }
    if up {
        app.emoji_selected = (app.emoji_selected + candidates.len() - 1) % candidates.len();
    }
    app.emoji_selected = app.emoji_selected.min(candidates.len() - 1);
    let submit = take_plain_key(ui, Key::Enter) || take_plain_key(ui, Key::Tab);
    let mut picked = submit.then(|| candidates[app.emoji_selected].clone());
    let palette = app.palette;

    ui.add_space(4.0);
    Frame::new()
        .fill(palette.overlay)
        .stroke(Stroke::new(1.0, palette.outline))
        .corner_radius(CornerRadius::same(theme::RADIUS + 2))
        .inner_margin(Margin::same(4))
        .show(ui, |ui| {
            let row_height = 36.0;
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, candidate) in candidates.iter().enumerate() {
                let (rect, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), row_height), Sense::click());
                if index == app.emoji_selected {
                    ui.painter()
                        .rect_filled(rect, 6.0, palette.accent.gamma_multiply(0.18));
                    ui.painter().rect_stroke(
                        rect,
                        6.0,
                        Stroke::new(1.0, palette.accent),
                        egui::StrokeKind::Inside,
                    );
                } else if response.hovered() {
                    ui.painter().rect_filled(rect, 6.0, palette.surface_hover);
                }

                let emoji = widgets::line(
                    ui,
                    candidate.emoji,
                    theme::regular(22.0),
                    palette.text,
                    30.0,
                    1,
                );
                emoji.paint(
                    ui,
                    pos2(rect.left() + 6.0, rect.center().y - emoji.size().y / 2.0),
                    palette.text,
                );
                let shortcode = widgets::line(
                    ui,
                    &candidate.shortcode,
                    theme::medium(13.0),
                    palette.text,
                    (rect.width() * 0.4).max(100.0),
                    1,
                );
                let text_x = rect.left() + 42.0;
                shortcode.paint(
                    ui,
                    pos2(text_x, rect.center().y - shortcode.size().y / 2.0),
                    palette.text,
                );
                let name_x = text_x + shortcode.size().x + 12.0;
                let name = widgets::line(
                    ui,
                    candidate.name,
                    theme::regular(12.5),
                    palette.secondary,
                    (rect.right() - name_x - 8.0).max(0.0),
                    1,
                );
                name.paint(
                    ui,
                    pos2(name_x, rect.center().y - name.size().y / 2.0),
                    palette.secondary,
                );
                if response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    app.emoji_selected = index;
                    picked = Some(candidate.clone());
                }
            }
        });
    if let Some(candidate) = picked {
        app.actions.push(Action::InsertEmojiCompletion {
            emoji: candidate.emoji.to_owned(),
            start,
            end,
        });
    }
}

/// Group-member suggestions above the composer.
fn mention_picker(app: &mut App, ui: &mut egui::Ui, chat: &Chat, field: egui::Id) {
    let cursor = egui::TextEdit::load_state(ui.ctx(), field)
        .and_then(|state| state.cursor.char_range())
        .map(|range| range.primary.index.0)
        .unwrap_or_else(|| app.composer.chars().count());
    let Some((end, query)) = active_mention(&app.composer, app.mention_start, cursor) else {
        app.mention_start = None;
        return;
    };
    let start = app.mention_start.expect("checked above");
    let candidates = app.mention_candidates(chat, query);
    if candidates.is_empty() {
        return;
    }
    let down = take_plain_key(ui, Key::ArrowDown);
    let up = take_plain_key(ui, Key::ArrowUp);
    if down {
        app.mention_selected = (app.mention_selected + 1) % candidates.len();
    }
    if up {
        app.mention_selected = (app.mention_selected + candidates.len() - 1) % candidates.len();
    }
    app.mention_selected = app.mention_selected.min(candidates.len() - 1);
    let submit = take_plain_key(ui, Key::Enter) || take_plain_key(ui, Key::Tab);
    let mut picked = submit.then(|| candidates[app.mention_selected].clone());
    let palette = app.palette;

    ui.add_space(4.0);
    Frame::new()
        .fill(palette.overlay)
        .stroke(Stroke::new(1.0, palette.outline))
        .corner_radius(CornerRadius::same(theme::RADIUS + 2))
        .inner_margin(Margin::same(4))
        .show(ui, |ui| {
            let row_height = 38.0;
            egui::ScrollArea::vertical()
                .id_salt("mention-members")
                .max_height(row_height * candidates.len().min(5) as f32)
                .auto_shrink([false, true])
                .show_rows(ui, row_height, candidates.len(), |ui, range| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for index in range {
                        let (id, label) = &candidates[index];
                        let (rect, response) = ui.allocate_exact_size(
                            vec2(ui.available_width(), row_height),
                            Sense::click(),
                        );
                        if index == app.mention_selected || response.hovered() {
                            ui.painter().rect_filled(rect, 6.0, palette.surface_hover);
                        }
                        let avatar = Rect::from_center_size(
                            pos2(rect.left() + 19.0, rect.center().y),
                            Vec2::splat(28.0),
                        );
                        let picture = app.avatar(id);
                        widgets::paint_avatar(
                            ui,
                            &palette,
                            avatar,
                            label.trim_start_matches('~'),
                            id,
                            picture.as_deref(),
                        );
                        let detail = crate::model::phone_of(id)
                            .map(crate::util::phone)
                            .unwrap_or_default();
                        let detail = widgets::line(
                            ui,
                            &detail,
                            theme::regular(11.5),
                            palette.secondary,
                            (rect.width() * 0.36).min(150.0),
                            1,
                        );
                        let name = widgets::line(
                            ui,
                            label,
                            theme::medium(13.5),
                            palette.text,
                            rect.width() - detail.size().x - 62.0,
                            1,
                        );
                        name.paint(
                            ui,
                            pos2(rect.left() + 40.0, rect.center().y - name.size().y / 2.0),
                            palette.text,
                        );
                        detail.paint(
                            ui,
                            pos2(
                                rect.right() - detail.size().x - 8.0,
                                rect.center().y - detail.size().y / 2.0,
                            ),
                            palette.secondary,
                        );
                        if response.hovered() {
                            app.mention_selected = index;
                        }
                        if (down || up) && index == app.mention_selected {
                            response.scroll_to_me(Some(Align::Center));
                        }
                        if response
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            picked = Some((id.clone(), label.clone()));
                        }
                    }
                });
        });
    if let Some((id, name)) = picked {
        app.actions.push(Action::InsertMention {
            id,
            name,
            start,
            end,
        });
    }
}

fn composer(app: &mut App, ui: &mut egui::Ui, chat: &Chat) {
    let palette = app.palette;
    let shown = egui::Panel::bottom("composer")
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(palette.panel)
                .inner_margin(Margin::symmetric(12, 8)),
        )
        .show(ui, |ui| {
            if let Some((selected_chat, selected)) = app.selection.clone()
                && selected_chat == chat.id
            {
                selection_bar(app, ui, &chat.id, &selected);
                return;
            }
            if !chat.can_send() {
                if chat.kind == crate::model::ChatKind::Broadcast {
                    ui.vertical_centered(|ui| {
                        theme::text(ui, "Channels are read-only in ZapFast", theme::regular(13.5), palette.secondary);
                    });
                    return;
                }
                if chat.locked {
                    ui.vertical_centered(|ui| {
                        theme::text(ui, "Locked chats are read-only in ZapFast", theme::regular(13.5), palette.secondary);
                    });
                    return;
                }
                ui.vertical_centered(|ui| {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let width = 230.0;
                        ui.add_space((ui.available_width() - width).max(0.0) / 2.0);
                        theme::text(ui, "Only", theme::regular(13.5), palette.secondary);
                        theme::text(ui, "admins", theme::semibold(13.5), palette.accent);
                        theme::text(
                            ui,
                            "can send messages",
                            theme::regular(13.5),
                            palette.secondary,
                        );
                    });
                    ui.add_space(8.0);
                });
                return;
            }
            if app.editing.is_some() {
                edit_strip(app, ui);
            } else if let Some(reply_id) = app.reply_to.clone() {
                let quoted = app
                    .conversations
                    .get(&chat.id)
                    .and_then(|conversation| conversation.message(&reply_id))
                    .cloned();
                match quoted {
                    Some(quoted) => reply_strip(app, ui, &quoted),
                    None => app.reply_to = None,
                }
            }
            let id = egui::Id::new("composer-text");
            let has_focus = ui.memory(|memory| memory.has_focus(id));
            let enter_sends = app.settings.enter_sends;
            let (typed_colon, typed_at) = ui.input(|input| {
                let typed = |needle: &str| {
                    input
                        .events
                        .iter()
                        .any(|event| matches!(event, egui::Event::Text(text) if text == needle))
                };
                (has_focus && typed(":"), has_focus && typed("@"))
            });
            if !app.pending.is_empty() {
                pending_strip(app, ui);
            }
            if app.recording.is_some() {
                recording_strip(app, ui);
                return;
            }
            emoji_suggestions(app, ui, id);
            mention_picker(app, ui, chat, id);
            // `consume_key(NONE, Enter)` also matches Shift+Enter. Check the
            // event modifiers directly. An active suggestion list consumes
            // plain Enter first when it has a selection.
            let send_key = has_focus
                && ui.input_mut(|input| {
                    let mut sent = false;
                    input.events.retain(|event| {
                        if sent {
                            return true;
                        }
                        let is_send = matches!(
                            event,
                            egui::Event::Key {
                                key: Key::Enter,
                                pressed: true,
                                modifiers,
                                ..
                            } if !modifiers.shift && !modifiers.alt
                                && (if enter_sends { !modifiers.command && !modifiers.ctrl } else { modifiers.command })
                        );
                        sent |= is_send;
                        !is_send
                    });
                    sent
                });
            let mut send_click = false;
            let line_height = ui
                .painter()
                .layout_no_wrap("x".to_owned(), theme::regular(BODY_SIZE), palette.text)
                .size()
                .y;
            // Match the button to a one-line field. The field grows to six
            // lines while the row stays bottom-aligned.
            let field_padding = 14.0;
            let button_width = line_height + field_padding;
            let text_height = ui
                .ctx()
                .read_response(id)
                .map(|previous| previous.rect.height())
                .unwrap_or(line_height)
                .clamp(line_height, line_height * 6.0);
            let row_height = (text_height + field_padding).max(button_width);
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), row_height),
                Layout::left_to_right(Align::Max),
                |ui| {
                if app.editing.is_none()
                    && theme::icon_button(
                        ui,
                        Icon::Paperclip,
                        20.0,
                        palette.secondary,
                        palette.text,
                        "Send files (or drop them on the window)",
                    )
                    .tab_stop(Stop::Attach)
                    .clicked()
                {
                    app.actions.push(Action::Attach);
                }
                if app.editing.is_none() && theme::icon_button(ui, Icon::ListChecks, 20.0, palette.secondary, palette.text, "Create poll").tab_stop(Stop::Poll).clicked() {
                    app.actions.push(Action::ShowDialog(Dialog::CreatePoll(chat.id.clone())));
                }
                if app.editing.is_none() {
                    let smile = theme::icon_button(
                        ui,
                        Icon::Smile,
                        20.0,
                        if app.picker.is_some() {
                            palette.accent
                        } else {
                            palette.secondary
                        },
                        palette.text,
                        "Emoji, GIFs, and stickers",
                    ).tab_stop(Stop::Emoji);
                    app.picker_anchor = Some(smile.rect);
                    if smile.clicked() {
                        app.actions.push(Action::TogglePicker(PickerTab::Emoji));
                    }
                }
                let field_width = (ui.available_width() - button_width - 10.0).max(0.0);
                let field = Frame::new()
                    .fill(palette.surface)
                    .corner_radius(CornerRadius::same(theme::RADIUS + 4))
                    .inner_margin(Margin::symmetric(12, 7))
                    .show(ui, |ui| {
                        ui.set_width((field_width - 24.0).max(0.0));
                        // Grow from one to six lines, then scroll.
                        egui::ScrollArea::vertical()
                            .id_salt("composer-scroll")
                            .max_height(line_height * 6.0)
                            .min_scrolled_height(0.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                // Keep emoji in the buffer so character offsets match, then
                                // paint their color bitmaps over the transparent glyphs.
                                let mut clusters: Vec<(usize, usize, String)> = Vec::new();
                                let format = egui::TextFormat::simple(
                                    theme::regular(BODY_SIZE),
                                    palette.text,
                                );
                                let composer_rtl = crate::bidi::base_rtl(&app.composer);
                                let mut layouter = |ui: &egui::Ui,
                                                    text: &dyn egui::TextBuffer,
                                                    wrap: f32| {
                                    let (galley, found) = crate::bidi::layout_editor(
                                        ui,
                                        text.as_str(),
                                        &format,
                                        wrap,
                                        true,
                                    );
                                    clusters = found;
                                    galley
                                };
                                let output = egui::TextEdit::multiline(&mut app.composer)
                                    .id(id)
                                    .frame(Frame::NONE)
                                    .margin(Margin::ZERO)
                                    .hint_text(
                                        egui::RichText::new(if app.pending.is_empty() {
                                            crate::i18n::gettext(app.locale, "Type a message")
                                                .into_owned()
                                        } else {
                                            crate::i18n::gettext(app.locale, "Add a caption")
                                                .into_owned()
                                        })
                                        .color(palette.dim)
                                        .font(theme::regular(BODY_SIZE)),
                                    )
                                    .font(theme::regular(BODY_SIZE))
                                    .text_color(palette.text)
                                    .desired_rows(1)
                                    .desired_width(f32::INFINITY)
                                    .horizontal_align(if composer_rtl {
                                        Align::RIGHT
                                    } else {
                                        Align::LEFT
                                    })
                                    .return_key(if enter_sends {
                                        Some(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter))
                                    } else {
                                        Some(KeyboardShortcut::new(Modifiers::NONE, Key::Enter))
                                    })
                                    .layouter(&mut layouter)
                                    .show(ui);
                                for (start, length, cluster) in &clusters {
                                    let Some(bounds) = crate::bidi::char_bounds(
                                        &output.galley,
                                        *start,
                                        start + length,
                                    ) else {
                                        continue;
                                    };
                                    // Skip emoji clusters split across rows.
                                    if bounds.height() > line_height * 1.5 {
                                        continue;
                                    }
                                    let rect = bounds.translate(output.galley_pos.to_vec2());
                                    crate::emoji::paint_cluster(ui, cluster, rect);
                                }
                                let response = output.response.response.clone().tab_stop(Stop::Composer);
                                ui.ctx().accesskit_node_builder(response.id, |node| node.set_label("Message"));
                                if response.changed() {
                                    app.actions.push(Action::Composing {
                                        chat: chat.id.clone(),
                                        composing: true,
                                    });
                                    let cursor = output
                                        .cursor_range
                                        .map(|range| range.primary.index.0)
                                        .unwrap_or_else(|| app.composer.chars().count());
                                    if typed_colon
                                        && let Some(at) = standalone_trigger(
                                            &app.composer,
                                            cursor,
                                            ':',
                                        )
                                    {
                                        app.picker = None;
                                        app.emoji_start = Some(at);
                                        app.emoji_selected = 0;
                                        app.mention_start = None;
                                    } else if typed_at
                                        && chat.is_group()
                                        && !chat.participants.is_empty()
                                        && let Some(at) = standalone_trigger(
                                            &app.composer,
                                            cursor,
                                            '@',
                                        )
                                    {
                                        app.emoji_start = None;
                                        app.mention_start = Some(at);
                                        app.mention_selected = 0;
                                    } else if app.emoji_start.is_some() {
                                        app.emoji_selected = 0;
                                    }
                                }
                                let cursor = output
                                    .cursor_range
                                    .map(|range| range.primary.index.0)
                                    .unwrap_or_else(|| app.composer.chars().count());
                                if app.emoji_start.is_some()
                                    && active_emoji(&app.composer, app.emoji_start, cursor).is_none()
                                {
                                    app.emoji_start = None;
                                }
                                if app.mention_start.is_some()
                                    && active_mention(&app.composer, app.mention_start, cursor)
                                        .is_none()
                                {
                                    app.mention_start = None;
                                }
                                // The composer waits for the image preview to
                                // close before taking focus back.
                                if app.focus_composer && app.image_preview.is_none() {
                                    app.focus_composer = false;
                                    response.request_focus();
                                }
                            });
                    });
                theme::focus_outline(ui, id, field.response.rect, f32::from(theme::RADIUS + 4));
                let ready = !app.composer.trim().is_empty() || !app.pending.is_empty();
                let (fill, hover, icon) = if ready {
                    (palette.accent, palette.accent_hover, palette.on_accent)
                } else {
                    (palette.surface, palette.surface_hover, palette.dim)
                };
                if !ready && app.editing.is_none() {
                    // An empty composer changes the send button to record.
                    if theme::circle_button(
                        ui,
                        Icon::Mic,
                        button_width,
                        fill,
                        hover,
                        palette.secondary,
                        "Record a voice message",
                    )
                    .tab_stop(Stop::Send)
                    .clicked()
                    {
                        app.actions.push(Action::StartRecording);
                    }
                } else {
                    let icon_kind = if app.editing.is_some() {
                        Icon::Check
                    } else {
                        Icon::Send
                    };
                    if theme::circle_button(ui, icon_kind, button_width, fill, hover, icon, "Send")
                        .tab_stop(Stop::Send)
                        .clicked()
                    {
                        send_click = true;
                    }
                }
            },
            );
            if (send_key || send_click)
                && (!app.composer.trim().is_empty() || !app.pending.is_empty())
            {
                let text = std::mem::take(&mut app.composer);
                if app.pending.is_empty() {
                    app.actions.push(Action::SendText {
                        chat: chat.id.clone(),
                        text,
                        quoting: app.reply_to.clone(),
                    });
                } else {
                    app.actions.push(Action::SendPending {
                        chat: chat.id.clone(),
                        caption: text,
                    });
                }
                app.focus_composer = true;
            }
            if app.settings.show_shortcut_hints {
                let hint_text = if enter_sends {
                    crate::i18n::gettext(app.locale, "Enter sends · Shift+Enter for a new line · *bold* _italic_ ~strike~ · Ctrl+V pastes a picture")
                } else {
                    crate::i18n::gettext(app.locale, "Ctrl+Enter sends · *bold* _italic_ ~strike~ · Ctrl+V pastes a picture")
                };
                let hint = super::keys::label(hint_text.as_ref());
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    if theme::icon_button(
                        ui,
                        Icon::X,
                        13.0,
                        palette.dim,
                        palette.secondary,
                        crate::i18n::gettext(app.locale, "Hide shortcut hints (restore in Settings)").as_ref(),
                    )
                    .clicked()
                    {
                        app.actions.push(Action::HideShortcutHints);
                    }
                    theme::text(ui, &hint, theme::regular(11.0), palette.dim);
                    // Open the shortcut list without consuming typed `?`.
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::icon_button(
                            ui,
                            Icon::Keyboard,
                            13.0,
                            palette.dim,
                            palette.secondary,
                            &crate::i18n::gettext(app.locale, "All shortcuts ({})")
                                .replace("{}", &super::keys::label("Ctrl+/")),
                        )
                        .clicked()
                        {
                            app.actions.push(Action::ShowDialog(Dialog::Shortcuts));
                        }
                    });
                });
            }
        });
    // Toasts sit above the composer so they never cover its buttons.
    ui.ctx()
        .data_mut(|data| data.insert_temp(super::composer_rect_id(), shown.response.rect));
}

fn edit_strip(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    Frame::new()
        .fill(palette.surface)
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::icon(ui, Icon::Pencil, 16.0, palette.accent);
                theme::text(ui, "Editing message", theme::semibold(12.5), palette.accent);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if theme::icon_button(
                        ui,
                        Icon::X,
                        16.0,
                        palette.secondary,
                        palette.text,
                        "Stop editing (Esc)",
                    )
                    .clicked()
                    {
                        app.actions.push(Action::CancelEdit);
                    }
                });
            });
        });
    ui.add_space(6.0);
}

fn reply_strip(app: &mut App, ui: &mut egui::Ui, quoted: &Message) {
    let palette = app.palette;
    let who = if quoted.from_me {
        "You".to_owned()
    } else {
        app.display_name_or(&quoted.sender, quoted.sender_name.as_deref())
    };
    let summary = markup::plain(&quoted.summary(), &app.mention_list(quoted));
    Frame::new()
        .fill(palette.surface)
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width().max(0.0));
            ui.horizontal(|ui| {
                let (bar, _) = ui.allocate_exact_size(vec2(3.0, 34.0), Sense::hover());
                ui.painter().rect_filled(bar, 2.0, palette.accent);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.set_max_width((ui.available_width() - 40.0).max(0.0));
                    widgets::rich_text(
                        ui,
                        &format!("Replying to {who}"),
                        theme::semibold(12.5),
                        palette.accent,
                    );
                    widgets::rich_text(ui, &summary, theme::regular(12.5), palette.secondary);
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if theme::icon_button(
                        ui,
                        Icon::X,
                        16.0,
                        palette.secondary,
                        palette.text,
                        "Cancel reply (Esc)",
                    )
                    .clicked()
                    {
                        app.actions.push(Action::CancelReply);
                    }
                });
            });
        });
    ui.add_space(6.0);
}

/// App data needed while drawing a checked-out conversation.
struct View<'a> {
    palette: Palette,
    locale: crate::i18n::Locale,
    chat: &'a Chat,
    me: Option<&'a str>,
    auto_download: bool,
    connected: bool,
    poll_voting: &'a HashSet<(ChatId, String)>,
    interactive_pending: &'a HashSet<(ChatId, String)>,
    /// Show avatars for all incoming messages, not only groups.
    pictures: bool,
    anchor: Option<&'a str>,
    /// Demo/test: keep this message's context menu open.
    open_menu: Option<&'a str>,
    reaction: Option<&'a str>,
    /// The reaction picker was opened from the message's context menu.
    reaction_menu: bool,
    reaction_emoji: &'a [(String, u32)],
    keyboard_navigation: &'a std::cell::Cell<bool>,
    /// Resolves a name with the message's stored name as fallback.
    names_or: &'a dyn Fn(&str, Option<&str>) -> String,
    /// Resolves mention names without replacing our name with "You".
    mention_names: &'a dyn Fn(&str) -> String,
    avatars: &'a HashMap<String, Option<PathBuf>>,
    now: i64,
    /// Animate media only while this window is active.
    animate: bool,
    player: &'a crate::audio::Player,
    video: &'a crate::video::Player,
    copy_rows: &'a std::sync::Mutex<Vec<crate::transcript::Row>>,
}

fn messages(app: &mut App, ui: &mut egui::Ui, chat: &Chat) {
    let palette = app.palette;
    // Check out the conversation while drawing rows and collecting actions.
    let mut conversation = app.conversations.remove(&chat.id).unwrap_or_default();
    let typing = app.typing_in(&chat.id);
    let mut avatars = HashMap::new();
    if chat.is_group() || app.settings.show_sender_pictures {
        let mut senders: HashSet<String> = conversation
            .messages
            .iter()
            .filter(|message| !message.from_me)
            .map(|message| message.sender.clone())
            .collect();
        // A typing participant may not have a visible message.
        senders.extend(typing.iter().map(|(id, _)| id.clone()));
        for sender in senders {
            let picture = app.avatar(&sender);
            avatars.insert(sender, picture);
        }
    }
    let names_or = |id: &str, hint: Option<&str>| app.display_name_or(id, hint);
    let mention_names = |id: &str| app.mention_name(id);
    let keyboard_navigation = std::cell::Cell::new(false);
    let view = View {
        palette,
        locale: app.locale,
        chat,
        me: app.me.as_deref(),
        auto_download: app.settings.auto_download,
        connected: app.link.is_connected(),
        poll_voting: &app.poll_voting,
        interactive_pending: &app.interactive_sending,
        pictures: app.settings.show_sender_pictures,
        anchor: if conversation.loading_older || conversation.fetching_phone {
            None
        } else {
            app.scroll_anchor.as_deref()
        },
        open_menu: app.open_message_menu.as_deref(),
        reaction: app
            .reaction_target
            .as_ref()
            .filter(|(id, _)| id == &chat.id)
            .map(|(_, message)| message.as_str()),
        reaction_menu: app.reaction_beside_menu,
        reaction_emoji: &app.settings.reaction_emoji,
        keyboard_navigation: &keyboard_navigation,
        names_or: &names_or,
        mention_names: &mention_names,
        avatars: &avatars,
        now: crate::util::now(),
        animate: app.window_focused,
        player: &app.player,
        video: &app.video,
        copy_rows: app.copy_rows.as_ref(),
    };
    let mut actions = Vec::new();
    let mut anchored = false;
    // The first unread message is the count-th incoming one from the end.
    let divider = app
        .unread_divider
        .as_ref()
        .filter(|divider| divider.chat == chat.id)
        .and_then(|divider| {
            conversation
                .messages
                .iter()
                .rev()
                .filter(|message| !message.from_me)
                .nth(divider.count.saturating_sub(1) as usize)
                .map(|message| (message.id.clone(), divider.count, divider.placed))
        });
    let mut divider_placed = false;
    let selection: Option<Vec<String>> = app
        .selection
        .as_ref()
        .filter(|(selected_chat, _)| *selected_chat == chat.id)
        .map(|(_, ids)| ids.clone());
    let scroll_to_bottom =
        app.scroll_to_bottom && divider.as_ref().is_none_or(|(.., placed)| *placed);
    let app_pictures = app.settings.show_sender_pictures;
    // Do not animate programmatic scrolling. Pending animations can delay a
    // later request to reach the end.
    let mut edge_scrolled_up = false;
    let output = egui::ScrollArea::vertical()
        .id_salt(("messages", &chat.id))
        .auto_shrink([false, false])
        .stick_to_bottom(view.reaction.is_none())
        .scroll_source(if view.reaction.is_some() {
            egui::scroll_area::ScrollSource::NONE
        } else {
            Default::default()
        })
        .animated(false)
        .show(ui, |ui| {
            // Scroll while selecting near an edge. `scroll_with_delta` also
            // releases stick-to-bottom; setting the offset directly does not.
            let viewport = ui.clip_rect();
            *app.selection_view.lock().unwrap_or_else(|p| p.into_inner()) = Some(viewport);
            // Only a drag that has moved past a click, such as selecting
            // text, scrolls; a click near an edge does not.
            let held_inside = ui.input(|input| {
                input.pointer.primary_down()
                    && input.pointer.is_decidedly_dragging()
                    && input.pointer.press_origin().is_some_and(|origin| {
                        viewport.contains(origin) && origin.x < viewport.right() - 16.0
                    })
            });
            if view.reaction.is_none()
                && held_inside
                && let Some(pointer) = ui.input(|input| input.pointer.latest_pos())
            {
                let delta = edge_scroll(pointer.y, viewport.top(), viewport.bottom());
                if delta != 0.0 {
                    if delta < 0.0 {
                        edge_scrolled_up = true;
                    }
                    ui.scroll_with_delta_animation(
                        vec2(0.0, -delta),
                        egui::style::ScrollAnimation::none(),
                    );
                    ui.ctx().request_repaint();
                }
            }
            Frame::new()
                .inner_margin(Margin::symmetric(18, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 3.0;
                    top_of_history(ui, &palette, &conversation, chat, &mut actions);
                    let mut previous: Option<&Message> = None;
                    for message in &conversation.messages {
                        let new_day = previous.is_none_or(|previous| {
                            crate::util::day_key(previous.timestamp)
                                != crate::util::day_key(message.timestamp)
                        });
                        if new_day {
                            ui.add_space(8.0);
                            ui.vertical_centered(|ui| {
                                widgets::chip(
                                    ui,
                                    &palette,
                                    &crate::util::day_label(app.locale, message.timestamp),
                                );
                            });
                            ui.add_space(4.0);
                        }
                        if let Some((id, count, placed)) = &divider
                            && *id == message.id
                        {
                            ui.add_space(6.0);
                            let label = crate::i18n::ngettext(
                                app.locale,
                                "{} unread message",
                                "{} unread messages",
                                *count,
                            )
                            .replace("{}", &count.to_string());
                            let response = ui
                                .vertical_centered(|ui| widgets::chip(ui, &palette, &label))
                                .inner;
                            if !placed && view.anchor.is_none() {
                                response.scroll_to_me(Some(Align::Min));
                                divider_placed = true;
                            }
                            ui.add_space(4.0);
                        }
                        let show_sender = (chat.is_group() || app_pictures)
                            && !message.from_me
                            && (new_day
                                || previous.is_none_or(|previous| {
                                    previous.sender != message.sender || previous.from_me
                                }));
                        let response = bubble(ui, &view, message, show_sender, &mut actions);
                        if let (Some(selected), Some(response)) = (&selection, &response) {
                            if selected.contains(&message.id) {
                                ui.painter().rect(
                                    response.rect.expand(2.0),
                                    10.0,
                                    palette.accent.gamma_multiply(0.18),
                                    Stroke::new(2.0, palette.accent),
                                    egui::StrokeKind::Outside,
                                );
                            }
                            if response.clicked() {
                                let shift = ui.input(|input| input.modifiers.shift);
                                actions.push(if shift {
                                    Action::SelectRange(message.id.clone())
                                } else {
                                    Action::ToggleSelected(message.id.clone())
                                });
                            }
                        } else if let Some(response) = &response
                            && response.clicked()
                            && ui.input(|input| input.modifiers.command)
                        {
                            // Ctrl-click (Command-click on macOS) starts a selection.
                            actions.push(Action::SelectMessage(message.id.clone()));
                        }
                        if let Some(response) = response
                            && view.anchor == Some(message.id.as_str())
                        {
                            response.scroll_to_me(Some(Align::Center));
                            anchored = true;
                        }
                        previous = Some(message);
                    }
                    if !typing.is_empty() {
                        typing_bubble(ui, &view, &typing);
                    }
                    ui.add_space(4.0);
                    if scroll_to_bottom && !keyboard_navigation.get() && view.reaction.is_none() {
                        ui.scroll_to_rect_animation(
                            Rect::from_min_size(ui.cursor().min, Vec2::ZERO),
                            None,
                            egui::style::ScrollAnimation::none(),
                        );
                    }
                });
        });
    let at_bottom =
        output.state.offset.y + output.inner_rect.height() >= output.content_size.y - 24.0;
    // Keep the view at the end while initial content expands, until the user
    // scrolls with the wheel, trackpad, or scrollbar.
    let bar = Rect::from_min_max(
        pos2(output.inner_rect.right() - 16.0, output.inner_rect.top()),
        output.inner_rect.right_bottom(),
    );
    let reader_scrolled = ui.input(|input| {
        input.smooth_scroll_delta.y != 0.0
            || input
                .raw
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::MouseWheel { .. }))
            || (input.pointer.primary_down()
                && input
                    .pointer
                    .interact_pos()
                    .is_some_and(|pos| bar.contains(pos)))
    });
    let complete = conversation.complete;
    let loading = conversation.loading_older;
    let fetching = conversation.fetching_phone;
    let exhausted = conversation.phone_exhausted;
    app.conversations
        .insert(chat.id.clone(), std::mem::take(&mut conversation));
    app.at_bottom = at_bottom;
    if app.scroll_to_bottom && (reader_scrolled || keyboard_navigation.get()) {
        app.scroll_to_bottom = false;
    }
    if divider_placed {
        if let Some(divider) = app.unread_divider.as_mut() {
            divider.placed = true;
        }
        app.scroll_to_bottom = false;
    }
    if anchored {
        app.scroll_anchor = None;
    } else if let Some(anchor) = app.scroll_anchor.clone()
        && !loading
        && !fetching
        && !app
            .conversations
            .get(&chat.id)
            .is_some_and(|c| c.message(&anchor).is_some())
        && app
            .conversations
            .get(&chat.id)
            .is_none_or(|c| c.complete || !c.messages.is_empty())
    {
        // Keep the anchor when the first page is still loading. Event::Messages
        // will request it again.
        app.scroll_anchor = None;
    }
    // At the top, load more from the archive and then the phone. Short chats
    // request more immediately.
    let fits = output.content_size.y <= output.inner_rect.height() + 1.0;
    let near_top = output.state.offset.y < 80.0;
    if (near_top || fits) && ((!complete && !loading) || (complete && !fetching && !exhausted)) {
        actions.push(Action::LoadOlder(chat.id.clone()));
    }
    app.actions.extend(actions);
    if edge_scrolled_up {
        // Scrolling up releases stick-to-bottom.
        app.scroll_to_bottom = false;
    }
    // Show a return-to-bottom button while reading older messages.
    if !at_bottom {
        let rect = output.inner_rect;
        let center = pos2(rect.right() - 34.0, rect.bottom() - 34.0);
        let button = Rect::from_center_size(center, Vec2::splat(40.0));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(button)
                .layout(Layout::centered_and_justified(egui::Direction::LeftToRight)),
        );
        let unread = app.chat(&chat.id).map_or(0, |chat| chat.unread);
        if theme::circle_button(
            &mut child,
            Icon::ArrowDown,
            40.0,
            palette.overlay,
            palette.surface_hover,
            palette.text,
            "Newest message",
        )
        .clicked()
        {
            app.actions.push(Action::ScrollToBottom);
        }
        if unread > 0 {
            widgets::badge(
                ui,
                &palette,
                pos2(center.x + 14.0, center.y - 16.0),
                unread,
                false,
            );
        }
    }
}

/// Loading state above the oldest visible message.
fn top_of_history(
    ui: &mut egui::Ui,
    palette: &Palette,
    conversation: &Conversation,
    _chat: &Chat,
    _actions: &mut [Action],
) {
    ui.vertical_centered(|ui| {
        if !conversation.complete {
            if conversation.loading_older {
                theme::spinner(ui, 18.0, palette.accent);
            } else {
                ui.add_space(18.0);
            }
        } else if conversation.fetching_phone {
            ui.horizontal(|ui| {
                let width = 260.0;
                ui.add_space((ui.available_width() - width).max(0.0) / 2.0);
                theme::spinner(ui, 16.0, palette.accent);
                theme::text(
                    ui,
                    "Loading older messages from your phone…",
                    theme::regular(12.5),
                    palette.secondary,
                );
            });
        } else if conversation.messages.is_empty() {
            ui.add_space(24.0);
            if conversation.fetching_phone {
                widgets::chip(ui, palette, "Loading messages from your phone…");
            } else {
                widgets::chip(ui, palette, "No messages here yet");
            }
        } else {
            ui.add_space(6.0);
        }
    });
}

/// Typing indicator with stacked avatars and animated dots.
fn typing_bubble(ui: &mut egui::Ui, view: &View<'_>, typers: &[(String, String)]) {
    let palette = view.palette;
    ui.horizontal(|ui| {
        if view.chat.is_group() || view.pictures {
            let count = typers.len().min(3);
            let step = SENDER_AVATAR * 0.6;
            let width = SENDER_AVATAR + step * (count.saturating_sub(1)) as f32;
            let (rect, _) = ui.allocate_exact_size(vec2(width, SENDER_AVATAR), Sense::hover());
            if ui.is_rect_visible(rect) {
                for (index, (id, name)) in typers.iter().take(count).enumerate() {
                    let avatar = Rect::from_min_size(
                        rect.min + vec2(step * index as f32, 0.0),
                        Vec2::splat(SENDER_AVATAR),
                    );
                    if index > 0 {
                        // Outline overlapping avatars with the chat background.
                        ui.painter().circle_filled(
                            avatar.center(),
                            SENDER_AVATAR / 2.0 + 1.5,
                            palette.chat,
                        );
                    }
                    widgets::paint_avatar(
                        ui,
                        &palette,
                        avatar,
                        name.trim_start_matches('~'),
                        id,
                        view.avatars.get(id).and_then(|picture| picture.as_deref()),
                    );
                }
            }
            ui.add_space(2.0);
        }
        Frame::new()
            .fill(palette.bubble_in)
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(12, 9))
            .show(ui, |ui| typing_dots(ui, &palette));
    });
}

fn typing_dots(ui: &mut egui::Ui, palette: &Palette) {
    let radius = 3.0;
    let gap = 5.0;
    let lift = 3.0;
    let (rect, _) = ui.allocate_exact_size(
        vec2(radius * 6.0 + gap * 2.0, radius * 2.0 + lift),
        Sense::hover(),
    );
    if !ui.is_rect_visible(rect) {
        return;
    }
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(100));
    let time = ui.input(|input| input.time);
    for index in 0..3 {
        let wave = ((time * std::f64::consts::TAU / 1.2) - f64::from(index) * 0.9).sin() as f32;
        let rise = wave.max(0.0);
        let center = pos2(
            rect.left() + radius + (radius * 2.0 + gap) * index as f32,
            rect.bottom() - radius - rise * lift,
        );
        ui.painter().circle_filled(
            center,
            radius,
            palette.secondary.gamma_multiply(0.45 + 0.55 * rise),
        );
    }
}

const REACTION_AFFORDANCE_SIZE: f32 = 26.0;
const REACTION_AFFORDANCE_GAP: f32 = 6.0;

/// Places the hover reaction control beside the bubble, swapping sides near an edge.
pub fn reaction_affordance_rect(bubble: Rect, bounds: Rect, own: bool) -> Rect {
    let size = Vec2::splat(REACTION_AFFORDANCE_SIZE);
    let before = bubble.left() - REACTION_AFFORDANCE_GAP - size.x;
    let after = bubble.right() + REACTION_AFFORDANCE_GAP;
    let (preferred, fallback) = if own {
        (before, after)
    } else {
        (after, before)
    };
    let fits = |x: f32| x >= bounds.left() && x + size.x <= bounds.right();
    let x = if fits(preferred) {
        preferred
    } else if fits(fallback) {
        fallback
    } else {
        preferred.clamp(bounds.left(), (bounds.right() - size.x).max(bounds.left()))
    };
    // Centred on the whole bubble, quote and footer included.
    let y = (bubble.center().y - size.y / 2.0)
        .clamp(bounds.top(), (bounds.bottom() - size.y).max(bounds.top()));
    Rect::from_min_size(pos2(x, y), size)
}

/// Includes the small gap so moving from the bubble to the control does not hide it.
fn reaction_affordance_visible(pointer: Option<egui::Pos2>, bubble: Rect, button: Rect) -> bool {
    pointer.is_some_and(|pointer| bubble.union(button).contains(pointer))
}

fn open_reaction_picker_action(chat: &str, message: &str) -> Action {
    Action::OpenReactionPicker {
        chat: chat.to_owned(),
        message: message.to_owned(),
        beside_menu: false,
    }
}

/// The screen-reader label names whose message the hover control reacts to, so
/// a user can tell the controls apart. The visual tooltip stays a short "React".
fn reaction_button_label(from_me: bool, sender: &str) -> String {
    if from_me {
        "React to your message".to_owned()
    } else {
        format!("React to {sender}'s message")
    }
}

/// Draws a Smile control beside a hovered message and opens the existing picker.
fn reaction_affordance(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    bubble: &egui::Response,
    actions: &mut Vec<Action>,
) {
    let bounds = ui.clip_rect().shrink(2.0);
    if bounds.width() < REACTION_AFFORDANCE_SIZE || bounds.height() < REACTION_AFFORDANCE_SIZE {
        return;
    }
    // Only the part of the bubble on screen can be hovered. Anchoring to it
    // keeps a bubble scrolled mostly out of view from parking its controls,
    // and catching the pointer, at the edge of the transcript.
    let shown = bubble.rect.intersect(bounds);
    if shown.height() < REACTION_AFFORDANCE_SIZE || shown.width() <= 0.0 {
        return;
    }
    let rect = reaction_affordance_rect(shown, bounds, message.from_me);
    let pointer = ui.input(|input| input.pointer.interact_pos());
    // Stay hidden under any floating layer, as the context menu does. The
    // affordance sits beside the bubble, so test the bubble itself rather than
    // the pointer, which may already be over the button, outside the bubble's
    // layer.
    let uncovered = ui
        .ctx()
        .layer_id_at(bubble.rect.center())
        .is_none_or(|layer| layer == bubble.layer_id);

    // Publish where the control actually landed, the same way the bubble rect is
    // published, so tests can act on the real rect instead of guessing it.
    ui.ctx()
        .data_mut(|data| data.insert_temp(bubble.id.with("react-rect"), rect));

    // Acquire under the same layer as the bubble so the row strip keeps clicks.
    let response = ui.interact(rect, bubble.id.with("react"), Sense::click());
    let sender = (view.names_or)(&message.sender, message.sender_name.as_deref());
    let label = reaction_button_label(message.from_me, &sender);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &label)
    });
    theme::reveal_focus(&response);
    // Reply sits one step further out, where the chat accepts messages.
    let can_reply = view.chat.can_send()
        && !matches!(
            message.content,
            Content::Revoked | Content::PhoneOnly { .. }
        );
    let step = REACTION_AFFORDANCE_SIZE + 4.0;
    let outward = if rect.center().x < bubble.rect.center().x {
        -step
    } else {
        step
    };
    let reply_rect = rect.translate(vec2(outward, 0.0));
    let reply_fits = can_reply && bounds.contains_rect(reply_rect);
    let reach = if reply_fits {
        rect.union(reply_rect)
    } else {
        rect
    };
    let revealed = reaction_affordance_visible(pointer, shown, reach);
    if ui.is_rect_visible(rect) && (response.has_focus() || (uncovered && revealed)) {
        ui.painter().circle_filled(
            rect.center(),
            rect.width() / 2.0,
            if response.hovered() {
                view.palette.surface_hover
            } else {
                view.palette.surface.gamma_multiply(0.72)
            },
        );
        theme::paint_icon(
            ui,
            Icon::Smile,
            rect.shrink(5.0),
            15.0,
            if response.hovered() {
                view.palette.text
            } else {
                view.palette.secondary
            },
        );
    }
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("React")
        .clicked()
    {
        actions.push(open_reaction_picker_action(&view.chat.id, &message.id));
    }
    if !reply_fits {
        return;
    }
    let reply = ui.interact(reply_rect, bubble.id.with("reply"), Sense::click());
    reply.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Reply")
    });
    theme::reveal_focus(&reply);
    if ui.is_rect_visible(reply_rect) && (reply.has_focus() || (uncovered && revealed)) {
        ui.painter().circle_filled(
            reply_rect.center(),
            reply_rect.width() / 2.0,
            if reply.hovered() {
                view.palette.surface_hover
            } else {
                view.palette.surface.gamma_multiply(0.72)
            },
        );
        theme::paint_icon(
            ui,
            Icon::Reply,
            reply_rect.shrink(5.0),
            15.0,
            if reply.hovered() {
                view.palette.text
            } else {
                view.palette.secondary
            },
        );
    }
    if reply
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Reply")
        .clicked()
    {
        actions.push(Action::Reply(message.id.clone()));
    }
}

/// Draws a message row and returns its bubble response for scrolling.
fn bubble(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    show_sender: bool,
    actions: &mut Vec<Action>,
) -> Option<egui::Response> {
    let own = message.from_me;
    // Greys that read on the panel can vanish on the bubble; use its own.
    let view = &View {
        palette: view.palette.on_bubble(own),
        ..*view
    };
    let with_avatar = !own && (view.chat.is_group() || view.pictures);
    let carousel = matches!(&message.content, Content::Interactive { card: Some(card), .. } if !card.carousel.is_empty());
    let max_width = ((ui.available_width() * if carousel { 0.95 } else { 0.72 })
        .min(if carousel { 920.0 } else { 560.0 })
        - if with_avatar {
            SENDER_AVATAR + 8.0
        } else {
            0.0
        })
    .max(0.0);
    // Register the empty strip beside the bubble from its previous rect, before
    // the row, so the avatar, the bubble, and the reactions win clicks.
    let id = bubble_id(&view.chat.id, &message.id);
    let previous = ui.ctx().data(|data| data.get_temp::<Rect>(id.with("rect")));
    if let Some(rect) = previous {
        let strip = Rect::from_x_y_ranges(ui.max_rect().x_range(), rect.y_range());
        let strip = ui.interact(strip, id.with("row"), Sense::CLICK);
        reply_on_double_click(&strip, message, actions);
    }
    let mut response = None;
    ui.with_layout(
        Layout::top_down(if own { Align::Max } else { Align::Min }),
        |ui| {
            if with_avatar {
                ui.horizontal_top(|ui| {
                    let (rect, avatar) = ui.allocate_exact_size(
                        Vec2::splat(SENDER_AVATAR),
                        if show_sender {
                            Sense::CLICK
                        } else {
                            Sense::hover()
                        },
                    );
                    theme::focus_outline(ui, avatar.id, rect, SENDER_AVATAR / 2.0);
                    if show_sender
                        && avatar
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                    {
                        actions.push(Action::ShowDialog(Dialog::ChatInfo(message.sender.clone())));
                    }
                    if show_sender && ui.is_rect_visible(rect) {
                        let name = (view.names_or)(&message.sender, message.sender_name.as_deref());
                        widgets::paint_avatar(
                            ui,
                            &view.palette,
                            rect,
                            name.trim_start_matches('~'),
                            &message.sender,
                            view.avatars
                                .get(&message.sender)
                                .and_then(|picture| picture.as_deref()),
                        );
                    }
                    // Keep bubble content vertically laid out inside the row.
                    ui.vertical(|ui| {
                        response = Some(bubble_frame(
                            ui,
                            view,
                            message,
                            show_sender,
                            max_width,
                            actions,
                        ));
                    });
                });
            } else {
                response = Some(bubble_frame(
                    ui,
                    view,
                    message,
                    show_sender,
                    max_width,
                    actions,
                ));
            }
            if !message.reactions.is_empty() {
                ui.add_space(-7.0);
                ui.horizontal(|ui| {
                    if with_avatar {
                        ui.add_space(SENDER_AVATAR + 8.0);
                    }
                    reactions(ui, view, message, actions);
                });
                ui.add_space(2.0);
            }
        },
    );
    response
}

/// Clamps message-selection drags to the view while the pointer is outside it.
/// This keeps a row under the pointer during edge scrolling. The input hook
/// adjusts positions before egui processes them, using the previous frame's
/// view rectangle.
pub struct SelectionLeash {
    pub view: std::sync::Arc<std::sync::Mutex<Option<Rect>>>,
    holding: bool,
}

impl SelectionLeash {
    pub fn new(view: std::sync::Arc<std::sync::Mutex<Option<Rect>>>) -> Self {
        Self {
            view,
            holding: false,
        }
    }
}

impl egui::plugin::Plugin for SelectionLeash {
    fn debug_name(&self) -> &'static str {
        "zapfast-selection-leash"
    }

    fn input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        let Some(view) = *self.view.lock().unwrap_or_else(|p| p.into_inner()) else {
            self.holding = false;
            return;
        };
        let inside = |pos: &egui::Pos2| view.contains(*pos) && pos.x < view.right() - 16.0;
        let mut gone = Vec::new();
        for (index, event) in input.events.iter_mut().enumerate() {
            match event {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    ..
                } => {
                    if *pressed {
                        self.holding = inside(pos);
                    } else {
                        if self.holding && !view.contains(*pos) {
                            *pos = clamp_into(*pos, view);
                        }
                        self.holding = false;
                    }
                }
                egui::Event::PointerMoved(pos) if self.holding && !view.contains(*pos) => {
                    *pos = clamp_into(*pos, view);
                }
                // Ignore PointerGone during a drag so selection continues.
                egui::Event::PointerGone if self.holding => gone.push(index),
                _ => {}
            }
        }
        for index in gone.into_iter().rev() {
            input.events.remove(index);
        }
    }
}

fn clamp_into(pos: egui::Pos2, view: Rect) -> egui::Pos2 {
    egui::pos2(
        pos.x.clamp(view.left() + 2.0, view.right() - 18.0),
        pos.y.clamp(view.top() + 2.0, view.bottom() - 2.0),
    )
}

/// Builds the transcript row used when copying across messages.
fn transcript_row(
    view: &View<'_>,
    message: &Message,
    body: String,
    placements: Vec<String>,
) -> crate::transcript::Row {
    let who = if message.from_me {
        (view.mention_names)(&message.sender)
    } else {
        (view.names_or)(&message.sender, message.sender_name.as_deref())
    };
    let marker = match &message.content {
        Content::Image { .. } => Some("[photo]".to_owned()),
        Content::Video { gif: true, .. } => Some("[GIF]".to_owned()),
        Content::Video { .. } => Some("[video]".to_owned()),
        Content::Audio {
            voice_note: true,
            seconds,
            ..
        } => Some(match seconds {
            Some(seconds) => format!("[voice message, {}]", crate::util::duration(*seconds)),
            None => "[voice message]".to_owned(),
        }),
        Content::Audio { .. } => Some("[audio]".to_owned()),
        Content::Document { file_name, .. } => Some(format!("[document: {file_name}]")),
        Content::Sticker { .. } => Some("[sticker]".to_owned()),
        Content::Location { .. } => Some("[location]".to_owned()),
        Content::LiveLocation { ended, .. } => Some(
            if *ended {
                "[live location ended]"
            } else {
                "[live location]"
            }
            .to_owned(),
        ),
        Content::Contact { display_name, .. } => Some(format!("[contact: {display_name}]")),
        Content::Poll { question, .. } => Some(format!("[poll: {question}]")),
        Content::Interactive {
            card: Some(card), ..
        } if card.image.is_some() || card.carousel.iter().any(|card| card.image.is_some()) => {
            Some("[photo]".to_owned())
        }
        _ => None,
    };
    let reactions = if message.reactions.is_empty() {
        String::new()
    } else {
        let listed: Vec<String> = message
            .reactions
            .iter()
            .map(|reaction| {
                format!(
                    "{} {}",
                    reaction.emoji,
                    (view.names_or)(&reaction.sender, None)
                )
            })
            .collect();
        format!(" ({})", listed.join(", "))
    };
    let quote = message.quoted.as_ref().map(|quoted| {
        let name = quoted
            .sender_name
            .clone()
            .unwrap_or_else(|| (view.names_or)(&quoted.sender, None));
        let summary = quoted.summary.clone();
        let short: String = summary.chars().take(48).collect();
        let cut = if summary.chars().count() > 48 {
            "…"
        } else {
            ""
        };
        format!("(replying to {name}: \"{short}{cut}\")")
    });
    crate::transcript::Row {
        header: format!("[{}] {}: ", crate::util::copy_stamp(message.timestamp), who),
        body,
        placements,
        marker,
        reactions,
        quote,
    }
}

/// Selection-scroll distance based on pointer proximity to the view edge.
pub fn edge_scroll(pointer: f32, top: f32, bottom: f32) -> f32 {
    const EDGE: f32 = 36.0;
    const PACE: f32 = 0.3;
    if pointer < top + EDGE {
        -(top + EDGE - pointer).min(EDGE * 1.5) * PACE
    } else if pointer > bottom - EDGE {
        (pointer - (bottom - EDGE)).min(EDGE * 1.5) * PACE
    } else {
        0.0
    }
}

/// Starts a reply when the response was double-clicked, as the menu's "Reply".
fn reply_on_double_click(response: &egui::Response, message: &Message, actions: &mut Vec<Action>) {
    if response.double_clicked() && !matches!(message.content, Content::Revoked) {
        actions.push(Action::Reply(message.id.clone()));
    }
}

/// Stable message-bubble id used by interaction tests.
pub fn bubble_id(chat: &str, message: &str) -> egui::Id {
    egui::Id::new(("bubble", chat, message))
}

/// Where a voice message's speed chip was drawn, for interaction tests.
pub fn speed_chip_id(chat: &str, message: &str) -> egui::Id {
    egui::Id::new(("speed-chip", chat, message))
}

/// Where a speed choice in a voice message's menu was drawn, for
/// interaction tests.
pub fn speed_button_id(chat: &str, message: &str, speed: f32) -> egui::Id {
    egui::Id::new(("speed", chat, message, speed.to_bits()))
}

/// Draws a playback speed pill labelled with `speed`, highlighted when
/// `active` and faded while that speed is still `preparing`.
fn speed_pill(
    ui: &mut egui::Ui,
    view: &View<'_>,
    size: Vec2,
    speed: f32,
    active: bool,
    preparing: bool,
) -> egui::Response {
    let palette = view.palette;
    let label = crate::audio::speed_label(speed);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            ui.is_enabled(),
            active,
            format!("Playback speed {label}"),
        )
    });
    theme::reveal_focus(&response);
    theme::focus_outline(ui, response.id, rect, rect.height() / 2.0);
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        // The resting fill uses the hover step because incoming bubbles
        // share the resting surface colour.
        let fill = if active {
            palette
                .accent
                .gamma_multiply(if hovered { 0.42 } else { 0.30 })
        } else if hovered {
            palette.surface_active
        } else {
            palette.surface_hover
        };
        ui.painter().rect_filled(rect, rect.height() / 2.0, fill);
        let colour = if active {
            palette.accent
        } else {
            palette.secondary
        };
        let colour = if preparing {
            colour.gamma_multiply(0.5)
        } else {
            colour
        };
        let galley = ui
            .painter()
            .layout_no_wrap(label, theme::medium(11.0), colour);
        ui.painter()
            .galley(rect.center() - galley.size() / 2.0, galley, colour);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The message menu's row of every playback speed for a playable voice
/// message, so 1.25x and 1.75x are reachable without cycling.
fn speed_menu_row(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    actions: &mut Vec<Action>,
) {
    let speed = view.player.speed();
    let preparing = view.player.preparing_speed(&message.id);
    widgets::menu_separator(ui, &view.palette);
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 28.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add_space(6.0);
            for option in crate::audio::SPEEDS {
                let selected = option == speed;
                let response = speed_pill(
                    ui,
                    view,
                    vec2(44.0, 24.0),
                    option,
                    selected,
                    preparing && selected,
                );
                ui.ctx().data_mut(|data| {
                    data.insert_temp(
                        speed_button_id(&view.chat.id, &message.id, option),
                        response.rect,
                    );
                });
                if response.clicked() {
                    actions.push(Action::SetVoiceSpeed(option));
                    ui.close();
                }
            }
        },
    );
}

/// Draws a message bubble and its menu.
fn bubble_frame(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    show_sender: bool,
    max_width: f32,
    actions: &mut Vec<Action>,
) -> egui::Response {
    let palette = view.palette;
    let own = message.from_me;
    let carousel = matches!(&message.content, Content::Interactive { card: Some(card), .. } if !card.carousel.is_empty());
    // Stickers and round video messages draw without a bubble.
    let bare = matches!(
        message.content,
        Content::Sticker { .. } | Content::Video { note: true, .. }
    );
    let fill = if carousel || bare {
        Color32::TRANSPARENT
    } else if own {
        palette.bubble_out
    } else {
        palette.bubble_in
    };
    // Register the bubble from its previous rect before its contents so inner
    // links, quotes, and attachments win clicks. The bubble handles right-click.
    let bubble_id = bubble_id(&view.chat.id, &message.id);
    // Store the final rect separately. Reusing the early response would keep
    // the first frame's rect.
    let rect_id = bubble_id.with("rect");
    let previous = ui.ctx().data(|data| data.get_temp::<Rect>(rect_id));
    let early = previous.map(|rect| ui.interact(rect, bubble_id, Sense::CLICK));
    let inner = Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin {
            left: 10,
            right: 10,
            top: 6,
            bottom: 5,
        })
        .show(ui, |ui| {
            ui.set_max_width(max_width);
            ui.spacing_mut().item_spacing.y = 4.0;
            if show_sender && view.chat.is_group() {
                let name = (view.names_or)(&message.sender, message.sender_name.as_deref());
                let response = widgets::rich_text(
                    ui,
                    &name,
                    theme::semibold(13.0),
                    palette.sender(crate::util::hue(&message.sender)),
                );
                let response = ui
                    .interact(
                        response.rect,
                        ui.id().with(("sender", &message.id)),
                        Sense::CLICK,
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if response.clicked() {
                    actions.push(Action::ShowDialog(Dialog::ChatInfo(message.sender.clone())));
                }
            }
            if message.forwarded {
                mirrored_row(
                    ui,
                    own,
                    |ui| {
                        theme::icon(ui, Icon::Forward, 14.0, palette.dim);
                    },
                    |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new("Forwarded")
                                    .font(theme::regular(12.5))
                                    .italics()
                                    .color(palette.dim),
                            )
                            .selectable(false),
                        );
                    },
                );
            }
            // Cards share the bubble's settled width: at least CARD_WIDTH and
            // no more than the cap. Text spans that width and stays left-aligned.
            // Bubbles without cards use the natural text width.
            let cap = ((max_width - 20.0).min(ui.available_width())).max(0.0);
            let reserve = footer_width(ui, message);
            let settled = settled_width(ui, view, message, cap);
            let slot = match settled {
                Some(width) => {
                    if let Some(quoted) = &message.quoted {
                        quote_block(ui, view, message, quoted, width, actions);
                    }
                    content(ui, view, message, width, reserve, actions)
                }
                None => content(ui, view, message, cap, reserve, actions),
            };
            footer(ui, &palette, message, slot);
            if matches!(message.content, Content::Poll { .. }) {
                super::polls::results_button(
                    ui,
                    &palette,
                    message,
                    settled.unwrap_or(cap),
                    actions,
                );
            }
            if let Content::Interactive {
                card: Some(card), ..
            } = &message.content
            {
                interactive_buttons(ui, view, message, card, settled.unwrap_or(cap), actions);
            }
        });
    ui.ctx()
        .data_mut(|data| data.insert_temp(rect_id, inner.response.rect));
    let bubble = early.unwrap_or_else(|| ui.interact(inner.response.rect, bubble_id, Sense::CLICK));
    theme::reveal_focus(&bubble);
    theme::focus_outline(ui, bubble.id, inner.response.rect, 10.0);
    if ui.ctx().data(|data| {
        data.get_temp::<bool>(theme::keyboard_focus_id())
            .unwrap_or(false)
    }) && let Some(focused) = ui
        .memory(|memory| memory.focused())
        .and_then(|id| ui.ctx().read_response(id))
        && (focused.id == bubble.id || inner.response.rect.contains(focused.rect.center()))
    {
        view.keyboard_navigation.set(true);
        if focused.gained_focus() && !ui.clip_rect().contains_rect(focused.rect) {
            ui.scroll_to_rect_animation(
                inner.response.rect.expand(4.0),
                None,
                egui::style::ScrollAnimation::none(),
            );
        }
    }
    // The context menu stays open beside the picker only when the picker came
    // from the menu itself.
    let reacting = view.reaction_menu && view.reaction == Some(message.id.as_str());
    // Inner widgets own their clicks, so this fires only on the bubble's padding
    // and footer. Double-click on the body keeps selecting the word.
    reply_on_double_click(&bubble, message, actions);

    reaction_affordance(ui, view, message, &bubble, actions);
    // Read right-click from input because inner widgets own their responses.
    // Open only when no floating layer covers the chat panel.
    let right_clicked = ui.input(|input| {
        input.pointer.secondary_clicked()
            && input
                .pointer
                .interact_pos()
                .is_some_and(|pos| bubble.rect.contains(pos))
    }) && ui
        .input(|input| input.pointer.interact_pos())
        .is_some_and(|pos| {
            ui.ctx()
                .layer_id_at(pos)
                .is_none_or(|layer| layer == bubble.layer_id)
        });
    let force_menu = view.open_menu == Some(message.id.as_str());
    let quick = quick_reactions(message, view.reaction_emoji).len() as f32 + 1.0;
    let width = widgets::menu_width(
        ui,
        &[
            "Delete for everyone",
            "Show in folder",
            if crate::util::twelve_hour_clock() {
                "Delivered Yesterday at 11:59 PM"
            } else {
                "Delivered Yesterday at 20:45"
            },
        ],
        true,
    )
    .max(quick * 36.0 + 12.0);
    let keyboard_clicked =
        bubble.clicked() && bubble.has_focus() && !ui.input(|input| input.pointer.any_click());
    let open = if right_clicked || force_menu || reacting || keyboard_clicked {
        Some(egui::SetOpenCommand::Bool(true))
    } else if bubble.clicked() {
        Some(egui::SetOpenCommand::Bool(false))
    } else {
        None
    };
    let popup = egui::Popup::menu(&bubble)
        .open_memory(open)
        .close_behavior(if reacting {
            egui::PopupCloseBehavior::IgnoreClicks
        } else {
            egui::PopupCloseBehavior::CloseOnClickOutside
        })
        .width(width)
        .frame(widgets::menu_frame(&palette));
    let popup = if reacting {
        // Keep the menu next to the picker, anchored to this message rather
        // than whichever pointer position happened to open the emoji grid.
        let screen = ui.ctx().content_rect();
        let menu = ui
            .ctx()
            .data(|data| data.get_temp::<Rect>(bubble_id.with("menu-rect")))
            .unwrap_or(bubble.rect);
        let x = menu.left().clamp(
            screen.left() + 8.0,
            (screen.right() - width - 468.0).max(screen.left() + 8.0),
        );
        popup.at_position(pos2(x, menu.top()))
    } else if force_menu || keyboard_clicked {
        popup.at_position(bubble.rect.left_top() + vec2(12.0, 8.0))
    } else {
        popup.at_pointer_fixed()
    };
    let menu = popup.show(|ui| {
        context_menu(ui, view, message, actions);
    });
    if let Some(menu) = menu {
        ui.ctx()
            .data_mut(|data| data.insert_temp(bubble_id.with("menu-rect"), menu.response.rect));
    }
    // Keep the target explicit for every menu action, not just the emoji
    // picker. Paint in the message layer so the menu itself remains above it.
    if egui::Popup::is_id_open(ui.ctx(), bubble_id.with("popup")) {
        ui.painter().rect_stroke(
            inner.response.rect.expand(2.0),
            12.0,
            Stroke::new(theme::FOCUS_STROKE_WIDTH, palette.accent),
            egui::StrokeKind::Outside,
        );
    }
    if reacting && !egui::Popup::is_id_open(ui.ctx(), bubble_id.with("popup")) {
        actions.push(Action::ClosePicker);
    }
    // Store this frame's final rect for later scrolling.
    inner.response
}

/// Minimum shared width for cards inside message bubbles.
const CARD_WIDTH: f32 = 320.0;
const CAROUSEL_CARD_WIDTH: f32 = 280.0;
const CAROUSEL_GAP: f32 = 8.0;

/// Returns the shared card width, bounded by [`CARD_WIDTH`] and `cap`.
fn settled_width(ui: &egui::Ui, view: &View<'_>, message: &Message, cap: f32) -> Option<f32> {
    if let Content::Interactive {
        card: Some(card), ..
    } = &message.content
        && !card.carousel.is_empty()
    {
        // Keep short carousels, their heading and their timestamp together.
        // Wider strips still occupy the cap and scroll horizontally.
        let count = card.carousel.len();
        let cards = count as f32 * (CAROUSEL_CARD_WIDTH + 20.0);
        let gaps = count.saturating_sub(1) as f32 * CAROUSEL_GAP;
        return Some((cards + gaps).min(cap));
    }
    if let Content::Interactive {
        card: Some(card), ..
    } = &message.content
        && card.image.is_some()
    {
        return Some(CARD_WIDTH.min(cap));
    }
    let card = message.quoted.is_some()
        || match &message.content {
            Content::Text { preview, .. } => preview.is_some(),
            Content::Document { .. } | Content::Audio { .. } | Content::Poll { .. } => true,
            Content::Interactive { card, .. } => card.is_some(),
            // Videos without a poster use the file-row layout, except the
            // round ones, which always draw as a circle.
            Content::Video { note, .. } => !note && message.thumbnail.is_none(),
            _ => false,
        };
    card.then(|| {
        let floor = CARD_WIDTH.min(cap).max(0.0);
        natural_text_width(ui, view, message, cap).map_or(floor, |width| width.clamp(floor, cap))
    })
}

/// Widest wrapped text row, including footer space on the last line.
fn natural_text_width(ui: &egui::Ui, view: &View<'_>, message: &Message, cap: f32) -> Option<f32> {
    let palette = view.palette;
    let text = match &message.content {
        Content::Text { text, .. } | Content::Interactive { text, .. } => text,
        Content::Image {
            caption: Some(caption),
            ..
        }
        | Content::Video {
            caption: Some(caption),
            ..
        }
        | Content::Document {
            caption: Some(caption),
            ..
        } => caption,
        _ => return None,
    };
    let style = markup::Style {
        size: BODY_SIZE,
        color: palette.text,
        secondary: palette.secondary,
        link: palette.link,
        mention: palette.accent,
    };
    let laid = markup::layout(ui, text, &mentions_of(view, message), &style, cap);
    let widest = laid
        .galley
        .rows
        .iter()
        .map(|row| row.row.size.x)
        .fold(0.0, f32::max);
    let last = laid.galley.rows.last().map_or(0.0, |row| row.row.size.x);
    let reserve = footer_width(ui, message);
    Some(if last + 8.0 + reserve <= cap {
        widest.max(last + 8.0 + reserve)
    } else {
        widest
    })
}

fn quote_block(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    quoted: &crate::model::Quoted,
    width: f32,
    actions: &mut Vec<Action>,
) {
    let palette = view.palette;
    let who = if view.me == Some(quoted.sender.as_str()) {
        "You".to_owned()
    } else {
        (view.names_or)(&quoted.sender, quoted.sender_name.as_deref())
    };
    let summary = markup::plain(&quoted.summary, &quote_mentions(view, quoted));
    let response = Frame::new()
        .fill(palette.window.gamma_multiply(0.35))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin {
            left: 8,
            right: 10,
            top: 5,
            bottom: 5,
        })
        .show(ui, |ui| {
            // Include frame margins in the settled width. Use a bounded,
            // left-aligned layout because own bubbles inherit right-to-left flow.
            let inner_width = (width - 18.0).max(0.0);
            ui.allocate_ui_with_layout(
                vec2(inner_width, 0.0),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.set_width(inner_width);
                    ui.horizontal(|ui| {
                        let (bar, _) = ui.allocate_exact_size(vec2(3.0, 30.0), Sense::hover());
                        ui.painter().rect_filled(bar, 2.0, palette.accent);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            // Use the space beside the quote bar and gap.
                            ui.set_width((width - 29.0).max(0.0));
                            widgets::rich_text(ui, &who, theme::semibold(12.5), palette.accent);
                            widgets::rich_text(
                                ui,
                                &summary,
                                theme::regular(12.5),
                                palette.secondary,
                            );
                        });
                    });
                },
            );
        })
        .response;
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            bubble_id(&view.chat.id, &message.id).with("quote"),
            response.rect,
        );
    });
    let response = ui
        .interact(
            response.rect,
            ui.id().with(("quote", &message.id, &quoted.id)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        actions.push(Action::ScrollTo(quoted.id.clone()));
    }
}

/// Keeps row contents left-to-right inside right-aligned own bubbles.
fn mirrored_row(
    ui: &mut egui::Ui,
    own: bool,
    first: impl FnOnce(&mut egui::Ui),
    second: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        if own {
            second(ui);
            first(ui);
        } else {
            first(ui);
            second(ui);
        }
    });
}

/// Width of the message footer.
fn footer_width(ui: &egui::Ui, message: &Message) -> f32 {
    let font = theme::regular(11.0);
    let time = ui
        .painter()
        .layout_no_wrap(
            crate::util::clock(message.timestamp),
            font.clone(),
            Color32::WHITE,
        )
        .size()
        .x;
    let edited = if message.edited {
        ui.painter()
            .layout_no_wrap("edited".to_owned(), font, Color32::WHITE)
            .size()
            .x
            + 4.0
    } else {
        0.0
    };
    let not_sent = if not_sent(message) {
        ui.painter()
            .layout_no_wrap(NOT_SENT.to_owned(), theme::medium(11.0), Color32::WHITE)
            .size()
            .x
            + 6.0
    } else {
        0.0
    };
    time + edited + not_sent + if message.from_me { 19.0 } else { 0.0 }
}

fn not_sent(message: &Message) -> bool {
    message.from_me && message.status == Delivery::Failed
}

/// Paints the time and ticks at the bubble's right edge without widening it.
fn footer(ui: &mut egui::Ui, palette: &Palette, message: &Message, slot: Option<Rect>) {
    let font = theme::regular(11.0);
    let time = ui.painter().layout_no_wrap(
        crate::util::clock(message.timestamp),
        font.clone(),
        palette.secondary,
    );
    let edited = message.edited.then(|| {
        ui.painter()
            .layout_no_wrap("edited".to_owned(), font, palette.dim)
    });
    // A red dot alone does not say what went wrong or what to do. The word
    // uses the text colour: the danger red on an outgoing bubble is too faint
    // to read, and the red icon beside it already carries the alarm.
    let failed = not_sent(message).then(|| {
        ui.painter()
            .layout_no_wrap(NOT_SENT.to_owned(), theme::medium(11.0), palette.text)
    });
    let tick_width = if message.from_me { 19.0 } else { 0.0 };
    let width = time.size().x
        + edited.as_ref().map_or(0.0, |galley| galley.size().x + 4.0)
        + failed.as_ref().map_or(0.0, |galley| galley.size().x + 6.0)
        + tick_width;
    let rect = match slot {
        Some(slot) => slot,
        None => {
            let row_width = ui.min_rect().width().max(width);
            let (rect, _) = ui.allocate_exact_size(vec2(row_width, 15.0), Sense::hover());
            rect
        }
    };
    let mut x = rect.right();
    if message.from_me {
        let ticks = Rect::from_center_size(pos2(x - 7.5, rect.center().y), Vec2::splat(15.0));
        widgets::ticks(ui, palette, ticks, message.status);
        x -= tick_width;
    }
    x -= time.size().x;
    ui.painter().galley(
        pos2(x, rect.center().y - time.size().y / 2.0),
        time,
        palette.secondary,
    );
    if let Some(edited) = edited {
        x -= edited.size().x + 4.0;
        ui.painter().galley(
            pos2(x, rect.center().y - edited.size().y / 2.0),
            edited,
            palette.dim,
        );
    }
    if let Some(failed) = failed {
        x -= failed.size().x + 6.0;
        let label = Rect::from_min_size(
            pos2(x, rect.center().y - failed.size().y / 2.0),
            failed.size(),
        );
        ui.painter().galley(label.min, failed, palette.text);
        // Explain on hover and to screen readers; hovering takes no clicks
        // from the bubble.
        let status = Rect::from_min_max(label.min, pos2(rect.right(), label.max.y));
        let response = ui.interact(
            status,
            ui.id().with(("not-sent", &message.id)),
            Sense::hover(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, NOT_SENT_HINT)
        });
        response.on_hover_text(NOT_SENT_HINT);
    }
}

fn reactions(ui: &mut egui::Ui, view: &View<'_>, message: &Message, actions: &mut Vec<Action>) {
    let palette = view.palette;
    let mut counts: Vec<(String, u32, bool, Vec<String>)> = Vec::new();
    for reaction in &message.reactions {
        let who = if reaction.from_me {
            "You".to_owned()
        } else {
            (view.names_or)(&reaction.sender, None)
        };
        match counts
            .iter_mut()
            .find(|(emoji, _, _, _)| *emoji == reaction.emoji)
        {
            Some((_, count, mine, names)) => {
                *count += 1;
                *mine |= reaction.from_me;
                names.push(who);
            }
            None => counts.push((reaction.emoji.clone(), 1, reaction.from_me, vec![who])),
        }
    }
    ui.spacing_mut().item_spacing.x = 3.0;
    for (emoji, count, mine, names) in counts {
        let label = if count > 1 {
            format!("{emoji} {count}")
        } else {
            emoji.clone()
        };
        let line = widgets::line(ui, &label, theme::regular(13.0), palette.text, 200.0, 1);
        let size = line.size() + vec2(12.0, 6.0);
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        if ui.is_rect_visible(rect) {
            ui.painter()
                .rect_filled(rect, rect.height() / 2.0, palette.overlay);
            ui.painter().rect_stroke(
                rect,
                rect.height() / 2.0,
                Stroke::new(1.0, if mine { palette.accent } else { palette.chat }),
                egui::StrokeKind::Inside,
            );
            line.paint(ui, rect.center() - line.size() / 2.0, palette.text);
        }
        let response = response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(names.join(", "));
        if response.clicked() {
            // Clicking our reaction removes it; clicking another adds it.
            actions.push(Action::React {
                chat: view.chat.id.clone(),
                message: message.id.clone(),
                emoji: if mine { String::new() } else { emoji },
            });
        }
    }
}

const QUICK_REACTIONS: [&str; 6] = ["👍", "❤️", "😂", "😮", "😢", "🙏"];

/// Our existing reaction to a message.
pub(crate) fn own_reaction(message: &Message) -> Option<&str> {
    message
        .reactions
        .iter()
        .find(|reaction| reaction.from_me)
        .map(|reaction| reaction.emoji.as_str())
}

/// Emoji to send for a reaction choice: empty string removes our current one.
pub(crate) fn reaction_choice(current: Option<&str>, emoji: &str) -> String {
    if current == Some(emoji) {
        String::new()
    } else {
        emoji.to_owned()
    }
}

/// Quick reactions plus our current reaction when needed.
fn quick_reactions<'a>(message: &'a Message, preferred: &'a [(String, u32)]) -> Vec<&'a str> {
    let mut list = Vec::new();
    for emoji in preferred
        .iter()
        .map(|(emoji, _)| emoji.as_str())
        .chain(QUICK_REACTIONS.iter().copied())
    {
        if emojis::get(emoji).is_some() && !list.contains(&emoji) {
            list.push(emoji);
        }
        if list.len() == QUICK_REACTIONS.len() {
            break;
        }
    }
    if let Some(mine) = own_reaction(message)
        && !list.contains(&mine)
    {
        list.push(mine);
    }
    list
}

fn context_menu(ui: &mut egui::Ui, view: &View<'_>, message: &Message, actions: &mut Vec<Action>) {
    let palette = view.palette;
    let chat = &view.chat.id;
    let mine = own_reaction(message);
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 34.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for emoji in quick_reactions(message, view.reaction_emoji) {
                let chosen = mine == Some(emoji);
                let line = widgets::line(ui, emoji, theme::regular(20.0), palette.text, 40.0, 1);
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::click());
                theme::focus_outline(ui, response.id, rect, 17.0);
                if chosen {
                    ui.painter()
                        .circle_filled(rect.center(), 17.0, palette.surface_active);
                    ui.painter().circle_stroke(
                        rect.center(),
                        16.0,
                        Stroke::new(1.5, palette.accent),
                    );
                } else if response.hovered() {
                    ui.painter()
                        .circle_filled(rect.center(), 17.0, palette.surface_hover);
                }
                line.paint(ui, rect.center() - line.size() / 2.0, palette.text);
                let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                let response = if chosen {
                    response.on_hover_text("Remove your reaction")
                } else {
                    response
                };
                if response.clicked() {
                    // Selecting our current reaction removes it.
                    actions.push(Action::React {
                        chat: chat.clone(),
                        message: message.id.clone(),
                        emoji: reaction_choice(mine, emoji),
                    });
                    ui.close();
                }
            }
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::click());
            theme::focus_outline(ui, response.id, rect, 17.0);
            if ui.is_rect_visible(rect) {
                let hovered = response.hovered();
                ui.painter().circle_filled(
                    rect.center(),
                    17.0,
                    if hovered {
                        palette.surface_hover
                    } else {
                        palette.surface
                    },
                );
                ui.painter()
                    .circle_stroke(rect.center(), 16.0, Stroke::new(1.0, palette.outline));
                theme::paint_icon(ui, Icon::Plus, rect, 16.0, palette.secondary);
            }
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("React with any emoji")
                .clicked()
            {
                actions.push(Action::OpenReactionPicker {
                    chat: chat.clone(),
                    message: message.id.clone(),
                    beside_menu: true,
                });
            }
        },
    );
    widgets::menu_separator(ui, &palette);
    if !matches!(message.content, Content::Revoked)
        && widgets::menu_item(ui, &palette, Some(Icon::Reply), "Reply")
    {
        actions.push(Action::Reply(message.id.clone()));
    }
    if !matches!(
        message.content,
        Content::Revoked
            | Content::Unsupported { .. }
            | Content::PhoneOnly { .. }
            | Content::Poll { .. }
            | Content::Interactive { .. }
    ) && widgets::menu_item(ui, &palette, Some(Icon::Forward), "Forward")
    {
        actions.push(Action::ShowDialog(Dialog::Forward {
            chat: chat.clone(),
            messages: vec![message.id.clone()],
        }));
    }
    if widgets::menu_item(ui, &palette, Some(Icon::Check), "Select") {
        actions.push(Action::SelectMessage(message.id.clone()));
    }
    let text = match &message.content {
        Content::Text { text, .. } | Content::Interactive { text, .. } => Some(text.clone()),
        Content::Image { caption, .. }
        | Content::Video { caption, .. }
        | Content::Document { caption, .. } => caption.clone(),
        Content::Location {
            latitude,
            longitude,
            ..
        }
        | Content::LiveLocation {
            latitude,
            longitude,
            ..
        } => Some(format!("{latitude},{longitude}")),
        Content::Contact { vcard, .. } => Some(vcard.clone()),
        _ => None,
    };
    if let Some(text) = text
        && widgets::menu_item(ui, &palette, Some(Icon::Copy), "Copy text")
    {
        let mentions = mentions_of(view, message);
        actions.push(Action::CopyText(markup::plain(&text, &mentions)));
    }
    let age = view.now - message.timestamp;
    let can_edit = message.from_me
        && matches!(message.content, Content::Text { .. })
        && age <= crate::app::EDIT_WINDOW.as_secs() as i64;
    let can_revoke = message.from_me
        && !matches!(message.content, Content::Revoked)
        && age <= crate::app::REVOKE_WINDOW.as_secs() as i64;
    if can_edit && widgets::menu_item(ui, &palette, Some(Icon::Pencil), "Edit") {
        actions.push(Action::Edit(message.id.clone()));
    }
    if can_revoke && widgets::menu_item(ui, &palette, Some(Icon::Trash), "Delete for everyone") {
        actions.push(Action::DeleteForEveryone(message.id.clone()));
    }
    if widgets::menu_item(ui, &palette, Some(Icon::EyeOff), "Delete for me") {
        actions.push(Action::DeleteForMe(message.id.clone()));
    }
    if let Content::Sticker { media, .. } = &message.content
        && let Some(path) = &media.path
        && widgets::menu_item(ui, &palette, Some(Icon::Sticker), "Save sticker")
    {
        actions.push(Action::SaveSticker(path.clone()));
    }
    if let Some(media) = message.content.media() {
        match &media.path {
            Some(path) => {
                let open = if matches!(message.content, Content::Video { gif: false, .. }) {
                    crate::i18n::gettext(view.locale, "Open in system player")
                } else {
                    "Open file".into()
                };
                if widgets::menu_item(ui, &palette, Some(Icon::ExternalLink), &open) {
                    actions.push(Action::OpenFile(path.clone()));
                }
                if widgets::menu_item(ui, &palette, Some(Icon::Download), "Save as…") {
                    actions.push(Action::SaveAttachmentAs {
                        path: path.clone(),
                        name: attachment_name(&message.content, path),
                    });
                }
                if let Some(folder) = path.parent()
                    && widgets::menu_item(ui, &palette, Some(Icon::FileText), "Show in folder")
                {
                    actions.push(Action::OpenFolder(folder.to_path_buf()));
                }
            }
            None => {
                let downloading = matches!(media.state, MediaState::Downloading);
                if widgets::menu_item_enabled(
                    ui,
                    &palette,
                    Some(Icon::Download),
                    if downloading {
                        "Downloading…"
                    } else {
                        "Download"
                    },
                    !downloading,
                ) {
                    actions.push(Action::Download {
                        card: None,
                        chat: chat.clone(),
                        message: message.id.clone(),
                    });
                }
            }
        }
    }
    if let Content::Audio { media, .. } = &message.content
        && media.path.is_some()
    {
        speed_menu_row(ui, view, message, actions);
    }
    widgets::menu_separator(ui, &palette);
    // Sent, delivered, and read times inform only; they are not actions.
    widgets::menu_info(
        ui,
        &palette,
        Icon::Check,
        &format!(
            "Sent {}",
            crate::util::moment_stamp(view.locale, message.timestamp)
        ),
    );
    // Delivery and read times, per member in a group, live in "Message info".
    if message.from_me
        && !matches!(message.content, Content::Revoked)
        && !matches!(
            message.status,
            Delivery::None | Delivery::Pending | Delivery::Failed
        )
        && widgets::menu_item(
            ui,
            &palette,
            Some(Icon::Info),
            &crate::i18n::gettext(view.locale, "Message info"),
        )
    {
        actions.push(Action::ShowDialog(Dialog::MessageInfo {
            chat: chat.clone(),
            message: message.id.clone(),
        }));
    }
    // The id helps when looking a message up for a bug report. Clicking
    // "Sent" used to copy it without saying so.
    if widgets::menu_item(ui, &palette, Some(Icon::Copy), "Copy message ID") {
        actions.push(Action::CopyText(message.id.clone()));
    }
}

fn mentions_of(view: &View<'_>, message: &Message) -> Vec<markup::Mention> {
    message
        .mentions
        .iter()
        .map(|mention| markup::Mention {
            user: mention.user.clone(),
            name: (view.mention_names)(&mention.id),
        })
        .collect()
}

fn quote_mentions(view: &View<'_>, quoted: &crate::model::Quoted) -> Vec<markup::Mention> {
    quoted
        .mentions
        .iter()
        .map(|mention| markup::Mention {
            user: mention.user.clone(),
            name: (view.mention_names)(&mention.id),
        })
        .collect()
}

/// Draws a message body and returns optional footer space on its last line.
fn content(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    width: f32,
    reserve: f32,
    actions: &mut Vec<Action>,
) -> Option<Rect> {
    let palette = view.palette;
    let own = message.from_me;
    // Add non-text messages to cross-message transcript copies.
    let has_body = match &message.content {
        Content::Text { .. } => true,
        // Image-only cards and carousels without a body draw no text.
        Content::Interactive { card, .. } => card.as_ref().is_none_or(|card| {
            !card.body.is_empty() || (card.image.is_none() && card.carousel.is_empty())
        }),
        Content::Image { caption, .. }
        | Content::Video { caption, .. }
        | Content::Document { caption, .. } => caption.is_some(),
        _ => false,
    };
    if !has_body {
        view.copy_rows
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(transcript_row(view, message, String::new(), Vec::new()));
    }
    match &message.content {
        Content::Interactive { text, card } => {
            let Some(card) = card else {
                let span = message.quoted.is_some().then_some(width);
                return rich_body(ui, view, message, text, width, Some(reserve), span, actions);
            };
            if !card.carousel.is_empty() {
                carousel(ui, view, message, card, width, actions);
                return None;
            }
            if let Some(image) = &card.image {
                picture(ui, view, message, image, width, None, actions);
                ui.add_space(4.0);
                if card.body.is_empty() {
                    return None;
                }
            }
            let body = if card.body.is_empty() {
                "Interactive message"
            } else {
                &card.body
            };
            rich_body(
                ui,
                view,
                message,
                body,
                width,
                Some(reserve),
                Some(width),
                actions,
            )
        }
        Content::Text { text, preview } => {
            if let Some(preview) = preview {
                preview_card(ui, view, message, preview, width, actions);
            }
            let span = (message.quoted.is_some() || preview.is_some()).then_some(width);
            rich_body(ui, view, message, text, width, Some(reserve), span, actions)
        }
        Content::Image { caption, media } => {
            let drawn = picture(ui, view, message, media, width, None, actions);
            caption.as_ref().and_then(|caption| {
                // Wrap the caption to the settled image or quote width.
                let wrap = if message.quoted.is_some() {
                    width.max(drawn)
                } else {
                    drawn
                };
                rich_body(
                    ui,
                    view,
                    message,
                    caption,
                    wrap,
                    Some(reserve),
                    Some(wrap),
                    actions,
                )
            })
        }
        Content::Sticker { media, animated } => {
            picture(ui, view, message, media, width, Some(*animated), actions);
            None
        }
        Content::Video {
            caption,
            media,
            seconds,
            gif,
            note,
        } => {
            let drawn = if *note {
                video_note(ui, view, message, media, *seconds, actions)
            } else {
                video(ui, view, message, media, *seconds, *gif, width, actions)
            };
            caption.as_ref().and_then(|caption| {
                let wrap = if message.quoted.is_some() {
                    width.max(drawn)
                } else {
                    drawn
                };
                rich_body(
                    ui,
                    view,
                    message,
                    caption,
                    wrap,
                    Some(reserve),
                    Some(wrap),
                    actions,
                )
            })
        }
        Content::Audio {
            media,
            seconds,
            waveform,
            ..
        } => {
            voice_player(ui, view, message, media, *seconds, waveform, width, actions);
            None
        }
        Content::Document {
            media,
            file_name,
            caption,
            pages,
        } => {
            let mut detail = Vec::new();
            if let Some(pages) = pages {
                detail.push(format!(
                    "{pages} page{}",
                    if *pages == 1 { "" } else { "s" }
                ));
            }
            detail.push(crate::util::bytes(media.size));
            attachment(
                ui,
                view,
                message,
                media,
                Icon::FileText,
                file_name,
                &detail.join(" · "),
                width,
                actions,
            );
            caption.as_ref().and_then(|caption| {
                rich_body(
                    ui,
                    view,
                    message,
                    caption,
                    width,
                    Some(reserve),
                    Some(width),
                    actions,
                )
            })
        }
        Content::Location {
            latitude,
            longitude,
            name,
            address,
        } => {
            let icon = |ui: &mut egui::Ui| {
                theme::icon(ui, Icon::MapPin, 18.0, palette.accent);
            };
            mirrored_row(ui, own, icon, |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    widgets::rich_text(
                        ui,
                        name.as_deref().unwrap_or("Location"),
                        theme::medium(14.0),
                        palette.text,
                    );
                    if let Some(address) = address {
                        widgets::rich_text(ui, address, theme::regular(12.5), palette.secondary);
                    }
                    if theme::link(
                        ui,
                        crate::i18n::gettext(view.locale, "Open in a map").as_ref(),
                        theme::regular(12.5),
                        palette.link,
                    )
                    .clicked()
                    {
                        actions.push(Action::OpenUrl(format!(
                            "https://www.openstreetmap.org/?mlat={latitude}&mlon={longitude}#map=16/{latitude}/{longitude}"
                        )));
                    }
                });
            });
            None
        }
        Content::LiveLocation {
            latitude,
            longitude,
            accuracy_m,
            speed_mps,
            sequence,
            updated,
            ..
        } => {
            let over = message
                .content
                .live_location_over(message.timestamp, view.now);
            let icon = |ui: &mut egui::Ui| {
                theme::icon(ui, Icon::MapPin, 18.0, palette.accent);
            };
            mirrored_row(ui, own, icon, |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    let title = if over {
                        crate::i18n::gettext(view.locale, "Live location ended")
                    } else {
                        crate::i18n::gettext(view.locale, "Live location")
                    };
                    widgets::rich_text(ui, &title, theme::medium(14.0), palette.text);
                    if !over {
                        // An absolute time stays true without repainting.
                        let at = if *updated > 0 {
                            *updated
                        } else {
                            message.timestamp
                        };
                        let mut meta = vec![
                            crate::i18n::gettext(view.locale, "Updated {time}")
                                .replace("{time}", &crate::util::clock(at)),
                        ];
                        if let Some(speed) = speed_mps.filter(|speed| *speed >= 0.5) {
                            meta.push(format!("{:.0} km/h", speed * 3.6));
                        }
                        if let Some(accuracy) = accuracy_m {
                            meta.push(format!("±{accuracy} m"));
                        }
                        widgets::rich_text(
                            ui,
                            &meta.join(" · "),
                            theme::regular(12.5),
                            palette.secondary,
                        );
                    }
                    // The map preview WhatsApp sent, with a pin at its centre.
                    // Each position gets its own image, as a later preview
                    // replaces the first.
                    if let Some(bytes) = message.thumbnail.as_deref()
                        && !bytes.is_empty()
                    {
                        let key = format!("{}-{sequence}-{updated}", message.id);
                        let uri = thumbnail_uri(ui.ctx(), &message.chat, &key, bytes);
                        ui.add_space(4.0);
                        let response = ui.add(
                            egui::Image::new(uri)
                                .fit_to_exact_size(Vec2::new(260.0, 150.0))
                                .corner_radius(8.0),
                        );
                        theme::paint_icon(ui, Icon::MapPin, response.rect, 30.0, palette.accent);
                        ui.add_space(2.0);
                    }
                    if theme::link(
                        ui,
                        crate::i18n::gettext(view.locale, "Open in a map").as_ref(),
                        theme::regular(12.5),
                        palette.link,
                    )
                    .clicked()
                    {
                        actions.push(Action::OpenUrl(format!(
                            "https://www.openstreetmap.org/?mlat={latitude}&mlon={longitude}#map=16/{latitude}/{longitude}"
                        )));
                    }
                });
            });
            None
        }
        Content::Contact {
            display_name,
            vcard,
        } => {
            let icon = |ui: &mut egui::Ui| {
                theme::icon(ui, Icon::Contact, 18.0, palette.accent);
            };
            mirrored_row(ui, own, icon, |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    widgets::rich_text(ui, display_name, theme::medium(14.0), palette.text);
                    let phone = vcard
                        .lines()
                        .find(|line| line.starts_with("TEL"))
                        .and_then(|line| line.rsplit(':').next())
                        .map(str::trim)
                        .unwrap_or("");
                    if !phone.is_empty() {
                        theme::text(ui, phone, theme::regular(12.5), palette.secondary);
                    }
                });
            });
            None
        }
        Content::Poll { .. } => {
            super::polls::ballot(
                ui,
                &palette,
                message,
                width,
                view.connected,
                view.poll_voting
                    .contains(&(message.chat.clone(), message.id.clone())),
                actions,
            );
            None
        }
        Content::Revoked => {
            mirrored_row(
                ui,
                own,
                |ui| {
                    theme::icon(ui, Icon::Ban, 14.0, palette.dim);
                },
                |ui| {
                    theme::text(
                        ui,
                        "This message was deleted",
                        theme::regular(13.5),
                        palette.secondary,
                    );
                },
            );
            None
        }
        Content::PhoneOnly { view_once } => {
            let text = if *view_once {
                "View once message. For your privacy, it opens only on your phone."
            } else {
                "This message can only be seen on your phone."
            };
            mirrored_row(
                ui,
                own,
                |ui| {
                    theme::icon(ui, Icon::Smartphone, 14.0, palette.dim);
                },
                |ui| {
                    theme::text(ui, text, theme::regular(13.5), palette.secondary);
                },
            );
            None
        }
        Content::Unsupported { what } => {
            mirrored_row(
                ui,
                own,
                |ui| {
                    theme::icon(ui, Icon::CircleAlert, 14.0, palette.dim);
                },
                |ui| {
                    theme::text(
                        ui,
                        format!("Unsupported: {what}"),
                        theme::regular(13.5),
                        palette.secondary,
                    );
                },
            );
            None
        }
    }
}

/// Horizontal, independently cached cards with overlaid previous/next controls.
/// Keep a partial next card visible and preserve native wheel/touchpad scrolling.
fn carousel(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    card: &crate::model::InteractiveCard,
    width: f32,
    actions: &mut Vec<Action>,
) {
    if !card.body.is_empty() {
        rich_body(
            ui,
            view,
            message,
            &card.body,
            width,
            None,
            Some(width),
            actions,
        );
        ui.add_space(4.0);
    }
    // Reserve a glimpse of the next card only when there is another card.
    let inset = if card.carousel.len() > 1 { 36.0 } else { 20.0 };
    let card_width = CAROUSEL_CARD_WIDTH.min((width - inset).max(140.0));
    let id = bubble_id(&message.chat, &message.id);
    for direction in [-1, 1] {
        ui.ctx().data_mut(|data| {
            data.remove::<Rect>(id.with(("carousel-arrow", direction)));
        });
    }
    let output = egui::ScrollArea::horizontal()
        .id_salt(("carousel", &message.chat, &message.id))
        .max_width(width)
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .auto_shrink([false, true])
        .show_viewport(ui, |ui, viewport| {
            let strip = ui
                .with_layout(Layout::left_to_right(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.x = CAROUSEL_GAP;
                    for (index, child) in card.carousel.iter().enumerate() {
                        let row = carousel_row(message, index, child);
                        ui.push_id(("carousel-card", index), |ui| {
                            let response = Frame::new()
                                .fill(if message.from_me {
                                    view.palette.bubble_out
                                } else {
                                    view.palette.bubble_in
                                })
                                .corner_radius(10)
                                .inner_margin(Margin {
                                    left: 10,
                                    right: 10,
                                    top: 6,
                                    bottom: 5,
                                })
                                .show(ui, |ui| {
                                    ui.set_width(card_width);
                                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                                        ui.set_width(card_width);
                                        if child.image.is_some() {
                                            carousel_picture(
                                                ui, view, message, index, child, card_width,
                                                actions,
                                            );
                                        }
                                        if !child.body.is_empty() {
                                            rich_body(
                                                ui,
                                                view,
                                                &row,
                                                &child.body,
                                                card_width,
                                                None,
                                                Some(card_width),
                                                actions,
                                            );
                                        }
                                        interactive_buttons(
                                            ui, view, &row, child, card_width, actions,
                                        );
                                    });
                                });
                            ui.ctx().data_mut(|data| {
                                data.insert_temp(
                                    bubble_id(&message.chat, &message.id)
                                        .with(("carousel-card", index)),
                                    response.response.rect,
                                )
                            });
                        });
                    }
                })
                .response
                .rect;
            let limit = (strip.width() - viewport.width()).max(0.0);
            let visible = Rect::from_min_size(
                pos2(strip.left() + viewport.left(), strip.top()),
                vec2(viewport.width(), strip.height()),
            );
            if limit > 1.0 {
                for direction in [-1, 1] {
                    let available = if direction < 0 {
                        viewport.left() > 1.0
                    } else {
                        viewport.left() < limit - 1.0
                    };
                    if !available {
                        continue;
                    }
                    let x = if direction < 0 {
                        visible.left() + 26.0
                    } else {
                        visible.right() - 26.0
                    };
                    let rect =
                        Rect::from_center_size(pos2(x, visible.center().y), Vec2::splat(40.0));
                    let arrow = carousel_arrow(
                        ui,
                        &view.palette,
                        rect,
                        id.with(("carousel-arrow", direction)),
                        direction < 0,
                    );
                    if arrow.clicked() {
                        let step = card_width + 20.0 + CAROUSEL_GAP;
                        let target = (viewport.left() + direction as f32 * step).clamp(0.0, limit);
                        // Issue this inside the horizontal ScrollArea so the
                        // conversation's vertical scroll cannot consume it.
                        ui.scroll_with_delta(vec2(viewport.left() - target, 0.0));
                    }
                }
            }
            visible
        });
    ui.ctx()
        .data_mut(|data| data.insert_temp(id.with("carousel-viewport"), output.inner));
}

/// Stands in for one carousel card when drawing its text and actions. Only the
/// parent's identity is kept: cloning the whole message would copy every
/// card's thumbnail per card each frame, and the parent's quote and reactions
/// would repeat on each card in copied transcripts.
fn carousel_row(message: &Message, index: usize, card: &crate::model::InteractiveCard) -> Message {
    Message {
        id: format!("{}-card-{index}", message.id),
        chat: message.chat.clone(),
        sender: message.sender.clone(),
        sender_name: message.sender_name.clone(),
        from_me: message.from_me,
        timestamp: message.timestamp,
        content: Content::Interactive {
            text: card.body.clone(),
            card: None,
        },
        status: message.status,
        delivered_at: None,
        read_at: None,
        quoted: None,
        reactions: Vec::new(),
        edited: message.edited,
        mentions: message.mentions.clone(),
        forwarded: false,
        thumbnail: None,
    }
}

fn carousel_arrow(
    ui: &mut egui::Ui,
    palette: &Palette,
    rect: Rect,
    id: egui::Id,
    previous: bool,
) -> egui::Response {
    let label = if previous {
        "Previous card"
    } else {
        "Next card"
    };
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    ui.ctx().data_mut(|data| data.insert_temp(id, rect));
    theme::reveal_focus(&response);
    if ui.is_rect_visible(rect) {
        let fill = if response.hovered() || response.has_focus() {
            palette.surface_hover
        } else {
            palette.overlay
        };
        ui.painter()
            .circle_filled(rect.center() + vec2(0.0, 2.0), 21.0, palette.shadow);
        ui.painter().circle_filled(rect.center(), 20.0, fill);
        ui.painter().circle_stroke(
            rect.center(),
            19.5,
            Stroke::new(
                1.0,
                if response.has_focus() {
                    palette.link
                } else {
                    palette.outline
                },
            ),
        );
        theme::paint_icon(
            ui,
            if previous {
                Icon::ChevronLeft
            } else {
                Icon::ChevronRight
            },
            rect,
            if response.is_pointer_button_down_on() {
                21.0
            } else {
                23.0
            },
            palette.text,
        );
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label)
}

/// A carousel uses a short image preview; opening it shows the full picture.
fn carousel_picture(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    index: usize,
    card: &crate::model::InteractiveCard,
    width: f32,
    actions: &mut Vec<Action>,
) {
    let Some(media) = &card.image else { return };
    let size = vec2(width, width * 0.56);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let label = match (&media.path, &media.state) {
        (Some(_), _) => "Open card image",
        (None, MediaState::Failed(_)) => "Retry card image download",
        (None, MediaState::Downloading) => "Downloading card image",
        (None, MediaState::Idle) => "Download card image",
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    // Tab can land on a card scrolled out of the carousel's viewport.
    theme::reveal_focus(&response);
    let visible = ui.is_rect_visible(rect);
    if visible {
        ui.painter().rect_filled(rect, 6.0, view.palette.surface);
        let uri = media
            .path
            .as_ref()
            .map(|path| {
                let uri = crate::util::image_uri(path);
                crate::image_cache::touch(ui.ctx(), &uri);
                uri
            })
            .or_else(|| {
                card.thumbnail.as_deref().map(|bytes| {
                    thumbnail_uri(
                        ui.ctx(),
                        &message.chat,
                        &format!("{}-card-{index}", message.id),
                        bytes,
                    )
                })
            });
        if let Some(uri) = uri {
            let image = egui::Image::new(uri);
            let dimensions = match image.load_for_size(ui.ctx(), size) {
                Ok(egui::load::TexturePoll::Ready { texture }) => Some(texture.size),
                Ok(egui::load::TexturePoll::Pending { .. }) => Some(vec2(
                    media.width.unwrap_or(16) as f32,
                    media.height.unwrap_or(9) as f32,
                )),
                Err(_) => None,
            };
            if let Some(dimensions) = dimensions {
                let ratio = dimensions.x / dimensions.y.max(1.0);
                let target = size.x / size.y.max(1.0);
                let uv_size = if ratio > target {
                    vec2(target / ratio, 1.0)
                } else {
                    vec2(1.0, ratio / target)
                };
                image
                    .uv(Rect::from_center_size(pos2(0.5, 0.5), uv_size))
                    .fit_to_exact_size(size)
                    .corner_radius(6.0)
                    .paint_at(ui, rect);
            } else if media.path.is_some() {
                theme::paint_icon(ui, Icon::CircleAlert, rect, 24.0, view.palette.danger);
                ui.painter().text(
                    rect.center() + vec2(0.0, 24.0),
                    Align2::CENTER_CENTER,
                    "Could not display this picture. Click to open it.",
                    theme::regular(11.5),
                    view.palette.secondary,
                );
            }
        }
        if media.path.is_none() {
            let disc = Rect::from_center_size(rect.center(), Vec2::splat(42.0));
            ui.painter()
                .circle_filled(disc.center(), 21.0, Color32::from_black_alpha(130));
            match &media.state {
                MediaState::Downloading => theme::paint_spinner(ui, disc, 20.0, Color32::WHITE),
                MediaState::Failed(_) => {
                    theme::paint_icon(ui, Icon::CircleAlert, disc, 20.0, Color32::WHITE);
                    ui.painter().text(
                        rect.center() + vec2(0.0, 34.0),
                        Align2::CENTER_CENTER,
                        "Download failed. Click to retry.",
                        theme::regular(11.5),
                        Color32::WHITE,
                    );
                }
                MediaState::Idle => {
                    theme::paint_icon(ui, Icon::Download, disc, 20.0, Color32::WHITE)
                }
            }
        }
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect.shrink(1.0),
                6.0,
                Stroke::new(theme::FOCUS_STROKE_WIDTH, view.palette.link),
                egui::StrokeKind::Inside,
            );
        }
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let clicked = response.clicked();
    if let MediaState::Failed(error) = &media.state {
        response.on_hover_text(format!("{error} · Click to retry"));
    }
    if let Some(path) = &media.path {
        if clicked {
            actions.push(Action::OpenFile(path.clone()));
        }
    } else if !matches!(media.state, MediaState::Downloading)
        && (clicked
            || (visible
                && auto_download_allowed(media, false, view.auto_download)
                && matches!(media.state, MediaState::Idle)))
    {
        actions.push(Action::Download {
            chat: message.chat.clone(),
            message: message.id.clone(),
            card: Some(index),
        });
    }
}

/// Full-width action rows below the message timestamp, matching business cards.
/// Show only executable actions as active, with the same keyboard path as clicks.
fn interactive_buttons(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    card: &crate::model::InteractiveCard,
    width: f32,
    actions: &mut Vec<Action>,
) {
    use crate::model::InteractiveAction;
    let palette = &view.palette;
    let spacing = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 0.0;
    for (index, button) in card.buttons.iter().enumerate() {
        let sends = matches!(
            button.action,
            InteractiveAction::Reply | InteractiveAction::Select(_)
        );
        let enabled = button.url.is_some()
            || match &button.action {
                InteractiveAction::Copy(_) => true,
                InteractiveAction::Reply | InteractiveAction::Select(_) => {
                    view.connected
                        && view.chat.can_send()
                        && !message.from_me
                        && !message.edited
                        && !view
                            .interactive_pending
                            .contains(&(message.chat.clone(), message.id.clone()))
                }
                InteractiveAction::Unavailable => false,
            };
        // Muted/link colours target incoming surfaces; outgoing tinted bubbles
        // need the main foreground to keep small labels readable in both themes.
        let color = if message.from_me {
            palette.text
        } else if enabled {
            palette.link
        } else {
            palette.secondary
        };
        let icon = if button.url.is_some() {
            Some(Icon::ExternalLink)
        } else {
            match button.action {
                InteractiveAction::Reply => None,
                InteractiveAction::Select(_) => Some(Icon::ListChecks),
                InteractiveAction::Copy(_) => Some(Icon::Copy),
                InteractiveAction::Unavailable => Some(Icon::Smartphone),
            }
        };
        let line = widgets::line(
            ui,
            &button.label,
            theme::medium(14.0),
            color,
            (width - 48.0).max(1.0),
            2,
        );
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, _) = ui.allocate_exact_size(
            vec2(width, (line.size().y + 22.0).max(44.0)),
            Sense::hover(),
        );
        // Action rows belong to the card itself, including its side padding.
        // The final row follows the bubble's bottom corners and margin.
        let last = index + 1 == card.buttons.len() && !card.needs_phone;
        let row_rect = Rect::from_min_max(
            rect.min - vec2(10.0, 0.0),
            rect.max + vec2(10.0, if last { 5.0 } else { 0.0 }),
        );
        let corners = CornerRadius {
            sw: if last { 10 } else { 0 },
            se: if last { 10 } else { 0 },
            ..CornerRadius::ZERO
        };
        let response = ui.interact(
            row_rect,
            bubble_id(&message.chat, &message.id).with(("interactive-action", index)),
            sense,
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &button.label)
        });
        theme::reveal_focus(&response);
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                bubble_id(&message.chat, &message.id).with(("interactive-button", index)),
                row_rect,
            )
        });
        if ui.is_rect_visible(rect) {
            if enabled && (response.hovered() || response.has_focus()) {
                ui.painter().rect_filled(
                    row_rect,
                    corners,
                    palette
                        .text
                        .gamma_multiply(if response.is_pointer_button_down_on() {
                            0.08
                        } else {
                            0.04
                        }),
                );
            }
            ui.painter().hline(
                row_rect.x_range(),
                rect.top(),
                Stroke::new(1.0, palette.secondary.gamma_multiply(0.2)),
            );
            if response.has_focus() {
                ui.painter().rect_stroke(
                    rect.shrink(2.0),
                    4.0,
                    Stroke::new(1.0, palette.link),
                    egui::StrokeKind::Inside,
                );
            }
            let inset = if icon.is_some() { 22.0 } else { 0.0 };
            let content_width = line.size().x + inset;
            let left = rect.center().x - content_width / 2.0;
            if let Some(icon) = icon {
                theme::paint_icon(
                    ui,
                    icon,
                    Rect::from_center_size(
                        egui::pos2(left + 7.0, rect.center().y),
                        Vec2::splat(14.0),
                    ),
                    14.0,
                    color,
                );
            }
            line.paint(
                ui,
                egui::pos2(left + inset, rect.center().y - line.size().y / 2.0),
                color,
            );
        }
        let reply = |choice| Action::ReplyInteractive {
            chat: message.chat.clone(),
            message: message.id.clone(),
            button: index,
            choice,
        };
        if !enabled {
            let reason = if sends
                && view
                    .interactive_pending
                    .contains(&(message.chat.clone(), message.id.clone()))
            {
                "Sending reply…"
            } else if sends && !view.connected {
                "Connect to WhatsApp to reply"
            } else if sends && !view.chat.can_send() {
                "This conversation is read-only"
            } else if sends && message.from_me {
                "Reply options are for the recipient"
            } else {
                "Open this option in WhatsApp Web or on your phone"
            };
            response.on_hover_text(format!("{}\n{reason}", button.label));
        } else if let Some(url) = &button.url {
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(format!("{}\n{url}", button.label))
                .clicked()
            {
                actions.push(Action::OpenUrl(url.clone()));
            }
        } else {
            let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
            match &button.action {
                InteractiveAction::Reply => {
                    if response
                        .on_hover_text(format!("Send reply: {}", button.label))
                        .clicked()
                    {
                        actions.push(reply(None));
                    }
                }
                InteractiveAction::Copy(code) => {
                    if response.on_hover_text("Copy code").clicked() {
                        actions.push(Action::CopyText(code.clone()));
                    }
                }
                InteractiveAction::Select(_) => {
                    if response.clicked() {
                        actions.push(Action::ShowDialog(crate::model::Dialog::InteractiveList {
                            chat: message.chat.clone(),
                            message: message.id.clone(),
                            button: index,
                        }));
                    }
                }
                InteractiveAction::Unavailable => {}
            }
        }
    }
    if card.needs_phone {
        ui.add_space(6.0);
        widgets::rich_text(
            ui,
            "More content in WhatsApp Web or on your phone",
            theme::regular(12.0),
            palette.secondary,
        );
    }
    ui.spacing_mut().item_spacing.y = spacing;
}

/// Draws formatted message text. `reserve` leaves footer space on the last
/// line. `span` sets a minimum left-aligned row width for text below cards.
#[allow(clippy::too_many_arguments)]
fn rich_body(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    text: &str,
    width: f32,
    reserve: Option<f32>,
    span: Option<f32>,
    actions: &mut Vec<Action>,
) -> Option<Rect> {
    let palette = view.palette;
    let mentions = mentions_of(view, message);
    let style = markup::Style {
        size: BODY_SIZE,
        color: palette.text,
        secondary: palette.secondary,
        link: palette.link,
        mention: palette.accent,
    };
    let laid = markup::layout(ui, text, &mentions, &style, width);
    let rtl = crate::bidi::message_rtl(text);
    let last_row = laid.galley.rows.last().map_or(0.0, |row| row.row.size.x);
    // Right-aligned text ends at the block's edge. Like official WhatsApp,
    // only a single line keeps the time beside it; otherwise it gets a row.
    let single = laid.galley.rows.len() == 1 && span.is_none();
    let inline = reserve.filter(|reserve| last_row + 8.0 + reserve <= width && (!rtl || single));
    let size = laid.galley.size();
    let mut allocation = match inline {
        Some(reserve) => vec2(size.x.max(last_row + 8.0 + reserve), size.y),
        None => size,
    };
    if let Some(span) = span {
        // Span the card width and keep the text left-aligned in own bubbles.
        allocation.x = allocation.x.max(span);
    }
    // Register the body for transcript formatting when copying across messages.
    view.copy_rows
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .push(transcript_row(
            view,
            message,
            laid.galley.text().to_owned(),
            laid.placements().to_vec(),
        ));
    // Click links and drag to select text.
    // Text selection and pointer links do not need a sequential Tab stop.
    // The surrounding transcript remains available to accessibility readers.
    let (rect, response) = ui.allocate_exact_size(allocation, Sense::CLICK | Sense::DRAG);
    // Store the body rect for selection tests.
    ui.ctx().data_mut(|data| {
        data.insert_temp(bubble_id(&view.chat.id, &message.id).with("body"), rect);
    });
    // Keep off-screen selected bodies registered so scrolling does not lose
    // the selection anchor or omit copied text.
    let visible = ui.is_rect_visible(rect);
    let selection_alive = ui.input(|input| input.pointer.primary_down())
        || ui
            .ctx()
            .plugin_opt::<egui::text_selection::LabelSelectionState>()
            .is_some_and(|plugin| plugin.lock().has_selection());
    let origin = if rtl && inline.is_none() {
        pos2(rect.right() - size.x, rect.top())
    } else {
        rect.min
    };
    if visible || selection_alive {
        markup::paint_selectable(ui, &laid, &response, origin, palette.text, visible);
    }
    if !laid.links.is_empty()
        && let Some(pos) = response.hover_pos()
    {
        let cursor = laid.galley.cursor_from_pos(pos - origin);
        if let Some(url) = laid.link_at(cursor.index.0) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            if response.clicked() {
                actions.push(Action::OpenUrl(url.to_owned()));
            }
        }
    }
    inline.map(|reserve| {
        Rect::from_min_max(
            pos2(rect.right() - reserve, rect.bottom() - 15.0),
            rect.right_bottom(),
        )
    })
}

/// Link preview with image, title, and description.
fn preview_card(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    preview: &LinkPreview,
    width: f32,
    actions: &mut Vec<Action>,
) {
    let palette = view.palette;
    let thumbnail = message.thumbnail.as_deref();
    let domain = preview
        .url
        .split("://")
        .nth(1)
        .unwrap_or(&preview.url)
        .split('/')
        .next()
        .unwrap_or_default()
        .to_owned();
    let response = Frame::new()
        .fill(palette.window.gamma_multiply(0.35))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::same(8))
        .show(ui, |ui| {
            // Include margins in the settled card width.
            let card_width = (width - 16.0).max(0.0);
            ui.set_width(card_width);
            // Limit text to the space beside the thumbnail and keep it
            // left-aligned in own bubbles.
            let column = (card_width - if thumbnail.is_some() { 72.0 } else { 0.0 }).max(0.0);
            ui.allocate_ui_with_layout(vec2(card_width, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(card_width);
                ui.horizontal(|ui| {
                    if let Some(bytes) = thumbnail {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
                        // Off-screen cards are laid out too; only a visible
                        // one keeps its thumbnail resident.
                        if ui.is_rect_visible(rect) {
                            let uri = thumbnail_uri(ui.ctx(), &message.chat, &message.id, bytes);
                            egui::Image::new(uri)
                                .fit_to_exact_size(rect.size())
                                .corner_radius(4.0)
                                .paint_at(ui, rect);
                        }
                    }
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.set_width(column);
                        if let Some(title) = &preview.title {
                            widgets::rich_text(ui, title, theme::semibold(13.5), palette.text);
                        }
                        if let Some(description) = &preview.description {
                            let line = widgets::line(
                                ui,
                                description,
                                theme::regular(12.5),
                                palette.secondary,
                                ui.available_width(),
                                2,
                            );
                            let (rect, _) = ui.allocate_exact_size(line.size(), Sense::hover());
                            line.paint(ui, rect.min, palette.secondary);
                        }
                        theme::text(ui, &domain, theme::regular(12.0), palette.dim);
                    });
                });
            });
        })
        .response;
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            bubble_id(&view.chat.id, &message.id).with("preview"),
            response.rect,
        );
    });
    let response = ui
        .interact(
            response.rect,
            ui.id().with(("preview", &message.id)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        actions.push(Action::OpenUrl(preview.url.clone()));
    }
}

fn thumbnail_uri(ctx: &egui::Context, chat: &str, id: &str, bytes: &[u8]) -> String {
    let uri = format!(
        "bytes://thumb-{}-{}",
        chat.chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>(),
        id
    );
    crate::image_cache::include(ctx, uri.clone(), bytes);
    uri
}

/// Default image bounds based on [`CARD_WIDTH`].
const PICTURE_WIDTH: f32 = CARD_WIDTH;
const PICTURE_HEIGHT: f32 = 440.0;
const STICKER_SIDE: f32 = 180.0;
/// Width of an image plus bubble padding.
const HEADER_ROW: f32 = 44.0;

/// Fits an image within bounds without upscaling and with a readable minimum.
fn fit_picture(width: f32, height: f32, max_width: f32, max_height: f32) -> Vec2 {
    let (width, height) = if width > 0.0 && height > 0.0 {
        (width, height)
    } else {
        (4.0, 3.0)
    };
    let max_width = max_width.max(0.0);
    let max_height = max_height.max(0.0);
    let scale = (max_width / width).min(max_height / height).clamp(0.0, 1.0);
    let scale = if width * scale < 120.0 {
        (120.0 / width).min(max_width / width).max(0.0)
    } else {
        scale
    };
    vec2(
        (width * scale).max(0.0),
        (height * scale).max(90.0).max(0.0),
    )
}

/// Fits a sticker to the standard square size.
fn fit_sticker(width: f32, height: f32) -> Vec2 {
    let (width, height) = if width > 0.0 && height > 0.0 {
        (width, height)
    } else {
        (1.0, 1.0)
    };
    let scale = (STICKER_SIDE / width).min(STICKER_SIDE / height);
    vec2(width * scale, height * scale)
}

/// Reserved size for an image or video before and after download.
fn frame_size(
    media: &Media,
    thumbnail_hint: Option<(u32, u32)>,
    max_width: f32,
    max_height: f32,
) -> Vec2 {
    let (w, h) = match (media.width, media.height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => (w as f32, h as f32),
        _ => match thumbnail_hint {
            Some((w, h)) if w > 0 && h > 0 => (w as f32, h as f32),
            _ => (4.0, 3.0),
        },
    };
    fit_picture(w, h, max_width, max_height)
}

/// Draws an image or sticker, using its preview until downloaded. Returns its width.
fn picture(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    media: &Media,
    width: f32,
    sticker: Option<bool>,
    actions: &mut Vec<Action>,
) -> f32 {
    let palette = view.palette;
    let (max_width, max_height) = match sticker {
        Some(_) => (STICKER_SIDE, STICKER_SIDE),
        None => (width.min(PICTURE_WIDTH), PICTURE_HEIGHT),
    };
    if let Some(path) = &media.path {
        if sticker == Some(true) {
            let size = fit_sticker(
                media.width.unwrap_or(180) as f32,
                media.height.unwrap_or(180) as f32,
            );
            let (rect, response) = ui.allocate_exact_size(size, Sense::click());
            if ui.is_rect_visible(rect) {
                match animation::frame(ui, path, rect, view.animate && response.hovered()) {
                    animation::Frame::Ready(texture) => {
                        ui.painter().image(
                            texture.id(),
                            rect,
                            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    _ => {
                        ui.painter().rect_filled(rect, 6.0, palette.surface);
                        theme::paint_icon(ui, Icon::Sticker, rect, 32.0, palette.secondary);
                    }
                }
            }
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                actions.push(Action::OpenFile(path.clone()));
            }
            return size.x;
        }
        // A row that is off screen only reserves its space. Loading the image
        // decodes it and uploads a texture, so it waits until it is scrolled
        // into view, and `image_cache` can release it once it leaves again.
        let reserved = frame_size(media, None, max_width, max_height);
        let position = ui.next_widget_position();
        if !ui.is_rect_visible(Rect::from_min_size(position, reserved)) {
            ui.allocate_exact_size(reserved, Sense::hover());
            return reserved.x;
        }
        let image = widgets::file_image(ui, path);
        return match image.load_for_size(ui.ctx(), vec2(max_width, max_height)) {
            Ok(egui::load::TexturePoll::Ready { texture }) => {
                let size = if sticker.is_some() {
                    fit_sticker(texture.size.x, texture.size.y)
                } else {
                    fit_picture(texture.size.x, texture.size.y, max_width, max_height)
                };
                let response = ui.add(
                    image
                        .fit_to_exact_size(size)
                        .corner_radius(if sticker.is_some() { 0.0 } else { 6.0 })
                        .sense(Sense::click()),
                );
                if response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    let action = match crate::image_preview::open_target(path, sticker.is_none()) {
                        crate::image_preview::OpenTarget::Preview => {
                            Action::PreviewImage(path.clone())
                        }
                        crate::image_preview::OpenTarget::External => {
                            Action::OpenFile(path.clone())
                        }
                    };
                    actions.push(action);
                }
                size.x
            }
            Ok(egui::load::TexturePoll::Pending { .. }) => {
                let size = if sticker.is_some() {
                    Vec2::splat(STICKER_SIDE)
                } else {
                    frame_size(media, None, max_width, max_height)
                };
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                if ui.is_rect_visible(rect) {
                    ui.painter().rect_filled(rect, 6.0, palette.surface);
                    theme::paint_spinner(ui, rect, 22.0, palette.accent);
                }
                size.x
            }
            Err(_) => {
                let size = if sticker.is_some() {
                    Vec2::splat(STICKER_SIDE)
                } else {
                    frame_size(media, None, max_width, max_height)
                };
                let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                if ui.is_rect_visible(rect) {
                    ui.painter().rect_filled(rect, 6.0, palette.surface);
                    theme::paint_icon(ui, Icon::CircleAlert, rect, 24.0, palette.danger);
                    ui.painter().text(
                        rect.center() + vec2(0.0, 24.0),
                        Align2::CENTER_CENTER,
                        "Could not display this picture. Click to open it.",
                        theme::regular(11.5),
                        palette.secondary,
                    );
                }
                if response.clicked() {
                    actions.push(Action::OpenFile(path.clone()));
                }
                size.x
            }
        };
    }
    let size = frame_size(media, None, max_width, max_height);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let thumbnail = message
            .thumbnail
            .as_deref()
            .filter(|_| sticker.is_none())
            .map(|bytes| thumbnail_uri(ui.ctx(), &message.chat, &message.id, bytes));
        match thumbnail {
            Some(uri) => {
                egui::Image::new(uri)
                    .fit_to_exact_size(size)
                    .corner_radius(6.0)
                    .paint_at(ui, rect);
                ui.painter()
                    .rect_filled(rect, 6.0, Color32::from_black_alpha(60));
            }
            None => {
                ui.painter().rect_filled(rect, 6.0, palette.surface);
            }
        }
        let disc = Rect::from_center_size(rect.center(), Vec2::splat(44.0));
        match &media.state {
            MediaState::Downloading => {
                ui.painter()
                    .circle_filled(disc.center(), 22.0, Color32::from_black_alpha(120));
                theme::paint_spinner(ui, disc, 22.0, Color32::WHITE);
            }
            MediaState::Failed(_) => {
                ui.painter()
                    .circle_filled(disc.center(), 22.0, Color32::from_black_alpha(120));
                theme::paint_icon(ui, Icon::CircleAlert, disc, 22.0, palette.danger);
                ui.painter().text(
                    rect.center() + vec2(0.0, 34.0),
                    Align2::CENTER_CENTER,
                    "Download failed. Click to retry.",
                    theme::regular(11.5),
                    Color32::WHITE,
                );
            }
            MediaState::Idle => {
                ui.painter()
                    .circle_filled(disc.center(), 22.0, Color32::from_black_alpha(120));
                theme::paint_icon(
                    ui,
                    if sticker.is_some() {
                        Icon::Sticker
                    } else {
                        Icon::Download
                    },
                    disc,
                    22.0,
                    Color32::WHITE,
                );
                if sticker.is_none() {
                    ui.painter().text(
                        rect.center() + vec2(0.0, 34.0),
                        Align2::CENTER_CENTER,
                        crate::util::bytes(media.size),
                        theme::regular(11.5),
                        Color32::WHITE,
                    );
                }
            }
        }
    }
    let wants = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
        && !matches!(media.state, MediaState::Downloading);
    let auto = ui.is_rect_visible(rect)
        && matches!(media.state, MediaState::Idle)
        && auto_download_allowed(media, sticker.is_some(), view.auto_download);
    if wants || auto {
        actions.push(Action::Download {
            card: None,
            chat: view.chat.id.clone(),
            message: message.id.clone(),
        });
    }
    size.x
}

/// Stickers always download when visible, while other media follows the setting.
/// Every automatic download still respects the shared size limit.
fn auto_download_allowed(media: &Media, sticker: bool, auto_download: bool) -> bool {
    media.is_within_download_limit() && (sticker || auto_download)
}

/// Draws a video. GIFs play in place; other videos play in the bubble once
/// clicked, downloading first when needed, with controls along the bottom.
#[allow(clippy::too_many_arguments)]
fn video(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    media: &Media,
    seconds: Option<u32>,
    gif: bool,
    width: f32,
    actions: &mut Vec<Action>,
) -> f32 {
    use crate::video::State;
    let palette = view.palette;
    let Some(thumbnail) = message.thumbnail.as_deref() else {
        let title = if gif { "GIF" } else { "Video" };
        let mut detail = Vec::new();
        if let Some(seconds) = seconds {
            detail.push(crate::util::duration(seconds));
        }
        detail.push(crate::util::bytes(media.size));
        attachment(
            ui,
            view,
            message,
            media,
            Icon::Video,
            title,
            &detail.join(" · "),
            width,
            actions,
        );
        return width;
    };
    let limit = width.min(PICTURE_WIDTH);
    let size = frame_size(media, Some((16, 9)), limit, PICTURE_HEIGHT.min(limit * 1.3));
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let playing = match (&media.path, gif) {
        (Some(path), true) => Some(animation::frame(
            ui,
            path,
            rect,
            view.animate && response.hovered(),
        )),
        _ => None,
    };
    if let Some(animation::Frame::Ready(texture)) = &playing {
        if ui.is_rect_visible(rect) {
            ui.painter().image(
                texture.id(),
                rect,
                Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
            && let Some(path) = &media.path
        {
            actions.push(Action::OpenFile(path.clone()));
        }
        return size.x;
    }
    let status = media
        .path
        .as_ref()
        .filter(|_| !gif)
        .and_then(|_| view.video.status(&message.id));
    if ui.is_rect_visible(rect) {
        if status.is_some() {
            view.video.saw(&message.id);
        }
        match status.as_ref().and_then(|status| status.frame.as_ref()) {
            Some(frame) => {
                ui.painter().rect_filled(rect, 6.0, Color32::BLACK);
                paint_texture(
                    ui,
                    fit_within(frame.size_vec2(), rect),
                    frame.id(),
                    Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    6.0,
                );
            }
            None => {
                // Registering the poster decodes it, so it waits for the row to show.
                let uri = thumbnail_uri(ui.ctx(), &message.chat, &message.id, thumbnail);
                egui::Image::new(uri)
                    .fit_to_exact_size(size)
                    .corner_radius(6.0)
                    .paint_at(ui, rect);
            }
        }
        let state = status.as_ref().map(|status| status.state);
        if state != Some(State::Playing) {
            ui.painter()
                .rect_filled(rect, 6.0, Color32::from_black_alpha(40));
            let disc = Rect::from_center_size(rect.center(), Vec2::splat(48.0));
            ui.painter()
                .circle_filled(disc.center(), 24.0, Color32::from_black_alpha(140));
            match (&media.path, &media.state) {
                (Some(_), _)
                    if state == Some(State::Loading)
                        || matches!(playing, Some(animation::Frame::Pending)) =>
                {
                    theme::paint_spinner(ui, disc, 24.0, Color32::WHITE)
                }
                (Some(_), _) if gif => {
                    theme::paint_icon(ui, Icon::ExternalLink, disc, 22.0, Color32::WHITE)
                }
                (None, MediaState::Downloading) => {
                    theme::paint_spinner(ui, disc, 24.0, Color32::WHITE)
                }
                (None, MediaState::Failed(_)) => {
                    theme::paint_icon(ui, Icon::CircleAlert, disc, 22.0, palette.danger)
                }
                (Some(_), _) | (None, MediaState::Idle) => {
                    theme::paint_icon(ui, Icon::Play, disc, 22.0, Color32::WHITE)
                }
            }
        }
        match (&status, &media.path) {
            (Some(status), Some(path)) => {
                // Controls show while paused and while the pointer is over
                // the video, as in other players.
                if status.state == State::Paused
                    || (status.state == State::Playing && ui.rect_contains_pointer(rect))
                {
                    video_controls(ui, view, message, path, rect, status, actions);
                }
            }
            _ => {
                let mut label = Vec::new();
                if gif {
                    label.push("GIF".to_owned());
                }
                if let Some(seconds) = seconds {
                    label.push(crate::util::duration(seconds));
                }
                if media.path.is_none() {
                    label.push(crate::util::bytes(media.size));
                }
                if !label.is_empty() {
                    let galley = ui.painter().layout_no_wrap(
                        label.join(" · "),
                        theme::medium(11.5),
                        Color32::WHITE,
                    );
                    let chip = Rect::from_min_size(
                        pos2(rect.left() + 8.0, rect.bottom() - galley.size().y - 14.0),
                        galley.size() + vec2(12.0, 6.0),
                    );
                    ui.painter().rect_filled(
                        chip,
                        chip.height() / 2.0,
                        Color32::from_black_alpha(140),
                    );
                    ui.painter()
                        .galley(chip.min + vec2(6.0, 3.0), galley, Color32::WHITE);
                }
            }
        }
    }
    let auto = ui.is_rect_visible(rect)
        && media.path.is_none()
        && matches!(media.state, MediaState::Idle)
        && view.auto_download
        && media.is_within_download_limit();
    if auto {
        actions.push(Action::Download {
            card: None,
            chat: view.chat.id.clone(),
            message: message.id.clone(),
        });
    }
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        video_clicked(view, message, media, gif, auto, actions);
    }
    size.x
}

/// What a click on a video does: a downloaded video plays or pauses, a GIF
/// that cannot play here opens in the system viewer, and a video still on
/// WhatsApp's servers downloads and then plays. `downloading` is set when
/// this frame already asked for the download.
fn video_clicked(
    view: &View<'_>,
    message: &Message,
    media: &Media,
    gif: bool,
    downloading: bool,
    actions: &mut Vec<Action>,
) {
    match &media.path {
        Some(path) if gif => actions.push(Action::OpenFile(path.clone())),
        Some(path) => actions.push(Action::PlayVideo {
            message: message.id.clone(),
            path: path.clone(),
        }),
        None => {
            if !downloading && !matches!(media.state, MediaState::Downloading) {
                actions.push(Action::Download {
                    card: None,
                    chat: view.chat.id.clone(),
                    message: message.id.clone(),
                });
            }
            if !gif {
                actions.push(Action::PlayVideoWhenDownloaded(message.id.clone()));
            }
        }
    }
}

/// Play/pause, the time, a seek bar, and a sound switch along the bottom of
/// a playing video.
fn video_controls(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    path: &Path,
    rect: Rect,
    status: &crate::video::Status,
    actions: &mut Vec<Action>,
) {
    let bar = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 32.0), rect.max);
    ui.painter().rect_filled(
        bar,
        CornerRadius {
            nw: 0,
            ne: 0,
            sw: 6,
            se: 6,
        },
        Color32::from_black_alpha(150),
    );
    let id = ui.id().with(("video-controls", &message.id));
    let toggle = Rect::from_center_size(pos2(bar.left() + 18.0, bar.center().y), Vec2::splat(26.0));
    let playing = status.state == crate::video::State::Playing;
    theme::paint_icon(
        ui,
        if playing { Icon::Pause } else { Icon::Play },
        toggle,
        16.0,
        Color32::WHITE,
    );
    let tooltip = if playing {
        crate::i18n::gettext(view.locale, "Pause")
    } else {
        crate::i18n::gettext(view.locale, "Play")
    };
    if ui
        .interact(toggle, id.with("toggle"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tooltip.as_ref())
        .clicked()
    {
        actions.push(Action::PlayVideo {
            message: message.id.clone(),
            path: path.to_owned(),
        });
    }
    let sound = Rect::from_center_size(pos2(bar.right() - 18.0, bar.center().y), Vec2::splat(26.0));
    let muted = view.video.muted();
    theme::paint_icon(
        ui,
        if muted { Icon::VolumeX } else { Icon::Volume2 },
        sound,
        16.0,
        Color32::WHITE,
    );
    let tooltip = if muted {
        crate::i18n::gettext(view.locale, "Unmute")
    } else {
        crate::i18n::gettext(view.locale, "Mute")
    };
    if ui
        .interact(sound, id.with("sound"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tooltip.as_ref())
        .clicked()
    {
        actions.push(Action::ToggleVideoSound);
    }
    let time = format!(
        "{} / {}",
        crate::util::duration(status.position.as_secs() as u32),
        crate::util::duration(status.total.as_secs() as u32)
    );
    let galley = ui
        .painter()
        .layout_no_wrap(time, theme::medium(11.5), Color32::WHITE);
    let text = pos2(toggle.right() + 4.0, bar.center().y - galley.size().y / 2.0);
    let track = Rect::from_min_max(
        pos2(text.x + galley.size().x + 10.0, bar.center().y - 8.0),
        pos2(sound.left() - 6.0, bar.center().y + 8.0),
    );
    ui.painter().galley(text, galley, Color32::WHITE);
    if track.width() < 12.0 {
        return;
    }
    let response = ui
        .interact(track, id.with("seek"), Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let pointed = response
        .interact_pointer_pos()
        .map(|pointer| ((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0));
    // Seeking restarts the decoder, so a drag seeks once, where it ends.
    let fraction = match pointed {
        Some(fraction) if response.dragged() => fraction,
        _ => status.fraction(),
    };
    let line = Rect::from_center_size(track.center(), vec2(track.width(), 3.0));
    ui.painter()
        .rect_filled(line, 1.5, Color32::from_white_alpha(90));
    let played = pos2(line.left() + fraction * line.width(), line.center().y);
    ui.painter().rect_filled(
        Rect::from_min_max(line.min, pos2(played.x, line.bottom())),
        1.5,
        view.palette.accent,
    );
    ui.painter().circle_filled(played, 5.0, view.palette.accent);
    if (response.clicked() || response.drag_stopped())
        && let Some(fraction) = pointed
    {
        actions.push(Action::SeekVideo {
            message: message.id.clone(),
            fraction,
        });
    }
}

/// Side of a round video message.
const NOTE_SIDE: f32 = 220.0;

/// Draws a round video message (PTV) as a circle that plays in place when
/// clicked, with a ring for the progress. Returns its width.
fn video_note(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    media: &Media,
    seconds: Option<u32>,
    actions: &mut Vec<Action>,
) -> f32 {
    use crate::video::State;
    let palette = view.palette;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(NOTE_SIDE), Sense::click());
    let status = media
        .path
        .as_ref()
        .and_then(|_| view.video.status(&message.id));
    if ui.is_rect_visible(rect) {
        if status.is_some() {
            view.video.saw(&message.id);
        }
        let center = rect.center();
        // Leave room around the picture for the progress ring.
        let picture = rect.shrink(5.0);
        let frame = status
            .as_ref()
            .and_then(|status| status.frame.as_ref())
            .map(|frame| (frame.id(), frame.size_vec2()));
        let poster = || {
            let bytes = message.thumbnail.as_deref()?;
            let uri = thumbnail_uri(ui.ctx(), &message.chat, &message.id, bytes);
            match egui::Image::new(uri).load_for_size(ui.ctx(), picture.size()) {
                Ok(egui::load::TexturePoll::Ready { texture }) => Some((texture.id, texture.size)),
                _ => None,
            }
        };
        match frame.or_else(poster) {
            Some((texture, size)) => paint_texture(
                ui,
                picture,
                texture,
                crate::video::square_uv(size.x, size.y),
                picture.width() / 2.0,
            ),
            None => {
                ui.painter()
                    .circle_filled(center, picture.width() / 2.0, palette.surface);
            }
        }
        let state = status.as_ref().map(|status| status.state);
        let ring = NOTE_SIDE / 2.0 - 2.0;
        if let Some(status) = &status {
            ui.painter().circle_stroke(
                center,
                ring,
                Stroke::new(3.0, palette.secondary.gamma_multiply(0.35)),
            );
            ui.painter().add(egui::Shape::line(
                crate::video::arc(center, ring, status.fraction()),
                Stroke::new(3.0, palette.accent),
            ));
        }
        let hovered = ui.rect_contains_pointer(rect);
        let disc = Rect::from_center_size(center, Vec2::splat(48.0));
        let waiting = matches!(
            (&media.path, &media.state, state),
            (Some(_), _, Some(State::Loading)) | (None, MediaState::Downloading, _)
        );
        let icon = match (&media.path, &media.state, state) {
            _ if waiting => None,
            // Only a pause sign under the pointer covers a playing video.
            (Some(_), _, Some(State::Playing)) => hovered.then_some(Icon::Pause),
            (None, MediaState::Failed(_), _) => Some(Icon::CircleAlert),
            _ => Some(Icon::Play),
        };
        if state != Some(State::Playing) {
            ui.painter().circle_filled(
                center,
                picture.width() / 2.0,
                Color32::from_black_alpha(40),
            );
        }
        if waiting || icon.is_some() {
            ui.painter()
                .circle_filled(center, 24.0, Color32::from_black_alpha(140));
        }
        if waiting {
            theme::paint_spinner(ui, disc, 24.0, Color32::WHITE);
        } else if let Some(icon) = icon {
            let color = if icon == Icon::CircleAlert {
                palette.danger
            } else {
                Color32::WHITE
            };
            theme::paint_icon(ui, icon, disc, 22.0, color);
        }
        // The time while it plays, otherwise its length.
        let label = match &status {
            Some(status) => Some(crate::util::duration(status.position.as_secs() as u32)),
            None => seconds
                .map(crate::util::duration)
                .or_else(|| media.path.is_none().then(|| crate::util::bytes(media.size))),
        };
        if let Some(label) = label {
            let galley = ui
                .painter()
                .layout_no_wrap(label, theme::medium(11.5), Color32::WHITE);
            let chip = Rect::from_center_size(
                pos2(center.x, picture.bottom() - galley.size().y / 2.0 - 16.0),
                galley.size() + vec2(12.0, 6.0),
            );
            ui.painter()
                .rect_filled(chip, chip.height() / 2.0, Color32::from_black_alpha(140));
            ui.painter()
                .galley(chip.min + vec2(6.0, 3.0), galley, Color32::WHITE);
        }
    }
    let auto = ui.is_rect_visible(rect)
        && media.path.is_none()
        && matches!(media.state, MediaState::Idle)
        && view.auto_download
        && media.is_within_download_limit();
    if auto {
        actions.push(Action::Download {
            card: None,
            chat: view.chat.id.clone(),
            message: message.id.clone(),
        });
    }
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        video_clicked(view, message, media, false, auto, actions);
    }
    NOTE_SIDE
}

/// Paints a texture into `rect` with rounded corners; a radius of half the
/// side makes a circle.
fn paint_texture(ui: &egui::Ui, rect: Rect, texture: egui::TextureId, uv: Rect, radius: f32) {
    ui.painter().add(
        egui::epaint::RectShape::filled(
            rect,
            CornerRadius::same(radius.round().clamp(0.0, 255.0) as u8),
            Color32::WHITE,
        )
        .with_texture(texture, uv),
    );
}

/// The largest rect with the proportions of `size` centred in `rect`.
fn fit_within(size: Vec2, rect: Rect) -> Rect {
    if size.x <= 0.0 || size.y <= 0.0 {
        return rect;
    }
    let scale = (rect.width() / size.x).min(rect.height() / size.y);
    Rect::from_center_size(rect.center(), size * scale)
}

#[allow(clippy::too_many_arguments)]
fn attachment(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    media: &Media,
    icon: Icon,
    title: &str,
    detail: &str,
    width: f32,
    actions: &mut Vec<Action>,
) {
    let palette = view.palette;
    let response = Frame::new()
        .fill(palette.window.gamma_multiply(0.35))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            // Include margins in the settled row width.
            let card = (width - 20.0).max(0.0);
            ui.set_width(card);

            let disc = |ui: &mut egui::Ui| {
                let (disc, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
                ui.painter().circle_filled(
                    disc.center(),
                    18.0,
                    palette.accent.gamma_multiply(0.25),
                );
                theme::paint_icon(ui, icon, disc, 18.0, palette.accent);
            };
            let action = |ui: &mut egui::Ui| match (&media.path, &media.state) {
                (Some(_), _) => {
                    theme::icon(ui, Icon::ExternalLink, 18.0, palette.secondary);
                }
                (None, MediaState::Downloading) => {
                    theme::spinner(ui, 18.0, palette.accent);
                }
                (None, _) => {
                    theme::icon(ui, Icon::Download, 18.0, palette.secondary);
                }
            };
            let column = |ui: &mut egui::Ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    // Reserve 70 points for the icon, action, and gaps.
                    ui.set_width((card - 70.0).max(0.0));
                    widgets::rich_text(ui, title, theme::medium(14.0), palette.text);
                    let detail = match &media.state {
                        MediaState::Failed(error) => format!("{error}. Click to retry."),
                        _ => detail.to_owned(),
                    };
                    theme::text(ui, detail, theme::regular(12.0), palette.secondary);
                });
            };
            // Fix the row left-to-right at the card width in own bubbles.
            ui.allocate_ui_with_layout(
                vec2(card, 52.0),
                Layout::left_to_right(egui::Align::Center),
                |ui| {
                    disc(ui);
                    column(ui);
                    action(ui);
                },
            );
        })
        .response;
    let auto = ui.is_rect_visible(response.rect)
        && media.path.is_none()
        && matches!(media.state, MediaState::Idle)
        && view.auto_download
        && media.is_within_download_limit();
    if auto {
        actions.push(Action::Download {
            card: None,
            chat: view.chat.id.clone(),
            message: message.id.clone(),
        });
    }
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            bubble_id(&view.chat.id, &message.id).with("card"),
            response.rect,
        );
    });
    let response = ui
        .interact(
            response.rect,
            ui.id().with(("attachment", &message.id)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() && !auto {
        match &media.path {
            Some(path) => actions.push(Action::OpenFile(path.clone())),
            None if !matches!(media.state, MediaState::Downloading) => {
                actions.push(Action::Download {
                    card: None,
                    chat: view.chat.id.clone(),
                    message: message.id.clone(),
                })
            }
            None => {}
        }
    }
}

/// In-chat voice and audio player.
#[allow(clippy::too_many_arguments)]
fn voice_player(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    media: &Media,
    seconds: Option<u32>,
    waveform: &[u8],
    width: f32,
    actions: &mut Vec<Action>,
) {
    use crate::audio::State;
    let palette = view.palette;
    let status = view.player.status(&message.id);
    let button = 36.0;
    let bar_height = 30.0;
    let chip = 44.0;
    // The chip appears with the playable clip; the waveform takes its space
    // back while the audio is still downloading.
    let shows_chip = media.path.is_some();
    let wave_width = (width - button - 10.0 - if shows_chip { chip + 10.0 } else { 0.0 }).max(0.0);
    let bars: Vec<u8> = if !waveform.is_empty() {
        waveform.to_vec()
    } else if let Some(bars) = view.player.bars(&message.id) {
        bars.to_vec()
    } else {
        vec![12; crate::voice::BARS]
    };
    let fill = palette.accent.gamma_multiply(0.22);
    let hover = palette.accent.gamma_multiply(0.38);
    let waiting = |ui: &mut egui::Ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(button), Sense::hover());
        ui.painter()
            .circle_filled(rect.center(), button / 2.0, fill);
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.centered_and_justified(|ui| {
                theme::spinner(ui, 18.0, palette.accent);
            });
        });
    };
    // Force left-to-right layout at the player's width inside own bubbles.
    ui.allocate_ui_with_layout(
        vec2(width.max(0.0), button),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            match (&media.path, &media.state) {
                (None, MediaState::Downloading) => waiting(ui),
                (None, _) => {
                    if theme::circle_button(
                        ui,
                        Icon::Download,
                        button,
                        fill,
                        hover,
                        palette.accent,
                        "Download",
                    )
                    .clicked()
                    {
                        actions.push(Action::Download {
                            card: None,
                            chat: view.chat.id.clone(),
                            message: message.id.clone(),
                        });
                    }
                }
                (Some(path), _) => match status.state {
                    State::Loading => waiting(ui),
                    State::Playing | State::Paused | State::Idle => {
                        let (icon, tooltip) = if status.state == State::Playing {
                            (Icon::Pause, "Pause")
                        } else {
                            (Icon::Play, "Play")
                        };
                        if theme::circle_button(
                            ui,
                            icon,
                            button,
                            fill,
                            hover,
                            palette.accent,
                            tooltip,
                        )
                        .clicked()
                        {
                            actions.push(Action::PlayVoice {
                                message: message.id.clone(),
                                path: path.clone(),
                            });
                        }
                    }
                },
            }
            let mut wave_middle = None;
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let (rect, response) =
                    ui.allocate_exact_size(vec2(wave_width, bar_height), Sense::click());
                wave_middle = Some(rect.center().y);
                let pitch = 3.0;
                let count = (rect.width() / pitch).floor() as usize;
                let fraction = if status.total > Duration::ZERO {
                    status.position.as_secs_f32() / status.total.as_secs_f32()
                } else {
                    0.0
                };
                let played_until = rect.left() + fraction * rect.width();
                let quiet = palette.secondary.gamma_multiply(0.7);
                if count > 0 {
                    for index in 0..count {
                        let level = f32::from(bars[index * bars.len() / count]) / 100.0;
                        let height = (2.0 + level * (bar_height - 4.0)).max(2.0);
                        let x = rect.left() + index as f32 * pitch + 1.0;
                        let colour = if status.state != State::Idle && x <= played_until {
                            palette.accent
                        } else {
                            quiet
                        };
                        ui.painter().rect_filled(
                            Rect::from_center_size(
                                egui::pos2(x, rect.center().y),
                                vec2(2.0, height),
                            ),
                            1.0,
                            colour,
                        );
                    }
                }
                if matches!(status.state, State::Playing | State::Paused) && rect.width() >= 10.0 {
                    let knob = played_until.clamp(rect.left() + 5.0, rect.right() - 5.0);
                    ui.painter().circle_filled(
                        egui::pos2(knob, rect.center().y),
                        5.0,
                        palette.accent,
                    );
                }
                if let Some(path) = &media.path {
                    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                    if response.clicked()
                        && let Some(pointer) = response.interact_pointer_pos()
                    {
                        let fraction = if rect.width() > 0.0 {
                            ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        actions.push(Action::SeekVoice {
                            message: message.id.clone(),
                            path: path.clone(),
                            fraction,
                        });
                    }
                }
                // Show playback position while active, otherwise total duration.
                let shown = match status.state {
                    State::Playing | State::Paused => {
                        crate::util::duration(status.position.as_secs() as u32)
                    }
                    _ => seconds
                        .or_else(|| {
                            (status.total > Duration::ZERO).then_some(status.total.as_secs() as u32)
                        })
                        .map(crate::util::duration)
                        .unwrap_or_else(|| crate::util::bytes(media.size)),
                };
                let text = match &media.state {
                    MediaState::Failed(error) => format!("{error}. Click to retry."),
                    _ => shown,
                };
                theme::text(ui, text, theme::regular(11.5), palette.secondary);
            });
            // Speed chip, cycling 1x, 1.5x, and 2x like the phone. The
            // message menu lists every speed, including 1.25x and 1.75x.
            if shows_chip {
                let speed = view.player.speed();
                // The label follows the click at once, faded until this
                // clip actually plays at that speed.
                let preparing = view.player.preparing_speed(&message.id);
                // Reserve the chip's place in the row, then draw it level
                // with the middle of the waveform rather than the whole row.
                let size = vec2(chip, 20.0);
                let (slot, _) = ui.allocate_exact_size(size, Sense::hover());
                let at = Rect::from_center_size(
                    egui::pos2(slot.center().x, wave_middle.unwrap_or(slot.center().y)),
                    size,
                );
                let response = ui
                    .scope_builder(egui::UiBuilder::new().max_rect(at), |ui| {
                        speed_pill(ui, view, size, speed, speed > 1.0, preparing)
                    })
                    .inner;
                ui.ctx().data_mut(|data| {
                    data.insert_temp(speed_chip_id(&view.chat.id, &message.id), response.rect);
                });
                if response.clicked() {
                    actions.push(Action::SetVoiceSpeed(crate::audio::next_cycled_speed(
                        speed,
                    )));
                }
                response.on_hover_text(if preparing {
                    "Preparing playback speed"
                } else {
                    "Playback speed. Right-click for every speed."
                });
            }
        },
    );
    let auto = media.path.is_none()
        && matches!(media.state, MediaState::Idle)
        && view.auto_download
        && media.is_within_download_limit();
    if auto {
        actions.push(Action::Download {
            card: None,
            chat: view.chat.id.clone(),
            message: message.id.clone(),
        });
    }
}

/// Voice-recording controls and live waveform.
fn recording_strip(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let (elapsed, levels) = match app.recording.as_ref() {
        Some(recorder) => (recorder.elapsed(), recorder.levels()),
        None => return,
    };
    let button = 36.0;
    ui.allocate_ui_with_layout(
        vec2(ui.available_width().max(0.0), button),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if theme::circle_button(
                ui,
                Icon::Trash,
                button,
                palette.surface,
                palette.surface_hover,
                palette.secondary,
                "Discard",
            )
            .clicked()
            {
                app.actions.push(Action::CancelRecording);
            }
            // Pulsing recording light and elapsed time.
            let (dot, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
            let pulse = 0.55 + 0.45 * (elapsed.as_secs_f32() * 3.0).sin().abs();
            ui.painter()
                .circle_filled(dot.center(), 5.0, palette.danger.gamma_multiply(pulse));
            theme::text(
                ui,
                crate::util::duration(elapsed.as_secs() as u32),
                theme::medium(14.0),
                palette.text,
            );
            // Recent audio levels, newest on the right.
            let wave_width = (ui.available_width() - button - 10.0).max(40.0);
            let (rect, _) = ui.allocate_exact_size(vec2(wave_width, 28.0), Sense::hover());
            let pitch = 3.0;
            let count = (rect.width() / pitch).floor() as usize;
            let start = levels.len().saturating_sub(count);
            for (index, level) in levels[start..].iter().enumerate() {
                let height = 2.0_f32 + (level * 4.0).min(1.0) * 24.0;
                let x = rect.left() + index as f32 * pitch + 1.0;
                ui.painter().rect_filled(
                    Rect::from_center_size(egui::pos2(x, rect.center().y), vec2(2.0, height)),
                    1.0,
                    palette.accent,
                );
            }
            if theme::circle_button(
                ui,
                Icon::Send,
                button,
                palette.accent,
                palette.accent_hover,
                palette.on_accent,
                "Send",
            )
            .clicked()
            {
                app.actions.push(Action::SendRecording);
            }
        },
    );
}

/// Replaces the composer while messages are selected.
fn selection_bar(app: &mut App, ui: &mut egui::Ui, chat: &str, selected: &[String]) {
    let palette = app.palette;
    ui.horizontal(|ui| {
        if theme::icon_button(
            ui,
            Icon::X,
            18.0,
            palette.secondary,
            palette.text,
            "Cancel selection",
        )
        .clicked()
            || ui.input(|input| input.key_pressed(Key::Escape))
        {
            app.actions.push(Action::CancelSelection);
        }
        let count = if selected.len() == 1 {
            "1 selected".to_owned()
        } else {
            format!("{} selected", selected.len())
        };
        theme::text(ui, &count, theme::medium(14.5), palette.text);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if theme::pill_button(ui, &palette, "Forward…", true).clicked() {
                app.actions.push(Action::ShowDialog(Dialog::Forward {
                    chat: chat.to_owned(),
                    messages: selected.to_vec(),
                }));
            }
        });
    });
}

/// The file name to suggest when saving an attachment: the sender's name for
/// documents, the cached file's name otherwise. Path separators are dropped so
/// a crafted name cannot point the dialog somewhere else.
fn attachment_name(content: &Content, path: &Path) -> String {
    let name = match content {
        Content::Document { file_name, .. } if !file_name.trim().is_empty() => file_name.clone(),
        _ => path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "attachment".to_owned()),
    };
    name.replace(['/', '\\'], "_")
}

/// Whether a conversation has visible content. Used by tests.
#[allow(dead_code)]
pub fn has_messages(conversation: &Conversation) -> bool {
    !conversation.messages.is_empty()
}

#[allow(dead_code)]
fn status_label(status: Delivery) -> &'static str {
    match status {
        Delivery::None => "",
        Delivery::Pending => "sending",
        Delivery::Sent => "sent",
        Delivery::Delivered => "delivered",
        Delivery::Read => "read",
        Delivery::Played => "played",
        Delivery::Failed => "failed",
    }
}

#[allow(dead_code)]
fn chat_of(chat: &ChatId) -> &str {
    chat
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_attachments_suggest_a_plain_file_name() {
        let document = |file_name: &str| Content::Document {
            media: media(None, None),
            file_name: file_name.into(),
            caption: None,
            pages: None,
        };
        let cached = Path::new("/cache/media/abc123.pdf");
        assert_eq!(attachment_name(&document("Notes.pdf"), cached), "Notes.pdf");
        assert_eq!(
            attachment_name(&document("../../.bashrc"), cached),
            ".._.._.bashrc"
        );
        assert_eq!(attachment_name(&document("  "), cached), "abc123.pdf");
    }

    fn media(w: Option<u32>, h: Option<u32>) -> Media {
        Media {
            mime: "image/jpeg".into(),
            size: 1,
            width: w,
            height: h,
            path: None,
            state: MediaState::Idle,
        }
    }

    #[test]
    fn picture_placeholder_matches_decoded_dimensions() {
        for limit in [200.0, PICTURE_WIDTH] {
            for (width, height) in [(900, 1200), (600, 1600), (1600, 900)] {
                assert_eq!(
                    frame_size(
                        &media(Some(width), Some(height)),
                        None,
                        limit,
                        PICTURE_HEIGHT
                    ),
                    fit_picture(width as f32, height as f32, limit, PICTURE_HEIGHT),
                    "decoding a {width}x{height} photo must not shift the transcript"
                );
            }
        }
    }

    #[test]
    fn reaction_affordance_action_targets_the_message_it_belongs_to() {
        let action = open_reaction_picker_action("chat@example", "message-42");
        assert!(matches!(
            action,
            Action::OpenReactionPicker { chat, message, beside_menu: false }
                if chat == "chat@example" && message == "message-42"
        ));
    }

    #[test]
    fn reaction_button_label_names_whose_message_it_reacts_to() {
        assert_eq!(reaction_button_label(true, "Ada"), "React to your message");
        assert_eq!(
            reaction_button_label(false, "Ada"),
            "React to Ada's message"
        );
    }

    #[test]
    fn reaction_affordance_sits_beside_the_bubble_without_clipping() {
        let bounds = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 300.0));
        let incoming = Rect::from_min_max(pos2(20.0, 20.0), pos2(140.0, 80.0));
        let outgoing = Rect::from_min_max(pos2(260.0, 20.0), pos2(380.0, 80.0));

        let incoming_button = reaction_affordance_rect(incoming, bounds, false);
        let outgoing_button = reaction_affordance_rect(outgoing, bounds, true);

        assert!(incoming_button.left() > incoming.right());
        assert!(outgoing_button.right() < outgoing.left());
        assert!(bounds.contains_rect(incoming_button));
        assert!(bounds.contains_rect(outgoing_button));

        let edge = Rect::from_min_max(pos2(360.0, 260.0), pos2(398.0, 298.0));
        assert!(bounds.contains_rect(reaction_affordance_rect(edge, bounds, false)));
    }

    #[test]
    fn reaction_affordance_stays_visible_while_crossing_to_its_button() {
        let bubble = Rect::from_min_max(pos2(20.0, 20.0), pos2(140.0, 80.0));
        let button = Rect::from_min_max(pos2(146.0, 24.0), pos2(172.0, 50.0));

        assert!(reaction_affordance_visible(
            Some(bubble.center()),
            bubble,
            button
        ));
        assert!(reaction_affordance_visible(
            Some(button.center()),
            bubble,
            button
        ));
        assert!(!reaction_affordance_visible(
            Some(pos2(300.0, 200.0)),
            bubble,
            button
        ));
        assert!(!reaction_affordance_visible(None, bubble, button));
    }

    #[test]
    fn picture_frames_keep_their_shape_within_the_limit() {
        let landscape = frame_size(&media(Some(1600), Some(1200)), None, 340.0, PICTURE_HEIGHT);
        assert!((landscape.x - 340.0).abs() < 0.01);
        assert!((landscape.y - 255.0).abs() < 0.01);
        let tall = frame_size(&media(Some(600), Some(1200)), None, 340.0, PICTURE_HEIGHT);
        assert!(tall.y > 340.0 && tall.y <= PICTURE_HEIGHT);
        let exact = fit_picture(900.0, 1600.0, PICTURE_WIDTH, PICTURE_HEIGHT);
        assert!((exact.y - PICTURE_HEIGHT).abs() < 0.01);
        assert!(exact.x < PICTURE_WIDTH);
        let unknown = frame_size(&media(None, None), Some((16, 9)), 340.0, PICTURE_HEIGHT);
        assert!(unknown.x > unknown.y);
        let tiny = frame_size(&media(Some(40), Some(40)), None, 340.0, PICTURE_HEIGHT);
        assert!(tiny.x >= 120.0);
    }

    #[test]
    fn a_failed_footer_reserves_room_for_its_label() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut message = Message {
            id: "fixture".into(),
            chat: "1@s.whatsapp.net".into(),
            sender: "me@s.whatsapp.net".into(),
            sender_name: None,
            from_me: true,
            timestamp: 1000,
            content: Content::text("Fixture"),
            status: Delivery::Sent,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: Vec::new(),
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        };
        let mut widths = Vec::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            widths.push(footer_width(ui, &message));
            message.status = Delivery::Failed;
            widths.push(footer_width(ui, &message));
            // Only our own messages can fail to send.
            message.from_me = false;
            widths.push(footer_width(ui, &message) + 19.0);
        });
        output.textures_delta.clear();
        assert!(widths[1] > widths[0] + 30.0, "{widths:?}");
        assert_eq!(widths[2], widths[0]);
    }

    #[test]
    fn visible_stickers_download_automatically_with_the_attachment_setting_off() {
        let mut sticker = media(Some(180), Some(180));
        assert!(auto_download_allowed(&sticker, true, false));
        assert!(!auto_download_allowed(&sticker, false, false));
        sticker.size = crate::model::ATTACHMENT_DOWNLOAD_LIMIT + 1;
        assert!(!auto_download_allowed(&sticker, true, false));
    }

    #[test]
    fn composer_triggers_only_start_at_word_boundaries() {
        assert_eq!(standalone_trigger(":", 1, ':'), Some(0));
        assert_eq!(standalone_trigger("hello :", 7, ':'), Some(6));
        assert_eq!(standalone_trigger("hello @", 7, '@'), Some(6));
        assert_eq!(standalone_trigger("19:30", 3, ':'), None);
        assert_eq!(standalone_trigger("mail@example.com", 5, '@'), None);
    }

    #[test]
    fn mention_query_runs_from_the_at_to_the_cursor() {
        assert_eq!(active_mention("hello @mi", Some(6), 9), Some((9, "mi")));
        assert_eq!(active_mention("hello @mi\n", Some(6), 10), None);
        assert_eq!(active_mention("hello @mi", Some(99), 9), None);
    }

    #[test]
    fn emoji_query_ends_at_spaces_and_punctuation() {
        assert_eq!(active_emoji("hello :gri", Some(6), 10), Some((10, "gri")));
        assert_eq!(
            active_emoji("hello :gri_ning", Some(6), 15),
            Some((15, "gri_ning"))
        );
        assert_eq!(active_emoji("hello :gri ", Some(6), 11), None);
        assert_eq!(active_emoji("hello :gri!", Some(6), 11), None);
        assert_eq!(active_emoji("hello gri", Some(6), 9), None);
    }

    #[test]
    fn emoji_shortcodes_rank_ahead_of_name_matches() {
        let grinning = emojis::get("😀").expect("known emoji");
        assert_eq!(emoji_match_score(grinning, "grinning"), Some(0));
        assert_eq!(emoji_match_score(grinning, "grin"), Some(1));
        assert_eq!(emoji_match_score(grinning, "face"), Some(3));
        assert_eq!(emoji_match_score(grinning, "rocket"), None);
    }

    #[test]
    fn completion_offers_skin_tone_capable_emoji() {
        let directory = tempfile::tempdir().unwrap();
        let (app, _events) = App::headless(
            crate::paths::AppDirs::under(directory.path()),
            crate::settings::Settings::default(),
        );
        let offers = |query: &str, emoji: &str| {
            emoji_candidates(&app, query)
                .iter()
                .any(|suggestion| suggestion.emoji == emoji)
        };
        assert!(offers("pregnant", "🤰"));
        assert!(offers("thumbsup", "👍"));
    }
}

#[cfg(test)]
mod reaction_tests {
    use super::*;
    use crate::model::{Delivery, Reaction};

    fn with_reactions(reactions: Vec<Reaction>) -> Message {
        Message {
            id: "m1".into(),
            chat: "a@s.whatsapp.net".into(),
            sender: "a@s.whatsapp.net".into(),
            sender_name: None,
            from_me: false,
            timestamp: 0,
            content: Content::text("hi"),
            status: Delivery::None,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions,
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        }
    }

    fn reaction(from_me: bool, emoji: &str) -> Reaction {
        Reaction {
            sender: if from_me { "me" } else { "them" }.into(),
            from_me,
            emoji: emoji.into(),
        }
    }

    #[test]
    fn only_our_own_reaction_counts_as_chosen() {
        let message = with_reactions(vec![reaction(false, "😂"), reaction(true, "❤️")]);
        assert_eq!(own_reaction(&message), Some("❤️"));
        assert_eq!(quick_reactions(&message, &[]), QUICK_REACTIONS.to_vec());
        assert_eq!(
            own_reaction(&with_reactions(vec![reaction(false, "😂")])),
            None
        );
    }

    #[test]
    fn quick_reactions_start_with_preferences_and_fill_with_unique_defaults() {
        let message = with_reactions(Vec::new());
        let preferred = vec![("🦀".into(), 5), ("👍".into(), 2), ("invalid".into(), 1)];
        let quick = quick_reactions(&message, &preferred);
        assert_eq!(&quick[..2], &["🦀", "👍"]);
        assert_eq!(quick.len(), 6);
        assert_eq!(quick.iter().filter(|&&emoji| emoji == "👍").count(), 1);
    }

    #[test]
    fn an_unusual_reaction_of_ours_joins_the_quick_row() {
        let message = with_reactions(vec![reaction(true, "🦀")]);
        let quick = quick_reactions(&message, &[]);
        assert_eq!(quick.len(), QUICK_REACTIONS.len() + 1);
        assert_eq!(quick.last(), Some(&"🦀"));
    }

    #[test]
    fn reaction_choice_toggles_or_replaces_our_emoji() {
        assert_eq!(reaction_choice(None, "🦀"), "🦀");
        assert_eq!(reaction_choice(Some("🦀"), "🦀"), "");
        assert_eq!(reaction_choice(Some("👍"), "🦀"), "🦀");
        assert_eq!(reaction_choice(Some("❤️"), "❤️"), "");
    }

    #[test]
    fn another_senders_custom_reaction_is_kept_on_the_message() {
        let message = with_reactions(vec![reaction(false, "🏆")]);
        assert_eq!(own_reaction(&message), None);
        assert_eq!(message.reactions[0].emoji, "🏆");
        assert!(!message.reactions[0].from_me);
        assert_eq!(quick_reactions(&message, &[]), QUICK_REACTIONS.to_vec());
    }
}

/// Pending attachment tiles above the composer.
fn pending_strip(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let tile = 72.0;
    let mut remove = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        for (index, item) in app.pending.iter_mut().enumerate() {
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(tile), Sense::hover());
            if ui.is_rect_visible(rect) {
                ui.painter().rect_filled(rect, 8.0, palette.surface);
                match item {
                    crate::app::Pending::Picture {
                        width,
                        height,
                        rgba,
                        texture,
                    } => {
                        let handle = texture.get_or_insert_with(|| {
                            // Limit thumbnails to the GPU's maximum texture size.
                            let image = if *width > 1024 || *height > 1024 {
                                let scale = 1024.0 / (*width).max(*height) as f32;
                                let (w, h) = (
                                    ((*width as f32 * scale) as u32).max(1),
                                    ((*height as f32 * scale) as u32).max(1),
                                );
                                match image::RgbaImage::from_raw(
                                    *width as u32,
                                    *height as u32,
                                    rgba.to_vec(),
                                ) {
                                    Some(full) => {
                                        let small = image::imageops::resize(
                                            &full,
                                            w,
                                            h,
                                            image::imageops::FilterType::Triangle,
                                        );
                                        egui::ColorImage::from_rgba_unmultiplied(
                                            [w as usize, h as usize],
                                            &small,
                                        )
                                    }
                                    None => egui::ColorImage::example(),
                                }
                            } else {
                                egui::ColorImage::from_rgba_unmultiplied([*width, *height], rgba)
                            };
                            ui.ctx().load_texture(
                                format!("pending-picture-{index}"),
                                image,
                                egui::TextureOptions::LINEAR,
                            )
                        });
                        // Preserve aspect ratio while filling the tile.
                        let side = tile - 8.0;
                        let scale =
                            (side / (*width).max(1) as f32).min(side / (*height).max(1) as f32);
                        let fitted = vec2(*width as f32 * scale, *height as f32 * scale);
                        let inner = Rect::from_center_size(rect.center(), fitted);
                        egui::Image::from_texture((handle.id(), fitted))
                            .corner_radius(6.0)
                            .paint_at(ui, inner);
                    }
                    crate::app::Pending::File(path) => {
                        if crate::app::Pending::is_picture_file(path) {
                            widgets::file_image(ui, path)
                                .fit_to_exact_size(Vec2::splat(tile - 8.0))
                                .corner_radius(6.0)
                                .paint_at(ui, rect.shrink(4.0));
                        } else {
                            let icon = Rect::from_center_size(
                                rect.center() - vec2(0.0, 10.0),
                                Vec2::splat(24.0),
                            );
                            theme::paint_icon(ui, Icon::FileText, icon, 22.0, palette.secondary);
                            let name = path
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            let line = widgets::line(
                                ui,
                                &name,
                                theme::regular(10.5),
                                palette.text,
                                tile - 8.0,
                                1,
                            );
                            line.paint(
                                ui,
                                egui::pos2(
                                    rect.center().x - line.size().x / 2.0,
                                    rect.bottom() - 18.0,
                                ),
                                palette.text,
                            );
                        }
                    }
                }
                // Remove button in the corner.
                let close =
                    Rect::from_center_size(rect.right_top() + vec2(-10.0, 10.0), Vec2::splat(18.0));
                let close_response =
                    ui.interact(close, ui.id().with(("unstage", index)), Sense::click());
                ui.painter()
                    .circle_filled(close.center(), 9.0, palette.overlay);
                theme::paint_icon(ui, Icon::X, close, 12.0, palette.text);
                if close_response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    remove = Some(index);
                }
            }
            let _ = response;
        }
    });
    if let Some(index) = remove {
        app.actions.push(Action::RemovePending(index));
    }
    ui.add_space(4.0);
}
