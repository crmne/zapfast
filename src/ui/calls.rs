//! Calls as the history shows them: the entry a call leaves among a chat's
//! messages, and the Calls tab, laid out as WhatsApp's: the call history in
//! the list pane (or the new-call list over it), and beside it a call's
//! details or the way to start one.
//!
//! Both read the same entries, which come from WhatsApp's own records (the
//! phone's call log and the call-log messages in history) and from the calls
//! ZapFast took part in.

use egui::{Align, Frame, Layout, Margin, Rect, Sense, Vec2, pos2, vec2};

use crate::app::App;
use crate::i18n::{Locale, gettext};
use crate::model::{Action, CallStatus, Content, Message, Page};
use crate::theme::{self, Icon};
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
            } => Some(Self {
                from_me: message.from_me,
                video,
                status,
                duration,
                group,
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

/// Whether `id`'s name or number matches `needle`, a `search_key`.
fn matches(app: &App, id: &str, needle: &str) -> bool {
    needle.is_empty()
        || crate::util::search_key(&app.display_name(id)).contains(needle)
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
    let needle = crate::util::search_key(app.calls_search.trim());
    let rows: Vec<Vec<Message>> = rows(&calls)
        .into_iter()
        .filter(|row| matches(app, &row[0].chat, &needle))
        .map(|row| row.into_iter().cloned().collect())
        .collect();
    let mut actions = Vec::new();
    crate::autoscroll::show(
        ui,
        egui::ScrollArea::vertical()
            .id_salt("call-history")
            .auto_shrink([false, false]),
        |ui| {
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
        },
    );
    app.actions.extend(actions);
}

/// Stable id of a history row, for tests.
pub fn row_id(chat: &str, message: &str) -> egui::Id {
    egui::Id::new(("call-row", chat, message))
}

/// One history row, as WhatsApp draws it: the person, a count when calls
/// fold together, the way the newest went and what it was, when, and a
/// button to call back the same way. A click shows the call details beside
/// the list.
fn history_row(app: &mut App, ui: &mut egui::Ui, row: &[Message], actions: &mut Vec<Action>) {
    let message = &row[0];
    let Some(entry) = Entry::of(message) else {
        return;
    };
    let palette = app.palette;
    let locale = app.locale;
    let name = app.display_name(&message.chat);
    let picture = app.avatar(&message.chat);
    let selected = app
        .call_info
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
    if response.clicked() {
        actions.push(Action::OpenCallInfo {
            chat: message.chat.clone(),
            message: message.id.clone(),
        });
    }
    // One-to-one calls only: a group call has no way back from here.
    let button_rect = Rect::from_center_size(
        pos2(rect.right() - 30.0, rect.center().y),
        Vec2::splat(34.0),
    );
    if !entry.group {
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
    widgets::paint_avatar(
        ui,
        &palette,
        avatar_rect,
        &name,
        &message.chat,
        picture.as_deref(),
    );
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

/// What a call was and how it went, on one line: "Video call · 12 min 34 s".
fn summary(entry: &Entry, locale: Locale) -> String {
    match entry.detail(locale) {
        Some(detail) => format!("{} · {detail}", entry.title(locale)),
        None => entry.title(locale),
    }
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
    crate::autoscroll::show(
        ui,
        egui::ScrollArea::vertical()
            .id_salt("new-call-contacts")
            .auto_shrink([false, false]),
        |ui| {
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
        },
    );
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

/// The Calls tab's main pane: a history row's call details, or WhatsApp's
/// start of a call when no row is chosen.
pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let calls = visible_calls(app);
    let chosen = app.call_info.clone().and_then(|(chat, _)| {
        let mine: Vec<Message> = calls
            .iter()
            .filter(|message| message.chat == chat)
            .cloned()
            .collect();
        (!mine.is_empty()).then_some((chat, mine))
    });
    match chosen {
        Some((chat, calls)) => info(app, ui, &chat, &calls),
        None => start(app, ui),
    }
}

/// Where a call details header stands, as the conversation header does, so
/// the two panes' first rows line up.
fn pane_header(app: &mut App, ui: &mut egui::Ui, title: &str, close: Option<Action>) {
    let palette = app.palette;
    let header = Frame::new()
        .fill(palette.panel)
        .inner_margin(Margin::symmetric(14, 8))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if theme::macos_chrome(ui.ctx()) {
                super::titlebar_drag(ui, ui.max_rect());
            }
            ui.horizontal(|ui| {
                ui.set_min_height(super::conversation::HEADER_ROW);
                if super::narrow(ui.ctx()) && theme::macos_chrome(ui.ctx()) {
                    ui.add_space(theme::traffic_light_inset(ui.ctx()));
                }
                if let Some(close) = close {
                    // A narrow window shows the details instead of the list:
                    // Back returns to it. Beside the list, they close.
                    let (icon, label) = if super::narrow(ui.ctx()) {
                        (Icon::ArrowLeft, gettext(app.locale, "Back to the calls"))
                    } else {
                        (Icon::X, gettext(app.locale, "Close"))
                    };
                    if theme::icon_button(
                        ui,
                        icon,
                        18.0,
                        palette.secondary,
                        palette.text,
                        label.as_ref(),
                    )
                    .clicked()
                    {
                        app.actions.push(close);
                    }
                    ui.add_space(2.0);
                }
                theme::text(ui, title, theme::bold(17.0), palette.text);
            });
        });
    let rect = header.response.rect;
    widgets::paint_shadow_below(ui, &palette, rect.left(), rect.right(), rect.bottom());
}

/// The pane with no call chosen: what WhatsApp shows there, a way to start
/// a call, and what ZapFast's calls are.
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

/// A history row's details, as WhatsApp's call info shows them: the person
/// with Message, Voice call and Video call, and every call with them by
/// day. A call opens the chat at it.
fn info(app: &mut App, ui: &mut egui::Ui, chat: &str, calls: &[Message]) {
    let palette = app.palette;
    let locale = app.locale;
    pane_header(
        app,
        ui,
        gettext(locale, "Call info").as_ref(),
        Some(Action::CloseCallInfo),
    );
    let name = app.display_name(chat);
    let picture = app.avatar(chat);
    let group = calls.iter().filter_map(Entry::of).any(|entry| entry.group);
    let mut actions = Vec::new();
    crate::autoscroll::show(
        ui,
        egui::ScrollArea::vertical()
            .id_salt("call-info")
            .auto_shrink([false, false]),
        |ui| {
            Frame::new()
                .inner_margin(Margin::symmetric(24, 18))
                .show(ui, |ui| {
                    ui.set_max_width(640.0_f32.min(ui.available_width()));
                    ui.horizontal(|ui| {
                        widgets::avatar(ui, &palette, &name, chat, 56.0, picture.as_deref());
                        ui.add_space(12.0);
                        ui.vertical(|ui| {
                            ui.add_space(6.0);
                            theme::text(ui, &name, theme::semibold(17.0), palette.text);
                            if let Some(phone) = crate::model::phone_of(chat)
                                .map(crate::util::phone)
                                .filter(|phone| *phone != name)
                            {
                                theme::text(ui, phone, theme::regular(13.0), palette.secondary);
                            }
                        });
                    });
                    ui.add_space(14.0);
                    ui.horizontal_wrapped(|ui| {
                        let message = gettext(locale, "Message");
                        if theme::soft_button(
                            ui,
                            &palette,
                            Some(Icon::MessageCircle),
                            message.as_ref(),
                            false,
                        )
                        .clicked()
                        {
                            actions.push(Action::Open(Page::Chats));
                            actions.push(Action::OpenChat(chat.to_owned()));
                        }
                        if !group {
                            let voice = gettext(locale, "Voice call");
                            if theme::soft_button(
                                ui,
                                &palette,
                                Some(Icon::Phone),
                                voice.as_ref(),
                                false,
                            )
                            .clicked()
                            {
                                actions.push(Action::StartCall(chat.to_owned()));
                            }
                            let video = gettext(locale, "Video call");
                            if theme::soft_button(
                                ui,
                                &palette,
                                Some(Icon::Video),
                                video.as_ref(),
                                false,
                            )
                            .clicked()
                            {
                                actions.push(Action::StartVideoCall(chat.to_owned()));
                            }
                        }
                    });
                    ui.add_space(18.0);
                    let mut day = String::new();
                    for message in calls {
                        let label = crate::util::day_label(locale, message.timestamp);
                        if label != day {
                            ui.add_space(8.0);
                            theme::text(ui, &label, theme::medium(13.0), palette.secondary);
                            ui.add_space(4.0);
                            day = label;
                        }
                        info_row(app, ui, message, &mut actions);
                    }
                });
        },
    );
    app.actions.extend(actions);
}

/// One call in the details: which way it went and what it was, at what
/// time, and how it went.
fn info_row(app: &App, ui: &mut egui::Ui, message: &Message, actions: &mut Vec<Action>) {
    let Some(entry) = Entry::of(message) else {
        return;
    };
    let palette = app.palette;
    let locale = app.locale;
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 44.0), Sense::click());
    theme::reveal_focus(&response);
    let title = entry.title(locale);
    let time = crate::util::clock(message.timestamp);
    let detail = entry.detail(locale).unwrap_or_default();
    let spoken = format!("{title}, {time}, {detail}");
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), spoken.as_str())
    });
    let response = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(gettext(locale, "Show in chat"));
    if response.clicked() {
        actions.push(Action::Open(Page::Chats));
        actions.push(Action::OpenMessage {
            chat: message.chat.clone(),
            message: message.id.clone(),
        });
    }
    if !ui.is_rect_visible(rect) {
        return;
    }
    if response.hovered() {
        widgets::row_highlight(ui, &palette, rect, palette.surface_hover);
    }
    let glyph =
        Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), Vec2::splat(16.0));
    theme::paint_icon(
        ui,
        entry.direction_icon(),
        glyph,
        16.0,
        if entry.status.is_bad() {
            palette.danger
        } else {
            palette.accent
        },
    );
    let left = rect.left() + 40.0;
    let detail_galley =
        ui.painter()
            .layout_no_wrap(detail, theme::regular(13.0), palette.secondary);
    let right = rect.right() - 12.0;
    ui.painter().galley(
        pos2(
            right - detail_galley.size().x,
            rect.center().y - detail_galley.size().y / 2.0,
        ),
        detail_galley.clone(),
        palette.secondary,
    );
    let text = format!("{title} · {time}");
    let line = widgets::line(
        ui,
        &text,
        theme::regular(14.0),
        palette.text,
        (right - detail_galley.size().x - 12.0 - left).max(0.0),
        1,
    );
    line.paint(
        ui,
        pos2(left, rect.center().y - line.size().y / 2.0),
        palette.text,
    );
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
        ];
        assert_eq!(frequent(&log, Some("me@s.whatsapp.net")), [ADA, GRACE]);
    }

    fn calls_app() -> (tempfile::TempDir, App, egui::Context) {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _events) = App::headless(
            crate::paths::AppDirs::under(directory.path()),
            crate::settings::Settings::default(),
        );
        app.page = Page::Calls;
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
    fn a_history_row_opens_its_details_and_its_button_calls_back() {
        let (_directory, mut app, ctx) = calls_app();
        frame(&mut app, &ctx, vec![]);
        let row = ctx.read_response(row_id(ADA, "1")).unwrap().rect;
        frame(&mut app, &ctx, click(row.left_center() + vec2(40.0, 0.0)));
        frame(&mut app, &ctx, vec![]);
        assert!(matches!(
            app.actions.as_slice(),
            [Action::OpenCallInfo { chat, message }] if chat == ADA && message == "1"
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
        }
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
}
