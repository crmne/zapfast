//! The Calls view: every 1:1 call this account placed or took, newest first.
//!
//! The list is read from the archive, never from the live call: what a call became is decided once,
//! when it ends, and this view only draws what was written down. Nothing here calls anyone by
//! accident either: a row's call buttons are the only way to start a call from this screen, and a
//! click anywhere else opens the chat the call belongs to.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Sense};

use crate::app::App;
use crate::i18n::{Locale, gettext};
use crate::model::{Action, CallMedia, CallRecord, CallStatus, Page};
use crate::theme::{self, Icon};
use crate::ui::widgets;

/// The row's height, which also sets how much room a list of them needs.
const ROW: f32 = 62.0;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    header(app, ui);
    ui.add_space(2.0);
    // Cloned so the rows can push actions while the list they come from is borrowed from the app.
    // A locked chat's calls are left out entirely while its folder is closed: this page is not a
    // place to learn who a hidden chat has been talking to, or when.
    let calls: Vec<CallRecord> = app
        .call_log
        .iter()
        .filter(|record| !app.chat_is_private(&record.chat))
        .cloned()
        .collect();
    if calls.is_empty() {
        widgets::empty_state(
            ui,
            &palette,
            Icon::Phone,
            gettext(app.locale, "No calls yet").as_ref(),
            gettext(
                app.locale,
                "Calls you make and take with ZapFast are listed here.",
            )
            .as_ref(),
        );
        return;
    }
    let mut actions = Vec::new();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for record in &calls {
                row(app, ui, record, &mut actions);
            }
            ui.add_space(12.0);
        });
    app.actions.extend(actions);
}

fn header(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    Frame::new()
        .inner_margin(Margin {
            left: 14,
            right: 14,
            top: 12,
            bottom: 8,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if theme::icon_button(
                    ui,
                    Icon::ArrowLeft,
                    18.0,
                    palette.secondary,
                    palette.text,
                    gettext(app.locale, "Back to chats").as_ref(),
                )
                .clicked()
                {
                    app.actions.push(Action::Open(Page::Chats));
                }
                ui.add_space(2.0);
                theme::text(
                    ui,
                    gettext(app.locale, "Calls"),
                    theme::bold(20.0),
                    palette.text,
                );
            });
        });
}

/// One call: who, which way, what it carried, how it ended, how long it lasted, and when.
fn row(app: &mut App, ui: &mut egui::Ui, record: &CallRecord, actions: &mut Vec<Action>) {
    let palette = app.palette;
    let locale = app.locale;
    let name = app.call_name(&record.chat);
    let picture = app.call_avatar(&record.chat);
    let missed = record.status.missed();
    // Register the whole-row hit target before the call-back buttons it contains, so a button keeps
    // its own click instead of the row also opening the chat. The row has a fixed height, so its
    // rectangle is known before it is drawn; the transcript uses the same parent-before-child order.
    let row = ui
        .interact(
            egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), ROW)),
            ui.id().with(("call-row", &record.id)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    Frame::new()
        .fill(palette.surface)
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_min_height(ROW - 16.0);
            ui.horizontal(|ui| {
                widgets::avatar(ui, &palette, &name, &record.chat, 40.0, picture.as_deref());
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.add_space(2.0);
                    theme::text(ui, &name, theme::semibold(14.5), palette.text);
                    ui.add_space(1.0);
                    theme::text(
                        ui,
                        direction_and_media(locale, record),
                        theme::medium(12.5),
                        if missed {
                            palette.danger
                        } else {
                            palette.secondary
                        },
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    theme::text(
                        ui,
                        crate::util::chat_stamp(locale, record.started_at),
                        theme::medium(12.0),
                        palette.secondary,
                    );
                    ui.add_space(10.0);
                    theme::text(
                        ui,
                        outcome(locale, record),
                        theme::medium(12.5),
                        if missed {
                            palette.danger
                        } else {
                            palette.secondary
                        },
                    );
                    ui.add_space(10.0);
                    let voice = gettext(locale, "Call back").into_owned();
                    if theme::icon_button(
                        ui,
                        Icon::Phone,
                        16.0,
                        palette.secondary,
                        palette.text,
                        &voice,
                    )
                    .clicked()
                    {
                        actions.push(Action::CallBack {
                            chat: record.chat.clone(),
                            video: false,
                        });
                    }
                    let video = gettext(locale, "Call back with video").into_owned();
                    if theme::icon_button(
                        ui,
                        Icon::Video,
                        16.0,
                        palette.secondary,
                        palette.text,
                        &video,
                    )
                    .clicked()
                    {
                        actions.push(Action::CallBack {
                            chat: record.chat.clone(),
                            video: true,
                        });
                    }
                });
            });
        });
    if row.clicked() {
        actions.push(Action::OpenCallChat(record.chat.clone()));
    }
    ui.add_space(6.0);
}

/// Which way the call went and what it carried, as one phrase the reader's language owns.
///
/// Shared with the transcript, which shows the same entry next to the messages it belongs to.
pub(crate) fn direction_and_media(locale: Locale, record: &CallRecord) -> String {
    match (record.direction, record.media) {
        (crate::model::CallDirection::Outgoing, CallMedia::Voice) => {
            gettext(locale, "Outgoing voice call").into_owned()
        }
        (crate::model::CallDirection::Incoming, CallMedia::Voice) => {
            gettext(locale, "Incoming voice call").into_owned()
        }
        (crate::model::CallDirection::Outgoing, CallMedia::Video) => {
            gettext(locale, "Outgoing video call").into_owned()
        }
        (crate::model::CallDirection::Incoming, CallMedia::Video) => {
            gettext(locale, "Incoming video call").into_owned()
        }
    }
}

/// How the call ended: how long it lasted when it was answered, and what became of it otherwise.
pub(crate) fn outcome(locale: Locale, record: &CallRecord) -> String {
    if record.status.connected() {
        return crate::util::duration(record.duration as u32);
    }
    match record.status {
        CallStatus::Answered => crate::util::duration(record.duration as u32),
        // Another device took the call. This device has no length to show, only that fact.
        CallStatus::AnsweredElsewhere => gettext(locale, "Answered elsewhere").into_owned(),
        CallStatus::Missed => gettext(locale, "Missed").into_owned(),
        CallStatus::Declined => gettext(locale, "Declined").into_owned(),
        CallStatus::Busy => gettext(locale, "Busy").into_owned(),
        CallStatus::Failed => gettext(locale, "Could not connect").into_owned(),
        CallStatus::NoAnswer => gettext(locale, "No answer").into_owned(),
        CallStatus::ConnectionLost => gettext(locale, "Connection lost").into_owned(),
    }
}
