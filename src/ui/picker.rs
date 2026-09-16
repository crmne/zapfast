//! The picker above the composer: emoji, GIFs, and stickers.
//! Also the full emoji picker used to react to a message.

use std::path::Path;

use egui::{
    Align, Align2, CornerRadius, Frame, Key, Layout, Margin, Modifiers, Rect, Sense, Stroke, Vec2,
    pos2, vec2,
};

use crate::app::App;
use crate::model::{Action, PickerTab};
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

/// Category tabs under the reaction emoji grid, WhatsApp-style.
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
) -> impl Iterator<Item = (Option<emojis::Group>, &'static str, &'static str)> {
    CATEGORIES
        .iter()
        .copied()
        .filter(move |(group, _, _)| group.is_some() || has_recent)
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
    if let Some(emoji) = emoji_grid(app, ui, palette, "emoji-search", "emoji-grid", "Recent") {
        app.actions.push(Action::InsertEmoji(emoji));
    }
}

fn reaction_picker(app: &mut App, ctx: &egui::Context) {
    let Some((chat, message)) = app.reaction_target.clone() else {
        return;
    };
    let palette = app.palette;
    let screen = ctx.content_rect();
    let outer_width = WIDTH + f32::from(FRAME_MARGIN) * 2.0;
    let outer_height = HEIGHT + f32::from(FRAME_MARGIN) * 2.0;
    let pos = place_picker(screen, app.reaction_anchor, outer_width, outer_height);
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
                            ui.set_width(WIDTH);
                            ui.set_height(HEIGHT);
                            ui.spacing_mut().item_spacing.y = 6.0;
                            let body_height = HEIGHT - 44.0;
                            ui.allocate_ui_with_layout(
                                vec2(WIDTH, body_height),
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
                                category_tabs(app, ui, &palette);
                            });
                        });
                },
            );
        });
    let rect = area.response.rect;
    let clicked_outside = ctx.input(|input| {
        input.pointer.any_pressed()
            && input
                .pointer
                .interact_pos()
                .is_some_and(|pos| !rect.contains(pos))
    });
    if clicked_outside {
        app.actions.push(Action::ClosePicker);
    }
}

fn category_tabs(app: &mut App, ui: &mut egui::Ui, palette: &Palette) {
    let has_recent = !usable_recent(&app.settings.recent_emoji).is_empty();
    let tabs: Vec<_> = category_entries(has_recent).collect();
    let default = tabs.first().map(|(_, _, label)| *label);
    let current = visible_category(ui, "reaction-emoji-grid", &app.picker_search)
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
    let rows = rows_for(
        &app.picker_search,
        &app.settings.recent_emoji,
        columns,
        recent_label,
    );
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
        let labels: Vec<&str> = category_entries(false).map(|(_, _, label)| label).collect();
        assert!(!labels.contains(&"Frequently Used"));
        assert_eq!(labels.first().copied(), Some("Smileys & Emotion"));
        let with_recent: Vec<&str> = category_entries(true).map(|(_, _, label)| label).collect();
        assert_eq!(with_recent.first().copied(), Some("Frequently Used"));
        assert_eq!(with_recent.len(), labels.len() + 1);
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
        Frame::new()
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
            });
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
                                egui::Image::new(crate::util::image_uri(still))
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

/// What a click or the right-click menu asked of a sticker tile.
#[derive(Default)]
struct StickerChoices {
    send: Option<std::path::PathBuf>,
    save: Option<std::path::PathBuf>,
    forget: Option<std::path::PathBuf>,
}

fn sticker_tab(app: &mut App, ui: &mut egui::Ui, palette: &Palette) {
    import_row(app, ui, palette);
    ui.add_space(4.0);
    if app.stickers.is_empty() && app.stickers_saved.is_empty() && app.sticker_packs.is_empty() {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            theme::paragraph(
                ui,
                if app.stickers_pending {
                    "Loading your stickers…"
                } else {
                    "Recent stickers appear here. Right-click one to save it. To import a pack, paste a signal.art link or open a .wastickers file."
                },
                theme::regular(13.0),
                palette.secondary,
            );
        });
        return;
    }
    let saved = app.stickers_saved.clone();
    let packs = app.sticker_packs.clone();
    let recent = app.stickers.clone();
    let mut choices = StickerChoices::default();
    let mut delete_pack = None;
    egui::ScrollArea::vertical()
        .id_salt("sticker-grid")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if !saved.is_empty() {
                theme::text(ui, "Saved", theme::semibold(12.5), palette.secondary);
                sticker_grid(ui, palette, &saved, true, &mut choices);
                ui.add_space(8.0);
            }
            for pack in &packs {
                ui.horizontal(|ui| {
                    theme::text(ui, &pack.name, theme::semibold(12.5), palette.secondary);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::icon_button(
                            ui,
                            Icon::Trash,
                            14.0,
                            palette.dim,
                            palette.danger,
                            "Remove this pack",
                        )
                        .clicked()
                        {
                            delete_pack = Some(pack.dir.clone());
                        }
                    });
                });
                sticker_grid(ui, palette, &pack.stickers, false, &mut choices);
                ui.add_space(8.0);
            }
            if !recent.is_empty() {
                theme::text(ui, "Recent", theme::semibold(12.5), palette.secondary);
                sticker_grid(ui, palette, &recent, false, &mut choices);
            }
        });
    if let Some(path) = choices.send {
        app.actions.push(Action::SendSticker(path));
    }
    if let Some(path) = choices.save {
        app.actions.push(Action::SaveSticker(path));
    }
    if let Some(path) = choices.forget {
        app.actions.push(Action::ForgetSticker(path));
    }
    if let Some(dir) = delete_pack {
        app.actions.push(Action::DeleteStickerPack(dir));
    }
}

/// Imports packs from pasted signal.art links or .wastickers files.
fn import_row(app: &mut App, ui: &mut egui::Ui, palette: &Palette) {
    ui.horizontal(|ui| {
        let spacing = ui.spacing().item_spacing.x;
        let buttons = theme::soft_button_width(ui, "Find packs", true)
            + theme::soft_button_width(ui, "Open file", true)
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
                            egui::RichText::new("Paste a signal.art link")
                                .color(palette.dim)
                                .font(theme::regular(13.0)),
                        )
                        .font(theme::regular(13.0))
                        .text_color(palette.text)
                        .frame(Frame::NONE)
                        .desired_width(ui.available_width() - buttons - 26.0),
                )
            })
            .inner;
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
        if theme::soft_button(ui, palette, Some(Icon::ExternalLink), "Find packs", false)
            .on_hover_text("Browse signalstickers.org")
            .clicked()
        {
            app.actions
                .push(Action::OpenUrl("https://signalstickers.org/".to_owned()));
        }
        if theme::soft_button(ui, palette, Some(Icon::FileText), "Open file", false).clicked() {
            app.actions.push(Action::PickStickerArchive);
        }
    });
    if app.sticker_import_pending {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            theme::spinner(ui, 14.0, palette.accent);
            theme::text(
                ui,
                "Importing the pack…",
                theme::regular(12.5),
                palette.secondary,
            );
        });
    }
}

/// Sticker tiles. Click sends; right-click saves or removes.
fn sticker_grid(
    ui: &mut egui::Ui,
    palette: &Palette,
    stickers: &[std::path::PathBuf],
    saved: bool,
    choices: &mut StickerChoices,
) {
    let columns = 5;
    let gap = 6.0;
    let cell = (ui.available_width() - gap * (columns as f32 - 1.0)) / columns as f32;
    ui.spacing_mut().item_spacing = vec2(gap, gap);
    let menu_width = widgets::menu_width(ui, &["Remove from saved"], true);
    for row in stickers.chunks(columns) {
        ui.horizontal(|ui| {
            for path in row {
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(cell), Sense::click());
                if ui.is_rect_visible(rect) {
                    if response.hovered() {
                        ui.painter().rect_filled(rect, 8.0, palette.surface_hover);
                    }
                    let shown = rect.shrink(4.0);
                    // Animate only the hovered sticker to limit decoder work.
                    let played = response.hovered()
                        && moves(path)
                        && match crate::animation::frame(ui, path, rect) {
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
                        sticker_picture(ui, path, shown);
                    }
                }
                egui::Popup::context_menu(&response)
                    .width(menu_width)
                    .frame(widgets::menu_frame(palette))
                    .show(|ui| {
                        if saved {
                            if widgets::menu_item(ui, palette, Some(Icon::X), "Remove from saved") {
                                choices.forget = Some(path.clone());
                            }
                        } else if widgets::menu_item(
                            ui,
                            palette,
                            Some(Icon::Sticker),
                            "Save sticker",
                        ) {
                            choices.save = Some(path.clone());
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

/// Checks the WebP header for animation.
fn moves(path: &Path) -> bool {
    let mut head = [0u8; 64];
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let Ok(read) = std::io::Read::read(&mut file, &mut head) else {
        return false;
    };
    let head = &head[..read];
    head.len() >= 12
        && &head[0..4] == b"RIFF"
        && &head[8..12] == b"WEBP"
        && head.windows(4).any(|window| window == b"ANIM")
}

fn sticker_picture(ui: &egui::Ui, path: &Path, rect: Rect) {
    egui::Image::new(crate::util::image_uri(path))
        .fit_to_exact_size(rect.size())
        .paint_at(ui, rect);
}
