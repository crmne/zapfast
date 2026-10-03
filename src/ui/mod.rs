//! Window layout: panels, overlays, keyboard shortcuts.

pub mod chats;
pub mod conversation;
pub mod dialogs;
pub(crate) mod focus;
pub mod image_preview;
pub mod keys;
pub mod labels;
pub mod lock;
pub mod login;
pub mod message_info;
pub mod pane;
pub mod picker;
pub mod polls;
pub mod settings;
pub mod update;
pub mod video_preview;
pub mod widgets;

use egui::{Align2, CornerRadius, Frame, Margin, Stroke, vec2};

use crate::app::App;
use crate::backend::LinkStatus;
use crate::model::{Action, Page, SidebarDisplayMode, ToastKind};
use crate::theme::{self, Icon};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let ctx = &ctx;
    track_keyboard_focus(ctx);
    // Locked, nothing else is drawn: no chat list, no messages, no dialogs,
    // no toasts, and no shortcut reaches them.
    if app.app_lock.is_locked() {
        titlebar_strip(app, ui);
        lock::show(app, ui);
        focus_ring(app, ctx);
        return;
    }
    keys::handle(app, ctx);
    let main_navigation = app.is_linked()
        && app.page == Page::Chats
        && app.dialog.is_none()
        && !app.show_update
        && app.picker.is_none()
        && app.reaction_target.is_none()
        && app.recording.is_none()
        && app.image_preview.is_none()
        && !app.video_expanded
        && app.emoji_start.is_none()
        && app.mention_start.is_none()
        // The day filter keeps egui's own order among its days.
        && !app.chat_search_calendar
        && !egui::Popup::is_any_open(ctx);
    focus::begin(ctx, main_navigation);
    // The open chat's composer records its rect again below, if there is one.
    ctx.data_mut(|data| data.remove::<egui::Rect>(composer_rect_id()));
    titlebar_strip(app, ui);
    if !app.is_linked() {
        login::show(app, ui);
        dialogs::show(app, ctx);
        update::show(app, ctx);
        toasts(app, ctx);
        focus_ring(app, ctx);
        return;
    }
    let macos = theme::macos_chrome(ctx);
    if !macos {
        banner(app, ui);
    }
    let narrow = narrow(ctx);
    let list_only = narrow && app.page == Page::Chats && app.current_chat().is_none();
    let view = narrow.then_some(if list_only {
        NarrowView::List
    } else {
        NarrowView::Detail
    });
    let slide = narrow_slide(ctx, view);
    match slide {
        Some(progress) => slide_in(app, ui, list_only, progress),
        None => main_view(app, ui, list_only),
    }
    slide_shield(ctx, narrow, slide.is_some());
    focus::finish(ctx, main_navigation);
    update::show(app, ctx);
    picker::show(app, ctx);
    dialogs::show(app, ctx);
    image_preview::show(app, ctx);
    video_preview::show(app, ctx);
    drop_target(app, ctx);
    toasts(app, ctx);
    focus_ring(app, ctx);
}

/// The chat list, or the open chat or page with the list beside it unless
/// the window is narrow.
fn main_view(app: &mut App, ui: &mut egui::Ui, list_only: bool) {
    if list_only {
        chats::full_show(app, ui);
        return;
    }
    // A narrow window gives the open chat or page all of its width; the
    // conversation header's Back returns to the list.
    if !narrow(ui.ctx()) {
        match app.sidebar_mode() {
            SidebarDisplayMode::Expanded => chats::show(app, ui),
            SidebarDisplayMode::CollapsedIconsOnly => chats::compact_show(app, ui),
        }
    }
    let search_overlay = pane::show(app, ui);
    egui::CentralPanel::default()
        .frame(central_frame(app))
        .show(ui, |ui| match app.page {
            Page::Settings => settings::show(app, ui),
            Page::Chats => conversation::show(app, ui),
            Page::Wallpaper => settings::wallpaper_show(app, ui),
        });
    if let Some(region) = search_overlay {
        pane::show_overlay(app, ui.ctx(), region);
    }
}

/// What a narrow window shows: the chat list, or what was opened from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NarrowView {
    List,
    Detail,
}

/// How long a narrow window takes to slide one view in over the other.
const SLIDE_SECONDS: f64 = 0.22;

/// Follows the narrow window's view from frame to frame and returns how far
/// the slide to it has come (0 to 1, eased) while one is under way. A view
/// shown on widening or narrowing the window appears without one.
fn narrow_slide(ctx: &egui::Context, view: Option<NarrowView>) -> Option<f32> {
    let view_id = egui::Id::new("narrow-view");
    let start_id = egui::Id::new("narrow-slide-start");
    let now = ctx.input(|input| input.time);
    let before = ctx.data_mut(|data| {
        let before = data.get_temp::<NarrowView>(view_id);
        match view {
            Some(view) => {
                data.insert_temp(view_id, view);
            }
            None => data.remove::<NarrowView>(view_id),
        }
        before
    });
    if view.is_none() {
        ctx.data_mut(|data| data.remove::<f64>(start_id));
        return None;
    }
    if before.is_some() && before != view {
        ctx.data_mut(|data| data.insert_temp(start_id, now));
    }
    let start = ctx.data(|data| data.get_temp::<f64>(start_id))?;
    let t = ((now - start) / SLIDE_SECONDS) as f32;
    if t >= 1.0 {
        ctx.data_mut(|data| data.remove::<f64>(start_id));
        return None;
    }
    ctx.request_repaint();
    // Ease out: quick to start, settling into place.
    Some(1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3))
}

/// A narrow window's view mid-slide, as a stack: an opened chat or page
/// comes in from the right over the list, which stays beneath it, dimmed;
/// going back brings the list in from the left.
fn slide_in(app: &mut App, ui: &mut egui::Ui, list_only: bool, progress: f32) {
    // Both views are drawn in scopes of their own, so the list in either is
    // a copy of the real one, which it follows.
    ui.ctx()
        .data_mut(|data| data.insert_temp(chats::slide_copy_id(), true));
    let rect = ui.available_rect_before_wrap();
    if list_only {
        // The view it covers is gone (the chat is closed): its backdrop.
        ui.painter().rect_filled(rect, 0.0, central_background(app));
    } else {
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.disable();
            chats::full_show(app, ui);
        });
    }
    let direction = if list_only { -1.0 } else { 1.0 };
    let offset = (1.0 - progress) * rect.width() * direction;
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.with_visual_transform(
            egui::emath::TSTransform::from_translation(vec2(offset, 0.0)),
            |ui| main_view(app, ui, list_only),
        );
    });
    ui.ctx()
        .data_mut(|data| data.remove::<bool>(chats::slide_copy_id()));
}

/// A slide moves what is drawn but not where presses land, so while one
/// runs an empty area over the window takes the pointer: nothing is clicked,
/// hovered or scrolled where it is not drawn. A narrow window keeps the area
/// between slides, taking nothing, because egui lays a new area out unseen
/// and without input in its first frame.
fn slide_shield(ctx: &egui::Context, narrow: bool, sliding: bool) {
    if !narrow {
        return;
    }
    let rect = ctx.content_rect();
    egui::Area::new(egui::Id::new("narrow-slide-shield"))
        .order(egui::Order::Middle)
        .interactable(sliding)
        .constrain(false)
        .fixed_pos(rect.min)
        .show(ctx, |ui| {
            let sense = if sliding {
                egui::Sense::click_and_drag()
            } else {
                egui::Sense::hover()
            };
            ui.allocate_rect(rect, sense);
        });
}

/// Below this window width the chat list and the open chat take turns at the
/// whole window: the narrowest list beside a readable conversation.
pub(crate) const NARROW_WIDTH: f32 = 620.0;

/// Whether the window is too narrow to show the chat list beside a chat.
pub(crate) fn narrow(ctx: &egui::Context) -> bool {
    ctx.content_rect().width() < NARROW_WIDTH
}

fn central_background(app: &App) -> egui::Color32 {
    if app.page == Page::Chats {
        app.settings.wallpaper_background(&app.palette)
    } else {
        app.palette.panel
    }
}

fn central_frame(app: &App) -> Frame {
    let stroke = if app.page == Page::Wallpaper {
        let color = if app.palette.dark {
            egui::Color32::from_rgba_unmultiplied(233, 237, 239, 32)
        } else {
            egui::Color32::from_rgba_unmultiplied(10, 10, 10, 32)
        };
        Stroke::new(1.0, color)
    } else {
        Stroke::NONE
    };

    Frame::new().fill(central_background(app)).stroke(stroke)
}

/// Where the focus ring was drawn this frame, used by interaction tests.
pub fn focus_ring_id() -> egui::Id {
    egui::Id::new("focus-ring")
}

/// Tab shows where focus is; a pointer press hides it again. Widgets scroll
/// themselves into view through `theme::reveal_focus`.
fn track_keyboard_focus(ctx: &egui::Context) {
    let (tab, pressed) = ctx.input(|input| {
        let tab = input.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key: egui::Key::Tab,
                    pressed: true,
                    ..
                }
            )
        });
        (tab, input.pointer.any_pressed())
    });
    let mut keyboard = ctx.data(|data| {
        data.get_temp::<bool>(theme::keyboard_focus_id())
            .unwrap_or(false)
    });
    if tab {
        keyboard = true;
    } else if pressed {
        keyboard = false;
    }
    ctx.data_mut(|data| data.insert_temp(theme::keyboard_focus_id(), keyboard));
    ctx.data_mut(|data| data.remove::<egui::Rect>(focus_ring_id()));
}

/// Outlines the focused widget after keyboard navigation. Custom widgets
/// paint themselves; a shared fallback covers them while shaped controls and
/// frameless editors register their visible bounds.
fn focus_ring(app: &App, ctx: &egui::Context) {
    let keyboard = ctx.data(|data| {
        data.get_temp::<bool>(theme::keyboard_focus_id())
            .unwrap_or(false)
    });
    let Some(response) = ctx
        .memory(|memory| memory.focused())
        .and_then(|id| ctx.read_response(id))
    else {
        return;
    };
    // Keep text fields visibly active even when reached by clicking or
    // a shortcut. A caret alone is easy to lose in a large conversation.
    let custom = ctx
        .data(|data| data.get_temp::<theme::FocusOutline>(response.id.with("focus-outline")))
        .filter(|outline| outline.frame == ctx.cumulative_frame_nr());
    if !keyboard && !ctx.text_edit_focused() {
        return;
    }
    let rect = custom.map_or(response.rect, |outline| outline.rect);
    if !rect.is_positive() {
        return;
    }
    let ring = rect;
    let radius = custom.map_or(f32::from(theme::RADIUS_SMALL), |outline| outline.radius);
    let clip = custom.map_or(response.interact_rect, |outline| outline.clip);
    // An inset accent border would disappear on a filled primary button.
    let color = if custom.is_some_and(|outline| outline.fill == app.palette.accent) {
        app.palette.on_accent
    } else {
        app.palette.accent
    };
    // Standard egui editors already paint the theme's one-point focus stroke.
    // Frameless editors register their enclosing field above. Never double up.
    if custom.is_some() || !ctx.text_edit_focused() {
        // Paint in the control's own layer. A global Tooltip layer would put
        // focus above dialogs, menus, and even toast notifications.
        ctx.layer_painter(response.layer_id)
            .with_clip_rect(clip)
            .rect_stroke(
                ring,
                radius,
                Stroke::new(theme::FOCUS_STROKE_WIDTH, color),
                egui::StrokeKind::Inside,
            );
    }
    ctx.data_mut(|data| data.insert_temp(focus_ring_id(), ring));
    ctx.data_mut(|data| data.insert_temp(focus_ring_id().with("layer"), response.layer_id));
}

/// Shows where dragged files will be sent.
fn drop_target(app: &mut App, ctx: &egui::Context) {
    if !app.dropping {
        return;
    }
    let palette = app.palette;
    let name = app
        .current_chat()
        .map(|chat| app.chat_title(chat))
        .unwrap_or_default();
    egui::Area::new(egui::Id::new("drop-target"))
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            Frame::new()
                .fill(palette.overlay)
                .stroke(Stroke::new(2.0, palette.accent))
                .corner_radius(CornerRadius::same(theme::RADIUS + 4))
                .inner_margin(Margin::symmetric(28, 20))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        theme::icon(ui, Icon::Paperclip, 28.0, palette.accent);
                        theme::text(
                            ui,
                            format!("Drop to send to {name}"),
                            theme::semibold(15.0),
                            palette.text,
                        );
                    });
                });
        });
}

/// Connection and history-sync banner.
fn banner(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let update = app.update.clone();
    let (icon, text, color, retry, download) = match &app.link {
        LinkStatus::Connected if app.syncing => (
            Icon::Refresh,
            match app.sync_percent {
                Some(percent) => format!("Loading chat history… {percent}%"),
                None => "Loading chat history…".to_owned(),
            },
            palette.accent,
            false,
            None,
        ),
        LinkStatus::Connected if update.is_some() => {
            let update = update.as_ref().expect("checked above");
            (
                Icon::Info,
                format!("ZapFast {} is available", update.version),
                palette.accent,
                false,
                Some(update.url.clone()),
            )
        }
        LinkStatus::Connected => return,
        LinkStatus::Starting | LinkStatus::Connecting => (
            Icon::Refresh,
            "Connecting to WhatsApp…".to_owned(),
            palette.secondary,
            false,
            None,
        ),
        LinkStatus::Disconnected { reason } => (
            Icon::WifiOff,
            format!("Offline ({reason}). Reconnecting…"),
            palette.warning,
            true,
            None,
        ),
        LinkStatus::Failed(message) => (
            Icon::CircleAlert,
            message.clone(),
            palette.danger,
            true,
            None,
        ),
        LinkStatus::Unlinked { .. } | LinkStatus::LoggedOut => (
            Icon::Smartphone,
            "Not linked to a phone".to_owned(),
            palette.warning,
            false,
            None,
        ),
    };
    egui::Panel::top("banner")
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(palette.panel)
                .inner_margin(Margin::symmetric(14, 6)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                // Progress/connection events wake the window. A long history
                // sync must not redraw every message just to spin this icon.
                theme::icon(ui, icon, 15.0, color);
                theme::text(ui, text, theme::medium(13.0), palette.text);
                if retry || download.is_some() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if download.is_some() {
                            if theme::soft_button(
                                ui,
                                &palette,
                                Some(Icon::ExternalLink),
                                "Update",
                                false,
                            )
                            .clicked()
                            {
                                app.actions.push(Action::ShowUpdate);
                            }
                        } else if theme::soft_button(
                            ui,
                            &palette,
                            Some(Icon::Refresh),
                            "Retry",
                            false,
                        )
                        .clicked()
                        {
                            app.actions.push(Action::Reconnect);
                        }
                    });
                }
            });
        });
}

/// Where the open chat's composer was drawn this frame.
pub fn composer_rect_id() -> egui::Id {
    egui::Id::new("composer-rect")
}

/// Stable id of a toast's close button, used by interaction tests.
pub fn toast_close_id(index: usize) -> egui::Id {
    egui::Id::new(("toast-close", index))
}

fn toasts(app: &mut App, ctx: &egui::Context) {
    if app.toasts.is_empty() {
        return;
    }
    let palette = app.palette;
    let lifetime = crate::app::INFO_TOAST_LIFETIME.as_secs_f32();
    // Errors carry buttons, so the area must take clicks while one is shown.
    let has_error = app
        .toasts
        .iter()
        .any(|toast| toast.kind == ToastKind::Error);
    let mut actions = Vec::new();
    // Stay clear of the composer: an error waiting to be dismissed must not
    // cover the send button.
    let bottom = ctx
        .data(|data| data.get_temp::<egui::Rect>(composer_rect_id()))
        .map_or(20.0, |composer| {
            ctx.content_rect().bottom() - composer.top() + 12.0
        });
    egui::Area::new(egui::Id::new("toasts"))
        .anchor(Align2::RIGHT_BOTTOM, vec2(-20.0, -bottom))
        .order(egui::Order::Tooltip)
        .interactable(has_error)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            for (index, toast) in app.toasts.iter().enumerate() {
                let error = toast.kind == ToastKind::Error;
                // Errors appear at once: fading in needs frames that an idle
                // window does not draw.
                let alpha = if error {
                    1.0
                } else {
                    let age = toast.created.elapsed().as_secs_f32();
                    if age < 0.15 {
                        age / 0.15
                    } else if age > lifetime - 0.4 {
                        ((lifetime - age) / 0.4).clamp(0.0, 1.0)
                    } else {
                        1.0
                    }
                };
                ui.set_opacity(alpha);
                Frame::new()
                    .fill(palette.overlay)
                    .stroke(Stroke::new(1.0, palette.outline))
                    .corner_radius(CornerRadius::same(theme::RADIUS))
                    .inner_margin(Margin::symmetric(14, 10))
                    .shadow(palette.float_shadow())
                    .show(ui, |ui| {
                        // Size to the message up to a readable maximum.
                        let font = theme::medium(13.5);
                        let laid = ui.painter().layout(
                            toast.message.clone(),
                            font.clone(),
                            palette.text,
                            (ctx.content_rect().width() - 150.0).clamp(80.0, 360.0),
                        );
                        let buttons = if error { 2.0 * 26.0 + 12.0 } else { 0.0 };
                        let text = widgets::line(
                            ui,
                            &toast.message,
                            font,
                            palette.text,
                            laid.size().x + 1.0,
                            usize::MAX,
                        );
                        ui.set_width(laid.size().x + 26.0 + buttons);
                        ui.horizontal(|ui| {
                            ui.set_min_height(text.size().y.max(26.0));
                            let (icon, color) = if error {
                                (Icon::CircleAlert, palette.danger)
                            } else {
                                (Icon::CircleCheck, palette.accent)
                            };
                            theme::icon(ui, icon, 16.0, color);
                            // Keep the text clear of the buttons beside it.
                            let (rect, _) =
                                ui.allocate_exact_size(text.size(), egui::Sense::hover());
                            text.paint(ui, rect.min, palette.text);
                            ui.ctx().data_mut(|data| {
                                data.insert_temp(toast_close_id(index).with("text"), rect);
                            });
                            if error {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let close = theme::icon_button(
                                            ui,
                                            Icon::X,
                                            14.0,
                                            palette.secondary,
                                            palette.text,
                                            "Dismiss",
                                        );
                                        // Store the rect for interaction tests.
                                        ui.ctx().data_mut(|data| {
                                            data.insert_temp(toast_close_id(index), close.rect)
                                        });
                                        if close.clicked() {
                                            actions.push(Action::DismissToast(index));
                                        }
                                        if theme::icon_button(
                                            ui,
                                            Icon::Copy,
                                            14.0,
                                            palette.secondary,
                                            palette.text,
                                            "Copy this message",
                                        )
                                        .clicked()
                                        {
                                            actions.push(Action::CopyText(toast.message.clone()));
                                        }
                                    },
                                );
                            }
                        });
                    });
            }
        });
    app.actions.extend(actions);
}

/// Draggable space for the macOS traffic-light title bar.
fn titlebar_strip(app: &App, ui: &mut egui::Ui) {
    let linked = app.is_linked() && !app.app_lock.is_locked();
    if linked {
        return;
    }
    let inset = theme::titlebar_inset(ui.ctx());
    if inset == 0.0 {
        return;
    }
    let fill = if linked {
        app.palette.panel
    } else {
        app.palette.window
    };
    egui::Panel::top("titlebar")
        .exact_size(inset)
        .show_separator_line(false)
        .frame(Frame::new().fill(fill))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            titlebar_drag(ui, rect);
        });
}

/// Makes `rect` drag the window.
pub fn titlebar_drag(ui: &mut egui::Ui, rect: egui::Rect) {
    let response = ui.interact(
        rect,
        ui.id().with("titlebar-drag"),
        egui::Sense::click_and_drag(),
    );
    // AppKit requires StartDrag during the original mouse-down event.
    if response.is_pointer_button_down_on() && ui.input(|input| input.pointer.primary_pressed()) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
}

#[cfg(test)]
mod idle_tests {
    use super::*;

    #[test]
    fn settings_uses_panel_background_not_wallpaper_color() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.page = Page::Settings;
        app.palette = crate::theme::Palette::light();

        assert_eq!(central_background(&app), app.palette.panel);
        assert_eq!(central_frame(&app).stroke, Stroke::NONE);

        app.palette = crate::theme::Palette::dark();
        assert_eq!(central_background(&app), app.palette.panel);
        assert_eq!(central_frame(&app).stroke, Stroke::NONE);

        app.page = Page::Wallpaper;
        assert_eq!(
            central_frame(&app).stroke,
            Stroke::new(
                1.0,
                egui::Color32::from_rgba_unmultiplied(233, 237, 239, 32)
            )
        );
    }

    #[test]
    fn history_sync_banner_does_not_animate_the_idle_window() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.link = LinkStatus::Connected;
        app.syncing = true;
        app.sync_percent = Some(42);
        app.open_chat = None;
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut delay = std::time::Duration::ZERO;
        for index in 0..6 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(index as f64),
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    ..Default::default()
                },
                |ui| banner(&mut app, ui),
            );
            delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
            output.textures_delta.clear();
        }
        assert!(
            delay > std::time::Duration::from_millis(100),
            "sync banner requested {delay:?}: {:?}",
            ctx.repaint_causes()
        );
    }

    #[test]
    fn a_waiting_error_does_not_keep_the_window_repainting() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.link = LinkStatus::Connected;
        app.toast_error("Could not record: no microphone");
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut delay = std::time::Duration::ZERO;
        for index in 0..6 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(index as f64),
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    ..Default::default()
                },
                |ui| app.frame_ui(ui),
            );
            delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
            output.textures_delta.clear();
        }
        assert_eq!(app.toasts.len(), 1, "the error is still shown");
        // Fading info toasts ask for a frame every 120 ms; a waiting error
        // must not.
        assert!(
            delay > std::time::Duration::from_secs(1),
            "an error toast requested {delay:?}: {:?}",
            ctx.repaint_causes()
        );
    }

    #[test]
    fn conversation_at_bottom_does_not_request_continuous_repaints() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.link = LinkStatus::Connected;
        let chat = crate::model::Chat::new("123@s.whatsapp.net".into(), "Alice".into());
        app.chats.push(chat.clone());
        app.open_chat = Some(chat.id.clone());
        app.scroll_to_bottom = true;
        let mut conversation = crate::app::Conversation::default();
        conversation.messages.push(crate::model::Message {
            id: "m1".into(),
            chat: chat.id.clone(),
            sender: "123@s.whatsapp.net".into(),
            sender_name: Some("Alice".into()),
            from_me: false,
            timestamp: 1000,
            content: crate::model::Content::text("Hello world"),
            status: crate::model::Delivery::None,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: Vec::new(),
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        });
        conversation.complete = true;
        app.conversations.insert(chat.id.clone(), conversation);

        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut delay = std::time::Duration::ZERO;
        for index in 0..6 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(index as f64),
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    ..Default::default()
                },
                |ui| conversation::show(&mut app, ui),
            );
            delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
            output.textures_delta.clear();
        }
        assert!(
            delay > std::time::Duration::from_millis(100),
            "idle conversation requested {delay:?}: {:?}",
            ctx.repaint_causes()
        );
    }

    #[test]
    fn reopening_a_scrolled_up_chat_scrolls_to_bottom() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        let chat = crate::model::Chat::new("123@s.whatsapp.net".into(), "Alice".into());
        app.chats.push(chat.clone());
        let mut conversation = crate::app::Conversation::default();
        for i in 0..20 {
            conversation.messages.push(crate::model::Message {
                id: format!("m{i}"),
                chat: chat.id.clone(),
                sender: "123@s.whatsapp.net".into(),
                sender_name: Some("Alice".into()),
                from_me: false,
                timestamp: 1000 + i,
                content: crate::model::Content::text(format!("Message {i}")),
                status: crate::model::Delivery::None,
                delivered_at: None,
                read_at: None,
                quoted: None,
                reactions: Vec::new(),
                edited: false,
                mentions: Vec::new(),
                forwarded: false,
                thumbnail: None,
            });
        }
        conversation.complete = true;
        app.conversations.insert(chat.id.clone(), conversation);

        let ctx = egui::Context::default();
        theme::install(&ctx);

        // Open chat initially
        app.open_chat = Some(chat.id.clone());
        app.scroll_to_bottom = true;

        for _ in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    ..Default::default()
                },
                |ui| conversation::show(&mut app, ui),
            );
            output.textures_delta.clear();
        }
        assert!(app.at_bottom, "at bottom after opening");

        // Simulate user scrolling up
        app.scroll_to_bottom = false;
        app.at_bottom = false;

        // Reopening chat requests scroll to bottom
        app.scroll_to_bottom = true;

        for _ in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    ..Default::default()
                },
                |ui| conversation::show(&mut app, ui),
            );
            output.textures_delta.clear();
        }
        assert!(app.at_bottom, "at bottom after reopening");
    }

    #[test]
    fn a_narrow_window_shows_the_list_or_the_chat_with_a_way_back() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.link = LinkStatus::Connected;
        let chat = crate::model::Chat::new("123@s.whatsapp.net".into(), "Alice".into());
        app.chats.push(chat.clone());
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let frame = std::cell::Cell::new(0.0);
        // Lets a slide between the list and the chat finish.
        let settle = || frame.set(frame.get() + 1.0);
        // Lays out one frame at `width` and returns where the chat list's and
        // the conversation's header rows landed, if they were drawn.
        let run = |app: &mut App, width: f32, events: Vec<egui::Event>| {
            ctx.data_mut(|data| {
                data.remove::<egui::Rect>(chats::header_row_id());
                data.remove::<egui::Rect>(conversation::header_row_id());
            });
            // Frame times a click apart; egui ignores a press held too long.
            frame.set(frame.get() + 1.0 / 60.0);
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(frame.get()),
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 780.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| show(app, ui),
            );
            output.textures_delta.clear();
            ctx.data(|data| {
                (
                    data.get_temp::<egui::Rect>(chats::header_row_id()),
                    data.get_temp::<egui::Rect>(conversation::header_row_id()),
                )
            })
        };

        // Wide: the list stands beside the open chat.
        app.open_chat = Some(chat.id.clone());
        let (list, header) = run(&mut app, 1180.0, Vec::new());
        assert!(list.is_some() && header.is_some());

        // Narrow with no chat open: the list takes the whole window.
        app.open_chat = None;
        let (list, header) = run(&mut app, 480.0, Vec::new());
        let list = list.expect("the chat list is drawn");
        assert!(list.width() > 400.0, "the list fills the window: {list:?}");
        assert!(header.is_none());

        // Narrow with a chat open: only the conversation, from the left edge.
        app.open_chat = Some(chat.id.clone());
        let (list, _) = run(&mut app, 480.0, Vec::new());
        assert!(list.is_some(), "the chat slides in over the list");
        settle();
        let (list, header) = run(&mut app, 480.0, Vec::new());
        assert!(list.is_none(), "no chat list beside a narrow conversation");
        let header = header.expect("the conversation header is drawn");
        assert!(
            header.left() < 20.0,
            "the chat starts at the edge: {header:?}"
        );

        let back = ctx
            .data(|data| data.get_temp::<egui::Rect>(conversation::back_button_id()))
            .expect("Back is drawn")
            .center();
        let press = |pressed| egui::Event::PointerButton {
            pos: back,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        app.actions.clear();
        run(&mut app, 480.0, vec![egui::Event::PointerMoved(back)]);
        run(&mut app, 480.0, vec![press(true)]);
        run(&mut app, 480.0, vec![press(false)]);
        assert!(
            app.actions
                .iter()
                .any(|action| matches!(action, Action::CloseChat)),
            "Back returns to the list: {:?}",
            app.actions
        );
    }

    #[test]
    fn a_narrow_window_slides_between_the_list_and_what_it_opens() {
        let ctx = egui::Context::default();
        let at = |time: f64, view: Option<NarrowView>| {
            let mut progress = None;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |_| progress = narrow_slide(&ctx, view),
            );
            output.textures_delta.clear();
            progress
        };
        // Narrowing the window shows its view at once.
        assert_eq!(at(0.0, None), None);
        assert_eq!(at(0.1, Some(NarrowView::List)), None);
        // Opening a chat starts a slide that eases into place and ends.
        assert_eq!(at(1.0, Some(NarrowView::Detail)), Some(0.0));
        let halfway = at(1.0 + SLIDE_SECONDS / 2.0, Some(NarrowView::Detail)).unwrap();
        assert!(halfway > 0.5 && halfway < 1.0, "eased: {halfway}");
        assert_eq!(at(1.0 + SLIDE_SECONDS, Some(NarrowView::Detail)), None);
        // Back to the list slides too.
        assert_eq!(at(2.0, Some(NarrowView::List)), Some(0.0));
        // Widening mid-slide drops it, and narrowing again does not slide.
        assert_eq!(at(2.05, None), None);
        assert_eq!(at(2.1, Some(NarrowView::Detail)), None);
    }

    #[test]
    fn the_list_beneath_a_sliding_chat_ignores_the_keyboard() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.link = LinkStatus::Connected;
        let alice = crate::model::Chat::new("123@s.whatsapp.net".into(), "Alice".into());
        let alan = crate::model::Chat::new("456@s.whatsapp.net".into(), "Alan".into());
        app.chats.push(alice.clone());
        app.chats.push(alan.clone());
        app.search = "Al".into();
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let frame = std::cell::Cell::new(0.0);
        let run = |app: &mut App, events: Vec<egui::Event>| {
            frame.set(frame.get() + 1.0 / 60.0);
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(frame.get()),
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(480.0, 780.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| show(app, ui),
            );
            output.textures_delta.clear();
        };
        // The search field holds the keyboard over the narrow list.
        run(&mut app, Vec::new());
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("chat-search")));
        run(&mut app, Vec::new());
        assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new("chat-search"))));
        // A chat opens without the field giving the keyboard up (as a
        // notification click does), and slides in over the list.
        app.open_chat = Some(alan.id.clone());
        app.actions.clear();
        app.focus_search = true;
        app.scroll_chat_into_view = Some(alice.id.clone());
        let enter = |pressed| egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        run(&mut app, vec![enter(true), enter(false)]);
        assert!(
            !app.actions
                .iter()
                .any(|action| matches!(action, Action::OpenChat(_) | Action::Search(_))),
            "the dimmed list took the key: {:?}",
            app.actions
        );
        assert!(app.search_selected.is_none());
        // Requests meant for the list wait for it to come back.
        assert!(app.focus_search);
        assert_eq!(
            app.scroll_chat_into_view.as_deref(),
            Some(alice.id.as_str())
        );
    }

    #[test]
    fn a_sliding_chat_takes_no_presses_until_it_lands() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.link = LinkStatus::Connected;
        let chat = crate::model::Chat::new("123@s.whatsapp.net".into(), "Alice".into());
        app.chats.push(chat.clone());
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let time = std::cell::Cell::new(0.0);
        let run = |app: &mut App, events: Vec<egui::Event>| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(time.get()),
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(480.0, 780.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| show(app, ui),
            );
            output.textures_delta.clear();
        };
        let click = |app: &mut App, pos: egui::Pos2| {
            app.actions.clear();
            let press = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            run(app, vec![egui::Event::PointerMoved(pos)]);
            run(app, vec![press(true)]);
            run(app, vec![press(false)]);
            run(app, vec![egui::Event::PointerGone]);
            app.actions
                .iter()
                .any(|action| matches!(action, Action::CloseChat))
        };
        run(&mut app, Vec::new());
        // Open the chat and hold its slide halfway in.
        app.open_chat = Some(chat.id.clone());
        time.set(1.0);
        run(&mut app, Vec::new());
        time.set(1.0 + SLIDE_SECONDS / 2.0);
        run(&mut app, Vec::new());
        // Where Back ends up (its laid-out rect, which the slide only moves
        // on screen), and where it is drawn halfway in.
        let landed = ctx
            .data(|data| data.get_temp::<egui::Rect>(conversation::back_button_id()))
            .expect("Back is drawn")
            .center();
        let eased = 1.0 - 0.5_f32.powi(3);
        let drawn = landed + egui::vec2((1.0 - eased) * 480.0, 0.0);
        assert!(!click(&mut app, landed), "not drawn there yet");
        assert!(!click(&mut app, drawn), "still moving");
        // Once it has landed, Back takes the press.
        time.set(2.0);
        run(&mut app, Vec::new());
        assert!(
            click(&mut app, landed),
            "Back works once the chat is in place"
        );
    }

    #[test]
    fn a_scrolled_list_stays_put_through_both_slides() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.link = LinkStatus::Connected;
        for i in 0..60 {
            app.chats.push(crate::model::Chat::new(
                format!("{i}@s.whatsapp.net"),
                format!("Chat {i}"),
            ));
        }
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let time = std::cell::Cell::new(0.0);
        let run = |app: &mut App, events: Vec<egui::Event>| {
            time.set(time.get() + 1.0 / 60.0);
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(time.get()),
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(480.0, 780.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| show(app, ui),
            );
            output.textures_delta.clear();
            ctx.data(|data| data.get_temp::<f32>(chats::list_offset_id()))
                .unwrap_or_default()
        };
        // Scroll the narrow list down, and let it come to rest.
        run(&mut app, Vec::new());
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -300.0),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        };
        for _ in 0..3 {
            run(
                &mut app,
                vec![
                    egui::Event::PointerMoved(egui::pos2(200.0, 500.0)),
                    wheel.clone(),
                ],
            );
        }
        run(&mut app, vec![egui::Event::PointerGone]);
        time.set(time.get() + 1.0);
        let scrolled = run(&mut app, Vec::new());
        assert!(scrolled > 100.0, "the list scrolled: {scrolled}");
        // The list shows the same rows beneath the chat sliding in, and as
        // it slides back in once the chat closes.
        for open in [Some("5@s.whatsapp.net".to_owned()), None] {
            app.open_chat = open;
            for frame in 0..4 {
                let offset = run(&mut app, Vec::new());
                assert!(
                    (offset - scrolled).abs() < 1.0,
                    "frame {frame} of the slide: {offset} vs {scrolled}"
                );
            }
            time.set(time.get() + 1.0);
            run(&mut app, Vec::new());
        }
        assert!((run(&mut app, Vec::new()) - scrolled).abs() < 1.0);
    }
}
