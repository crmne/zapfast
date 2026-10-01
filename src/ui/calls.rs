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
        // A log that has not been read yet is not an empty one: privacy recovery can delay the
        // read, and "No calls yet" would tell someone with a history that it is gone. The loading
        // state holds until the backend has really answered.
        let (title, body) = if app.call_log_loaded {
            (
                gettext(app.locale, "No calls yet"),
                gettext(
                    app.locale,
                    "Calls you make and take with ZapFast are listed here.",
                ),
            )
        } else {
            (
                gettext(app.locale, "Loading…"),
                gettext(app.locale, "Reading this account's call history."),
            )
        };
        widgets::empty_state(ui, &palette, Icon::Phone, title.as_ref(), body.as_ref());
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
    let bad = state_is_bad(record.status);
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
        .fill(if row.hovered() {
            palette.surface_hover
        } else {
            palette.surface
        })
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
                    ui.horizontal(|ui| {
                        // One glyph for the whole state: the arrow says which way the call went
                        // and the camera says what it carried, read at a glance before the text.
                        theme::icon(
                            ui,
                            state_icon(record),
                            13.0,
                            if bad {
                                palette.danger
                            } else {
                                palette.accent
                            },
                        );
                        ui.add_space(5.0);
                        theme::text(
                            ui,
                            direction_and_media(locale, record),
                            theme::medium(12.5),
                            if bad {
                                palette.danger
                            } else {
                                palette.secondary
                            },
                        );
                    });
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
                        if bad {
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
                    // Only where the backend can really carry video: the same capability gate the
                    // chat header uses, so an unsupported platform is not offered an action that
                    // can only fail.
                    if crate::calls::capabilities().video {
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
                    }
                });
            });
        });
    // The whole row is a control, so a screen reader has to be told what it does and the keyboard
    // has to be able to reach it: a painted hit target is invisible to both otherwise.
    let label = gettext(app.locale, "Open the chat with {name}").replace("{name}", &name);
    row.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.as_str()));
    theme::reveal_focus(&row);
    if row.clicked() {
        actions.push(Action::OpenCallChat(record.chat.clone()));
    }
    ui.add_space(6.0);
}

/// The one shape that stands for a finished call: which way it went and what became of it, in a
/// single glyph.
///
/// The shape answers "what happened"; the colour answers "did it connect". A call that rang and
/// was never picked up gets the missed handset, a call that was put down or never came up gets
/// the slashed one, and a call that connected gets the arrow that says which way — with the
/// camera in place of the handset when it carried video. Shared with the transcript entry, which
/// draws the same call.
pub(crate) fn state_icon(record: &CallRecord) -> Icon {
    use crate::model::{CallDirection, CallMedia, CallStatus};
    match record.status {
        // Rang out, whether the ring was here or on the far phone.
        CallStatus::Missed | CallStatus::NoAnswer => Icon::PhoneMissed,
        // Put down — by either side, on either device — or never came up at all.
        CallStatus::Declined
        | CallStatus::DeclinedElsewhere
        | CallStatus::Busy
        | CallStatus::Failed
        | CallStatus::ConnectionLost => Icon::PhoneOff,
        // Connected, here or on another of the account's devices.
        CallStatus::Answered | CallStatus::AnsweredElsewhere => {
            let video = record.media == CallMedia::Video;
            match (record.direction, video) {
                (CallDirection::Incoming, false) => Icon::PhoneIncoming,
                (CallDirection::Outgoing, false) => Icon::PhoneOutgoing,
                (CallDirection::Incoming, true) => Icon::VideoIncoming,
                (CallDirection::Outgoing, true) => Icon::VideoOutgoing,
            }
        }
    }
}

/// Whether a finished call reads with the destructive colour. A call the reader missed, refused,
/// or that never connected is a call to notice; an answered one, here or elsewhere, is not.
pub(crate) fn state_is_bad(status: CallStatus) -> bool {
    matches!(
        status,
        CallStatus::Missed
            | CallStatus::Declined
            | CallStatus::DeclinedElsewhere
            | CallStatus::Busy
            | CallStatus::Failed
            | CallStatus::NoAnswer
            | CallStatus::ConnectionLost
    )
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
        CallStatus::DeclinedElsewhere => gettext(locale, "Declined elsewhere").into_owned(),
        CallStatus::Busy => gettext(locale, "Busy").into_owned(),
        CallStatus::Failed => gettext(locale, "Could not connect").into_owned(),
        CallStatus::NoAnswer => gettext(locale, "No answer").into_owned(),
        CallStatus::ConnectionLost => gettext(locale, "Connection lost").into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CallDirection, CallMedia, CallRecord};

    fn record(direction: CallDirection, media: CallMedia, status: CallStatus) -> CallRecord {
        CallRecord {
            id: "test".into(),
            chat: "chat".into(),
            started_at: 0,
            ended_at: 0,
            direction,
            media,
            status,
            duration: 0,
        }
    }

    /// The four states a reader has to tell apart before reading anything are four shapes, not one
    /// handset in four colours.
    #[test]
    fn the_glanceable_call_states_are_four_different_glyphs() {
        let shapes = [
            state_icon(&record(
                CallDirection::Incoming,
                CallMedia::Voice,
                CallStatus::Answered,
            )),
            state_icon(&record(
                CallDirection::Outgoing,
                CallMedia::Voice,
                CallStatus::Answered,
            )),
            state_icon(&record(
                CallDirection::Incoming,
                CallMedia::Voice,
                CallStatus::Missed,
            )),
            state_icon(&record(
                CallDirection::Incoming,
                CallMedia::Voice,
                CallStatus::Declined,
            )),
        ];
        for (index, first) in shapes.iter().enumerate() {
            for second in &shapes[index + 1..] {
                assert_ne!(first, second, "two call states share a glyph: {first:?}");
            }
        }
    }

    /// A video call keeps its direction, with the camera where the handset would be.
    #[test]
    fn a_video_call_keeps_its_direction_in_the_shape() {
        assert_eq!(
            state_icon(&record(
                CallDirection::Incoming,
                CallMedia::Video,
                CallStatus::Answered
            )),
            Icon::VideoIncoming
        );
        assert_eq!(
            state_icon(&record(
                CallDirection::Outgoing,
                CallMedia::Video,
                CallStatus::Answered
            )),
            Icon::VideoOutgoing
        );
        // Video or not, a call that never connected still says so with the phone shapes.
        assert_eq!(
            state_icon(&record(
                CallDirection::Outgoing,
                CallMedia::Video,
                CallStatus::Declined
            )),
            Icon::PhoneOff
        );
    }

    /// Only a call that did not connect reads with the destructive colour; an answered call on
    /// another device is a call that happened.
    #[test]
    fn a_call_that_never_connected_is_the_one_that_reads_red() {
        for bad in [
            CallStatus::Missed,
            CallStatus::Declined,
            CallStatus::DeclinedElsewhere,
            CallStatus::Busy,
            CallStatus::Failed,
            CallStatus::NoAnswer,
            CallStatus::ConnectionLost,
        ] {
            assert!(state_is_bad(bad), "{bad:?} should read red");
        }
        assert!(!state_is_bad(CallStatus::Answered));
        assert!(!state_is_bad(CallStatus::AnsweredElsewhere));
    }
}
