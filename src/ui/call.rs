//! The call surface: a full-window screen for a live 1:1 call, and the prompt for one that rings.
//!
//! Nothing here decides what a call is doing. The phase, the point the duration counts from, the
//! mute state and the device selections all come from [`crate::calls::CallUpdate`], which the worker
//! fills from the backend's own events. This module only draws them, and pushes [`Action`]s when the
//! user presses something.

use std::path::Path;
use std::time::Instant;

use egui::{Align, Color32, CornerRadius, Layout, Rect, Sense, TextureOptions, Vec2, pos2, vec2};

use crate::app::App;
use crate::calls::{CallOutcome, CallPhase, CallUpdate, PeerAudio, SilenceReason};
use crate::i18n::{Locale, gettext};
use crate::model::Action;
use crate::theme::{self, Icon, Palette};
use crate::ui::widgets;

/// The area's id, so the surface keeps one place in the layer order for the life of the call.
const SURFACE: &str = "zapfast-call-surface";
/// The bar a call the reader stepped away from keeps on screen.
const MINIBAR: &str = "zapfast-call-bar";
/// Padding between the window edge and everything the surface draws.
const PADDING: f32 = 26.0;
/// The main round controls.
const CONTROL: f32 = 58.0;
/// How much of the surface the corner preview may take, so a portrait camera stays a preview.
const PREVIEW_WIDTH_SHARE: f32 = 0.34;
const PREVIEW_HEIGHT_SHARE: f32 = 0.38;

/// Draws the call surface, if there is one.
///
/// A live call the reader stepped away from is not drawn over the window at all: it leaves the chat
/// it interrupted readable and keeps a bar at the bottom, so nothing is lost and nothing is in the
/// way. The call itself is never touched by stepping away, and hanging up is the only thing that
/// ends it.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(call) = app.call.clone() else {
        return;
    };
    let palette = app.palette;
    // The call's own name and picture: no phone number for a caller with no name, and nothing at all
    // for a chat the lock is hiding.
    let peer = app.call_name(&call.chat);
    let picture = app.call_avatar(&call.chat);
    if app.call_surface_hidden && call.phase.is_live() {
        bar(app, ctx, &call, &peer, &palette);
        return;
    }
    // The whole window, minus the strip macOS reserves for the traffic lights, which the surface
    // must not paint over.
    let screen = ctx.content_rect();
    let screen = match theme::titlebar_inset(ctx) {
        0.0 => screen,
        inset => screen.with_min_y(screen.top() + inset),
    };

    egui::Area::new(egui::Id::new(SURFACE))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            ui.set_min_size(screen.size());
            egui::Frame::new()
                .fill(surface_color(&palette, &call))
                .show(ui, |ui| {
                    ui.set_min_size(screen.size());
                    let inner = screen.shrink(PADDING);
                    ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                        if call.phase == CallPhase::Incoming {
                            ringing(ui, app, &call, &peer, picture.as_deref(), &palette, inner);
                        } else {
                            live(ui, app, &call, &peer, picture.as_deref(), &palette, inner);
                        }
                    });
                });
        });
}

/// The backdrop: near-black under a peer's picture, a window colour otherwise.
fn surface_color(palette: &Palette, call: &CallUpdate) -> Color32 {
    if call.video && call.remote_video {
        Color32::from_rgb(8, 10, 12)
    } else if palette.dark {
        Color32::from_rgb(18, 21, 24)
    } else {
        palette.window
    }
}

/// A call that is up, coming up, or just over.
fn live(
    ui: &mut egui::Ui,
    app: &mut App,
    call: &CallUpdate,
    peer: &str,
    picture: Option<&Path>,
    palette: &Palette,
    area: Rect,
) {
    // The peer's picture fills the surface when there is one; everything else is drawn on top of
    // it. Both textures are re-uploaded by name, so no handle is kept between frames.
    if call.video
        && let Some(image) = app.call_remote_frame.clone()
    {
        let at = fit_inside(area, vec2(image.width() as f32, image.height() as f32));
        let texture = ui.ctx().load_texture(
            "zapfast-call-remote",
            image.as_ref().clone(),
            TextureOptions::LINEAR,
        );
        ui.painter().image(
            texture.id(),
            at,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    // Our own camera sits in the top corner, where a call app puts it, at its own shape: a camera
    // held upright previews upright rather than being stretched into a landscape box.
    if call.video
        && let Some(image) = app.call_local_frame.clone()
    {
        let size = fit_inside(
            Rect::from_min_size(
                area.min,
                vec2(
                    area.width() * PREVIEW_WIDTH_SHARE,
                    area.height() * PREVIEW_HEIGHT_SHARE,
                ),
            ),
            vec2(image.width() as f32, image.height() as f32),
        )
        .size();
        let at = Rect::from_min_size(pos2(area.right() - size.x, area.top()), size);
        ui.painter()
            .rect_filled(at.expand(3.0), CornerRadius::same(12), palette.outline);
        let texture = ui.ctx().load_texture(
            "zapfast-call-local",
            image.as_ref().clone(),
            TextureOptions::LINEAR,
        );
        ui.painter().image(
            texture.id(),
            at,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }

    let has_picture = call.video && call.remote_video;
    if !has_picture {
        ui.vertical_centered(|ui| {
            ui.add_space(area.height() * 0.16);
            let size = (area.height() * 0.26).clamp(96.0, 168.0);
            widgets::avatar(ui, palette, peer, &call.chat, size, picture);
            ui.add_space(18.0);
            theme::text(ui, peer, theme::bold(26.0), palette.text);
            ui.add_space(6.0);
            theme::text(
                ui,
                status(app, call),
                theme::medium(15.0),
                status_color(call, palette),
            );
            under_status(ui, app, call, palette, false);
        });
    } else {
        // The name and the timer ride on top of the picture, which is why they are painted rather
        // than laid out: the picture owns the whole surface.
        ui.vertical_centered(|ui| {
            ui.add_space(6.0);
            theme::text(ui, peer, theme::bold(17.0), Color32::WHITE);
            theme::text(
                ui,
                status(app, call),
                theme::medium(13.0),
                Color32::from_white_alpha(190),
            );
            under_status(ui, app, call, palette, true);
        });
    }

    // Stepping away from the screen without ending the call, so a chat can be read while the call
    // runs. The bar at the bottom offers the way back.
    if call.phase.is_live() {
        let back = gettext(app.locale, "Back to the chat").into_owned();
        let response = control(
            ui,
            Icon::ArrowLeft,
            38.0,
            palette.surface_active,
            palette.text,
            &back,
            true,
        );
        if response.clicked() {
            app.actions.push(Action::LeaveCallSurface);
        }
    }

    ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
        ui.add_space(6.0);
        if app.call_devices_open {
            devices(ui, app, call);
            ui.add_space(12.0);
        }
        controls(ui, app, call, palette);
    });
}

/// The bar that keeps a call the reader stepped away from in view.
///
/// It carries the same state the full screen would (who, how far along, and how long) and one way
/// back, floating above the page rather than covering it.
fn bar(app: &mut App, ctx: &egui::Context, call: &CallUpdate, peer: &str, palette: &Palette) {
    let locale = app.locale;
    let live = call.phase == CallPhase::Active;
    egui::Area::new(egui::Id::new(MINIBAR))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -18.0))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(if palette.dark {
                    Color32::from_rgb(28, 32, 36)
                } else {
                    palette.panel
                })
                .stroke(egui::Stroke::new(1.0, palette.outline))
                .corner_radius(CornerRadius::same(22))
                .inner_margin(egui::Margin::symmetric(16, 9))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 4],
                    blur: 18,
                    spread: 0,
                    color: palette.shadow,
                })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (dot, _) = ui.allocate_exact_size(Vec2::splat(9.0), Sense::hover());
                        ui.painter().circle_filled(
                            dot.center(),
                            4.5,
                            if live { palette.danger } else { palette.accent },
                        );
                        ui.add_space(2.0);
                        theme::text(ui, peer, theme::semibold(13.5), palette.text);
                        theme::text(
                            ui,
                            status(app, call),
                            theme::medium(13.0),
                            palette.secondary,
                        );
                        ui.add_space(8.0);
                        if ui
                            .button(gettext(locale, "Return to the call"))
                            .on_hover_text(gettext(locale, "Return to the call"))
                            .clicked()
                        {
                            app.actions.push(Action::ReturnToCall);
                        }
                    });
                });
        });
}

/// The line under the peer's name, derived from the backend's phase and nothing else.
fn status(app: &App, call: &CallUpdate) -> String {
    let locale = app.locale;
    match call.phase {
        CallPhase::Dialing => gettext(locale, "Calling…").into_owned(),
        // The peer's phone is ringing: its `<preaccept>` arrived and nobody has answered yet.
        CallPhase::Ringing => gettext(locale, "Ringing…").into_owned(),
        // The peer answered and the media is still coming up. The duration has not started, which
        // is the whole point: this is the line that used to stay on screen as "Ringing…".
        CallPhase::Connecting => gettext(locale, "Connected").into_owned(),
        // This side picked the phone up and the media is still being negotiated.
        CallPhase::Accepted => gettext(locale, "Connecting…").into_owned(),
        // A live call shows its length, which starts when the media plane came up rather than when
        // the button was pressed.
        CallPhase::Active => call
            .started
            .map(elapsed)
            .unwrap_or_else(|| gettext(locale, "Connected").into_owned()),
        // A finished call says what became of it, from the peer's own signaling and the media
        // plane rather than from the button that was pressed.
        CallPhase::Ended | CallPhase::Failed => call
            .outcome
            .map(|outcome| outcome_text(locale, outcome, call.phase))
            .unwrap_or_else(|| gettext(locale, "Call ended").into_owned()),
        CallPhase::Incoming => gettext(locale, "Incoming call").into_owned(),
    }
}

/// How a call that is over reads on screen.
///
/// A lost relay is told from a call that never came up by the phase, since the outcome is the same
/// cause in both cases.
fn outcome_text(locale: Locale, outcome: CallOutcome, phase: CallPhase) -> String {
    match outcome {
        CallOutcome::Answered => gettext(locale, "Call ended").into_owned(),
        CallOutcome::Missed => gettext(locale, "Missed call").into_owned(),
        CallOutcome::Declined => gettext(locale, "Declined").into_owned(),
        CallOutcome::Busy => gettext(locale, "Busy").into_owned(),
        CallOutcome::Failed => gettext(locale, "Could not connect").into_owned(),
        CallOutcome::NoAnswer => gettext(locale, "No answer").into_owned(),
        CallOutcome::ConnectionLost => {
            if phase == CallPhase::Ended {
                gettext(locale, "Connection lost").into_owned()
            } else {
                gettext(locale, "The call could not be established").into_owned()
            }
        }
        CallOutcome::AnsweredElsewhere => {
            gettext(locale, "Answered on another device").into_owned()
        }
        CallOutcome::DeclinedElsewhere => {
            gettext(locale, "Declined on another device").into_owned()
        }
    }
}

fn status_color(call: &CallUpdate, palette: &Palette) -> Color32 {
    match call.phase {
        CallPhase::Failed => palette.danger,
        CallPhase::Ended => palette.secondary,
        _ => palette.accent,
    }
}

/// The lines under the status: how far along a video call's picture is, and what moved beneath it.
///
/// A device that went away is shown rather than swallowed, so a headset switching off reads as a
/// reason instead of as a call that quietly started using another microphone.
fn under_status(
    ui: &mut egui::Ui,
    app: &App,
    call: &CallUpdate,
    palette: &Palette,
    on_picture: bool,
) {
    if call.video && call.phase == CallPhase::Active && !call.remote_video {
        let tint = if on_picture {
            Color32::from_white_alpha(170)
        } else {
            palette.secondary
        };
        ui.add_space(2.0);
        theme::text(
            ui,
            gettext(app.locale, "Waiting for video…"),
            theme::medium(12.5),
            tint,
        );
    }
    if !call.lost_devices.is_empty() {
        let tint = if on_picture {
            Color32::from_rgb(255, 214, 150)
        } else {
            palette.warning
        };
        ui.add_space(4.0);
        for lost in &call.lost_devices {
            theme::text(ui, lost_device(app, lost), theme::medium(12.5), tint);
        }
    }
    // The engine's own diagnosis of a call that carries no audio from the peer: the peer is better
    // told that the call is one-sided than left wondering whether their microphone is broken.
    if let Some(audio) = call.peer_audio {
        let tint = if on_picture {
            Color32::from_rgb(255, 214, 150)
        } else {
            palette.warning
        };
        ui.add_space(4.0);
        theme::text(
            ui,
            peer_audio_text(app.locale, audio),
            theme::medium(12.5),
            tint,
        );
    }
}

/// What the engine says about the peer's audio, in the reader's language.
fn peer_audio_text(locale: Locale, audio: PeerAudio) -> String {
    match audio {
        PeerAudio::Stalled => gettext(locale, "No audio is arriving from this call").into_owned(),
        PeerAudio::Silent(SilenceReason::NoDecoder) => {
            gettext(locale, "This call's audio cannot be decoded").into_owned()
        }
        PeerAudio::Silent(SilenceReason::AuthenticationFailing) => {
            gettext(locale, "This call's audio is not authenticating").into_owned()
        }
        PeerAudio::Silent(SilenceReason::UnexpectedPayloadType) => gettext(
            locale,
            "This call's audio is arriving in an unexpected format",
        )
        .into_owned(),
        PeerAudio::Silent(SilenceReason::CodecRejectingFrames) => {
            gettext(locale, "This call's audio is being rejected by the decoder").into_owned()
        }
        PeerAudio::Silent(SilenceReason::CodecFlapping) => {
            gettext(locale, "This call's audio format keeps changing").into_owned()
        }
        PeerAudio::Silent(SilenceReason::Unknown) => {
            gettext(locale, "The peer's audio stopped").into_owned()
        }
    }
}

/// Why a device the user picked is not the one in use, with its name filled in.
///
/// Translators: `{name}` is replaced by the device's own description, which may be long.
fn lost_device(app: &App, lost: &crate::calls::LostDevice) -> String {
    let locale = app.locale;
    let template = match lost.kind {
        crate::calls::DeviceKind::Microphone => gettext(
            locale,
            "Microphone “{name}” is not available; using the system default",
        ),
        crate::calls::DeviceKind::Speaker => gettext(
            locale,
            "Speaker “{name}” is not available; using the system default",
        ),
        crate::calls::DeviceKind::Camera => gettext(locale, "Camera “{name}” is not available"),
    };
    template.replace("{name}", &lost.name)
}

/// `mm:ss`, or `h:mm:ss` past an hour.
fn elapsed(started: Instant) -> String {
    let seconds = started.elapsed().as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

/// The round buttons.
fn controls(ui: &mut egui::Ui, app: &mut App, call: &CallUpdate, palette: &Palette) {
    let locale = app.locale;
    let connected = call.phase.is_connected();
    let live = call.phase.is_live();
    let buttons = if call.video { 5.0 } else { 4.0 };
    ui.horizontal(|ui| {
        let spacing = 14.0;
        let width = buttons * CONTROL + (buttons - 1.0) * spacing;
        ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));

        // Microphone: the engine's own mute flag, which is what stops outgoing audio.
        let tip = if call.muted {
            gettext(locale, "Unmute").into_owned()
        } else {
            gettext(locale, "Mute").into_owned()
        };
        let response = control(
            ui,
            if call.muted {
                Icon::VolumeX
            } else {
                Icon::Mic
            },
            CONTROL,
            if call.muted {
                palette.danger
            } else {
                palette.surface_active
            },
            palette.text,
            &tip,
            connected,
        );
        if response.clicked() {
            app.actions.push(Action::SetCallMuted(!call.muted));
        }

        // Camera: starts video on a voice call, and stops sending our picture on a video call.
        let tip = if !call.video {
            gettext(locale, "Start video").into_owned()
        } else if call.camera_on {
            gettext(locale, "Turn the camera off").into_owned()
        } else {
            gettext(locale, "Turn the camera on").into_owned()
        };
        let response = control(
            ui,
            Icon::Video,
            CONTROL,
            if call.camera_on {
                palette.accent
            } else {
                palette.surface_active
            },
            if call.camera_on {
                palette.on_accent
            } else {
                palette.text
            },
            &tip,
            connected,
        );
        if response.clicked() {
            app.actions.push(Action::SetCallCamera(!call.camera_on));
        }

        // Screen sharing has no 1:1 path in the protocol crate. The button is driven by the
        // capability flag rather than by a literal `false`, so the day the crate grows the path this
        // becomes a live control and nothing else here changes.
        if call.video {
            let supported = crate::calls::screen_share_supported();
            let tip = if supported {
                gettext(locale, "Share your screen").into_owned()
            } else {
                // Stated plainly rather than hinted at: the pinned whatsapp-rust revision has no
                // 1:1 screen-share path, and pointing at a virtual camera instead is an honest
                // workaround rather than a screen-share implementation.
                gettext(
                    locale,
                    "1:1 screen sharing is not available in the whatsapp-rust revision this app uses; an OBS virtual camera can be selected as a camera instead",
                )
                .into_owned()
            };
            control(
                ui,
                Icon::Monitor,
                CONTROL,
                palette.surface_active,
                palette.text,
                &tip,
                supported,
            );
        }

        // The speaker button shows and hides the device pickers below it.
        let tip = gettext(locale, "Speaker and devices").into_owned();
        let response = control(
            ui,
            Icon::Volume2,
            CONTROL,
            if app.call_devices_open {
                palette.accent
            } else {
                palette.surface_active
            },
            if app.call_devices_open {
                palette.on_accent
            } else {
                palette.text
            },
            &tip,
            live,
        );
        if response.clicked() {
            app.call_devices_open = !app.call_devices_open;
        }

        let tip = gettext(locale, "Hang up").into_owned();
        let response = control(
            ui,
            Icon::Phone,
            CONTROL,
            palette.danger,
            Color32::WHITE,
            &tip,
            true,
        );
        if response.clicked() {
            app.actions.push(Action::HangupCall);
        }
    });
}

/// One round control. A disabled control still reads as a control rather than disappearing.
fn control(
    ui: &mut egui::Ui,
    icon: Icon,
    diameter: f32,
    fill: Color32,
    tint: Color32,
    tooltip: &str,
    enabled: bool,
) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(diameter), sense);
    if enabled {
        theme::reveal_focus(&response);
        theme::focus_outline(ui, response.id, rect, diameter / 2.0);
    }
    if ui.is_rect_visible(rect) {
        let hovered = enabled && (response.hovered() || response.has_focus());
        let fill = if !enabled {
            fill.gamma_multiply(0.6)
        } else if hovered {
            fill.gamma_multiply(1.15)
        } else {
            fill
        };
        let tint = if enabled {
            tint
        } else {
            tint.gamma_multiply(0.6)
        };
        ui.painter()
            .circle_filled(rect.center(), diameter / 2.0, fill);
        theme::paint_icon(ui, icon, rect, diameter * 0.42, tint);
    }
    let response = if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    };
    // A painted control is invisible to a screen reader unless it says what it is; the shared icon
    // buttons register the same way, and a disabled one is reported as disabled rather than missing.
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, tooltip));
    response.on_hover_text(tooltip)
}

/// The device pickers. Each one rebinds a live stream, so what it shows is what is in use.
fn devices(ui: &mut egui::Ui, app: &mut App, call: &CallUpdate) {
    let locale = app.locale;
    let default_label = gettext(locale, "Default device").into_owned();
    let microphones: Vec<(String, String)> = app
        .call_devices
        .microphones
        .iter()
        .map(|device| (device.id.clone(), device.label.clone()))
        .collect();
    let speakers: Vec<(String, String)> = app
        .call_devices
        .speakers
        .iter()
        .map(|device| (device.id.clone(), device.label.clone()))
        .collect();
    let cameras: Vec<(String, String)> = app
        .call_devices
        .cameras
        .iter()
        .map(|device| (device.id.clone(), device.label.clone()))
        .collect();
    let connected = call.phase.is_connected();

    ui.horizontal(|ui| {
        let width = if call.video { 560.0 } else { 384.0 };
        ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
        let mut actions = Vec::new();
        let choices = Picker {
            salt: "call-microphone",
            icon: Icon::Mic,
            title: &gettext(locale, "Microphone"),
            default_label: &default_label,
            current: call.microphone.as_deref(),
            devices: &microphones,
            enabled: connected,
        };
        choices.show(ui, &mut actions, Action::SetCallMicrophone);
        let choices = Picker {
            salt: "call-speaker",
            icon: Icon::Volume2,
            title: &gettext(locale, "Speaker"),
            default_label: &default_label,
            current: call.speaker.as_deref(),
            devices: &speakers,
            enabled: connected,
        };
        choices.show(ui, &mut actions, Action::SetCallSpeaker);
        if call.video {
            let choices = Picker {
                salt: "call-camera",
                icon: Icon::Video,
                title: &gettext(locale, "Camera"),
                default_label: &default_label,
                current: call.camera.as_deref(),
                devices: &cameras,
                enabled: connected,
            };
            choices.show(ui, &mut actions, Action::SetCallCameraDevice);
        }
        app.actions.extend(actions);
    });
}

/// One device picker: a title with its icon, and a list of what the machine really has.
struct Picker<'a> {
    salt: &'a str,
    icon: Icon,
    title: &'a str,
    default_label: &'a str,
    current: Option<&'a str>,
    devices: &'a [(String, String)],
    enabled: bool,
}

impl Picker<'_> {
    fn show(
        &self,
        ui: &mut egui::Ui,
        actions: &mut Vec<Action>,
        action: impl Fn(Option<String>) -> Action,
    ) {
        let label = self
            .current
            .and_then(|id| {
                self.devices
                    .iter()
                    .find(|(known, _)| known == id)
                    .map(|(_, label)| label.clone())
            })
            .or_else(|| self.current.map(str::to_owned))
            .unwrap_or_else(|| self.default_label.to_owned());
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                theme::icon(ui, self.icon, 13.0, ui.visuals().weak_text_color());
                theme::text(
                    ui,
                    self.title,
                    theme::medium(11.5),
                    ui.visuals().weak_text_color(),
                );
            });
            ui.add_enabled_ui(self.enabled, |ui| {
                egui::ComboBox::from_id_salt(self.salt)
                    .selected_text(label)
                    .width(168.0)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(self.current.is_none(), self.default_label)
                            .clicked()
                        {
                            actions.push(action(None));
                        }
                        for (id, name) in self.devices {
                            if ui
                                .selectable_label(self.current == Some(id.as_str()), name)
                                .clicked()
                            {
                                actions.push(action(Some(id.clone())));
                            }
                        }
                    });
            });
        });
        ui.add_space(12.0);
    }
}

/// A call that is ringing: who it is, and the two answers.
fn ringing(
    ui: &mut egui::Ui,
    app: &mut App,
    call: &CallUpdate,
    peer: &str,
    picture: Option<&Path>,
    palette: &Palette,
    area: Rect,
) {
    let locale = app.locale;
    ui.vertical_centered(|ui| {
        ui.add_space(area.height() * 0.17);
        theme::text(
            ui,
            gettext(locale, "Incoming call"),
            theme::medium(13.0),
            palette.accent,
        );
        ui.add_space(22.0);
        let size = (area.height() * 0.24).clamp(96.0, 160.0);
        widgets::avatar(ui, palette, peer, &call.chat, size, picture);
        ui.add_space(18.0);
        theme::text(ui, peer, theme::bold(26.0), palette.text);
        ui.add_space(6.0);
        theme::text(
            ui,
            if call.video {
                gettext(locale, "Incoming video call")
            } else {
                gettext(locale, "Incoming voice call")
            },
            theme::medium(15.0),
            palette.secondary,
        );
    });
    ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let width = 2.0 * CONTROL + 60.0;
            ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
            let accept = gettext(locale, "Accept").into_owned();
            let response = control(
                ui,
                Icon::Phone,
                CONTROL,
                palette.accent,
                palette.on_accent,
                &accept,
                true,
            );
            if response.clicked() {
                app.actions.push(Action::AnswerCall);
            }
            ui.add_space(60.0);
            let decline = gettext(locale, "Decline").into_owned();
            let response = control(
                ui,
                Icon::Phone,
                CONTROL,
                palette.danger,
                Color32::WHITE,
                &decline,
                true,
            );
            if response.clicked() {
                app.actions.push(Action::DeclineCall);
            }
        });
    });
}

/// The largest rect of `size`'s aspect ratio that fits inside `area`.
fn fit_inside(area: Rect, size: Vec2) -> Rect {
    if size.x <= 0.0 || size.y <= 0.0 {
        return area;
    }
    let scale = (area.width() / size.x).min(area.height() / size.y);
    Rect::from_center_size(area.center(), Vec2::new(size.x * scale, size.y * scale))
}
