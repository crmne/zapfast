//! The call surface: a full-window screen for a live 1:1 call, and the prompt for one that rings.
//!
//! Nothing here decides what a call is doing. The phase, the point the duration counts from, the
//! mute state and the device selections all come from [`crate::calls::CallUpdate`], which the worker
//! fills from the backend's own events. This module only draws them, and pushes [`Action`]s when the
//! user presses something.

use std::path::Path;
use std::time::{Duration, Instant};

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
/// The seconds of one ringing breath, and how far the icon and the ring around the picture move
/// within it. Deliberately small: a call should catch the eye, not shake the window.
const RING_PERIOD: f64 = 1.6;
/// The part of the period spent moving; the rest is a still pause before the next breath.
const RING_ACTIVE_SHARE: f64 = 0.6;
const RING_ICON_AMPLITUDE: f32 = 0.12;
const RING_HALO_GROWTH: f32 = 16.0;
/// An outgoing call's own, fainter breath: a call this side placed can wait more quietly.
const OUTGOING_PERIOD: f64 = 2.4;
const OUTGOING_ICON_AMPLITUDE: f32 = 0.05;

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
    // A call the reader stepped away from keeps the bar; so does a locked chat's call, including
    // the moment it ends. The farewell is the same disclosure as the ring, so a hidden call stays
    // behind the bar rather than painting the window one last time to announce its outcome. Only a
    // call that should really be seen — including a visible chat's four-second farewell — reaches
    // the full surface below.
    if app.call_surface_hidden {
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
    // it. Both textures are kept between frames and updated in place, so a per-frame repaint does
    // not allocate a new GPU texture every time.
    if call.video
        && let Some(image) = app.call_remote_frame.clone()
    {
        let at = fit_inside(area, vec2(image.width() as f32, image.height() as f32));
        let texture = call_texture(
            ui,
            &mut app.call_remote_texture,
            "zapfast-call-remote",
            &image,
        );
        ui.painter().image(
            texture,
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
        let texture = call_texture(
            ui,
            &mut app.call_local_texture,
            "zapfast-call-local",
            &image,
        );
        ui.painter().image(
            texture,
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
            // A call going out breathes too, but slower and shallower than one coming in: this
            // side already knows it is waiting, so the screen need not work as hard for it.
            if matches!(call.phase, CallPhase::Dialing | CallPhase::Ringing) {
                let breath = pulse_phase(ui, app.settings.call_animations, OUTGOING_PERIOD);
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::hover());
                    if ui.is_rect_visible(rect) {
                        let scale = breath.map_or(1.0, |share| {
                            1.0
                                + OUTGOING_ICON_AMPLITUDE
                                    * (share * std::f32::consts::PI).sin()
                        });
                        theme::paint_icon(
                            ui,
                            outgoing_icon(call.video),
                            rect,
                            15.0 * scale,
                            palette.accent,
                        );
                    }
                    ui.add_space(6.0);
                    theme::text(
                        ui,
                        status(app, call),
                        theme::medium(15.0),
                        status_color(call, palette),
                    );
                });
            } else {
                theme::text(
                    ui,
                    status(app, call),
                    theme::medium(15.0),
                    status_color(call, palette),
                );
            }
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

    // Window-level controls: stepping back to the chat, and taking the surface full screen. Neither
    // one ends the call. They come after whatever the surface fills itself with, so they are drawn
    // over a picture rather than under it.
    ui.horizontal(|ui| {
        if call.phase.is_live() {
            // Named for what it does rather than for where it goes: "Back to the chat" reads as
            // ending the call to anyone who has not guessed, and this button is the one that says
            // the call is still running behind the bar it leaves behind.
            let back = gettext(app.locale, "Hide the call screen").into_owned();
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
            ui.add_space(8.0);
        }
        let (icon, tip) = if app.call_fullscreen {
            (
                Icon::Minimize,
                gettext(app.locale, "Exit full screen").into_owned(),
            )
        } else {
            (
                Icon::Maximize,
                gettext(app.locale, "Full screen").into_owned(),
            )
        };
        let response = control(
            ui,
            icon,
            38.0,
            palette.surface_active,
            palette.text,
            &tip,
            true,
        );
        if response.clicked() {
            app.actions.push(Action::ToggleCallFullscreen);
        }
    });

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
                        // A call in the colour the surface uses for a call that is up. The whole
                        // point of the bar is that the call is still there, so it says what kind
                        // of call it is before it says who is on it.
                        theme::icon(
                            ui,
                            if !live {
                                // A farewell behind the bar is a call that is over: the slashed
                                // handset, not the receiver that is still up.
                                Icon::PhoneOff
                            } else if call.video {
                                Icon::Video
                            } else {
                                Icon::Phone
                            },
                            13.0,
                            if live { palette.danger } else { palette.accent },
                        );
                        ui.add_space(4.0);
                        theme::text(ui, peer, theme::semibold(13.5), palette.text);
                        theme::text(
                            ui,
                            status(app, call),
                            theme::medium(13.0),
                            palette.secondary,
                        );
                        ui.add_space(8.0);
                        // Only a call that is still up can be returned to: a hidden farewell is
                        // the end of the call, and an actionable button would promise a screen
                        // that is already gone.
                        if call.phase.is_live()
                            && ui
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

/// Draws `image` through the kept texture, updating it in place after the first frame.
///
/// A video call repaints once per frame, so allocating a fresh texture each time churns GPU memory
/// and re-uploads the whole picture. `TextureHandle::set` resizes and refills the one already there.
fn call_texture(
    ui: &egui::Ui,
    kept: &mut Option<egui::TextureHandle>,
    name: &str,
    image: &egui::ColorImage,
) -> egui::TextureId {
    match kept {
        Some(texture) => {
            texture.set(image.clone(), TextureOptions::LINEAR);
            texture.id()
        }
        None => {
            let texture = ui
                .ctx()
                .load_texture(name, image.clone(), TextureOptions::LINEAR);
            let id = texture.id();
            *kept = Some(texture);
            id
        }
    }
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
    // A platform whose backend cannot carry video offers no camera control at all, so the row is
    // centred on what is really drawn: five on a video call (microphone, camera, screen share,
    // speaker/devices, hang up), four on a voice call where screen share is not drawn, and three
    // where there is no video capability to press.
    let video = crate::calls::capabilities().video;
    let buttons = if !video {
        3.0
    } else if call.video {
        5.0
    } else {
        4.0
    };
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
            if call.muted { Icon::MicOff } else { Icon::Mic },
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

        // Camera: starts video on a voice call, and stops sending our picture on a video call. Only
        // where the backend can really carry video, so macOS and Windows do not offer a control
        // whose only possible outcome is a backend error; the chat header gates on the same flag.
        if video {
            let tip = if !call.video {
                gettext(locale, "Start video").into_owned()
            } else if call.camera_on {
                gettext(locale, "Turn the camera off").into_owned()
            } else {
                gettext(locale, "Turn the camera on").into_owned()
            };
            let response = control(
                ui,
                if call.video && !call.camera_on {
                    Icon::VideoOff
                } else {
                    Icon::Video
                },
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

        // Hang up belongs to a call that is still up. The farewell screen keeps the controls it
        // was drawn with, but this one is no longer actionable: the backend has already released
        // the call, so a click would only send a command nobody is listening for.
        let tip = gettext(locale, "Hang up").into_owned();
        let response = control(
            ui,
            Icon::PhoneOff,
            CONTROL,
            palette.danger,
            Color32::WHITE,
            &tip,
            live,
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
    // The ringing call is the one thing on this screen that moves, and the one that most needs a
    // reader's eye. The status icon breathes and a ring swells out of the picture: both are
    // painted, never laid out, so nothing around them shifts, and both hold still when the reader
    // has turned call motion off.
    let breath = pulse_phase(ui, app.settings.call_animations, RING_PERIOD);
    ui.vertical_centered(|ui| {
        ui.add_space(area.height() * 0.17);
        ui.horizontal(|ui| {
            // A fixed box with a growing glyph: the row is the same width at every phase.
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::hover());
            if ui.is_rect_visible(rect) {
                let scale = breath.map_or(1.0, |share| {
                    1.0 + RING_ICON_AMPLITUDE * (share * std::f32::consts::PI).sin()
                });
                theme::paint_icon(ui, incoming_icon(call.video), rect, 15.0 * scale, palette.accent);
            }
            ui.add_space(6.0);
            theme::text(
                ui,
                gettext(locale, "Incoming call"),
                theme::medium(13.0),
                palette.accent,
            );
        });
        ui.add_space(22.0);
        let size = (area.height() * 0.24).clamp(96.0, 160.0);
        let avatar = widgets::avatar(ui, palette, peer, &call.chat, size, picture);
        // Drawn after the picture but outside it, so the ring never covers a face.
        if let Some(share) = breath {
            let radius = avatar.rect.width() / 2.0 + 4.0 + RING_HALO_GROWTH * share;
            let alpha = (1.0 - share) * 0.55;
            ui.painter().circle_stroke(
                avatar.rect.center(),
                radius,
                egui::Stroke::new(2.0, palette.accent.gamma_multiply(alpha)),
            );
        }
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
            // Answering lifts the receiver to the ear; the two answers are told apart by their
            // shape as well as their colour, so green-and-red is never the only difference.
            let accept = gettext(locale, "Accept").into_owned();
            let response = control(
                ui,
                incoming_icon(call.video),
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
            // Refusing a call puts the receiver down: a slashed handset, not a red one.
            let decline = gettext(locale, "Decline").into_owned();
            let response = control(
                ui,
                Icon::PhoneOff,
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

/// The shape that stands for a call coming in, in the medium it carries.
fn incoming_icon(video: bool) -> Icon {
    if video {
        Icon::VideoIncoming
    } else {
        Icon::PhoneIncoming
    }
}

/// The shape that stands for a call going out, in the medium it carries.
fn outgoing_icon(video: bool) -> Icon {
    if video {
        Icon::VideoOutgoing
    } else {
        Icon::PhoneOutgoing
    }
}

/// The share of one breath this frame sits at, `0.0..=1.0`, or `None` when call motion is off.
///
/// The value rises across the moving part of the cycle and holds at `1.0` through the still
/// tail, so a caller's `sin(share * PI)` starts and ends at rest and nothing jumps between
/// breaths. Returning `None` (rather than `Some(1.0)`) keeps the caller from asking the window to
/// repaint for a picture that is not moving.
fn pulse_phase(ui: &egui::Ui, on: bool, period: f64) -> Option<f32> {
    if !on {
        return None;
    }
    // A third of the display's rate is plenty for a slow breath, and the request is what keeps
    // the window drawing while the call is on screen.
    ui.ctx()
        .request_repaint_after(Duration::from_millis(33));
    Some(pulse_at(ui.input(|input| input.time), period))
}

/// The share of one breath at `time`, `0.0..=1.0`: it climbs across the moving part of the cycle
/// and holds at `1.0` through the still tail. Pure, so the motion can be checked without a
/// window.
fn pulse_at(time: f64, period: f64) -> f32 {
    let share = (time % period) / period;
    (share / RING_ACTIVE_SHARE).min(1.0) as f32
}

/// The largest rect of `size`'s aspect ratio that fits inside `area`.
fn fit_inside(area: Rect, size: Vec2) -> Rect {
    if size.x <= 0.0 || size.y <= 0.0 {
        return area;
    }
    let scale = (area.width() / size.x).min(area.height() / size.y);
    Rect::from_center_size(area.center(), Vec2::new(size.x * scale, size.y * scale))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The breath must start and end at rest, or the glyph would jump between cycles, and it must
    /// have a still tail so the call reads as calm rather than as a flashing light.
    #[test]
    fn the_ringing_breath_rises_then_rests_before_the_next_one() {
        assert_eq!(pulse_at(0.0, RING_PERIOD), 0.0, "the breath starts at rest");
        assert_eq!(
            pulse_at(RING_PERIOD, RING_PERIOD),
            0.0,
            "and the next cycle starts where the last one did"
        );
        assert!(
            (pulse_at(RING_PERIOD * 0.3, RING_PERIOD) - 0.5).abs() < 1e-6,
            "the moving part is half over at its middle"
        );
        assert_eq!(
            pulse_at(RING_PERIOD * (RING_ACTIVE_SHARE + 0.01), RING_PERIOD),
            1.0,
            "the tail holds still at the top of the share"
        );
        assert_eq!(pulse_at(RING_PERIOD * 0.99, RING_PERIOD), 1.0);
    }

    /// With call motion off there is no phase at all, so a caller draws one still frame and, the
    /// early return being before the repaint request, never asks the window to keep drawing.
    #[test]
    fn motion_off_has_no_phase_and_draws_a_still_frame() {
        let ctx = egui::Context::default();
        let mut seen = Vec::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            seen.push(pulse_phase(ui, true, RING_PERIOD));
            seen.push(pulse_phase(ui, false, RING_PERIOD));
        });
        // Headless tests must apply font-atlas updates themselves.
        output.textures_delta.clear();
        assert!(seen[0].is_some(), "motion on has a phase to draw");
        assert!(seen[1].is_none(), "motion off has none, so nothing moves");
    }

    /// A slower, shallower breath for an outgoing call, and the same resting endpoints.
    #[test]
    fn an_outgoing_call_breathes_slower_and_shallower_than_a_ringing_one() {
        let scale = |period: f64, amplitude: f32| {
            1.0 + amplitude * (pulse_at(period * 0.3, period) * std::f32::consts::PI).sin()
        };
        let ringing = scale(RING_PERIOD, RING_ICON_AMPLITUDE);
        let outgoing = scale(OUTGOING_PERIOD, OUTGOING_ICON_AMPLITUDE);
        assert!(ringing > outgoing, "the ringing cue is the stronger one");
        assert!(outgoing > 1.0, "and the outgoing cue still moves");
        // A second in, the quicker breath has already settled while the slower one is still
        // moving: the outgoing call never works as hard.
        assert_eq!(pulse_at(1.0, RING_PERIOD), 1.0, "the ring has paused");
        assert!(pulse_at(1.0, OUTGOING_PERIOD) < 1.0, "the dial is still waiting");
    }
}
