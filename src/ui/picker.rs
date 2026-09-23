//! The picker above the composer: emoji, GIFs, and stickers.
//! Also the full emoji picker used to react to a message.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use egui::{
    Align, Align2, CornerRadius, Frame, Key, Layout, Margin, Modifiers, Rect, Sense, Stroke, Vec2,
    pos2, vec2,
};

use crate::app::App;
use crate::i18n::gettext;
use crate::model::{Action, PickerTab, StickerPack, StickerShelf};
use crate::theme::{self, Icon, Palette};

use super::conversation;
use super::widgets;

const WIDTH: f32 = 420.0;
const HEIGHT: f32 = 400.0;
/// Frame inner margin on each side. Placement uses the outer size.
const FRAME_MARGIN: i8 = 10;
/// Minimum emoji cell width. Columns expand to fill the grid.
const CELL: f32 = 40.0;

/// An emoji-grid heading or row.
enum Row {
    Header(&'static str),
    Emoji {
        first: usize,
        values: Vec<&'static str>,
    },
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    if app.reaction_target.is_some() {
        reaction_picker(app, ctx);
        return;
    }
    let Some(tab) = app.picker else {
        return;
    };
    let palette = app.palette;
    let Some(anchor) = app.picker_anchor else {
        return;
    };
    let screen = ctx.content_rect();
    let x = anchor
        .left()
        .clamp(screen.left() + 8.0, (screen.right() - WIDTH - 8.0).max(8.0));
    let y = (anchor.top() - HEIGHT - 10.0).max(screen.top() + 8.0);
    let area = egui::Area::new(egui::Id::new("picker"))
        .fixed_pos(pos2(x, y))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            Frame::new()
                .fill(palette.overlay)
                .stroke(Stroke::new(1.0, palette.outline))
                .corner_radius(CornerRadius::same(theme::RADIUS + 4))
                .inner_margin(Margin::same(10))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 8],
                    blur: 28,
                    spread: 0,
                    color: palette.shadow,
                })
                .show(ui, |ui| {
                    ui.set_width(WIDTH);
                    ui.set_height(HEIGHT);
                    ui.spacing_mut().item_spacing.y = 6.0;
                    let body_height = HEIGHT - 44.0;
                    ui.allocate_ui_with_layout(
                        vec2(WIDTH, body_height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| match tab {
                            PickerTab::Emoji => emoji_tab(app, ui, &palette),
                            PickerTab::Gifs => gif_tab(app, ui, &palette),
                            PickerTab::Stickers => sticker_tab(app, ui, &palette),
                        },
                    );
                    // Keep the tabs at the bottom when content is short.
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                        tabs(app, ui, &palette, tab);
                    });
                });
        });
    // Close on outside clicks, except on the toggle button or a sticker menu.
    let rect = area.response.rect;
    let clicked_outside = !egui::Popup::is_any_open(ctx)
        && ctx.input(|input| {
            input.pointer.any_pressed()
                && input
                    .pointer
                    .interact_pos()
                    .is_some_and(|pos| !rect.contains(pos) && !anchor.contains(pos))
        });
    if clicked_outside {
        app.actions.push(Action::ClosePicker);
    }
}

fn tabs(app: &mut App, ui: &mut egui::Ui, palette: &Palette, current: PickerTab) {
    ui.horizontal(|ui| {
        let entries = [
            (PickerTab::Emoji, Icon::Smile, "Emoji"),
            (PickerTab::Gifs, Icon::Gif, "GIF"),
            (PickerTab::Stickers, Icon::Sticker, "Stickers"),
        ];
        let spacing = ui.spacing().item_spacing.x;
        let total = entries
            .iter()
            .map(|(_, _, label)| theme::soft_button_width(ui, label, true))
            .sum::<f32>()
            + spacing * (entries.len() as f32 - 1.0);
        ui.add_space((ui.available_width() - total).max(0.0) / 2.0);
        for (tab, icon, label) in entries {
            if theme::soft_button(ui, palette, Some(icon), label, tab == current).clicked()
                && tab != current
            {
                app.actions.push(Action::TogglePicker(tab));
            }
        }
    });
}

fn search_box(
    ui: &mut egui::Ui,
    palette: &Palette,
    id: &str,
    text: &mut String,
    hint: &str,
) -> egui::Response {
    let width = ui.available_width();
    widgets::search_field(ui, palette, egui::Id::new(id), text, hint, width)
}

// --- emoji ---------------------------------------------------------------

fn group_name(group: emojis::Group) -> &'static str {
    match group {
        emojis::Group::SmileysAndEmotion => "Smileys & Emotion",
        emojis::Group::PeopleAndBody => "People & Body",
        emojis::Group::AnimalsAndNature => "Animals & Nature",
        emojis::Group::FoodAndDrink => "Food & Drink",
        emojis::Group::TravelAndPlaces => "Travel & Places",
        emojis::Group::Activities => "Activities",
        emojis::Group::Objects => "Objects",
        emojis::Group::Symbols => "Symbols",
        emojis::Group::Flags => "Flags",
    }
}

fn usable_recent(recent: &[String]) -> Vec<&'static str> {
    recent
        .iter()
        .filter_map(|emoji| emojis::get(emoji).map(|emoji| emoji.as_str()))
        .collect()
}

fn rows_for(
    query: &str,
    recent: &[String],
    columns: usize,
    recent_label: &'static str,
) -> Vec<Row> {
    let query = query.trim().to_lowercase();
    let mut rows = Vec::new();
    let mut next = 0;
    let mut chunk = |rows: &mut Vec<Row>, list: Vec<&'static str>| {
        for part in list.chunks(columns) {
            let first = next;
            next += part.len();
            rows.push(Row::Emoji {
                first,
                values: part.to_vec(),
            });
        }
    };
    if !query.is_empty() {
        let found: Vec<&'static str> = emojis::iter()
            .filter(|emoji| {
                emoji.name().to_lowercase().contains(&query)
                    || emoji
                        .shortcodes()
                        .any(|code| code.to_lowercase().contains(&query))
            })
            .map(|emoji| emoji.as_str())
            .collect();
        if found.is_empty() {
            rows.push(Row::Header("Nothing matches"));
        } else {
            chunk(&mut rows, found);
        }
        return rows;
    }
    let recent = usable_recent(recent);
    if !recent.is_empty() {
        rows.push(Row::Header(recent_label));
        chunk(&mut rows, recent);
    }
    for group in emojis::Group::iter() {
        rows.push(Row::Header(group_name(group)));
        chunk(
            &mut rows,
            group.emojis().map(|emoji| emoji.as_str()).collect(),
        );
    }
    rows
}

fn place_picker(screen: Rect, anchor: Option<Rect>, width: f32, height: f32) -> egui::Pos2 {
    let max_x = (screen.right() - width - 8.0).max(screen.left() + 8.0);
    let x = match anchor {
        Some(anchor) => anchor.left().clamp(screen.left() + 8.0, max_x),
        None => (screen.center().x - width / 2.0).clamp(screen.left() + 8.0, max_x),
    };
    let y = match anchor {
        Some(anchor) => {
            let above = anchor.top() - height - 10.0;
            if above >= screen.top() + 8.0 {
                above
            } else {
                (anchor.bottom() + 10.0)
                    .min((screen.bottom() - height - 8.0).max(screen.top() + 8.0))
            }
        }
        None => (screen.center().y - height / 2.0).max(screen.top() + 8.0),
    };
    pos2(x, y)
}

/// Category tabs under an emoji grid, WhatsApp-style. The first entry stands
/// for the grid's own recent section, whatever it is called there.
const CATEGORIES: &[(Option<emojis::Group>, &str, &str)] = &[
    (None, "🕒", "Frequently Used"),
    (
        Some(emojis::Group::SmileysAndEmotion),
        "😀",
        "Smileys & Emotion",
    ),
    (Some(emojis::Group::PeopleAndBody), "👋", "People & Body"),
    (
        Some(emojis::Group::AnimalsAndNature),
        "🐻",
        "Animals & Nature",
    ),
    (Some(emojis::Group::FoodAndDrink), "🍔", "Food & Drink"),
    (
        Some(emojis::Group::TravelAndPlaces),
        "🚗",
        "Travel & Places",
    ),
    (Some(emojis::Group::Activities), "⚽", "Activities"),
    (Some(emojis::Group::Objects), "💡", "Objects"),
    (Some(emojis::Group::Symbols), "🔣", "Symbols"),
    (Some(emojis::Group::Flags), "🏁", "Flags"),
];

fn category_entries(
    has_recent: bool,
    recent_label: &'static str,
) -> impl Iterator<Item = (Option<emojis::Group>, &'static str, &'static str)> {
    CATEGORIES
        .iter()
        .copied()
        .filter(move |(group, _, _)| group.is_some() || has_recent)
        .map(move |(group, glyph, label)| {
            (
                group,
                glyph,
                if group.is_none() { recent_label } else { label },
            )
        })
}

fn header_row(rows: &[Row], label: &str) -> Option<usize> {
    rows.iter().position(|row| match row {
        Row::Header(found) => *found == label,
        Row::Emoji { .. } => false,
    })
}

/// Scroll target for a category tab. A stale Recent / Frequently Used jump
/// falls back to the first Unicode group when that header is absent.
fn resolve_jump(jump: Option<&'static str>, rows: &[Row]) -> Option<&'static str> {
    let label = jump?;
    if header_row(rows, label).is_some() {
        return Some(label);
    }
    emojis::Group::iter()
        .map(group_name)
        .find(|name| header_row(rows, name).is_some())
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

fn move_emoji_selection(selected: usize, count: usize, columns: usize, key: Key) -> usize {
    if count == 0 {
        return 0;
    }
    let selected = selected.min(count - 1);
    match key {
        Key::ArrowLeft => selected.saturating_sub(1),
        Key::ArrowRight => (selected + 1).min(count - 1),
        Key::ArrowUp if selected >= columns => selected - columns,
        Key::ArrowDown if selected + columns < count => selected + columns,
        _ => selected,
    }
}

fn emoji_tab(app: &mut App, ui: &mut egui::Ui, palette: &Palette) {
    let width = ui.available_width();
    let grid_height = (ui.available_height() - 40.0).max(0.0);
    ui.allocate_ui_with_layout(
        vec2(width, grid_height),
        Layout::top_down(Align::Min),
        |ui| {
            if let Some(emoji) =
                emoji_grid(app, ui, palette, "emoji-search", "emoji-grid", "Recent")
            {
                app.actions.push(Action::InsertEmoji(emoji));
            }
        },
    );
    let has_recent = app
        .settings
        .recent_emoji
        .iter()
        .any(|emoji| emojis::get(emoji).is_some());
    category_tabs(app, ui, palette, "emoji-grid", "Recent", has_recent);
}

fn reaction_picker(app: &mut App, ctx: &egui::Context) {
    let Some((chat, message)) = app.reaction_target.clone() else {
        return;
    };
    let palette = app.palette;
    let screen = ctx.content_rect();
    let menu = if app.reaction_beside_menu {
        ctx.data(|data| {
            data.get_temp::<Rect>(conversation::bubble_id(&chat, &message).with("menu-rect"))
        })
        .or(app.reaction_anchor)
    } else {
        app.reaction_anchor
    };
    let width = menu.map_or(WIDTH, |menu| {
        (screen.right() - menu.right() - 32.0).clamp(260.0, WIDTH)
    });
    let outer_width = width + f32::from(FRAME_MARGIN) * 2.0;
    let outer_height = HEIGHT + f32::from(FRAME_MARGIN) * 2.0;
    let pos = if let Some(menu) = menu
        && menu.right() + outer_width + 16.0 <= screen.right()
    {
        pos2(
            menu.right() + 8.0,
            menu.top()
                .min((screen.bottom() - outer_height - 8.0).max(screen.top() + 8.0)),
        )
    } else {
        place_picker(screen, menu, outer_width, outer_height)
    };
    let preview = app
        .conversations
        .get(&chat)
        .and_then(|conversation| conversation.message(&message))
        .map(|message| (app.display_name(&message.sender), message.content.summary()));
    let area = egui::Area::new(egui::Id::new("reaction-picker"))
        .fixed_pos(pos)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            // Picker chrome stays LTR even in RTL chats, matching WhatsApp.
            ui.allocate_ui_with_layout(
                vec2(outer_width, outer_height),
                Layout::top_down(Align::Min),
                |ui| {
                    Frame::new()
                        .fill(palette.overlay)
                        .stroke(Stroke::new(1.0, palette.outline))
                        .corner_radius(CornerRadius::same(theme::RADIUS + 4))
                        .inner_margin(Margin::same(FRAME_MARGIN))
                        .shadow(egui::epaint::Shadow {
                            offset: [0, 8],
                            blur: 28,
                            spread: 0,
                            color: palette.shadow,
                        })
                        .show(ui, |ui| {
                            ui.set_width(width);
                            ui.set_height(HEIGHT);
                            ui.spacing_mut().item_spacing.y = 6.0;
                            ui.horizontal(|ui| {
                                theme::text(
                                    ui,
                                    "React to message",
                                    theme::semibold(13.0),
                                    palette.text,
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if theme::icon_button(
                                        ui,
                                        Icon::X,
                                        14.0,
                                        palette.secondary,
                                        palette.text,
                                        "Close reactions",
                                    )
                                    .clicked()
                                    {
                                        app.actions.push(Action::ClosePicker);
                                    }
                                });
                            });
                            if let Some((name, summary)) = &preview {
                                let line = widgets::line(
                                    ui,
                                    &format!("{name}: {summary}"),
                                    theme::regular(12.0),
                                    palette.secondary,
                                    width,
                                    1,
                                );
                                let (rect, _) = ui.allocate_exact_size(
                                    vec2(width, line.size().y),
                                    Sense::hover(),
                                );
                                line.paint(ui, rect.min, palette.secondary);
                            }
                            let body_height = HEIGHT - 100.0;
                            ui.allocate_ui_with_layout(
                                vec2(width, body_height),
                                Layout::top_down(Align::Min),
                                |ui| {
                                    if let Some(emoji) = emoji_grid(
                                        app,
                                        ui,
                                        &palette,
                                        "reaction-emoji-search",
                                        "reaction-emoji-grid",
                                        "Frequently Used",
                                    ) {
                                        let current = app
                                            .conversations
                                            .get(&chat)
                                            .and_then(|conversation| conversation.message(&message))
                                            .and_then(conversation::own_reaction);
                                        app.actions.push(Action::React {
                                            chat: chat.clone(),
                                            message: message.clone(),
                                            emoji: conversation::reaction_choice(current, &emoji),
                                        });
                                    }
                                },
                            );
                            ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                                let has_recent = app
                                    .settings
                                    .reaction_emoji
                                    .iter()
                                    .any(|(emoji, _)| emojis::get(emoji).is_some());
                                category_tabs(
                                    app,
                                    ui,
                                    &palette,
                                    "reaction-emoji-grid",
                                    "Frequently Used",
                                    has_recent,
                                );
                            });
                        });
                },
            );
        });
    let rect = area.response.rect;
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("reaction-picker-rect"), rect));
    let clicked_outside = ctx.input(|input| {
        input.pointer.any_pressed()
            && input.pointer.interact_pos().is_some_and(|pos| {
                !rect.contains(pos) && !menu.is_some_and(|menu| menu.contains(pos))
            })
    });
    if clicked_outside {
        app.actions.push(Action::ClosePicker);
    }
}

fn category_tabs(
    app: &mut App,
    ui: &mut egui::Ui,
    palette: &Palette,
    grid_salt: &str,
    recent_label: &'static str,
    has_recent: bool,
) {
    let tabs: Vec<_> = category_entries(has_recent, recent_label).collect();
    let default = tabs.first().map(|(_, _, label)| *label);
    let current = visible_category(ui, grid_salt, &app.picker_search)
        .filter(|label| tabs.iter().any(|&(_, _, tab)| tab == *label));
    let cell = ((ui.available_width() - 4.0) / tabs.len() as f32).clamp(24.0, 36.0);
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), cell),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let extra = (ui.available_width() - cell * tabs.len() as f32).max(0.0) / 2.0;
            ui.add_space(extra);
            for &(_, glyph, label) in &tabs {
                let selected = current == Some(label)
                    || (current.is_none()
                        && app.picker_search.is_empty()
                        && Some(label) == default);
                let (rect, response) = ui.allocate_exact_size(vec2(cell, cell), Sense::click());
                if ui.is_rect_visible(rect) {
                    if selected {
                        ui.painter().rect_filled(
                            rect.shrink(2.0),
                            6.0,
                            palette.accent.gamma_multiply(0.22),
                        );
                    } else if response.hovered() {
                        ui.painter()
                            .rect_filled(rect.shrink(2.0), 6.0, palette.surface_hover);
                    }
                    let line =
                        widgets::line(ui, glyph, theme::regular(16.0), palette.text, cell, 1);
                    line.paint(ui, rect.center() - line.size() / 2.0, palette.text);
                    if selected {
                        ui.painter().hline(
                            rect.x_range().shrink(6.0),
                            rect.bottom() - 3.0,
                            Stroke::new(2.0, palette.accent),
                        );
                    }
                }
                if response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(label)
                    .clicked()
                {
                    app.picker_search.clear();
                    app.emoji_selected = 0;
                    app.emoji_jump = Some(label);
                }
            }
        },
    );
}

fn visible_category(ui: &egui::Ui, scroll_salt: &str, query: &str) -> Option<&'static str> {
    if !query.trim().is_empty() {
        return None;
    }
    ui.ctx()
        .data(|data| {
            data.get_temp::<Option<&'static str>>(egui::Id::new(("emoji-visible", scroll_salt)))
        })
        .flatten()
}

fn emoji_grid(
    app: &mut App,
    ui: &mut egui::Ui,
    palette: &Palette,
    search_id: &'static str,
    scroll_salt: &'static str,
    recent_label: &'static str,
) -> Option<String> {
    let newly_opened = app.picker_focus;
    let search_active =
        app.picker_focus || ui.memory(|memory| memory.has_focus(egui::Id::new(search_id)));
    let movement = search_active
        .then(|| {
            [
                Key::ArrowLeft,
                Key::ArrowRight,
                Key::ArrowUp,
                Key::ArrowDown,
            ]
            .into_iter()
            .find(|key| take_plain_key(ui, *key))
        })
        .flatten();
    let submit = search_active && take_plain_key(ui, Key::Enter);
    let mut search = app.picker_search.clone();
    let response = search_box(ui, palette, search_id, &mut search, "Search emoji");
    let query_changed = search != app.picker_search;
    if query_changed {
        app.picker_search = search;
        app.emoji_selected = 0;
    }
    if app.picker_focus {
        app.picker_focus = false;
        response.request_focus();
    }
    // Reserve scrollbar space and fill the remaining width with whole columns.
    let width = ui.available_width() - 6.0;
    let columns = ((width / CELL).floor() as usize).max(1);
    let cell = width / columns as f32;
    let frequent: Vec<_> = app
        .settings
        .reaction_emoji
        .iter()
        .map(|(emoji, _)| emoji.clone())
        .collect();
    let recent = if app.reaction_target.is_some() {
        &frequent
    } else {
        &app.settings.recent_emoji
    };
    let rows = rows_for(&app.picker_search, recent, columns, recent_label);
    let emoji_count = rows
        .iter()
        .map(|row| match row {
            Row::Header(_) => 0,
            Row::Emoji { values, .. } => values.len(),
        })
        .sum();
    if let Some(key) = movement {
        app.emoji_selected = move_emoji_selection(app.emoji_selected, emoji_count, columns, key);
    } else if emoji_count == 0 {
        app.emoji_selected = 0;
    } else {
        app.emoji_selected = app.emoji_selected.min(emoji_count - 1);
    }
    let row_height = CELL;
    let mut picked = (submit && emoji_count > 0).then(|| {
        rows.iter()
            .filter_map(|row| match row {
                Row::Header(_) => None,
                Row::Emoji { values, .. } => Some(values.as_slice()),
            })
            .flatten()
            .nth(app.emoji_selected)
            .copied()
            .expect("emoji selection is in range")
            .to_owned()
    });
    // `show_rows` must use the same zero spacing as the grid.
    ui.spacing_mut().item_spacing = Vec2::ZERO;
    let scroll_id = ui.make_persistent_id(scroll_salt);
    let mut grid = egui::ScrollArea::vertical()
        .id_salt(scroll_salt)
        .auto_shrink([false, false]);
    let jump = resolve_jump(app.emoji_jump.take(), &rows);
    if let Some(label) = jump
        && let Some(row) = header_row(&rows, label)
    {
        grid = grid.vertical_scroll_offset(row as f32 * row_height);
    } else if newly_opened || query_changed {
        grid = grid.vertical_scroll_offset(0.0);
    } else if movement.is_some()
        && let Some(row) = rows.iter().position(|row| match row {
            Row::Header(_) => false,
            Row::Emoji { first, values } => {
                (*first..*first + values.len()).contains(&app.emoji_selected)
            }
        })
    {
        let current =
            egui::scroll_area::State::load(ui.ctx(), scroll_id).map_or(0.0, |state| state.offset.y);
        let visible = ui.available_height();
        let top = row as f32 * row_height;
        let bottom = top + row_height;
        let target = if top < current {
            top
        } else if bottom > current + visible {
            bottom - visible
        } else {
            current
        };
        grid = grid.vertical_scroll_offset(target.max(0.0));
    }
    let offset =
        egui::scroll_area::State::load(ui.ctx(), scroll_id).map_or(0.0, |state| state.offset.y);
    let visible_index = (offset / row_height).floor() as usize;
    let visible_label = rows
        .iter()
        .take(visible_index.saturating_add(1).min(rows.len()))
        .rev()
        .find_map(|row| match row {
            Row::Header(label) => Some(*label),
            Row::Emoji { .. } => None,
        });
    ui.ctx().data_mut(|data| {
        data.insert_temp(egui::Id::new(("emoji-visible", scroll_salt)), visible_label);
    });
    grid.show_rows(ui, row_height, rows.len(), |ui, range| {
        for row in &rows[range] {
            match row {
                Row::Header(label) => {
                    let (rect, _) = ui.allocate_exact_size(
                        vec2(ui.available_width(), row_height),
                        Sense::hover(),
                    );
                    ui.painter().text(
                        pos2(rect.left() + 4.0, rect.bottom() - 8.0),
                        Align2::LEFT_BOTTOM,
                        *label,
                        theme::semibold(12.5),
                        palette.secondary,
                    );
                }
                Row::Emoji { first, values } => {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::ZERO;
                        for (offset, emoji) in values.iter().enumerate() {
                            let selected = *first + offset == app.emoji_selected;
                            let (rect, response) =
                                ui.allocate_exact_size(vec2(cell, row_height), Sense::click());
                            if ui.is_rect_visible(rect) {
                                if selected {
                                    ui.painter().rect_filled(
                                        rect.shrink(2.0),
                                        6.0,
                                        palette.accent.gamma_multiply(0.22),
                                    );
                                    ui.painter().rect_stroke(
                                        rect.shrink(2.0),
                                        6.0,
                                        Stroke::new(1.0, palette.accent),
                                        egui::StrokeKind::Inside,
                                    );
                                } else if response.hovered() {
                                    ui.painter().rect_filled(
                                        rect.shrink(2.0),
                                        6.0,
                                        palette.surface_hover,
                                    );
                                }
                                let line = widgets::line(
                                    ui,
                                    emoji,
                                    theme::regular(24.0),
                                    palette.text,
                                    cell,
                                    1,
                                );
                                line.paint(ui, rect.center() - line.size() / 2.0, palette.text);
                            }
                            if response
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                app.emoji_selected = *first + offset;
                                picked = Some((*emoji).to_owned());
                            }
                        }
                    });
                }
            }
        }
    });
    picked
}

#[cfg(test)]
mod emoji_tests {
    use super::*;

    #[test]
    fn arrows_move_through_the_emoji_grid() {
        assert_eq!(move_emoji_selection(0, 25, 10, Key::ArrowRight), 1);
        assert_eq!(move_emoji_selection(1, 25, 10, Key::ArrowDown), 11);
        assert_eq!(move_emoji_selection(11, 25, 10, Key::ArrowLeft), 10);
        assert_eq!(move_emoji_selection(10, 25, 10, Key::ArrowUp), 0);
        assert_eq!(move_emoji_selection(20, 25, 10, Key::ArrowDown), 20);
        assert_eq!(move_emoji_selection(24, 25, 10, Key::ArrowRight), 24);
    }

    #[test]
    fn search_finds_emoji_by_name_and_shortcode() {
        let rows = rows_for("crab", &[], 8, "Recent");
        let found: Vec<&str> = rows
            .iter()
            .filter_map(|row| match row {
                Row::Emoji { values, .. } => Some(values.as_slice()),
                Row::Header(_) => None,
            })
            .flatten()
            .copied()
            .collect();
        assert!(found.contains(&"🦀"), "{found:?}");
    }

    #[test]
    fn empty_query_lists_recent_then_groups() {
        let rows = rows_for("", &["👍".into()], 8, "Frequently Used");
        assert!(matches!(rows.first(), Some(Row::Header("Frequently Used"))));
        assert!(
            rows.iter()
                .any(|row| matches!(row, Row::Header("Smileys & Emotion")))
        );
        assert!(rows.iter().any(|row| matches!(row, Row::Header("Flags"))));
    }

    #[test]
    fn empty_recent_omits_the_recent_header() {
        for recent in [Vec::new(), vec!["not-an-emoji".into()]] {
            let rows = rows_for("", &recent, 8, "Frequently Used");
            assert!(
                !rows.iter().any(|row| matches!(
                    row,
                    Row::Header("Frequently Used") | Row::Header("Recent")
                )),
                "{recent:?}"
            );
            assert!(
                matches!(rows.first(), Some(Row::Header("Smileys & Emotion"))),
                "{recent:?}"
            );
        }
    }

    #[test]
    fn empty_recent_omits_the_frequently_used_tab() {
        let labels: Vec<&str> = category_entries(false, "Frequently Used")
            .map(|(_, _, label)| label)
            .collect();
        assert!(!labels.contains(&"Frequently Used"));
        assert_eq!(labels.first().copied(), Some("Smileys & Emotion"));
        let with_recent: Vec<&str> = category_entries(true, "Frequently Used")
            .map(|(_, _, label)| label)
            .collect();
        assert_eq!(with_recent.first().copied(), Some("Frequently Used"));
        assert_eq!(with_recent.len(), labels.len() + 1);
        // The composer names its recent section "Recent".
        let composer: Vec<&str> = category_entries(true, "Recent")
            .map(|(_, _, label)| label)
            .collect();
        assert_eq!(composer.first().copied(), Some("Recent"));
        assert_eq!(&composer[1..], &with_recent[1..]);
    }

    #[test]
    fn empty_recent_jump_falls_back_to_the_first_unicode_group() {
        let empty = rows_for("", &[], 8, "Frequently Used");
        assert_eq!(
            resolve_jump(Some("Frequently Used"), &empty),
            Some("Smileys & Emotion")
        );
        assert_eq!(
            resolve_jump(Some("Recent"), &rows_for("", &[], 8, "Recent")),
            Some("Smileys & Emotion")
        );
        assert_eq!(resolve_jump(Some("Flags"), &empty), Some("Flags"));
        assert_eq!(resolve_jump(None, &empty), None);
        let with_recent = rows_for("", &["👍".into()], 8, "Frequently Used");
        assert_eq!(
            resolve_jump(Some("Frequently Used"), &with_recent),
            Some("Frequently Used")
        );
    }

    #[test]
    fn place_picker_keeps_the_framed_size_on_screen() {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(500.0, 450.0));
        let outer_width = WIDTH + f32::from(FRAME_MARGIN) * 2.0;
        let outer_height = HEIGHT + f32::from(FRAME_MARGIN) * 2.0;
        let anchor = Rect::from_min_size(pos2(460.0, 10.0), vec2(34.0, 34.0));
        let inner = place_picker(screen, Some(anchor), WIDTH, HEIGHT);
        let outer = place_picker(screen, Some(anchor), outer_width, outer_height);
        assert!(
            inner.x + outer_width > screen.right() - 8.0,
            "inner size would clip the framed picker on the right: {inner:?}"
        );
        assert!(
            outer.x + outer_width <= screen.right() - 8.0 + 0.01,
            "outer size stays on the right: {outer:?}"
        );
        assert!(
            outer.y + outer_height <= screen.bottom() - 8.0 + 0.01,
            "outer size stays on the bottom: {outer:?}"
        );
        assert!(outer.x >= screen.left() + 8.0 - 0.01);
        assert!(outer.y >= screen.top() + 8.0 - 0.01);
    }
}

// --- GIFs ---------------------------------------------------------------

fn gif_tab(app: &mut App, ui: &mut egui::Ui, palette: &Palette) {
    // Ask for a key when none is set or GIPHY rejects it.
    let bad_key = app.gif_error.as_ref().is_some_and(|error| error.bad_key);
    if app.settings.effective_giphy_key().is_none() || bad_key {
        ui.add_space(8.0);
        if let Some(error) = app.gif_error.as_ref().filter(|error| error.bad_key) {
            theme::paragraph(ui, &error.message, theme::regular(13.0), palette.danger);
            ui.add_space(6.0);
        }
        theme::paragraph(
            ui,
            if bad_key {
                "This GIPHY API key was rejected. Create a free key at developers.giphy.com and paste it here. It is saved in your settings."
            } else {
                "GIF search needs a GIPHY API key. Create a free key at developers.giphy.com and paste it here. It is saved in your settings."
            },
            theme::regular(13.0),
            palette.text,
        );
        ui.add_space(6.0);
        if theme::link(
            ui,
            "developers.giphy.com",
            theme::medium(13.0),
            palette.link,
        )
        .clicked()
        {
            app.actions
                .push(Action::OpenUrl("https://developers.giphy.com/".to_owned()));
        }
        ui.add_space(6.0);
        let field = Frame::new()
            .fill(palette.surface)
            .corner_radius(CornerRadius::same(theme::RADIUS))
            .inner_margin(Margin::symmetric(10, 6))
            .show(ui, |ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut app.settings.giphy_key)
                        .hint_text(
                            egui::RichText::new("GIPHY API key")
                                .color(palette.dim)
                                .font(theme::regular(13.5)),
                        )
                        .font(theme::regular(13.5))
                        .text_color(palette.text)
                        .frame(Frame::NONE)
                        .desired_width(f32::INFINITY),
                );
                if response.changed() {
                    app.actions.push(Action::SettingsChanged);
                }
                if response.lost_focus() && !app.settings.giphy_key.trim().is_empty() {
                    app.actions.push(Action::SearchGifs(String::new()));
                }
                response
            });
        theme::focus_outline(
            ui,
            field.inner.id,
            field.response.rect,
            f32::from(theme::RADIUS),
        );
        return;
    }
    let mut query = app.picker_search.clone();
    let submit = ui.memory(|memory| memory.has_focus(egui::Id::new("gif-search")))
        && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
    search_box(
        ui,
        palette,
        "gif-search",
        &mut query,
        "Search GIFs via GIPHY",
    );
    if query != app.picker_search {
        app.picker_search = query.clone();
    }
    if submit && query.trim() != app.gif_query.trim() {
        app.actions.push(Action::SearchGifs(query));
    }
    if app.gif_pending {
        ui.horizontal(|ui| {
            theme::spinner(ui, 16.0, palette.accent);
            theme::text(ui, "Searching…", theme::regular(12.5), palette.secondary);
        });
    } else if let Some(error) = &app.gif_error {
        theme::paragraph(ui, &error.message, theme::regular(13.0), palette.danger);
    }
    let results = app.gif_results.clone();
    let columns = 3;
    let gap = 6.0;
    let tile_width = (ui.available_width() - gap * (columns as f32 - 1.0)) / columns as f32;
    let mut picked = None;
    egui::ScrollArea::vertical()
        .id_salt("gif-grid")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(gap, gap);
            for row in results.chunks(columns) {
                ui.horizontal(|ui| {
                    for gif in row {
                        let ratio =
                            (gif.height.max(1) as f32 / gif.width.max(1) as f32).clamp(0.5, 1.4);
                        let size = vec2(tile_width, (tile_width * ratio).min(150.0));
                        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                        if ui.is_rect_visible(rect) {
                            ui.painter().rect_filled(rect, 6.0, palette.surface);
                            if let Some(still) = &gif.still {
                                widgets::file_image(ui, still)
                                    .fit_to_exact_size(size)
                                    .corner_radius(6.0)
                                    .paint_at(ui, rect);
                            }
                            if response.hovered() {
                                ui.painter().rect_stroke(
                                    rect,
                                    6.0,
                                    Stroke::new(2.0, palette.accent),
                                    egui::StrokeKind::Inside,
                                );
                            }
                        }
                        if response
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            picked = Some(gif.clone());
                        }
                    }
                });
            }
            if results.is_empty() && !app.gif_pending && app.gif_error.is_none() {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    theme::text(
                        ui,
                        "Search for a GIF or browse trending results.",
                        theme::regular(13.0),
                        palette.secondary,
                    );
                });
            }
        });
    if let Some(gif) = picked {
        app.actions.push(Action::SendGif(gif));
    }
}

// --- stickers -----------------------------------------------------------

/// Side of one tab in the sticker tab strip.
const SHELF_TAB: f32 = 34.0;

/// What a click or the right-click menu asked of a sticker tile.
#[derive(Default)]
struct StickerChoices {
    send: Option<PathBuf>,
    save: Option<PathBuf>,
    forget: Option<PathBuf>,
    /// Taking a sticker out of Recent.
    unrecent: Option<PathBuf>,
    /// Filing a sticker into a local pack, or taking it out of one.
    pack: Option<(PathBuf, PathBuf, bool)>,
}

/// The tab strip, as in WhatsApp: Recent, Favorites, every pack, then the
/// page for adding stickers.
fn shelf_entries(packs: &[StickerPack]) -> Vec<StickerShelf> {
    let mut shelves = vec![StickerShelf::Recent, StickerShelf::Favorites];
    shelves.extend(
        packs
            .iter()
            .map(|pack| StickerShelf::Pack(pack.dir.clone())),
    );
    shelves.push(StickerShelf::Add);
    shelves
}

/// Where a shelf's tab was drawn, so interaction tests and the tour can click it.
pub fn shelf_tab_id(shelf: &StickerShelf) -> egui::Id {
    egui::Id::new(("sticker-shelf-tab", shelf))
}

/// The row of tabs under the search field. Packs show their first sticker.
fn shelf_strip(app: &mut App, ui: &mut egui::Ui, palette: &Palette) {
    let current = app.sticker_shelf.clone();
    let locale = app.locale;
    let shelves = shelf_entries(&app.sticker_packs);
    let mut picked = None;
    egui::ScrollArea::horizontal()
        .id_salt("sticker-shelves")
        .auto_shrink([false, true])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for shelf in shelves {
                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::splat(SHELF_TAB), Sense::click());
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(shelf_tab_id(&shelf), rect));
                    let pack = match &shelf {
                        StickerShelf::Pack(dir) => {
                            app.sticker_packs.iter().find(|pack| pack.dir == *dir)
                        }
                        _ => None,
                    };
                    let selected = shelf == current;
                    if ui.is_rect_visible(rect) {
                        if selected {
                            ui.painter().rect_filled(
                                rect.shrink(1.0),
                                6.0,
                                palette.accent.gamma_multiply(0.22),
                            );
                        } else if response.hovered() {
                            ui.painter()
                                .rect_filled(rect.shrink(1.0), 6.0, palette.surface_hover);
                        }
                        let icon = match &shelf {
                            StickerShelf::Recent => Some(Icon::Clock),
                            StickerShelf::Favorites => Some(Icon::Star),
                            StickerShelf::Add => Some(Icon::Plus),
                            StickerShelf::Pack(_) => None,
                        };
                        let color = if selected {
                            palette.text
                        } else {
                            palette.secondary
                        };
                        match (icon, pack.and_then(|pack| pack.stickers.first())) {
                            (Some(icon), _) => theme::paint_icon(ui, icon, rect, 17.0, color),
                            (None, Some(cover)) => sticker_picture(ui, cover, rect.shrink(5.0)),
                            (None, None) => theme::paint_icon(ui, Icon::Sticker, rect, 17.0, color),
                        }
                        if selected {
                            ui.painter().hline(
                                rect.x_range().shrink(6.0),
                                rect.bottom() - 2.0,
                                Stroke::new(2.0, palette.accent),
                            );
                        }
                    }
                    let tip = match (&shelf, pack) {
                        (StickerShelf::Recent, _) => gettext(locale, "Recent"),
                        (StickerShelf::Favorites, _) => gettext(locale, "Favorites"),
                        (StickerShelf::Add, _) => gettext(locale, "Add stickers"),
                        (StickerShelf::Pack(_), Some(pack)) => pack.name.clone().into(),
                        (StickerShelf::Pack(_), None) => "".into(),
                    };
                    if response
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(tip.as_ref())
                        .clicked()
                    {
                        picked = Some(shelf);
                    }
                }
            });
        });
    if let Some(shelf) = picked {
        app.sticker_search.clear();
        app.actions.push(Action::SelectStickerShelf(shelf));
    }
}

/// A short centered note in place of an empty grid.
fn shelf_hint(ui: &mut egui::Ui, palette: &Palette, text: &str) {
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        theme::paragraph(ui, text, theme::regular(13.0), palette.secondary);
    });
}

fn sticker_tab(app: &mut App, ui: &mut egui::Ui, palette: &Palette) {
    let locale = app.locale;
    search_box(
        ui,
        palette,
        "sticker-search",
        &mut app.sticker_search,
        &gettext(locale, "Search by emoji, word, or pack"),
    );
    shelf_strip(app, ui, palette);
    ui.add_space(2.0);
    let query = app.sticker_search.trim().to_owned();
    let shelf = app.sticker_shelf.clone();
    if query.is_empty() && shelf == StickerShelf::Add {
        add_page(app, ui, palette);
        return;
    }
    let local: Vec<StickerPack> = app
        .sticker_packs
        .iter()
        .filter(|pack| pack.local)
        .cloned()
        .collect();
    let grid = GridContext {
        locale,
        animate: app.window_focused,
        local: &local,
    };
    let mut choices = StickerChoices::default();
    let mut delete_pack = None;
    let mut share_pack = None;
    if !query.is_empty() {
        let found = crate::sticker_search::search(
            &crate::sticker_search::Library {
                recent: &app.stickers,
                favorites: &app.stickers_saved,
                packs: &app.sticker_packs,
                emojis: &app.sticker_emojis,
            },
            &query,
        );
        if found.is_empty() {
            shelf_hint(
                ui,
                palette,
                &gettext(locale, "No stickers match your search."),
            );
        } else {
            egui::ScrollArea::vertical()
                .id_salt("sticker-results")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    sticker_grid(ui, palette, &found, Shelf::Pack, &grid, &mut choices);
                });
        }
    } else {
        match &shelf {
            StickerShelf::Recent if app.stickers.is_empty() => {
                let text = if app.stickers_pending {
                    gettext(locale, "Loading your stickers…")
                } else {
                    gettext(locale, "Stickers you send appear here.")
                };
                shelf_hint(ui, palette, &text);
            }
            StickerShelf::Favorites if app.stickers_saved.is_empty() => shelf_hint(
                ui,
                palette,
                &gettext(
                    locale,
                    "Right-click any sticker to add it to your favorites. They stay in sync with your phone.",
                ),
            ),
            StickerShelf::Pack(_) => {
                let Some(pack) = app.selected_pack().cloned() else {
                    return;
                };
                ui.horizontal(|ui| {
                    theme::text(ui, &pack.name, theme::semibold(13.0), palette.text);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let remove = theme::icon_button(
                            ui,
                            Icon::Trash,
                            15.0,
                            palette.dim,
                            palette.danger,
                            &gettext(locale, "Remove this pack"),
                        );
                        ui.ctx()
                            .data_mut(|data| data.insert_temp(remove_pack_id(), remove.rect));
                        if remove.clicked() {
                            delete_pack = Some(pack.dir.clone());
                        }
                        if !pack.stickers.is_empty()
                            && theme::icon_button(
                                ui,
                                Icon::Forward,
                                15.0,
                                palette.dim,
                                palette.text,
                                &gettext(locale, "Send this pack to the chat"),
                            )
                            .clicked()
                        {
                            share_pack = Some(pack.dir.clone());
                        }
                    });
                });
                if pack.stickers.is_empty() {
                    shelf_hint(
                        ui,
                        palette,
                        &gettext(
                            locale,
                            "Nothing here yet. Right-click any sticker to add it to this pack.",
                        ),
                    );
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt(("sticker-pack", &pack.dir))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            sticker_grid(
                                ui,
                                palette,
                                &pack.stickers,
                                Shelf::Pack,
                                &grid,
                                &mut choices,
                            );
                        });
                }
            }
            StickerShelf::Recent | StickerShelf::Favorites => {
                let (stickers, kind) = if shelf == StickerShelf::Recent {
                    (&app.stickers, Shelf::Recent)
                } else {
                    (&app.stickers_saved, Shelf::Favorites)
                };
                egui::ScrollArea::vertical()
                    .id_salt(("sticker-shelf", &shelf))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        sticker_grid(ui, palette, stickers, kind, &grid, &mut choices);
                    });
            }
            StickerShelf::Add => {}
        }
    }
    if let Some(path) = choices.send {
        app.actions.push(Action::SendSticker(path));
    }
    if let Some(path) = choices.save {
        app.actions.push(Action::SaveSticker(path));
    }
    if let Some(path) = choices.forget {
        app.actions.push(Action::ForgetSticker(path));
    }
    if let Some(path) = choices.unrecent {
        app.actions.push(Action::RemoveRecentSticker(path));
    }
    if let Some((pack, sticker, member)) = choices.pack {
        app.actions.push(Action::SetStickerPack {
            pack,
            sticker,
            member,
        });
    }
    if let Some(dir) = delete_pack {
        app.actions.push(Action::DeleteStickerPack(dir));
    }
    if let Some(dir) = share_pack {
        app.actions.push(Action::ShareStickerPack(dir));
    }
}

/// The page behind the strip's plus: import packs, start a pack, or make a
/// sticker from a picture.
fn add_page(app: &mut App, ui: &mut egui::Ui, palette: &Palette) {
    let locale = app.locale;
    egui::ScrollArea::vertical()
        .id_salt("sticker-add")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(4.0);
            theme::text(
                ui,
                gettext(locale, "Import a pack"),
                theme::semibold(13.0),
                palette.text,
            );
            theme::paragraph(
                ui,
                gettext(
                    locale,
                    "Paste a signal.art link from signalstickers.org, or open a .wastickers file.",
                ),
                theme::regular(12.5),
                palette.secondary,
            );
            ui.add_space(4.0);
            import_row(app, ui, palette);
            ui.add_space(14.0);
            theme::text(
                ui,
                gettext(locale, "Make your own"),
                theme::semibold(13.0),
                palette.text,
            );
            theme::paragraph(
                ui,
                gettext(
                    locale,
                    "Start a pack, then right-click any sticker to add it. Or turn a picture into a sticker.",
                ),
                theme::regular(12.5),
                palette.secondary,
            );
            ui.add_space(4.0);
            new_pack_row(app, ui, palette);
            ui.add_space(6.0);
            if theme::soft_button(
                ui,
                palette,
                Some(Icon::Image),
                &gettext(locale, "Make a sticker from a picture…"),
                false,
            )
            .clicked()
            {
                app.actions.push(Action::PickStickerPicture);
            }
        });
}

/// Names and creates a local pack.
fn new_pack_row(app: &mut App, ui: &mut egui::Ui, palette: &Palette) {
    let locale = app.locale;
    let mut create = false;
    ui.horizontal(|ui| {
        let label = gettext(locale, "Create pack");
        let button = theme::soft_button_width(ui, &label, true);
        let field = Frame::new()
            .fill(palette.surface)
            .corner_radius(CornerRadius::same(theme::RADIUS))
            .inner_margin(Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut app.sticker_pack_name)
                        .id(egui::Id::new("sticker-pack-name"))
                        .hint_text(
                            egui::RichText::new(gettext(locale, "New pack…"))
                                .color(palette.dim)
                                .font(theme::regular(13.0)),
                        )
                        .font(theme::regular(13.0))
                        .text_color(palette.text)
                        .frame(Frame::NONE)
                        .desired_width(ui.available_width() - button - 26.0),
                )
            });
        theme::focus_outline(
            ui,
            field.inner.id,
            field.response.rect,
            f32::from(theme::RADIUS),
        );
        if field.inner.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
            create = true;
        }
        if theme::soft_button(ui, palette, Some(Icon::Plus), &label, false).clicked() {
            create = true;
        }
    });
    let name = app.sticker_pack_name.trim().to_owned();
    if create && !name.is_empty() {
        app.actions.push(Action::CreateStickerPack(name));
        app.sticker_pack_name.clear();
    }
}

/// Imports packs from pasted signal.art links or .wastickers files.
fn import_row(app: &mut App, ui: &mut egui::Ui, palette: &Palette) {
    let locale = app.locale;
    let find = gettext(locale, "Find packs");
    let open = gettext(locale, "Open file");
    ui.horizontal(|ui| {
        let spacing = ui.spacing().item_spacing.x;
        let buttons = theme::soft_button_width(ui, &find, true)
            + theme::soft_button_width(ui, &open, true)
            + spacing * 2.0;
        let field = Frame::new()
            .fill(palette.surface)
            .corner_radius(CornerRadius::same(theme::RADIUS))
            .inner_margin(Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut app.sticker_link)
                        .id(egui::Id::new("sticker-link"))
                        .hint_text(
                            egui::RichText::new(gettext(locale, "Paste a signal.art link"))
                                .color(palette.dim)
                                .font(theme::regular(13.0)),
                        )
                        .font(theme::regular(13.0))
                        .text_color(palette.text)
                        .frame(Frame::NONE)
                        .desired_width(ui.available_width() - buttons - 26.0),
                )
            });
        theme::focus_outline(
            ui,
            field.inner.id,
            field.response.rect,
            f32::from(theme::RADIUS),
        );
        let field = field.inner;
        let pasted = field.changed()
            && crate::backend::sticker_import::looks_like_signal_url(app.sticker_link.trim());
        let submitted = field.lost_focus()
            && ui.input(|input| input.key_pressed(Key::Enter))
            && !app.sticker_link.trim().is_empty();
        if pasted || submitted {
            app.actions
                .push(Action::ImportStickerUrl(app.sticker_link.trim().to_owned()));
        }
        // Open the gallery where users can copy signal.art pack links.
        if theme::soft_button(ui, palette, Some(Icon::ExternalLink), &find, false)
            .on_hover_text(gettext(locale, "Browse signalstickers.org").as_ref())
            .clicked()
        {
            app.actions
                .push(Action::OpenUrl("https://signalstickers.org/".to_owned()));
        }
        if theme::soft_button(ui, palette, Some(Icon::FileText), &open, false).clicked() {
            app.actions.push(Action::PickStickerArchive);
        }
    });
    if app.sticker_import_pending {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            theme::spinner(ui, 14.0, palette.accent);
            theme::text(
                ui,
                gettext(locale, "Importing the pack…"),
                theme::regular(12.5),
                palette.secondary,
            );
        });
    }
}

/// Which list a sticker grid shows, which decides its right-click menu.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shelf {
    Recent,
    Favorites,
    Pack,
}

/// Where the pack header's remove button was drawn, for interaction tests.
fn remove_pack_id() -> egui::Id {
    egui::Id::new("remove-pack-button")
}

/// Where the first tile of the last grid was drawn, so interaction tests and
/// the demo tour can click it.
pub fn first_tile_id() -> egui::Id {
    egui::Id::new("first-sticker-tile")
}

/// What every sticker grid in the tab shares.
struct GridContext<'a> {
    locale: crate::i18n::Locale,
    /// Play animated stickers on hover.
    animate: bool,
    /// Local packs a sticker can be filed into.
    local: &'a [StickerPack],
}

/// Sticker tiles. Click sends; right-click saves, removes, or files the tile
/// into one of the local packs.
fn sticker_grid(
    ui: &mut egui::Ui,
    palette: &Palette,
    stickers: &[PathBuf],
    shelf: Shelf,
    grid: &GridContext<'_>,
    choices: &mut StickerChoices,
) {
    let animate = grid.animate;
    let remove_label = gettext(grid.locale, "Remove from favorites");
    let save_label = gettext(grid.locale, "Add to favorites");
    let unrecent_label = gettext(grid.locale, "Remove from recents");
    let columns = 5;
    let gap = 6.0;
    let cell = (ui.available_width() - gap * (columns as f32 - 1.0)) / columns as f32;
    ui.spacing_mut().item_spacing = vec2(gap, gap);
    let mut labels = vec![
        remove_label.as_ref(),
        save_label.as_ref(),
        unrecent_label.as_ref(),
    ];
    labels.extend(grid.local.iter().map(|pack| pack.name.as_str()));
    let menu_width = widgets::menu_width(ui, &labels, true);
    for row in stickers.chunks(columns) {
        ui.horizontal(|ui| {
            for path in row {
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(cell), Sense::click());
                if Some(path) == stickers.first() {
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(first_tile_id(), rect));
                }
                paint_tile(ui, palette, path, rect, response.hovered(), animate);
                egui::Popup::context_menu(&response)
                    .width(menu_width)
                    .frame(widgets::menu_frame(palette))
                    .show(|ui| {
                        if shelf == Shelf::Favorites {
                            if widgets::menu_item(ui, palette, Some(Icon::StarOff), &remove_label) {
                                choices.forget = Some(path.clone());
                            }
                        } else if widgets::menu_item(ui, palette, Some(Icon::Star), &save_label) {
                            choices.save = Some(path.clone());
                        }
                        if shelf == Shelf::Recent
                            && widgets::menu_item(ui, palette, Some(Icon::X), &unrecent_label)
                        {
                            choices.unrecent = Some(path.clone());
                        }
                        if !grid.local.is_empty() {
                            widgets::menu_separator(ui, palette);
                            let hash = content_hash(ui.ctx(), path);
                            for pack in grid.local {
                                // A checked entry is a member: picking it again
                                // takes the sticker back out.
                                let member =
                                    hash.as_deref().is_some_and(|hash| pack_holds(pack, hash));
                                let icon = member.then_some(Icon::Check);
                                if widgets::menu_item(ui, palette, icon, &pack.name) {
                                    choices.pack = Some((pack.dir.clone(), path.clone(), !member));
                                }
                            }
                        }
                    });
                if response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    choices.send = Some(path.clone());
                }
            }
        });
    }
}

/// Draws one sticker tile. Only visible animated stickers decode, and they
/// keep a still first frame until the focused pointer hovers the tile.
fn paint_tile(
    ui: &mut egui::Ui,
    palette: &Palette,
    path: &Path,
    rect: Rect,
    hovered: bool,
    animate: bool,
) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    if hovered {
        ui.painter().rect_filled(rect, 8.0, palette.surface_hover);
    }
    let shown = rect.shrink(4.0);
    let animated = moves(ui.ctx(), path);
    let played = animated
        && match crate::animation::frame(ui, path, rect, animate && hovered) {
            crate::animation::Frame::Ready(texture) => {
                let size = texture.size_vec2();
                let scale = (shown.width() / size.x).min(shown.height() / size.y);
                let fitted = Rect::from_center_size(shown.center(), size * scale);
                ui.painter().image(
                    texture.id(),
                    fitted,
                    Rect::from_min_max(egui::Pos2::ZERO, pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                true
            }
            _ => false,
        };
    if !played {
        if animated {
            ui.painter().rect_filled(shown, 6.0, palette.surface);
            theme::paint_icon(ui, Icon::Sticker, shown, 24.0, palette.secondary);
        } else {
            sticker_picture(ui, path, shown);
        }
    }
}

/// Sticker tiles to look at, as in a shared pack before adding it.
pub fn sticker_preview_grid(
    ui: &mut egui::Ui,
    palette: &Palette,
    stickers: &[PathBuf],
    animate: bool,
) {
    let columns = 5;
    let gap = 6.0;
    let cell = (ui.available_width() - gap * (columns as f32 - 1.0)) / columns as f32;
    ui.spacing_mut().item_spacing = vec2(gap, gap);
    for row in stickers.chunks(columns) {
        ui.horizontal(|ui| {
            for path in row {
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(cell), Sense::hover());
                paint_tile(ui, palette, path, rect, response.hovered(), animate);
            }
        });
    }
}

/// A file's size and modification time, used to notice when a sticker changed
/// on disk so its memoized motion probe can be re-run.
#[derive(Clone, Copy, Debug, PartialEq)]
struct FileStamp {
    len: u64,
    modified: Option<std::time::SystemTime>,
}

impl FileStamp {
    fn of(path: &Path) -> Self {
        match std::fs::metadata(path) {
            Ok(metadata) => FileStamp {
                len: metadata.len(),
                modified: metadata.modified().ok(),
            },
            Err(_) => FileStamp {
                len: 0,
                modified: None,
            },
        }
    }
}

/// A memoized motion probe: the animated flag plus the file stamp it was read
/// from.
#[derive(Clone, Debug, PartialEq)]
struct MotionEntry {
    animated: bool,
    stamp: FileStamp,
}

/// Caches one motion probe per sticker path, re-probing only when the file
/// changes on disk. Without this the picker opened and read every visible
/// sticker every frame, which made tiles flicker between empty and decoded
/// and, when a read failed, left tiles permanently blank.
#[derive(Clone, Default)]
struct MotionMemo(HashMap<PathBuf, MotionEntry>);

impl MotionMemo {
    /// Returns whether `path` moves, calling `probe` only when the cached
    /// result is missing or the file changed since it was last probed.
    ///
    /// A failed probe is not memoized: on Windows a sharing violation or a
    /// transient read error would otherwise be remembered as "still" and an
    /// animated sticker would stay misclassified until the file next changed.
    fn moves(&mut self, path: &Path, probe: impl FnOnce(&Path) -> Option<bool>) -> bool {
        let stamp = FileStamp::of(path);
        if let Some(entry) = self.0.get(path)
            && entry.stamp == stamp
        {
            return entry.animated;
        }
        let Some(animated) = probe(path) else {
            // Report "still" for this frame without caching the failure, so the
            // next frame retries instead of trusting a transient error.
            return false;
        };
        self.0
            .insert(path.to_path_buf(), MotionEntry { animated, stamp });
        animated
    }
}

/// Content hashes of sticker files, re-read only when a file changes.
#[derive(Clone, Default)]
struct HashMemo(HashMap<PathBuf, (FileStamp, String)>);

/// A sticker's content hash, which names it inside local packs. Read once per
/// file version, and only while a sticker menu is open.
fn content_hash(ctx: &egui::Context, path: &Path) -> Option<String> {
    let id = egui::Id::new("sticker-content-hashes");
    let stamp = FileStamp::of(path);
    let known = ctx.data(|data| {
        data.get_temp::<HashMemo>(id).and_then(|memo| {
            memo.0
                .get(path)
                .filter(|(seen, _)| *seen == stamp)
                .map(|(_, hash)| hash.clone())
        })
    });
    if known.is_some() {
        return known;
    }
    let hash = crate::backend::sticker_store::content_hash(&std::fs::read(path).ok()?);
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<HashMemo>(id)
            .0
            .insert(path.to_path_buf(), (stamp, hash.clone()))
    });
    Some(hash)
}

/// Whether a local pack holds the sticker with this content hash.
fn pack_holds(pack: &StickerPack, hash: &str) -> bool {
    pack.stickers
        .iter()
        .any(|path| path.file_stem().is_some_and(|stem| stem == hash))
}

/// Returns whether a sticker moves, probing each path once and re-probing
/// only when the file changes on disk.
fn moves(ctx: &egui::Context, path: &Path) -> bool {
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<MotionMemo>(egui::Id::new("animated-sticker-paths"))
            .moves(path, probe_motion)
    })
}

/// Checks a WebP header for animation without decoding the file.
///
/// Returns `None` when the header cannot be read, so the caller can tell a read
/// failure from a still image and retry instead of memoizing the failure.
fn probe_motion(path: &Path) -> Option<bool> {
    let mut head = [0u8; 64];
    let mut file = std::fs::File::open(path).ok()?;
    let read = std::io::Read::read(&mut file, &mut head).ok()?;
    let head = &head[..read];
    Some(
        head.len() >= 12
            && &head[0..4] == b"RIFF"
            && &head[8..12] == b"WEBP"
            && head.windows(4).any(|window| window == b"ANIM"),
    )
}

fn sticker_picture(ui: &egui::Ui, path: &Path, rect: Rect) {
    widgets::file_image(ui, path)
        .fit_to_exact_size(rect.size())
        .paint_at(ui, rect);
}

#[cfg(test)]
mod motion_tests {
    use super::*;

    #[test]
    fn sticker_motion_is_probed_once_until_the_file_changes() {
        let dir = std::env::temp_dir().join(format!("zapfast-motion-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("creates temporary directory");
        let path = dir.join("animated.webp");
        std::fs::write(&path, b"RIFF0000WEBPANIM").expect("writes animated header");
        let mut memo = MotionMemo::default();
        let reads = std::cell::Cell::new(0usize);
        let probe = |path: &Path| {
            reads.set(reads.get() + 1);
            probe_motion(path)
        };
        assert!(memo.moves(&path, probe), "animated header is detected");
        assert!(memo.moves(&path, probe), "the memo answers the next probe");
        assert_eq!(
            reads.get(),
            1,
            "an unchanged file must not be re-read per frame"
        );
        // A different-sized still header changes the file stamp, so the memo
        // re-probes instead of trusting a stale result.
        std::fs::write(&path, b"RIFF0000WEBPVP8X still").expect("writes a still header");
        assert!(!memo.moves(&path, probe), "a changed file is re-probed");
        assert_eq!(reads.get(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn sticker_motion_is_served_from_the_picker_context() {
        let dir = std::env::temp_dir().join(format!("zapfast-motion-ctx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("creates temporary directory");
        let path = dir.join("animated.webp");
        std::fs::write(&path, b"RIFF0000WEBPANIM").expect("writes animated header");
        let ctx = egui::Context::default();
        assert!(moves(&ctx, &path));
        assert!(moves(&ctx, &path), "the picker memo survives frames");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_probe_is_retried_instead_of_memoized() {
        let dir = std::env::temp_dir().join(format!("zapfast-motion-retry-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("creates temporary directory");
        let path = dir.join("animated.webp");
        std::fs::write(&path, b"RIFF0000WEBPANIM").expect("writes animated header");
        let mut memo = MotionMemo::default();
        let reads = std::cell::Cell::new(0usize);
        // The first probe fails the way a sharing violation or a transient read
        // error does; the header is readable afterwards.
        let probe = |path: &Path| {
            reads.set(reads.get() + 1);
            if reads.get() == 1 {
                None
            } else {
                probe_motion(path)
            }
        };
        assert!(
            !memo.moves(&path, probe),
            "a failed probe reports the sticker as still for this frame"
        );
        assert!(
            memo.moves(&path, probe),
            "the failure is not memoized: the next frame re-probes and sees the animation"
        );
        assert_eq!(reads.get(), 2);
        assert!(memo.moves(&path, probe), "the successful probe is memoized");
        assert_eq!(reads.get(), 2, "a memoized success is not re-read");
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod pack_tests {
    use super::*;

    fn pack(name: &str, local: bool) -> StickerPack {
        StickerPack {
            name: name.into(),
            dir: PathBuf::from(format!("/packs/{name}")),
            stickers: Vec::new(),
            local,
        }
    }

    #[test]
    fn the_strip_holds_recent_favorites_every_pack_then_add() {
        let packs = vec![pack("Bom dia", true), pack("Frogs", false)];
        assert_eq!(
            shelf_entries(&packs),
            vec![
                StickerShelf::Recent,
                StickerShelf::Favorites,
                StickerShelf::Pack(PathBuf::from("/packs/Bom dia")),
                StickerShelf::Pack(PathBuf::from("/packs/Frogs")),
                StickerShelf::Add,
            ]
        );
    }

    #[test]
    fn a_local_pack_holds_a_sticker_by_its_content_hash() {
        let dir = tempfile::tempdir().expect("temp");
        let path = dir.path().join("from-a-chat.webp");
        std::fs::write(&path, b"sun").expect("writes");
        let ctx = egui::Context::default();
        let hash = content_hash(&ctx, &path).expect("hashes");
        assert_eq!(hash, crate::backend::sticker_store::content_hash(b"sun"));
        let mut holder = pack("Bom dia", true);
        assert!(!pack_holds(&holder, &hash));
        holder
            .stickers
            .push(holder.dir.join(format!("{hash}.webp")));
        assert!(pack_holds(&holder, &hash));
        // A changed file is hashed again.
        std::fs::write(&path, b"moon!").expect("writes");
        assert_ne!(content_hash(&ctx, &path).expect("hashes"), hash);
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

    /// Draws one frame of the sticker tab.
    fn frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
        let palette = app.palette;
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 420.0));
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            },
            |ui| sticker_tab(app, ui, &palette),
        );
        output.textures_delta.clear();
    }

    #[test]
    fn a_tab_opens_its_pack_and_the_pack_can_be_removed() {
        let directory = tempfile::tempdir().expect("creates a temporary directory");
        let (mut app, _events) = App::headless(
            crate::paths::AppDirs::under(directory.path()),
            crate::settings::Settings::default(),
        );
        app.sticker_packs = vec![pack("Bom dia", true), pack("Futebol", true)];
        let ctx = egui::Context::default();
        app.attach(&ctx);
        frame(&mut app, &ctx, Vec::new());
        let futebol = StickerShelf::Pack(PathBuf::from("/packs/Futebol"));
        let tab = ctx
            .data(|data| data.get_temp::<egui::Rect>(shelf_tab_id(&futebol)))
            .expect("the strip draws a tab per pack");
        frame(&mut app, &ctx, click(tab.center()));
        assert!(
            app.actions
                .contains(&Action::SelectStickerShelf(futebol.clone())),
            "clicking the tab opens the pack"
        );
        app.actions.clear();
        app.sticker_shelf = futebol;
        frame(&mut app, &ctx, Vec::new());
        // The pack's header offers its removal.
        let trash = ctx
            .data(|data| data.get_temp::<egui::Rect>(remove_pack_id()))
            .expect("the header draws a remove button");
        frame(&mut app, &ctx, click(trash.center()));
        assert!(
            app.actions
                .contains(&Action::DeleteStickerPack(PathBuf::from("/packs/Futebol"))),
            "the trash button removes the pack"
        );
    }

    #[test]
    fn typing_in_search_lists_matching_stickers_instead_of_the_shelf() {
        let directory = tempfile::tempdir().expect("creates a temporary directory");
        let (mut app, _events) = App::headless(
            crate::paths::AppDirs::under(directory.path()),
            crate::settings::Settings::default(),
        );
        let sticker = directory.path().join("duck.webp");
        std::fs::write(&sticker, b"RIFF0000WEBP").expect("writes");
        app.stickers_saved = vec![sticker.clone()];
        app.sticker_emojis = [(sticker.clone(), vec!["🦆".to_owned()])].into();
        app.sticker_search = "duck".into();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        frame(&mut app, &ctx, Vec::new());
        let tile = ctx
            .data(|data| data.get_temp::<egui::Rect>(first_tile_id()))
            .expect("a result is drawn");
        frame(&mut app, &ctx, click(tile.center()));
        assert!(app.actions.contains(&Action::SendSticker(sticker)));
    }
}
