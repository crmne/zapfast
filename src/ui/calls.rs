//! Calls as the history shows them: the entry a call leaves among a chat's
//! messages, and the Calls tab, laid out as WhatsApp's: the call history in
//! the list pane (or the new-call list over it), and beside it the open chat
//! or, with none open, the way to start a call.
//!
//! Both read the same entries, which come from WhatsApp's own records (the
//! phone's call log and the call-log messages in history) and from the calls
//! ZapFast took part in.

use egui::{Align, Frame, Layout, Margin, Rect, Sense, Vec2, pos2, vec2};

use crate::app::App;
use crate::i18n::{Locale, gettext, ngettext};
use crate::model::{Action, CallFilter, CallStatus, Content, Message};
use crate::theme::{self, Icon};
use crate::ui::focus::{Stop, TabStop};
use crate::ui::widgets;

/// One call, as its entry reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Whether this account placed the call.
    pub from_me: bool,
    pub video: bool,
    pub status: CallStatus,
    /// Seconds the call was up.
    pub duration: u64,
    pub group: bool,
    /// How many people were on a group call, this account counted once, from the entry's own
    /// record; `None` for a one-to-one call and for a group call whose record never named who
    /// took part. The Calls page's history row is where it is drawn.
    pub participants: Option<u32>,
}

impl Entry {
    /// The entry of a call message, if it is one.
    pub fn of(message: &Message) -> Option<Self> {
        match message.content {
            Content::Call {
                video,
                status,
                duration,
                group,
                participants,
            } => Some(Self {
                from_me: message.from_me,
                video,
                status,
                duration,
                group,
                participants,
            }),
            _ => None,
        }
    }

    /// One glyph for the state: the arrow says which way an answered call
    /// went, the camera what it carried, and a call that never connected has
    /// the missed or the slashed handset.
    pub fn icon(&self) -> Icon {
        match self.status {
            CallStatus::Missed | CallStatus::NoAnswer => Icon::PhoneMissed,
            CallStatus::Declined
            | CallStatus::DeclinedElsewhere
            | CallStatus::Busy
            | CallStatus::Failed
            | CallStatus::ConnectionLost => Icon::PhoneOff,
            CallStatus::Answered | CallStatus::AnsweredElsewhere => {
                if self.video {
                    Icon::Video
                } else if self.from_me {
                    Icon::PhoneOutgoing
                } else {
                    Icon::PhoneIncoming
                }
            }
        }
    }

    /// Whether this was a call to us that nobody took.
    pub fn missed(&self) -> bool {
        self.status == CallStatus::Missed && !self.from_me
    }

    /// The history's glyph: which way the call went, and a missed call's
    /// own. What it carried shows in its title and its call-back button.
    pub fn direction_icon(&self) -> Icon {
        if self.missed() {
            Icon::PhoneMissed
        } else if self.from_me {
            Icon::PhoneOutgoing
        } else {
            Icon::PhoneIncoming
        }
    }

    /// What the call was: "Missed voice call", "Video call", "Group voice call".
    pub fn title(&self, locale: Locale) -> String {
        let text = match (self.missed(), self.group, self.video) {
            (true, false, false) => gettext(locale, "Missed voice call"),
            (true, false, true) => gettext(locale, "Missed video call"),
            (true, true, false) => gettext(locale, "Missed group voice call"),
            (true, true, true) => gettext(locale, "Missed group video call"),
            (false, false, false) => gettext(locale, "Voice call"),
            (false, false, true) => gettext(locale, "Video call"),
            (false, true, false) => gettext(locale, "Group voice call"),
            (false, true, true) => gettext(locale, "Group video call"),
        };
        text.into_owned()
    }

    /// How it went: its length when it was up, else what became of it. A
    /// missed call says it in its title.
    pub fn detail(&self, locale: Locale) -> Option<String> {
        let text = match self.status {
            CallStatus::Answered if self.duration > 0 => {
                return Some(crate::util::call_length(locale, self.duration));
            }
            CallStatus::Answered | CallStatus::Missed => return None,
            CallStatus::AnsweredElsewhere => gettext(locale, "Answered on another device"),
            CallStatus::Declined => gettext(locale, "Declined"),
            CallStatus::DeclinedElsewhere => gettext(locale, "Declined on another device"),
            CallStatus::Busy => gettext(locale, "Busy"),
            CallStatus::Failed => gettext(locale, "Could not connect"),
            CallStatus::NoAnswer => gettext(locale, "No answer"),
            CallStatus::ConnectionLost => gettext(locale, "Connection lost"),
        };
        Some(text.into_owned())
    }
}

/// Calls that share one history row: consecutive calls with the same chat
/// that went the same way and ended alike, as WhatsApp folds them into one
/// row with a count. `calls` runs newest first, and so do the rows; each
/// row's first call is its newest.
pub fn rows(calls: &[Message]) -> Vec<Vec<&Message>> {
    let mut rows: Vec<Vec<&Message>> = Vec::new();
    for message in calls {
        let Some(entry) = Entry::of(message) else {
            continue;
        };
        let joins = rows.last().and_then(|row| row.last()).is_some_and(|last| {
            Entry::of(last).is_some_and(|before| {
                last.chat == message.chat
                    && before.from_me == entry.from_me
                    && before.missed() == entry.missed()
                    && before.group == entry.group
            })
        });
        match rows.last_mut() {
            Some(row) if joins => row.push(message),
            _ => rows.push(vec![message]),
        }
    }
    rows
}

/// The calls the Calls tab lists: a locked chat's stay out while its folder
/// is closed.
fn visible_calls(app: &App) -> Vec<Message> {
    app.call_log
        .iter()
        .filter(|message| {
            app.chat(&message.chat)
                .is_none_or(|chat| !chat.locked || app.locked_folder_open())
        })
        .cloned()
        .collect()
}

/// The name a row of the history goes by: its chat's, or "Call link" for
/// the calls joined through one, which belong to no chat.
fn row_name(app: &App, chat: &str) -> String {
    if chat == crate::model::CALL_LINK_CHAT {
        gettext(app.locale, "Call link").into_owned()
    } else {
        app.display_name(chat)
    }
}

/// Whether `id`'s name or number matches `needle`, a `search_key`.
fn matches(app: &App, id: &str, needle: &str) -> bool {
    needle.is_empty()
        || crate::util::search_key(&row_name(app, id)).contains(needle)
        || crate::model::phone_of(id).is_some_and(|phone| phone.contains(needle))
}

/// The Calls tab's list pane, under the header the chat list's header
/// draws for it: the call history, or the new-call list over it.
pub fn list(app: &mut App, ui: &mut egui::Ui) {
    if app.new_call {
        new_call_list(app, ui);
    } else {
        history(app, ui);
    }
}

/// Every call of the account, newest first, a row per run of alike calls.
fn history(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let calls = visible_calls(app);
    if calls.is_empty() {
        let (title, body) = if app.call_log_loaded {
            (
                gettext(app.locale, "No calls yet"),
                gettext(
                    app.locale,
                    "Calls from your phone's call log and the ones you make here appear here.",
                ),
            )
        } else {
            (
                gettext(app.locale, "Loading…"),
                gettext(app.locale, "Reading this account's calls."),
            )
        };
        Frame::new()
            .inner_margin(Margin::symmetric(24, 0))
            .show(ui, |ui| {
                widgets::empty_state(ui, &palette, Icon::Phone, title.as_ref(), body.as_ref());
            });
        return;
    }
    // The chip picks rows after they fold, so a row reads as it does
    // under All.
    let missed_only = app.call_filter == CallFilter::Missed;
    let shown =
        |row: &Vec<&Message>| !missed_only || Entry::of(row[0]).is_some_and(|entry| entry.missed());
    let folded = rows(&calls);
    if !folded.iter().any(shown) {
        Frame::new()
            .inner_margin(Margin::symmetric(24, 0))
            .show(ui, |ui| {
                widgets::empty_state(
                    ui,
                    &palette,
                    Icon::PhoneMissed,
                    gettext(app.locale, "No missed calls").as_ref(),
                    gettext(app.locale, "Choose All to see every call.").as_ref(),
                );
            });
        return;
    }
    let needle = crate::util::search_key(app.calls_search.trim());
    let rows: Vec<Vec<Message>> = folded
        .into_iter()
        .filter(|row| shown(row) && matches(app, &row[0].chat, &needle))
        .map(|row| row.into_iter().cloned().collect())
        .collect();
    let mut actions = Vec::new();
    egui::ScrollArea::vertical()
        .id_salt("call-history")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if rows.is_empty() {
                ui.add_space(12.0);
                ui.vertical_centered(|ui| {
                    theme::text(
                        ui,
                        gettext(app.locale, "No matching calls"),
                        theme::regular(13.0),
                        palette.secondary,
                    );
                });
            }
            for row in &rows {
                history_row(app, ui, row, &mut actions);
            }
            ui.add_space(12.0);
        });
    app.actions.extend(actions);
}

/// Stable id of a call filter chip, for tests.
pub fn filter_chip_id(filter: CallFilter) -> egui::Id {
    egui::Id::new(("call-filter", filter as u8))
}

/// The chips under the Calls page's search field, drawn as the chat list's
/// are: every call, or the missed ones. They stay while a search is typed,
/// which looks among the chip's calls. A missed call has no unread state,
/// so Missed carries no count.
pub fn filter_chips(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 6.0);
        for filter in CallFilter::EVERY {
            let selected = app.call_filter == filter;
            let chip =
                widgets::filter_chip(ui, &palette, filter.label(app.locale).as_ref(), 0, selected)
                    .tab_stop(match filter {
                        CallFilter::All => Stop::All,
                        CallFilter::Missed => Stop::Missed,
                    });
            ui.ctx()
                .data_mut(|data| data.insert_temp(filter_chip_id(filter), chip.rect));
            if chip.clicked() {
                // A second click on Missed returns to every call, as the
                // chat list's chips do.
                let next = if selected { CallFilter::All } else { filter };
                if next != app.call_filter {
                    app.actions.push(Action::SetCallFilter(next));
                }
            }
        }
    });
}

/// Stable id of a history row, for tests.
pub fn row_id(chat: &str, message: &str) -> egui::Id {
    egui::Id::new(("call-row", chat, message))
}

/// One history row, as WhatsApp draws it: the person, a count when calls
/// fold together, the way the newest went and what it was, when, and a
/// button to call back the same way. A click opens the chat beside the
/// list at that call.
fn history_row(app: &mut App, ui: &mut egui::Ui, row: &[Message], actions: &mut Vec<Action>) {
    let message = &row[0];
    let Some(entry) = Entry::of(message) else {
        return;
    };
    let palette = app.palette;
    let locale = app.locale;
    // A call joined through a link has no chat to open or call back, and no
    // photo: its row only records the call.
    let link = message.chat == crate::model::CALL_LINK_CHAT;
    let name = row_name(app, &message.chat);
    let picture = if link {
        None
    } else {
        app.avatar(&message.chat)
    };
    let selected = app.call_picked()
        && app
            .picked_call
            .as_ref()
            .is_some_and(|(chat, id)| *chat == message.chat && *id == message.id);
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, theme::ROW_HEIGHT), Sense::hover());
    // The row's own target goes first, so the call-back button inside keeps
    // its clicks.
    let response = ui
        .interact(rect, row_id(&message.chat, &message.id), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    theme::reveal_focus(&response);
    let mut label = name.clone();
    if row.len() > 1 {
        label = format!("{label} ({})", row.len());
    }
    let summary = summary(&entry, locale);
    let stamp = crate::util::chat_stamp(locale, message.timestamp);
    let spoken = format!("{label}, {summary}, {stamp}");
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            spoken.as_str(),
        )
    });
    if response.clicked() && !link {
        actions.push(Action::PickCall {
            chat: message.chat.clone(),
            message: message.id.clone(),
        });
    }
    // A person's call is called back, and a group's call in its group while
    // group calls are on; a call among several people with no group behind
    // it has no way back from here.
    let button_rect = Rect::from_center_size(
        pos2(rect.right() - 30.0, rect.center().y),
        Vec2::splat(34.0),
    );
    let group_chat = crate::model::ChatKind::from_id(&message.chat)
        == crate::model::ChatKind::Group
        && crate::calls::group_calls_enabled();
    if !link && (!entry.group || group_chat) {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(button_rect)
                .layout(Layout::centered_and_justified(egui::Direction::LeftToRight)),
        );
        let (icon, tooltip) = if entry.video {
            (Icon::Video, gettext(locale, "Video call"))
        } else {
            (Icon::Phone, gettext(locale, "Voice call"))
        };
        if theme::icon_button(
            &mut child,
            icon,
            18.0,
            palette.secondary,
            palette.text,
            tooltip.as_ref(),
        )
        .clicked()
        {
            actions.push(if entry.video {
                Action::StartVideoCall(message.chat.clone())
            } else {
                Action::StartCall(message.chat.clone())
            });
        }
    }
    if !ui.is_rect_visible(rect) {
        return;
    }
    if selected {
        widgets::row_highlight(ui, &palette, rect, palette.surface_active);
    } else if response.hovered() {
        widgets::row_highlight(ui, &palette, rect, palette.surface_hover);
    }
    let avatar_rect =
        Rect::from_center_size(pos2(rect.left() + 38.0, rect.center().y), Vec2::splat(48.0));
    if link {
        crate::ui::call::paint_link_avatar(ui, &palette, avatar_rect);
    } else {
        widgets::paint_avatar(
            ui,
            &palette,
            avatar_rect,
            &name,
            &message.chat,
            picture.as_deref(),
        );
    }
    let left = rect.left() + 76.0;
    let right = button_rect.left() - 6.0;
    let stamp_galley = ui
        .painter()
        .layout_no_wrap(stamp, theme::regular(11.5), palette.dim);
    let name_top = rect.top() + 14.0;
    ui.painter().galley(
        pos2(right - stamp_galley.size().x, name_top + 2.0),
        stamp_galley.clone(),
        palette.dim,
    );
    // A missed call's name is red, as on the phone.
    let name_color = if entry.missed() {
        palette.danger
    } else {
        palette.text
    };
    let name_line = widgets::line(
        ui,
        &label,
        theme::medium(14.5),
        name_color,
        (right - stamp_galley.size().x - 8.0 - left).max(0.0),
        1,
    );
    name_line.paint(ui, pos2(left, name_top), name_color);
    let glyph_rect = Rect::from_min_size(pos2(left, rect.top() + 39.0), Vec2::splat(15.0));
    theme::paint_icon(
        ui,
        entry.direction_icon(),
        glyph_rect,
        15.0,
        if entry.status.is_bad() {
            palette.danger
        } else {
            palette.accent
        },
    );
    let detail = widgets::line(
        ui,
        &summary,
        theme::regular(13.0),
        palette.secondary,
        (right - left - 21.0).max(0.0),
        1,
    );
    detail.paint(ui, pos2(left + 21.0, rect.top() + 38.0), palette.secondary);
}

/// What a call was and how it went, on one line: "Video call · 12 min 34 s", with how many
/// people were on a group call once its record says: "Group video call · 12 min 34 s ·
/// 4 participants".
fn summary(entry: &Entry, locale: Locale) -> String {
    let mut text = match entry.detail(locale) {
        Some(detail) => format!("{} · {detail}", entry.title(locale)),
        None => entry.title(locale),
    };
    if let Some(count) = entry.participants {
        text.push_str(" · ");
        text.push_str(&participants(locale, count));
    }
    text
}

/// How many people were on a call, in this language's own plural form.
fn participants(locale: Locale, count: u32) -> String {
    ngettext(locale, "{} participant", "{} participants", count).replace("{}", &count.to_string())
}

/// How many people the new-call list offers as frequently contacted.
const FREQUENT: usize = 3;

/// The people called most often, most first, from the call history; the
/// newer call breaks a tie. Groups and ourselves stay out.
pub fn frequent(calls: &[Message], me: Option<&str>) -> Vec<String> {
    let mut counts: Vec<(String, usize, i64)> = Vec::new();
    for message in calls {
        if !matches!(
            crate::model::ChatKind::from_id(&message.chat),
            crate::model::ChatKind::Direct
        ) || me == Some(message.chat.as_str())
            || message.chat == crate::model::CALL_LINK_CHAT
        {
            continue;
        }
        match counts.iter_mut().find(|(chat, ..)| *chat == message.chat) {
            Some((_, count, newest)) => {
                *count += 1;
                *newest = (*newest).max(message.timestamp);
            }
            None => counts.push((message.chat.clone(), 1, message.timestamp)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)));
    counts
        .into_iter()
        .take(FREQUENT)
        .map(|(chat, ..)| chat)
        .collect()
}

/// WhatsApp's new-call list: the people called most often, then every
/// contact, each with a voice and a video button. A row alone starts
/// nothing, so a stray click never rings anyone.
fn new_call_list(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let needle = crate::util::search_key(app.new_call_search.trim());
    let contacts = super::dialogs::contact_choices(app, &needle);
    let frequent: Vec<String> = if needle.is_empty() {
        let calls = visible_calls(app);
        frequent(&calls, app.me.as_deref())
            .into_iter()
            .filter(|chat| !app.chat(chat).is_some_and(|chat| chat.locked))
            .collect()
    } else {
        Vec::new()
    };
    let mut actions = Vec::new();
    egui::ScrollArea::vertical()
        .id_salt("new-call-contacts")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if !frequent.is_empty() {
                section(ui, &palette, &gettext(app.locale, "Frequently contacted"));
                for chat in &frequent {
                    ui.push_id(("frequent", chat), |ui| {
                        person(app, ui, chat, &mut actions);
                    });
                }
            }
            if contacts.is_empty() {
                ui.add_space(12.0);
                ui.vertical_centered(|ui| {
                    theme::text(
                        ui,
                        gettext(app.locale, "No matching contacts"),
                        theme::regular(13.0),
                        palette.secondary,
                    );
                });
            } else {
                section(ui, &palette, &gettext(app.locale, "All contacts"));
            }
            for contact in &contacts {
                ui.push_id(&contact.id, |ui| {
                    person(app, ui, &contact.id, &mut actions);
                });
            }
            ui.add_space(12.0);
        });
    if !actions.is_empty() {
        actions.push(Action::ShowNewCall(false));
    }
    app.actions.extend(actions);
}

fn section(ui: &mut egui::Ui, palette: &theme::Palette, label: &str) {
    ui.add_space(10.0);
    Frame::new()
        .inner_margin(Margin {
            left: 16,
            right: 14,
            top: 4,
            bottom: 6,
        })
        .show(ui, |ui| {
            theme::text(ui, label, theme::medium(13.0), palette.accent);
        });
}

/// One person on the new-call list, with a voice and a video button.
fn person(app: &mut App, ui: &mut egui::Ui, id: &str, actions: &mut Vec<Action>) {
    let palette = app.palette;
    let locale = app.locale;
    let name = app.display_name(id);
    let picture = app.avatar(id);
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), theme::ROW_HEIGHT),
        Sense::hover(),
    );
    let buttons = Rect::from_min_max(
        pos2(rect.right() - 92.0, rect.top()),
        pos2(rect.right() - 12.0, rect.bottom()),
    );
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            widgets::row_highlight(ui, &palette, rect, palette.surface_hover);
        }
        let avatar_rect =
            Rect::from_center_size(pos2(rect.left() + 38.0, rect.center().y), Vec2::splat(48.0));
        widgets::paint_avatar(ui, &palette, avatar_rect, &name, id, picture.as_deref());
        let left = rect.left() + 76.0;
        let width = (buttons.left() - 8.0 - left).max(0.0);
        let name_line = widgets::line(ui, &name, theme::medium(14.5), palette.text, width, 1);
        name_line.paint(ui, pos2(left, rect.top() + 14.0), palette.text);
        if let Some(phone) = crate::model::phone_of(id).map(crate::util::phone) {
            let phone_line = widgets::line(ui, &phone, theme::regular(13.0), palette.dim, width, 1);
            phone_line.paint(ui, pos2(left, rect.top() + 38.0), palette.dim);
        }
    }
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(buttons)
            .layout(Layout::right_to_left(Align::Center)),
    );
    let video = gettext(locale, "Video call {name}").replace("{name}", &name);
    if theme::icon_button(
        &mut child,
        Icon::Video,
        18.0,
        palette.secondary,
        palette.text,
        &video,
    )
    .clicked()
    {
        actions.push(Action::StartVideoCall(id.to_owned()));
    }
    child.add_space(4.0);
    let voice = gettext(locale, "Voice call {name}").replace("{name}", &name);
    if theme::icon_button(
        &mut child,
        Icon::Phone,
        18.0,
        palette.secondary,
        palette.text,
        &voice,
    )
    .clicked()
    {
        actions.push(Action::StartCall(id.to_owned()));
    }
}

/// The Calls tab's main pane when no chat is open beside the history:
/// WhatsApp's start of a call. An open chat shows there instead, the one
/// picked from the history or the one that was open before.
pub fn show(app: &mut App, ui: &mut egui::Ui) {
    start(app, ui);
}

/// The pane with no chat open: what WhatsApp shows there, a way to start a
/// call, and what ZapFast's calls are.
fn start(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let locale = app.locale;
    let height = ui.available_height();
    ui.vertical_centered(|ui| {
        ui.add_space((height / 2.0 - 110.0).max(24.0));
        let label = gettext(locale, "Start call");
        let response = theme::circle_button(
            ui,
            Icon::PhonePlus,
            72.0,
            palette.accent,
            palette.accent_hover,
            palette.on_accent,
            label.as_ref(),
        );
        if response.clicked() {
            app.actions.push(Action::ShowNewCall(true));
        }
        ui.add_space(10.0);
        theme::text(ui, label.as_ref(), theme::semibold(15.0), palette.text);
        ui.add_space(18.0);
        ui.set_max_width(360.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(gettext(
                    locale,
                    "Call one person by voice or video, or choose a call in the list to see it.",
                ))
                .font(theme::regular(13.5))
                .color(palette.secondary),
            )
            .wrap()
            .selectable(false),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADA: &str = "4917000000001@s.whatsapp.net";
    const GRACE: &str = "4917000000002@s.whatsapp.net";

    fn call(chat: &str, id: &str, from_me: bool, timestamp: i64, status: CallStatus) -> Message {
        Message {
            id: id.to_owned(),
            chat: chat.to_owned(),
            sender: chat.to_owned(),
            sender_name: None,
            from_me,
            timestamp,
            content: Content::Call {
                video: false,
                status,
                duration: 0,
                group: false,
                participants: None,
            },
            status: crate::model::Delivery::None,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: Vec::new(),
            history_order: None,
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        }
    }

    #[test]
    fn alike_calls_in_a_row_fold_into_one_history_row() {
        let log = [
            call(ADA, "1", false, 60, CallStatus::Missed),
            call(ADA, "2", false, 50, CallStatus::Missed),
            call(ADA, "3", false, 40, CallStatus::Answered),
            call(GRACE, "4", true, 30, CallStatus::Answered),
            call(ADA, "5", false, 20, CallStatus::Answered),
        ];
        let rows: Vec<Vec<&str>> = rows(&log)
            .iter()
            .map(|row| row.iter().map(|call| call.id.as_str()).collect())
            .collect();
        assert_eq!(rows, [vec!["1", "2"], vec!["3"], vec!["4"], vec!["5"]]);
    }

    #[test]
    fn the_new_call_list_offers_the_people_called_most() {
        let log = [
            call(GRACE, "1", true, 60, CallStatus::Answered),
            call(ADA, "2", false, 50, CallStatus::Missed),
            call(ADA, "3", false, 40, CallStatus::Answered),
            call(
                "120363000000000000@g.us",
                "4",
                false,
                30,
                CallStatus::Missed,
            ),
            call("me@s.whatsapp.net", "5", true, 20, CallStatus::Answered),
            call(
                crate::model::CALL_LINK_CHAT,
                "6",
                true,
                10,
                CallStatus::Answered,
            ),
        ];
        assert_eq!(frequent(&log, Some("me@s.whatsapp.net")), [ADA, GRACE]);
    }

    fn calls_app() -> (tempfile::TempDir, App, egui::Context) {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _events) = App::headless(
            crate::paths::AppDirs::under(directory.path()),
            crate::settings::Settings::default(),
        );
        app.page = crate::model::Page::Calls;
        app.call_log = vec![
            call(ADA, "1", false, 60, CallStatus::Missed),
            call(GRACE, "2", true, 30, CallStatus::Answered),
        ];
        app.call_log_loaded = true;
        let ctx = egui::Context::default();
        app.attach(&ctx);
        (directory, app, ctx)
    }

    fn frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 600.0))),
                events,
                ..Default::default()
            },
            |ui| list(app, ui),
        );
        output.textures_delta.clear();
    }

    fn click(at: egui::Pos2) -> Vec<egui::Event> {
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        vec![egui::Event::PointerMoved(at), press(true), press(false)]
    }

    #[test]
    fn a_history_row_opens_its_chat_and_its_button_calls_back() {
        let (_directory, mut app, ctx) = calls_app();
        frame(&mut app, &ctx, vec![]);
        let row = ctx.read_response(row_id(ADA, "1")).unwrap().rect;
        frame(&mut app, &ctx, click(row.left_center() + vec2(40.0, 0.0)));
        frame(&mut app, &ctx, vec![]);
        assert!(matches!(
            app.actions.as_slice(),
            [Action::PickCall { chat, message }] if chat == ADA && message == "1"
        ));
        app.actions.clear();
        // The call-back button sits at the row's right end.
        frame(&mut app, &ctx, click(row.right_center() - vec2(30.0, 0.0)));
        frame(&mut app, &ctx, vec![]);
        assert!(
            matches!(app.actions.as_slice(), [Action::StartCall(chat)] if chat == ADA),
            "{:?}",
            app.actions
        );
    }

    /// A group's call is called back in its group, as its chat's header
    /// starts one, while group calls are on.
    #[test]
    fn a_groups_call_is_called_back_in_the_group() {
        let _on = crate::calls::override_group_calls(true);
        let group = "120363000000000009@g.us";
        let actions = press_a_groups_call_back(group);
        assert!(
            matches!(actions.as_slice(), [Action::StartCall(chat)] if chat == group),
            "{actions:?}"
        );
    }

    /// With group calls off, a group's row has no button to call it back.
    #[test]
    fn with_group_calls_off_a_groups_call_is_not_called_back() {
        let _off = crate::calls::override_group_calls(false);
        let actions = press_a_groups_call_back("120363000000000009@g.us");
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, Action::StartCall(_) | Action::StartVideoCall(_))),
            "{actions:?}"
        );
    }

    /// Presses where a group call's row has its call-back button, and returns what that did.
    fn press_a_groups_call_back(group: &str) -> Vec<Action> {
        let (_directory, mut app, ctx) = calls_app();
        let mut entry = call(group, "8", true, 60, CallStatus::Answered);
        if let Content::Call { group, .. } = &mut entry.content {
            *group = true;
        }
        app.call_log.insert(0, entry);
        frame(&mut app, &ctx, vec![]);
        let row = ctx
            .read_response(row_id(group, "8"))
            .expect("the group's call is listed")
            .rect;
        frame(&mut app, &ctx, click(row.right_center() - vec2(30.0, 0.0)));
        frame(&mut app, &ctx, vec![]);
        std::mem::take(&mut app.actions)
    }

    /// A call joined through a link is listed as "Call link", with nothing
    /// to open and nobody to call back: a click on it does nothing.
    #[test]
    fn a_call_link_row_only_records_the_call() {
        let (_directory, mut app, ctx) = calls_app();
        let link = crate::model::CALL_LINK_CHAT;
        app.call_log
            .insert(0, call(link, "9", true, 90, CallStatus::Answered));
        app.calls_search = "call link".to_owned();
        frame(&mut app, &ctx, vec![]);
        let row = ctx
            .read_response(row_id(link, "9"))
            .expect("the row is listed and found by its name")
            .rect;
        frame(&mut app, &ctx, click(row.left_center() + vec2(40.0, 0.0)));
        frame(&mut app, &ctx, click(row.right_center() - vec2(30.0, 0.0)));
        frame(&mut app, &ctx, vec![]);
        assert!(app.actions.is_empty(), "{:?}", app.actions);
    }

    #[test]
    fn a_call_from_the_new_call_list_closes_it() {
        let (_directory, mut app, ctx) = calls_app();
        app.new_call = true;
        app.chats.push(crate::model::Chat::new(
            GRACE.to_owned(),
            "Grace".to_owned(),
        ));
        frame(&mut app, &ctx, vec![]);
        // The voice button is the second from the right on each row: the
        // first one a click down the list meets calls.
        let mut found = None;
        for y in (0..600).step_by(4) {
            let at = egui::pos2(400.0 - 12.0 - 30.0 - 4.0 - 15.0, y as f32);
            app.actions.clear();
            frame(&mut app, &ctx, click(at));
            frame(&mut app, &ctx, vec![]);
            if !app.actions.is_empty() {
                found = Some(app.actions.clone());
                break;
            }
        }
        let actions = found.expect("a voice button answers a click");
        assert!(
            matches!(
                actions.as_slice(),
                [Action::StartCall(_), Action::ShowNewCall(false)]
            ),
            "{actions:?}"
        );
    }

    fn entry(from_me: bool, video: bool, status: CallStatus) -> Entry {
        Entry {
            from_me,
            video,
            status,
            duration: 0,
            group: false,
            participants: None,
        }
    }

    /// Every word one frame of the history drew.
    fn drawn_rows(app: &mut App, ctx: &egui::Context) -> Vec<String> {
        fn texts(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, out)),
                _ => {}
            }
        }
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 600.0))),
                ..Default::default()
            },
            |ui| list(app, ui),
        );
        output.textures_delta.clear();
        let mut out = Vec::new();
        for clipped in &output.shapes {
            texts(&clipped.shape, &mut out);
        }
        out
    }

    fn group_entry(chat: &str, id: &str, count: Option<u32>) -> Message {
        let mut message = call(chat, id, true, 60, CallStatus::Answered);
        message.content = Content::Call {
            video: true,
            status: CallStatus::Answered,
            duration: 754,
            group: true,
            participants: count,
        };
        message
    }

    /// A group call's history row says how many people were on it, drawn as the rest of the
    /// summary is, and a one-to-one row says nothing of the kind.
    #[test]
    fn a_groups_history_row_says_how_many_were_on_the_call() {
        let (_directory, mut app, ctx) = calls_app();
        let group = "120363000000000009@g.us";
        app.call_log.insert(0, group_entry(group, "g", Some(4)));
        frame(&mut app, &ctx, vec![]);
        let drawn = drawn_rows(&mut app, &ctx);
        let counted: Vec<&String> = drawn
            .iter()
            .filter(|text| text.contains("participant"))
            .collect();
        assert_eq!(
            counted,
            ["Group video call · 12 min 34 s · 4 participants"],
            "only the group's row is counted: {drawn:?}"
        );
        // The one-to-one rows the same list holds keep to what they said before.
        assert!(
            drawn.iter().any(|text| text == "Missed voice call"),
            "{drawn:?}"
        );
        assert!(drawn.iter().any(|text| text == "Voice call"), "{drawn:?}");
    }

    /// The count is a plural message of its own, so one person reads as one and the rest as many,
    /// and it follows the call whether or not anyone answered.
    #[test]
    fn a_rows_count_reads_as_one_or_as_many() {
        let locale = Locale::default();
        let mut alone = entry(true, false, CallStatus::Answered);
        alone.group = true;
        alone.participants = Some(1);
        assert_eq!(summary(&alone, locale), "Group voice call · 1 participant");
        let many = Entry {
            participants: Some(12),
            ..alone
        };
        assert_eq!(summary(&many, locale), "Group voice call · 12 participants");
        let missed = Entry {
            from_me: false,
            status: CallStatus::Missed,
            participants: Some(3),
            ..alone
        };
        assert_eq!(
            summary(&missed, locale),
            "Missed group voice call · 3 participants",
            "the title already says it went unanswered, and the count still stands"
        );
    }

    /// A group call whose record names nobody, and one filed before a count was kept, reads as it
    /// did before there was one: the title, then how the call went.
    #[test]
    fn a_group_row_without_a_count_says_only_what_it_used_to() {
        let locale = Locale::default();
        let mut older = entry(true, true, CallStatus::Answered);
        older.group = true;
        older.duration = 754;
        assert_eq!(older.participants, None);
        assert_eq!(summary(&older, locale), "Group video call · 12 min 34 s");
    }

    #[test]
    fn the_states_a_reader_tells_apart_have_different_glyphs() {
        let glyphs = [
            entry(false, false, CallStatus::Answered).icon(),
            entry(true, false, CallStatus::Answered).icon(),
            entry(false, false, CallStatus::Missed).icon(),
            entry(false, false, CallStatus::Declined).icon(),
            entry(true, true, CallStatus::Answered).icon(),
        ];
        for (index, first) in glyphs.iter().enumerate() {
            for second in &glyphs[index + 1..] {
                assert_ne!(first, second);
            }
        }
    }

    #[test]
    fn an_entry_says_what_the_call_was_and_how_it_went() {
        let locale = Locale::default();
        let mut answered = entry(true, false, CallStatus::Answered);
        answered.duration = 221;
        assert_eq!(answered.title(locale), "Voice call");
        assert_eq!(answered.detail(locale).as_deref(), Some("3 min 41 s"));
        let missed = entry(false, true, CallStatus::Missed);
        assert_eq!(missed.title(locale), "Missed video call");
        assert_eq!(missed.detail(locale), None, "the title already says it");
        let refused = entry(true, false, CallStatus::NoAnswer);
        assert_eq!(refused.detail(locale).as_deref(), Some("No answer"));
    }

    /// The count comes off the message, so a row shows what the entry that was filed holds.
    #[test]
    fn a_row_reads_the_count_off_the_entry_it_lists() {
        let message = group_entry("120363000000000009@g.us", "g", Some(4));
        let entry = Entry::of(&message).expect("a call entry");
        assert_eq!(entry.participants, Some(4));
        assert!(entry.group);
        let one_to_one = call(ADA, "1", true, 60, CallStatus::Answered);
        assert_eq!(Entry::of(&one_to_one).unwrap().participants, None);
    }
}
