//! Window layout: panels, overlays, keyboard shortcuts.

pub mod accounts;
pub mod call;
pub mod calls;
pub mod chats;
pub mod conversation;
pub mod dialogs;
pub(crate) mod focus;
mod glass;
pub mod image_editor;
pub mod image_preview;
pub mod info;
pub mod keys;
pub mod labels;
pub mod list_nav;
pub mod locations;
pub mod lock;
pub mod login;
pub mod message_info;
pub mod new_group;
pub(crate) mod overlay;
pub mod pane;
pub mod picker;
pub mod polls;
pub mod rail;
pub mod settings;
pub mod update;
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
    info::release_thumbnails(app, ctx);
    // The message list sets it again when it is drawn with focus.
    app.messages_focused = false;
    // Locked, nothing else is drawn: no chat list, no messages, no dialogs,
    // no toasts, and no shortcut reaches them.
    if app.app_lock.is_locked() {
        titlebar_strip(app, ui);
        lock::show(app, ui);
        focus_ring(app, ctx);
        announce(app, ctx);
        window_chrome(app, ui);
        return;
    }
    keys::handle(app, ctx);
    history_buttons(app, ctx);
    let main_navigation = app.is_linked()
        && app.chat_view()
        && app.dialog.is_none()
        && !app.show_update
        && app.picker.is_none()
        && app.reaction_target.is_none()
        && app.recording.is_none()
        && app.image_preview.is_none()
        && app.picture_edit.is_none()
        && app.sticker_view.is_none()
        && app.emoji_start.is_none()
        && app.mention_start.is_none()
        // Selection keeps egui's order so Tab can reach message checkboxes.
        && app.selection.is_none()
        // The day filter keeps egui's own order among its days.
        && !app.chat_search_calendar
        // The call screen covers the chats.
        && (app.call.is_none() || app.call_surface_hidden)
        // A name typed in the info panel keeps Tab between its fields.
        && !(app.info_visible() && (app.contact_edit.is_some() || app.group_name_edit.is_some()))
        && !egui::Popup::is_any_open(ctx);
    focus::begin(ctx, main_navigation);
    // The open chat's composer records its rect again below, if there is one.
    ctx.data_mut(|data| data.remove::<egui::Rect>(composer_rect_id()));
    titlebar_strip(app, ui);
    if !app.is_linked() {
        login::show(app, ui);
        accounts::corner(app, ctx);
        dialogs::show(app, ctx);
        overlay::finish(ctx);
        update::show(app, ctx);
        toasts(app, ctx);
        focus_ring(app, ctx);
        announce(app, ctx);
        window_chrome(app, ui);
        return;
    }
    let narrow = narrow(ctx);
    let list_only = narrow && list_pane_only(app);
    let view = narrow.then_some(if list_only {
        NarrowView::List
    } else {
        NarrowView::Detail
    });
    let slide = narrow_slide(ctx, view);
    // A call the reader stepped away from keeps a strip along the bottom.
    call::strip(app, ui);
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
    image_preview::show_sticker(app, ctx);
    call::show(app, ctx);
    image_editor::show(app, ctx);
    overlay::finish(ctx);
    drop_target(app, ctx);
    toasts(app, ctx);
    focus_ring(app, ctx);
    announce(app, ctx);
    window_chrome(app, ui);
}

/// Whether a narrow window shows the list pane: the chat list with no chat
/// open, or the Calls tab's history with no call picked from it. Its header's
/// buttons, its rows and the rail beside it lead on to the rest.
fn list_pane_only(app: &App) -> bool {
    match app.page {
        Page::Chats => app.current_chat().is_none(),
        Page::Calls => app.current_chat().is_none() || !app.call_picked(),
        Page::Settings | Page::Wallpaper => false,
    }
}

/// Emits the screen reader announcement nodes; private ones stay silent
/// while the app is locked.
fn announce(app: &mut App, ctx: &egui::Context) {
    app.announcer.show(ctx, !app.app_lock.is_locked());
}

/// The chat list, or the open chat or page with the list beside it unless
/// the window is narrow.
fn main_view(app: &mut App, ui: &mut egui::Ui, list_only: bool) {
    if list_only {
        chats::full_show(app, ui);
        return;
    }
    // A narrow window gives the open chat or page all of its width; the
    // conversation header's Back returns to the list, and the rail beside
    // it. The info panel takes a column of its own, and when the list leaves
    // too little room for it and the conversation, the list and the rail
    // step aside while the panel is open. The Calls tab's history needs the
    // full list pane: only the chats collapse into the rail's avatars.
    let full_list = app.sidebar_mode() == SidebarDisplayMode::Expanded || app.page == Page::Calls;
    let mut list_width = chats::compact_width(ui.ctx());
    if full_list {
        list_width += app.settings.sidebar_width;
    }
    let crowded = app.info_visible()
        && ui.available_width() - list_width < pane::INFO_MIN + pane::INFO_CONVERSATION_MIN;
    if !narrow(ui.ctx()) && !crowded {
        rail::show(app, ui, !full_list);
        if full_list {
            chats::show(app, ui);
        }
    }
    let search_overlay = pane::show(app, ui);
    let info_overlay = info::show(app, ui);
    egui::CentralPanel::default()
        .frame(central_frame(app))
        .show(ui, |ui| {
            // The top of every page drags the window, beneath its own
            // widgets: the empty pane, Settings, and the Calls tab's start.
            let top = ui.max_rect();
            caption_drag(
                ui,
                egui::Rect::from_min_size(top.min, vec2(top.width(), theme::TOP_BAR_HEIGHT)),
            );
            match app.page {
                Page::Settings => settings::show(app, ui),
                Page::Chats => conversation::show(app, ui),
                // The open chat stays beside the call history.
                Page::Calls if app.current_chat().is_some() => conversation::show(app, ui),
                Page::Calls => {
                    banner(app, ui);
                    calls::show(app, ui);
                }
                Page::Wallpaper => settings::wallpaper_show(app, ui),
            }
        });
    if let Some(region) = search_overlay {
        pane::show_overlay(app, ui.ctx(), region);
    }
    if let Some(region) = info_overlay {
        info::show_overlay(app, ui.ctx(), region);
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
    // The Calls tab shows the open chat beside its history, or its own pane.
    let conversation =
        app.page == Page::Chats || (app.page == Page::Calls && app.current_chat().is_some());
    if conversation && app.palette.vibrant() {
        // The conversation paints its own translucent background.
        egui::Color32::TRANSPARENT
    } else if conversation {
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

/// Steps through the window's history with the mouse's side buttons, as a
/// browser does: `Extra1` (Back) goes back, `Extra2` (Forward) forward.
/// Overlays are not places, and the buttons stay out of them while one is
/// open, as the keyboard's own shortcuts do: the media viewer, a dialog, the
/// update screen, a picker, a reaction, a recording, a message selection, or
/// a menu.
fn history_buttons(app: &mut App, ctx: &egui::Context) {
    if app.image_preview.is_some()
        || app.dialog.is_some()
        || app.show_update
        || app.picker.is_some()
        || app.reaction_target.is_some()
        || app.recording.is_some()
        || app.selection.is_some()
        || egui::Popup::is_any_open(ctx)
    {
        return;
    }
    let (mut back, mut forward) = (false, false);
    ctx.input_mut(|input| {
        input.events.retain(|event| match event {
            egui::Event::PointerButton {
                button: egui::PointerButton::Extra1,
                pressed,
                ..
            } => {
                back |= *pressed;
                false
            }
            egui::Event::PointerButton {
                button: egui::PointerButton::Extra2,
                pressed,
                ..
            } => {
                forward |= *pressed;
                false
            }
            _ => true,
        });
    });
    if back {
        app.actions.push(Action::NavigateBack);
    }
    if forward {
        app.actions.push(Action::NavigateForward);
    }
}

/// Where the focus ring was drawn this frame, used by interaction tests.
pub fn focus_ring_id() -> egui::Id {
    egui::Id::new("focus-ring")
}

/// Tab shows where focus is; a pointer press hides it again. Widgets scroll
/// themselves into view through `theme::reveal_focus`. In a list that rings
/// its current item (`list_nav::outline_only`), the keys that move the item
/// count as keyboard navigation too: the reader may have reached the list by
/// a shortcut or a screen reader rather than Tab.
fn track_keyboard_focus(ctx: &egui::Context) {
    // Checked before the list registers again this frame, so last frame's
    // registration counts.
    let in_list = ctx
        .memory(|memory| memory.focused())
        .is_some_and(|id| list_nav::was_outline_only(ctx, id));
    let (navigated, pressed) = ctx.input(|input| {
        let navigated = input.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key,
                    pressed: true,
                    ..
                } if *key == egui::Key::Tab
                    || (in_list
                        && matches!(
                            key,
                            egui::Key::ArrowUp
                                | egui::Key::ArrowDown
                                | egui::Key::ArrowLeft
                                | egui::Key::ArrowRight
                                | egui::Key::Home
                                | egui::Key::End
                                | egui::Key::PageUp
                                | egui::Key::PageDown
                        ))
            )
        });
        (navigated, input.pointer.any_pressed())
    });
    let mut keyboard = ctx.data(|data| {
        data.get_temp::<bool>(theme::keyboard_focus_id())
            .unwrap_or(false)
    });
    if navigated {
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
    // A list rings its current item only. With that item not drawn this
    // frame there is nothing to ring, and never the whole list.
    if custom.is_none() && list_nav::is_outline_only(ctx, response.id) {
        return;
    }
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
                        let drop_text = crate::i18n::gettext(app.locale, "Drop to send to {name}")
                            .replace("{name}", &name);
                        theme::text(ui, drop_text, theme::semibold(15.0), palette.text);
                    });
                });
        });
}

/// Connection and history-sync banner.
/// What the banner needs beside the caption buttons ZapFast draws: its line
/// and a button.
const BANNER_MIN_CONTENT: f32 = 320.0;

fn banner(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let update = app.update.clone();
    let (icon, text, color, retry, download) = match &app.link {
        LinkStatus::Connected if app.syncing => (
            Icon::Refresh,
            match app.sync_percent {
                Some(percent) => {
                    crate::i18n::gettext(app.locale, "Loading chat history… {percent}%")
                        .replace("{percent}", &percent.to_string())
                }
                None => crate::i18n::gettext(app.locale, "Loading chat history…").into_owned(),
            },
            palette.accent,
            false,
            None,
        ),
        LinkStatus::Connected if update.is_some() => {
            let update = update.as_ref().expect("checked above");
            (
                Icon::Info,
                crate::i18n::gettext(app.locale, "ZapFast {version} is available")
                    .replace("{version}", &update.version),
                palette.accent,
                false,
                Some(update.url.clone()),
            )
        }
        LinkStatus::Connected => return,
        LinkStatus::Starting | LinkStatus::Connecting => (
            Icon::Refresh,
            crate::i18n::gettext(app.locale, "Connecting to WhatsApp…").into_owned(),
            palette.secondary,
            false,
            None,
        ),
        LinkStatus::Disconnected { reason } => (
            Icon::WifiOff,
            crate::i18n::gettext(app.locale, "Offline ({reason}). Reconnecting…")
                .replace("{reason}", reason),
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
            crate::i18n::gettext(app.locale, "Not linked to a phone").into_owned(),
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
            // Over a page with no header (Settings, or no chat open) the
            // banner takes the top-right corner from the caption buttons
            // ZapFast draws, beside them or under them.
            let room = caption_room(ui.ctx(), ui.max_rect(), BANNER_MIN_CONTENT);
            caption_drag(ui, ui.max_rect().with_max_y(ui.max_rect().top() + room.top));
            ui.add_space(room.top);
            ui.horizontal(|ui| {
                ui.add_space(room.left);
                // Progress/connection events wake the window. A long history
                // sync must not redraw every message just to spin this icon.
                theme::icon(ui, icon, 15.0, color);
                theme::text(ui, text, theme::medium(13.0), palette.text);
                if retry || download.is_some() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(room.right);
                        if download.is_some() {
                            if theme::soft_button(
                                ui,
                                &palette,
                                Some(Icon::ExternalLink),
                                &crate::i18n::gettext(app.locale, "Update"),
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
                            &crate::i18n::gettext(app.locale, "Retry"),
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
                                            &crate::i18n::gettext(app.locale, "Dismiss"),
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
                                            &crate::i18n::gettext(app.locale, "Copy this message"),
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

/// Draggable space across the top of the linking and lock screens, which
/// have no header: for the macOS traffic lights, or for the caption buttons
/// ZapFast draws.
fn titlebar_strip(app: &App, ui: &mut egui::Ui) {
    let linked = app.is_linked() && !app.app_lock.is_locked();
    if linked {
        return;
    }
    let inset = if theme::macos_chrome(ui.ctx()) {
        theme::titlebar_inset(ui.ctx())
    } else {
        theme::caption_footprint(ui.ctx()).height
    };
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

/// Makes `rect` drag the window, and answers a double click on it as the
/// platform does (fastframe-titlebar).
pub fn titlebar_drag(ui: &mut egui::Ui, rect: egui::Rect) {
    fastframe_titlebar::drag(ui, rect);
}

/// Makes `rect` drag the window while ZapFast draws the title bar, for the
/// places macOS leaves to its own title bar strip: the pages, the panels at
/// the right, and the screens over the window. Call it before the widgets
/// inside `rect`, so they keep their clicks.
pub(crate) fn caption_drag(ui: &mut egui::Ui, rect: egui::Rect) {
    if theme::drawn_titlebar(ui.ctx()) && !theme::macos_chrome(ui.ctx()) {
        titlebar_drag(ui, rect);
    }
}

/// Where a header's own widgets go beside the caption buttons ZapFast draws
/// (see [`caption_room`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct CaptionRoom {
    /// Width to leave free at the header's left end.
    pub left: f32,
    /// Width to leave free at the header's right end.
    pub right: f32,
    /// How far to move the header's widgets down, under the buttons, when
    /// the header is too narrow to share the row with them.
    pub top: f32,
}

/// The room a header laid out in `rect` leaves for the caption buttons
/// ZapFast draws over the window's top corners. Only a header that reaches
/// a corner shares it: the buttons' room at that end, less what already
/// separates the header from the window's edge. A header whose own widgets
/// need `min_content` beside the buttons and do not get it goes under them
/// instead (`fastframe_titlebar::Footprint::reserve`). Nothing on macOS,
/// whose traffic lights keep their own inset, or under the desktop's title
/// bar.
pub(crate) fn caption_room(ctx: &egui::Context, rect: egui::Rect, min_content: f32) -> CaptionRoom {
    let footprint = theme::caption_footprint(ctx);
    let window = ctx.content_rect();
    let below = rect.top() - window.top();
    if footprint.height <= below {
        return CaptionRoom::default();
    }
    let left = (footprint.left - (rect.left() - window.left())).max(0.0);
    let right = (footprint.right - (window.right() - rect.right())).max(0.0);
    // Both ends taken as one, so a header narrower than its content and the
    // buttons at either end moves down under them.
    let reserve = fastframe_titlebar::Footprint {
        left: 0.0,
        right: left + right,
        height: footprint.height - below,
    }
    .reserve(rect.width(), min_content);
    if reserve.push_down > 0.0 {
        CaptionRoom {
            top: reserve.push_down,
            ..CaptionRoom::default()
        }
    } else {
        CaptionRoom {
            left,
            right,
            top: 0.0,
        }
    }
}

/// The caption buttons ZapFast draws, over the window's top corners and
/// above everything else in it, and the resizable edges, while the custom
/// title bar is on (Windows and Linux).
fn window_chrome(app: &App, ui: &mut egui::Ui) {
    if !fastframe_titlebar::enabled() {
        return;
    }
    let palette = app.palette;
    let style = fastframe_titlebar::Style {
        // GNOME's round buttons carry the text colour on the hover surface;
        // Windows' glyphs are quieter, in the secondary colour.
        icon: if cfg!(target_os = "linux") {
            palette.text
        } else {
            palette.secondary
        },
        button: palette.surface_hover,
        header_height: theme::TOP_BAR_HEIGHT,
    };
    // Literal calls, so xgettext extracts each label.
    let minimize = crate::i18n::gettext(app.locale, "Minimize");
    let maximize = crate::i18n::gettext(app.locale, "Maximize");
    let restore = crate::i18n::gettext(app.locale, "Restore");
    let close = crate::i18n::gettext(app.locale, "Close");
    let labels = fastframe_titlebar::Labels {
        minimize: &minimize,
        maximize: &maximize,
        restore: &restore,
        close: &close,
    };
    fastframe_titlebar::controls(ui, &style, &labels);
    // The buttons draw in egui's foreground layer, as the call screen, the
    // media viewer and dialogs do; a newer area there would cover them, and a
    // modal one would take their clicks. These are the crate's area names.
    let layout = fastframe_titlebar::layout();
    for (area, buttons) in [
        ("fastframe-titlebar-left", &layout.left),
        ("fastframe-titlebar-right", &layout.right),
    ] {
        if buttons.is_empty() {
            continue;
        }
        ui.ctx().move_to_top(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new(area),
        ));
    }
    fastframe_titlebar::resize(ui);
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
    fn the_mouse_side_buttons_ask_for_history_steps() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        let ctx = egui::Context::default();
        let press = |app: &mut App, button: egui::PointerButton| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::PointerButton {
                        pos: egui::pos2(20.0, 20.0),
                        button,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ui| history_buttons(app, ui.ctx()),
            );
            output.textures_delta.clear();
            std::mem::take(&mut app.actions)
        };
        assert!(
            press(&mut app, egui::PointerButton::Extra1).contains(&Action::NavigateBack),
            "the back side button steps back"
        );
        assert!(
            press(&mut app, egui::PointerButton::Extra2).contains(&Action::NavigateForward),
            "the forward side button steps forward"
        );
        // The media viewer owns the screen, as it does for the keyboard.
        app.image_preview = Some(crate::image_preview::PreviewState::new(
            None,
            "1@s.whatsapp.net".into(),
            "m1".into(),
        ));
        assert!(press(&mut app, egui::PointerButton::Extra1).is_empty());
        app.image_preview = None;
        // A dialog is not a place, and the buttons stay out of it.
        app.dialog = Some(crate::model::Dialog::NewChat);
        assert!(press(&mut app, egui::PointerButton::Extra1).is_empty());
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
            history_order: None,
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
                history_order: None,
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
                data.remove::<egui::Rect>(rail::rect_id());
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

        // Narrow with no chat open: the list takes the whole window beside
        // the rail.
        app.open_chat = None;
        let (list, header) = run(&mut app, 480.0, Vec::new());
        let list = list.expect("the chat list is drawn");
        let rail = ctx
            .data(|data| data.get_temp::<egui::Rect>(rail::rect_id()))
            .expect("the rail stands beside the narrow list");
        assert!(rail.left() < 1.0 && rail.right() <= list.left());
        assert!(
            list.width() > 400.0 - rail.width(),
            "the list fills the rest of the window: {list:?}"
        );
        assert!(header.is_none());

        // Narrow with a chat open: only the conversation, from the left edge.
        app.open_chat = Some(chat.id.clone());
        let (list, _) = run(&mut app, 480.0, Vec::new());
        assert!(list.is_some(), "the chat slides in over the list");
        settle();
        let (list, header) = run(&mut app, 480.0, Vec::new());
        assert!(list.is_none(), "no chat list beside a narrow conversation");
        assert!(
            ctx.data(|data| data.get_temp::<egui::Rect>(rail::rect_id()))
                .is_none(),
            "the rail goes with the list"
        );
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
            app.chats.push(crate::model::Chat {
                last_activity: 1,
                ..crate::model::Chat::new(format!("{i}@s.whatsapp.net"), format!("Chat {i}"))
            });
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

    #[test]
    fn a_chat_opened_for_the_first_time_shows_its_end_in_the_first_frame() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        let chat = crate::model::Chat::new("123@s.whatsapp.net".into(), "Alice".into());
        app.chats.push(chat.clone());
        let mut conversation = crate::app::Conversation::default();
        for i in 0..60 {
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
                history_order: None,
            });
        }
        conversation.complete = true;
        app.conversations.insert(chat.id.clone(), conversation);
        // Another chat was open before, so the window is laid out already.
        let other = crate::model::Chat::new("456@s.whatsapp.net".into(), "Bob".into());
        app.chats.push(other.clone());
        app.open_chat = Some(other.id.clone());

        let ctx = egui::Context::default();
        theme::install(&ctx);
        let frame = |app: &mut App| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    ..Default::default()
                },
                |ui| conversation::show(app, ui),
            );
            output.textures_delta.clear();
            output
        };
        for _ in 0..3 {
            frame(&mut app);
        }
        app.open_chat = Some(chat.id.clone());
        app.scroll_to_bottom = true;
        let output = frame(&mut app);
        // The text the reader sees: drawn inside its clip rect.
        let shown: Vec<String> = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text)
                    if clipped.clip_rect.intersects(text.visual_bounding_rect()) =>
                {
                    Some(text.galley.text().to_owned())
                }
                _ => None,
            })
            .collect();
        assert!(
            shown.iter().any(|text| text == "Message 59"),
            "the newest message is in view at once: {shown:?}"
        );
        assert!(
            !shown.iter().any(|text| text == "Message 0"),
            "the oldest never flashes: {shown:?}"
        );
    }
}
