//! The search pane beside the open chat: a query, an optional day, and the
//! chat's matching messages, newest first. It docks at the right when the
//! window has room for it and a readable conversation, and lies over the
//! conversation otherwise.

use std::ops::Range;

use egui::{Align, Align2, Frame, Key, Layout, Margin, Modifiers, Rect, Sense, Stroke, Vec2};
use egui::{pos2, vec2};
use jiff::civil::Date;

use crate::app::App;
use crate::model::{Action, ChatKind, Message};
use crate::theme::{self, Icon, Palette};
use crate::util;
use crate::{bidi, emoji};

use super::focus::{Stop, TabStop};
use super::widgets;

const CELL: f32 = 30.0;
const MIN_WIDTH: f32 = 300.0;
const MAX_WIDTH: f32 = 520.0;
/// The narrowest conversation a docked pane leaves beside it. Below this the
/// pane lies over the conversation instead of squeezing it.
const CONVERSATION_MIN: f32 = 360.0;
const ROW_HEIGHT: f32 = 60.0;
/// Characters of context kept before a match deep in a long line.
const LEAD: usize = 18;

fn field_id() -> egui::Id {
    egui::Id::new("chat-message-search")
}

/// Docks the pane when there is room, before the conversation is laid out.
/// Otherwise returns the conversation's rect for [`show_overlay`], drawn
/// after the conversation so it lies on top.
pub fn show(app: &mut App, ui: &mut egui::Ui) -> Option<Rect> {
    if !app.chat_search_visible() {
        return None;
    }
    let region = ui.available_rect_before_wrap();
    if region.width() < MIN_WIDTH + CONVERSATION_MIN {
        return Some(region);
    }
    let palette = app.palette;
    let max = (region.width() - CONVERSATION_MIN).clamp(MIN_WIDTH, MAX_WIDTH);
    let wanted = app.settings.search_pane_width.clamp(MIN_WIDTH, max);
    let id = egui::Id::new("chat-search-pane");
    // egui would remember a width a narrow window squeezed; the saved width
    // is the reader's, so it comes back when the window widens again.
    ui.ctx()
        .data_mut(|data| data.remove::<egui::PanelState>(id));
    let response = egui::Panel::right(id)
        .resizable(true)
        .default_size(wanted)
        .size_range(MIN_WIDTH..=max)
        .show_separator_line(false)
        .frame(Frame::new().fill(palette.panel).inner_margin(Margin::ZERO))
        .show(ui, |ui| search(app, ui, false));
    let rect = response.response.rect;
    ui.painter().vline(
        rect.left(),
        rect.y_range(),
        Stroke::new(1.0, palette.outline),
    );
    // Only a drag moves the edge off the width asked for.
    if (rect.width() - wanted).abs() > 1.0 {
        app.settings.search_pane_width = rect.width();
        app.actions.push(Action::SettingsChanged);
    }
    None
}

/// The pane over the right of a conversation too narrow to share.
pub fn show_overlay(app: &mut App, ctx: &egui::Context, region: Rect) {
    let palette = app.palette;
    let width = app
        .settings
        .search_pane_width
        .clamp(MIN_WIDTH, MAX_WIDTH)
        .min(region.width());
    let rect = Rect::from_min_max(pos2(region.right() - width, region.top()), region.max);
    egui::Area::new(egui::Id::new("chat-search-overlay"))
        .order(egui::Order::Middle)
        .fixed_pos(rect.min)
        .constrain(false)
        .show(ctx, |ui| {
            ui.set_clip_rect(rect.expand2(vec2(24.0, 0.0)));
            Frame::new()
                .fill(palette.panel)
                .shadow(egui::epaint::Shadow {
                    offset: [-4, 0],
                    blur: 16,
                    spread: 0,
                    color: palette.shadow,
                })
                .show(ui, |ui| {
                    ui.set_min_size(rect.size());
                    ui.set_max_size(rect.size());
                    search(app, ui, true);
                });
            ui.painter().vline(
                rect.left(),
                rect.y_range(),
                Stroke::new(1.0, palette.outline),
            );
        });
}

/// The pane's contents. Over a narrow conversation (`overlay`) a picked
/// result folds the pane away, so the message it jumps to can be seen;
/// the search comes back as it was with the header's Search.
fn search(app: &mut App, ui: &mut egui::Ui, overlay: bool) {
    let palette = app.palette;
    let (scroll_to, mut picked) = keyboard(app, ui);
    let mut calendar_button = Rect::NOTHING;
    Frame::new()
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_min_height(32.0);
                let close = crate::i18n::gettext(app.locale, "Close search");
                if theme::icon_button(ui, Icon::X, 18.0, palette.secondary, palette.text, &close)
                    .tab_stop(Stop::ChatSearchClose)
                    .clicked()
                {
                    app.actions.push(Action::CloseChatSearch);
                }
                ui.add_space(6.0);
                theme::text(
                    ui,
                    crate::i18n::gettext(app.locale, "Search messages"),
                    theme::semibold(16.0),
                    palette.text,
                );
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.set_min_height(36.0);
                let calendar = theme::icon_button(
                    ui,
                    Icon::Calendar,
                    18.0,
                    if app.chat_search_day.is_some() || app.chat_search_calendar {
                        palette.accent
                    } else {
                        palette.secondary
                    },
                    palette.text,
                    &crate::i18n::gettext(app.locale, "Filter by date"),
                )
                .tab_stop(Stop::ChatSearchDate);
                calendar_button = calendar.rect;
                if calendar.clicked() {
                    app.chat_search_calendar = !app.chat_search_calendar;
                    if app.chat_search_calendar {
                        app.chat_search_month = app.chat_search_day.unwrap_or_else(util::today);
                    }
                }
                ui.add_space(6.0);
                let width = ui.available_width();
                let mut text = app.chat_search.clone();
                let response = widgets::search_field(
                    ui,
                    &palette,
                    field_id(),
                    &mut text,
                    &crate::i18n::gettext(app.locale, "Search"),
                    width,
                )
                .tab_stop(Stop::ChatSearchField);
                if text != app.chat_search {
                    app.actions.push(Action::ChatSearch(text));
                }
                if app.focus_chat_search {
                    app.focus_chat_search = false;
                    response.request_focus();
                }
            });
            if let Some(day) = app.chat_search_day {
                ui.add_space(8.0);
                day_chip(app, ui, &palette, day);
            }
        });
    if app.chat_search_calendar {
        calendar_popup(app, ui, &palette, calendar_button);
    }
    ui.painter().hline(
        ui.max_rect().x_range(),
        ui.cursor().top(),
        Stroke::new(1.0, palette.outline),
    );
    if app.chat_search.trim().is_empty() && app.chat_search_day.is_none() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            // The whole sentence is translated, with the chat's name filled
            // in: word order differs between languages.
            let title = app
                .current_chat()
                .map(|chat| app.chat_title(chat))
                .unwrap_or_default();
            let hint =
                crate::i18n::gettext(app.locale, "Search messages with {}").replace("{}", &title);
            let line = widgets::line(
                ui,
                &hint,
                theme::regular(13.5),
                palette.dim,
                ui.available_width() - 28.0,
                2,
            );
            let (rect, _) = ui.allocate_exact_size(line.size(), Sense::hover());
            line.paint(ui, rect.min, palette.dim);
        });
        return;
    }
    if app.chat_search_hits.is_empty() {
        if app.chat_search_pending {
            ui.add_space(32.0);
            ui.vertical_centered(|ui| theme::spinner(ui, 22.0, palette.secondary));
        } else {
            widgets::empty_state(
                ui,
                &palette,
                Icon::Search,
                &crate::i18n::gettext(app.locale, "No messages found"),
                &crate::i18n::gettext(app.locale, "Try another word or pick a different day."),
            );
        }
        return;
    }
    // Taken for the frame rather than cloned: the rows only push actions.
    let hits = std::mem::take(&mut app.chat_search_hits);
    let query = app.chat_search.trim().to_owned();
    egui::ScrollArea::vertical()
        .id_salt("chat-search-hits")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (index, hit) in hits.iter().enumerate() {
                let selected = app.chat_search_selected == Some(index);
                if hit_row(app, ui, hit, &query, selected, scroll_to == Some(index)) {
                    app.chat_search_selected = Some(index);
                    picked = true;
                }
            }
            if app.chat_search_truncated {
                // The list is capped, so say so rather than dropping the rest
                // without a word.
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    let notice = crate::i18n::gettext(
                        app.locale,
                        "Only the newest {} matches are listed. Narrow the search or pick a day.",
                    )
                    .replace("{}", &hits.len().to_string());
                    let line = widgets::line(
                        ui,
                        &notice,
                        theme::regular(12.0),
                        palette.dim,
                        ui.available_width() - 14.0,
                        3,
                    );
                    let (rect, _) = ui.allocate_exact_size(line.size(), Sense::hover());
                    line.paint(ui, rect.min, palette.dim);
                });
            }
            ui.add_space(8.0);
        });
    app.chat_search_hits = hits;
    if picked && overlay {
        // Folded, not closed: the query, day and results wait for the
        // header's Search or Ctrl+F.
        app.chat_search_open = false;
        app.chat_search_calendar = false;
    }
}

/// Walks the results from the search field: the arrows move through them
/// and Enter opens the one reached, or the newest. Returns the row to bring
/// into view, and whether one was opened.
fn keyboard(app: &mut App, ui: &egui::Ui) -> (Option<usize>, bool) {
    let focused = ui.memory(|memory| memory.has_focus(field_id()));
    if !focused || app.chat_search_calendar {
        return (None, false);
    }
    // Taken before the field sees them: a single-line field would give up
    // its focus on Enter.
    let (down, up, enter) = ui.input_mut(|input| {
        (
            input.consume_key(Modifiers::NONE, Key::ArrowDown),
            input.consume_key(Modifiers::NONE, Key::ArrowUp),
            input.consume_key(Modifiers::NONE, Key::Enter),
        )
    });
    let count = app.chat_search_hits.len();
    if count == 0 {
        return (None, false);
    }
    let before = app.chat_search_selected;
    let mut selected = before;
    if down {
        selected = Some(selected.map_or(0, |index| (index + 1).min(count - 1)));
    }
    if up {
        selected = selected.map(|index| index.saturating_sub(1));
    }
    if enter {
        let index = selected.unwrap_or(0).min(count - 1);
        selected = Some(index);
        let hit = &app.chat_search_hits[index];
        app.actions.push(Action::OpenMessage {
            chat: hit.chat.clone(),
            message: hit.id.clone(),
        });
    }
    app.chat_search_selected = selected;
    (selected.filter(|_| selected != before), enter)
}

/// The picked day: clicking it opens the calendar on it, and the cross
/// beside it clears it.
fn day_chip(app: &mut App, ui: &mut egui::Ui, palette: &Palette, day: Date) {
    ui.horizontal(|ui| {
        let label = util::short_date(app.locale, day);
        let chip = widgets::filter_chip(ui, palette, &label, 0, true);
        let change = crate::i18n::gettext(app.locale, "Filter by date");
        chip.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                true,
                format!("{change}: {}", util::long_date(app.locale, day)),
            )
        });
        if chip
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            app.chat_search_month = day;
            app.chat_search_calendar = true;
        }
        let clear = crate::i18n::gettext(app.locale, "Clear the day filter");
        if theme::icon_button(ui, Icon::X, 14.0, palette.secondary, palette.text, &clear)
            .tab_stop(Stop::ChatSearchDay)
            .clicked()
        {
            app.actions.push(Action::SetChatSearchDay(None));
        }
    });
}

/// The day filter floats under its icon and closes on a click elsewhere.
fn calendar_popup(app: &mut App, ui: &egui::Ui, palette: &Palette, button: Rect) {
    let id = egui::Id::new("chat-search-calendar");
    let width = CELL * 7.0 + 24.0;
    let screen = ui.ctx().content_rect();
    let x = button
        .left()
        .min(screen.right() - width - 8.0)
        .max(screen.left() + 8.0);
    let area = egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(pos2(x, button.bottom() + 4.0))
        .show(ui.ctx(), |ui| {
            widgets::menu_frame(palette)
                .fill(palette.surface)
                .inner_margin(Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(CELL * 7.0);
                    month_header(app, ui, palette);
                    ui.add_space(6.0);
                    month_grid(app, ui, palette);
                });
        });
    let popup = area.response.rect;
    let clicked_outside = ui.ctx().input(|input| {
        input.pointer.button_clicked(egui::PointerButton::Primary)
            && input
                .pointer
                .interact_pos()
                .or(input.pointer.latest_pos())
                .is_some_and(|pos| !popup.contains(pos) && !button.contains(pos))
    });
    if clicked_outside {
        app.chat_search_calendar = false;
    }
}

fn month_header(app: &mut App, ui: &mut egui::Ui, palette: &Palette) {
    let month = app.chat_search_month;
    let current = util::today();
    let latest = (month.year(), month.month()) >= (current.year(), current.month());
    ui.horizontal(|ui| {
        theme::text(
            ui,
            util::month_heading(app.locale, month),
            theme::medium(14.5),
            palette.text,
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // Nothing has been said in a month still to come.
            if !latest
                && theme::icon_button(
                    ui,
                    Icon::ChevronRight,
                    16.0,
                    palette.secondary,
                    palette.text,
                    &crate::i18n::gettext(app.locale, "Next month"),
                )
                .clicked()
            {
                app.chat_search_month = month_step(month, 1);
            }
            if theme::icon_button(
                ui,
                Icon::ChevronLeft,
                16.0,
                palette.secondary,
                palette.text,
                &crate::i18n::gettext(app.locale, "Previous month"),
            )
            .clicked()
            {
                app.chat_search_month = month_step(month, -1);
            }
        });
    });
}

fn month_grid(app: &mut App, ui: &mut egui::Ui, palette: &Palette) {
    let month = app.chat_search_month;
    let today = util::today();
    let block = CELL * 7.0;
    let head = 18.0;
    let height = head + 4.0 + CELL * 6.0;
    let (rect, _) = ui.allocate_exact_size(vec2(block, height), Sense::hover());
    let left = rect.left();
    let top = rect.top() + head + 4.0;
    for (offset, name) in util::weekday_headings(app.locale).iter().enumerate() {
        ui.painter().text(
            pos2(
                left + CELL * offset as f32 + CELL / 2.0,
                rect.top() + head / 2.0,
            ),
            Align2::CENTER_CENTER,
            name,
            theme::regular(11.0),
            palette.dim,
        );
    }
    let first = month.first_of_month();
    let lead = usize::try_from(first.weekday().to_monday_zero_offset()).unwrap_or(0);
    for day in 1..=first.days_in_month() {
        let Ok(date) = Date::new(month.year(), month.month(), day) else {
            continue;
        };
        let index = lead + usize::try_from(day - 1).unwrap_or(0);
        let cell = Rect::from_center_size(
            pos2(
                left + CELL * (index % 7) as f32 + CELL / 2.0,
                top + CELL * (index / 7) as f32 + CELL / 2.0,
            ),
            Vec2::splat(CELL),
        );
        let future = date > today;
        let response = ui.interact(
            cell,
            egui::Id::new(("chat-search-day", month.year(), month.month(), day)),
            if future {
                Sense::hover()
            } else {
                Sense::click()
            },
        );
        let selected = app.chat_search_day == Some(date);
        // Reachable and announced like every other button: the keyboard walks
        // to each day and a screen reader reads the date it picks.
        theme::reveal_focus(&response);
        theme::focus_outline(ui, response.id, cell.shrink(1.0), CELL / 2.0);
        let label = util::long_date(app.locale, date);
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Button, !future, selected, &label)
        });
        let radius = CELL / 2.0 - 1.0;
        if selected {
            ui.painter()
                .circle_filled(cell.center(), radius, palette.accent);
        } else if response.hovered() && !future {
            ui.painter()
                .circle_filled(cell.center(), radius, palette.surface_hover);
        } else if date == today {
            ui.painter().circle_stroke(
                cell.center(),
                radius,
                Stroke::new(1.0, palette.surface_active),
            );
        }
        let colour = if selected {
            palette.on_accent
        } else if future {
            palette.dim
        } else {
            palette.text
        };
        ui.painter().text(
            cell.center(),
            Align2::CENTER_CENTER,
            day.to_string(),
            theme::regular(13.0),
            colour,
        );
        if response.clicked() {
            // A second click on the picked day clears it.
            let next = if selected { None } else { Some(date) };
            app.actions.push(Action::SetChatSearchDay(next));
        }
        if !future {
            response.on_hover_cursor(egui::CursorIcon::PointingHand);
        }
    }
}

fn month_step(month: Date, direction: i32) -> Date {
    let (year, number) = if direction < 0 {
        if month.month() == 1 {
            (month.year() - 1, 12)
        } else {
            (month.year(), month.month() - 1)
        }
    } else if month.month() == 12 {
        (month.year() + 1, 1)
    } else {
        (month.year(), month.month() + 1)
    };
    Date::new(year, number, 1).unwrap_or(month)
}

/// One result: when it was said, and the line that matched, with who said
/// it in a group. Returns whether it was clicked.
fn hit_row(
    app: &mut App,
    ui: &mut egui::Ui,
    hit: &Message,
    query: &str,
    selected: bool,
    scroll_to: bool,
) -> bool {
    let palette = app.palette;
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    if scroll_to {
        response.scroll_to_me(None);
    }
    theme::reveal_focus(&response);
    theme::focus_outline(ui, response.id, rect.shrink(2.0), 6.0);
    let stamp = util::chat_stamp(app.locale, hit.timestamp);
    // The preview comes from the line the query matched: the archive searches
    // the whole text, so a hit on a later line would otherwise preview a first
    // line the query is nowhere in.
    let line = hit.text_matching(query).unwrap_or_else(|| hit.summary());
    let line = crate::markup::plain(&app.resolve_mention_tokens(&line), &[]);
    let (snippet, found) = snippet(&line, query);
    let who = if hit.from_me {
        None
    } else if ChatKind::from_id(&hit.chat) == ChatKind::Group {
        let sender = app.display_name_or(&hit.sender, hit.sender_name.as_deref());
        Some(
            sender
                .split_whitespace()
                .next()
                .unwrap_or(&sender)
                .to_owned(),
        )
    } else {
        None
    };
    response.widget_info(|| {
        let text = match &who {
            Some(who) => format!("{stamp}, {who}: {line}"),
            None => format!("{stamp}, {line}"),
        };
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, text)
    });
    if ui.is_rect_visible(rect) {
        if selected {
            ui.painter().rect_filled(rect, 0.0, palette.surface_active);
        } else if response.hovered() {
            ui.painter().rect_filled(rect, 0.0, palette.surface_hover);
        }
        let left = rect.left() + 16.0;
        let right = rect.right() - 16.0;
        ui.painter().text(
            pos2(left, rect.top() + 10.0),
            Align2::LEFT_TOP,
            &stamp,
            theme::regular(11.5),
            palette.dim,
        );
        let line_y = rect.top() + 30.0;
        let mut x = left;
        if hit.from_me {
            let ticks = Rect::from_center_size(pos2(x + 8.0, line_y + 9.0), Vec2::splat(16.0));
            widgets::ticks(ui, &palette, ticks, hit.status);
            x += 20.0;
        } else if let Some(who) = &who {
            let name = widgets::line(
                ui,
                &format!("{who}: "),
                theme::regular(13.5),
                palette.dim,
                (right - x) * 0.5,
                1,
            );
            name.paint(ui, pos2(x, line_y), palette.dim);
            x += name.size().x;
        }
        paint_snippet(
            ui,
            pos2(x, line_y),
            (right - x).max(0.0),
            &snippet,
            found,
            &palette,
        );
        ui.painter().hline(
            left..=rect.right(),
            rect.bottom() - 0.5,
            Stroke::new(1.0, palette.outline),
        );
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        app.actions.push(Action::OpenMessage {
            chat: hit.chat.clone(),
            message: hit.id.clone(),
        });
        return true;
    }
    false
}

/// The part of `line` a one-line preview shows, and where `query` sits in
/// it. A match deep in a long line keeps a little context before it behind
/// an ellipsis, so it is not cut off at the end.
fn snippet(line: &str, query: &str) -> (String, Option<Range<usize>>) {
    let Some(found) = find_ignoring_case(line, query) else {
        return (line.to_owned(), None);
    };
    let before = line[..found.start].chars().count();
    if before <= LEAD {
        return (line.to_owned(), Some(found));
    }
    // Start at a word within the lead when there is one.
    let skip = before - LEAD;
    let mut start = line
        .char_indices()
        .nth(skip)
        .map_or(found.start, |(index, _)| index);
    if let Some(space) = line[start..found.start].find(char::is_whitespace) {
        start += space;
    }
    let kept = line[start..].trim_start();
    let start = line.len() - kept.len();
    let ellipsis = '…'.len_utf8();
    let found = found.start - start + ellipsis..found.end - start + ellipsis;
    (format!("…{kept}"), Some(found))
}

/// Where `needle` first appears in `text`, compared in lower case, as a
/// byte range of `text`. Lowercasing can change a letter's length, so the
/// comparison walks `text` instead of searching a lowercased copy.
fn find_ignoring_case(text: &str, needle: &str) -> Option<Range<usize>> {
    let needle = needle.trim().to_lowercase();
    if needle.is_empty() {
        return None;
    }
    for (start, _) in text.char_indices() {
        let mut lowered = String::new();
        for (offset, letter) in text[start..].char_indices() {
            lowered.extend(letter.to_lowercase());
            if !needle.starts_with(&lowered) {
                break;
            }
            if lowered.len() == needle.len() {
                return Some(start..start + offset + letter.len_utf8());
            }
        }
    }
    None
}

/// One line of preview with the match in bold, colour emoji, and
/// right-to-left runs in reading order.
fn paint_snippet(
    ui: &egui::Ui,
    pos: egui::Pos2,
    width: f32,
    text: &str,
    found: Option<Range<usize>>,
    palette: &Palette,
) {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = width;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let plain = egui::TextFormat::simple(theme::regular(13.5), palette.secondary);
    let strong = egui::TextFormat::simple(theme::semibold(13.5), palette.text);
    let mut placements = Vec::new();
    match found {
        Some(found) => {
            for (part, format) in [
                (&text[..found.start], &plain),
                (&text[found.clone()], &strong),
                (&text[found.end..], &plain),
            ] {
                if !part.is_empty() {
                    emoji::append(ui, &mut job, &mut placements, part, format);
                }
            }
        }
        None => {
            emoji::append(ui, &mut job, &mut placements, text, &plain);
        }
    }
    let galley = bidi::layout_job(ui, job);
    ui.painter().galley(pos, galley.clone(), palette.secondary);
    emoji::paint(ui, &galley, pos, &placements);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months_step_across_the_year_boundary() {
        let december = Date::new(2026, 12, 1).expect("a date");
        assert_eq!(month_step(december, 1).month(), 1);
        assert_eq!(month_step(december, 1).year(), 2027);
        let january = Date::new(2026, 1, 1).expect("a date");
        assert_eq!(month_step(january, -1).month(), 12);
        assert_eq!(month_step(january, -1).year(), 2025);
    }

    #[test]
    fn a_match_is_found_whatever_its_case_and_letters() {
        assert_eq!(find_ignoring_case("The Engine room", "engine"), Some(4..10));
        assert_eq!(find_ignoring_case("ÜBER alles", "über"), Some(0..5));
        // "İ" lowercases to two characters; the range still covers the
        // original letter.
        assert_eq!(find_ignoring_case("İstanbul", "i\u{307}s"), Some(0..3));
        assert_eq!(find_ignoring_case("مرحبا بالعالم", "بالعالم"), Some(11..25));
        assert_eq!(find_ignoring_case("nothing here", "engine"), None);
        assert_eq!(find_ignoring_case("anything", "  "), None);
    }

    #[test]
    fn a_match_deep_in_a_long_line_keeps_a_little_context() {
        let line = "We talked for a very long time about everything before the engine failed";
        let (shown, found) = snippet(line, "engine");
        let found = found.expect("a match");
        assert!(shown.starts_with('…'), "{shown}");
        assert_eq!(&shown[found.clone()], "engine");
        let lead = shown[..found.start].chars().count();
        assert!(lead <= LEAD + 1, "{lead} characters before the match");
        assert_eq!(shown, "…before the engine failed", "from a word");
        // A match near the start shows the line as it is.
        assert_eq!(
            snippet("The engine failed", "engine"),
            ("The engine failed".to_owned(), Some(4..10))
        );
        // Emoji and right-to-left text keep their characters whole.
        let (shown, found) = snippet("🎉🎉🎉🎉🎉🎉🎉🎉🎉🎉🎉🎉🎉🎉🎉🎉🎉🎉🎉🎉 party 🎉", "party");
        assert_eq!(&shown[found.expect("a match")], "party");
        let (shown, found) = snippet("שלום לכולם, היום נדבר הרבה מאוד על המנוע החדש", "המנוע");
        assert_eq!(&shown[found.expect("a match")], "המנוע");
    }
}
