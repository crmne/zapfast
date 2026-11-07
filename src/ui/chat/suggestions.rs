//! Autocomplete popups for :emoji and @mention triggers.

use super::*;

pub struct EmojiSuggestions;

impl EmojiSuggestions {
    pub fn show(app: &mut App, ui: &mut egui::Ui, field: egui::Id) {
        emoji_suggestions(app, ui, field);
    }
}

pub struct MentionPicker;

impl MentionPicker {
    pub fn show(app: &mut App, ui: &mut egui::Ui, chat: &Chat, field: egui::Id) {
        mention_picker(app, ui, chat, field);
    }
}

pub(crate) fn standalone_trigger(text: &str, cursor: usize, trigger: char) -> Option<usize> {
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
pub(crate) fn active_mention(text: &str, start: Option<usize>, cursor: usize) -> Option<(usize, &str)> {
    let start = start?;
    let end = text
        .char_indices()
        .nth(cursor)
        .map_or(text.len(), |(at, _)| at);
    let query = text.get(start.checked_add(1)?..end)?;
    (!query.contains(['@', '\n'])).then_some((end, query))
}

pub(crate) fn active_emoji(text: &str, start: Option<usize>, cursor: usize) -> Option<(usize, &str)> {
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
pub(crate) struct EmojiSuggestion {
    pub(crate) emoji: &'static str,
    pub(crate) shortcode: String,
    pub(crate) name: &'static str,
}

pub(crate) fn emoji_match_score(emoji: &emojis::Emoji, query: &str) -> Option<u8> {
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

pub(crate) fn emoji_suggestion(emoji: &'static emojis::Emoji) -> EmojiSuggestion {
    let shortcode = emoji
        .shortcode()
        .map_or_else(|| emoji.name().replace([' ', '-'], "_"), str::to_owned);
    EmojiSuggestion {
        emoji: emoji.as_str(),
        shortcode: format!(":{shortcode}:"),
        name: emoji.name(),
    }
}

pub(crate) fn emoji_candidates(app: &App, query: &str) -> Vec<EmojiSuggestion> {
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

pub(crate) fn take_plain_key(ui: &mut egui::Ui, key: Key) -> bool {
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

pub(crate) fn emoji_suggestions(app: &mut App, ui: &mut egui::Ui, field: egui::Id) {
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

    let list = widgets::raised(ui, &palette, suggestion_frame(&palette), |ui| {
        let row_height = 36.0;
        ui.spacing_mut().item_spacing.y = 0.0;
        for (index, candidate) in candidates.iter().enumerate() {
            let (rect, response) =
                ui.allocate_exact_size(vec2(ui.available_width(), row_height), Sense::click());
            if index == app.emoji_selected {
                ui.painter().rect_filled(
                    rect,
                    SUGGESTION_ROW_RADIUS,
                    palette.accent.gamma_multiply(0.18),
                );
                ui.painter().rect_stroke(
                    rect,
                    SUGGESTION_ROW_RADIUS,
                    Stroke::new(1.0, palette.accent),
                    egui::StrokeKind::Inside,
                );
            } else if response.hovered() {
                ui.painter()
                    .rect_filled(rect, SUGGESTION_ROW_RADIUS, palette.surface_hover);
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
    ui.ctx()
        .data_mut(|data| data.insert_temp(suggestion_list_id(), list.response.rect));
    strip_gap(ui);
    if let Some(candidate) = picked {
        app.actions.push(Action::InsertEmojiCompletion {
            emoji: candidate.emoji.to_owned(),
            start,
            end,
        });
    }
}

/// Group-member suggestions above the composer.
pub(crate) fn mention_picker(app: &mut App, ui: &mut egui::Ui, chat: &Chat, field: egui::Id) {
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
    let everyone_alias = crate::mentions::everyone_alias(app.locale);

    let list = widgets::raised(ui, &palette, suggestion_frame(&palette), |ui| {
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
                        ui.painter().rect_filled(
                            rect,
                            SUGGESTION_ROW_RADIUS,
                            palette.surface_hover,
                        );
                    }
                    let avatar = Rect::from_center_size(
                        pos2(rect.left() + 19.0, rect.center().y),
                        Vec2::splat(28.0),
                    );
                    let everyone = id == crate::mentions::ALL_ID;
                    if everyone {
                        ui.painter().circle_filled(
                            avatar.center(),
                            avatar.width() / 2.0,
                            palette.avatar(crate::util::hue(id)),
                        );
                        theme::paint_icon(
                            ui,
                            Icon::Users,
                            avatar,
                            avatar.width() * 0.5,
                            Color32::WHITE,
                        );
                    } else {
                        let picture = app.avatar(id);
                        widgets::paint_avatar(
                            ui,
                            &palette,
                            avatar,
                            label.trim_start_matches('~'),
                            id,
                            picture.as_deref(),
                        );
                    }
                    let detail = if everyone
                        && !everyone_alias.eq_ignore_ascii_case(crate::mentions::ALL_USER)
                    {
                        everyone_alias.clone()
                    } else {
                        crate::model::phone_of(id)
                            .map(crate::util::phone)
                            .unwrap_or_default()
                    };
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
    ui.ctx()
        .data_mut(|data| data.insert_temp(suggestion_list_id(), list.response.rect));
    strip_gap(ui);
    if let Some((id, name)) = picked {
        app.actions.push(Action::InsertMention {
            id,
            name,
            start,
            end,
        });
    }
}
