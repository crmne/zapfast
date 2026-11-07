//! Message transcript list, scroll management, albums, and selection leash.

use super::*;

pub struct MessageList;

impl MessageList {
    pub fn show(app: &mut App, ui: &mut egui::Ui, chat: &Chat, under: f32) {
        messages(app, ui, chat, under);
    }
}


/// How far `message`'s attachment download has got, once the worker has said.
pub(crate) fn download_fraction(view: &View<'_>, message: &Message) -> Option<f32> {
    let percent = view.download_progress?.get(&message.id)?;
    Some(f32::from(*percent) / 100.0)
}

/// Protocol album parent carried by pictures and ordinary videos.
pub(crate) fn album_id(message: &Message) -> Option<&str> {
    match &message.content {
        Content::Image { media, .. }
        | Content::Video {
            media, note: false, ..
        } => media.album.as_deref(),
        _ => None,
    }
}

pub(crate) const LOOSE_ALBUM_MIN: usize = 4;

pub(crate) fn loose_picture(message: &Message) -> bool {
    matches!(
        &message.content,
        Content::Image { caption: None, media, .. } if media.album.is_none()
    ) && message.quoted.is_none()
}

pub(crate) fn loose_run(messages: &[Message], index: usize) -> Option<std::ops::Range<usize>> {
    let message = messages
        .get(index)
        .filter(|message| loose_picture(message))?;
    let joins = |other: &Message| {
        loose_picture(other)
            && other.from_me == message.from_me
            && other.sender == message.sender
            && crate::util::day_key(other.timestamp) == crate::util::day_key(message.timestamp)
    };
    let before = messages[..index]
        .iter()
        .rev()
        .take_while(|m| joins(m))
        .count();
    let after = messages[index + 1..]
        .iter()
        .take_while(|m| joins(m))
        .count();
    let run = index - before..index + after + 1;
    (run.len() >= LOOSE_ALBUM_MIN).then_some(run)
}

pub(crate) fn album_continues(messages: &[Message], index: usize) -> bool {
    let Some(previous) = index.checked_sub(1).and_then(|before| messages.get(before)) else {
        return false;
    };
    let message = &messages[index];
    if loose_run(messages, index).is_some_and(|run| run.start < index) {
        return true;
    }
    album_id(message).is_some()
        && album_id(previous) == album_id(message)
        && previous.from_me == message.from_me
        && previous.sender == message.sender
        && crate::util::day_key(previous.timestamp) == crate::util::day_key(message.timestamp)
}

pub fn row_starts(messages: &[Message], selecting: bool) -> Vec<usize> {
    (0..messages.len())
        .filter(|&index| selecting || !album_continues(messages, index))
        .collect()
}

/// Which of `starts`' rows draws the message at `index`.
pub fn row_of(starts: &[usize], index: usize) -> usize {
    match starts.binary_search(&index) {
        Ok(row) => row,
        Err(row) => row.saturating_sub(1),
    }
}

pub(crate) fn album_run(messages: &[Message], start: usize) -> Option<&[Message]> {
    let first = messages.get(start)?;
    if let Some(run) = loose_run(messages, start) {
        return (run.start == start).then(|| &messages[run]);
    }
    let album = album_id(first)?;
    let mut end = start + 1;
    while let Some(message) = messages.get(end) {
        if album_id(message) != Some(album)
            || message.from_me != first.from_me
            || message.sender != first.sender
            || crate::util::day_key(message.timestamp) != crate::util::day_key(first.timestamp)
        {
            break;
        }
        end += 1;
    }
    (end > start + 1).then_some(&messages[start..end])
}

pub(crate) fn estimated_height(message: &Message, width: f32, new_day: bool, notices: usize) -> f32 {
    let bubble = ((width * 0.72).min(560.0) - 20.0).max(40.0);
    let text_rows = |text: &str| {
        let per_row = bubble / 7.5;
        (text.chars().count() as f32 / per_row).ceil().max(1.0)
    };
    let caption_rows = |caption: &Option<String>| {
        caption
            .as_deref()
            .map_or(0.0, |caption| text_rows(caption) * 19.0)
    };
    let body = match &message.content {
        Content::Text { text, preview } => {
            text_rows(text) * 19.0 + if preview.is_some() { 60.0 } else { 0.0 }
        }
        Content::Interactive { text, .. } => text_rows(text) * 19.0 + 60.0,
        Content::Image { caption, .. } | Content::Video { caption, .. } => {
            200.0 + caption_rows(caption)
        }
        Content::Document { caption, .. } => 70.0 + caption_rows(caption),
        Content::Sticker { .. } => 140.0,
        Content::Audio { .. } => 60.0,
        Content::DisappearingTimer { .. } => 0.0,
        _ => 40.0,
    };
    40.0 + body + if new_day { 36.0 } else { 0.0 } + notices as f32 * PIN_NOTICE_HEIGHT
}

pub(crate) fn pin_notices_in_time_order(rows: &[crate::archive::Pinned]) -> Vec<crate::archive::Pinned> {
    let mut sorted = rows.to_vec();
    sorted.sort_by_key(|row| row.pinned_at);
    sorted
}

pub(crate) fn notices_above(notices: &[crate::archive::Pinned], timestamp: i64) -> usize {
    notices
        .iter()
        .take_while(|notice| notice.pinned_at <= timestamp)
        .count()
}

pub(crate) fn pin_notice(
    ui: &mut egui::Ui,
    palette: &Palette,
    locale: crate::i18n::Locale,
    names: &dyn Fn(&str, Option<&str>) -> String,
    row: &crate::archive::Pinned,
) {
    let label = pin_notice_label(locale, row, names);
    ui.vertical_centered(|ui| {
        widgets::chip(ui, palette, &label);
    });
}

pub(crate) fn pin_notice_label(
    locale: crate::i18n::Locale,
    row: &crate::archive::Pinned,
    names: &dyn Fn(&str, Option<&str>) -> String,
) -> String {
    if row.pinner.by_me {
        return crate::i18n::gettext(locale, "You pinned a message").to_string();
    }
    if row.pinner.sender.is_empty() {
        return crate::i18n::gettext(locale, "A message was pinned").to_string();
    }
    crate::i18n::gettext(locale, "{} pinned a message")
        .replace("{}", &names(&row.pinner.sender, None))
}

pub(crate) fn live_pins(rows: &[crate::archive::Pinned], now: i64) -> Vec<crate::archive::Pinned> {
    rows.iter()
        .filter(|row| row.expires_at > now)
        .cloned()
        .collect()
}

/// What one pin notice takes in the transcript: the chip and its spacing.
pub(crate) const PIN_NOTICE_HEIGHT: f32 = 32.0;

pub(crate) fn shows_sender_pictures(chat: &Chat) -> bool {
    chat.is_group()
}

/// How long the sticky date stays after the transcript stops moving.
pub(crate) const STICKY_DATE_HOLD: f64 = 1.0;
/// How long the sticky date takes to fade out.
pub(crate) const STICKY_DATE_FADE: f32 = 0.25;

pub(crate) fn sticky_date<R>(
    ui: &mut egui::Ui,
    palette: &Palette,
    locale: crate::i18n::Locale,
    chat: &Chat,
    output: &egui::scroll_area::ScrollAreaOutput<R>,
    visible_day: Option<(i64, f32)>,
) {
    let id = egui::Id::new(("visible-message-date", &chat.id));
    let ctx = ui.ctx().clone();
    let now = ui.input(|input| input.time);
    let offset = output.state.offset.y;
    let viewport = output.inner_rect;
    let at_bottom = offset + viewport.height() >= output.content_size.y - 24.0;
    let (last_offset, was_at_bottom, mut moved_at) = ctx
        .data(|data| data.get_temp::<(f32, bool, f64)>(id))
        .unwrap_or((offset, at_bottom, f64::NEG_INFINITY));
    if (offset - last_offset).abs() > 0.5 && !(at_bottom && was_at_bottom) {
        moved_at = now;
    }
    ctx.data_mut(|data| data.insert_temp(id, (offset, at_bottom, moved_at)));
    let since = now - moved_at;
    let scrolling = since < STICKY_DATE_HOLD;
    if scrolling {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(STICKY_DATE_HOLD - since));
    }
    // It appears at once when scrolling starts and fades only on the way out.
    let fade = if scrolling { 0.0 } else { STICKY_DATE_FADE };
    let opacity = ctx.animate_bool_with_time(id, scrolling, fade);
    let Some((timestamp, separator_top)) = visible_day else {
        return;
    };
    if opacity <= 0.0 || separator_top >= viewport.top() {
        return;
    }
    let mut overlay = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id)
            .max_rect(viewport.shrink2(vec2(0.0, 8.0)))
            .layout(Layout::top_down(Align::Center)),
    );
    overlay.set_clip_rect(viewport);
    overlay.multiply_opacity(opacity);
    widgets::chip(
        &mut overlay,
        palette,
        &crate::util::day_label(locale, timestamp),
    );
}

#[derive(Clone)]
pub(crate) struct TextSelectionRows {
    anchor: String,
    extent: String,
}

pub(crate) fn messages(app: &mut App, ui: &mut egui::Ui, chat: &Chat, under: f32) {
    let palette = app.palette;
    let pending_scroll = app.scroll_page.take();
    let carried = app.scroll_route.take(crate::app::ScrollPane::Messages);
    let scroll_forced = std::mem::take(&mut app.scroll_to_bottom_forced);
    // The message list's one-shot requests, taken for the same reason.
    let list_id = messages_list_id(&chat.id);
    let keyboard = ui.ctx().data(|data| {
        data.get_temp::<bool>(theme::keyboard_focus_id())
            .unwrap_or(false)
    });
    let composer_fallback = app.focus_composer
        && keyboard
        && !chat.can_send()
        && app.selection.is_none()
        && app.image_preview.is_none()
        && app.dialog.is_none();
    if composer_fallback || (!chat.can_send() && !keyboard) {
        app.focus_composer = false;
    }
    let focus_list = if ui.is_visible() {
        let deferred = app
            .list_focus_deferred
            .take()
            .is_some_and(|deferred| deferred == chat.id);
        std::mem::take(&mut app.focus_messages) || composer_fallback || deferred
    } else {
        if composer_fallback {
            app.list_focus_deferred = Some(chat.id.clone());
        }
        false
    };
    let mut reveal = std::mem::take(&mut app.reveal_cursor);
    // Check out the conversation while drawing rows and collecting actions.
    let mut conversation = app.conversations.remove(&chat.id).unwrap_or_default();
    // The list's items are its rows: an album is one.
    let starts = row_starts(&conversation.messages, app.selecting_in(&chat.id));
    let total = starts.len();
    let cursor = app
        .message_cursor
        .get(&chat.id)
        .and_then(|id| {
            conversation
                .messages
                .iter()
                .position(|message| &message.id == id)
        })
        .map(|index| starts[row_of(&starts, index)])
        .or(starts.last().copied());
    let cursor_id = cursor.map(|index| conversation.messages[index].id.clone());
    let typing = app.typing_in(&chat.id);
    let mut avatars = HashMap::new();
    let mut senders = HashSet::new();
    if shows_sender_pictures(chat) {
        senders.extend(
            conversation
                .messages
                .iter()
                .filter(|message| !message.from_me)
                .map(|message| message.sender.clone()),
        );
        // A typing participant may not have a visible message.
        senders.extend(typing.iter().map(|(id, _, _)| id.clone()));
    }
    senders.extend(
        conversation
            .messages
            .iter()
            .flat_map(|message| &message.reactions)
            .map(|reaction| {
                if reaction.from_me {
                    app.me.clone().unwrap_or_else(|| reaction.sender.clone())
                } else {
                    reaction.sender.clone()
                }
            }),
    );
    for sender in senders {
        let picture = app.avatar(&sender);
        avatars.insert(sender, picture);
    }
    let names_or = |id: &str, hint: Option<&str>| app.display_name_or(id, hint);
    let mention_names = |id: &str| app.mention_name(id);
    let keyboard_navigation = std::cell::Cell::new(false);
    let list_label =
        || crate::a11y::messages_list_label(app.locale, &app.chat_title(chat), chat.is_group());
    let view = View {
        palette,
        locale: app.locale,
        chat,
        me: app.me.as_deref(),
        auto_download: app.account().settings.auto_download,
        connected: app.link.is_connected(),
        account_receipts_off: app.account_receipts_off,
        poll_voting: &app.poll_voting,
        interactive_pending: &app.interactive_sending,
        selecting: app
            .selection
            .as_ref()
            .is_some_and(|(selected_chat, _)| *selected_chat == chat.id),
        anchor: if conversation.loading_older || conversation.fetching_phone {
            None
        } else {
            app.scroll_anchor.as_deref()
        },
        open_menu: app
            .open_message_menu
            .as_ref()
            .filter(|(menu_chat, _)| *menu_chat == chat.id)
            .map(|(_, message)| message.as_str()),
        starred: app.stars.get(&chat.id),
        pins: app.pins.get(&chat.id),
        pin_notices: app
            .chat_pins
            .get(&chat.id)
            .map_or(&[][..], |rows| rows.as_slice()),
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
        contacts: &app.contacts,
        now: crate::util::now(),
        animate: app.window_focused,
        motion_playing: app
            .motion_playing
            .as_ref()
            .filter(|(playing, _)| *playing == chat.id)
            .map(|(_, message)| message.as_str()),
        player: &app.player,
        voice_speed: app.voice_speed_in(Some(&chat.id)),
        video: &app.video,
        copy_rows: (!app.selection.as_ref().is_some_and(|(id, _)| *id == chat.id))
            .then_some(app.copy_rows.as_ref()),
        download_progress: app.download_progress.get(&chat.id),
        arrivals: &conversation.arrivals,
    };
    let mut actions = Vec::new();
    let mut anchored = false;
    // The first unread message is the count-th incoming one from the end.
    let divider = app
        .unread_divider
        .as_ref()
        .filter(|divider| divider.chat == chat.id)
        .and_then(|divider| {
            unread_divider_message(&conversation.messages, divider.count)
                .map(|message| (message.id.clone(), divider.count, divider.placed))
        });
    let mut divider_placed = false;
    let selection: Option<Vec<String>> = app
        .selection
        .as_ref()
        .filter(|(selected_chat, _)| *selected_chat == chat.id)
        .map(|(_, ids)| ids.clone());
    let sweep = app
        .sweep
        .as_ref()
        .filter(|sweep| sweep.chat == chat.id)
        .map(|sweep| (sweep.anchor.clone(), sweep.to.clone()));
    if app.sweep.is_some() && (sweep.is_none() || !ui.input(|input| input.pointer.primary_down())) {
        actions.push(Action::EndSweep);
    }
    let sweep = sweep.filter(|_| ui.input(|input| input.pointer.primary_down()));
    let sweep_pointer = ui.input(|input| input.pointer.latest_pos());
    let mut swept_to: Option<&str> = None;
    let mut first_row: Option<&str> = None;
    let scroll_to_bottom =
        app.scroll_to_bottom && divider.as_ref().is_none_or(|(.., placed)| *placed);
    // The message a quote or search result jumped to flashes once in view.
    let jump = app
        .jump_highlight
        .clone()
        .filter(|jump| jump.chat == chat.id);
    let jump_since = std::cell::Cell::new(jump.as_ref().and_then(|jump| jump.since));
    let time = ui.input(|input| input.time);
    let mut edge_scrolled_up = false;
    let text_selection_id =
        ui.make_persistent_id(("message-text-selection", &app.account().id, &chat.id));
    let has_text_selection = ui
        .ctx()
        .plugin_opt::<egui::text_selection::LabelSelectionState>()
        .is_some_and(|plugin| plugin.lock().has_selection());
    let (pressed, extending, drag_frame, origin) = ui.input(|input| {
        (
            input.pointer.primary_pressed(),
            input.modifiers.shift,
            (input.pointer.primary_down() || input.pointer.primary_released()),
            input.pointer.press_origin(),
        )
    });
    let mut text_selection = ui
        .ctx()
        .data(|data| data.get_temp::<TextSelectionRows>(text_selection_id));
    if !has_text_selection || (pressed && !extending) {
        text_selection = None;
    }
    if pressed
        && let Some(origin) =
            origin.filter(|origin| ui.available_rect_before_wrap().contains(*origin))
        && let Some(row) = conversation.messages.iter().find(|row| {
            ui.ctx().data(|data| {
                data.get_temp::<Rect>(text_selection_id.with(&row.id))
                    .is_some_and(|rect| rect.contains(origin))
            })
        })
    {
        match &mut text_selection {
            Some(span) if extending => span.extent.clone_from(&row.id),
            _ => {
                text_selection = Some(TextSelectionRows {
                    anchor: row.id.clone(),
                    extent: row.id.clone(),
                })
            }
        }
    }
    let selected_rows = text_selection.as_ref().and_then(|span| {
        let anchor = conversation
            .messages
            .iter()
            .position(|row| row.id == span.anchor)?;
        let extent = conversation
            .messages
            .iter()
            .position(|row| row.id == span.extent)?;
        Some((anchor, anchor.min(extent)..=anchor.max(extent)))
    });
    if selected_rows.is_none() {
        text_selection = None;
    }
    let layout_width = ui.available_width();
    let lay_out_all = view.anchor.is_some() || divider.as_ref().is_some_and(|(.., placed)| !placed);
    let pass = ui.ctx().cumulative_pass_nr();
    let redo = ui.ctx().current_pass_index() > 0;
    let mut rows = std::mem::take(&mut conversation.rows);
    if rows.len() > conversation.messages.len() * 2 + 64 {
        let ids: HashSet<&str> = conversation
            .messages
            .iter()
            .map(|message| message.id.as_str())
            .collect();
        rows.retain(|id, _| ids.contains(id.as_str()));
    }
    let mut grew_above = 0.0;
    let scroll_id = ui.make_persistent_id(egui::IdSalt::new(("messages", &chat.id)));
    let scroll_state = egui::scroll_area::State::load(ui.ctx(), scroll_id);
    let first_show = scroll_state.is_none();
    let offset = scroll_state.map_or(0.0, |state| state.offset.y);
    let mut key_scroll = conversation.key_scroll.take();
    let list = ui.available_rect_before_wrap();
    if focus_list {
        ui.memory_mut(|memory| memory.request_focus(list_id));
    }
    let list_response =
        super::list_nav::container(ui, list_id, list, list_label, total).tab_stop(Stop::Messages);
    let list_focused = list_response.has_focus();
    if list_response.gained_focus() && keyboard {
        reveal = true;
    }
    if list_focused && keyboard {
        keyboard_navigation.set(true);
    }
    let hold_bottom = list_focused
        && keyboard
        && app.message_cursor.get(&chat.id).is_none_or(|id| {
            app.cursor_followed
                .as_ref()
                .is_some_and(|(followed_chat, followed)| {
                    *followed_chat == chat.id && followed == id
                })
        });
    let mut active_row = None;
    let mut cursor_drawn = false;
    let wheel_over_list = ui.input(|input| {
        (input.smooth_scroll_delta.y != 0.0
            || input
                .raw
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::MouseWheel { .. })))
            && input
                .pointer
                .hover_pos()
                .is_some_and(|pos| list.contains(pos))
    });
    // The reaction bar holds the view still.
    let carried = if view.reaction.is_some() {
        0.0
    } else {
        carried
    };
    if view.anchor.is_some() || view.reaction.is_some() || wheel_over_list || carried != 0.0 {
        key_scroll = None;
    }
    let key_duration = ui.style().scroll_animation.duration.max;
    let key_start = time - f64::from(ui.input(|input| input.stable_dt.min(0.1)));
    let mut pinned = false;
    let mut visible_day = None;
    let selection_offset = (pressed && text_selection.is_some())
        .then(|| {
            ui.ctx()
                .data(|data| data.get_temp::<f32>(text_selection_id.with("painted-offset")))
        })
        .flatten();
    let scroll = egui::ScrollArea::vertical();
    let scroll = if let Some(offset) = selection_offset {
        scroll.vertical_scroll_offset(offset)
    } else {
        scroll
    };
    let output = scroll
        .id_salt(("messages", &chat.id))
        .auto_shrink([false, false])
        .stick_to_bottom(view.reaction.is_none() && !(text_selection.is_some() && drag_frame))
        .scroll_source(if view.reaction.is_some() {
            egui::scroll_area::ScrollSource::NONE
        } else {
            Default::default()
        })
        .animated(false)
        .show(ui, |ui| {
            let viewport = ui.clip_rect();
            ui.ctx().data_mut(|data| {
                data.insert_temp(
                    text_selection_id.with("painted-offset"),
                    (viewport.top() - ui.max_rect().top()).max(0.0),
                );
            });
            *app.selection_view.lock().unwrap_or_else(|p| p.into_inner()) = Some(viewport);
            if carried != 0.0 {
                ui.scroll_with_delta_animation(
                    vec2(0.0, carried),
                    egui::style::ScrollAnimation::none(),
                );
            }
            let background = ui.interact(
                viewport,
                ui.id().with(("message-background", &chat.id)),
                Sense::click(),
            );
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
                    key_scroll = None;
                    ui.scroll_with_delta_animation(
                        vec2(0.0, -delta),
                        egui::style::ScrollAnimation::none(),
                    );
                    ui.ctx().request_repaint();
                }
            }
            let natural_id = scroll_id.with("natural-height");
            let natural = ui.ctx().data(|data| data.get_temp::<f32>(natural_id));
            // The spacing after the filler is part of the space it takes.
            let filler = bottom_filler(viewport.height(), natural, ui.spacing().item_spacing.y);
            let filler_id = scroll_id.with("filler");
            let held = ui.input(|input| input.pointer.any_down());
            let filler = match ui.ctx().data(|data| data.get_temp::<f32>(filler_id)) {
                Some(kept) if held => kept,
                _ => filler,
            };
            ui.ctx()
                .data_mut(|data| data.insert_temp(filler_id, filler));
            if filler > 0.0 {
                ui.add_space(filler);
            }
            let side = if palette.bubbles == theme::BubbleStyle::Messages {
                MESSAGES_SIDE_MARGIN
            } else {
                WHATSAPP_SIDE_MARGIN
            };
            let measured = Frame::new()
                .inner_margin(Margin::symmetric(side, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 3.0;
                    if under > 0.0 {
                        ui.add_space(under);
                    }
                    top_of_history(ui, &palette, &conversation, chat, &mut actions);
                    let mut previous: Option<&Message> = None;
                    let mut day_separator_top = f32::NEG_INFINITY;
                    let notices = pin_notices_in_time_order(view.pin_notices);
                    let mut placed = 0;
                    let margin = (viewport.height() * 3.0).max(600.0);
                    for (message_index, message) in conversation.messages.iter().enumerate() {
                        let index = message_index;
                        if selection.is_none()
                            && album_continues(&conversation.messages, message_index)
                        {
                            rows.insert(
                                message.id.clone(),
                                RowHeight {
                                    height: 0.0,
                                    pass: Some(pass),
                                },
                            );
                            previous = Some(message);
                            continue;
                        }
                        let album = selection
                            .is_none()
                            .then(|| album_run(&conversation.messages, message_index))
                            .flatten();
                        let row_contains = |id: &str| {
                            message.id == id
                                || album.is_some_and(|items| items.iter().any(|item| item.id == id))
                        };
                        let row_is_anchor = view.anchor.is_some_and(row_contains);
                        let before = ui.cursor().top();
                        let new_day = previous.is_none_or(|previous| {
                            crate::util::day_key(previous.timestamp)
                                != crate::util::day_key(message.timestamp)
                        });
                        if new_day {
                            day_separator_top = before + 8.0;
                        }
                        let here = notices_above(&notices[placed..], message.timestamp);
                        let known = rows.get(&message.id).copied();
                        let height = known.map_or_else(
                            || estimated_height(message, layout_width, new_day, here),
                            |row| row.height,
                        );
                        let reach = if redo
                            && known
                                .and_then(|row| row.pass)
                                .is_none_or(|last| last + 1 != pass)
                        {
                            0.0
                        } else {
                            margin
                        };
                        let near = before + height >= viewport.top() - reach
                            && before <= viewport.bottom() + reach;
                        let selected = selected_rows.as_ref().is_some_and(|(anchor, span)| {
                            span.contains(&index)
                                || (drag_frame
                                    && if index < *anchor {
                                        before + height >= viewport.top() - margin
                                    } else {
                                        before <= viewport.bottom() + margin
                                    })
                        });
                        if !lay_out_all && !near && !selected {
                            if known
                                .is_some_and(|row| row.pass.is_some_and(|last| last + 1 == pass))
                            {
                                let id = bubble_id(&chat.id, &message.id).with("body");
                                ui.ctx().data_mut(|data| {
                                    data.remove::<Rect>(id);
                                    data.remove::<Rect>(text_selection_id.with(&message.id));
                                });
                            }
                            ui.add_space(height);
                            if known.is_none() {
                                rows.insert(message.id.clone(), RowHeight { height, pass: None });
                            }
                            previous = Some(message);
                            continue;
                        }
                        if known.is_none_or(|row| row.pass.is_none_or(|last| last + 1 < pass)) {
                            let id = bubble_id(&chat.id, &message.id).with("rect");
                            ui.ctx().data_mut(|data| data.remove::<Rect>(id));
                        }
                        if sweep.is_some() || (text_selection.is_some() && drag_frame) {
                            first_row.get_or_insert(message.id.as_str());
                            if sweep_pointer.is_some_and(|pointer| pointer.y >= before) {
                                swept_to = Some(message.id.as_str());
                            }
                        }
                        let notices_before = placed;
                        let current = cursor == Some(index);
                        let hidden = !near
                            && !selected
                            && !current
                            && !row_is_anchor
                            && divider.as_ref().is_none_or(|(id, ..)| !row_contains(id));
                        let builder = super::list_nav::row_builder(
                            ui,
                            message_row_id(&chat.id, &message.id),
                            list_id,
                        );
                        let row = ui.scope_builder(
                            if hidden { builder.invisible() } else { builder },
                            |ui| {
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
                                for _ in 0..here {
                                    let Some(notice) = notices.get(placed) else {
                                        break;
                                    };
                                    pin_notice(ui, &palette, view.locale, view.names_or, notice);
                                    placed += 1;
                                }
                                if let Some((id, count, placed)) = &divider
                                    && row_contains(id)
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
                                        // Clear of the header that may lie over the top.
                                        ui.scroll_to_rect(
                                            response.rect.with_min_y(response.rect.top() - under),
                                            Some(Align::Min),
                                        );
                                        divider_placed = true;
                                    }
                                    ui.add_space(4.0);
                                }
                                let show_sender = shows_sender_pictures(chat)
                                    && !message.from_me
                                    && (new_day
                                        || previous
                                            .is_none_or(|previous| !same_run(previous, message)));
                                let first_in_run = new_day
                                    || previous.is_none_or(|previous| !same_run(previous, message));
                                let show_avatar = if palette.bubbles == theme::BubbleStyle::Messages
                                {
                                    shows_sender_pictures(chat)
                                        && !message.from_me
                                        && last_of_run(&conversation.messages, index)
                                } else {
                                    show_sender
                                };
                                let tailed = if palette.bubbles == theme::BubbleStyle::Messages {
                                    last_bubble_of_run(&palette, &conversation.messages, index)
                                } else {
                                    first_in_run
                                };
                                if first_in_run && !new_day && previous.is_some() {
                                    ui.add_space(RUN_GAP);
                                }
                                let flash = jump
                                    .as_ref()
                                    .filter(|jump| row_contains(&jump.message))
                                    .map(|_| {
                                        (ui.painter().add(egui::Shape::Noop), ui.cursor().top())
                                    });
                                let band = selection.as_ref().map(|_| {
                                    (ui.painter().add(egui::Shape::Noop), ui.cursor().top())
                                });
                                let response = if selection.is_some() {
                                    ui.horizontal_top(|ui| {
                                        ui.add_space(SELECT_GUTTER);
                                        ui.vertical(|ui| {
                                            bubble(
                                                ui,
                                                &view,
                                                message,
                                                album,
                                                RunPlace {
                                                    show_sender,
                                                    show_avatar,
                                                    tailed,
                                                },
                                                &mut actions,
                                            )
                                        })
                                        .inner
                                    })
                                    .inner
                                } else {
                                    bubble(
                                        ui,
                                        &view,
                                        message,
                                        album,
                                        RunPlace {
                                            show_sender,
                                            show_avatar,
                                            tailed,
                                        },
                                        &mut actions,
                                    )
                                };
                                if let Some((slot, top)) = flash {
                                    if row_is_anchor && response.is_some() {
                                        jump_since.set(Some(time));
                                    }
                                    let strength = jump_since
                                        .get()
                                        .map_or(0.0, |since| JumpHighlight::strength(time - since));
                                    if strength > 0.0 {
                                        let band = Rect::from_x_y_ranges(
                                            viewport.x_range(),
                                            top - 3.0..=ui.min_rect().bottom() + 3.0,
                                        );
                                        ui.painter().set(
                                            slot,
                                            egui::Shape::rect_filled(
                                                band,
                                                0.0,
                                                palette.accent.gamma_multiply(0.22 * strength),
                                            ),
                                        );
                                    }
                                    if jump_since.get().is_some() {
                                        crate::motion::request_frame(ui.ctx());
                                    }
                                }
                                if let (Some(selected), Some(response), Some((slot, top))) =
                                    (&selection, &response, band)
                                {
                                    let checked = selected.contains(&message.id);
                                    let selectable = crate::app::can_select(&message.content);
                                    let row = Rect::from_x_y_ranges(
                                        viewport.x_range(),
                                        top..=ui.min_rect().bottom(),
                                    )
                                    .expand2(vec2(0.0, ui.spacing().item_spacing.y / 2.0));
                                    let pick = ui.interact(
                                        row,
                                        bubble_id(&chat.id, &message.id).with("pick"),
                                        if selectable {
                                            Sense::click_and_drag()
                                        } else {
                                            Sense::hover()
                                        },
                                    );
                                    let fill = if checked {
                                        palette.accent.gamma_multiply(0.16)
                                    } else if pick.hovered() {
                                        palette.text.gamma_multiply(0.05)
                                    } else {
                                        Color32::TRANSPARENT
                                    };
                                    ui.painter()
                                        .set(slot, egui::Shape::rect_filled(row, 0.0, fill));
                                    let drawn = ui
                                        .ctx()
                                        .data(|data| {
                                            data.get_temp::<Rect>(
                                                bubble_id(&chat.id, &message.id).with("rect"),
                                            )
                                        })
                                        .unwrap_or(response.rect);
                                    let check = Rect::from_center_size(
                                        pos2(
                                            ui.max_rect().left() + SELECT_BOX / 2.0,
                                            drawn.center().y,
                                        ),
                                        Vec2::splat(SELECT_BOX),
                                    );
                                    // Only forwardable messages get a box.
                                    if selectable {
                                        select_box(ui, &palette, check, checked);
                                        if pick.has_focus()
                                            && ui.ctx().data(|data| {
                                                data.get_temp::<bool>(theme::keyboard_focus_id())
                                                    .unwrap_or(false)
                                            })
                                        {
                                            keyboard_navigation.set(true);
                                        }
                                        theme::reveal_focus(&pick);
                                        theme::focus_outline(ui, pick.id, check.expand(3.0), 4.0);
                                        pick.widget_info(|| {
                                            let sender = if message.from_me {
                                                crate::i18n::gettext(view.locale, "You")
                                                    .into_owned()
                                            } else {
                                                (view.names_or)(
                                                    &message.sender,
                                                    message.sender_name.as_deref(),
                                                )
                                            };
                                            let summary: String = message
                                                .content
                                                .summary_in(view.locale)
                                                .chars()
                                                .take(120)
                                                .collect();
                                            let label = crate::i18n::gettext(
                                                view.locale,
                                                "Select message from {sender}, {time}: {summary}",
                                            )
                                            .replace("{sender}", &sender)
                                            .replace(
                                                "{time}",
                                                &crate::util::moment_stamp(
                                                    view.locale,
                                                    message.timestamp,
                                                ),
                                            )
                                            .replace("{summary}", &summary);
                                            egui::WidgetInfo::selected(
                                                egui::WidgetType::Checkbox,
                                                pick.enabled(),
                                                checked,
                                                label,
                                            )
                                        });
                                        #[cfg(any(test, feature = "demo"))]
                                        ui.ctx().data_mut(|data| {
                                            data.insert_temp(
                                                bubble_id(&chat.id, &message.id).with("check"),
                                                check,
                                            );
                                        });
                                    }
                                    if pick.drag_started() {
                                        actions.push(Action::SweepMessages {
                                            anchor: message.id.clone(),
                                            to: message.id.clone(),
                                        });
                                    }
                                    let keyboard_clicked = (pick.clicked() && pick.has_focus()
                                        || response.clicked() && response.has_focus())
                                        && ui.input(|input| {
                                            input.raw.events.iter().any(|event| {
                                                matches!(
                                                    event,
                                                    egui::Event::Key {
                                                        key: egui::Key::Enter,
                                                        pressed: true,
                                                        ..
                                                    }
                                                )
                                            })
                                        });
                                    theme::focus_outline(ui, response.id, check.expand(3.0), 4.0);
                                    let popup = egui::Popup::menu(&pick)
                                        .open_memory(
                                            if pick.secondary_clicked() || keyboard_clicked {
                                                Some(egui::SetOpenCommand::Bool(true))
                                            } else {
                                                None
                                            },
                                        )
                                        .frame(widgets::menu_frame(&palette));
                                    let popup = if keyboard_clicked {
                                        popup.at_position(row.left_top() + vec2(12.0, 8.0))
                                    } else {
                                        popup.at_pointer_fixed()
                                    };
                                    popup.show(|ui| {
                                        selection_menu(
                                            ui,
                                            &view,
                                            &conversation.messages,
                                            selected,
                                            &mut actions,
                                        );
                                    });
                                    if selectable
                                        && !keyboard_clicked
                                        && (response.clicked() || pick.clicked())
                                    {
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
                                    && crate::app::can_select(&message.content)
                                {
                                    // Ctrl-click (Command-click on macOS) starts a selection.
                                    actions.push(Action::SelectMessage(message.id.clone()));
                                }
                                if current && let Some(response) = &response {
                                    if reveal {
                                        let mut target = response.clone();
                                        target.rect = target.rect.expand(4.0);
                                        target.scroll_to_me_animation(
                                            None,
                                            egui::style::ScrollAnimation::none(),
                                        );
                                    }
                                }
                                if let Some(response) = response
                                    && row_is_anchor
                                {
                                    response.scroll_to_me(Some(Align::Center));
                                    anchored = true;
                                }
                                ui.unique_id()
                            },
                        );
                        if ui.ctx().accesskit_node_builder(row.inner, |_| ()).is_some() {
                            let sender = if message.from_me {
                                crate::i18n::gettext(view.locale, "You").into_owned()
                            } else {
                                (view.names_or)(&message.sender, message.sender_name.as_deref())
                            };
                            // The same text the bubble draws as its body or caption.
                            let text = match &message.content {
                                Content::Text { text, .. } | Content::Interactive { text, .. } => {
                                    Some(text)
                                }
                                Content::Image { caption, .. }
                                | Content::Video { caption, .. }
                                | Content::Document { caption, .. } => caption.as_ref(),
                                _ => None,
                            };
                            let text = text.map_or_else(String::new, |text| {
                                markup::plain(text, &mentions_of(&view, message))
                            });
                            let members = album.unwrap_or(std::slice::from_ref(message));
                            let last = members.last().unwrap_or(message);
                            let body = if let Some(members) = album {
                                album_label(&view, members)
                            } else {
                                match super::calls::Entry::of(message) {
                                    Some(entry) => {
                                        crate::a11y::call_description(view.locale, &entry)
                                    }
                                    None => crate::a11y::content_description(
                                        view.locale,
                                        &message.content,
                                        &text,
                                    ),
                                }
                            };
                            let day = new_day
                                .then(|| crate::util::day_label(view.locale, message.timestamp));
                            // The pin notices this row drew above the message.
                            let pin_notices: Vec<String> = notices[notices_before..placed]
                                .iter()
                                .map(|notice| {
                                    pin_notice_label(view.locale, notice, &|id, hint| {
                                        crate::a11y::without_push_marker(&(view.names_or)(id, hint))
                                            .to_owned()
                                    })
                                })
                                .collect();
                            let unread = divider.as_ref().is_some_and(|(id, ..)| row_contains(id));
                            let quoted = members.iter().find_map(|member| member.quoted.as_ref());
                            let reply = quoted.map(|quoted| {
                                (
                                    quoted_sender_name(
                                        &view,
                                        quoted,
                                        &crate::i18n::gettext(view.locale, "You"),
                                    ),
                                    markup::plain(
                                        &quoted.summary_in(view.locale),
                                        &quote_mentions(&view, quoted),
                                    ),
                                )
                            });
                            let all_reactions: Vec<_> = members
                                .iter()
                                .flat_map(|member| member.reactions.iter().cloned())
                                .collect();
                            let reactions = crate::a11y::reaction_counts(&all_reactions);
                            let status = last
                                .from_me
                                .then(|| {
                                    crate::a11y::status_word(
                                        view.locale,
                                        shown_delivery(&view, last),
                                    )
                                })
                                .flatten();
                            let time = crate::util::clock(last.timestamp);
                            let attachment = members.iter().find_map(|member| {
                                crate::a11y::attachment_state(
                                    view.locale,
                                    &member.content,
                                    download_starts_by_itself(&member.content, view.auto_download),
                                )
                            });
                            let label = crate::a11y::message_label(
                                view.locale,
                                &crate::a11y::MessageLabel {
                                    day: day.as_deref(),
                                    notices: &pin_notices,
                                    unread,
                                    sender: &sender,
                                    body: &body,
                                    time: &time,
                                    status: status.as_deref(),
                                    reply_to: reply
                                        .as_ref()
                                        .map(|(name, text)| (name.as_str(), text.as_str())),
                                    forwarded: message.forwarded,
                                    edited: last.edited,
                                    starred: view.starred.is_some_and(|ids| ids.contains(&last.id)),
                                    pinned: view.pins.is_some_and(|ids| ids.contains(&last.id)),
                                    reactions: &reactions,
                                    attachment: attachment.as_deref(),
                                },
                            );
                            super::list_nav::mark_row(
                                ui.ctx(),
                                row.inner,
                                label,
                                row_of(&starts, index),
                                total,
                                row.response.rect,
                                None,
                            );
                        }
                        if current {
                            active_row = Some(row.inner);
                            cursor_drawn = true;
                        }
                        let measured = ui.cursor().top() - before;
                        ui.ctx().data_mut(|data| {
                            data.insert_temp(
                                text_selection_id.with(&message.id),
                                Rect::from_min_max(
                                    pos2(viewport.left(), before),
                                    pos2(viewport.right(), before + measured),
                                ),
                            );
                        });
                        if visible_day.is_none()
                            && before + measured > viewport.top()
                            && before < viewport.bottom()
                        {
                            visible_day = Some((message.timestamp, day_separator_top));
                        }
                        if before + height <= viewport.top() {
                            grew_above += measured - height;
                        }
                        rows.insert(
                            message.id.clone(),
                            RowHeight {
                                height: measured,
                                pass: Some(pass),
                            },
                        );
                        previous = Some(message);
                    }
                    super::list_nav::set_active(ui.ctx(), list_id, active_row);
                    if let Some((anchor, to)) = &sweep
                        && let Some(row) = swept_to.or(first_row)
                        && row != to
                    {
                        actions.push(Action::SweepMessages {
                            anchor: anchor.clone(),
                            to: row.to_owned(),
                        });
                    }
                    if drag_frame
                        && sweep_pointer.is_some()
                        && let Some(span) = &mut text_selection
                        && let Some(row) = swept_to.or(first_row)
                    {
                        span.extent = row.to_owned();
                    }
                    for notice in &notices[placed..] {
                        pin_notice(ui, &palette, view.locale, view.names_or, notice);
                    }
                    if !typing.is_empty() {
                        typing_bubble(ui, &view, &typing);
                    }
                    ui.add_space(4.0);
                    if scroll_to_bottom
                        && (scroll_forced || hold_bottom || !keyboard_navigation.get())
                        && view.reaction.is_none()
                    {
                        ui.scroll_to_rect_animation(
                            Rect::from_min_size(ui.cursor().min, Vec2::ZERO),
                            None,
                            egui::style::ScrollAnimation::none(),
                        );
                        pinned = true;
                    }
                })
                .response
                .rect
                .height();
            if natural.is_none_or(|natural| (natural - measured).abs() > 0.5) {
                ui.ctx()
                    .data_mut(|data| data.insert_temp(natural_id, measured));
                ui.ctx().request_repaint();
            }
            if background.clicked() {
                actions.push(Action::FocusComposer);
            }
            if pinned {
                key_scroll = None;
            } else if let Some(kind) = pending_scroll
                && view.reaction.is_none()
            {
                let page = match kind {
                    Scroll::PageUp => viewport.height() * 0.9,
                    Scroll::PageDown => -(viewport.height() * 0.9),
                    Scroll::Top | Scroll::Bottom => 0.0,
                };
                let left = key_scroll
                    .filter(|scroll| scroll.kind == kind)
                    .map_or(0.0, |scroll| scroll.remaining);
                key_scroll = Some(KeyScroll::new(kind, left + page, key_start, key_duration));
            }
            if let Some(scroll) = &mut key_scroll {
                let fraction = scroll.advance(time);
                let step = match scroll.kind {
                    Scroll::PageUp | Scroll::PageDown => {
                        let step = scroll.remaining * fraction;
                        scroll.remaining -= step;
                        step
                    }
                    Scroll::Top => offset * fraction,
                    Scroll::Bottom => {
                        -(ui.min_rect().bottom() - viewport.bottom()).max(0.0) * fraction
                    }
                };
                ui.scroll_with_delta_animation(
                    vec2(0.0, step),
                    egui::style::ScrollAnimation::none(),
                );
                ui.ctx().request_repaint();
            }
        });
    app.scroll_route
        .place(crate::app::ScrollPane::Messages, output.inner_rect);
    ui.ctx().data_mut(|data| match text_selection {
        Some(span) => {
            data.insert_temp(text_selection_id, span);
        }
        None => {
            data.remove::<TextSelectionRows>(text_selection_id);
        }
    });
    sticky_date(ui, &palette, app.locale, chat, &output, visible_day);
    let at_bottom =
        output.state.offset.y + output.inner_rect.height() >= output.content_size.y - 24.0;
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            scroll_metrics_id(&chat.id),
            (output.state.offset.y, output.inner_rect.height()),
        );
        #[cfg(test)]
        data.insert_temp(
            scroll_metrics_id(&chat.id).with("bounds"),
            (
                output.id,
                (output.content_size.y - output.inner_rect.height()).max(0.0),
            ),
        );
    });
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
    let fetching_explicit = conversation.phone_explicit;
    let exhausted = conversation.phone_exhausted;
    conversation.rows = rows;
    if grew_above.abs() >= 0.5
        && !lay_out_all
        && !scroll_to_bottom
        && !at_bottom
        && key_scroll.is_none_or(|scroll| scroll.kind != Scroll::Top)
    {
        let mut state = output.state;
        let max_offset = (output.content_size.y - output.inner_rect.height()).max(0.0);
        state.offset.y = (state.offset.y + grew_above).clamp(0.0, max_offset);
        state.store(ui.ctx(), output.id);
        ui.ctx()
            .request_discard("transcript rows above the viewport changed height");
    }
    if first_show && pinned && !redo && (output.state.offset.y - offset).abs() >= 0.5 {
        ui.ctx().request_discard("scrolled to the end");
    }
    if reader_scrolled {
        key_scroll = None;
    }
    if let Some(scroll) = key_scroll
        && scroll.done()
    {
        key_scroll = None;
        if scroll.kind == Scroll::Bottom {
            // Keep later messages pinned, as Ctrl+End already does.
            app.scroll_to_bottom = true;
        }
    }
    conversation.key_scroll = key_scroll;
    app.conversations
        .insert(chat.id.clone(), std::mem::take(&mut conversation));
    app.at_bottom = at_bottom;
    if app.scroll_to_bottom
        && (reader_scrolled || (keyboard_navigation.get() && !scroll_forced && !hold_bottom))
    {
        app.scroll_to_bottom = false;
    }
    if divider_placed {
        if let Some(divider) = app
            .unread_divider
            .as_mut()
            .filter(|divider| divider.chat == chat.id)
        {
            divider.placed = true;
        }
        app.scroll_to_bottom = false;
    }
    if let Some(jump) = &mut app.jump_highlight
        && jump.chat == chat.id
    {
        jump.since = jump_since.get();
        if jump
            .since
            .is_some_and(|since| time - since > JumpHighlight::DURATION)
        {
            app.jump_highlight = None;
        }
    }
    if anchored {
        app.scroll_anchor = None;
        if let Some(divider) = app
            .unread_divider
            .as_mut()
            .filter(|divider| divider.chat == chat.id)
        {
            divider.placed = true;
        }
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
        app.scroll_anchor = None;
    }
    app.messages_focused = ui.memory(|memory| memory.has_focus(list_id));
    if reveal
        && !cursor_drawn
        && app.scroll_anchor.is_none()
        && let Some(id) = cursor_id
    {
        app.scroll_anchor = Some(id);
        ui.ctx()
            .request_discard("lay out the current message far from the view");
    }
    let fits = output.content_size.y <= output.inner_rect.height() + 1.0;
    let near_top = output.state.offset.y < 80.0;
    let explicit = near_top && !fits && !scroll_to_bottom;
    // A reader reaching the top while an automatic request waits claims it.
    let claim = fetching && explicit && !fetching_explicit;
    if (near_top || fits)
        && ((!complete && !loading) || (complete && (!fetching || claim) && !exhausted))
    {
        actions.push(Action::LoadOlder {
            chat: chat.id.clone(),
            explicit,
        });
    }
    app.actions.extend(actions);
    if edge_scrolled_up {
        app.scroll_to_bottom = false;
    }
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
        child.painter().add(
            egui::epaint::Shadow {
                offset: [0, 3],
                blur: 10,
                spread: 0,
                color: palette.lift_shadow(),
            }
            .as_shape(button, CornerRadius::same(20)),
        );
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

#[allow(dead_code)]
pub(crate) fn floating_circle(ui: &egui::Ui, palette: &Palette, rect: Rect, hovered: bool) {
    let radius = CornerRadius::from(rect.width() / 2.0);
    ui.painter()
        .add(palette.bubble_shadow().as_shape(rect, radius));
    ui.painter().circle_filled(
        rect.center(),
        rect.width() / 2.0,
        if hovered {
            palette.surface_hover
        } else {
            palette.overlay
        },
    );
}

/// Loading state above the oldest visible message.
pub(crate) fn top_of_history(
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

/// Live chat activity with stacked avatars, typing dots, and a recording icon.
pub(crate) fn typing_bubble(ui: &mut egui::Ui, view: &View<'_>, typers: &[(String, String, ChatActivity)]) {
    let palette = view.palette;
    ui.horizontal(|ui| {
        if shows_sender_pictures(view.chat) {
            let count = typers.len().min(3);
            let step = SENDER_AVATAR * 0.6;
            let width = SENDER_AVATAR + step * (count.saturating_sub(1)) as f32;
            let (rect, _) = ui.allocate_exact_size(vec2(width, SENDER_AVATAR), Sense::hover());
            if ui.is_rect_visible(rect) {
                for (index, (id, name, _)) in typers.iter().take(count).enumerate() {
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
        let backdrop = ui.painter().add(egui::Shape::Noop);
        let rect = Frame::new()
            .inner_margin(Margin::symmetric(12, 9))
            .show(ui, |ui| {
                let recording = typers
                    .iter()
                    .any(|(_, _, activity)| *activity == ChatActivity::RecordingAudio);
                let typing = typers
                    .iter()
                    .any(|(_, _, activity)| *activity == ChatActivity::Typing);
                ui.horizontal(|ui| {
                    if recording {
                        theme::icon(ui, Icon::Mic, 18.0, palette.secondary);
                    }
                    if typing {
                        typing_dots(ui, &palette);
                    }
                });
            })
            .response
            .rect;
        ui.painter().set(
            backdrop,
            widgets::bubble_shape(
                &palette,
                rect,
                palette.bubble_in,
                Some(widgets::Side::Left),
                ui.ctx().pixels_per_point(),
                ui.clip_rect().y_range(),
            ),
        );
    });
}

pub(crate) fn typing_dots(ui: &mut egui::Ui, palette: &Palette) {
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
    crate::motion::request_frame(ui.ctx());
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

pub(crate) const REACTION_AFFORDANCE_SIZE: f32 = 26.0;
pub(crate) const REACTION_AFFORDANCE_GAP: f32 = 6.0;

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
pub(crate) fn reaction_affordance_visible(pointer: Option<egui::Pos2>, bubble: Rect, button: Rect) -> bool {
    pointer.is_some_and(|pointer| bubble.union(button).contains(pointer))
}

pub(crate) fn open_reaction_picker_action(chat: &str, message: &str, from_list: bool) -> Action {
    Action::OpenReactionPicker {
        chat: chat.to_owned(),
        message: message.to_owned(),
        beside_menu: false,
        from_list,
    }
}

#[allow(dead_code)]
pub(crate) fn reaction_affordance(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    bubble: &egui::Response,
    tiles: &[(Rect, String)],
    actions: &mut Vec<Action>,
) {
    let bounds = ui.clip_rect().shrink(2.0);
    if bounds.width() < REACTION_AFFORDANCE_SIZE || bounds.height() < REACTION_AFFORDANCE_SIZE {
        return;
    }
    let shown = bubble.rect.intersect(bounds);
    if shown.height() < REACTION_AFFORDANCE_SIZE || shown.width() <= 0.0 {
        return;
    }
    // A call in the chat is a record, as in WhatsApp: no reactions, no replies.
    if matches!(message.content, Content::Call { .. }) {
        return;
    }
    let rect = reaction_affordance_rect(shown, bounds, message.from_me);
    let pointer = ui.input(|input| input.pointer.interact_pos());
    let uncovered = ui
        .ctx()
        .layer_id_at(bubble.rect.center())
        .is_none_or(|layer| layer == bubble.layer_id);

    ui.ctx()
        .data_mut(|data| data.insert_temp(bubble.id.with("react-rect"), rect));

    // Acquire under the same layer as the bubble so the row strip keeps clicks.
    let response = ui.interact(rect, bubble.id.with("react"), Sense::CLICK);
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
    let forward_rect = rect.translate(vec2(outward * if reply_fits { 2.0 } else { 1.0 }, 0.0));
    let forward_fits =
        crate::app::can_select(&message.content) && bounds.contains_rect(forward_rect);
    let reach = if forward_fits {
        rect.union(forward_rect)
    } else if reply_fits {
        rect.union(reply_rect)
    } else {
        rect
    };
    ui.ctx().data_mut(|data| {
        data.insert_temp(bubble.id.with("forward-rect"), forward_rect);
    });
    let revealed = reaction_affordance_visible(pointer, shown, reach);
    let album_target = bubble.id.with("album-hover-target");
    if !revealed {
        ui.ctx()
            .data_mut(|data| data.remove::<String>(album_target));
    } else if let Some(pos) = pointer {
        if let Some((_, id)) = tiles.iter().find(|(rect, _)| rect.contains(pos)) {
            ui.ctx()
                .data_mut(|data| data.insert_temp(album_target, id.clone()));
        } else if shown.contains(pos) {
            ui.ctx()
                .data_mut(|data| data.remove::<String>(album_target));
        }
    }
    if ui.is_rect_visible(rect) && uncovered && revealed {
        floating_circle(ui, &view.palette, rect, response.hovered());
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
        .on_hover_text(crate::i18n::gettext(view.locale, "React"))
        .clicked()
    {
        actions.push(open_reaction_picker_action(
            &view.chat.id,
            &message.id,
            false,
        ));
    }
    if reply_fits {
        let reply = ui.interact(reply_rect, bubble.id.with("reply"), Sense::CLICK);
        if ui.is_rect_visible(reply_rect) && uncovered && revealed {
            floating_circle(ui, &view.palette, reply_rect, reply.hovered());
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
            .on_hover_text(crate::i18n::gettext(view.locale, "Reply"))
            .clicked()
        {
            actions.push(Action::Reply(message.id.clone()));
        }
    }
    if forward_fits {
        let forward = ui.interact(forward_rect, bubble.id.with("forward"), Sense::CLICK);
        if ui.is_rect_visible(forward_rect) && uncovered && revealed {
            floating_circle(ui, &view.palette, forward_rect, forward.hovered());
            theme::paint_icon(
                ui,
                Icon::Forward,
                forward_rect.shrink(5.0),
                15.0,
                if forward.hovered() {
                    view.palette.text
                } else {
                    view.palette.secondary
                },
            );
        }
        if forward
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(crate::i18n::gettext(view.locale, "Forward").as_ref())
            .clicked()
        {
            let target = ui
                .ctx()
                .data(|data| data.get_temp::<String>(album_target))
                .unwrap_or_else(|| message.id.clone());
            actions.push(Action::ShowDialog(Dialog::Forward {
                chat: view.chat.id.clone(),
                messages: vec![target],
            }));
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct RunPlace {
    pub(crate) show_sender: bool,
    pub(crate) show_avatar: bool,
    pub(crate) tailed: bool,
}

pub(crate) fn unread_divider_message(messages: &[Message], count: u32) -> Option<&Message> {
    messages
        .iter()
        .rev()
        .filter(|message| !message.from_me && !crate::model::local_message_id(&message.id))
        .nth(count.saturating_sub(1) as usize)
}

pub(crate) fn timer_notice(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    seconds: u32,
) -> egui::Response {
    let name = if message.from_me {
        crate::i18n::gettext(view.locale, "You").into_owned()
    } else {
        (view.names_or)(&message.sender, message.sender_name.as_deref())
    };
    let text = crate::util::disappearing_change(view.locale, &name, seconds);
    ui.vertical_centered(|ui| widgets::chip(ui, &view.palette, &text))
        .inner
}



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

    fn input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        let Some(view) = *self.view.lock().unwrap_or_else(|p| p.into_inner()) else {
            self.holding = false;
            return;
        };
        let handle = ctx.global_style().interaction.resize_grab_radius_side;
        let inside = |pos: &egui::Pos2| {
            view.contains(*pos) && pos.x >= view.left() + handle && pos.x < view.right() - 16.0
        };
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

pub(crate) fn clamp_into(pos: egui::Pos2, view: Rect) -> egui::Pos2 {
    egui::pos2(
        pos.x.clamp(view.left() + 2.0, view.right() - 18.0),
        pos.y.clamp(view.top() + 2.0, view.bottom() - 2.0),
    )
}

/// Builds the transcript row used when copying across messages.
pub(crate) fn transcript_row(
    view: &View<'_>,
    message: &Message,
    body: String,
    placements: Vec<String>,
) -> crate::transcript::Row {
    message_copy_row(message, body, placements, view.names_or, view.mention_names)
}

pub fn selected_text(app: &App) -> Option<String> {
    let (chat, selected) = app.selection.as_ref()?;
    if app.open_chat.as_ref() != Some(chat) {
        return None;
    }
    let conversation = app.conversations.get(chat)?;
    let selected: HashSet<&str> = selected.iter().map(String::as_str).collect();
    let names_or = |id: &str, hint: Option<&str>| app.display_name_or(id, hint);
    let mention_names = |id: &str| app.mention_name(id);
    let lines: Vec<String> = conversation
        .messages
        .iter()
        .filter(|message| selected.contains(message.id.as_str()))
        .map(|message| {
            let text = match &message.content {
                Content::Text { text, .. } | Content::Interactive { text, .. } => text.as_str(),
                Content::Image { caption, .. }
                | Content::Video { caption, .. }
                | Content::Document { caption, .. }
                | Content::StickerPack { caption, .. } => caption.as_deref().unwrap_or_default(),
                _ => "",
            };
            let mentions: Vec<markup::Mention> = message
                .mentions
                .iter()
                .map(|mention| markup::Mention {
                    user: mention.user.clone(),
                    name: mention_names(&mention.id),
                })
                .collect();
            let body = markup::plain(text, &mentions);
            let mut row = message_copy_row(message, body, Vec::new(), &names_or, &mention_names);
            // Content without a transcript marker still needs a useful label.
            if row.body.is_empty() && row.marker.is_none() {
                row.body = message.content.full_summary();
            }
            row.line(&row.body)
        })
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Builds shared transcript metadata for drawn text and whole-message exports.
pub(crate) fn message_copy_row(
    message: &Message,
    body: String,
    placements: Vec<String>,
    names_or: &dyn Fn(&str, Option<&str>) -> String,
    mention_names: &dyn Fn(&str) -> String,
) -> crate::transcript::Row {
    let who = if message.from_me {
        mention_names(&message.sender)
    } else {
        names_or(&message.sender, message.sender_name.as_deref())
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
        Content::StickerPack { name, .. } => Some(format!("[sticker pack: {name}]")),
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
            .map(|reaction| format!("{} {}", reaction.emoji, names_or(&reaction.sender, None)))
            .collect();
        format!(" ({})", listed.join(", "))
    };
    let quote = message.quoted.as_ref().map(|quoted| {
        let name = quoted
            .sender_name
            .clone()
            .unwrap_or_else(|| names_or(&quoted.sender, None));
        let mentions: Vec<markup::Mention> = quoted
            .mentions
            .iter()
            .map(|mention| markup::Mention {
                user: mention.user.clone(),
                name: mention_names(&mention.id),
            })
            .collect();
        let summary = markup::plain(&quoted.summary, &mentions);
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

pub(crate) fn callable(chat: &crate::model::Chat, me: Option<&str>) -> bool {
    match chat.kind {
        crate::model::ChatKind::Direct => me != Some(chat.id.as_str()),
        crate::model::ChatKind::Group => crate::calls::group_calls_enabled(),
        _ => false,
    }
}

pub(crate) fn call_menu(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &str,
    group: bool,
    actions: &mut Vec<Action>,
) {
    let palette = view.palette;
    let chat = &view.chat.id;
    let back = match view.chat.kind {
        crate::model::ChatKind::Group => crate::calls::group_calls_enabled(),
        crate::model::ChatKind::Direct => !group,
        _ => false,
    };
    if !back {
        let label = crate::i18n::gettext(view.locale, "Group calls happen on the phone");
        let keys_menu = ui
            .ctx()
            .data(|data| data.get_temp::<bool>(keys_menu_id(ui.layer_id().id)))
            .unwrap_or(false);
        if keys_menu {
            let row = widgets::menu_item_focusable(
                ui,
                &palette,
                Some(Icon::Phone),
                &label,
                &label,
                false,
                true,
            );
            if !ui.is_sizing_pass() {
                ui.ctx().data_mut(|data| {
                    data.insert_temp(menu_first_item_id(chat, message), Some(row.id));
                });
            }
        } else {
            ui.add_enabled_ui(false, |ui| {
                widgets::menu_item(ui, &palette, Some(Icon::Phone), &label);
            });
        }
        return;
    }
    let voice = widgets::menu_item_response(
        ui,
        &palette,
        Some(Icon::Phone),
        &crate::i18n::gettext(view.locale, "Voice call"),
        true,
    );
    if !ui.is_sizing_pass() {
        ui.ctx().data_mut(|data| {
            data.insert_temp(menu_first_item_id(chat, message), Some(voice.id));
        });
    }
    if voice.clicked() {
        actions.push(Action::StartCall(chat.clone()));
    }
    if widgets::menu_item(
        ui,
        &palette,
        Some(Icon::Video),
        &crate::i18n::gettext(view.locale, "Video call"),
    ) {
        actions.push(Action::StartVideoCall(chat.clone()));
    }
}

/// Starts a reply when the response was double-clicked, as the menu's "Reply".
pub(crate) fn reply_on_double_click(response: &egui::Response, message: &Message, actions: &mut Vec<Action>) {
    if response.double_clicked()
        && !matches!(message.content, Content::Revoked | Content::Call { .. })
    {
        actions.push(Action::Reply(message.id.clone()));
    }
}

pub(crate) fn album_label(view: &View<'_>, members: &[Message]) -> String {
    let videos = members
        .iter()
        .filter(|member| matches!(member.content, Content::Video { .. }))
        .count() as u32;
    let photos = members.len() as u32 - videos;
    let captions: Vec<String> = members
        .iter()
        .filter_map(|member| match &member.content {
            Content::Image { caption, .. } | Content::Video { caption, .. } => caption
                .as_ref()
                .map(|caption| markup::plain(caption, &mentions_of(view, member))),
            _ => None,
        })
        .collect();
    crate::a11y::album_description(view.locale, photos, videos, &captions)
}

pub fn message_row_id(chat: &str, message: &str) -> egui::Id {
    egui::Id::new(("message-row", chat, message))
}

/// The open chat's message list: its one Tab stop and its ListBox node.
pub fn messages_list_id(chat: &str) -> egui::Id {
    egui::Id::new(("messages-list", chat))
}

/// Stable message-bubble id used by interaction tests.
pub fn bubble_id(chat: &str, message: &str) -> egui::Id {
    egui::Id::new(("bubble", chat, message))
}

/// Where a voice message's speed chip was drawn, for interaction tests.
pub fn speed_chip_id(chat: &str, message: &str) -> egui::Id {
    egui::Id::new(("speed-chip", chat, message))
}

pub fn speed_button_id(chat: &str, message: &str, speed: f32) -> egui::Id {
    egui::Id::new(("speed", chat, message, speed.to_bits()))
}

/// A selecting row's check box: an outline, or ticked in the accent.
pub(crate) fn select_box(ui: &egui::Ui, palette: &Palette, rect: Rect, checked: bool) {
    if checked {
        ui.painter().rect_filled(rect, 4.0, palette.accent);
        theme::paint_icon(ui, Icon::Check, rect, 14.0, palette.on_accent);
    } else {
        ui.painter().rect_stroke(
            rect,
            4.0,
            Stroke::new(1.5, palette.secondary),
            egui::StrokeKind::Inside,
        );
    }
}


/// Whether a conversation has visible content. Used by tests.
#[allow(dead_code)]
pub fn has_messages(conversation: &Conversation) -> bool {
    !conversation.messages.is_empty()
}

#[allow(dead_code)]
pub(crate) fn status_label(status: Delivery) -> &'static str {
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
pub(crate) fn chat_of(chat: &ChatId) -> &str {
    chat
}
