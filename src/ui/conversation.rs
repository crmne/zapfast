//! The open chat: its header, the messages, and the composer.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::{
    Align, Align2, Color32, CornerRadius, Frame, Key, KeyboardShortcut, Layout, Margin, Modifiers,
    Rect, Sense, Stroke, Vec2, pos2, vec2,
};

use crate::animation;
use crate::app::{App, Conversation, JumpHighlight, KeyScroll, RowHeight};
use crate::geo::Spot;
use crate::markup;
use crate::model::{
    Action, Chat, ChatActivity, ChatId, Content, Delivery, Dialog, LinkPreview, Media, MediaState,
    Message, PickerTab, Scroll,
};
use crate::theme::{self, Icon, Palette};
use crate::wallpaper;

use super::focus::{self, Stop, TabStop};
use super::widgets;

/// Group-message avatar size.
const SENDER_AVATAR: f32 = 28.0;
const BODY_SIZE: f32 = widgets::MESSAGE_TEXT_SIZE;
/// Extra space above the first message of a run from one side. With the
/// list's 3 points between rows, runs stand 8 points apart and a run's
/// bubbles 3, Messages' own spacing on the Mac
/// (`CKUIBehaviorMac.balloonNonContiguousSpace` and
/// `balloonContiguousSpace`).
const RUN_GAP: f32 = 5.0;
/// How much of the transcript's width a bubble may take in the Messages
/// style (`CKUIBehaviorMac.balloonMaxWidthPercent`), and in WhatsApp's.
const MESSAGES_WIDTH_SHARE: f32 = 0.65;
const WHATSAPP_WIDTH_SHARE: f32 = 0.72;
/// The transcript's side margin in the Messages style: Messages' 11 points
/// (`CKUIBehaviorMac.minTranscriptMarginInsets`) plus the 5 points beside
/// each balloon's body that its tail curls into, at ZapFast's text size.
const MESSAGES_SIDE_MARGIN: i8 = 17;
/// The transcript's side margin in WhatsApp's style.
const WHATSAPP_SIDE_MARGIN: i8 = 18;
/// Room around a Messages bubble's contents: Messages' 12 points beside its
/// text and 7 above and below (`CKUIBehaviorMac.textBalloonPillContentInsets`)
/// at ZapFast's text size, 13.4 and 7.8, rounded.
const MESSAGES_BUBBLE_MARGIN: Margin = Margin {
    left: 13,
    right: 13,
    top: 8,
    bottom: 8,
};
/// The side of a selecting row's check box, and the column that holds it.
const SELECT_BOX: f32 = 18.0;
const SELECT_GUTTER: f32 = SELECT_BOX + 14.0;
/// Footer label on an outgoing message that failed to send.
const NOT_SENT: &str = "Not sent";
const NOT_SENT_HINT: &str =
    "This message could not be sent, and ZapFast will not retry it. Send it again yourself.";

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let Some(chat) = app.current_chat().cloned() else {
        if theme::chrome_in_headers(ui.ctx()) {
            super::banner(app, ui);
        }
        empty(app, ui);
        return;
    };
    wallpaper::paint(ui, &app.wallpaper());
    // On a vibrant window the messages scroll on under the header, as in
    // Messages: the header and any banner go on a layer above them.
    if app.palette.vibrant() {
        let layer = egui::LayerId::new(egui::Order::Middle, ui.id().with("chat-header"));
        let bar = ui
            .scope_builder(egui::UiBuilder::new().layer_id(layer), |ui| {
                // Beneath the header: the messages under it, frosted, and a
                // veil of the chat's colour that keeps the title readable.
                let glass = ui.painter().add(egui::Shape::Noop);
                let veil = ui.painter().add(egui::Shape::Noop);
                header(app, ui, &chat);
                if theme::chrome_in_headers(ui.ctx()) {
                    super::banner(app, ui);
                }
                let bar = ui.min_rect();
                // Strongest at the top, both ease off from within the bar to
                // a little below it, so where they start never shows.
                let frosted = bar.with_max_y(bar.bottom() + HEADER_TAIL);
                let ease = HEADER_TAIL + bar.height() * HEADER_EASE;
                ui.painter().set(glass, super::glass::shape(frosted, ease));
                ui.painter()
                    .set(veil, header_veil(&app.palette, frosted, ease));
                bar
            })
            .inner;
        composer(app, ui, &chat);
        let list = ui.available_rect_before_wrap().with_min_y(bar.top());
        ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
            messages(app, ui, &chat, bar.height());
        });
        // A vibrant window is flat, as Messages is: no shadows over the chat.
        return;
    }
    let header = header(app, ui, &chat);
    if theme::chrome_in_headers(ui.ctx()) {
        super::banner(app, ui);
    }
    pin_banner(app, ui, &chat);
    composer(app, ui, &chat);
    messages(app, ui, &chat, 0.0);
    // Over the messages, which scroll under the header.
    widgets::paint_shadow_below(
        ui,
        &app.palette,
        header.left(),
        header.right(),
        header.bottom(),
    );
    // The chat list, or its rail of avatars, stands at the header's level
    // beside the conversation: it casts the same shadow across it, from
    // under the header down. A narrow window has no list beside it.
    if !super::narrow(ui.ctx()) {
        widgets::paint_shadow_beside(
            ui,
            &app.palette,
            header.left(),
            header.bottom(),
            ui.max_rect().bottom(),
        );
    }
}

/// Height of the pinned-message line under the header. It is one line
/// whatever the number of pins, as on WhatsApp: it shows one pin at a time.
const PIN_ROW_HEIGHT: f32 = 44.0;
/// Gap between the banner edge and the pin count bar, and the icon's own box.
const PIN_ROW_INSET: f32 = 12.0;
const PIN_ICON: f32 = 16.0;
/// The bar at the line's left edge: one segment per pin, the shown one lit.
const PIN_BAR_WIDTH: f32 = 3.0;
const PIN_BAR_GAP: f32 = 2.0;

/// Which pin the line shows: the one it showed last, or the newest pin when
/// that one is gone. `rows` is newest first.
fn shown_pin(rows: &[crate::archive::Pinned], cursor: Option<&str>) -> usize {
    cursor
        .and_then(|id| rows.iter().position(|row| row.id == id))
        .unwrap_or(0)
}

/// The pin a click leaves on the line: the next older one, back to the
/// newest after the oldest, so repeated clicks walk every pin.
fn next_pin(rows: usize, shown: usize) -> usize {
    if rows == 0 { 0 } else { (shown + 1) % rows }
}

/// Where each segment of the pin count bar goes, top to bottom, for `count`
/// pins in a bar `height` tall. The newest pin is the bottom segment, as on
/// WhatsApp.
fn pin_bar_segments(top: f32, height: f32, count: usize) -> Vec<(f32, f32)> {
    let count = count.max(1);
    let gaps = PIN_BAR_GAP * (count - 1) as f32;
    let segment = ((height - gaps) / count as f32).max(1.0);
    (0..count)
        .map(|index| {
            let start = top + (segment + PIN_BAR_GAP) * index as f32;
            (start, start + segment)
        })
        .collect()
}

/// The chat's pinned messages under its header, one line like WhatsApp's: it
/// shows one pin, the newest first, with the message's own words cut off with
/// an ellipsis when they do not fit. Clicking it opens that message and moves
/// the line on to the next pin, so repeated clicks walk the chat's pins. A bar
/// at its left has a segment per pin and lights the one shown.
fn pin_banner(app: &mut App, ui: &mut egui::Ui, chat: &Chat) {
    let now = crate::util::now();
    if let Some(wait) = app.chat_pins.get(&chat.id).and_then(|rows| {
        rows.iter()
            .filter(|row| row.expires_at > now)
            .map(|row| row.expires_at)
            .min()
    }) {
        let left = wait.saturating_sub(now).clamp(0, 86_400) as u64;
        ui.ctx()
            .request_repaint_after(Duration::from_secs(left.saturating_add(1)));
    }
    let rows = live_pins(
        app.chat_pins.get(&chat.id).map_or(&[][..], Vec::as_slice),
        now,
    );
    if rows.is_empty() {
        return;
    }
    // Which pin is shown is the view's own state, kept per chat.
    let cursor_id = pin_cursor_id(&chat.id);
    let cursor = ui.ctx().data(|data| data.get_temp::<String>(cursor_id));
    let shown = shown_pin(&rows, cursor.as_deref());
    let row = &rows[shown];
    let palette = app.palette;
    let (bar, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), PIN_ROW_HEIGHT), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    ui.painter().rect_filled(
        bar,
        0.0,
        if response.hovered() {
            palette.surface_hover
        } else {
            palette.panel
        },
    );
    // Newest pin at the bottom, so the bar fills upward as pins are added.
    let segments = pin_bar_segments(bar.top() + 8.0, bar.height() - 16.0, rows.len());
    let left = bar.left() + PIN_ROW_INSET;
    for (index, (top, bottom)) in segments.iter().enumerate() {
        let pin = rows.len() - 1 - index;
        let colour = if pin == shown {
            palette.accent
        } else {
            palette.dim
        };
        ui.painter().rect_filled(
            Rect::from_x_y_ranges(left..=left + PIN_BAR_WIDTH, *top..=*bottom),
            PIN_BAR_WIDTH / 2.0,
            colour,
        );
    }
    let icon_left = left + PIN_BAR_WIDTH + 10.0;
    let icon = Rect::from_center_size(
        pos2(icon_left + PIN_ICON / 2.0, bar.center().y),
        Vec2::splat(PIN_ICON),
    );
    theme::paint_icon(ui, Icon::Pin, icon, 14.0, palette.accent);
    let text_left = icon_left + PIN_ICON + 8.0;
    let text_width = (bar.right() - PIN_ROW_INSET - text_left).max(1.0);
    let label = widgets::line(
        ui,
        &crate::markup::plain(&app.resolve_mention_tokens(&row.text), &[]),
        theme::regular(13.0),
        palette.text,
        text_width,
        1,
    );
    label.paint(
        ui,
        pos2(text_left, bar.center().y - label.size().y / 2.0),
        palette.text,
    );
    theme::reveal_focus(&response);
    let described = crate::i18n::gettext(app.locale, "Pinned message {index} of {count}: {text}")
        .replace("{index}", &(shown + 1).to_string())
        .replace("{count}", &rows.len().to_string())
        .replace("{text}", &row.text);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &described)
    });
    if response.clicked() {
        app.actions.push(Action::OpenMessage {
            chat: chat.id.clone(),
            message: row.id.clone(),
        });
        let next = rows[next_pin(rows.len(), shown)].id.clone();
        ui.ctx().data_mut(|data| data.insert_temp(cursor_id, next));
    }
}

/// Where the pin line keeps the id of the pin it shows, per chat.
fn pin_cursor_id(chat: &str) -> egui::Id {
    egui::Id::new(("pin-cursor", chat))
}

fn empty(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let rect = ui.max_rect();
    let center = rect.center() - vec2(0.0, 30.0);
    theme::logo(
        ui,
        center - vec2(0.0, 60.0),
        72.0,
        palette.surface,
        palette.dim,
    );
    ui.painter().text(
        center,
        Align2::CENTER_CENTER,
        "ZapFast",
        theme::bold(24.0),
        palette.text,
    );
    ui.painter().text(
        center + vec2(0.0, 30.0),
        Align2::CENTER_CENTER,
        if app.chats.is_empty() {
            crate::i18n::gettext(app.locale, "Your chats appear on the left as they load.")
        } else {
            crate::i18n::gettext(app.locale, "Select a chat on the left.")
        },
        theme::regular(14.0),
        palette.secondary,
    );
    if app.settings.show_shortcut_hints {
        ui.painter().text(
            center + vec2(0.0, 56.0),
            Align2::CENTER_CENTER,
            super::keys::label(
                crate::i18n::gettext(app.locale, "Ctrl+K to search · ? for keyboard shortcuts")
                    .as_ref(),
            ),
            theme::regular(12.5),
            palette.dim,
        );
    }
}

/// The share of a vibrant header, from its bottom, over which its frost and
/// veil begin to ease off.
const HEADER_EASE: f32 = 0.35;
/// How far below a vibrant header its frost and veil carry on, easing off.
const HEADER_TAIL: f32 = 24.0;
/// Rows of the veil's ease, enough for its curve to look smooth.
const VEIL_STEPS: usize = 8;

/// The veil over a vibrant header's frost: the palette's at the top, then
/// easing to nothing over the bottom `ease` points of `rect` along the same
/// curve as the frost, so the messages under it never stop at a line.
fn header_veil(palette: &Palette, rect: Rect, ease: f32) -> egui::Shape {
    let top = palette.vibrant_header(palette.chat);
    let start = rect.bottom() - ease;
    let mut mesh = egui::Mesh::default();
    let mut row = |y: f32, color: Color32| {
        mesh.colored_vertex(pos2(rect.left(), y), color);
        mesh.colored_vertex(pos2(rect.right(), y), color);
    };
    row(rect.top(), top);
    for step in 0..=VEIL_STEPS {
        let t = step as f32 / VEIL_STEPS as f32;
        // smoothstep, as the frost's shader eases.
        let strength = 1.0 - t * t * (3.0 - 2.0 * t);
        row(start + ease * t, top.gamma_multiply(strength));
    }
    let rows = mesh.vertices.len() as u32 / 2;
    for row in 0..rows - 1 {
        let at = row * 2;
        mesh.add_triangle(at, at + 1, at + 3);
        mesh.add_triangle(at, at + 3, at + 2);
    }
    egui::Shape::mesh(mesh)
}

/// How round the glass under a vibrant header's name is: straight-sided,
/// leaving room around the text, rather than a capsule.
const NAME_GLASS_RADIUS: f32 = 12.0;
/// The room around the name and status on their glass: wide at the sides,
/// and little above, so the glass keeps clear of the window's top edge.
const NAME_GLASS_PADDING: Vec2 = vec2(14.0, 3.5);
/// The room above and below a name alone on its glass.
const NAME_GLASS_SINGLE: f32 = 7.0;
/// How tall the name and status stand together on glass: less than the
/// avatar, so the two lines sit a little closer around its middle.
const NAME_GLASS_LINES: f32 = 32.0;
/// How wide the round glass under each of a vibrant header's buttons is.
const BUTTON_GLASS: f32 = 36.0;
/// The space between two of a vibrant header's buttons, beyond their glass.
const BUTTON_GAP: f32 = 10.0;
/// What the chat header needs beside the caption buttons ZapFast draws: its
/// most buttons (Back, two calls, Search and More), the avatar, and a name.
const HEADER_MIN_CONTENT: f32 = 320.0;

/// The round glass under one of a vibrant header's buttons.
fn glass_button(palette: &Palette, button: Rect) -> egui::Shape {
    glass(
        palette,
        Rect::from_center_size(button.center(), Vec2::splat(BUTTON_GLASS)),
        BUTTON_GLASS / 2.0,
    )
}

/// Glass under a vibrant header's name or one of its buttons: the veil
/// thickened where they sit, with a faint edge, as Messages floats them.
fn glass(palette: &Palette, rect: Rect, radius: f32) -> egui::Shape {
    let radius = CornerRadius::from(radius);
    egui::Shape::Vec(vec![
        egui::Shape::rect_filled(rect, radius, palette.vibrant_pill(palette.chat)),
        egui::Shape::rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, palette.text.gamma_multiply(0.08)),
            egui::StrokeKind::Inside,
        ),
    ])
}

fn header(app: &mut App, ui: &mut egui::Ui, chat: &Chat) -> Rect {
    let palette = app.palette;
    let title = app.chat_title(chat);
    egui::Panel::top("chat-header")
        .show_separator_line(false)
        .frame(
            Frame::new()
                // On a vibrant window the messages scroll on under it,
                // frosted, with the veil drawn beneath the header instead.
                .fill(if palette.vibrant() {
                    Color32::TRANSPARENT
                } else {
                    palette.panel
                })
                .inner_margin(Margin::symmetric(14, 8)),
        )
        .show(ui, |ui| {
            // The chat list, expanded or collapsed, clears the traffic lights.
            if theme::chrome_in_headers(ui.ctx()) {
                super::titlebar_drag(ui, ui.max_rect());
            }
            // The caption buttons ZapFast draws over a corner the header
            // reaches: beside its own buttons, or above them when narrow.
            let room = super::caption_room(ui.ctx(), ui.max_rect(), HEADER_MIN_CONTENT);
            ui.add_space(room.top);
            let narrow = super::narrow(ui.ctx());
            let row = ui.horizontal(|ui| {
                // Give both rows a fixed height so their contents align.
                ui.set_min_height(HEADER_ROW);
                // With no chat list beside it, the header clears the traffic
                // lights itself.
                if narrow && theme::macos_chrome(ui.ctx()) {
                    ui.add_space(theme::traffic_light_inset(ui.ctx()));
                } else if app.sidebar_mode() == crate::model::SidebarDisplayMode::CollapsedIconsOnly
                    && app.page == crate::model::Page::Chats
                {
                    // Beside the rail alone, the lights reach over this header.
                    ui.add_space(super::rail::lights_overhang(ui.ctx()));
                }
                ui.add_space(room.left);
                let picture = app.avatar(&chat.id);
                let (subtitle, color) = subtitle(app, chat);
                let call_here = app.call_live_here(&chat.id);
                // More and Search, and Back in a narrow window, and the call
                // buttons in a person's or a group's chat, each on its own
                // glass with room around it on a vibrant window.
                let call_buttons = if callable(chat, app.me.as_deref()) {
                    if call_here { 1.0 } else { 2.0 }
                } else {
                    0.0
                };
                let buttons = if narrow { 3.0 } else { 2.0 } + call_buttons;
                let right_controls = if palette.vibrant() {
                    buttons * 36.0 + (buttons - 0.5) * BUTTON_GAP + (BUTTON_GLASS - 30.0)
                } else {
                    buttons * 36.0
                } + room.right;
                // On a vibrant window the name and the buttons sit on glass
                // pills, drawn beneath them once they are measured.
                let name_pill = ui.painter().add(egui::Shape::Noop);
                let mut text = Rect::NOTHING;
                let mut avatar = Rect::NOTHING;
                // Treat the avatar, name, and subtitle as one info button.
                let block = ui
                    .scope(|ui| {
                        // Fix the child height before centering its contents.
                        ui.allocate_ui_with_layout(
                            vec2(
                                (ui.available_width() - right_controls).max(80.0),
                                HEADER_ROW,
                            ),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                let avatar_response = widgets::avatar(
                                    ui,
                                    &palette,
                                    &title,
                                    &chat.id,
                                    40.0,
                                    picture.as_deref(),
                                );
                                avatar = avatar_response.rect;
                                if chat.disappearing_timer().is_some() {
                                    widgets::paint_disappearing_badge(
                                        ui,
                                        &palette,
                                        avatar_response.rect,
                                    );
                                }
                                // Room for the name's glass pill to clear the avatar.
                                ui.add_space(if palette.vibrant() { 14.0 } else { 4.0 });
                                ui.vertical(|ui| {
                                    let width = (ui.available_width() - right_controls).max(80.0);
                                    ui.set_max_width(width);
                                    // On glass the lines centre on the avatar: the
                                    // column starts at the row's top otherwise.
                                    if palette.vibrant() {
                                        let column = if subtitle.is_empty() {
                                            40.0
                                        } else {
                                            NAME_GLASS_LINES
                                        };
                                        let top = avatar.center().y - column / 2.0;
                                        ui.add_space((top - ui.cursor().top()).max(0.0));
                                    }
                                    if subtitle.is_empty() {
                                        // Center the name on the avatar.
                                        ui.allocate_ui_with_layout(
                                            vec2(width, 40.0),
                                            Layout::left_to_right(Align::Center),
                                            |ui| {
                                                text = widgets::rich_text(
                                                    ui,
                                                    &title,
                                                    // Lighter on glass, as Apple's.
                                                    if palette.vibrant() {
                                                        theme::medium(15.0)
                                                    } else {
                                                        theme::semibold(17.0)
                                                    },
                                                    palette.text,
                                                )
                                                .rect;
                                            },
                                        );
                                    } else {
                                        // Align the name and subtitle with the avatar edges,
                                        // or on glass a little closer, around its middle.
                                        ui.allocate_ui_with_layout(
                                            vec2(
                                                width,
                                                if palette.vibrant() {
                                                    NAME_GLASS_LINES
                                                } else {
                                                    40.0
                                                },
                                            ),
                                            Layout::top_down(Align::Min),
                                            |ui| {
                                                text = widgets::rich_text(
                                                    ui,
                                                    &title,
                                                    theme::semibold(if palette.vibrant() {
                                                        14.0
                                                    } else {
                                                        15.0
                                                    }),
                                                    palette.text,
                                                )
                                                .rect;
                                                ui.with_layout(
                                                    Layout::bottom_up(Align::Min),
                                                    |ui| {
                                                        text = text.union(
                                                            widgets::rich_text(
                                                                ui,
                                                                &subtitle,
                                                                theme::regular(
                                                                    if palette.vibrant() {
                                                                        11.5
                                                                    } else {
                                                                        12.5
                                                                    },
                                                                ),
                                                                color,
                                                            )
                                                            .rect,
                                                        );
                                                    },
                                                );
                                            },
                                        );
                                    }
                                });
                            },
                        );
                    })
                    .response;
                if palette.vibrant() && text.is_positive() {
                    // A lone name gets Apple's roomier padding.
                    let padding = vec2(
                        NAME_GLASS_PADDING.x,
                        if subtitle.is_empty() {
                            NAME_GLASS_SINGLE
                        } else {
                            NAME_GLASS_PADDING.y
                        },
                    );
                    // Centred on the avatar, whatever room the lines leave
                    // above and below their glyphs.
                    let glass_rect = Rect::from_center_size(
                        pos2(text.center().x, avatar.center().y),
                        text.size() + 2.0 * padding,
                    );
                    ui.painter()
                        .set(name_pill, glass(&palette, glass_rect, NAME_GLASS_RADIUS));
                }
                let block = ui
                    .interact(block.rect, ui.id().with("chat-header-info"), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if block.clicked() {
                    app.actions.push(Action::OpenInfo(chat.id.clone()));
                }
                // The item and the width that has to hold it are measured from
                // the same localized label: a translation wider than the
                // English one would otherwise be clipped.
                let leave_label = if chat.is_channel() {
                    crate::i18n::gettext(app.locale, "Leave channel")
                } else {
                    crate::i18n::gettext(app.locale, "Leave group")
                };
                let select_messages_label = crate::i18n::gettext(app.locale, "Select messages");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(room.right);
                    // Each button on a vibrant window floats on its own glass,
                    // with room around it.
                    if palette.vibrant() {
                        ui.add_space(BUTTON_GAP / 2.0);
                    }
                    let more_glass = ui.painter().add(egui::Shape::Noop);
                    let more = theme::icon_button(
                        ui,
                        Icon::Ellipsis,
                        18.0,
                        palette.secondary,
                        palette.text,
                        &crate::i18n::gettext(app.locale, "More"),
                    );
                    if palette.vibrant() {
                        ui.painter()
                            .set(more_glass, glass_button(&palette, more.rect));
                    }
                    let info_label = crate::i18n::gettext(app.locale, "Info");
                    let pin_label = if chat.pinned {
                        crate::i18n::gettext(app.locale, "Unpin")
                    } else {
                        crate::i18n::gettext(app.locale, "Pin to top")
                    };
                    let archive_label = if chat.archived {
                        crate::i18n::gettext(app.locale, "Unarchive")
                    } else {
                        crate::i18n::gettext(app.locale, "Archive")
                    };
                    let clear_label = crate::i18n::gettext(app.locale, "Clear chat");
                    let copy_number_label = crate::i18n::gettext(app.locale, "Copy number");
                    let close_label = crate::i18n::gettext(app.locale, "Close chat");
                    let width = widgets::menu_width(
                        ui,
                        &[
                            info_label.as_ref(),
                            select_messages_label.as_ref(),
                            pin_label.as_ref(),
                            archive_label.as_ref(),
                            clear_label.as_ref(),
                            leave_label.as_ref(),
                            copy_number_label.as_ref(),
                            close_label.as_ref(),
                        ],
                        true,
                    );
                    // Demo/test: hold this menu open for a screenshot.
                    #[cfg(any(test, feature = "demo"))]
                    if app.open_header_menu.as_deref() == Some(chat.id.as_str()) {
                        egui::Popup::open_id(ui.ctx(), more.id.with("popup"));
                    }
                    egui::Popup::menu(&more)
                        .width(width)
                        .frame(widgets::menu_frame(&palette))
                        .show(|ui| {
                            if widgets::menu_item(
                                ui,
                                &palette,
                                Some(Icon::Info),
                                info_label.as_ref(),
                            ) {
                                app.actions.push(Action::OpenInfo(chat.id.clone()));
                            }
                            if widgets::menu_item(
                                ui,
                                &palette,
                                Some(Icon::Check),
                                select_messages_label.as_ref(),
                            ) {
                                app.actions.push(Action::StartSelection);
                            }
                            if widgets::menu_item(
                                ui,
                                &palette,
                                Some(if chat.pinned { Icon::PinOff } else { Icon::Pin }),
                                pin_label.as_ref(),
                            ) {
                                app.actions
                                    .push(Action::SetPinned(chat.id.clone(), !chat.pinned));
                            }
                            if widgets::menu_item(
                                ui,
                                &palette,
                                Some(Icon::Archive),
                                archive_label.as_ref(),
                            ) {
                                app.actions
                                    .push(Action::SetArchived(chat.id.clone(), !chat.archived));
                            }
                            // Clearing reaches the phone, so it waits for a
                            // connection, as deleting does in the chat list.
                            if widgets::menu_item_enabled(
                                ui,
                                &palette,
                                Some(Icon::Eraser),
                                clear_label.as_ref(),
                                app.is_connected(),
                            ) {
                                app.actions
                                    .push(Action::ShowDialog(Dialog::ConfirmClearChat(
                                        chat.id.clone(),
                                    )));
                            }
                            widgets::menu_separator(ui, &palette);
                            if chat.can_leave(&app.our_ids())
                                && widgets::menu_item(
                                    ui,
                                    &palette,
                                    Some(Icon::LogOut),
                                    leave_label.as_ref(),
                                )
                            {
                                app.actions
                                    .push(Action::ShowDialog(Dialog::ConfirmLeaveGroup(
                                        chat.id.clone(),
                                    )));
                            }
                            if let Some(phone) = chat.phone()
                                && widgets::menu_item(
                                    ui,
                                    &palette,
                                    Some(Icon::Copy),
                                    copy_number_label.as_ref(),
                                )
                            {
                                app.actions.push(Action::CopyText(format!("+{phone}")));
                            }
                            if widgets::menu_item(ui, &palette, Some(Icon::X), close_label.as_ref())
                            {
                                app.actions.push(Action::CloseChat);
                            }
                        });
                    // A person's chat calls them and a group's starts a call
                    // in the group; a channel, a broadcast list and the chat
                    // with ourselves have no call button. While this chat has
                    // the call, the button ends it.
                    if call_buttons > 0.0 {
                        let (call_tooltip, call_icon, call_fill, call) = if call_here {
                            (
                                crate::i18n::gettext(app.locale, "Hang up").into_owned(),
                                Icon::PhoneOff,
                                palette.danger,
                                Action::HangupCall,
                            )
                        } else {
                            (
                                crate::i18n::gettext(app.locale, "Voice call").into_owned(),
                                Icon::Phone,
                                palette.secondary,
                                Action::StartCall(chat.id.clone()),
                            )
                        };
                        // On a vibrant window each sits on its own glass, as
                        // Search and More do.
                        if palette.vibrant() {
                            ui.add_space(BUTTON_GAP);
                        }
                        let call_glass = ui.painter().add(egui::Shape::Noop);
                        let call_button = theme::icon_button(
                            ui,
                            call_icon,
                            18.0,
                            call_fill,
                            palette.text,
                            &call_tooltip,
                        );
                        if palette.vibrant() {
                            ui.painter()
                                .set(call_glass, glass_button(&palette, call_button.rect));
                        }
                        if call_button.clicked() {
                            app.actions.push(call);
                        }
                        if !call_here {
                            let tip = crate::i18n::gettext(app.locale, "Video call");
                            if palette.vibrant() {
                                ui.add_space(BUTTON_GAP);
                            }
                            let video_glass = ui.painter().add(egui::Shape::Noop);
                            let video = theme::icon_button(
                                ui,
                                Icon::Video,
                                18.0,
                                palette.secondary,
                                palette.text,
                                &tip,
                            );
                            if palette.vibrant() {
                                ui.painter()
                                    .set(video_glass, glass_button(&palette, video.rect));
                            }
                            if video.clicked() {
                                app.actions.push(Action::StartVideoCall(chat.id.clone()));
                            }
                        }
                    }
                    let searching = app.chat_search_open;
                    let tip = format!(
                        "{} ({})",
                        crate::i18n::gettext(app.locale, "Search messages"),
                        super::keys::label("Ctrl+F")
                    );
                    if palette.vibrant() {
                        ui.add_space(BUTTON_GAP);
                    }
                    let search_glass = ui.painter().add(egui::Shape::Noop);
                    let search = theme::icon_button(
                        ui,
                        Icon::Search,
                        18.0,
                        if searching {
                            palette.accent
                        } else {
                            palette.secondary
                        },
                        palette.text,
                        &tip,
                    )
                    .tab_stop(Stop::ChatSearch);
                    if palette.vibrant() {
                        ui.painter()
                            .set(search_glass, glass_button(&palette, search.rect));
                    }
                    if search.clicked() {
                        app.actions.push(if searching {
                            Action::CloseChatSearch
                        } else {
                            Action::OpenChatSearch
                        });
                    }
                    // A narrow window shows the list or the chat, not both;
                    // this is the way back to the list.
                    if narrow {
                        if palette.vibrant() {
                            ui.add_space(BUTTON_GAP);
                        }
                        let back_glass = ui.painter().add(egui::Shape::Noop);
                        // On the Calls tab the list underneath is the call
                        // history.
                        let back_label = if app.page == crate::model::Page::Calls {
                            crate::i18n::gettext(app.locale, "Back to the calls").into_owned()
                        } else {
                            "Back to chats".to_owned()
                        };
                        let back = theme::icon_button(
                            ui,
                            Icon::ArrowLeft,
                            18.0,
                            palette.secondary,
                            palette.text,
                            &back_label,
                        )
                        .tab_stop(Stop::Back);
                        if palette.vibrant() {
                            ui.painter()
                                .set(back_glass, glass_button(&palette, back.rect));
                        }
                        ui.ctx()
                            .data_mut(|data| data.insert_temp(back_button_id(), back.rect));
                        if back.clicked() {
                            app.actions.push(Action::CloseChat);
                        }
                    }
                });
            });
            ui.ctx()
                .data_mut(|data| data.insert_temp(header_row_id(), row.response.rect));
        })
        .response
        .rect
}

/// Where a narrow window's Back was laid out, for interaction tests.
pub(crate) fn back_button_id() -> egui::Id {
    egui::Id::new("conversation-back")
}

/// Where the conversation header's row was laid out, for layout tests.
pub(crate) fn header_row_id() -> egui::Id {
    egui::Id::new("conversation-header-row")
}

/// Chat-header subtitle.
fn subtitle(app: &App, chat: &Chat) -> (String, Color32) {
    let palette = app.palette;
    let typing = app.typing_in(&chat.id);
    if !typing.is_empty() {
        let activity = typing[0].2;
        let text = if chat.is_group() {
            let describe = |activity| {
                let names: Vec<&str> = typing
                    .iter()
                    .filter(|(_, _, other)| *other == activity)
                    .map(|(_, name, _)| name.as_str())
                    .collect();
                match (activity, names.as_slice()) {
                    (_, []) => None,
                    (ChatActivity::RecordingAudio, [one]) => Some(
                        crate::i18n::gettext(app.locale, "{name} is recording audio…")
                            .replace("{name}", one),
                    ),
                    (ChatActivity::RecordingAudio, [rest @ .., last]) => Some(
                        crate::i18n::gettext(app.locale, "{names} and {last} are recording audio…")
                            .replace("{names}", &rest.join(", "))
                            .replace("{last}", last),
                    ),
                    (_, [one]) => Some(format!("{one} is typing…")),
                    (_, [rest @ .., last]) => {
                        Some(format!("{} and {last} are typing…", rest.join(", ")))
                    }
                }
            };
            [ChatActivity::Typing, ChatActivity::RecordingAudio]
                .into_iter()
                .filter_map(describe)
                .collect::<Vec<_>>()
                .join(" · ")
        } else if activity == ChatActivity::RecordingAudio {
            crate::i18n::gettext(app.locale, "recording audio…").into_owned()
        } else {
            "typing…".to_owned()
        };
        return (text, palette.accent);
    }
    if chat.is_group() {
        let names = app.participant_names(chat);
        return (
            if names.is_empty() {
                "Group".to_owned()
            } else {
                names
            },
            palette.secondary,
        );
    }
    if let Some(presence) = app.presence.get(&chat.id) {
        if presence.online {
            return ("online".to_owned(), palette.accent);
        }
        if let Some(seen) = presence.last_seen {
            return (crate::util::last_seen(app.locale, seen), palette.secondary);
        }
    }
    match chat.phone() {
        Some(phone) if !app.is_saved_contact(&chat.id) => {
            (crate::util::phone(phone), palette.secondary)
        }
        _ => (String::new(), palette.secondary),
    }
}

/// Byte position of a freshly typed standalone trigger immediately before
/// the text cursor. Colons inside times and URLs, and `@` inside addresses,
/// remain ordinary text.
fn standalone_trigger(text: &str, cursor: usize, trigger: char) -> Option<usize> {
    let cursor = text
        .char_indices()
        .nth(cursor)
        .map_or(text.len(), |(at, _)| at);
    let (at, found) = text[..cursor].char_indices().next_back()?;
    if found != trigger {
        return None;
    }
    (at == 0
        || text[..at]
            .chars()
            .next_back()
            .is_some_and(|character| !character.is_alphanumeric()))
    .then_some(at)
}

/// Active mention query from its `@` through the current text cursor.
fn active_mention(text: &str, start: Option<usize>, cursor: usize) -> Option<(usize, &str)> {
    let start = start?;
    let end = text
        .char_indices()
        .nth(cursor)
        .map_or(text.len(), |(at, _)| at);
    let query = text.get(start.checked_add(1)?..end)?;
    (!query.contains(['@', '\n'])).then_some((end, query))
}

/// Active emoji query from its `:` through the current text cursor. Spaces
/// and punctuation end autocomplete without changing what the user typed.
fn active_emoji(text: &str, start: Option<usize>, cursor: usize) -> Option<(usize, &str)> {
    let start = start?;
    let after = start.checked_add(1)?;
    if text.get(start..after) != Some(":") {
        return None;
    }
    let end = text
        .char_indices()
        .nth(cursor)
        .map_or(text.len(), |(at, _)| at);
    let query = text.get(after..end)?;
    query
        .chars()
        .all(|character| character.is_alphanumeric() || matches!(character, '_' | '-' | '+'))
        .then_some((end, query))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EmojiSuggestion {
    emoji: &'static str,
    shortcode: String,
    name: &'static str,
}

fn emoji_match_score(emoji: &emojis::Emoji, query: &str) -> Option<u8> {
    let name = emoji.name();
    let shortcodes = emoji.shortcodes();
    if shortcodes.clone().any(|code| code == query) {
        Some(0)
    } else if shortcodes.clone().any(|code| code.starts_with(query)) {
        Some(1)
    } else if name == query {
        Some(2)
    } else if name.starts_with(query)
        || name
            .split([' ', '-', '_'])
            .any(|word| word.starts_with(query))
    {
        Some(3)
    } else if shortcodes.clone().any(|code| code.contains(query)) {
        Some(4)
    } else if name.contains(query) {
        Some(5)
    } else {
        None
    }
}

fn emoji_suggestion(emoji: &'static emojis::Emoji) -> EmojiSuggestion {
    let shortcode = emoji
        .shortcode()
        .map_or_else(|| emoji.name().replace([' ', '-'], "_"), str::to_owned);
    EmojiSuggestion {
        emoji: emoji.as_str(),
        shortcode: format!(":{shortcode}:"),
        name: emoji.name(),
    }
}

/// Completions for `:query`. `emojis::iter` yields each emoji once, and a
/// skin-tone-capable one such as 👍 reports `Some(SkinTone::Default)`.
fn emoji_candidates(app: &App, query: &str) -> Vec<EmojiSuggestion> {
    const LIMIT: usize = 6;
    let query = query.to_lowercase();
    let mut seen = HashSet::new();
    if query.is_empty() {
        let recent = app
            .settings
            .recent_emoji
            .iter()
            .filter_map(|emoji| emojis::get(emoji))
            .chain(emojis::iter())
            .filter(|emoji| seen.insert(emoji.as_str()))
            .take(LIMIT)
            .map(emoji_suggestion)
            .collect();
        return recent;
    }

    let mut found: Vec<_> = emojis::iter()
        .enumerate()
        .filter_map(|(order, emoji)| {
            emoji_match_score(emoji, &query).map(|score| (score, order, emoji))
        })
        .collect();
    found.sort_by_key(|(score, order, _)| (*score, *order));
    found
        .into_iter()
        .filter(|(_, _, emoji)| seen.insert(emoji.as_str()))
        .take(LIMIT)
        .map(|(_, _, emoji)| emoji_suggestion(emoji))
        .collect()
}

fn take_plain_key(ui: &mut egui::Ui, key: Key) -> bool {
    ui.input_mut(|input| {
        let mut taken = false;
        input.events.retain(|event| {
            if taken {
                return true;
            }
            let matches = matches!(
                event,
                egui::Event::Key {
                    key: found,
                    pressed: true,
                    modifiers,
                    ..
                } if *found == key && *modifiers == Modifiers::NONE
            );
            taken |= matches;
            !matches
        });
        taken
    })
}

/// Slack-style emoji suggestions above the composer. The composer keeps
/// focus, so ordinary typing continues refining the query.
fn emoji_suggestions(app: &mut App, ui: &mut egui::Ui, field: egui::Id) {
    let cursor = egui::TextEdit::load_state(ui.ctx(), field)
        .and_then(|state| state.cursor.char_range())
        .map(|range| range.primary.index.0)
        .unwrap_or_else(|| app.composer.chars().count());
    let Some((end, query)) = active_emoji(&app.composer, app.emoji_start, cursor) else {
        app.emoji_start = None;
        return;
    };
    let start = app.emoji_start.expect("checked above");
    let candidates = emoji_candidates(app, query);
    if candidates.is_empty() {
        return;
    }

    let down = take_plain_key(ui, Key::ArrowDown);
    let up = take_plain_key(ui, Key::ArrowUp);
    if down {
        app.emoji_selected = (app.emoji_selected + 1) % candidates.len();
    }
    if up {
        app.emoji_selected = (app.emoji_selected + candidates.len() - 1) % candidates.len();
    }
    app.emoji_selected = app.emoji_selected.min(candidates.len() - 1);
    let submit = take_plain_key(ui, Key::Enter) || take_plain_key(ui, Key::Tab);
    let mut picked = submit.then(|| candidates[app.emoji_selected].clone());
    let palette = app.palette;

    let list = widgets::raised(ui, &palette, suggestion_frame(&palette), |ui| {
        let row_height = 36.0;
        ui.spacing_mut().item_spacing.y = 0.0;
        for (index, candidate) in candidates.iter().enumerate() {
            let (rect, response) =
                ui.allocate_exact_size(vec2(ui.available_width(), row_height), Sense::click());
            if index == app.emoji_selected {
                ui.painter().rect_filled(
                    rect,
                    SUGGESTION_ROW_RADIUS,
                    palette.accent.gamma_multiply(0.18),
                );
                ui.painter().rect_stroke(
                    rect,
                    SUGGESTION_ROW_RADIUS,
                    Stroke::new(1.0, palette.accent),
                    egui::StrokeKind::Inside,
                );
            } else if response.hovered() {
                ui.painter()
                    .rect_filled(rect, SUGGESTION_ROW_RADIUS, palette.surface_hover);
            }

            let emoji = widgets::line(
                ui,
                candidate.emoji,
                theme::regular(22.0),
                palette.text,
                30.0,
                1,
            );
            emoji.paint(
                ui,
                pos2(rect.left() + 6.0, rect.center().y - emoji.size().y / 2.0),
                palette.text,
            );
            let shortcode = widgets::line(
                ui,
                &candidate.shortcode,
                theme::medium(13.0),
                palette.text,
                (rect.width() * 0.4).max(100.0),
                1,
            );
            let text_x = rect.left() + 42.0;
            shortcode.paint(
                ui,
                pos2(text_x, rect.center().y - shortcode.size().y / 2.0),
                palette.text,
            );
            let name_x = text_x + shortcode.size().x + 12.0;
            let name = widgets::line(
                ui,
                candidate.name,
                theme::regular(12.5),
                palette.secondary,
                (rect.right() - name_x - 8.0).max(0.0),
                1,
            );
            name.paint(
                ui,
                pos2(name_x, rect.center().y - name.size().y / 2.0),
                palette.secondary,
            );
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                app.emoji_selected = index;
                picked = Some(candidate.clone());
            }
        }
    });
    ui.ctx()
        .data_mut(|data| data.insert_temp(suggestion_list_id(), list.response.rect));
    strip_gap(ui);
    if let Some(candidate) = picked {
        app.actions.push(Action::InsertEmojiCompletion {
            emoji: candidate.emoji.to_owned(),
            start,
            end,
        });
    }
}

/// Group-member suggestions above the composer.
fn mention_picker(app: &mut App, ui: &mut egui::Ui, chat: &Chat, field: egui::Id) {
    let cursor = egui::TextEdit::load_state(ui.ctx(), field)
        .and_then(|state| state.cursor.char_range())
        .map(|range| range.primary.index.0)
        .unwrap_or_else(|| app.composer.chars().count());
    let Some((end, query)) = active_mention(&app.composer, app.mention_start, cursor) else {
        app.mention_start = None;
        return;
    };
    let start = app.mention_start.expect("checked above");
    let candidates = app.mention_candidates(chat, query);
    if candidates.is_empty() {
        return;
    }
    let down = take_plain_key(ui, Key::ArrowDown);
    let up = take_plain_key(ui, Key::ArrowUp);
    if down {
        app.mention_selected = (app.mention_selected + 1) % candidates.len();
    }
    if up {
        app.mention_selected = (app.mention_selected + candidates.len() - 1) % candidates.len();
    }
    app.mention_selected = app.mention_selected.min(candidates.len() - 1);
    let submit = take_plain_key(ui, Key::Enter) || take_plain_key(ui, Key::Tab);
    let mut picked = submit.then(|| candidates[app.mention_selected].clone());
    let palette = app.palette;
    let everyone_alias = crate::mentions::everyone_alias(app.locale);

    let list = widgets::raised(ui, &palette, suggestion_frame(&palette), |ui| {
        let row_height = 38.0;
        egui::ScrollArea::vertical()
            .id_salt("mention-members")
            .max_height(row_height * candidates.len().min(5) as f32)
            .auto_shrink([false, true])
            .show_rows(ui, row_height, candidates.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for index in range {
                    let (id, label) = &candidates[index];
                    let (rect, response) = ui.allocate_exact_size(
                        vec2(ui.available_width(), row_height),
                        Sense::click(),
                    );
                    if index == app.mention_selected || response.hovered() {
                        ui.painter().rect_filled(
                            rect,
                            SUGGESTION_ROW_RADIUS,
                            palette.surface_hover,
                        );
                    }
                    let avatar = Rect::from_center_size(
                        pos2(rect.left() + 19.0, rect.center().y),
                        Vec2::splat(28.0),
                    );
                    let everyone = id == crate::mentions::ALL_ID;
                    if everyone {
                        ui.painter().circle_filled(
                            avatar.center(),
                            avatar.width() / 2.0,
                            palette.avatar(crate::util::hue(id)),
                        );
                        theme::paint_icon(
                            ui,
                            Icon::Users,
                            avatar,
                            avatar.width() * 0.5,
                            Color32::WHITE,
                        );
                    } else {
                        let picture = app.avatar(id);
                        widgets::paint_avatar(
                            ui,
                            &palette,
                            avatar,
                            label.trim_start_matches('~'),
                            id,
                            picture.as_deref(),
                        );
                    }
                    let detail = if everyone
                        && !everyone_alias.eq_ignore_ascii_case(crate::mentions::ALL_USER)
                    {
                        everyone_alias.clone()
                    } else {
                        crate::model::phone_of(id)
                            .map(crate::util::phone)
                            .unwrap_or_default()
                    };
                    let detail = widgets::line(
                        ui,
                        &detail,
                        theme::regular(11.5),
                        palette.secondary,
                        (rect.width() * 0.36).min(150.0),
                        1,
                    );
                    let name = widgets::line(
                        ui,
                        label,
                        theme::medium(13.5),
                        palette.text,
                        rect.width() - detail.size().x - 62.0,
                        1,
                    );
                    name.paint(
                        ui,
                        pos2(rect.left() + 40.0, rect.center().y - name.size().y / 2.0),
                        palette.text,
                    );
                    detail.paint(
                        ui,
                        pos2(
                            rect.right() - detail.size().x - 8.0,
                            rect.center().y - detail.size().y / 2.0,
                        ),
                        palette.secondary,
                    );
                    if response.hovered() {
                        app.mention_selected = index;
                    }
                    if (down || up) && index == app.mention_selected {
                        response.scroll_to_me(Some(Align::Center));
                    }
                    if response
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        picked = Some((id.clone(), label.clone()));
                    }
                }
            });
    });
    ui.ctx()
        .data_mut(|data| data.insert_temp(suggestion_list_id(), list.response.rect));
    strip_gap(ui);
    if let Some((id, name)) = picked {
        app.actions.push(Action::InsertMention {
            id,
            name,
            start,
            end,
        });
    }
}

/// Draws the composer and queues sends with the originating account and selected mentions.
fn composer(app: &mut App, ui: &mut egui::Ui, chat: &Chat) {
    let palette = app.palette;
    let shown = egui::Panel::bottom("composer")
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(Color32::TRANSPARENT)
                .inner_margin(Margin {
                    left: 8,
                    right: 8,
                    top: 0,
                    bottom: 8,
                }),
        )
        .show(ui, |ui| {
            if let Some((selected_chat, selected)) = app.selection.clone()
                && selected_chat == chat.id
            {
                selection_bar(app, ui, &chat.id, &selected);
                return;
            }
            if !chat.can_send() {
                if chat.kind == crate::model::ChatKind::Broadcast {
                    // A channel we left says so; the rest are only read-only.
                    ui.vertical_centered(|ui| {
                        theme::text(
                            ui,
                            if chat.left {
                                crate::i18n::gettext(app.locale, "You left this channel")
                            } else {
                                crate::i18n::gettext(
                                    app.locale,
                                    "Channels are read-only in ZapFast",
                                )
                            }
                            .as_ref(),
                            theme::regular(13.5),
                            palette.secondary,
                        );
                    });
                    return;
                }
                ui.vertical_centered(|ui| {
                    ui.add_space(8.0);
                    // A group we left says so instead of blaming the admins:
                    // either we left it here, or the phone says we are no
                    // longer a member.
                    let ours = app.our_ids();
                    let left = chat.left
                        || (!ours.is_empty()
                            && !chat.participants.is_empty()
                            && !chat.lists_any(&ours));
                    if left {
                        theme::text(
                            ui,
                            crate::i18n::gettext(app.locale, "You left this group"),
                            theme::regular(13.5),
                            palette.secondary,
                        );
                    } else {
                        ui.horizontal(|ui| {
                            let width = 230.0;
                            ui.add_space((ui.available_width() - width).max(0.0) / 2.0);
                            theme::text(ui, "Only", theme::regular(13.5), palette.secondary);
                            theme::text(ui, "admins", theme::semibold(13.5), palette.accent);
                            theme::text(
                                ui,
                                "can send messages",
                                theme::regular(13.5),
                                palette.secondary,
                            );
                        });
                    }
                    ui.add_space(8.0);
                });
                return;
            }
            // A refused voice message waits in its own chat only.
            let unsent_voice = app
                .unsent_voice
                .as_ref()
                .filter(|(unsent, _)| *unsent == chat.id)
                .map(|(_, samples)| samples.len());
            if let Some(samples) = unsent_voice
                && app.editing.is_none()
            {
                unsent_voice_strip(app, ui, samples);
            }
            if app.editing.is_some() {
                edit_strip(app, ui);
            } else if let Some(reply_id) = app.reply_to.clone() {
                let quoted = app.reply_preview(&reply_id, &chat.id).cloned();
                match quoted {
                    Some(quoted) => reply_strip(app, ui, &quoted),
                    None if reply_id.chat.is_some() => {
                        // Losing the source page must never silently turn a
                        // private reply into an unquoted send. The worker
                        // resolves the original from the archive on Send.
                        let palette = app.palette;
                        let strip = widgets::raised(ui, &palette, strip_frame(&palette), |ui| {
                            ui.set_width(ui.available_width().max(0.0));
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.set_max_width((ui.available_width() - 40.0).max(0.0));
                                    widgets::rich_text(
                                        ui,
                                        &crate::i18n::gettext(app.locale, "Original group message unavailable"),
                                        theme::regular(12.5),
                                        palette.secondary,
                                    );
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if theme::icon_button(
                                        ui, Icon::X, 16.0, palette.secondary, palette.text,
                                        "Cancel reply (Esc)",
                                    ).clicked() {
                                        app.actions.push(Action::CancelReply);
                                    }
                                });
                            });
                        });
                        ui.ctx().data_mut(|data| data.insert_temp(reply_strip_id(), strip.response.rect));
                        strip_gap(ui);
                    }
                    None => app.reply_to = None,
                }
            }
            app.prepare_composer_undo(ui.ctx());
            let id = composer_id();
            let has_focus = ui.memory(|memory| memory.has_focus(id));
            let enter_sends = app.settings.enter_sends;
            let (typed_colon, typed_at) = ui.input(|input| {
                let typed = |needle: &str| {
                    input
                        .events
                        .iter()
                        .any(|event| matches!(event, egui::Event::Text(text) if text == needle))
                };
                (has_focus && typed(":"), has_focus && typed("@"))
            });
            if !app.pending.is_empty() {
                pending_strip(app, ui);
            }
            if app.recording.is_some() {
                widgets::raised(ui, &palette, composer_pill(&palette), |ui| {
                    recording_strip(app, ui)
                });
                return;
            }
            emoji_suggestions(app, ui, id);
            mention_picker(app, ui, chat, id);
            // `consume_key(NONE, Enter)` also matches Shift+Enter. Check the
            // event modifiers directly. An active suggestion list consumes
            // plain Enter first when it has a selection.
            let send_key = has_focus
                && ui.input_mut(|input| {
                    let mut sent = false;
                    input.events.retain(|event| {
                        if sent {
                            return true;
                        }
                        let is_send = matches!(
                            event,
                            egui::Event::Key {
                                key: Key::Enter,
                                pressed: true,
                                modifiers,
                                ..
                            } if !modifiers.shift && !modifiers.alt
                                && (if enter_sends { !modifiers.command && !modifiers.ctrl } else { modifiers.command })
                        );
                        sent |= is_send;
                        !is_send
                    });
                    sent
                });
            let mut send_click = false;
            let line_height = ui
                .painter()
                .layout_no_wrap("x".to_owned(), theme::regular(BODY_SIZE), palette.text)
                .size()
                .y;
            // Every control sits in a band as tall as a one-line field at the
            // bottom of the row, centred on it. The field grows to six lines
            // above that band, so the buttons stay beside its last line.
            // A one-line field, like the composer before the rounded field:
            // the text line with padding, the send button as tall as that.
            let line = (line_height + COMPOSER_PADDING)
                .round()
                .max(COMPOSER_CONTROL);
            let button_width = line;
            let view_once = app.editing.is_none()
                && (app.pending_supports_view_once() || app.pending_view_once);
            let view_once_width = if view_once { COMPOSER_CONTROL + ui.spacing().item_spacing.x } else { 0.0 };
            let field_margin = ((line - line_height) / 2.0).round().max(0.0);
            // Measure this frame's draft at the text column's width, so the
            // field and the panel holding it grow on the keystroke that wraps
            // a line rather than a frame later, which made them jump.
            let wrap_id = id.with("wrap");
            let text_height = ui
                .ctx()
                .data(|data| data.get_temp::<f32>(wrap_id))
                .map(|wrap| {
                    let format =
                        egui::TextFormat::simple(theme::regular(BODY_SIZE), palette.text);
                    crate::bidi::layout_editor(ui, &app.composer, &format, wrap, true)
                        .0
                        .size()
                        .y
                })
                .or_else(|| ui.ctx().read_response(id).map(|previous| previous.rect.height()))
                .unwrap_or(line_height)
                .clamp(line_height, line_height * 6.0);
            let row_height = (text_height + 2.0 * field_margin).max(line);
            let pill = widgets::raised(ui, &palette, composer_pill(&palette), |ui| {
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), row_height),
                Layout::left_to_right(Align::Max),
                |ui| {
                // The plus sits in the field's rounded left end, centred as
                // the send button is in the right one.
                ui.add_space((line / 2.0 - PLUS_EDGE / 2.0).max(0.0));
                if app.editing.is_none() {
                    let tools = last_line(ui, line, |ui| theme::icon_button(
                        ui,
                        Icon::Plus,
                        22.0,
                        if app.composer_tools_open {
                            palette.accent
                        } else {
                            palette.secondary
                        },
                        palette.text,
                        &crate::i18n::gettext(app.locale, "Attach"),
                    ))
                    .tab_stop(Stop::Attach);
                    composer_tools_menu(app, chat, &tools);
                    // Plus and emoji sit close together, as a pair.
                    ui.add_space(COMPOSER_PAIR_GAP - ui.spacing().item_spacing.x);
                    let smile = last_line(ui, line, |ui| theme::icon_button(
                        ui,
                        Icon::Smile,
                        22.0,
                        if app.picker.is_some() {
                            palette.accent
                        } else {
                            palette.secondary
                        },
                        palette.text,
                        "Emoji, GIFs, and stickers",
                    )).tab_stop(Stop::Emoji);
                    app.picker_anchor = Some(smile.rect);
                    if smile.clicked() {
                        if app.composer_tools_open {
                            app.actions.push(Action::SetComposerTools(false));
                        }
                        app.actions.push(Action::TogglePicker(PickerTab::Emoji));
                    }
                    // The text follows the pair as closely as the emoji
                    // follows the plus (the field's own left margin included).
                    ui.add_space(COMPOSER_TEXT_GAP - ui.spacing().item_spacing.x - 8.0);
                }
                // The send button closes the row, flush with the field's end.
                let field_width =
                    (ui.available_width() - button_width - view_once_width - ui.spacing().item_spacing.x).max(0.0);
                // The ink is centred on the controls, not the line box: the
                // span from a capital's top to a descender's bottom sits as far
                // from the field's top as from its bottom. Inter's box leaves
                // more room above the capitals than below the descenders, and
                // at fractional scales the glyphs round to the pixel grid off
                // centre in it. Measured at this scale, snapped to a pixel.
                let ink_middle = ink_middle(ui, line_height);
                let top = fastframe_text::snap_to_pixels(
                    row_height - line / 2.0 - (text_height - line_height) - ink_middle,
                    ui.ctx().pixels_per_point(),
                )
                .max(0.0);
                let bottom = (row_height - text_height - top).max(0.0);
                Frame::new()
                    .fill(Color32::TRANSPARENT)
                    .inner_margin(Margin {
                        left: 8,
                        right: 8,
                        top: 0,
                        bottom: 0,
                    })
                    .show(ui, |ui| {
                        ui.set_width((field_width - 16.0).max(0.0));
                        ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.add_space(top);
                        // Grow from one to six lines, then scroll. The height
                        // is this frame's draft, measured above, rather than
                        // the scroll area's memory of the last frame.
                        egui::ScrollArea::vertical()
                            .id_salt("composer-scroll")
                            .max_height(text_height)
                            .min_scrolled_height(text_height)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                // Keep emoji in the buffer so character offsets match, then
                                // paint their color bitmaps over the transparent glyphs.
                                let mut clusters: Vec<(usize, usize, String)> = Vec::new();
                                let format = egui::TextFormat::simple(
                                    theme::regular(BODY_SIZE),
                                    palette.text,
                                );
                                let composer_rtl = crate::bidi::base_rtl(&app.composer);
                                let mut layouter = |ui: &egui::Ui,
                                                    text: &dyn egui::TextBuffer,
                                                    wrap: f32| {
                                    let (galley, found) = crate::bidi::layout_editor(
                                        ui,
                                        text.as_str(),
                                        &format,
                                        wrap,
                                        true,
                                    );
                                    clusters = found;
                                    galley
                                };
                                let wrap = ui.available_width();
                                ui.ctx().data_mut(|data| data.insert_temp(wrap_id, wrap));
                                let output = egui::TextEdit::multiline(&mut app.composer)
                                    .id(id)
                                    .frame(Frame::NONE)
                                    .margin(Margin::ZERO)
                                    .hint_text(
                                        egui::RichText::new(if app.pending.is_empty() {
                                            crate::i18n::gettext(app.locale, "Type a message")
                                                .into_owned()
                                        } else {
                                            crate::i18n::gettext(app.locale, "Add a caption")
                                                .into_owned()
                                        })
                                        .color(palette.dim)
                                        .font(theme::regular(BODY_SIZE)),
                                    )
                                    .font(theme::regular(BODY_SIZE))
                                    .text_color(palette.text)
                                    .desired_rows(1)
                                    .desired_width(f32::INFINITY)
                                    .horizontal_align(if composer_rtl {
                                        Align::RIGHT
                                    } else {
                                        Align::LEFT
                                    })
                                    .return_key(if enter_sends {
                                        Some(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter))
                                    } else {
                                        Some(KeyboardShortcut::new(Modifiers::NONE, Key::Enter))
                                    })
                                    .layouter(&mut layouter)
                                    .show(ui);
                                for (start, length, cluster) in &clusters {
                                    let Some(bounds) = crate::bidi::char_bounds(
                                        &output.galley,
                                        *start,
                                        start + length,
                                    ) else {
                                        continue;
                                    };
                                    // Skip emoji clusters split across rows.
                                    if bounds.height() > line_height * 1.5 {
                                        continue;
                                    }
                                    let rect = bounds.translate(output.galley_pos.to_vec2());
                                    crate::emoji::paint_cluster(ui, cluster, rect);
                                }
                                // The row was sized from last frame's text. When a
                                // keystroke wraps or unwraps a line, lay the frame
                                // out again instead of showing the field a frame
                                // late, which made it jump while typing.
                                // egui sizes the box before applying the keystroke,
                                // and text typed into an empty field reaches its
                                // galley a frame later still, so measure the
                                // edited draft itself.
                                let painted = Rect::from_min_size(
                                    output.galley_pos,
                                    output.galley.size(),
                                );
                                let measured = crate::bidi::layout_editor(
                                    ui,
                                    &app.composer,
                                    &format,
                                    wrap,
                                    true,
                                )
                                .0
                                .size()
                                .y
                                .clamp(line_height, line_height * 6.0);
                                ui.ctx().data_mut(|data| {
                                    data.insert_temp(composer_text_id(), painted);
                                });
                                if (measured - text_height).abs() > 0.5 {
                                    ui.ctx().request_discard("composer height changed");
                                }
                                let response = output.response.response.clone().tab_stop(Stop::Composer);
                                ui.ctx().accesskit_node_builder(response.id, |node| node.set_label("Message"));
                                if response.changed() {
                                    app.actions.push(Action::Composing {
                                        chat: chat.id.clone(),
                                        composing: true,
                                    });
                                    let cursor = output
                                        .cursor_range
                                        .map(|range| range.primary.index.0)
                                        .unwrap_or_else(|| app.composer.chars().count());
                                    if typed_colon
                                        && let Some(at) = standalone_trigger(
                                            &app.composer,
                                            cursor,
                                            ':',
                                        )
                                    {
                                        app.picker = None;
                                        app.emoji_start = Some(at);
                                        app.emoji_selected = 0;
                                        app.mention_start = None;
                                    } else if typed_at
                                        && chat.is_group()
                                        && !chat.participants.is_empty()
                                        && let Some(at) = standalone_trigger(
                                            &app.composer,
                                            cursor,
                                            '@',
                                        )
                                    {
                                        app.emoji_start = None;
                                        app.mention_start = Some(at);
                                        app.mention_selected = 0;
                                    } else if app.emoji_start.is_some() {
                                        app.emoji_selected = 0;
                                    }
                                }
                                let cursor = output
                                    .cursor_range
                                    .map(|range| range.primary.index.0)
                                    .unwrap_or_else(|| app.composer.chars().count());
                                if app.emoji_start.is_some()
                                    && active_emoji(&app.composer, app.emoji_start, cursor).is_none()
                                {
                                    app.emoji_start = None;
                                }
                                if app.mention_start.is_some()
                                    && active_mention(&app.composer, app.mention_start, cursor)
                                        .is_none()
                                {
                                    app.mention_start = None;
                                }
                                // The composer waits for the image preview to
                                // close before taking focus back.
                                if app.focus_composer
                                    && app.image_preview.is_none()
                                    && app.sticker_view.is_none()
                                {
                                    app.focus_composer = false;
                                    response.request_focus();
                                }
                            });
                        ui.add_space(bottom);
                        });
                    });
                if view_once
                    && last_line(ui, line, |ui| view_once_button(app, ui))
                        .tab_stop(Stop::ViewOnce)
                        .clicked()
                {
                    app.actions.push(Action::SetPendingViewOnce(!app.pending_view_once));
                }
                let ready = !app.composer.trim().is_empty()
                    || !app.pending.is_empty()
                    || (unsent_voice.is_some() && app.editing.is_none());
                let (fill, hover, icon) = if ready {
                    (palette.accent, palette.accent_hover, palette.on_accent)
                } else {
                    (palette.surface, palette.surface_hover, palette.dim)
                };
                last_line(ui, line, |ui| if !ready && app.editing.is_none() {
                    // An empty composer changes the send button to record.
                    if theme::circle_button(
                        ui,
                        Icon::Mic,
                        button_width,
                        fill,
                        hover,
                        palette.secondary,
                        "Record a voice message",
                    )
                    .tab_stop(Stop::Send)
                    .clicked()
                    {
                        app.actions.push(Action::StartRecording);
                    }
                } else {
                    let icon_kind = if app.editing.is_some() {
                        Icon::Check
                    } else {
                        Icon::Send
                    };
                    if theme::circle_button(ui, icon_kind, button_width, fill, hover, icon, "Send")
                        .tab_stop(Stop::Send)
                        .clicked()
                    {
                        send_click = true;
                    }
                });
            },
                    );
                });
            theme::focus_outline(ui, id, pill.response.rect, f32::from(COMPOSER_RADIUS));
            ui.ctx()
                .data_mut(|data| data.insert_temp(composer_pill_id(), pill.response.rect));
            if (send_key || send_click)
                && (!app.composer.trim().is_empty() || !app.pending.is_empty())
            {
                let text = std::mem::take(&mut app.composer);
                if app.pending.is_empty() {
                    app.actions.push(Action::SendText {
                        chat: chat.id.clone(),
                        text,
                        quoting: app.reply_to.clone(),
                    });
                } else {
                    app.actions.push(Action::SendPending {
                        account: app.account().id.clone(),
                        mentions: app.composer_mentions(),
                        chat: chat.id.clone(),
                        caption: text,
                    });
                }
                app.focus_composer = true;
            } else if (send_key || send_click) && unsent_voice.is_some() && app.editing.is_none() {
                // An empty composer with a refused voice message: Send tries
                // that message again.
                app.actions.push(Action::SendRecording);
                app.focus_composer = true;
            }
            if app.settings.show_shortcut_hints {
                let hint_text = if enter_sends {
                    crate::i18n::gettext(app.locale, "Enter sends · Shift+Enter for a new line · *bold* _italic_ ~strike~ · Ctrl+V pastes a picture")
                } else {
                    crate::i18n::gettext(app.locale, "Ctrl+Enter sends · *bold* _italic_ ~strike~ · Ctrl+V pastes a picture")
                };
                let hint = super::keys::label(hint_text.as_ref());
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    if theme::icon_button(
                        ui,
                        Icon::X,
                        13.0,
                        palette.dim,
                        palette.secondary,
                        crate::i18n::gettext(app.locale, "Hide shortcut hints (bring them back from Keyboard shortcuts)").as_ref(),
                    )
                    .clicked()
                    {
                        app.actions.push(Action::SetShortcutHints(false));
                    }
                    theme::text(ui, &hint, theme::regular(11.0), palette.dim);
                    // Open the shortcut list without consuming typed `?`.
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::icon_button(
                            ui,
                            Icon::Keyboard,
                            13.0,
                            palette.dim,
                            palette.secondary,
                            &crate::i18n::gettext(app.locale, "All shortcuts ({})")
                                .replace("{}", &super::keys::label("Ctrl+/")),
                        )
                        .clicked()
                        {
                            app.actions.push(Action::ShowDialog(Dialog::Shortcuts));
                        }
                    });
                });
            }
        });
    // A bottom panel is placed from last frame's height. When the composer
    // grows or shrinks, lay the frame out again so it never shows the field
    // hanging below the window or a gap above it for a frame.
    let height = shown.response.rect.height();
    let height_id = egui::Id::new("composer-panel-height");
    let previous = ui.ctx().data_mut(|data| {
        let previous = data.get_temp::<f32>(height_id);
        data.insert_temp(height_id, height);
        previous
    });
    if previous.is_some_and(|previous| (previous - height).abs() > 0.5) {
        ui.ctx().request_discard("composer height changed");
    }
    // Toasts sit above the composer so they never cover its buttons.
    ui.ctx()
        .data_mut(|data| data.insert_temp(super::composer_rect_id(), shown.response.rect));
}

/// Corner radius of the composer's rounded field.
const COMPOSER_RADIUS: u8 = 24;
/// Padding around the text of a one-line composer row. The row, and the send
/// and record button, are one text line plus this; the controls are centred
/// on it.
const COMPOSER_PADDING: f32 = 14.0;
/// Height of the plus and emoji buttons: a row is never shorter, or they
/// would stretch it and pull the text off its centre.
const COMPOSER_CONTROL: f32 = 36.0;
/// Space between the composer's rounded field and the buttons at its ends.
const COMPOSER_INSET: i8 = 2;
/// Space between the plus and emoji buttons' hit areas. Each area pads its
/// 22-point icon by six points a side, so this leaves eight between the
/// icons, a pair.
const COMPOSER_PAIR_GAP: f32 = -4.0;
/// From the emoji button's edge to the text. The plus's glyph leaves more of
/// its box empty than the round emoji's, so this is wider than the pair's
/// gap: at it, the text starts as far from the emoji's ink as the emoji
/// starts from the plus's.
pub(crate) const COMPOSER_TEXT_GAP: f32 = 6.0;
/// Width of the plus button: its 22-point icon and `icon_button`'s padding.
const PLUS_EDGE: f32 = 34.0;

/// The message field.
pub(crate) fn composer_id() -> egui::Id {
    egui::Id::new("composer-text")
}

/// Where the composer's text was painted in the frame's last pass, for
/// layout tests.
pub(crate) fn composer_text_id() -> egui::Id {
    egui::Id::new("composer-text-rect")
}

/// Where the composer's rounded field was drawn, for layout tests.
pub(crate) fn composer_pill_id() -> egui::Id {
    egui::Id::new("composer-pill")
}

/// The open chat's message list scroll offset and viewport height, as
/// `(offset.y, height)`, as of its last frame: for tests that need them
/// precisely. `App::at_bottom` covers ordinary UI checks.
pub(crate) fn scroll_metrics_id(chat: &ChatId) -> egui::Id {
    egui::Id::new(("message-scroll-metrics", chat))
}

/// How far below the top of a composer text row the middle of its ink sits,
/// at this scale: halfway from a capital's top to a descender's bottom.
fn ink_middle(ui: &egui::Ui, line_height: f32) -> f32 {
    let format = egui::TextFormat::simple(theme::regular(BODY_SIZE), Color32::WHITE);
    let (galley, _) = crate::bidi::layout_editor(ui, "Hy", &format, f32::INFINITY, true);
    widgets::ink_middle(&galley).unwrap_or(line_height / 2.0)
}

/// Lays out a control centred in the composer's last-line band, however
/// tall the draft has grown, so every control shares one vertical centre.
fn last_line<R>(ui: &mut egui::Ui, line: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.allocate_ui_with_layout(vec2(0.0, line), Layout::left_to_right(Align::Center), add)
        .inner
}

/// The rounded field that holds the composer's controls, or the recorder.
fn composer_pill(palette: &Palette) -> Frame {
    // Glass over a vibrant conversation, as the header's.
    let (fill, edge) = if palette.vibrant() {
        (palette.glass(), palette.glass_edge())
    } else {
        (palette.bubble_in, Stroke::NONE)
    };
    Frame::new()
        .fill(fill)
        .stroke(edge)
        .corner_radius(CornerRadius::same(COMPOSER_RADIUS))
        // The end buttons are inset by as much at the sides as above and
        // below, so they sit evenly in the rounded ends.
        .inner_margin(Margin {
            left: COMPOSER_INSET,
            right: COMPOSER_INSET,
            top: COMPOSER_INSET,
            bottom: COMPOSER_INSET,
        })
}

/// The plus menu beside the composer: send files, documents or a location,
/// or create a poll.
fn composer_tools_menu(app: &mut App, chat: &Chat, plus: &egui::Response) {
    let id = plus.id.with("composer-tools");
    let mut open = app.composer_tools_open;
    let picker_open = app.picker.is_some();
    if plus.clicked() {
        open = !open;
        if open && picker_open {
            app.actions.push(Action::ClosePicker);
        }
    }
    let mut draw_open = open;
    if draw_open && !picker_open {
        egui::Popup::menu(plus)
            .id(id)
            .open_bool(&mut draw_open)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
            .width(190.0)
            .frame(widgets::menu_frame(&app.palette))
            .show(|ui| {
                let send_files = crate::i18n::gettext(app.locale, "Send files");
                if widgets::menu_item(ui, &app.palette, Some(Icon::Paperclip), &send_files) {
                    app.actions.push(Action::Attach { documents: false });
                }
                // Pictures picked here keep their own bytes instead of
                // becoming compressed photos.
                let document = crate::i18n::gettext(app.locale, "Send as document");
                if widgets::menu_item(ui, &app.palette, Some(Icon::FileText), &document) {
                    app.actions.push(Action::Attach { documents: true });
                }
                let send_location = crate::i18n::gettext(app.locale, "Send location");
                if widgets::menu_item(ui, &app.palette, Some(Icon::MapPin), &send_location) {
                    app.actions
                        .push(Action::ShowDialog(Dialog::SendLocation(chat.id.clone())));
                }
                let create_poll = crate::i18n::gettext(app.locale, "Create poll");
                if widgets::menu_item(ui, &app.palette, Some(Icon::ListChecks), &create_poll) {
                    app.actions
                        .push(Action::ShowDialog(Dialog::CreatePoll(chat.id.clone())));
                }
            });
    }
    if draw_open != app.composer_tools_open {
        app.actions.push(Action::SetComposerTools(draw_open));
    }
}

/// Offers a refused voice message for another try, or to discard it.
/// From a strip above the composer (reply, edit, unsent voice) to the field
/// it belongs to: close enough to read as one piece.
pub(crate) const STRIP_GAP: f32 = 3.0;

/// Leaves [`STRIP_GAP`] below a strip, the layout's own spacing included.
fn strip_gap(ui: &mut egui::Ui) {
    ui.add_space(STRIP_GAP - ui.spacing().item_spacing.y);
}

/// Corner radius of the strips above the composer and of its suggestion
/// lists: rounder than a bubble, closer to the composer's own ends.
const STRIP_RADIUS: u8 = 16;

/// The frame of a strip above the composer (reply, edit, unsent voice).
/// Like the composer it takes the incoming bubble's colour, which in the
/// light theme is white on the chat rather than the grey interface surface.
/// Its content starts 14 points in, so a reply's accent bar lines up with
/// the composer's plus below it.
fn strip_frame(palette: &Palette) -> Frame {
    Frame::new()
        .fill(palette.bubble_in)
        .corner_radius(CornerRadius::same(STRIP_RADIUS))
        .inner_margin(Margin {
            left: 14,
            right: 10,
            top: 6,
            bottom: 6,
        })
}

/// The emoji and @mention suggestion lists, raised above the composer like
/// the strips, with the same fill and corners.
fn suggestion_frame(palette: &Palette) -> Frame {
    Frame::new()
        .fill(palette.bubble_in)
        .corner_radius(CornerRadius::same(STRIP_RADIUS))
        .inner_margin(Margin::same(SUGGESTION_INSET))
}

/// Space between a suggestion list's edge and its rows.
const SUGGESTION_INSET: i8 = 4;
/// A suggestion row's corners, concentric with the list's.
const SUGGESTION_ROW_RADIUS: f32 = (STRIP_RADIUS as i8 - SUGGESTION_INSET) as f32;

fn unsent_voice_strip(app: &mut App, ui: &mut egui::Ui, samples: usize) {
    let palette = app.palette;
    let seconds = (samples as f64 / f64::from(crate::voice::RATE))
        .round()
        .max(1.0) as u32;
    let label = crate::i18n::gettext(app.locale, "Voice message ({duration}) not sent")
        .replace("{duration}", &crate::util::duration(seconds));
    let discard = crate::i18n::gettext(app.locale, "Discard voice message");
    widgets::raised(ui, &palette, strip_frame(&palette), |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            theme::icon(ui, Icon::Mic, 16.0, palette.danger);
            theme::text(ui, &label, theme::semibold(12.5), palette.danger);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let button = theme::icon_button(
                    ui,
                    Icon::X,
                    16.0,
                    palette.secondary,
                    palette.text,
                    discard.as_ref(),
                );
                #[cfg(test)]
                ui.ctx().data_mut(|data| {
                    data.insert_temp(egui::Id::new("unsent-voice-discard"), button.rect)
                });
                if button.clicked() {
                    app.actions.push(Action::DiscardUnsentVoice);
                }
            });
        });
    });
    strip_gap(ui);
}

fn edit_strip(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    widgets::raised(ui, &palette, strip_frame(&palette), |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            theme::icon(ui, Icon::Pencil, 16.0, palette.accent);
            theme::text(ui, "Editing message", theme::semibold(12.5), palette.accent);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if theme::icon_button(
                    ui,
                    Icon::X,
                    16.0,
                    palette.secondary,
                    palette.text,
                    "Stop editing (Esc)",
                )
                .clicked()
                {
                    app.actions.push(Action::CancelEdit);
                }
            });
        });
    });
    strip_gap(ui);
}

fn reply_strip(app: &mut App, ui: &mut egui::Ui, quoted: &Message) {
    let palette = app.palette;
    let who = if quoted.from_me {
        "You".to_owned()
    } else {
        app.display_name_or(&quoted.sender, quoted.sender_name.as_deref())
    };
    let summary = markup::plain(&quoted.summary_in(app.locale), &app.mention_list(quoted));
    let strip = widgets::raised(ui, &palette, strip_frame(&palette), |ui| {
        ui.set_width(ui.available_width().max(0.0));
        ui.horizontal(|ui| {
            let (bar, _) = ui.allocate_exact_size(vec2(3.0, 34.0), Sense::hover());
            ui.painter().rect_filled(bar, 2.0, palette.accent);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.set_max_width((ui.available_width() - 40.0).max(0.0));
                widgets::rich_text(
                    ui,
                    &format!("Replying to {who}"),
                    theme::semibold(12.5),
                    palette.accent,
                );
                widgets::rich_text(ui, &summary, theme::regular(12.5), palette.secondary);
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if theme::icon_button(
                    ui,
                    Icon::X,
                    16.0,
                    palette.secondary,
                    palette.text,
                    "Cancel reply (Esc)",
                )
                .clicked()
                {
                    app.actions.push(Action::CancelReply);
                }
            });
        });
    });
    ui.ctx()
        .data_mut(|data| data.insert_temp(reply_strip_id(), strip.response.rect));
    strip_gap(ui);
}

/// Where the emoji or @mention suggestion list was drawn, for layout tests.
pub(crate) fn suggestion_list_id() -> egui::Id {
    egui::Id::new("suggestion-list")
}

/// Where the reply strip was drawn, for layout tests.
pub(crate) fn reply_strip_id() -> egui::Id {
    egui::Id::new("reply-strip")
}

/// App data needed while drawing a checked-out conversation.
struct View<'a> {
    palette: Palette,
    locale: crate::i18n::Locale,
    chat: &'a Chat,
    me: Option<&'a str>,
    auto_download: bool,
    connected: bool,
    account_receipts_off: bool,
    poll_voting: &'a HashSet<(ChatId, String)>,
    interactive_pending: &'a HashSet<(ChatId, String)>,
    /// Whether this chat is selecting whole messages.
    selecting: bool,
    anchor: Option<&'a str>,
    /// Open this message's context menu (`App::open_message_menu`).
    open_menu: Option<&'a str>,
    /// Ids of the starred messages of this chat, for the mark in the footer.
    starred: Option<&'a HashSet<String>>,
    /// Ids of this chat's pinned messages, for the menu and the bubble mark.
    pins: Option<&'a HashSet<String>>,
    /// Active pins of this chat, for the notices in the transcript.
    pin_notices: &'a [crate::archive::Pinned],
    reaction: Option<&'a str>,
    /// The reaction picker was opened from the message's context menu.
    reaction_menu: bool,
    reaction_emoji: &'a [(String, u32)],
    keyboard_navigation: &'a std::cell::Cell<bool>,
    /// Resolves a name with the message's stored name as fallback.
    names_or: &'a dyn Fn(&str, Option<&str>) -> String,
    /// Resolves mention names without replacing our name with "You".
    mention_names: &'a dyn Fn(&str) -> String,
    avatars: &'a HashMap<String, Option<PathBuf>>,
    contacts: &'a HashMap<String, crate::model::Contact>,
    now: i64,
    /// Animate media only while this window is active.
    animate: bool,
    /// The motion photo of this chat whose clip plays.
    motion_playing: Option<&'a str>,
    player: &'a crate::audio::Player,
    /// The speed this chat's voice messages play at.
    voice_speed: f32,
    video: &'a crate::video::Player,
    copy_rows: Option<&'a std::sync::Mutex<Vec<crate::transcript::Row>>>,
}

/// Protocol album parent carried by pictures and ordinary videos.
fn album_id(message: &Message) -> Option<&str> {
    match &message.content {
        Content::Image { media, .. }
        | Content::Video {
            media, note: false, ..
        } => media.album.as_deref(),
        _ => None,
    }
}

/// Whether the message at `index` is drawn in the album row of the one
/// before it, as the list draws albums outside a selection: it then has no
/// row of its own.
fn album_continues(messages: &[Message], index: usize) -> bool {
    let Some(previous) = index.checked_sub(1).and_then(|before| messages.get(before)) else {
        return false;
    };
    let message = &messages[index];
    album_id(message).is_some()
        && album_id(previous) == album_id(message)
        && previous.from_me == message.from_me
        && previous.sender == message.sender
        && crate::util::day_key(previous.timestamp) == crate::util::day_key(message.timestamp)
}

/// The index of the first message of each row the message list draws: the
/// list's items, which its cursor steps through. Every message has its own
/// row, except an album's items outside a selection, which share their
/// first item's.
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

/// The consecutive messages belonging to the album beginning at `start`.
/// A single marked item is rendered normally because there is nothing to group.
fn album_run(messages: &[Message], start: usize) -> Option<&[Message]> {
    let first = messages.get(start)?;
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

/// A row height to assume for a message that has not been laid out yet. Rows
/// near the viewport are always measured, and a change in the height of a row
/// above the viewport moves the scroll offset with it, so this only shapes the
/// scrollbar until the reader scrolls near the row.
fn estimated_height(message: &Message, width: f32, new_day: bool, notices: usize) -> f32 {
    // Bubbles take at most 72% of the transcript, and 560 points.
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
        _ => 40.0,
    };
    // Bubble padding, the sender line, and the row spacing, plus the date
    // chip above the first message of a day, and any pin notice this row
    // carries.
    40.0 + body + if new_day { 36.0 } else { 0.0 } + notices as f32 * PIN_NOTICE_HEIGHT
}

/// The pins the transcript places, oldest first. `chat_pins` hands them over
/// newest first, and a notice belongs above the first message that came after
/// its pin, so the list goes back into time order before it is placed.
fn pin_notices_in_time_order(rows: &[crate::archive::Pinned]) -> Vec<crate::archive::Pinned> {
    let mut sorted = rows.to_vec();
    sorted.sort_by_key(|row| row.pinned_at);
    sorted
}

/// How many notices, from the front of the list, belong above a message that
/// arrived at `timestamp`.
fn notices_above(notices: &[crate::archive::Pinned], timestamp: i64) -> usize {
    notices
        .iter()
        .take_while(|notice| notice.pinned_at <= timestamp)
        .count()
}

/// The notice a pin leaves in the transcript, where the pin happened, as the
/// phone shows it.
fn pin_notice(
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

/// The words a pin notice shows: who pinned it, or a neutral line when the
/// archive does not say.
fn pin_notice_label(
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

/// The pins the line under the header shows: one that has run out its seven
/// days leaves, whether or not a refresh has reached this window yet.
fn live_pins(rows: &[crate::archive::Pinned], now: i64) -> Vec<crate::archive::Pinned> {
    rows.iter()
        .filter(|row| row.expires_at > now)
        .cloned()
        .collect()
}

/// What one pin notice takes in the transcript: the chip and its spacing.
const PIN_NOTICE_HEIGHT: f32 = 32.0;

/// Incoming messages carry their sender's picture and name in groups only,
/// as on WhatsApp.
fn shows_sender_pictures(chat: &Chat) -> bool {
    chat.is_group()
}

/// How long the sticky date stays after the transcript stops moving.
const STICKY_DATE_HOLD: f64 = 1.0;
/// How long the sticky date takes to fade out.
const STICKY_DATE_FADE: f32 = 0.25;

/// Keeps the first visible message's date at the top of the transcript once
/// its day separator has scrolled away, while the reader scrolls: it fades
/// out a moment after the transcript stops moving, as on the phone.
/// Following new messages at the bottom does not count as scrolling.
fn sticky_date<R>(
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
        // One wake-up for the moment the fade begins; the fade itself
        // repaints through `animate_bool` and stops once it is done.
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
    // Paint after the scroll contents, inside their clip, without changing
    // their height or taking clicks away from the messages underneath.
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

/// Message endpoints whose text must remain registered while it is selected.
/// Stable message ids survive older-page insertion and row-height changes.
#[derive(Clone)]
struct TextSelectionRows {
    anchor: String,
    extent: String,
}

/// The conversation's messages. `under` is how much of the list's top lies
/// beneath the header, which a vibrant window lays over it.
fn messages(app: &mut App, ui: &mut egui::Ui, chat: &Chat, under: f32) {
    let palette = app.palette;
    // Taken up front: `names_or` below borrows the rest of `app` for the
    // whole function, so a pending scroll must come out before that.
    let pending_scroll = app.scroll_page.take();
    // A scroll gesture that began over the messages stays with them when
    // the pointer drifts off (#274), taken here for the same reason.
    let carried = app.scroll_route.take(crate::app::ScrollPane::Messages);
    // An explicit jump (Ctrl+End, or the return-to-bottom button) must reach
    // the bottom even while a message bubble retains keyboard focus.
    let scroll_forced = std::mem::take(&mut app.scroll_to_bottom_forced);
    // The message list's one-shot requests, taken for the same reason.
    let list_id = messages_list_id(&chat.id);
    // Whether focus last moved by keyboard. The list's focus takeover, its
    // reveal and its hold on the pin below all wait for it, so the pointer
    // sees the chat exactly as before the list existed.
    let keyboard = ui.ctx().data(|data| {
        data.get_temp::<bool>(theme::keyboard_focus_id())
            .unwrap_or(false)
    });
    // A chat that cannot send (a channel, a locked chat, a group we left)
    // draws no message field, so `composer` above left its focus request
    // pending. After keyboard navigation the list, all there is to read,
    // takes that focus instead, in this pass, which draws it: a reader
    // switching chats from the list would otherwise land on nothing. Opened
    // with the pointer, focus stays where it was, so the page keys still
    // scroll the view.
    // Never under a dialog, whose own controls hold focus.
    let composer_fallback = app.focus_composer
        && keyboard
        && !chat.can_send()
        && app.selection.is_none()
        && app.image_preview.is_none()
        && app.dialog.is_none();
    // Declined with the pointer, the request is dropped, not left pending:
    // the field it names does not exist here, and a later Tab would
    // otherwise take it up and scroll the view to the current message.
    if composer_fallback || (!chat.can_send() && !keyboard) {
        app.focus_composer = false;
    }
    // A pass that draws the list hidden (the first after a chat opens is)
    // gives no widget focus: the request waits for the pass that shows it.
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
    // The current message: the one the cursor names, or the newest when it
    // names none, or a message that is gone. An album's item stands for its
    // row, which its first item draws.
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
    if shows_sender_pictures(chat) {
        let mut senders: HashSet<String> = conversation
            .messages
            .iter()
            .filter(|message| !message.from_me)
            .map(|message| message.sender.clone())
            .collect();
        // A typing participant may not have a visible message.
        senders.extend(typing.iter().map(|(id, _, _)| id.clone()));
        for sender in senders {
            let picture = app.avatar(&sender);
            avatars.insert(sender, picture);
        }
    }
    let names_or = |id: &str, hint: Option<&str>| app.display_name_or(id, hint);
    let mention_names = |id: &str| app.mention_name(id);
    let keyboard_navigation = std::cell::Cell::new(false);
    // The list's name says whose messages it holds. Built only while a screen
    // reader is attached, like the rows' labels, once the list's node exists.
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
        // Whole-message exports are already formatted. Do not let the
        // text-selection annotator reinterpret them as drawn body text.
        copy_rows: (!app.selection.as_ref().is_some_and(|(id, _)| *id == chat.id))
            .then_some(app.copy_rows.as_ref()),
    };
    let mut actions = Vec::new();
    let mut anchored = false;
    // The first unread message is the count-th incoming one from the end.
    let divider = app
        .unread_divider
        .as_ref()
        .filter(|divider| divider.chat == chat.id)
        .and_then(|divider| {
            conversation
                .messages
                .iter()
                .rev()
                .filter(|message| !message.from_me)
                .nth(divider.count.saturating_sub(1) as usize)
                .map(|message| (message.id.clone(), divider.count, divider.placed))
        });
    let mut divider_placed = false;
    let selection: Option<Vec<String>> = app
        .selection
        .as_ref()
        .filter(|(selected_chat, _)| *selected_chat == chat.id)
        .map(|(_, ids)| ids.clone());
    // A sweep over messages (#246) follows the pointer while the button is
    // held, whichever row took the press, and ends on release.
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
    // The row under the pointer: the last laid-out row whose top it is
    // below, or the first one when it is above them all.
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
    // Do not animate programmatic scrolling. Pending animations can delay a
    // later request to reach the end. Keyboard page, Home, and End scrolls
    // ease by applying small instant steps each frame instead (`KeyScroll`).
    let mut edge_scrolled_up = false;
    // egui must see both selection endpoints and the text between them, not
    // the entire history. Keep that span plus nearby rows laid out, so a
    // selection does not remeasure distant rows and move the view.
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
    // Jumps and first unread-divider placement need exact absolute positions.
    // Normal reading and text selection retain virtualization.
    let layout_width = ui.available_width();
    let lay_out_all = view.anchor.is_some() || divider.as_ref().is_some_and(|(.., placed)| !placed);
    let pass = ui.ctx().cumulative_pass_nr();
    let redo = ui.ctx().current_pass_index() > 0;
    let mut rows = std::mem::take(&mut conversation.rows);
    // Forget rows that left the conversation, such as deleted messages.
    if rows.len() > conversation.messages.len() * 2 + 64 {
        let ids: HashSet<&str> = conversation
            .messages
            .iter()
            .map(|message| message.id.as_str())
            .collect();
        rows.retain(|id, _| ids.contains(id.as_str()));
    }
    // How far rows entirely above the viewport grew this frame. The offset
    // follows, so what the reader looks at stays put while rows scrolled past
    // are measured for the first time.
    let mut grew_above = 0.0;
    // The offset the message list is about to load, the distance a Home
    // scroll still has to cover: the same id the `ScrollArea` below loads.
    // `.id_salt()` hashes its salt into an `IdSalt` before combining it with
    // the `Ui`'s id, so the salt must go through the same `IdSalt::new` here
    // to land on the same id.
    let scroll_id = ui.make_persistent_id(egui::IdSalt::new(("messages", &chat.id)));
    let scroll_state = egui::scroll_area::State::load(ui.ctx(), scroll_id);
    // The list has not been shown before: it starts at the top.
    let first_show = scroll_state.is_none();
    let offset = scroll_state.map_or(0.0, |state| state.offset.y);
    // A keyboard scroll under way stops for a jump to a message, for the
    // reaction bar, which holds the view still, and for a wheel or trackpad
    // turn over the message list, before it takes another step.
    let mut key_scroll = conversation.key_scroll.take();
    let list = ui.available_rect_before_wrap();
    // The list itself: one Tab stop over the whole message view, created
    // before the rows so their nodes can hang from its node. It senses no
    // pointer input. Asked for by a shortcut, it takes focus here, while it
    // is drawn, so focus never names a node missing from this frame's tree.
    if focus_list {
        ui.memory_mut(|memory| memory.request_focus(list_id));
    }
    let list_response =
        super::list_nav::container(ui, list_id, list, list_label, total).tab_stop(Stop::Messages);
    let list_focused = list_response.has_focus();
    // Both below only after keyboard navigation, as for a focused bubble.
    // Arriving in the list shows where its current message is.
    if list_response.gained_focus() && keyboard {
        reveal = true;
    }
    // A reader moving through the list keeps the view where they are: an
    // automatic pin to the newest message must not pull it away.
    if list_focused && keyboard {
        keyboard_navigation.set(true);
    }
    // Except on the newest message the keys took the view to the true
    // bottom (`App::cursor_followed`), or in a list with no cursor of its
    // own, which stands on the newest: there the pin holds as it does after
    // the return-to-bottom button, so new messages are followed.
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
    // A key scroll starts one frame back, so it moves on its first frame and
    // a held key's repeats, which restart it, never stall it.
    let key_start = time - f64::from(ui.input(|input| input.stable_dt.min(0.1)));
    let mut pinned = false;
    let mut visible_day = None;
    // A pending stick-to-bottom adjustment may already have changed the stored
    // offset since the last paint. Start a text drag at the offset the user saw,
    // and let edge scrolling move it instead of snapping away from the pointer.
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
            // Scroll while selecting near an edge. `scroll_with_delta` also
            // releases stick-to-bottom; setting the offset directly does not.
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
            // Keep ordinary conversation-space clicks useful: after reading,
            // the next keystroke should go straight to the composer. Register
            // this before the message controls so text, links, media, and
            // selection interactions remain in front of the background.
            let background = ui.interact(
                viewport,
                ui.id().with(("message-background", &chat.id)),
                Sense::click(),
            );
            // Only a drag that has moved past a click, such as selecting
            // text, scrolls; a click near an edge does not.
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
            // A chat shorter than the view sits at its bottom, next to the
            // composer, as on WhatsApp: the space above it is what the
            // messages left free last frame.
            let natural_id = scroll_id.with("natural-height");
            let natural = ui.ctx().data(|data| data.get_temp::<f32>(natural_id));
            // The spacing after the filler is part of the space it takes.
            let filler = bottom_filler(viewport.height(), natural, ui.spacing().item_spacing.y);
            // While a button is held (a sweep, a text selection, a click),
            // the rows stay where the pointer found them: a sweep turns the
            // composer into the selection bar, and the view's new height
            // would otherwise move every row under the pointer.
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
                    // The pin notices still to place, in time order. Each one
                    // sits above the first message that came after it, so the
                    // row it belongs to carries its height.
                    let notices = pin_notices_in_time_order(view.pin_notices);
                    let mut placed = 0;
                    // Rows within a few viewports of the screen are laid out
                    // and their height remembered, so scrolling finds them
                    // measured before they show.
                    let margin = (viewport.height() * 3.0).max(600.0);
                    for (message_index, message) in conversation.messages.iter().enumerate() {
                        let index = message_index;
                        // WhatsApp marks every item in a media album with the
                        // same parent id. Only its first item owns a row; the
                        // remaining consecutive items are painted in that row.
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
                        // A pass redone after the offset followed rows that
                        // grew keeps to the rows it laid out before (and any
                        // now on screen): measuring rows the shift brought
                        // into range would move the view once more.
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
                            // The body rect of a row that just left the
                            // layout marks where it last was, not where it is.
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
                        // A row laid out again after a skip still has the
                        // rect it last had on screen, where other rows are
                        // now. The bubble registers its click targets from
                        // it, so drop it rather than let it take their clicks.
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
                        // One scope per row so its Ui id can carry the row's
                        // accessible node. A scope adds no spacing of its own:
                        // the row's items keep the spacing they had in the
                        // parent, and the parent adds the usual gap after it.
                        let notices_before = placed;
                        // Its node is an item of the list's node, not of the
                        // scroll area's containers in between, and keeps its
                        // id while rows above it come and go from the layout.
                        let current = cursor == Some(index);
                        // A row laid out only because a jump or the divider
                        // lays out every row, far from the view, is laid out
                        // hidden: it is off screen whatever this frame
                        // scrolls to, and its widgets' accessible nodes,
                        // thousands in a long chat, would only slow this
                        // frame and the screen reader taking it in. The row
                        // jumped to, the divider's and the current one stay
                        // shown, so the list can name the current one. Selected
                        // rows stay visible to egui so it retains off-screen
                        // endpoints and the text between them.
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
                                        || previous.is_none_or(|previous| {
                                            previous.sender != message.sender || previous.from_me
                                        }));
                                // The first message of a run from one side, as the
                                // phone draws it: a little apart, with a tail.
                                let first_in_run = new_day
                                    || previous.is_none_or(|previous| !same_run(previous, message));
                                // Messages curls its tail under a run's last bubble.
                                // The sender's picture: beside the first message of a
                                // run in WhatsApp's style, beside the last in Messages.
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
                                // While selecting, every row gains a check box in a
                                // column on the left and a band behind it, as in
                                // WhatsApp Web, painted under the row once its height
                                // is known.
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
                                        // Behind the row, across the whole message
                                        // view, like WhatsApp's.
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
                                    // While selecting, a click anywhere on the row picks
                                    // the message: its check box, text, links and media,
                                    // and the strip beside it, from one edge of the view
                                    // to the other (#241). Registered after the row, so
                                    // it takes those clicks, and drags, which sweep
                                    // messages rather than text (#246).
                                    // Half the gap to each neighbour, so the space
                                    // between rows picks one of them too.
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
                                    // Beside the bubble as drawn this frame: the
                                    // response carries the rect it had the frame before.
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
                                    // The list's focus ring goes around its
                                    // current message, the bubble only, as a
                                    // focused bubble's own ring does.
                                    if list_focused {
                                        theme::focus_outline(ui, list_id, response.rect, 10.0);
                                    }
                                    if reveal {
                                        // As `theme::reveal_focus`: at once, with
                                        // room for the ring.
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
                        // Built only while a screen reader is attached; the
                        // probe returns None when AccessKit is off, so idle
                        // frames pay nothing for the label.
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
                            // An album row reads as its bubble draws it: the
                            // items, every member's quote and reactions, and the
                            // footer of its last member.
                            let members = album.unwrap_or(std::slice::from_ref(message));
                            let last = members.last().unwrap_or(message);
                            // A call reads the title its bubble draws, which
                            // depends on who placed it.
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
                            // The ticks the bubble shows: a read receipt hidden
                            // while ours are off reads as delivered, as drawn.
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
                            // Whether the file is there to open, still coming, or why not;
                            // in an album, the first item that is not there yet.
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
                        // Laid out this pass, so its node is in this pass's
                        // tree and may be the list's active descendant.
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
                    // A pin made after the newest message keeps its notice at
                    // the end of the transcript.
                    for notice in &notices[placed..] {
                        pin_notice(ui, &palette, view.locale, view.names_or, notice);
                    }
                    if !typing.is_empty() {
                        typing_bubble(ui, &view, &typing);
                    }
                    ui.add_space(4.0);
                    // An explicit jump (Ctrl+End, or the return-to-bottom
                    // button) wins over a message bubble's retained keyboard
                    // focus; other triggers still defer to it, so an
                    // automatic pin does not pull the view away from what a
                    // keyboard-navigating reader is looking at.
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
            // A keyboard PgUp/PgDn/Home/End scroll moves by the next slice of
            // its eased distance each frame, as an instant scroll: relative
            // steps compose with the row-height compensation below, and
            // `scroll_with_delta` releases stick-to-bottom like edge-scroll
            // above. An instant jump to the end this frame wins over it.
            if pinned {
                key_scroll = None;
            } else if let Some(kind) = pending_scroll
                && view.reaction.is_none()
            {
                // A repeated page key adds a page to the distance left, so a
                // held key keeps moving; anything else starts over.
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
                    // Measured again every frame, so messages that arrive
                    // on the way are included.
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
    // Keep the view at the end while initial content expands, until the user
    // scrolls with the wheel, trackpad, or scrollbar.
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
    // Keep the rows on screen where they were when rows above them changed
    // height, unless this frame scrolled on purpose (to the end, a jump, or
    // the divider) or sticks to the end, where egui keeps the offset anyway.
    // Home targets an absolute offset (0), so it is skipped while Home
    // scrolls, rather than pushing the offset away from the top; PgUp/PgDn
    // steps are relative and still get it. The pass is redone at the new
    // offset, so the shift never shows.
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
    // A jump to the end moves the offset only after the rows were painted at
    // the old one. On a chat's first showing that is the top of its first
    // page, which would flash for a frame before the end; redo the pass at
    // the end instead.
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
        // The message the reader jumped to wins over the unread divider,
        // which would otherwise scroll away from it on the next frame.
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
        // Keep the anchor when the first page is still loading. Event::Messages
        // will request it again.
        app.scroll_anchor = None;
    }
    // egui's focus, not the response's: that one also asks for the window's
    // focus, and the list keeps its place while the window is away.
    app.messages_focused = ui.memory(|memory| memory.has_focus(list_id));
    // A current message too far away to be laid out is reached as a jump is:
    // the anchor lays out every row and centres it. Its node exists only
    // from then on, so it becomes the active descendant then, not now. The
    // pass is redone at once for it: a frame whose focused list names no
    // active descendant has a screen reader announce the list itself, then
    // the message.
    if reveal
        && !cursor_drawn
        && app.scroll_anchor.is_none()
        && let Some(id) = cursor_id
    {
        app.scroll_anchor = Some(id);
        ui.ctx()
            .request_discard("lay out the current message far from the view");
    }
    // At the top, load more from the archive and then the phone. Short chats
    // request more immediately.
    let fits = output.content_size.y <= output.inner_rect.height() + 1.0;
    let near_top = output.state.offset.y < 80.0;
    // Only a transcript the reader scrolled up through is their ask; a short
    // chat, or one still on its way to the end, is not.
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
        // Scrolling up releases stick-to-bottom.
        app.scroll_to_bottom = false;
    }
    // Show a return-to-bottom button while reading older messages.
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
        // It floats over the messages: a soft shadow lifts it off them.
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

/// The disc behind a message's react or reply button: opaque, so the chat's
/// wallpaper does not show through, and lifted like a bubble.
fn floating_circle(ui: &egui::Ui, palette: &Palette, rect: Rect, hovered: bool) {
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
fn top_of_history(
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
fn typing_bubble(ui: &mut egui::Ui, view: &View<'_>, typers: &[(String, String, ChatActivity)]) {
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

fn typing_dots(ui: &mut egui::Ui, palette: &Palette) {
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
    // Every frame, paced by vsync like egui's own animations; drawn only
    // while visible, and a hidden window gets no frames at all. Stepped
    // while a screen reader is attached: see `motion`.
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

const REACTION_AFFORDANCE_SIZE: f32 = 26.0;
const REACTION_AFFORDANCE_GAP: f32 = 6.0;

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
fn reaction_affordance_visible(pointer: Option<egui::Pos2>, bubble: Rect, button: Rect) -> bool {
    pointer.is_some_and(|pointer| bubble.union(button).contains(pointer))
}

/// Opens the reaction picker beside the message, as its hover control and
/// the message list's E (`from_list`) do.
pub(crate) fn open_reaction_picker_action(chat: &str, message: &str, from_list: bool) -> Action {
    Action::OpenReactionPicker {
        chat: chat.to_owned(),
        message: message.to_owned(),
        beside_menu: false,
        from_list,
    }
}

/// Draws the Smile, Reply and Forward controls beside a hovered message; the
/// Smile opens the existing picker.
/// These hover controls are for the pointer only. They use `Sense::CLICK`, which
/// is not focusable and adds no accessibility node; the message menu and the
/// message keys give keyboard and screen-reader users the same actions.
fn reaction_affordance(
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
    // Only the part of the bubble on screen can be hovered. Anchoring to it
    // keeps a bubble scrolled mostly out of view from parking its controls,
    // and catching the pointer, at the edge of the transcript.
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
    // Stay hidden under any floating layer, as the context menu does. The
    // affordance sits beside the bubble, so test the bubble itself rather than
    // the pointer, which may already be over the button, outside the bubble's
    // layer.
    let uncovered = ui
        .ctx()
        .layer_id_at(bubble.rect.center())
        .is_none_or(|layer| layer == bubble.layer_id);

    // Publish where the control actually landed, the same way the bubble rect is
    // published, so tests can act on the real rect instead of guessing it.
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
    // Forward takes the next free slot outwards: past reply where the chat can
    // reply, and reply's own place where it cannot, so every message the
    // forward dialog accepts offers the control.
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
    // In an album the control acts on the item under the pointer, as the
    // context menu does. Remember that item: by the time the click lands, the
    // pointer has already left it for the control. It is dropped over anything
    // else in the bubble, such as a caption, and once the controls are hidden,
    // so Forward falls back to the message the row draws instead of forwarding
    // an item from an earlier hover.
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
            // An album forwards the item the control is beside, not always
            // its first member.
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

/// Where a message sits in its sender's run: whether it names the sender,
/// shows their picture, and carries the bubble's tail.
#[derive(Clone, Copy)]
struct RunPlace {
    show_sender: bool,
    show_avatar: bool,
    tailed: bool,
}

/// Draws a message row and returns its bubble response for scrolling.
fn bubble(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    album: Option<&[Message]>,
    place: RunPlace,
    actions: &mut Vec<Action>,
) -> Option<egui::Response> {
    let RunPlace {
        show_sender,
        show_avatar,
        tailed,
    } = place;
    let own = message.from_me;
    // Greys that read on the panel can vanish on the bubble; use its own.
    let view = &View {
        palette: view.palette.on_bubble(own),
        ..*view
    };
    let with_avatar = !own && shows_sender_pictures(view.chat);
    let carousel = matches!(&message.content, Content::Interactive { card: Some(card), .. } if !card.carousel.is_empty());
    let share = if carousel {
        0.95
    } else if view.palette.bubbles == theme::BubbleStyle::Messages {
        MESSAGES_WIDTH_SHARE
    } else {
        WHATSAPP_WIDTH_SHARE
    };
    let max_width = ((ui.available_width() * share).min(if carousel { 920.0 } else { 560.0 })
        - if with_avatar {
            SENDER_AVATAR + 8.0
        } else {
            0.0
        })
    .max(0.0);
    // Register the empty strip beside the bubble from its previous rect, before
    // the row, so the avatar, the bubble, and the reactions win clicks, and the
    // text wins drags. A drag that starts here, off the text, sweeps messages
    // into a selection (#246).
    let id = bubble_id(&view.chat.id, &message.id);
    let previous = ui.ctx().data(|data| data.get_temp::<Rect>(id.with("rect")));
    if let Some(rect) = previous {
        let strip = Rect::from_x_y_ranges(ui.max_rect().x_range(), rect.y_range());
        let strip = ui.interact(strip, id.with("row"), Sense::click_and_drag());
        if strip.clicked() {
            actions.push(Action::FocusComposer);
        }
        if strip.drag_started() {
            actions.push(Action::SweepMessages {
                anchor: message.id.clone(),
                to: message.id.clone(),
            });
        }
        reply_on_double_click(&strip, message, actions);
    }
    let mut response = None;
    ui.with_layout(
        Layout::top_down(if own { Align::Max } else { Align::Min }),
        |ui| {
            if with_avatar {
                ui.horizontal_top(|ui| {
                    let (space, _) =
                        ui.allocate_exact_size(Vec2::splat(SENDER_AVATAR), Sense::hover());
                    // Keep bubble content vertically laid out inside the row.
                    ui.vertical(|ui| {
                        response = Some(bubble_frame(
                            ui,
                            view,
                            message,
                            album,
                            show_sender,
                            tailed,
                            max_width,
                            actions,
                        ));
                    });
                    if !show_avatar {
                        return;
                    }
                    // Messages puts the avatar beside the run's last message,
                    // level with its bottom; WhatsApp beside the first, at
                    // its top.
                    let rect = match &response {
                        Some(bubble) if view.palette.bubbles == theme::BubbleStyle::Messages => {
                            Rect::from_min_size(
                                pos2(
                                    space.left(),
                                    (bubble.rect.bottom() - SENDER_AVATAR).max(space.top()),
                                ),
                                Vec2::splat(SENDER_AVATAR),
                            )
                        }
                        _ => space,
                    };
                    let avatar = ui.interact(rect, id.with("avatar"), Sense::CLICK);
                    theme::focus_outline(ui, avatar.id, rect, SENDER_AVATAR / 2.0);
                    if avatar
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        actions.push(sender_info_action(message));
                    }
                    if ui.is_rect_visible(rect) {
                        let name = (view.names_or)(&message.sender, message.sender_name.as_deref());
                        widgets::paint_avatar(
                            ui,
                            &view.palette,
                            rect,
                            name.trim_start_matches('~'),
                            &message.sender,
                            view.avatars
                                .get(&message.sender)
                                .and_then(|picture| picture.as_deref()),
                        );
                    }
                });
            } else {
                response = Some(bubble_frame(
                    ui,
                    view,
                    message,
                    album,
                    show_sender,
                    tailed,
                    max_width,
                    actions,
                ));
            }
            // An album row carries every member's reactions, each set
            // reacting to its own item.
            let members = album.unwrap_or(std::slice::from_ref(message));
            if members.iter().any(|member| !member.reactions.is_empty()) {
                ui.add_space(-7.0);
                ui.horizontal(|ui| {
                    if with_avatar {
                        ui.add_space(SENDER_AVATAR + 8.0);
                    }
                    for member in members.iter().filter(|member| !member.reactions.is_empty()) {
                        reactions(ui, view, member, actions);
                    }
                });
                ui.add_space(2.0);
            }
        },
    );
    response
}

/// Clamps message-selection drags to the view while the pointer is outside it.
/// This keeps a row under the pointer during edge scrolling. The input hook
/// adjusts positions before egui processes them, using the previous frame's
/// view rectangle.
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
        // The chat list's resize handle reaches into the view from its left
        // edge, as the scroll bar does on the right. Leashing a drag of the
        // handle pinned the pointer, and the list's edge, just inside the
        // view: stuck at its widest, or growing while dragged left (#239).
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

fn clamp_into(pos: egui::Pos2, view: Rect) -> egui::Pos2 {
    egui::pos2(
        pos.x.clamp(view.left() + 2.0, view.right() - 18.0),
        pos.y.clamp(view.top() + 2.0, view.bottom() - 2.0),
    )
}

/// Builds the transcript row used when copying across messages.
fn transcript_row(
    view: &View<'_>,
    message: &Message,
    body: String,
    placements: Vec<String>,
) -> crate::transcript::Row {
    message_copy_row(message, body, placements, view.names_or, view.mention_names)
}

/// Copies whole selected messages, including rows outside the viewport, in
/// conversation order rather than the order in which they were selected.
pub(super) fn selected_text(app: &App) -> Option<String> {
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
fn message_copy_row(
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

/// Whether a chat's header offers to call it: a person's chat calls them, a
/// group's starts a call in the group while group calls are on
/// (`calls::GROUP_CALLS_ENABLED`). Never the chat with ourselves, a channel or
/// a broadcast list.
pub(crate) fn callable(chat: &crate::model::Chat, me: Option<&str>) -> bool {
    match chat.kind {
        crate::model::ChatKind::Direct => me != Some(chat.id.as_str()),
        crate::model::ChatKind::Group => crate::calls::group_calls_enabled(),
        _ => false,
    }
}

/// A call's menu: calling back, in a chat that can take a call. A group's
/// call is called back in its group while group calls are on; a call among
/// several people with no group behind it cannot be started here.
fn call_menu(
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
            // Opened from the keys, the menu's one row takes focus, unavailable,
            // so the reader hears why there is nothing to choose.
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
    // The item a menu opened from the keyboard focuses, as the message
    // menu's Reply; only a pass that draws the menu names it.
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
fn reply_on_double_click(response: &egui::Response, message: &Message, actions: &mut Vec<Action>) {
    if response.double_clicked()
        && !matches!(message.content, Content::Revoked | Content::Call { .. })
    {
        actions.push(Action::Reply(message.id.clone()));
    }
}

/// What an album row holds, in words: its photos and videos counted, then
/// the captions drawn under them.
fn album_label(view: &View<'_>, members: &[Message]) -> String {
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

/// The id salt of a message row's scope; the scope `Ui`'s unique id is the
/// row's accessibility node.
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

/// Where a speed choice in a voice message's menu was drawn, for
/// interaction tests.
pub fn speed_button_id(chat: &str, message: &str, speed: f32) -> egui::Id {
    egui::Id::new(("speed", chat, message, speed.to_bits()))
}

/// Draws a playback speed pill labelled with `speed`, highlighted when
/// `active` and faded while that speed is still `preparing`.
fn speed_pill(
    ui: &mut egui::Ui,
    view: &View<'_>,
    size: Vec2,
    speed: f32,
    active: bool,
    preparing: bool,
) -> egui::Response {
    let palette = view.palette;
    let label = crate::audio::speed_label(speed);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            ui.is_enabled(),
            active,
            format!("Playback speed {label}"),
        )
    });
    theme::reveal_focus(&response);
    theme::focus_outline(ui, response.id, rect, rect.height() / 2.0);
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        // The resting fill uses the hover step because incoming bubbles
        // share the resting surface colour.
        let fill = if active {
            palette
                .accent
                .gamma_multiply(if hovered { 0.42 } else { 0.30 })
        } else if hovered {
            palette.surface_active
        } else {
            palette.surface_hover
        };
        ui.painter().rect_filled(rect, rect.height() / 2.0, fill);
        let colour = if active {
            palette.accent
        } else {
            palette.secondary
        };
        let colour = if preparing {
            colour.gamma_multiply(0.5)
        } else {
            colour
        };
        let galley = ui
            .painter()
            .layout_no_wrap(label, theme::medium(11.0), colour);
        ui.painter()
            .galley(rect.center() - galley.size() / 2.0, galley, colour);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The message menu's row of every playback speed for a playable voice
/// message, so 1.25x and 1.75x are reachable without cycling.
/// Choosing `speed` for a voice message of `chat`: a chat with its own speed
/// changes that, and any other changes the speed in Settings.
fn speed_action(chat: &Chat, speed: f32) -> Action {
    if chat.voice_speed.is_some() {
        Action::SetChatVoiceSpeed {
            chat: chat.id.clone(),
            speed: Some(speed),
        }
    } else {
        Action::SetVoiceSpeed(speed)
    }
}

fn speed_menu_row(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    actions: &mut Vec<Action>,
) {
    let speed = view.voice_speed;
    let preparing = view.player.preparing_speed(&message.id);
    widgets::menu_separator(ui, &view.palette);
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 28.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add_space(6.0);
            for option in crate::audio::SPEEDS {
                let selected = option == speed;
                let response = speed_pill(
                    ui,
                    view,
                    vec2(44.0, 24.0),
                    option,
                    selected,
                    preparing && selected,
                );
                ui.ctx().data_mut(|data| {
                    data.insert_temp(
                        speed_button_id(&view.chat.id, &message.id, option),
                        response.rect,
                    );
                });
                if response.clicked() {
                    actions.push(speed_action(view.chat, option));
                    ui.close();
                }
            }
        },
    );
}

/// Whether `message` draws without a bubble around it: stickers, round video
/// messages, carousels, and in the Messages style a lone picture.
fn bubbleless(palette: &Palette, message: &Message) -> bool {
    matches!(
        &message.content,
        Content::Sticker { .. } | Content::Video { note: true, .. }
    ) || matches!(&message.content, Content::Interactive { card: Some(card), .. } if !card.carousel.is_empty())
        || bare_picture(palette, message)
}

/// Whether `messages[index]` ends its run: the next message is from the other
/// side, another sender, or another day.
fn last_of_run(messages: &[Message], index: usize) -> bool {
    let message = &messages[index];
    messages.get(index + 1).is_none_or(|next| {
        !same_run(message, next)
            || crate::util::day_key(message.timestamp) != crate::util::day_key(next.timestamp)
    })
}

/// Whether `messages[index]` is the last bubble of its run, which Messages
/// tails: messages drawn without one after it, a lone photo or a sticker,
/// leave the tail to it.
fn last_bubble_of_run(palette: &Palette, messages: &[Message], index: usize) -> bool {
    let message = &messages[index];
    !messages[index + 1..]
        .iter()
        .take_while(|next| {
            same_run(message, next)
                && crate::util::day_key(message.timestamp) == crate::util::day_key(next.timestamp)
        })
        .any(|next| !bubbleless(palette, next))
}

/// Whether `next` continues the run `message` belongs to: the same side,
/// and in a group the same sender.
fn same_run(message: &Message, next: &Message) -> bool {
    message.from_me == next.from_me && (message.from_me || message.sender == next.sender)
}

/// Room around a bubble's contents: none around a picture that stands
/// without one, Messages' insets in its style, WhatsApp's otherwise.
fn bubble_margin(palette: &Palette, message: &Message, over_picture: bool) -> Margin {
    if bare_picture(palette, message) {
        Margin::ZERO
    } else if palette.bubbles == theme::BubbleStyle::Messages {
        MESSAGES_BUBBLE_MARGIN
    } else {
        Margin {
            left: 10,
            right: 10,
            top: 6,
            bottom: if over_picture { 6 } else { 5 },
        }
    }
}

/// Draws a message bubble and its menu.
#[allow(clippy::too_many_arguments)]
fn bubble_frame(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    album: Option<&[Message]>,
    show_sender: bool,
    tailed: bool,
    max_width: f32,
    actions: &mut Vec<Action>,
) -> egui::Response {
    let palette = view.palette;
    let own = message.from_me;
    let fill = if bubbleless(&palette, message) {
        Color32::TRANSPARENT
    } else if own {
        palette.bubble_out
    } else {
        palette.bubble_in
    };
    // Register the bubble from its previous rect before its contents so inner
    // links, quotes, and attachments win clicks. The bubble handles right-click.
    let bubble_id = bubble_id(&view.chat.id, &message.id);
    // Store the final rect separately. Reusing the early response would keep
    // the first frame's rect.
    let rect_id = bubble_id.with("rect");
    let previous = ui.ctx().data(|data| data.get_temp::<Rect>(rect_id));
    let early = previous.map(|rect| ui.interact(rect, bubble_id, Sense::CLICK));
    // Painted once the contents are measured, beneath them.
    let backdrop = ui.painter().add(egui::Shape::Noop);
    let tail = (tailed && fill != Color32::TRANSPARENT).then_some(if own {
        widgets::Side::Right
    } else {
        widgets::Side::Left
    });
    // A picture without a caption carries its time over its corner, so
    // the bubble closes under it as evenly as it opens above it.
    let over_picture = album.is_none() && time_over_picture(message);
    let mut tiles: Vec<(Rect, String)> = Vec::new();
    let inner = Frame::new()
        .inner_margin(bubble_margin(&palette, message, over_picture))
        .show(ui, |ui| {
            ui.set_max_width(max_width);
            ui.spacing_mut().item_spacing.y = 4.0;
            if show_sender && view.chat.is_group() {
                let name = (view.names_or)(&message.sender, message.sender_name.as_deref());
                let response = widgets::rich_text(
                    ui,
                    &name,
                    theme::semibold(13.0),
                    palette.sender(crate::util::hue(&message.sender)),
                );
                let response = ui
                    .interact(
                        response.rect,
                        ui.id().with(("sender", &message.id)),
                        Sense::CLICK,
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                ui.ctx()
                    .data_mut(|data| data.insert_temp(bubble_id.with("sender"), response.rect));
                if response.clicked() {
                    actions.push(sender_info_action(message));
                }
            } else {
                // A name from an earlier frame must not trim the text's
                // click target below.
                ui.ctx()
                    .data_mut(|data| data.remove::<Rect>(bubble_id.with("sender")));
            }
            // Reserve the label's line now and paint it once the contents
            // are measured, at their start: an own bubble lays out from the
            // right, and its width is known only then.
            let forwarded = message.forwarded.then(|| forwarded_label(ui, &palette));
            // Cards share the bubble's settled width: at least CARD_WIDTH and
            // no more than the cap. Text spans that width and stays left-aligned.
            // Bubbles without cards use the natural text width.
            let inset = if palette.bubbles == theme::BubbleStyle::Messages
                && !bare_picture(&palette, message)
            {
                MESSAGES_BUBBLE_MARGIN.sum().x
            } else {
                20.0
            };
            let cap = ((max_width - inset).min(ui.available_width())).max(0.0);
            let last = album
                .and_then(|messages| messages.last())
                .unwrap_or(message);
            // An album's footer is its last member's, marks included.
            let starred = view.starred.is_some_and(|ids| ids.contains(&last.id));
            let pinned = view.pins.is_some_and(|ids| ids.contains(&last.id));
            let reserve = footer_width(ui, last, starred, pinned);
            let settled = album
                .map(|_| CARD_WIDTH.min(cap))
                .or_else(|| settled_width(ui, view, message, cap));
            let slot = if let Some(messages) = album {
                let mut quoted_targets = HashMap::new();
                let mut quotes: Vec<(&Message, &crate::model::Quoted)> = Vec::new();
                for member in messages {
                    if let Some(quoted) = &member.quoted {
                        // WhatsApp can repeat the same reply on every album
                        // item. Show each target once, keeping distinct replies.
                        let target = (
                            quoted.chat.as_deref().unwrap_or(&view.chat.id),
                            quoted.id.as_str(),
                            quoted.sender.as_str(),
                        );
                        let index = *quoted_targets.entry(target).or_insert_with(|| {
                            quotes.push((member, quoted));
                            quotes.len() - 1
                        });
                        // Keep targets in their first-seen order, but let a
                        // later copy supply text missing from the first one.
                        if quotes[index].1.summary.trim().is_empty()
                            && !quoted.summary.trim().is_empty()
                        {
                            quotes[index] = (member, quoted);
                        }
                    }
                }
                for (member, quoted) in quotes {
                    quote_block(ui, view, member, quoted, settled.unwrap_or(cap), actions);
                }
                tiles = album_content(ui, view, messages, settled.unwrap_or(cap), actions);
                None
            } else {
                match settled {
                    Some(width) => {
                        if let Some(quoted) = &message.quoted {
                            quote_block(ui, view, message, quoted, width, actions);
                        }
                        content(ui, view, message, width, reserve, actions)
                    }
                    None => content(ui, view, message, cap, reserve, actions),
                }
            };
            if over_picture && let Some(picture) = slot {
                footer_over_picture(ui, view, message, picture, starred, pinned, actions);
            } else {
                footer(ui, view, last, slot, starred, pinned, actions);
            }
            if matches!(message.content, Content::Poll { .. }) {
                super::polls::results_button(
                    ui,
                    &palette,
                    view.locale,
                    message,
                    settled.unwrap_or(cap),
                    actions,
                );
            }
            if let Content::Interactive {
                card: Some(card), ..
            } = &message.content
            {
                interactive_buttons(ui, view, message, card, settled.unwrap_or(cap), actions);
            }
            if let Some((reserved, galley)) = forwarded {
                let at = pos2(ui.min_rect().left(), reserved.top());
                let painted = paint_forwarded_label(ui, &palette, at, galley);
                ui.ctx().data_mut(|data| {
                    data.insert_temp(bubble_id.with("forwarded"), painted);
                });
            }
        });
    if fill != Color32::TRANSPARENT && ui.is_rect_visible(inner.response.rect) {
        ui.painter().set(
            backdrop,
            widgets::bubble_shape(
                &palette,
                inner.response.rect,
                fill,
                tail,
                ui.ctx().pixels_per_point(),
                // The transcript's viewport: own balloons shade down it.
                ui.clip_rect().y_range(),
            ),
        );
    }
    ui.ctx()
        .data_mut(|data| data.insert_temp(rect_id, inner.response.rect));
    let bubble = early.unwrap_or_else(|| ui.interact(inner.response.rect, bubble_id, Sense::CLICK));
    if view.selecting {
        return bubble;
    }
    theme::reveal_focus(&bubble);
    theme::focus_outline(ui, bubble.id, inner.response.rect, 10.0);
    if ui.ctx().data(|data| {
        data.get_temp::<bool>(theme::keyboard_focus_id())
            .unwrap_or(false)
    }) && let Some(focused) = ui
        .memory(|memory| memory.focused())
        .and_then(|id| ui.ctx().read_response(id))
        && (focused.id == bubble.id || inner.response.rect.contains(focused.rect.center()))
    {
        view.keyboard_navigation.set(true);
        if focused.gained_focus() && !ui.clip_rect().contains_rect(focused.rect) {
            ui.scroll_to_rect_animation(
                inner.response.rect.expand(4.0),
                None,
                egui::style::ScrollAnimation::none(),
            );
        }
    }
    // The context menu stays open beside the picker only when the picker came
    // from the menu itself.
    let reacting = view.reaction_menu && view.reaction == Some(message.id.as_str());
    // Inner widgets own their clicks, so this fires only on the bubble's padding
    // and footer. Double-click on the body keeps selecting the word.
    reply_on_double_click(&bubble, message, actions);

    reaction_affordance(ui, view, message, &bubble, &tiles, actions);
    // Read right-click from input because inner widgets own their responses.
    // The whole row counts, the empty strip beside the bubble included, as in
    // other messaging apps (#240). Count only the part inside the transcript's
    // viewport: the chat header shares this layer, and a bubble scrolled under
    // it is hidden there. Open only when no floating layer covers the chat
    // panel.
    let viewport = ui.clip_rect();
    let shown =
        Rect::from_x_y_ranges(viewport.x_range(), bubble.rect.y_range()).intersect(viewport);
    let right_clicked = ui.input(|input| {
        input.pointer.secondary_clicked()
            && input
                .pointer
                .interact_pos()
                .is_some_and(|pos| shown.contains(pos))
    }) && ui
        .input(|input| input.pointer.interact_pos())
        .is_some_and(|pos| {
            ui.ctx()
                .layer_id_at(pos)
                .is_none_or(|layer| layer == bubble.layer_id)
        });
    // In an album the menu belongs to the member under the pointer, so a
    // reply, reaction, forward or deletion reaches that item, not the first.
    let target_id = bubble_id.with("album-target");
    if album.is_some() && right_clicked {
        let pointer = ui.input(|input| input.pointer.interact_pos());
        let hit = tiles
            .iter()
            .find(|(rect, _)| pointer.is_some_and(|pos| rect.contains(pos)))
            .map(|(_, id)| id.clone());
        ui.ctx().data_mut(|data| {
            if let Some(id) = hit {
                data.insert_temp(target_id, id);
            } else {
                data.remove::<String>(target_id);
            }
        });
    }
    if let Some(members) = album
        && let Some(opened) = view.open_menu
        && members.iter().any(|member| member.id == opened)
    {
        ui.ctx()
            .data_mut(|data| data.insert_temp(target_id, opened.to_owned()));
    }
    let menu_target = album
        .and_then(|members| {
            let id = ui.ctx().data(|data| data.get_temp::<String>(target_id))?;
            members.iter().find(|member| member.id == id)
        })
        .unwrap_or(message);
    let force_menu = match album {
        Some(members) => view
            .open_menu
            .is_some_and(|opened| members.iter().any(|member| member.id == opened)),
        None => view.open_menu == Some(message.id.as_str()),
    };
    let reacting = reacting
        || (album.is_some()
            && view.reaction_menu
            && view.reaction == Some(menu_target.id.as_str()));
    let width = menu_width(ui, view.locale, view.reaction_emoji, menu_target);
    let keyboard_clicked =
        bubble.clicked() && bubble.has_focus() && !ui.input(|input| input.pointer.any_click());
    let popup_id = bubble_id.with("popup");
    let was_open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    // A menu opened without the pointer stays beside its message for as
    // long as it is open, not only in the frame that opened it: the
    // pointer-anchored placement has no position to go to.
    let at_message_id = bubble_id.with("menu-at-message");
    let at_message = force_menu
        || keyboard_clicked
        || (was_open
            && !right_clicked
            && ui
                .ctx()
                .data(|data| data.get_temp::<bool>(at_message_id))
                .unwrap_or(false));
    // Opened by the message list's keys: focus goes into the menu, stays in
    // it, and returns to the list when it closes. True while the first item
    // is still to be focused.
    let from_keys_id = bubble_id.with("menu-from-keys");
    if force_menu {
        // Opened once: from here on the menu closes as any menu does.
        actions.push(Action::MessageMenuOpened);
        let keyboard = ui.ctx().data(|data| {
            data.get_temp::<bool>(theme::keyboard_focus_id())
                .unwrap_or(false)
        });
        if keyboard {
            ui.ctx()
                .data_mut(|data| data.insert_temp(from_keys_id, true));
        }
    }
    let from_keys = ui.ctx().data(|data| data.get_temp::<bool>(from_keys_id));
    // Whether keyboard focus is on one of the menu's items (true), on a
    // widget elsewhere (false), or on nothing drawn (None, as between two
    // items while Tab moves on).
    let focus_in_menu = |ctx: &egui::Context| {
        ctx.memory(|memory| memory.focused())
            .and_then(|id| ctx.read_response(id))
            .map(|response| response.layer_id.id == popup_id)
    };
    if from_keys == Some(false) && was_open && !reacting && focus_in_menu(ui.ctx()) == Some(true) {
        // Up and Down walk the items in order and wrap, as Tab does, as in
        // any desktop menu. egui's own arrow moves go to the nearest widget
        // on screen, which can be one under the menu.
        let step = ui.ctx().input_mut(|input| {
            if super::keys::take_plain(input, egui::Key::ArrowDown) {
                Some(egui::FocusDirection::Next)
            } else if super::keys::take_plain(input, egui::Key::ArrowUp) {
                Some(egui::FocusDirection::Previous)
            } else {
                None
            }
        });
        if let Some(step) = step {
            ui.memory_mut(|memory| memory.move_focus(step));
        }
    }
    // Opened from the keys, the menu also lists what only a click reaches
    // in the bubble (`inner_entries`), and is wide enough for it. A menu
    // opened with the pointer is as it was.
    let keys_menu = from_keys.is_some();
    ui.ctx()
        .data_mut(|data| data.insert_temp(keys_menu_id(popup_id), keys_menu));
    let width = if keys_menu {
        let labels: Vec<String> = inner_entries(view, menu_target)
            .into_iter()
            .map(|entry| entry.label)
            .collect();
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        width.max(widgets::menu_width(ui, &labels, true))
    } else {
        width
    };
    let open = if right_clicked || force_menu || reacting || keyboard_clicked {
        Some(egui::SetOpenCommand::Bool(true))
    } else if bubble.clicked() {
        Some(egui::SetOpenCommand::Bool(false))
    } else {
        None
    };
    // egui gives a popup's background a focusable sense, a nameless Tab
    // stop; it only needs the clicks.
    let popup = egui::Popup::menu(&bubble)
        .sense(Sense::CLICK)
        .open_memory(open)
        .close_behavior(if reacting {
            egui::PopupCloseBehavior::IgnoreClicks
        } else {
            egui::PopupCloseBehavior::CloseOnClickOutside
        })
        .width(width)
        .frame(widgets::menu_frame(&palette));
    let popup = if reacting {
        // Keep the menu next to the picker, anchored to this message rather
        // than whichever pointer position happened to open the emoji grid.
        let screen = ui.ctx().content_rect();
        let menu = ui
            .ctx()
            .data(|data| data.get_temp::<Rect>(bubble_id.with("menu-rect")))
            .unwrap_or(bubble.rect);
        let x = menu.left().clamp(
            screen.left() + 8.0,
            (screen.right() - width - 468.0).max(screen.left() + 8.0),
        );
        popup.at_position(pos2(x, menu.top()))
    } else if at_message {
        popup.at_position(bubble.rect.left_top() + vec2(12.0, 8.0))
    } else {
        popup.at_pointer_fixed()
    };
    let menu = popup.show(|ui| {
        context_menu(ui, view, menu_target, MenuOrigin::Chat, actions);
    });
    if let Some(menu) = &menu {
        ui.ctx()
            .data_mut(|data| data.insert_temp(bubble_id.with("menu-rect"), menu.response.rect));
    }
    let mut open_now = egui::Popup::is_id_open(ui.ctx(), popup_id);
    if let Some(focus_pending) = from_keys {
        if open_now && !focus_pending && !reacting && focus_in_menu(ui.ctx()) == Some(false) {
            // A shortcut (Ctrl+L, Ctrl+K, Alt+2) took focus out of the menu:
            // it closes, as a desktop menu does when focus leaves, rather
            // than staying open unseen and holding on to the arrows, Tab and
            // the next Escape. Focus stays where the shortcut put it.
            egui::Popup::close_id(ui.ctx(), popup_id);
            open_now = false;
            ui.ctx().data_mut(|data| data.remove::<bool>(from_keys_id));
        } else if !open_now {
            ui.ctx().data_mut(|data| data.remove::<bool>(from_keys_id));
            // Closed in this pass by Escape or a chosen item: back to the
            // message list, drawn earlier in this pass. A click elsewhere
            // keeps the focus it gave. A menu that went away unseen, its
            // message not drawn (another chat, Settings), returns nothing:
            // focus has moved on since.
            if was_open
                && !ui.input(|input| input.pointer.any_pressed() || input.pointer.any_click())
            {
                let item = ui.memory(|memory| memory.focused());
                super::overlay::hand_back(ui.ctx(), messages_list_id(&view.chat.id), item);
            }
        } else if let Some(menu) = &menu
            && !reacting
        {
            // Tab stays among the menu's items. Not beside the reaction
            // picker, whose own layer must keep its focus.
            ui.memory_mut(|memory| memory.set_modal_layer(menu.response.layer_id));
            // The first item, drawn in this pass, takes focus once.
            if focus_pending
                && let Some(first) = ui
                    .ctx()
                    .data(|data| {
                        data.get_temp::<Option<egui::Id>>(menu_first_item_id(
                            &view.chat.id,
                            &menu_target.id,
                        ))
                    })
                    .flatten()
            {
                ui.memory_mut(|memory| memory.request_focus(first));
                ui.ctx()
                    .data_mut(|data| data.insert_temp(from_keys_id, false));
            }
        }
    }
    if at_message || was_open {
        ui.ctx().data_mut(|data| {
            if at_message && open_now {
                data.insert_temp(at_message_id, true);
            } else {
                data.remove::<bool>(at_message_id);
            }
        });
    }
    // Keep the target explicit for every menu action, not just the emoji
    // picker. Paint in the message layer so the menu itself remains above it.
    if open_now {
        ui.painter().rect_stroke(
            inner.response.rect.expand(2.0),
            12.0,
            Stroke::new(theme::FOCUS_STROKE_WIDTH, palette.accent),
            egui::StrokeKind::Outside,
        );
    }
    if reacting && !egui::Popup::is_id_open(ui.ctx(), bubble_id.with("popup")) {
        actions.push(Action::ClosePicker);
    }
    // This frame's final rect, for later scrolling, with the bubble's own
    // clicks: the frame alone only senses hover, so a Ctrl-click to select
    // or a click to add to a selection would never register.
    inner.response.union(bubble)
}

/// Minimum shared width for cards inside message bubbles.
const CARD_WIDTH: f32 = 320.0;
const CAROUSEL_CARD_WIDTH: f32 = 280.0;
/// Location cards: a map preview across the top, then the details.
const LOCATION_CARD_WIDTH: f32 = 300.0;
const CAROUSEL_GAP: f32 = 8.0;

/// Returns the shared card width, bounded by [`CARD_WIDTH`] and `cap`.
fn settled_width(ui: &egui::Ui, view: &View<'_>, message: &Message, cap: f32) -> Option<f32> {
    if matches!(
        message.content,
        Content::Location { .. } | Content::LiveLocation { .. }
    ) {
        return Some(LOCATION_CARD_WIDTH.min(cap));
    }
    if let Content::Interactive {
        card: Some(card), ..
    } = &message.content
        && !card.carousel.is_empty()
    {
        // Keep short carousels, their heading and their timestamp together.
        // Wider strips still occupy the cap and scroll horizontally.
        let count = card.carousel.len();
        let cards = count as f32 * (CAROUSEL_CARD_WIDTH + 20.0);
        let gaps = count.saturating_sub(1) as f32 * CAROUSEL_GAP;
        return Some((cards + gaps).min(cap));
    }
    if let Content::Interactive {
        card: Some(card), ..
    } = &message.content
        && card.image.is_some()
    {
        return Some(CARD_WIDTH.min(cap));
    }
    let card = message.quoted.is_some()
        || match &message.content {
            Content::Text { preview, .. } => preview.is_some(),
            Content::Document { .. } | Content::Audio { .. } | Content::Poll { .. } => true,
            Content::Interactive { card, .. } => card.is_some(),
            // Videos without a poster use the file-row layout, except the
            // round ones, which always draw as a circle.
            Content::Video { note, .. } => !note && message.thumbnail.is_none(),
            _ => false,
        };
    card.then(|| {
        let floor = CARD_WIDTH.min(cap).max(0.0);
        natural_text_width(ui, view, message, cap).map_or(floor, |width| width.clamp(floor, cap))
    })
}

/// Widest wrapped text row, including footer space on the last line.
fn natural_text_width(ui: &egui::Ui, view: &View<'_>, message: &Message, cap: f32) -> Option<f32> {
    let palette = view.palette;
    let text = match &message.content {
        Content::Text { text, .. } | Content::Interactive { text, .. } => text,
        Content::Image {
            caption: Some(caption),
            ..
        }
        | Content::Video {
            caption: Some(caption),
            ..
        }
        | Content::Document {
            caption: Some(caption),
            ..
        } => caption,
        _ => return None,
    };
    let style = markup::Style {
        size: BODY_SIZE,
        color: palette.text,
        secondary: palette.secondary,
        link: palette.link,
        mention: palette.accent,
    };
    let laid = markup::layout(ui, text, &mentions_of(view, message), &style, cap);
    let widest = laid
        .galley
        .rows
        .iter()
        .map(|row| row.row.size.x)
        .fold(0.0, f32::max);
    let last = laid.galley.rows.last().map_or(0.0, |row| row.row.size.x);
    let starred = view.starred.is_some_and(|ids| ids.contains(&message.id));
    let pinned = view.pins.is_some_and(|ids| ids.contains(&message.id));
    let reserve = footer_width(ui, message, starred, pinned);
    Some(if last + 8.0 + reserve <= cap {
        widest.max(last + 8.0 + reserve)
    } else {
        widest
    })
}

/// Width of a quote's accent bar.
const QUOTE_BAR: f32 = 4.0;
/// Corner radius of a quote.
const QUOTE_RADIUS: u8 = 6;

/// Draws a stored quote and routes navigation to its source conversation.
/// How a quoted reply is coloured inside its bubble.
#[derive(Clone, Copy, Debug, PartialEq)]
struct QuoteColours {
    fill: Color32,
    bar: Color32,
    name: Color32,
    text: Color32,
}

/// The quote's colours. On your own Messages balloon, iMessage blue, it is
/// all white, as Messages draws what sits on that blue: a bold white name,
/// its text at Apple's secondary 60% white
/// (`CKUITheme.blue_balloonTranslationSecondaryTextColor`), the bar at 60%
/// and the fill a 15% white wash over the bubble. Elsewhere the bar and name
/// take `tint`, the quoted sender's colour, over the window's.
fn quote_colours(palette: &Palette, own: bool, tint: impl FnOnce() -> Color32) -> QuoteColours {
    if own && palette.bubbles == theme::BubbleStyle::Messages {
        return QuoteColours {
            fill: Color32::WHITE.gamma_multiply(0.15),
            bar: Color32::WHITE.gamma_multiply(0.6),
            name: Color32::WHITE,
            text: Color32::WHITE.gamma_multiply(0.6),
        };
    }
    let tint = tint();
    QuoteColours {
        fill: palette.window.gamma_multiply(0.35),
        bar: tint,
        name: tint,
        text: palette.secondary,
    }
}

/// Whether a quote is of one of our own messages.
fn quotes_me(view: &View<'_>, quoted: &crate::model::Quoted) -> bool {
    view.me == Some(quoted.sender.as_str())
}

/// The name a quote's block draws for its sender, and the message label
/// reads: `you` for our own message, otherwise the name `names_or` resolves,
/// which prefers a saved contact name to the one the quote carries.
fn quoted_sender_name(view: &View<'_>, quoted: &crate::model::Quoted, you: &str) -> String {
    if quotes_me(view, quoted) {
        you.to_owned()
    } else {
        (view.names_or)(&quoted.sender, quoted.sender_name.as_deref())
    }
}

fn quote_block(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    quoted: &crate::model::Quoted,
    width: f32,
    actions: &mut Vec<Action>,
) {
    let palette = view.palette;
    let mine = quotes_me(view, quoted);
    let who = quoted_sender_name(view, quoted, "You");
    let summary = markup::plain(
        &quoted.summary_in(view.locale),
        &quote_mentions(view, quoted),
    );
    // As in WhatsApp, the bar and name take the quoted sender's colour, the
    // one their name has in groups, kept readable on this bubble.
    let bubble = if message.from_me {
        palette.bubble_out
    } else {
        palette.bubble_in
    };
    let colours = quote_colours(&palette, message.from_me, || {
        theme::readable_on(
            bubble,
            if mine {
                palette.accent
            } else {
                palette.sender(crate::util::hue(&quoted.sender))
            },
            palette.text,
            3.0,
        )
    });
    let response = Frame::new()
        .fill(colours.fill)
        .corner_radius(CornerRadius::same(QUOTE_RADIUS))
        .inner_margin(Margin {
            left: QUOTE_BAR as i8 + 7,
            right: 10,
            top: 5,
            bottom: 6,
        })
        .show(ui, |ui| {
            // Include frame margins in the settled width. Use a bounded,
            // left-aligned layout because own bubbles inherit right-to-left flow.
            let inner_width = (width - QUOTE_BAR - 17.0).max(0.0);
            ui.allocate_ui_with_layout(
                vec2(inner_width, 0.0),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.set_width(inner_width);
                    ui.spacing_mut().item_spacing.y = 1.0;
                    widgets::rich_text(ui, &who, theme::semibold(12.5), colours.name);
                    widgets::rich_text(ui, &summary, theme::regular(12.5), colours.text);
                },
            );
        })
        .response;
    // The bar runs the quote's full height along its rounded left edge.
    let bar = Rect::from_min_size(response.rect.min, vec2(QUOTE_BAR, response.rect.height()));
    ui.painter().rect_filled(
        bar,
        CornerRadius {
            nw: QUOTE_RADIUS,
            sw: QUOTE_RADIUS,
            ne: 0,
            se: 0,
        },
        colours.bar,
    );
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            bubble_id(&view.chat.id, &message.id).with("quote"),
            response.rect,
        );
    });
    let response = ui
        .interact(
            response.rect,
            ui.id().with(("quote", &message.id, &quoted.id)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        actions.push(quote_jump_action(quoted));
    }
}

/// Diameter of the tinted disc behind a call entry's glyph.
const CALL_BADGE: f32 = 36.0;
/// Size of the glyph inside it.
const CALL_GLYPH: f32 = 18.0;
/// Room between the disc and the words.
const CALL_BADGE_GAP: f32 = 10.0;
/// Room between the title and the line under it.
const CALL_LINE_GAP: f32 = 2.0;
/// Height of the time's line, as [`footer`] draws it.
const FOOTER_HEIGHT: f32 = 15.0;

/// Draws a call entry as WhatsApp does: the glyph in a tinted disc, centred
/// on the title and the detail beside it, and the time at the end of the
/// detail's line when it fits there. Painted at measured positions inside one
/// allocation, so an own bubble's right-to-left layout cannot reorder or
/// stretch it. Returns the time's slot, or `None` when the time needs a line
/// of its own.
fn call_entry(
    ui: &mut egui::Ui,
    palette: &Palette,
    entry: &super::calls::Entry,
    locale: crate::i18n::Locale,
    tint: Color32,
    width: f32,
    reserve: f32,
) -> Option<Rect> {
    let title = entry.title(locale);
    let detail = entry.detail(locale);
    let title_galley =
        ui.painter()
            .layout_no_wrap(title.clone(), theme::medium(14.0), palette.text);
    let detail_galley = detail.as_ref().map(|detail| {
        ui.painter()
            .layout_no_wrap(detail.clone(), theme::regular(12.5), palette.secondary)
    });
    let detail_width = detail_galley.as_ref().map_or(0.0, |galley| galley.size().x);
    // The time follows the detail on its line, 8 points after it as after a
    // text's last line; with no detail it has that line to itself.
    let gap = if detail_galley.is_some() { 8.0 } else { 0.0 };
    let lead = CALL_BADGE + CALL_BADGE_GAP;
    let inline = lead + detail_width + gap + reserve <= width;
    let second_width = if inline {
        detail_width + gap + reserve
    } else {
        detail_width
    };
    let second_height = match (&detail_galley, inline) {
        (Some(galley), true) => galley.size().y.max(FOOTER_HEIGHT),
        (Some(galley), false) => galley.size().y,
        (None, true) => FOOTER_HEIGHT,
        (None, false) => 0.0,
    };
    let block_height = title_galley.size().y
        + if second_height > 0.0 {
            CALL_LINE_GAP + second_height
        } else {
            0.0
        };
    let content_width = lead + title_galley.size().x.max(second_width);
    // Span the bubble when a sender's name above made it wider, so the time
    // keeps to the bubble's end as it does under text.
    let row_width = content_width
        .max(ui.min_rect().width())
        .min(width.max(content_width));
    let (rect, response) = ui.allocate_exact_size(
        vec2(row_width, block_height.max(CALL_BADGE)),
        Sense::hover(),
    );
    response.widget_info(|| {
        let label = match &detail {
            Some(detail) => format!("{title}, {detail}"),
            None => title.clone(),
        };
        egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label)
    });
    let badge = Rect::from_center_size(
        pos2(rect.left() + CALL_BADGE / 2.0, rect.center().y),
        Vec2::splat(CALL_BADGE),
    );
    let text_left = badge.right() + CALL_BADGE_GAP;
    let top = rect.center().y - block_height / 2.0;
    let second_top = top + title_galley.size().y + CALL_LINE_GAP;
    let second_middle = second_top + second_height / 2.0;
    if ui.is_rect_visible(rect) {
        ui.painter()
            .circle_filled(badge.center(), CALL_BADGE / 2.0, tint.gamma_multiply(0.16));
        theme::paint_icon(
            ui,
            entry.icon(),
            Rect::from_center_size(badge.center(), Vec2::splat(CALL_GLYPH)),
            CALL_GLYPH,
            tint,
        );
        ui.painter()
            .galley(pos2(text_left, top), title_galley, palette.text);
        if let Some(galley) = detail_galley {
            let at = pos2(text_left, second_middle - galley.size().y / 2.0);
            ui.painter().galley(at, galley, palette.secondary);
        }
    }
    inline.then(|| {
        Rect::from_min_size(
            pos2(rect.right() - reserve, second_middle - FOOTER_HEIGHT / 2.0),
            vec2(reserve, FOOTER_HEIGHT),
        )
    })
}

/// Keeps row contents left-to-right inside right-aligned own bubbles.
fn mirrored_row(
    ui: &mut egui::Ui,
    own: bool,
    first: impl FnOnce(&mut egui::Ui),
    second: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        if own {
            second(ui);
            first(ui);
        } else {
            first(ui);
            second(ui);
        }
    });
}

/// Size of the forwarded label's arrow.
const FORWARDED_ICON: f32 = 14.0;
/// Gap between the forwarded label's arrow and its word.
const FORWARDED_GAP: f32 = 4.0;
/// Height of the forwarded label's line: its word's height, without the
/// taller line an icon and a label in a row would take.
const FORWARDED_HEIGHT: f32 = 15.0;

/// Reserves the line for a forwarded message's label at the top of its
/// bubble and returns that space with the laid-out word, for
/// [`paint_forwarded_label`] once the bubble's contents are measured.
fn forwarded_label(ui: &mut egui::Ui, palette: &Palette) -> (Rect, std::sync::Arc<egui::Galley>) {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        "Forwarded",
        0.0,
        egui::TextFormat {
            font_id: theme::regular(12.5),
            color: palette.dim,
            italics: true,
            ..Default::default()
        },
    );
    let galley = ui.painter().layout_job(job);
    let width = FORWARDED_ICON + FORWARDED_GAP + galley.size().x;
    let (reserved, response) =
        ui.allocate_exact_size(vec2(width, FORWARDED_HEIGHT), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, "Forwarded"));
    // Sit closer to what follows than the bubble's usual spacing.
    ui.add_space(-1.0);
    (reserved, galley)
}

/// Paints a forwarded label with its left edge at `at`, as WhatsApp does:
/// at the start of the bubble on either side. Returns where it went.
fn paint_forwarded_label(
    ui: &egui::Ui,
    palette: &Palette,
    at: egui::Pos2,
    galley: std::sync::Arc<egui::Galley>,
) -> Rect {
    let rect = Rect::from_min_size(
        at,
        vec2(
            FORWARDED_ICON + FORWARDED_GAP + galley.size().x,
            FORWARDED_HEIGHT,
        ),
    );
    let icon = Rect::from_min_size(
        pos2(rect.left(), rect.center().y - FORWARDED_ICON / 2.0),
        Vec2::splat(FORWARDED_ICON),
    );
    theme::paint_icon(ui, Icon::Forward, icon, FORWARDED_ICON, palette.dim);
    ui.painter().galley(
        pos2(
            icon.right() + FORWARDED_GAP,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        palette.dim,
    );
    rect
}

/// Painted size of the star mark in the bubble footer.
const FOOTER_MARK: f32 = 13.0;
/// One gap: time to the mark.
const FOOTER_MARK_GAP: f32 = 8.0;
/// Room one visible mark keeps next to the clock.
const FOOTER_MARK_SLOT: f32 = FOOTER_MARK + FOOTER_MARK_GAP;
const FOOTER_STAR: Color32 = Color32::from_rgb(0xEA, 0xB3, 0x08);

/// The clock's own width at the footer's size: the floor a footer starts from,
/// and the one part of it that depends on the reader's clock format.
fn clock_width(ui: &egui::Ui, message: &Message) -> f32 {
    ui.painter()
        .layout_no_wrap(
            crate::util::clock(message.timestamp),
            theme::regular(11.0),
            Color32::WHITE,
        )
        .size()
        .x
}

/// Width of the message footer, including the room its marks take.
fn footer_width(ui: &egui::Ui, message: &Message, starred: bool, pinned: bool) -> f32 {
    let font = theme::regular(11.0);
    let time = clock_width(ui, message);
    let edited = if message.edited {
        ui.painter()
            .layout_no_wrap("edited".to_owned(), font, Color32::WHITE)
            .size()
            .x
            + 4.0
    } else {
        0.0
    };
    let not_sent = if not_sent(message) {
        ui.painter()
            .layout_no_wrap(NOT_SENT.to_owned(), theme::medium(11.0), Color32::WHITE)
            .size()
            .x
            + 6.0
    } else {
        0.0
    };
    // Only a message that carries a mark reserves its slot, as WhatsApp does:
    // a plain footer is the clock and the ticks alone. Starring or pinning
    // grows that bubble's footer by one slot each.
    let marks =
        if starred { FOOTER_MARK_SLOT } else { 0.0 } + if pinned { FOOTER_MARK_SLOT } else { 0.0 };
    time + edited + not_sent + if message.from_me { 19.0 } else { 0.0 } + marks
}

/// Whether the message's time and ticks sit over its picture rather than
/// on a line of their own: a picture without a caption, as in WhatsApp.
fn time_over_picture(message: &Message) -> bool {
    matches!(message.content, Content::Image { caption: None, .. })
}

/// The ticks a message in this view shows (see [`display_delivery`]).
fn shown_delivery(view: &View<'_>, message: &Message) -> Delivery {
    display_delivery(
        message.status,
        message.from_me,
        view.chat.kind,
        view.account_receipts_off,
    )
}

/// A DM read receipt is not visible in WhatsApp when our account has read
/// receipts off. Keep the archived status intact, but match that display here.
pub(super) fn display_delivery(
    status: Delivery,
    from_me: bool,
    chat_kind: crate::model::ChatKind,
    account_receipts_off: bool,
) -> Delivery {
    if account_receipts_off
        && chat_kind == crate::model::ChatKind::Direct
        && from_me
        && status == Delivery::Read
    {
        Delivery::Delivered
    } else {
        status
    }
}

/// Whether a picture stands without a bubble, as Messages draws a photo
/// with nothing else in its message: it takes the bubble's round shape
/// itself. A quote or the forwarded label keeps the bubble around it.
fn bare_picture(palette: &Palette, message: &Message) -> bool {
    palette.bubbles == theme::BubbleStyle::Messages
        && time_over_picture(message)
        && message.quoted.is_none()
        && !message.forwarded
}

/// How round a picture's corners are: a bubble's, when it has none.
fn picture_radius(palette: &Palette, message: &Message) -> f32 {
    if bare_picture(palette, message) {
        f32::from(widgets::MESSAGES_BUBBLE_RADIUS)
    } else {
        6.0
    }
}

/// Space between a picture's edges and the time drawn over it: the scrim
/// around the time keeps a few points clear of the rounded corner.
const OVER_PICTURE_INSET: Vec2 = vec2(10.0, 6.0);

/// Paints the time and ticks over the bottom corner of a picture without a
/// caption, in white on a soft dark scrim so they read on any picture.
fn footer_over_picture(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    picture: Rect,
    starred: bool,
    pinned: bool,
    actions: &mut Vec<Action>,
) {
    let palette = &view.palette;
    let font = theme::regular(11.0);
    let time =
        ui.painter()
            .layout_no_wrap(crate::util::clock(message.timestamp), font, Color32::WHITE);
    let failed = not_sent(message).then(|| {
        ui.painter()
            .layout_no_wrap(NOT_SENT.to_owned(), theme::medium(11.0), Color32::WHITE)
    });
    let tick_width = if message.from_me { 19.0 } else { 0.0 };
    let marks =
        if starred { FOOTER_MARK_SLOT } else { 0.0 } + if pinned { FOOTER_MARK_SLOT } else { 0.0 };
    let width = time.size().x
        + failed.as_ref().map_or(0.0, |galley| galley.size().x + 6.0)
        + tick_width
        + marks;
    let row = Rect::from_min_max(
        pos2(
            picture.right() - OVER_PICTURE_INSET.x - width,
            picture.bottom() - OVER_PICTURE_INSET.y - 15.0,
        ),
        picture.right_bottom() - OVER_PICTURE_INSET,
    );
    if ui.is_rect_visible(picture) {
        let scrim = row.expand2(vec2(6.0, 2.0)).intersect(picture);
        ui.painter()
            .rect_filled(scrim, scrim.height() / 2.0, Color32::from_black_alpha(110));
    }
    let mut x = row.right();
    if message.from_me {
        let ticks = Rect::from_center_size(pos2(x - 7.5, row.center().y), Vec2::splat(15.0));
        widgets::ticks_in(
            ui,
            palette,
            ticks,
            shown_delivery(view, message),
            Color32::WHITE,
        );
        x -= tick_width;
    }
    x -= time.size().x;
    ui.painter().galley(
        pos2(x, row.center().y - time.size().y / 2.0),
        time,
        Color32::WHITE,
    );
    let status =
        Rect::from_x_y_ranges(x..=row.right(), row.center().y - 7.5..=row.center().y + 7.5);
    status_opens_info(ui, view, message, status, actions);
    if let Some(failed) = failed {
        x -= failed.size().x + 6.0;
        let label = Rect::from_min_size(
            pos2(x, row.center().y - failed.size().y / 2.0),
            failed.size(),
        );
        ui.painter().galley(label.min, failed, Color32::WHITE);
        let status = Rect::from_min_max(label.min, pos2(row.right(), label.max.y));
        let response = ui.interact(
            status,
            ui.id().with(("not-sent", &message.id)),
            Sense::hover(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, NOT_SENT_HINT)
        });
        response.on_hover_text(NOT_SENT_HINT);
    }
    if pinned {
        x -= FOOTER_MARK_SLOT;
        theme::paint_icon(
            ui,
            Icon::Pin,
            Rect::from_center_size(
                pos2(x + FOOTER_MARK / 2.0, row.center().y),
                Vec2::splat(FOOTER_MARK),
            ),
            FOOTER_MARK,
            Color32::WHITE,
        );
    }
    if starred {
        x -= FOOTER_MARK_SLOT;
        theme::paint_icon(
            ui,
            Icon::StarFill,
            Rect::from_center_size(
                pos2(x + FOOTER_MARK / 2.0, row.center().y),
                Vec2::splat(FOOTER_MARK),
            ),
            FOOTER_MARK,
            FOOTER_STAR,
        );
    }
    ui.ctx().data_mut(|data| {
        data.insert_temp(footer_id(&message.chat, &message.id), row);
    });
}

/// Where a message's time and ticks were drawn, for layout tests.
pub fn footer_id(chat: &str, message: &str) -> egui::Id {
    bubble_id(chat, message).with("footer")
}

/// Whether "Message info" has receipts to show: our own sent message.
fn has_message_info(message: &Message) -> bool {
    message.from_me
        && !matches!(message.content, Content::Revoked)
        && !matches!(
            message.status,
            Delivery::None | Delivery::Pending | Delivery::Failed
        )
}

/// Clicking our time and ticks opens "Message info", as in WhatsApp.
fn status_opens_info(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    status: Rect,
    actions: &mut Vec<Action>,
) {
    if !has_message_info(message) {
        return;
    }
    let label = crate::i18n::gettext(view.locale, "Message info");
    let response = ui.interact(
        status.expand(2.0),
        ui.id().with(("status", &message.id)),
        Sense::click(),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    let response = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(&*label);
    if response.clicked() {
        actions.push(Action::ShowDialog(Dialog::MessageInfo {
            chat: message.chat.clone(),
            message: message.id.clone(),
        }));
    }
}

fn not_sent(message: &Message) -> bool {
    message.from_me && message.status == Delivery::Failed
}

/// Paints the time and ticks at the bubble's right edge without widening it.
fn footer(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    slot: Option<Rect>,
    starred: bool,
    pinned: bool,
    actions: &mut Vec<Action>,
) {
    let palette = &view.palette;
    let font = theme::regular(11.0);
    let time = ui.painter().layout_no_wrap(
        crate::util::clock(message.timestamp),
        font.clone(),
        palette.secondary,
    );
    let edited = message.edited.then(|| {
        ui.painter()
            .layout_no_wrap("edited".to_owned(), font, palette.dim)
    });
    // A red dot alone does not say what went wrong or what to do. The word
    // uses the text colour: the danger red on an outgoing bubble is too faint
    // to read, and the red icon beside it already carries the alarm.
    let failed = not_sent(message).then(|| {
        ui.painter()
            .layout_no_wrap(NOT_SENT.to_owned(), theme::medium(11.0), palette.text)
    });
    let tick_width = if message.from_me { 19.0 } else { 0.0 };
    let width = time.size().x
        + edited.as_ref().map_or(0.0, |galley| galley.size().x + 4.0)
        + failed.as_ref().map_or(0.0, |galley| galley.size().x + 6.0)
        + tick_width;
    let rect = match slot {
        Some(slot) => slot,
        None => {
            let row_width = ui.min_rect().width().max(width);
            let (rect, _) = ui.allocate_exact_size(vec2(row_width, 15.0), Sense::hover());
            rect
        }
    };
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            footer_id(&message.chat, &message.id),
            Rect::from_min_max(pos2(rect.right() - width, rect.top()), rect.max),
        );
    });
    let mut x = rect.right();
    if message.from_me {
        let ticks = Rect::from_center_size(pos2(x - 7.5, rect.center().y), Vec2::splat(15.0));
        widgets::ticks(ui, palette, ticks, shown_delivery(view, message));
        x -= tick_width;
    }
    x -= time.size().x;
    ui.painter().galley(
        pos2(x, rect.center().y - time.size().y / 2.0),
        time,
        palette.secondary,
    );
    let status = Rect::from_x_y_ranges(
        x..=rect.right(),
        rect.center().y - 7.5..=rect.center().y + 7.5,
    );
    status_opens_info(ui, view, message, status, actions);
    if pinned {
        // Left of the clock, in the room `footer_width` counted for it.
        x -= FOOTER_MARK_SLOT;
        theme::paint_icon(
            ui,
            Icon::Pin,
            Rect::from_center_size(
                pos2(x + FOOTER_MARK / 2.0, rect.center().y),
                Vec2::splat(FOOTER_MARK),
            ),
            FOOTER_MARK,
            palette.secondary,
        );
    }
    if let Some(edited) = edited {
        x -= edited.size().x + 4.0;
        ui.painter().galley(
            pos2(x, rect.center().y - edited.size().y / 2.0),
            edited,
            palette.dim,
        );
    }
    if let Some(failed) = failed {
        x -= failed.size().x + 6.0;
        let label = Rect::from_min_size(
            pos2(x, rect.center().y - failed.size().y / 2.0),
            failed.size(),
        );
        ui.painter().galley(label.min, failed, palette.text);
        // Explain on hover and to screen readers; hovering takes no clicks
        // from the bubble.
        let status = Rect::from_min_max(label.min, pos2(rect.right(), label.max.y));
        let response = ui.interact(
            status,
            ui.id().with(("not-sent", &message.id)),
            Sense::hover(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, NOT_SENT_HINT)
        });
        response.on_hover_text(NOT_SENT_HINT);
    }
    if starred {
        x -= FOOTER_MARK_SLOT;
        theme::paint_icon(
            ui,
            Icon::StarFill,
            Rect::from_center_size(
                pos2(x + FOOTER_MARK / 2.0, rect.center().y),
                Vec2::splat(FOOTER_MARK),
            ),
            FOOTER_MARK,
            FOOTER_STAR,
        );
    }
}

fn reactions(ui: &mut egui::Ui, view: &View<'_>, message: &Message, actions: &mut Vec<Action>) {
    let palette = view.palette;
    let mut counts: Vec<(String, u32, bool, Vec<String>)> = Vec::new();
    for reaction in &message.reactions {
        let who = if reaction.from_me {
            "You".to_owned()
        } else {
            (view.names_or)(&reaction.sender, None)
        };
        match counts
            .iter_mut()
            .find(|(emoji, _, _, _)| *emoji == reaction.emoji)
        {
            Some((_, count, mine, names)) => {
                *count += 1;
                *mine |= reaction.from_me;
                names.push(who);
            }
            None => counts.push((reaction.emoji.clone(), 1, reaction.from_me, vec![who])),
        }
    }
    ui.spacing_mut().item_spacing.x = 3.0;
    for (emoji, count, mine, names) in counts {
        let label = if count > 1 {
            format!("{emoji} {count}")
        } else {
            emoji.clone()
        };
        let line = widgets::line(ui, &label, theme::regular(13.0), palette.text, 200.0, 1);
        let size = line.size() + vec2(12.0, 6.0);
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        if ui.is_rect_visible(rect) {
            // Lifted as gently as a hovered chat-list row, so the pill
            // stands off the bubble it overlaps.
            let mut shadow = palette.bubble_shadow();
            shadow.color = shadow.color.gamma_multiply(0.6);
            ui.painter()
                .add(shadow.as_shape(rect, CornerRadius::from(rect.height() / 2.0)));
            ui.painter()
                .rect_filled(rect, rect.height() / 2.0, palette.overlay);
            ui.painter().rect_stroke(
                rect,
                rect.height() / 2.0,
                Stroke::new(1.0, if mine { palette.accent } else { palette.chat }),
                egui::StrokeKind::Inside,
            );
            line.paint(ui, rect.center() - line.size() / 2.0, palette.text);
        }
        let response = response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(names.join(", "));
        if response.clicked() {
            // Clicking our reaction removes it; clicking another adds it.
            actions.push(Action::React {
                chat: view.chat.id.clone(),
                message: message.id.clone(),
                emoji: if mine { String::new() } else { emoji },
            });
        }
    }
}

const QUICK_REACTIONS: [&str; 6] = ["👍", "❤️", "😂", "😮", "😢", "🙏"];

/// Our existing reaction to a message.
pub(crate) fn own_reaction(message: &Message) -> Option<&str> {
    message
        .reactions
        .iter()
        .find(|reaction| reaction.from_me)
        .map(|reaction| reaction.emoji.as_str())
}

/// Emoji to send for a reaction choice: empty string removes our current one.
pub(crate) fn reaction_choice(current: Option<&str>, emoji: &str) -> String {
    if current == Some(emoji) {
        String::new()
    } else {
        emoji.to_owned()
    }
}

/// Width a message menu needs: its longest entry, or the quick-reaction row
/// when that is wider. Both the bubble and a starred row use it, so the two
/// menus come out the same size.
pub(crate) fn menu_width(
    ui: &egui::Ui,
    locale: crate::i18n::Locale,
    reaction_emoji: &[(String, u32)],
    message: &Message,
) -> f32 {
    let quick = quick_reactions(message, reaction_emoji).len() as f32 + 1.0;
    let labels = message_menu_labels(locale);
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    widgets::menu_width(ui, &refs, true).max(quick * 36.0 + 12.0)
}

/// Every label the menu of a message other than a call can show, in `locale`.
/// The menu is as wide as the longest one, and which one that is depends on
/// the language: a label left out here is cut when its translation is the
/// longest.
fn message_menu_labels(locale: crate::i18n::Locale) -> Vec<String> {
    use crate::i18n::gettext;
    [
        gettext(locale, "Reply"),
        gettext(locale, "Reply privately"),
        gettext(locale, "Star"),
        gettext(locale, "Unstar"),
        gettext(locale, "Pin message"),
        gettext(locale, "Unpin message"),
        gettext(locale, "Forward"),
        gettext(locale, "Open in the viewer"),
        gettext(locale, "Select"),
        gettext(locale, "Copy text"),
        gettext(locale, "Edit"),
        gettext(locale, "Delete for everyone"),
        gettext(locale, "Delete for me"),
        gettext(locale, "Add to favorites"),
        gettext(locale, "Open in system player"),
        gettext(locale, "Open file"),
        gettext(locale, "Copy image"),
        gettext(locale, "Save as…"),
        gettext(locale, "Show in folder"),
        gettext(locale, "Downloading…"),
        gettext(locale, "Download"),
        gettext(locale, "Message info"),
        gettext(locale, "Reload earlier messages"),
        gettext(locale, "Copy message ID"),
    ]
    .into_iter()
    .map(|label| label.into_owned())
    .collect()
}

/// Where a message menu was opened from. A starred row has no composer, no
/// selection bar, and no revoke window of its own, so the entries that need one
/// ask for the chat to be opened at the message first.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuOrigin {
    Chat,
    List,
}

/// How a left-list row's message is marked, for the menu's Star label.
#[derive(Clone, Copy)]
pub(crate) struct Marks {
    pub starred: bool,
}

/// The message menu for a row drawn outside the conversation: the starred
/// list. It is the chat's own menu, with the marks the list already knows, so
/// both places offer exactly the same entries.
pub(crate) fn list_menu(
    app: &App,
    ui: &mut egui::Ui,
    chat: &Chat,
    message: &Message,
    marks: Marks,
    actions: &mut Vec<Action>,
) {
    let stars: HashSet<String> = marks
        .starred
        .then(|| message.id.clone())
        .into_iter()
        .collect();
    let names_or = |id: &str, hint: Option<&str>| app.display_name_or(id, hint);
    let mention_names = |id: &str| app.mention_name(id);
    let avatars = HashMap::new();
    let keyboard_navigation = std::cell::Cell::new(false);
    let view = View {
        palette: app.palette,
        locale: app.locale,
        chat,
        me: app.me.as_deref(),
        auto_download: app.settings.auto_download,
        connected: app.link.is_connected(),
        poll_voting: &app.poll_voting,
        interactive_pending: &app.interactive_sending,
        // Nothing scrolls, highlights, or selects in a list row.
        anchor: None,
        open_menu: None,
        reaction: None,
        reaction_menu: false,
        reaction_emoji: &app.settings.reaction_emoji,
        keyboard_navigation: &keyboard_navigation,
        names_or: &names_or,
        mention_names: &mention_names,
        avatars: &avatars,
        contacts: &app.contacts,
        now: crate::util::now(),
        // A row drawn outside the conversation holds still.
        animate: false,
        player: &app.player,
        voice_speed: app.voice_speed_in(Some(&chat.id)),
        video: &app.video,
        account_receipts_off: app.account_receipts_off,
        copy_rows: Some(app.copy_rows.as_ref()),
        selecting: false,
        motion_playing: None,
        starred: marks.starred.then_some(&stars),
        pins: app.pins.get(&message.chat),
        pin_notices: &[],
    };
    context_menu(ui, &view, message, MenuOrigin::List, actions);
}

/// Quick reactions plus our current reaction when needed.
fn quick_reactions<'a>(message: &'a Message, preferred: &'a [(String, u32)]) -> Vec<&'a str> {
    let mut list = Vec::new();
    for emoji in preferred
        .iter()
        .map(|(emoji, _)| emoji.as_str())
        .chain(QUICK_REACTIONS.iter().copied())
    {
        if emojis::get(emoji).is_some() && !list.contains(&emoji) {
            list.push(emoji);
        }
        if list.len() == QUICK_REACTIONS.len() {
            break;
        }
    }
    if let Some(mine) = own_reaction(message)
        && !list.contains(&mine)
    {
        list.push(mine);
    }
    list
}

/// The file the menu's viewer item would open, when the message has one to
/// show.
///
/// The album decides what belongs in the viewer: a sticker, a GIF or a card's
/// image is not in it, so none of them offers the item. The recorded path has
/// to still be a file: a path the archive kept after the file was deleted or
/// moved elsewhere would open a viewer with nothing to show, and the click
/// would fall back to opening a file that is not there.
fn viewer_file(message: &Message) -> Option<&Path> {
    message.content.gallery_kind()?;
    message
        .content
        .media()
        .and_then(|media| media.path.as_deref())
        .filter(|path| crate::image_preview::can_view(path) && path.is_file())
}

/// Checks every selected message's revoke eligibility with one scan of loaded history.
fn selection_can_revoke(messages: &[Message], selected: &[String], now: i64) -> bool {
    if selected.is_empty() {
        return false;
    }
    let mut remaining: std::collections::HashSet<&str> =
        selected.iter().map(String::as_str).collect();
    for message in messages {
        if remaining.remove(message.id.as_str()) {
            if !message.from_me
                || matches!(message.content, Content::Revoked)
                || now - message.timestamp > crate::app::REVOKE_WINDOW.as_secs() as i64
            {
                return false;
            }
            if remaining.is_empty() {
                return true;
            }
        }
    }
    false
}

/// Draws batch actions for the selected messages, including separate deletion choices.
fn selection_menu(
    ui: &mut egui::Ui,
    view: &View<'_>,
    messages: &[Message],
    selected: &[String],
    actions: &mut Vec<Action>,
) {
    selection_delete_items(
        ui,
        &view.palette,
        view.locale,
        &view.chat.id,
        selection_can_revoke(messages, selected, view.now),
        selected,
        actions,
    );
}

/// The two ways to delete a selection, shared by the selection's context
/// menu and the selection bar's Delete menu. Each opens the confirmation.
fn selection_delete_items(
    ui: &mut egui::Ui,
    palette: &Palette,
    locale: crate::i18n::Locale,
    chat: &str,
    can_revoke: bool,
    selected: &[String],
    actions: &mut Vec<Action>,
) {
    for (label, for_everyone, enabled) in [
        (
            crate::i18n::gettext(locale, "Delete for everyone"),
            true,
            can_revoke,
        ),
        (
            crate::i18n::gettext(locale, "Delete for me"),
            false,
            !selected.is_empty(),
        ),
    ] {
        if widgets::menu_item_enabled(ui, palette, Some(Icon::Trash), &label, enabled) {
            actions.push(Action::ShowDialog(Dialog::ConfirmDeleteSelection {
                chat: chat.to_owned(),
                messages: selected.to_vec(),
                for_everyone,
            }));
        }
    }
}

/// Offers actions supported by the selected message, including private replies for eligible group senders.
fn context_menu(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    origin: MenuOrigin,
    actions: &mut Vec<Action>,
) {
    let palette = view.palette;
    let chat = &view.chat.id;
    if let Content::Call { group, .. } = message.content {
        call_menu(ui, view, &message.id, group, actions);
        return;
    }
    let mine = own_reaction(message);
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 34.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for emoji in quick_reactions(message, view.reaction_emoji) {
                let chosen = mine == Some(emoji);
                let line = widgets::line(ui, emoji, theme::regular(20.0), palette.text, 40.0, 1);
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::click());
                // Screen readers speak the emoji's name.
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), emoji)
                });
                theme::focus_outline(ui, response.id, rect, 17.0);
                if chosen {
                    ui.painter()
                        .circle_filled(rect.center(), 17.0, palette.surface_active);
                    ui.painter().circle_stroke(
                        rect.center(),
                        16.0,
                        Stroke::new(1.5, palette.accent),
                    );
                } else if response.hovered() {
                    ui.painter()
                        .circle_filled(rect.center(), 17.0, palette.surface_hover);
                }
                line.paint(ui, rect.center() - line.size() / 2.0, palette.text);
                let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                let response = if chosen {
                    response.on_hover_text(
                        crate::i18n::gettext(view.locale, "Remove your reaction").into_owned(),
                    )
                } else {
                    response
                };
                if response.clicked() {
                    // Selecting our current reaction removes it.
                    actions.push(Action::React {
                        chat: chat.clone(),
                        message: message.id.clone(),
                        emoji: reaction_choice(mine, emoji),
                    });
                    ui.close();
                }
            }
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::click());
            // The tooltip doubles as the screen reader's label.
            let more = crate::i18n::gettext(view.locale, "React with any emoji").into_owned();
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &more)
            });
            theme::focus_outline(ui, response.id, rect, 17.0);
            if ui.is_rect_visible(rect) {
                let hovered = response.hovered();
                ui.painter().circle_filled(
                    rect.center(),
                    17.0,
                    if hovered {
                        palette.surface_hover
                    } else {
                        palette.surface
                    },
                );
                ui.painter()
                    .circle_stroke(rect.center(), 16.0, Stroke::new(1.0, palette.outline));
                theme::paint_icon(ui, Icon::Plus, rect, 16.0, palette.secondary);
            }
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(more)
                .clicked()
            {
                actions.push(Action::OpenReactionPicker {
                    chat: chat.clone(),
                    message: message.id.clone(),
                    beside_menu: true,
                    from_list: false,
                });
            }
        },
    );
    widgets::menu_separator(ui, &palette);
    // The first item below the reactions, which a menu opened from the
    // keyboard focuses: Reply, or whichever comes first. A deleted message
    // has none of the rows down to Select: its first entry, or Delete for
    // me, which every message menu has, is its first.
    let mut first = None;
    let mut item = |ui: &mut egui::Ui, icon: Icon, label: &str| {
        let response = widgets::menu_item_response(ui, &palette, Some(icon), label, true);
        first.get_or_insert(response.id);
        response.clicked()
    };
    if can_reply(&message.content)
        && item(ui, Icon::Reply, &crate::i18n::gettext(view.locale, "Reply"))
    {
        if matches!(origin, MenuOrigin::List) {
            actions.push(Action::OpenMessage {
                chat: chat.clone(),
                message: message.id.clone(),
            });
        }
        actions.push(Action::Reply(message.id.clone()));
    }
    // From the starred list the source chat is not on screen, and the
    // private reply starts from the group the reader is looking at.
    if !matches!(origin, MenuOrigin::List)
        && message.private_reply_recipient().is_some()
        && item(
            ui,
            Icon::Reply,
            &crate::i18n::gettext(view.locale, "Reply privately"),
        )
    {
        actions.push(Action::ReplyPrivately {
            chat: chat.clone(),
            message: message.id.clone(),
        });
    }
    let starred = view.starred.is_some_and(|ids| ids.contains(&message.id));
    let star_label = if starred {
        crate::i18n::gettext(view.locale, "Unstar")
    } else {
        crate::i18n::gettext(view.locale, "Star")
    };
    if !matches!(message.content, Content::Revoked) && item(ui, Icon::Star, &star_label) {
        actions.push(Action::SetStar {
            chat: chat.clone(),
            message: message.id.clone(),
            starred: !starred,
        });
        ui.close();
    }
    let pinned = view.pins.is_some_and(|ids| ids.contains(&message.id));
    let pin_label = if pinned {
        crate::i18n::gettext(view.locale, "Unpin message")
    } else {
        crate::i18n::gettext(view.locale, "Pin message")
    };
    if !matches!(message.content, Content::Revoked) && item(ui, Icon::Pin, &pin_label) {
        actions.push(Action::SetMessagePinned {
            chat: chat.clone(),
            message: message.id.clone(),
            pinned: !pinned,
        });
    }
    if crate::app::can_select(&message.content)
        && item(
            ui,
            Icon::Forward,
            &crate::i18n::gettext(view.locale, "Forward"),
        )
    {
        actions.push(forward_action(chat, &message.id));
    }
    // A photo opens the viewer on its own click; a clip plays in its bubble
    // there, so this is how the album is reached from a chat that holds no
    // pictures.
    if let Some(path) = viewer_file(message)
        && item(
            ui,
            Icon::Maximize,
            &crate::i18n::gettext(view.locale, "Open in the viewer"),
        )
    {
        actions.push(Action::PreviewImage {
            path: path.to_owned(),
            chat: chat.clone(),
            message: message.id.clone(),
            card: None,
        });
    }
    if crate::app::can_select(&message.content)
        && item(
            ui,
            Icon::Check,
            &crate::i18n::gettext(view.locale, "Select"),
        )
    {
        if matches!(origin, MenuOrigin::List) {
            actions.push(Action::OpenMessage {
                chat: chat.clone(),
                message: message.id.clone(),
            });
        }
        actions.push(Action::SelectMessage(message.id.clone()));
    }
    // What else a pointer reaches in the bubble, above Copy, in a menu the
    // keys opened: a card's buttons, the quote, the sender. A disabled entry
    // takes no focus, as its row in the bubble.
    let keys_menu = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(keys_menu_id(ui.layer_id().id)))
        .unwrap_or(false);
    let entries = if keys_menu {
        inner_entries(view, message)
    } else {
        Vec::new()
    };
    if !entries.is_empty() {
        widgets::menu_separator(ui, &palette);
        for entry in entries {
            // Hover text is out of a screen reader's reach, so an unavailable
            // entry says why in its name, translated; the hover text stays
            // as the bubble shows it.
            let accessible = match entry.hint {
                Some(hint) if !entry.enabled => crate::a11y::fill(
                    &crate::i18n::gettext(view.locale, "{option}, unavailable: {reason}"),
                    &[
                        ("option", &entry.label),
                        ("reason", &hint.said(view.locale)),
                    ],
                ),
                _ => entry.label.clone(),
            };
            let response = widgets::menu_item_focusable(
                ui,
                &palette,
                Some(entry.icon),
                &entry.label,
                &accessible,
                entry.enabled,
                entry.enabled,
            );
            if entry.enabled {
                first.get_or_insert(response.id);
            }
            let clicked = response.clicked();
            if let Some(hint) = entry.hint {
                response.on_hover_text(hint.shown());
            }
            if clicked && let Some(action) = entry.action {
                actions.push(action);
            }
        }
        widgets::menu_separator(ui, &palette);
    }
    if let Some(text) = copyable_text(&message.content)
        && widgets::menu_item(
            ui,
            &palette,
            Some(Icon::Copy),
            &crate::i18n::gettext(view.locale, "Copy text"),
        )
    {
        let mentions = mentions_of(view, message);
        actions.push(Action::CopyText(markup::plain(&text, &mentions)));
    }
    let age = view.now - message.timestamp;
    let can_edit = message.from_me
        && matches!(message.content, Content::Text { .. })
        && age <= crate::app::EDIT_WINDOW.as_secs() as i64;
    let can_revoke = revoke_blocker(message, view.now).is_none();
    if can_edit
        && widgets::menu_item(
            ui,
            &palette,
            Some(Icon::Pencil),
            &crate::i18n::gettext(view.locale, "Edit"),
        )
    {
        if matches!(origin, MenuOrigin::List) {
            actions.push(Action::OpenMessage {
                chat: chat.clone(),
                message: message.id.clone(),
            });
        }
        actions.push(Action::Edit(message.id.clone()));
    }
    if can_revoke
        && widgets::menu_item(
            ui,
            &palette,
            Some(Icon::Trash),
            &crate::i18n::gettext(view.locale, "Delete for everyone"),
        )
    {
        actions.push(delete_for_everyone_action(chat, &message.id));
    }
    let delete_for_me = widgets::menu_item_response(
        ui,
        &palette,
        Some(Icon::EyeOff),
        &crate::i18n::gettext(view.locale, "Delete for me"),
        true,
    );
    first.get_or_insert(delete_for_me.id);
    // A popup's first pass only measures it, drawing nothing that could
    // take focus; only a pass that draws the menu names its item.
    let first = (!ui.is_sizing_pass()).then_some(first).flatten();
    ui.ctx().data_mut(|data| {
        data.insert_temp(menu_first_item_id(chat, &message.id), first);
    });
    if delete_for_me.clicked() {
        actions.push(delete_for_me_action(chat, &message.id));
    }
    if let Content::Sticker { media, .. } = &message.content
        && let Some(path) = &media.path
        && widgets::menu_item(
            ui,
            &palette,
            Some(Icon::Star),
            &crate::i18n::gettext(view.locale, "Add to favorites"),
        )
    {
        actions.push(Action::SaveSticker(path.clone()));
    }
    if let Some(media) = message.content.media() {
        match &media.path {
            Some(path) => {
                let open = if matches!(message.content, Content::Video { gif: false, .. }) {
                    crate::i18n::gettext(view.locale, "Open in system player")
                } else {
                    crate::i18n::gettext(view.locale, "Open file")
                };
                if widgets::menu_item(ui, &palette, Some(Icon::ExternalLink), &open) {
                    actions.push(Action::OpenFile(path.clone()));
                }
                if matches!(message.content, Content::Image { .. })
                    && widgets::menu_item(
                        ui,
                        &palette,
                        Some(Icon::Copy),
                        &crate::i18n::gettext(view.locale, "Copy image"),
                    )
                {
                    actions.push(Action::CopyImage(path.clone()));
                }
                if widgets::menu_item(
                    ui,
                    &palette,
                    Some(Icon::Download),
                    &crate::i18n::gettext(view.locale, "Save as…"),
                ) {
                    actions.push(Action::SaveAttachmentAs {
                        path: path.clone(),
                        name: attachment_name(&message.content, path),
                    });
                }
                if let Some(folder) = path.parent()
                    && widgets::menu_item(
                        ui,
                        &palette,
                        Some(Icon::FileText),
                        &crate::i18n::gettext(view.locale, "Show in folder"),
                    )
                {
                    actions.push(Action::OpenFolder(folder.to_path_buf()));
                }
            }
            None => {
                let downloading = matches!(media.state, MediaState::Downloading);
                if widgets::menu_item_enabled(
                    ui,
                    &palette,
                    Some(Icon::Download),
                    &if downloading {
                        crate::i18n::gettext(view.locale, "Downloading…")
                    } else {
                        crate::i18n::gettext(view.locale, "Download")
                    },
                    !downloading,
                ) {
                    actions.push(Action::Download {
                        manual: true,
                        card: None,
                        chat: chat.clone(),
                        message: message.id.clone(),
                    });
                }
            }
        }
    }
    if let Content::Audio { media, .. } = &message.content
        && media.path.is_some()
    {
        speed_menu_row(ui, view, message, actions);
    }
    widgets::menu_separator(ui, &palette);
    // The menu holds actions only. Sent, delivery, and read times, per member
    // in a group, live in "Message info".
    if has_message_info(message)
        && widgets::menu_item(
            ui,
            &palette,
            Some(Icon::Info),
            &crate::i18n::gettext(view.locale, "Message info"),
        )
    {
        actions.push(Action::ShowDialog(Dialog::MessageInfo {
            chat: chat.clone(),
            message: message.id.clone(),
        }));
    }
    if widgets::menu_item_enabled(
        ui,
        &palette,
        Some(Icon::Refresh),
        &crate::i18n::gettext(view.locale, "Reload earlier messages"),
        view.connected && !matches!(message.status, Delivery::Pending | Delivery::Failed),
    ) {
        actions.push(Action::ReloadHistory {
            chat: chat.clone(),
            message: message.id.clone(),
        });
    }
    // The id helps when looking a message up for a bug report.
    if widgets::menu_item(
        ui,
        &palette,
        Some(Icon::Copy),
        &crate::i18n::gettext(view.locale, "Copy message ID"),
    ) {
        actions.push(Action::CopyText(message.id.clone()));
    }
}

/// One menu entry for something inside a message: a link, a poll option or
/// an interactive button. Everything a pointer reaches in the bubble is also
/// here, for the keyboard and for screen readers.
struct InnerEntry {
    label: String,
    icon: Icon,
    /// What choosing it does. `None` for an entry that has nothing to do.
    action: Option<Action>,
    enabled: bool,
    /// Why the entry is unavailable, shown on hover as in the bubble.
    hint: Option<ButtonBlocker>,
}

/// The menu entries for the buttons of one interactive card. `card_number`
/// names the card within a carousel.
fn button_entries(
    view: &View<'_>,
    message: &Message,
    card: &crate::model::InteractiveCard,
    card_number: Option<usize>,
    entries: &mut Vec<InnerEntry>,
) {
    use crate::model::InteractiveAction;
    for (index, button) in card.buttons.iter().enumerate() {
        let enabled = interactive_enabled(view, message, button);
        let icon = if button.url.is_some() {
            Icon::ExternalLink
        } else {
            match button.action {
                InteractiveAction::Reply => Icon::Reply,
                InteractiveAction::Select(_) => Icon::ListChecks,
                InteractiveAction::Copy(_) => Icon::Copy,
                InteractiveAction::Unavailable => Icon::Smartphone,
            }
        };
        let label = match card_number {
            Some(number) => crate::a11y::fill(
                &crate::i18n::gettext(view.locale, "Card {number}: {button}"),
                &[
                    ("number", &(number + 1).to_string()),
                    ("button", &button.label),
                ],
            ),
            None => button.label.clone(),
        };
        entries.push(InnerEntry {
            label,
            icon,
            action: enabled
                .then(|| interactive_action(message, index, button))
                .flatten(),
            enabled,
            hint: (!enabled).then(|| interactive_reason(view, message, button)),
        });
    }
}

/// The entries the message menu adds for what the keys reach nowhere else,
/// in reading order: an interactive card's buttons, then what is about the
/// message rather than in it, each as its click target does: the quote,
/// and a group sender's name and picture. The links, cards, poll options
/// and files in the message are the item cursor's; a video fills the window
/// from the menu's own "Open in the viewer".
fn inner_entries(view: &View<'_>, message: &Message) -> Vec<InnerEntry> {
    let mut entries = Vec::new();
    if let Content::Interactive {
        card: Some(card), ..
    } = &message.content
    {
        button_entries(view, message, card, None, &mut entries);
        // One group per carousel card, each acting on its own card row as the
        // card's buttons do.
        for (index, child) in card.carousel.iter().enumerate() {
            let row = carousel_row(message, index, child);
            button_entries(view, &row, child, Some(index), &mut entries);
        }
    }
    if let Some(quoted) = &message.quoted {
        entries.push(InnerEntry {
            label: crate::i18n::gettext(view.locale, "Go to replied message").into_owned(),
            icon: Icon::Reply,
            action: Some(quote_key_action(quoted)),
            enabled: true,
            hint: None,
        });
    }
    if shows_sender_pictures(view.chat) && !message.from_me {
        let name = (view.names_or)(&message.sender, message.sender_name.as_deref());
        entries.push(InnerEntry {
            label: crate::a11y::fill(
                &crate::i18n::gettext(view.locale, "Info about {name}"),
                &[("name", name.trim_start_matches('~'))],
            ),
            icon: Icon::Info,
            action: Some(sender_info_action(message)),
            enabled: true,
            hint: None,
        });
    }
    entries
}

/// Where a message menu's popup records whether the keys opened it.
fn keys_menu_id(popup: egui::Id) -> egui::Id {
    popup.with("opened-by-keys")
}

/// What clicking a group sender's name or picture does: their info.
fn sender_info_action(message: &Message) -> Action {
    Action::OpenInfo(message.sender.clone())
}

/// Where the message menu records its first item for this pass.
fn menu_first_item_id(chat: &str, message: &str) -> egui::Id {
    bubble_id(chat, message).with("menu-first-item")
}

/// Whether the message menu offers Reply. A call in the chat is a record,
/// with its own menu and no replies.
pub(crate) fn can_reply(content: &Content) -> bool {
    !matches!(content, Content::Revoked | Content::Call { .. })
}

/// The message menu's Forward: the forward dialog for this one message.
pub(crate) fn forward_action(chat: &str, message: &str) -> Action {
    Action::ShowDialog(Dialog::Forward {
        chat: chat.to_owned(),
        messages: vec![message.to_owned()],
    })
}

/// The message menu's "Delete for me": its confirmation, never the deletion.
pub(crate) fn delete_for_me_action(chat: &str, message: &str) -> Action {
    Action::ShowDialog(Dialog::ConfirmDeleteMessage {
        chat: chat.to_owned(),
        message: message.to_owned(),
        for_everyone: false,
    })
}

/// The message menu's "Delete for everyone": its confirmation, never the
/// deletion.
pub(crate) fn delete_for_everyone_action(chat: &str, message: &str) -> Action {
    Action::ShowDialog(Dialog::ConfirmDeleteMessage {
        chat: chat.to_owned(),
        message: message.to_owned(),
        for_everyone: true,
    })
}

/// Why the message menu offers no "Delete for everyone" for a message.
/// WhatsApp deletes only our own messages for everyone, once, and only
/// within its time limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RevokeBlocker {
    NotOurs,
    Deleted,
    TooOld,
}

impl RevokeBlocker {
    #[cfg(test)]
    const ALL: [Self; 3] = [Self::NotOurs, Self::Deleted, Self::TooOld];

    /// The reason, translated, as Shift+Delete says it.
    fn said(self, locale: crate::i18n::Locale) -> String {
        use crate::i18n::gettext;
        match self {
            Self::NotOurs => gettext(locale, "Only messages you sent"),
            Self::Deleted => gettext(locale, "This message was deleted"),
            Self::TooOld => gettext(locale, "The time limit has passed"),
        }
        .into_owned()
    }
}

/// Why `message` cannot be deleted for everyone at `now`, or None when the
/// message menu offers "Delete for everyone" for it.
pub(crate) fn revoke_blocker(message: &Message, now: i64) -> Option<RevokeBlocker> {
    if !message.from_me {
        Some(RevokeBlocker::NotOurs)
    } else if matches!(message.content, Content::Revoked) {
        Some(RevokeBlocker::Deleted)
    } else if now - message.timestamp > crate::app::REVOKE_WINDOW.as_secs() as i64 {
        Some(RevokeBlocker::TooOld)
    } else {
        None
    }
}

/// What Shift+Delete says on a message the menu offers no "Delete for
/// everyone" for: the item's name and why, as the keyboard menu names an
/// unavailable entry.
pub(crate) fn revoke_refusal_said(locale: crate::i18n::Locale, blocker: RevokeBlocker) -> String {
    crate::a11y::fill(
        &crate::i18n::gettext(locale, "{option}, unavailable: {reason}"),
        &[
            (
                "option",
                &crate::i18n::gettext(locale, "Delete for everyone"),
            ),
            ("reason", &blocker.said(locale)),
        ],
    )
}

/// What the menu's "Copy text" copies, before mentions are resolved: the
/// text or caption, a location's coordinates or a contact's card.
fn copyable_text(content: &Content) -> Option<String> {
    match content {
        Content::Text { text, .. } | Content::Interactive { text, .. } => Some(text.clone()),
        Content::Image { caption, .. }
        | Content::Video { caption, .. }
        | Content::Document { caption, .. } => caption.clone(),
        Content::Location {
            latitude,
            longitude,
            ..
        }
        | Content::LiveLocation {
            latitude,
            longitude,
            ..
        } => Some(format!("{latitude},{longitude}")),
        Content::Contact { vcard, .. } => Some(vcard.clone()),
        _ => None,
    }
}

/// What C copies from the focused message: what the menu's first copy item
/// copies, its text, or else a downloaded picture. "Copy message ID", a
/// debugging aid, does not count; a message with neither copies nothing.
pub(crate) fn copy_key_action(
    message: &Message,
    mention_names: &dyn Fn(&str) -> String,
) -> Option<Action> {
    if let Some(text) = copyable_text(&message.content) {
        let mentions = mentions_named(message, mention_names);
        return Some(Action::CopyText(markup::plain(&text, &mentions)));
    }
    match &message.content {
        Content::Image { media, .. } => media.path.clone().map(Action::CopyImage),
        _ => None,
    }
}

/// What clicking a quote does: the list moves to the replied message, which
/// it brings into view, loading older history when needed. A private reply
/// quotes a message of another chat, which opens at it.
pub(crate) fn quote_jump_action(quoted: &crate::model::Quoted) -> Action {
    match &quoted.chat {
        Some(chat) => Action::OpenMessage {
            chat: chat.clone(),
            message: quoted.id.clone(),
        },
        None => Action::ScrollTo(quoted.id.clone()),
    }
}

/// The quote's jump from the keys: the same, except that in the same chat
/// the list keeps its place until the replied message is there to move to.
pub(crate) fn quote_key_action(quoted: &crate::model::Quoted) -> Action {
    match &quoted.chat {
        Some(_) => quote_jump_action(quoted),
        None => Action::GoToReplied(quoted.id.clone()),
    }
}

fn mentions_of(view: &View<'_>, message: &Message) -> Vec<markup::Mention> {
    mentions_named(message, view.mention_names)
}

fn mentions_named(
    message: &Message,
    mention_names: &dyn Fn(&str) -> String,
) -> Vec<markup::Mention> {
    message
        .mentions
        .iter()
        .map(|mention| markup::Mention {
            user: mention.user.clone(),
            name: mention_names(&mention.id),
        })
        .collect()
}

fn quote_mentions(view: &View<'_>, quoted: &crate::model::Quoted) -> Vec<markup::Mention> {
    quoted
        .mentions
        .iter()
        .map(|mention| markup::Mention {
            user: mention.user.clone(),
            name: (view.mention_names)(&mention.id),
        })
        .collect()
}

fn vcard_tel_preference(property: &str) -> Option<u8> {
    let mut best = None;
    for parameter in property.split(';').skip(1) {
        let (name, value) = parameter.split_once('=').unwrap_or(("TYPE", parameter));
        let value = value.trim_matches('"');
        let rank = if name.eq_ignore_ascii_case("PREF") {
            value
                .parse::<u8>()
                .ok()
                .filter(|rank| (1..=100).contains(rank))
        } else if name.eq_ignore_ascii_case("TYPE")
            && value
                .split(',')
                .any(|value| value.eq_ignore_ascii_case("PREF"))
        {
            Some(1)
        } else {
            None
        };
        if let Some(rank) = rank {
            best = Some(best.map_or(rank, |current: u8| current.min(rank)));
        }
    }
    best
}

/// The first card of a shared contact, as the bubble uses it.
#[derive(Debug, PartialEq)]
struct SharedContact {
    name: String,
    /// The telephone number as the card writes it.
    number: String,
    /// The WhatsApp account behind the number, in digits: the `waid`
    /// parameter WhatsApp adds, or a number written in international form.
    /// A local number without a country code cannot name one.
    account: Option<String>,
    /// The first name from the card's structured `N` property, middle names
    /// included, for the contact editor; none when the card has none.
    first_name: Option<String>,
}

/// The first name in a vCard `N` value
/// (`family;given;additional;prefix;suffix`), middle names included.
/// Components are split at unescaped semicolons only.
fn vcard_first_name(value: &str) -> Option<String> {
    let mut parts = vec![String::new()];
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        match character {
            '\\' => match chars.next() {
                Some('n' | 'N') => parts.last_mut()?.push(' '),
                Some(escaped) => parts.last_mut()?.push(escaped),
                None => {}
            },
            ';' => parts.push(String::new()),
            _ => parts.last_mut()?.push(character),
        }
    }
    let part = |index: usize| parts.get(index).map_or("", |part| part.trim());
    let first = [part(1), part(2)]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (!first.is_empty()).then_some(first)
}

fn vcard_tel_account(property: &str, value: &str) -> Option<String> {
    let digits = |text: &str| {
        text.chars()
            .filter(char::is_ascii_digit)
            .collect::<String>()
    };
    let waid = property.split(';').skip(1).find_map(|parameter| {
        let (name, value) = parameter.split_once('=')?;
        name.eq_ignore_ascii_case("WAID")
            .then(|| digits(value.trim_matches('"')))
    });
    waid.or_else(|| value.trim().starts_with('+').then(|| digits(value)))
        .filter(|account| account.len() >= 7)
}

fn shared_contact_details(vcard: &str, fallback_name: &str) -> Option<SharedContact> {
    let mut name = None;
    let mut first_name = None;
    let mut first_phone = None;
    let mut preferred_phone: Option<(u8, (String, Option<String>))> = None;
    let mut first_card: Vec<String> = Vec::new();
    for line in vcard.lines() {
        let line = line.trim_end_matches('\r');
        if line.starts_with([' ', '\t']) {
            if let Some(previous) = first_card.last_mut() {
                previous.push_str(line.trim_start());
            }
            continue;
        }
        if line.eq_ignore_ascii_case("END:VCARD") && !first_card.is_empty() {
            break;
        }
        if line.eq_ignore_ascii_case("BEGIN:VCARD") {
            if first_card.is_empty() {
                first_card.push(line.to_owned());
            }
            continue;
        }
        if !first_card.is_empty() {
            first_card.push(line.to_owned());
        }
    }
    if first_card.is_empty() {
        first_card.extend(vcard.lines().map(str::to_owned));
    }
    for line in first_card {
        let Some((property, value)) = line.split_once(':') else {
            continue;
        };
        let raw_name = property.split(';').next().unwrap_or(property);
        let property_name = raw_name.rsplit('.').next().unwrap_or(raw_name);
        if property_name.eq_ignore_ascii_case("FN") {
            name = Some(value.trim().to_owned());
        } else if property_name.eq_ignore_ascii_case("N") {
            first_name = vcard_first_name(value);
        } else if property_name.eq_ignore_ascii_case("TEL") {
            let number = value.trim().to_owned();
            if number.chars().filter(char::is_ascii_digit).count() < 7 {
                continue;
            }
            let phone = (number, vcard_tel_account(property, value));
            if let Some(rank) = vcard_tel_preference(property) {
                let replace = preferred_phone
                    .as_ref()
                    .is_none_or(|(best_rank, _)| rank < *best_rank);
                if replace {
                    preferred_phone = Some((rank, phone));
                }
            } else if first_phone.is_none() {
                first_phone = Some(phone);
            }
        }
    }
    let (number, account) = preferred_phone.map(|(_, phone)| phone).or(first_phone)?;
    let name = name
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| fallback_name.to_owned());
    Some(SharedContact {
        name,
        number,
        account,
        first_name,
    })
}

/// Draws a message body and returns optional footer space on its last line.
fn content(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    width: f32,
    reserve: f32,
    actions: &mut Vec<Action>,
) -> Option<Rect> {
    let palette = view.palette;
    let own = message.from_me;
    // Add non-text messages to cross-message transcript copies.
    let has_body = match &message.content {
        Content::Text { .. } => true,
        // Image-only cards and carousels without a body draw no text.
        Content::Interactive { card, .. } => card.as_ref().is_none_or(|card| {
            !card.body.is_empty() || (card.image.is_none() && card.carousel.is_empty())
        }),
        Content::Image { caption, .. }
        | Content::Video { caption, .. }
        | Content::Document { caption, .. } => caption.is_some(),
        _ => false,
    };
    if !has_body && let Some(copy_rows) = view.copy_rows {
        copy_rows
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(transcript_row(view, message, String::new(), Vec::new()));
    }
    match &message.content {
        Content::Interactive { text, card } => {
            let Some(card) = card else {
                let span = message.quoted.is_some().then_some(width);
                return rich_body(ui, view, message, text, width, Some(reserve), span, actions);
            };
            if !card.carousel.is_empty() {
                carousel(ui, view, message, card, width, actions);
                return None;
            }
            if let Some(image) = &card.image {
                picture(ui, view, message, image, width, None, actions);
                ui.add_space(4.0);
                if card.body.is_empty() {
                    return None;
                }
            }
            let body = if card.body.is_empty() {
                "Interactive message"
            } else {
                &card.body
            };
            rich_body(
                ui,
                view,
                message,
                body,
                width,
                Some(reserve),
                Some(width),
                actions,
            )
        }
        Content::Text { text, preview } => {
            if let Some(preview) = preview {
                preview_card(ui, view, message, preview, width, actions);
            }
            let span = (message.quoted.is_some() || preview.is_some()).then_some(width);
            rich_body(ui, view, message, text, width, Some(reserve), span, actions)
        }
        Content::Image {
            caption,
            media,
            motion,
        } => {
            let drawn = picture(ui, view, message, media, width, None, actions);
            if let Some(motion) = motion {
                motion_clip(ui, view, message, motion, drawn, actions);
            }
            ui.ctx().data_mut(|data| {
                data.insert_temp(bubble_id(&view.chat.id, &message.id).with("picture"), drawn);
            });
            let Some(caption) = caption else {
                // The time and ticks go over the picture's corner.
                return Some(drawn);
            };
            let drawn = drawn.width();
            {
                // Wrap the caption to the settled image or quote width.
                let wrap = if message.quoted.is_some() {
                    width.max(drawn)
                } else {
                    drawn
                };
                rich_body(
                    ui,
                    view,
                    message,
                    caption,
                    wrap,
                    Some(reserve),
                    Some(wrap),
                    actions,
                )
            }
        }
        Content::Sticker { media, animated } => {
            picture(ui, view, message, media, width, Some(*animated), actions);
            None
        }
        Content::Video {
            caption,
            media,
            seconds,
            gif,
            note,
        } => {
            let drawn = if *note {
                video_note(ui, view, message, media, *seconds, actions)
            } else {
                video(ui, view, message, media, *seconds, *gif, width, actions)
            };
            caption.as_ref().and_then(|caption| {
                let wrap = if message.quoted.is_some() {
                    width.max(drawn)
                } else {
                    drawn
                };
                rich_body(
                    ui,
                    view,
                    message,
                    caption,
                    wrap,
                    Some(reserve),
                    Some(wrap),
                    actions,
                )
            })
        }
        Content::Audio {
            media,
            seconds,
            waveform,
            ..
        } => voice_player(
            ui, view, message, media, *seconds, waveform, width, reserve, actions,
        ),
        Content::Document {
            media,
            file_name,
            caption,
            pages,
        } => {
            let mut detail = Vec::new();
            if let Some(pages) = pages {
                detail.push(format!(
                    "{pages} page{}",
                    if *pages == 1 { "" } else { "s" }
                ));
            }
            detail.push(crate::util::bytes(media.size));
            attachment(
                ui,
                view,
                message,
                media,
                Icon::FileText,
                file_name,
                &detail.join(" · "),
                width,
                actions,
            );
            caption.as_ref().and_then(|caption| {
                rich_body(
                    ui,
                    view,
                    message,
                    caption,
                    width,
                    Some(reserve),
                    Some(width),
                    actions,
                )
            })
        }
        Content::Location {
            latitude,
            longitude,
            name,
            address,
        } => {
            let title = name
                .clone()
                .unwrap_or_else(|| crate::i18n::gettext(view.locale, "Location").into_owned());
            // A spot sent from here names no place, so the coordinates take the
            // line an address would.
            let detail = address.clone().or_else(|| {
                Some(
                    Spot {
                        latitude: *latitude,
                        longitude: *longitude,
                    }
                    .text(),
                )
            });
            location_card(
                ui,
                view,
                message,
                &message.id,
                &title,
                detail,
                None,
                (*latitude, *longitude),
                actions,
            );
            None
        }
        Content::LiveLocation {
            latitude,
            longitude,
            accuracy_m,
            speed_mps,
            sequence,
            updated,
            newer_on_phone,
            ..
        } => {
            let over = message
                .content
                .live_location_over(message.timestamp, view.now);
            if !over {
                let until_expiry =
                    (message.timestamp + crate::model::LIVE_LOCATION_LIMIT - view.now).max(1);
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs(until_expiry as u64));
            }
            let title = if over {
                crate::i18n::gettext(view.locale, "Live location ended")
            } else {
                crate::i18n::gettext(view.locale, "Live location")
            };
            let detail = (!over).then(|| {
                // An absolute time stays true without repainting.
                let at = if *updated > 0 {
                    *updated
                } else {
                    message.timestamp
                };
                let mut meta = vec![
                    crate::i18n::gettext(view.locale, "Updated {time}")
                        .replace("{time}", &crate::util::clock(at)),
                ];
                if let Some(speed) = speed_mps.filter(|speed| *speed >= 0.5) {
                    meta.push(format!("{:.0} km/h", speed * 3.6));
                }
                if let Some(accuracy) = accuracy_m {
                    meta.push(format!("±{accuracy} m"));
                }
                meta.join(" · ")
            });
            // The phone holds the positions that follow this one: WhatsApp
            // keeps live locations off linked devices, so say where the rest
            // of the share is instead of letting the pin look live.
            let note = (!over && *newer_on_phone).then(|| {
                crate::i18n::gettext(view.locale, "Open WhatsApp on your phone to follow it.")
                    .into_owned()
            });
            // Each position gets its own preview, as a later one replaces the first.
            let key = format!("{}-{sequence}-{updated}", message.id);
            location_card(
                ui,
                view,
                message,
                &key,
                &title,
                detail,
                note,
                (*latitude, *longitude),
                actions,
            );
            None
        }
        Content::Contact {
            display_name,
            vcard,
        } => {
            let icon = |ui: &mut egui::Ui| {
                theme::icon(ui, Icon::Contact, 18.0, palette.accent);
            };
            let details = shared_contact_details(vcard, display_name);
            mirrored_row(ui, own, icon, |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 5.0;
                    let name = details
                        .as_ref()
                        .map_or(display_name.as_str(), |contact| contact.name.as_str());
                    widgets::rich_text(ui, name, theme::medium(14.0), palette.text);
                    match details.as_ref() {
                        Some(SharedContact {
                            account: Some(account),
                            first_name,
                            ..
                        }) => {
                            let id = format!("{account}@s.whatsapp.net");
                            ui.horizontal(|ui| {
                                if theme::pill_button(
                                    ui,
                                    &palette,
                                    crate::i18n::gettext(view.locale, "Chat").as_ref(),
                                    true,
                                )
                                .clicked()
                                {
                                    actions.push(Action::StartChat {
                                        id: id.clone(),
                                        name: name.to_owned(),
                                        dismiss_dialog: true,
                                    });
                                }
                                if !view.contacts.contains_key(&id)
                                    && theme::pill_button(
                                        ui,
                                        &palette,
                                        crate::i18n::gettext(view.locale, "Add").as_ref(),
                                        false,
                                    )
                                    .clicked()
                                {
                                    // Split where the card's first name ends, so a
                                    // first name of several words stays whole.
                                    let (first, last) =
                                        crate::util::editor_names(name, first_name.as_deref());
                                    actions.push(Action::NewContact {
                                        phone: account.clone(),
                                        first,
                                        last,
                                        to_phone: None,
                                    });
                                }
                            });
                        }
                        // Without an account there is nothing to open, so
                        // the number stays readable, as it was.
                        Some(contact) => {
                            theme::text(
                                ui,
                                &contact.number,
                                theme::regular(12.5),
                                palette.secondary,
                            );
                        }
                        None => {}
                    }
                });
            });
            None
        }
        Content::StickerPack {
            name,
            publisher,
            count,
            caption,
        } => {
            let icon = |ui: &mut egui::Ui| {
                theme::icon(ui, Icon::Sticker, 20.0, palette.accent);
            };
            mirrored_row(ui, own, icon, |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    widgets::rich_text(ui, name, theme::medium(14.0), palette.text);
                    let stickers =
                        crate::i18n::ngettext(view.locale, "{} sticker", "{} stickers", *count)
                            .replace("{}", &count.to_string());
                    let detail = if publisher.trim().is_empty() {
                        stickers
                    } else {
                        format!("{publisher} · {stickers}")
                    };
                    widgets::rich_text(ui, &detail, theme::regular(12.5), palette.secondary);
                    if let Some(caption) = caption {
                        widgets::rich_text(ui, caption, theme::regular(13.5), palette.text);
                    }
                    if theme::link(
                        ui,
                        crate::i18n::gettext(view.locale, "View stickers"),
                        theme::regular(12.5),
                        palette.link,
                    )
                    .clicked()
                    {
                        actions.push(Action::ViewStickerPack(message.id.clone()));
                    }
                });
            });
            None
        }
        Content::Poll { .. } => {
            super::polls::ballot(
                ui,
                &palette,
                view.locale,
                message,
                width,
                view.connected,
                view.poll_voting
                    .contains(&(message.chat.clone(), message.id.clone())),
                actions,
            );
            None
        }
        Content::Call {
            video,
            status,
            duration,
            group,
            participants,
        } => {
            let entry = super::calls::Entry {
                from_me: own,
                video: *video,
                status: *status,
                duration: *duration,
                group: *group,
                participants: *participants,
            };
            let tint = if status.is_bad() {
                palette.danger
            } else {
                palette.accent
            };
            call_entry(ui, &palette, &entry, view.locale, tint, width, reserve)
        }
        Content::Revoked => {
            mirrored_row(
                ui,
                own,
                |ui| {
                    theme::icon(ui, Icon::Ban, 14.0, palette.dim);
                },
                |ui| {
                    theme::text(
                        ui,
                        "This message was deleted",
                        theme::regular(13.5),
                        palette.secondary,
                    );
                },
            );
            None
        }
        Content::PhoneOnly {
            live_location: true,
            ..
        } => {
            mirrored_row(
                ui,
                own,
                |ui| {
                    theme::icon(ui, Icon::MapPin, 18.0, palette.accent);
                },
                |ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        widgets::rich_text(
                            ui,
                            &crate::i18n::gettext(view.locale, "Live location"),
                            theme::medium(14.0),
                            palette.text,
                        );
                        widgets::rich_text(
                            ui,
                            &crate::i18n::gettext(
                                view.locale,
                                "Open WhatsApp on your phone to follow it.",
                            ),
                            theme::regular(12.5),
                            palette.secondary,
                        );
                    });
                },
            );
            None
        }
        Content::PhoneOnly {
            view_once, once, ..
        } => {
            use crate::i18n::gettext;
            use crate::model::OnceMedia;
            let text = match once {
                Some(OnceMedia::Photo) => gettext(
                    view.locale,
                    "View once photo. For your privacy, it opens only on your phone.",
                ),
                Some(OnceMedia::Video) => gettext(
                    view.locale,
                    "View once video. For your privacy, it opens only on your phone.",
                ),
                Some(OnceMedia::Voice) => gettext(
                    view.locale,
                    "View once voice message. For your privacy, it opens only on your phone.",
                ),
                Some(OnceMedia::Audio) => gettext(
                    view.locale,
                    "View once audio. For your privacy, it opens only on your phone.",
                ),
                None if *view_once => gettext(
                    view.locale,
                    "View once message. For your privacy, it opens only on your phone.",
                ),
                None => gettext(view.locale, "This message can only be seen on your phone."),
            };
            mirrored_row(
                ui,
                own,
                |ui| {
                    theme::icon(ui, Icon::Smartphone, 14.0, palette.dim);
                },
                |ui| {
                    theme::text(ui, text, theme::regular(13.5), palette.secondary);
                },
            );
            None
        }
        Content::Unsupported { what } => {
            mirrored_row(
                ui,
                own,
                |ui| {
                    theme::icon(ui, Icon::CircleAlert, 14.0, palette.dim);
                },
                |ui| {
                    theme::text(
                        ui,
                        format!("Unsupported: {what}"),
                        theme::regular(13.5),
                        palette.secondary,
                    );
                },
            );
            None
        }
    }
}

/// Horizontal, independently cached cards with overlaid previous/next controls.
/// Keep a partial next card visible and preserve native wheel/touchpad scrolling.
fn carousel(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    card: &crate::model::InteractiveCard,
    width: f32,
    actions: &mut Vec<Action>,
) {
    if !card.body.is_empty() {
        rich_body(
            ui,
            view,
            message,
            &card.body,
            width,
            None,
            Some(width),
            actions,
        );
        ui.add_space(4.0);
    }
    // Reserve a glimpse of the next card only when there is another card.
    let inset = if card.carousel.len() > 1 { 36.0 } else { 20.0 };
    let card_width = CAROUSEL_CARD_WIDTH.min((width - inset).max(140.0));
    let id = bubble_id(&message.chat, &message.id);
    for direction in [-1, 1] {
        ui.ctx().data_mut(|data| {
            data.remove::<Rect>(id.with(("carousel-arrow", direction)));
        });
    }
    let output = egui::ScrollArea::horizontal()
        .id_salt(("carousel", &message.chat, &message.id))
        .max_width(width)
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .auto_shrink([false, true])
        .show_viewport(ui, |ui, viewport| {
            let strip = ui
                .with_layout(Layout::left_to_right(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.x = CAROUSEL_GAP;
                    for (index, child) in card.carousel.iter().enumerate() {
                        let row = carousel_row(message, index, child);
                        ui.push_id(("carousel-card", index), |ui| {
                            let response = Frame::new()
                                .fill(if message.from_me {
                                    view.palette.bubble_out
                                } else {
                                    view.palette.bubble_in
                                })
                                .corner_radius(10)
                                .inner_margin(Margin {
                                    left: 10,
                                    right: 10,
                                    top: 6,
                                    bottom: 5,
                                })
                                .show(ui, |ui| {
                                    ui.set_width(card_width);
                                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                                        ui.set_width(card_width);
                                        if child.image.is_some() {
                                            carousel_picture(
                                                ui, view, message, index, child, card_width,
                                                actions,
                                            );
                                        }
                                        if !child.body.is_empty() {
                                            rich_body(
                                                ui,
                                                view,
                                                &row,
                                                &child.body,
                                                card_width,
                                                None,
                                                Some(card_width),
                                                actions,
                                            );
                                        }
                                        interactive_buttons(
                                            ui, view, &row, child, card_width, actions,
                                        );
                                    });
                                });
                            ui.ctx().data_mut(|data| {
                                data.insert_temp(
                                    bubble_id(&message.chat, &message.id)
                                        .with(("carousel-card", index)),
                                    response.response.rect,
                                )
                            });
                        });
                    }
                })
                .response
                .rect;
            let limit = (strip.width() - viewport.width()).max(0.0);
            let visible = Rect::from_min_size(
                pos2(strip.left() + viewport.left(), strip.top()),
                vec2(viewport.width(), strip.height()),
            );
            if limit > 1.0 {
                for direction in [-1, 1] {
                    let available = if direction < 0 {
                        viewport.left() > 1.0
                    } else {
                        viewport.left() < limit - 1.0
                    };
                    if !available {
                        continue;
                    }
                    let x = if direction < 0 {
                        visible.left() + 26.0
                    } else {
                        visible.right() - 26.0
                    };
                    let rect =
                        Rect::from_center_size(pos2(x, visible.center().y), Vec2::splat(40.0));
                    let arrow = carousel_arrow(
                        ui,
                        &view.palette,
                        rect,
                        id.with(("carousel-arrow", direction)),
                        direction < 0,
                    );
                    if arrow.clicked() {
                        let step = card_width + 20.0 + CAROUSEL_GAP;
                        let target = (viewport.left() + direction as f32 * step).clamp(0.0, limit);
                        // Issue this inside the horizontal ScrollArea so the
                        // conversation's vertical scroll cannot consume it.
                        ui.scroll_with_delta(vec2(viewport.left() - target, 0.0));
                    }
                }
            }
            visible
        });
    ui.ctx()
        .data_mut(|data| data.insert_temp(id.with("carousel-viewport"), output.inner));
}

/// Stands in for one carousel card when drawing its text and actions. Only the
/// parent's identity is kept: cloning the whole message would copy every
/// card's thumbnail per card each frame, and the parent's quote and reactions
/// would repeat on each card in copied transcripts.
fn carousel_row(message: &Message, index: usize, card: &crate::model::InteractiveCard) -> Message {
    Message {
        id: format!("{}-card-{index}", message.id),
        chat: message.chat.clone(),
        sender: message.sender.clone(),
        sender_name: message.sender_name.clone(),
        from_me: message.from_me,
        timestamp: message.timestamp,
        history_order: message.history_order,
        content: Content::Interactive {
            text: card.body.clone(),
            card: None,
        },
        status: message.status,
        delivered_at: None,
        read_at: None,
        quoted: None,
        reactions: Vec::new(),
        edited: message.edited,
        mentions: message.mentions.clone(),
        forwarded: false,
        thumbnail: None,
    }
}

fn carousel_arrow(
    ui: &mut egui::Ui,
    palette: &Palette,
    rect: Rect,
    id: egui::Id,
    previous: bool,
) -> egui::Response {
    let label = if previous {
        "Previous card"
    } else {
        "Next card"
    };
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    ui.ctx().data_mut(|data| data.insert_temp(id, rect));
    theme::reveal_focus(&response);
    if ui.is_rect_visible(rect) {
        let fill = if response.hovered() || response.has_focus() {
            palette.surface_hover
        } else {
            palette.overlay
        };
        ui.painter()
            .circle_filled(rect.center() + vec2(0.0, 2.0), 21.0, palette.shadow);
        ui.painter().circle_filled(rect.center(), 20.0, fill);
        ui.painter().circle_stroke(
            rect.center(),
            19.5,
            Stroke::new(
                1.0,
                if response.has_focus() {
                    palette.link
                } else {
                    palette.outline
                },
            ),
        );
        theme::paint_icon(
            ui,
            if previous {
                Icon::ChevronLeft
            } else {
                Icon::ChevronRight
            },
            rect,
            if response.is_pointer_button_down_on() {
                21.0
            } else {
                23.0
            },
            palette.text,
        );
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label)
}

/// Draws WhatsApp's media-album items as one compact grid with one batch
/// action, and returns where each member's tile was drawn.
fn album_content(
    ui: &mut egui::Ui,
    view: &View<'_>,
    messages: &[Message],
    width: f32,
    actions: &mut Vec<Action>,
) -> Vec<(Rect, String)> {
    let gap = 3.0;
    let columns = 2;
    let tile = ((width - gap * (columns - 1) as f32) / columns as f32).max(80.0);
    let mut tiles = Vec::with_capacity(messages.len());
    for (row, chunk) in messages.chunks(columns).enumerate() {
        // An own bubble lays out right to left; the grid keeps reading order.
        ui.allocate_ui_with_layout(
            vec2(tile * columns as f32 + gap, tile),
            Layout::left_to_right(Align::Min),
            |ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (column, message) in chunk.iter().enumerate() {
                    if let Some(rect) =
                        album_tile(ui, view, message, vec2(tile, tile), row, column, actions)
                    {
                        tiles.push((rect, message.id.clone()));
                    }
                }
            },
        );
    }

    for message in messages {
        let caption = match &message.content {
            Content::Image { caption, .. } | Content::Video { caption, .. } => caption.as_deref(),
            _ => None,
        };
        if let Some(caption) = caption.filter(|caption| !caption.is_empty()) {
            let _ = rich_body(
                ui,
                view,
                message,
                caption,
                width,
                None,
                Some(width),
                actions,
            );
        } else if let Some(copy_rows) = view.copy_rows {
            copy_rows
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(transcript_row(view, message, String::new(), Vec::new()));
        }
    }

    let missing: Vec<&Message> = messages
        .iter()
        .filter(|message| {
            message
                .content
                .media()
                .is_some_and(|media| media.path.is_none())
        })
        .collect();
    let files: Vec<(PathBuf, String)> = messages
        .iter()
        .filter_map(|message| {
            let path = message.content.media()?.path.as_ref()?;
            Some((path.clone(), attachment_name(&message.content, path)))
        })
        .collect();
    if !files.is_empty()
        && theme::pill_button(
            ui,
            &view.palette,
            &crate::i18n::gettext(view.locale, "Save all…"),
            true,
        )
        .clicked()
    {
        actions.push(Action::SaveAttachments { files });
    }
    if !missing.is_empty() {
        let downloading = missing.iter().all(|message| {
            message
                .content
                .media()
                .is_some_and(|media| matches!(media.state, MediaState::Downloading))
        });
        if theme::pill_button(
            ui,
            &view.palette,
            &crate::i18n::gettext(
                view.locale,
                if downloading {
                    "Downloading…"
                } else {
                    "Download all"
                },
            ),
            !downloading,
        )
        .clicked()
        {
            for message in missing {
                if let Some(media) = message.content.media()
                    && !matches!(media.state, MediaState::Downloading)
                {
                    actions.push(Action::Download {
                        manual: true,
                        card: None,
                        chat: message.chat.clone(),
                        message: message.id.clone(),
                    });
                }
            }
        }
    }
    tiles
}

/// Draws one album member with its own download, selection and activation
/// targets, and returns its rect.
fn album_tile(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    size: Vec2,
    row: usize,
    column: usize,
    actions: &mut Vec<Action>,
) -> Option<Rect> {
    let media = message.content.media()?;
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let kind = if matches!(message.content, Content::Image { .. }) {
        "Photo"
    } else {
        "Video"
    };
    let state = match (&media.path, &media.state) {
        (Some(_), _) => "Open".to_owned(),
        (None, MediaState::Downloading) => "Downloading".to_owned(),
        (None, MediaState::Failed(error)) => format!("Failed: {error}"),
        (None, MediaState::Idle) => "Download".to_owned(),
    };
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{kind}, {state}"))
    });
    theme::reveal_focus(&response);
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, 5.0, view.palette.surface);
        let uri = media
            .path
            .as_ref()
            .filter(|_| matches!(message.content, Content::Image { .. }))
            .map(|path| {
                let uri = crate::util::image_uri(path);
                crate::image_cache::touch(ui.ctx(), &uri);
                uri
            })
            .or_else(|| {
                message.thumbnail.as_deref().map(|bytes| {
                    thumbnail_uri(
                        ui.ctx(),
                        &message.chat,
                        &format!("{}-album-{row}-{column}", message.id),
                        bytes,
                    )
                })
            });
        if let Some(uri) = uri {
            let image = egui::Image::new(uri);
            let dimensions = match image.load_for_size(ui.ctx(), size) {
                Ok(egui::load::TexturePoll::Ready { texture }) => Some(texture.size),
                Ok(egui::load::TexturePoll::Pending { .. }) => Some(vec2(
                    media.width.unwrap_or(1) as f32,
                    media.height.unwrap_or(1) as f32,
                )),
                Err(_) => None,
            };
            if let Some(dimensions) = dimensions {
                let ratio = dimensions.x / dimensions.y.max(1.0);
                let target = size.x / size.y.max(1.0);
                let uv = if ratio > target {
                    vec2(target / ratio, 1.0)
                } else {
                    vec2(1.0, ratio / target)
                };
                image
                    .uv(Rect::from_center_size(pos2(0.5, 0.5), uv))
                    .fit_to_exact_size(size)
                    .corner_radius(5.0)
                    .paint_at(ui, rect);
            }
        }
        if matches!(message.content, Content::Video { .. }) {
            theme::paint_icon(ui, Icon::Play, rect, 30.0, Color32::WHITE);
        }
        if media.path.is_none() {
            let disc = Rect::from_center_size(rect.center(), Vec2::splat(42.0));
            ui.painter()
                .circle_filled(disc.center(), 21.0, Color32::from_black_alpha(130));
            match media.state {
                MediaState::Downloading => theme::paint_spinner(ui, disc, 20.0, Color32::WHITE),
                MediaState::Failed(_) => {
                    theme::paint_icon(ui, Icon::CircleAlert, disc, 20.0, Color32::WHITE)
                }
                MediaState::Idle => {
                    theme::paint_icon(ui, Icon::Download, disc, 20.0, Color32::WHITE)
                }
            }
        }
        if not_sent(message) {
            let starred = view.starred.is_some_and(|ids| ids.contains(&message.id));
            let pinned = view.pins.is_some_and(|ids| ids.contains(&message.id));
            footer_over_picture(ui, view, message, rect, starred, pinned, actions);
        }
    }
    // A motion photo in an album keeps its own badge and clip.
    if let Content::Image {
        motion: Some(motion),
        ..
    } = &message.content
    {
        motion_clip(ui, view, message, motion, rect, actions);
    }
    theme::focus_outline(ui, response.id, rect, 5.0);
    // Each member keeps its own failure: the others stay usable, and a click
    // on this one retries it.
    let response = if let MediaState::Failed(error) = &media.state {
        response.on_hover_text(format!("{error} · Click to retry"))
    } else {
        response
    };
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        if let Some(path) = &media.path {
            // A photo or a clip the viewer plays opens there, over the
            // chat's album; anything else goes to the system's own viewer.
            actions.push(match crate::image_preview::open_target(path, true) {
                crate::image_preview::OpenTarget::Preview => Action::PreviewImage {
                    path: path.clone(),
                    chat: message.chat.clone(),
                    message: message.id.clone(),
                    card: None,
                },
                crate::image_preview::OpenTarget::External => Action::OpenFile(path.clone()),
            });
        } else if !matches!(media.state, MediaState::Downloading) {
            actions.push(Action::Download {
                manual: true,
                card: None,
                chat: message.chat.clone(),
                message: message.id.clone(),
            });
        }
    }
    ui.ctx()
        .data_mut(|data| data.insert_temp(album_tile_id(&view.chat.id, &message.id), rect));
    Some(rect)
}

/// Where an album member's tile was drawn, for interaction tests.
pub fn album_tile_id(chat: &str, message: &str) -> egui::Id {
    bubble_id(chat, message).with("album-tile")
}

/// A carousel uses a short image preview; opening it shows the full picture.
fn carousel_picture(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    index: usize,
    card: &crate::model::InteractiveCard,
    width: f32,
    actions: &mut Vec<Action>,
) {
    let Some(media) = &card.image else { return };
    let size = vec2(width, width * 0.56);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let label = match (&media.path, &media.state) {
        (Some(_), _) => "Open card image",
        (None, MediaState::Failed(_)) => "Retry card image download",
        (None, MediaState::Downloading) => "Downloading card image",
        (None, MediaState::Idle) => "Download card image",
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    // Tab can land on a card scrolled out of the carousel's viewport.
    theme::reveal_focus(&response);
    let visible = ui.is_rect_visible(rect);
    if visible {
        ui.painter().rect_filled(rect, 6.0, view.palette.surface);
        let uri = media
            .path
            .as_ref()
            .map(|path| {
                let uri = crate::util::image_uri(path);
                crate::image_cache::touch(ui.ctx(), &uri);
                uri
            })
            .or_else(|| {
                card.thumbnail.as_deref().map(|bytes| {
                    thumbnail_uri(
                        ui.ctx(),
                        &message.chat,
                        &format!("{}-card-{index}", message.id),
                        bytes,
                    )
                })
            });
        if let Some(uri) = uri {
            let image = egui::Image::new(uri);
            let dimensions = match image.load_for_size(ui.ctx(), size) {
                Ok(egui::load::TexturePoll::Ready { texture }) => Some(texture.size),
                Ok(egui::load::TexturePoll::Pending { .. }) => Some(vec2(
                    media.width.unwrap_or(16) as f32,
                    media.height.unwrap_or(9) as f32,
                )),
                Err(_) => None,
            };
            if let Some(dimensions) = dimensions {
                let ratio = dimensions.x / dimensions.y.max(1.0);
                let target = size.x / size.y.max(1.0);
                let uv_size = if ratio > target {
                    vec2(target / ratio, 1.0)
                } else {
                    vec2(1.0, ratio / target)
                };
                image
                    .uv(Rect::from_center_size(pos2(0.5, 0.5), uv_size))
                    .fit_to_exact_size(size)
                    .corner_radius(6.0)
                    .paint_at(ui, rect);
            } else if media.path.is_some() {
                theme::paint_icon(ui, Icon::CircleAlert, rect, 24.0, view.palette.danger);
                ui.painter().text(
                    rect.center() + vec2(0.0, 24.0),
                    Align2::CENTER_CENTER,
                    "Could not display this picture. Click to open it.",
                    theme::regular(11.5),
                    view.palette.secondary,
                );
            }
        }
        if media.path.is_none() {
            let disc = Rect::from_center_size(rect.center(), Vec2::splat(42.0));
            ui.painter()
                .circle_filled(disc.center(), 21.0, Color32::from_black_alpha(130));
            match &media.state {
                MediaState::Downloading => theme::paint_spinner(ui, disc, 20.0, Color32::WHITE),
                MediaState::Failed(_) => {
                    theme::paint_icon(ui, Icon::CircleAlert, disc, 20.0, Color32::WHITE);
                    ui.painter().text(
                        rect.center() + vec2(0.0, 34.0),
                        Align2::CENTER_CENTER,
                        crate::i18n::gettext(view.locale, "Download failed. Click to retry."),
                        theme::regular(11.5),
                        Color32::WHITE,
                    );
                }
                MediaState::Idle => {
                    theme::paint_icon(ui, Icon::Download, disc, 20.0, Color32::WHITE)
                }
            }
        }
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect.shrink(1.0),
                6.0,
                Stroke::new(theme::FOCUS_STROKE_WIDTH, view.palette.link),
                egui::StrokeKind::Inside,
            );
        }
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let clicked = response.clicked();
    if let MediaState::Failed(error) = &media.state {
        response.on_hover_text(format!("{error} · Click to retry"));
    }
    if let Some(path) = &media.path {
        if clicked {
            // A card's picture opens in the viewer, among the chat's album,
            // as any other photo does.
            actions.push(match crate::image_preview::open_target(path, true) {
                crate::image_preview::OpenTarget::Preview => Action::PreviewImage {
                    path: path.clone(),
                    chat: message.chat.clone(),
                    message: message.id.clone(),
                    card: Some(index),
                },
                crate::image_preview::OpenTarget::External => Action::OpenFile(path.clone()),
            });
        }
    } else if !matches!(media.state, MediaState::Downloading)
        && (clicked
            || (visible
                && auto_download_allowed(media, false, view.auto_download)
                && matches!(media.state, MediaState::Idle)))
    {
        actions.push(Action::Download {
            manual: clicked,
            chat: message.chat.clone(),
            message: message.id.clone(),
            card: Some(index),
        });
    }
}

/// Whether an interactive button can be used now: links and copy codes
/// always, replies and lists only while connected, in a chat that takes
/// messages, on a message from someone else that was not edited.
fn interactive_enabled(
    view: &View<'_>,
    message: &Message,
    button: &crate::model::InteractiveButton,
) -> bool {
    use crate::model::InteractiveAction;
    button.url.is_some()
        || match &button.action {
            InteractiveAction::Copy(_) => true,
            InteractiveAction::Reply | InteractiveAction::Select(_) => {
                view.connected
                    && view.chat.can_send()
                    && !message.from_me
                    && !message.edited
                    && !view
                        .interactive_pending
                        .contains(&(message.chat.clone(), message.id.clone()))
            }
            InteractiveAction::Unavailable => false,
        }
}

/// Why [`interactive_enabled`] refuses a button.
fn interactive_reason(
    view: &View<'_>,
    message: &Message,
    button: &crate::model::InteractiveButton,
) -> ButtonBlocker {
    use crate::model::InteractiveAction;
    let sends = matches!(
        button.action,
        InteractiveAction::Reply | InteractiveAction::Select(_)
    );
    if sends
        && view
            .interactive_pending
            .contains(&(message.chat.clone(), message.id.clone()))
    {
        ButtonBlocker::Sending
    } else if sends && !view.connected {
        ButtonBlocker::Offline
    } else if sends && !view.chat.can_send() {
        ButtonBlocker::ReadOnly
    } else if sends && message.from_me {
        ButtonBlocker::OwnMessage
    } else {
        ButtonBlocker::Elsewhere
    }
}

/// Why a card's button is unavailable. Each reason has its hover text and,
/// in its own match arm, its translated words for a screen reader, so a new
/// reason cannot go unspoken or untranslated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ButtonBlocker {
    Sending,
    Offline,
    ReadOnly,
    OwnMessage,
    Elsewhere,
}

impl ButtonBlocker {
    #[cfg(test)]
    const ALL: [Self; 5] = [
        Self::Sending,
        Self::Offline,
        Self::ReadOnly,
        Self::OwnMessage,
        Self::Elsewhere,
    ];

    /// As the bubble's hover text shows it.
    fn shown(self) -> &'static str {
        match self {
            Self::Sending => "Sending reply…",
            Self::Offline => "Connect to WhatsApp to reply",
            Self::ReadOnly => "This conversation is read-only",
            Self::OwnMessage => "Reply options are for the recipient",
            Self::Elsewhere => "Open this option in WhatsApp Web or on your phone",
        }
    }

    /// As the keyboard menu speaks it, translated.
    fn said(self, locale: crate::i18n::Locale) -> String {
        use crate::i18n::gettext;
        match self {
            Self::Sending => gettext(locale, "Sending reply…"),
            Self::Offline => gettext(locale, "Connect to WhatsApp to reply"),
            Self::ReadOnly => gettext(locale, "This conversation is read-only"),
            Self::OwnMessage => gettext(locale, "Reply options are for the recipient"),
            Self::Elsewhere => gettext(locale, "Open this option in WhatsApp Web or on your phone"),
        }
        .into_owned()
    }
}

/// What using an interactive button does, for a click and for the message
/// menu alike. `None` for a button with nothing to do here.
fn interactive_action(
    message: &Message,
    index: usize,
    button: &crate::model::InteractiveButton,
) -> Option<Action> {
    use crate::model::InteractiveAction;
    if let Some(url) = &button.url {
        return Some(Action::OpenUrl(url.clone()));
    }
    match &button.action {
        InteractiveAction::Reply => Some(Action::ReplyInteractive {
            chat: message.chat.clone(),
            message: message.id.clone(),
            button: index,
            choice: None,
        }),
        InteractiveAction::Copy(code) => Some(Action::CopyText(code.clone())),
        InteractiveAction::Select(_) => {
            Some(Action::ShowDialog(crate::model::Dialog::InteractiveList {
                chat: message.chat.clone(),
                message: message.id.clone(),
                button: index,
            }))
        }
        InteractiveAction::Unavailable => None,
    }
}

/// Full-width action rows below the message timestamp, matching business cards.
/// Show only executable actions as active, with the same keyboard path as clicks.
fn interactive_buttons(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    card: &crate::model::InteractiveCard,
    width: f32,
    actions: &mut Vec<Action>,
) {
    use crate::model::InteractiveAction;
    let palette = &view.palette;
    let spacing = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 0.0;
    for (index, button) in card.buttons.iter().enumerate() {
        let enabled = interactive_enabled(view, message, button);
        // Muted/link colours target incoming surfaces; outgoing tinted bubbles
        // need the main foreground to keep small labels readable in both themes.
        let color = if message.from_me {
            palette.text
        } else if enabled {
            palette.link
        } else {
            palette.secondary
        };
        let icon = if button.url.is_some() {
            Some(Icon::ExternalLink)
        } else {
            match button.action {
                InteractiveAction::Reply => None,
                InteractiveAction::Select(_) => Some(Icon::ListChecks),
                InteractiveAction::Copy(_) => Some(Icon::Copy),
                InteractiveAction::Unavailable => Some(Icon::Smartphone),
            }
        };
        let line = widgets::line(
            ui,
            &button.label,
            theme::medium(14.0),
            color,
            (width - 48.0).max(1.0),
            2,
        );
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, _) = ui.allocate_exact_size(
            vec2(width, (line.size().y + 22.0).max(44.0)),
            Sense::hover(),
        );
        // Action rows belong to the card itself, including its side padding.
        // The final row follows the bubble's bottom corners and margin.
        let last = index + 1 == card.buttons.len() && !card.needs_phone;
        let row_rect = Rect::from_min_max(
            rect.min - vec2(10.0, 0.0),
            rect.max + vec2(10.0, if last { 5.0 } else { 0.0 }),
        );
        let corners = CornerRadius {
            sw: if last { 10 } else { 0 },
            se: if last { 10 } else { 0 },
            ..CornerRadius::ZERO
        };
        let response = ui.interact(
            row_rect,
            bubble_id(&message.chat, &message.id).with(("interactive-action", index)),
            sense,
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &button.label)
        });
        theme::reveal_focus(&response);
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                bubble_id(&message.chat, &message.id).with(("interactive-button", index)),
                row_rect,
            )
        });
        if ui.is_rect_visible(rect) {
            if enabled && (response.hovered() || response.has_focus()) {
                ui.painter().rect_filled(
                    row_rect,
                    corners,
                    palette
                        .text
                        .gamma_multiply(if response.is_pointer_button_down_on() {
                            0.08
                        } else {
                            0.04
                        }),
                );
            }
            ui.painter().hline(
                row_rect.x_range(),
                rect.top(),
                Stroke::new(1.0, palette.secondary.gamma_multiply(0.2)),
            );
            if response.has_focus() {
                ui.painter().rect_stroke(
                    rect.shrink(2.0),
                    4.0,
                    Stroke::new(1.0, palette.link),
                    egui::StrokeKind::Inside,
                );
            }
            let inset = if icon.is_some() { 22.0 } else { 0.0 };
            let content_width = line.size().x + inset;
            let left = rect.center().x - content_width / 2.0;
            if let Some(icon) = icon {
                theme::paint_icon(
                    ui,
                    icon,
                    Rect::from_center_size(
                        egui::pos2(left + 7.0, rect.center().y),
                        Vec2::splat(14.0),
                    ),
                    14.0,
                    color,
                );
            }
            line.paint(
                ui,
                egui::pos2(left + inset, rect.center().y - line.size().y / 2.0),
                color,
            );
        }
        if !enabled {
            let reason = interactive_reason(view, message, button).shown();
            response.on_hover_text(format!("{}\n{reason}", button.label));
        } else if let Some(url) = &button.url {
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(format!("{}\n{url}", button.label))
                .clicked()
                && let Some(action) = interactive_action(message, index, button)
            {
                actions.push(action);
            }
        } else {
            let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
            let response = match &button.action {
                InteractiveAction::Reply => response.on_hover_text(
                    crate::i18n::gettext(view.locale, "Send reply: {label}")
                        .replace("{label}", &button.label),
                ),
                InteractiveAction::Copy(_) => {
                    response.on_hover_text(crate::i18n::gettext(view.locale, "Copy code"))
                }
                InteractiveAction::Select(_) | InteractiveAction::Unavailable => response,
            };
            if response.clicked()
                && let Some(action) = interactive_action(message, index, button)
            {
                actions.push(action);
            }
        }
    }
    if card.needs_phone {
        ui.add_space(6.0);
        widgets::rich_text(
            ui,
            "More content in WhatsApp Web or on your phone",
            theme::regular(12.0),
            palette.secondary,
        );
    }
    ui.spacing_mut().item_spacing.y = spacing;
}

/// Draws formatted message text. `reserve` leaves footer space on the last
/// line. `span` sets a minimum left-aligned row width for text below cards.
#[allow(clippy::too_many_arguments)]
fn rich_body(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    text: &str,
    width: f32,
    reserve: Option<f32>,
    span: Option<f32>,
    actions: &mut Vec<Action>,
) -> Option<Rect> {
    let palette = view.palette;
    let mentions = mentions_of(view, message);
    let style = markup::Style {
        size: BODY_SIZE,
        color: palette.text,
        secondary: palette.secondary,
        link: palette.link,
        mention: palette.accent,
    };
    let laid = markup::layout(ui, text, &mentions, &style, width);
    let rtl = crate::bidi::message_rtl(text);
    let last_row = laid.galley.rows.last().map_or(0.0, |row| row.row.size.x);
    // Right-aligned text ends at the block's edge. Like official WhatsApp,
    // only a single line keeps the time beside it; otherwise it gets a row.
    let single = laid.galley.rows.len() == 1 && span.is_none();
    let inline = reserve.filter(|reserve| last_row + 8.0 + reserve <= width && (!rtl || single));
    let size = laid.galley.size();
    let mut allocation = match inline {
        Some(reserve) => vec2(size.x.max(last_row + 8.0 + reserve), size.y),
        None => size,
    };
    if let Some(span) = span {
        // Span the card width and keep the text left-aligned in own bubbles.
        allocation.x = allocation.x.max(span);
    }
    // Register the body for transcript formatting when copying across messages.
    if let Some(copy_rows) = view.copy_rows {
        copy_rows
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(transcript_row(
                view,
                message,
                laid.galley.text().to_owned(),
                laid.placements().to_vec(),
            ));
    }
    // Click links and drag to select text.
    // Text selection and web links do not need a sequential Tab stop.
    // The surrounding transcript remains available to accessibility readers.
    let (_, rect) = ui.allocate_space(allocation);
    // egui matches selection endpoints to widgets by id every frame and drops
    // the selection when one is missed. A positional auto id shifts whenever
    // a sibling allocates differently (virtualized rows), killing the
    // selection mid-drag; an explicit id keeps the anchor alive.
    let id = bubble_id(&view.chat.id, &message.id);
    // Starting a browser-style sweep on a plain text message should work from
    // anywhere in its bubble, including the footer and generous side padding.
    // Captioned media keeps the narrower expansion so this response cannot
    // cover the picture or another control inside the same bubble.
    let selection_rect = ui
        .ctx()
        .data(|data| data.get_temp::<Rect>(id.with("rect")))
        .map(|bubble| {
            // Virtualized rows can move between frames. Align the cached bubble
            // to this frame's body before expanding its input target, so it
            // cannot cover a neighboring message after a layout change.
            let bubble = ui
                .ctx()
                .data(|data| data.get_temp::<Rect>(id.with("body")))
                .map(|previous| {
                    let mut bubble = bubble.translate(rect.min - previous.min);
                    bubble.max += rect.size() - previous.size();
                    bubble
                })
                .unwrap_or(bubble);
            if matches!(message.content, Content::Text { .. }) {
                let mut target = bubble.union(rect);
                // Quote and preview cards have their own click actions and
                // were registered before the text. Keep their current-frame
                // rectangles outside this later, enlarged input target.
                for kind in ["quote", "preview"] {
                    let present = if kind == "quote" {
                        message.quoted.is_some()
                    } else {
                        matches!(
                            message.content,
                            Content::Text {
                                preview: Some(_),
                                ..
                            }
                        )
                    };
                    if !present {
                        continue;
                    }
                    if let Some(card) = ui.ctx().data(|data| data.get_temp::<Rect>(id.with(kind))) {
                        target.min.y = target.min.y.max(card.bottom());
                    }
                }
                // A group sender's name, drawn above the text this frame,
                // opens their info on a click: keep it outside as well.
                if let Some(name) = ui
                    .ctx()
                    .data(|data| data.get_temp::<Rect>(id.with("sender")))
                {
                    target.min.y = target.min.y.max(name.bottom());
                }
                target
            } else {
                rect.expand2(vec2(10.0, 6.0)).intersect(bubble).union(rect)
            }
        })
        .unwrap_or(rect);
    let response = ui.interact(
        selection_rect,
        id.with("body-text"),
        Sense::CLICK | Sense::DRAG,
    );
    // The expanded target owns pointer input in the padding. Preserve the
    // bubble gestures that lived there before, while double-clicks on actual
    // text still select a word.
    let padding = response
        .interact_pointer_pos()
        .is_some_and(|pointer| !rect.contains(pointer));
    if padding {
        reply_on_double_click(&response, message, actions);
        if response.clicked() && !response.double_clicked() {
            let modifiers = ui.input(|input| input.modifiers);
            if view.selecting {
                actions.push(if modifiers.shift {
                    Action::SelectRange(message.id.clone())
                } else {
                    Action::ToggleSelected(message.id.clone())
                });
            } else if modifiers.command {
                actions.push(Action::SelectMessage(message.id.clone()));
            }
        }
    }
    // Store the body rect for selection tests.
    ui.ctx().data_mut(|data| {
        data.insert_temp(id.with("body"), rect);
    });
    // Keep off-screen selected bodies registered so scrolling does not lose
    // the selection anchor or omit copied text.
    let visible = ui.is_rect_visible(rect);
    let selection_alive = ui.input(|input| input.pointer.primary_down())
        || ui
            .ctx()
            .plugin_opt::<egui::text_selection::LabelSelectionState>()
            .is_some_and(|plugin| plugin.lock().has_selection());
    let origin = if rtl && inline.is_none() {
        pos2(rect.right() - size.x, rect.top())
    } else {
        rect.min
    };
    if visible || selection_alive {
        markup::paint_selectable(ui, &laid, &response, origin, palette.text, visible);
    }
    let message_number = crate::i18n::gettext(view.locale, "Message");
    let copy_number = crate::i18n::gettext(view.locale, "Copy number");
    let menu_labels = [message_number.as_ref(), copy_number.as_ref()];
    for (range, url) in &laid.links {
        let Some(phone) = url.strip_prefix("tel:") else {
            continue;
        };
        let number = phone.strip_prefix('+').unwrap_or(phone);
        let copy_text: String = laid
            .galley
            .text()
            .chars()
            .skip(range.start)
            .take(range.len())
            .collect();
        for (row, bounds) in crate::bidi::char_bounds_by_row(&laid.galley, range.start, range.end) {
            let link = ui
                .interact(
                    bounds.translate(origin.to_vec2()).expand(2.0),
                    bubble_id(&view.chat.id, &message.id).with((
                        "phone-link",
                        range.start,
                        range.end,
                        row,
                    )),
                    Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            link.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), number)
            });
            theme::reveal_focus(&link);
            let link = focus::phone_link(link);
            egui::Popup::menu(&link)
                .id(link.id.with("popup"))
                .width(widgets::menu_width(ui, &menu_labels, true))
                .frame(widgets::menu_frame(&view.palette))
                .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
                .show(|ui| {
                    #[cfg(test)]
                    let message_rect =
                        Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), 28.0));
                    let message_clicked =
                        widgets::menu_item(ui, &view.palette, None, message_number.as_ref());
                    #[cfg(test)]
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(link.id.with("test-message-action"), message_rect);
                    });
                    if message_clicked {
                        actions.push(Action::NewContact {
                            phone: number.to_owned(),
                            first: String::new(),
                            last: String::new(),
                            to_phone: None,
                        });
                    }
                    #[cfg(test)]
                    let copy_rect =
                        Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), 28.0));
                    let copy_clicked = widgets::menu_item(
                        ui,
                        &view.palette,
                        Some(Icon::Copy),
                        copy_number.as_ref(),
                    );
                    #[cfg(test)]
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(link.id.with("test-copy-action"), copy_rect);
                    });
                    if copy_clicked {
                        actions.push(Action::CopyText(copy_text.clone()));
                    }
                });
        }
    }
    if !laid.links.is_empty()
        && let Some(pos) = response.hover_pos()
        && laid
            .galley
            .rows
            .iter()
            .any(|row| Rect::from_min_size(origin + row.pos.to_vec2(), row.row.size).contains(pos))
    {
        let cursor = laid.galley.cursor_from_pos(pos - origin);
        if let Some(url) = laid.link_at(cursor.index.0) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            if !url.starts_with("tel:") && response.clicked() {
                actions.push(Action::OpenUrl(url.to_owned()));
            }
        }
    }
    inline.map(|reserve| {
        Rect::from_min_max(
            pos2(rect.right() - reserve, rect.bottom() - 15.0),
            rect.right_bottom(),
        )
    })
}

/// Link preview with image, title, and description.
fn preview_card(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    preview: &LinkPreview,
    width: f32,
    actions: &mut Vec<Action>,
) {
    let palette = view.palette;
    let thumbnail = message.thumbnail.as_deref();
    let domain = preview
        .url
        .split("://")
        .nth(1)
        .unwrap_or(&preview.url)
        .split('/')
        .next()
        .unwrap_or_default()
        .to_owned();
    let response = Frame::new()
        .fill(palette.window.gamma_multiply(0.35))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::same(8))
        .show(ui, |ui| {
            // Include margins in the settled card width.
            let card_width = (width - 16.0).max(0.0);
            ui.set_width(card_width);
            // Limit text to the space beside the thumbnail and keep it
            // left-aligned in own bubbles.
            let column = (card_width - if thumbnail.is_some() { 72.0 } else { 0.0 }).max(0.0);
            ui.allocate_ui_with_layout(vec2(card_width, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(card_width);
                ui.horizontal(|ui| {
                    if let Some(bytes) = thumbnail {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
                        // Off-screen cards are laid out too; only a visible
                        // one keeps its thumbnail resident.
                        if ui.is_rect_visible(rect) {
                            let uri = thumbnail_uri(ui.ctx(), &message.chat, &message.id, bytes);
                            egui::Image::new(uri)
                                .fit_to_exact_size(rect.size())
                                .corner_radius(4.0)
                                .paint_at(ui, rect);
                        }
                    }
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.set_width(column);
                        if let Some(title) = &preview.title {
                            widgets::rich_text(ui, title, theme::semibold(13.5), palette.text);
                        }
                        if let Some(description) = &preview.description {
                            let line = widgets::line(
                                ui,
                                description,
                                theme::regular(12.5),
                                palette.secondary,
                                ui.available_width(),
                                2,
                            );
                            let (rect, _) = ui.allocate_exact_size(line.size(), Sense::hover());
                            line.paint(ui, rect.min, palette.secondary);
                        }
                        theme::text(ui, &domain, theme::regular(12.0), palette.dim);
                    });
                });
            });
        })
        .response;
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            bubble_id(&view.chat.id, &message.id).with("preview"),
            response.rect,
        );
    });
    let response = ui
        .interact(
            response.rect,
            ui.id().with(("preview", &message.id)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        actions.push(Action::OpenUrl(preview.url.clone()));
    }
}

/// A location as a card: the map preview WhatsApp sent across the top, then
/// a pinned title, an optional detail line, and a link to open the spot.
#[expect(clippy::too_many_arguments)]
fn location_card(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    preview_key: &str,
    title: &str,
    detail: Option<String>,
    note: Option<String>,
    (latitude, longitude): (f64, f64),
    actions: &mut Vec<Action>,
) {
    let palette = view.palette;
    // A bounded, left-aligned layout: own bubbles inherit right-to-left flow,
    // which would otherwise stretch the card and make its width oscillate.
    let card = ui.available_width().min(LOCATION_CARD_WIDTH);
    ui.allocate_ui_with_layout(vec2(card, 0.0), Layout::top_down(Align::Min), |ui| {
        ui.set_width(card);
        ui.spacing_mut().item_spacing.y = 2.0;
        // The preview already marks the spot. Crop the square preview to the
        // card's shape.
        if let Some(bytes) = message.thumbnail.as_deref()
            && !bytes.is_empty()
        {
            let uri = thumbnail_uri(ui.ctx(), &message.chat, preview_key, bytes);
            let height = (card * 0.56).round();
            let crop = (1.0 - height / card) / 2.0;
            ui.add(
                egui::Image::new(uri)
                    .maintain_aspect_ratio(false)
                    .uv(Rect::from_min_max(pos2(0.0, crop), pos2(1.0, 1.0 - crop)))
                    .fit_to_exact_size(Vec2::new(card, height))
                    .corner_radius(8.0),
            );
            ui.add_space(4.0);
        }
        ui.horizontal(|ui| {
            theme::icon(ui, Icon::MapPin, 16.0, palette.accent);
            widgets::rich_text(ui, title, theme::medium(14.0), palette.text);
        });
        if let Some(detail) = detail {
            widgets::rich_text(ui, &detail, theme::regular(12.5), palette.secondary);
        }
        if let Some(note) = note {
            widgets::rich_text(ui, &note, theme::regular(12.5), palette.secondary);
        }
        if theme::link(
            ui,
            crate::i18n::gettext(view.locale, "Open in a map").as_ref(),
            theme::regular(12.5),
            palette.link,
        )
        .clicked()
        {
            let spot = Spot {
                latitude,
                longitude,
            };
            actions.push(Action::OpenUrl(crate::geo::map_url(spot)));
        }
    });
}

pub(crate) fn thumbnail_uri(ctx: &egui::Context, chat: &str, id: &str, bytes: &[u8]) -> String {
    let uri = format!(
        "bytes://thumb-{}-{}",
        chat.chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>(),
        id
    );
    crate::image_cache::include(ctx, &uri, bytes);
    uri
}

/// Default image bounds based on [`CARD_WIDTH`].
const PICTURE_WIDTH: f32 = CARD_WIDTH;
const PICTURE_HEIGHT: f32 = 440.0;
const STICKER_SIDE: f32 = 180.0;
/// Height of a header row, the chat list's and the conversation's alike, so
/// their titles share a centre line.
pub(crate) const HEADER_ROW: f32 = 44.0;

/// Fits an image within bounds without upscaling and with a readable minimum.
fn fit_picture(width: f32, height: f32, max_width: f32, max_height: f32) -> Vec2 {
    let (width, height) = if width > 0.0 && height > 0.0 {
        (width, height)
    } else {
        (4.0, 3.0)
    };
    let max_width = max_width.max(0.0);
    let max_height = max_height.max(0.0);
    let scale = (max_width / width).min(max_height / height).clamp(0.0, 1.0);
    let scale = if width * scale < 120.0 {
        (120.0 / width).min(max_width / width).max(0.0)
    } else {
        scale
    };
    vec2(
        (width * scale).max(0.0),
        (height * scale).max(90.0).max(0.0),
    )
}

/// Fits a sticker to the standard square size.
fn fit_sticker(width: f32, height: f32) -> Vec2 {
    let (width, height) = if width > 0.0 && height > 0.0 {
        (width, height)
    } else {
        (1.0, 1.0)
    };
    let scale = (STICKER_SIDE / width).min(STICKER_SIDE / height);
    vec2(width * scale, height * scale)
}

/// Reserved size for an image or video before and after download.
fn frame_size(
    media: &Media,
    thumbnail_hint: Option<(u32, u32)>,
    max_width: f32,
    max_height: f32,
) -> Vec2 {
    let (w, h) = match (media.width, media.height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => (w as f32, h as f32),
        _ => match thumbnail_hint {
            Some((w, h)) if w > 0 && h > 0 => (w as f32, h as f32),
            _ => (4.0, 3.0),
        },
    };
    fit_picture(w, h, max_width, max_height)
}

/// Draws an image or sticker, using its preview until downloaded. Returns its width.
fn picture(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    media: &Media,
    width: f32,
    sticker: Option<bool>,
    actions: &mut Vec<Action>,
) -> Rect {
    let palette = view.palette;
    let radius = picture_radius(&palette, message);
    let (max_width, max_height) = match sticker {
        Some(_) => (STICKER_SIDE, STICKER_SIDE),
        None => (width.min(PICTURE_WIDTH), PICTURE_HEIGHT),
    };
    if let Some(path) = &media.path {
        // Phones often leave a sticker's animated flag out, so a downloaded
        // file that says it moves is played as well.
        let sticker = sticker.map(|animated| animated || crate::ui::picker::moves(ui.ctx(), path));
        if sticker == Some(true) {
            let size = fit_sticker(
                media.width.unwrap_or(180) as f32,
                media.height.unwrap_or(180) as f32,
            );
            let (rect, response) = ui.allocate_exact_size(size, Sense::click());
            let mut unreadable = false;
            if ui.is_rect_visible(rect) {
                match animation::frame(ui, path, rect, view.animate && response.hovered()) {
                    animation::Frame::Ready(texture) => {
                        ui.painter().image(
                            texture.id(),
                            rect,
                            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    frame => {
                        ui.painter().rect_filled(rect, radius, palette.surface);
                        theme::paint_icon(ui, Icon::Sticker, rect, 32.0, palette.secondary);
                        // A file that is gone or does not decode would stay
                        // this placeholder for good (#603).
                        unreadable = matches!(frame, animation::Frame::Unavailable);
                    }
                }
            }
            if unreadable {
                actions.push(unreadable_media(view, message, path));
            } else if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                actions.push(Action::PreviewSticker(path.clone()));
            }
            return rect;
        }
        // A row that is off screen only reserves its space. Loading the image
        // decodes it and uploads a texture, so it waits until it is scrolled
        // into view, and `image_cache` can release it once it leaves again.
        // The space is the size the picture was last drawn at, when known:
        // a message without dimensions (or with wrong ones) would otherwise
        // take one height on screen and another off it, and a picture across
        // the top edge of a transcript held at its end would flip between
        // them on every frame, shaking the whole chat (#179).
        let fit = |pixels: Vec2| {
            if sticker.is_some() {
                fit_sticker(pixels.x, pixels.y)
            } else {
                fit_picture(pixels.x, pixels.y, max_width, max_height)
            }
        };
        let pixels_id = egui::Id::new(("picture-pixels", path));
        let drawn = ui.ctx().data(|data| data.get_temp::<Vec2>(pixels_id));
        let reserved = drawn.map_or_else(|| frame_size(media, None, max_width, max_height), fit);
        let position = ui.next_widget_position();
        if !ui.is_rect_visible(Rect::from_min_size(position, reserved)) {
            return ui.allocate_exact_size(reserved, Sense::hover()).0;
        }
        let image = widgets::file_image(ui, path);
        return match image.load_for_size(ui.ctx(), vec2(max_width, max_height)) {
            Ok(egui::load::TexturePoll::Ready { texture }) => {
                if drawn != Some(texture.size) {
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(pixels_id, texture.size));
                }
                let size = fit(texture.size);
                let response = ui.add(
                    image
                        .fit_to_exact_size(size)
                        .corner_radius(if sticker.is_some() { 0.0 } else { radius })
                        .sense(Sense::click()),
                );
                let rect = response.rect;
                if response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    let action = match sticker {
                        Some(_) => Action::PreviewSticker(path.clone()),
                        None => match crate::image_preview::open_target(path, true) {
                            crate::image_preview::OpenTarget::Preview => Action::PreviewImage {
                                path: path.clone(),
                                chat: view.chat.id.clone(),
                                message: message.id.clone(),
                                card: None,
                            },
                            crate::image_preview::OpenTarget::External => {
                                Action::OpenFile(path.clone())
                            }
                        },
                    };
                    actions.push(action);
                }
                rect
            }
            Ok(egui::load::TexturePoll::Pending { .. }) => {
                // A picture released while away loads again at its old size.
                let size = match drawn {
                    Some(pixels) => fit(pixels),
                    None if sticker.is_some() => Vec2::splat(STICKER_SIDE),
                    None => frame_size(media, None, max_width, max_height),
                };
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                if ui.is_rect_visible(rect) {
                    ui.painter().rect_filled(rect, radius, palette.surface);
                    theme::paint_spinner(ui, rect, 22.0, palette.accent);
                }
                rect
            }
            Err(_) if sticker.is_some() || !path.is_file() => {
                // A sticker has nothing else to open it with, and a file that
                // is gone has nothing to open: the bubble downloads it again,
                // or offers to, instead of drawing a dead picture (#603).
                let size = drawn.map_or_else(
                    || {
                        if sticker.is_some() {
                            Vec2::splat(STICKER_SIDE)
                        } else {
                            frame_size(media, None, max_width, max_height)
                        }
                    },
                    fit,
                );
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                if ui.is_rect_visible(rect) {
                    ui.painter().rect_filled(rect, radius, palette.surface);
                }
                actions.push(unreadable_media(view, message, path));
                rect
            }
            Err(_) => {
                let size = frame_size(media, None, max_width, max_height);
                let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                if ui.is_rect_visible(rect) {
                    ui.painter().rect_filled(rect, radius, palette.surface);
                    theme::paint_icon(ui, Icon::CircleAlert, rect, 24.0, palette.danger);
                    ui.painter().text(
                        rect.center() + vec2(0.0, 24.0),
                        Align2::CENTER_CENTER,
                        "Could not display this picture. Click to open it.",
                        theme::regular(11.5),
                        palette.secondary,
                    );
                }
                if response.clicked() {
                    actions.push(Action::OpenFile(path.clone()));
                }
                rect
            }
        };
    }
    let size = frame_size(media, None, max_width, max_height);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let thumbnail = message
            .thumbnail
            .as_deref()
            .filter(|_| sticker.is_none())
            .map(|bytes| thumbnail_uri(ui.ctx(), &message.chat, &message.id, bytes));
        match thumbnail {
            Some(uri) => {
                egui::Image::new(uri)
                    .fit_to_exact_size(size)
                    .corner_radius(radius)
                    .paint_at(ui, rect);
                ui.painter()
                    .rect_filled(rect, radius, Color32::from_black_alpha(60));
            }
            None => {
                ui.painter().rect_filled(rect, radius, palette.surface);
            }
        }
        let disc = Rect::from_center_size(rect.center(), Vec2::splat(44.0));
        match &media.state {
            MediaState::Downloading => {
                ui.painter()
                    .circle_filled(disc.center(), 22.0, Color32::from_black_alpha(120));
                theme::paint_spinner(ui, disc, 22.0, Color32::WHITE);
            }
            MediaState::Failed(_) => {
                ui.painter()
                    .circle_filled(disc.center(), 22.0, Color32::from_black_alpha(120));
                theme::paint_icon(ui, Icon::CircleAlert, disc, 22.0, palette.danger);
                ui.painter().text(
                    rect.center() + vec2(0.0, 34.0),
                    Align2::CENTER_CENTER,
                    crate::i18n::gettext(view.locale, "Download failed. Click to retry."),
                    theme::regular(11.5),
                    Color32::WHITE,
                );
            }
            MediaState::Idle => {
                ui.painter()
                    .circle_filled(disc.center(), 22.0, Color32::from_black_alpha(120));
                theme::paint_icon(
                    ui,
                    if sticker.is_some() {
                        Icon::Sticker
                    } else {
                        Icon::Download
                    },
                    disc,
                    22.0,
                    Color32::WHITE,
                );
                if sticker.is_none() {
                    ui.painter().text(
                        rect.center() + vec2(0.0, 34.0),
                        Align2::CENTER_CENTER,
                        crate::util::bytes(media.size),
                        theme::regular(11.5),
                        Color32::WHITE,
                    );
                }
            }
        }
    }
    let wants = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
        && !matches!(media.state, MediaState::Downloading);
    let auto = ui.is_rect_visible(rect)
        && matches!(media.state, MediaState::Idle)
        && auto_download_allowed(media, sticker.is_some(), view.auto_download);
    if wants || auto {
        actions.push(Action::Download {
            manual: wants,
            card: None,
            chat: view.chat.id.clone(),
            message: message.id.clone(),
        });
    }
    rect
}

/// Reports a downloaded picture or sticker that could not be drawn, so the app
/// forgets its file and downloads it again or offers a retry.
fn unreadable_media(view: &View<'_>, message: &Message, path: &Path) -> Action {
    Action::UnreadableMedia {
        chat: view.chat.id.clone(),
        message: message.id.clone(),
        path: path.to_path_buf(),
    }
}

/// Draws a motion photo's badge in the corner of its picture, and its clip
/// over the picture while it plays, muted and looping.
fn motion_clip(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    motion: &crate::model::Motion,
    picture: Rect,
    actions: &mut Vec<Action>,
) {
    if !ui.is_rect_visible(picture) {
        return;
    }
    let playing = view.motion_playing == Some(message.id.as_str());
    let toggle = || Action::ToggleMotion {
        chat: view.chat.id.clone(),
        message: message.id.clone(),
    };
    let badge = Rect::from_min_size(picture.min + vec2(6.0, 6.0), Vec2::splat(28.0));
    let response = ui
        .interact(
            badge,
            bubble_id(&view.chat.id, &message.id).with("motion"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        actions.push(toggle());
    }
    let frame = motion
        .path
        .as_ref()
        .filter(|_| playing)
        .map(|path| (path, animation::frame(ui, path, picture, view.animate)));
    let icon = match (&frame, &motion.state) {
        (Some((_, animation::Frame::Ready(texture))), _) => {
            ui.painter().rect_filled(picture, 6.0, Color32::BLACK);
            paint_texture(
                ui,
                fit_within(texture.size_vec2(), picture),
                texture.id(),
                Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                6.0,
            );
            Some(Icon::Pause)
        }
        (Some((path, animation::Frame::Unavailable)), _) => {
            // No decoder here plays it: the system player gets the clip.
            actions.push(toggle());
            actions.push(Action::OpenFile((*path).clone()));
            Some(Icon::Play)
        }
        (Some((_, animation::Frame::Pending)), _) => None,
        (None, MediaState::Downloading) if playing => None,
        (None, MediaState::Failed(_)) => Some(Icon::CircleAlert),
        (None, _) => Some(Icon::Play),
    };
    ui.painter()
        .circle_filled(badge.center(), 14.0, Color32::from_black_alpha(140));
    match icon {
        Some(icon) => theme::paint_icon(ui, icon, badge, 14.0, Color32::WHITE),
        None => theme::paint_spinner(ui, badge, 16.0, Color32::WHITE),
    }
    if let MediaState::Failed(error) = &motion.state {
        response.on_hover_text(error);
    }
}

/// Whether this content downloads on its own once drawn, as the bubbles decide
/// with `auto_download_allowed`: stickers always load, other media and
/// documents follow the setting, and nothing over the size limit starts.
fn download_starts_by_itself(content: &Content, auto_download: bool) -> bool {
    match content {
        Content::Sticker { media, .. } => auto_download_allowed(media, true, auto_download),
        other => other
            .media()
            .is_some_and(|media| auto_download_allowed(media, false, auto_download)),
    }
}

/// Stickers always download when visible, while other media follows the setting.
/// Every automatic download still respects the shared size limit.
fn auto_download_allowed(media: &Media, sticker: bool, auto_download: bool) -> bool {
    media.is_within_download_limit() && (sticker || auto_download)
}

/// Draws a video. GIFs play in place; other videos play in the bubble once
/// clicked, downloading first when needed, with controls along the bottom.
#[allow(clippy::too_many_arguments)]
fn video(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    media: &Media,
    seconds: Option<u32>,
    gif: bool,
    width: f32,
    actions: &mut Vec<Action>,
) -> f32 {
    use crate::video::State;
    let palette = view.palette;
    // A video sent before ZapFast made thumbnails has none; its file is
    // here, so its first frame stands in. Without either, it is a file card.
    let thumbnail = message.thumbnail.as_deref();
    if thumbnail.is_none() && media.path.is_none() {
        let title = if gif { "GIF" } else { "Video" };
        let mut detail = Vec::new();
        if let Some(seconds) = seconds {
            detail.push(crate::util::duration(seconds));
        }
        detail.push(crate::util::bytes(media.size));
        attachment(
            ui,
            view,
            message,
            media,
            Icon::Video,
            title,
            &detail.join(" · "),
            width,
            actions,
        );
        return width;
    }
    let limit = width.min(PICTURE_WIDTH);
    // Without its size, a widescreen frame that fills the bubble: the hint is
    // in pixels, so a bare 16 by 9 would shrink it to the narrowest picture.
    let size = frame_size(
        media,
        Some((1280, 720)),
        limit,
        PICTURE_HEIGHT.min(limit * 1.3),
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let playing = match (&media.path, gif) {
        (Some(path), true) => Some(animation::frame(
            ui,
            path,
            rect,
            view.animate && response.hovered(),
        )),
        _ => None,
    };
    if let Some(animation::Frame::Ready(texture)) = &playing {
        if ui.is_rect_visible(rect) {
            ui.painter().image(
                texture.id(),
                rect,
                Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
            && let Some(path) = &media.path
        {
            actions.push(Action::OpenFile(path.clone()));
        }
        return size.x;
    }
    let status = media
        .path
        .as_ref()
        .filter(|_| !gif)
        .and_then(|_| view.video.status(&message.id));
    if ui.is_rect_visible(rect) {
        if status.is_some() {
            view.video.saw(&message.id);
        }
        match status.as_ref().and_then(|status| status.frame.as_ref()) {
            Some(frame) => {
                ui.painter().rect_filled(rect, 6.0, Color32::BLACK);
                paint_texture(
                    ui,
                    fit_within(frame.size_vec2(), rect),
                    frame.id(),
                    Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    6.0,
                );
            }
            None => match (thumbnail, &media.path) {
                (Some(thumbnail), _) => {
                    // Registering the poster decodes it, so it waits for the row to show.
                    let uri = thumbnail_uri(ui.ctx(), &message.chat, &message.id, thumbnail);
                    egui::Image::new(uri)
                        .fit_to_exact_size(size)
                        .corner_radius(6.0)
                        .paint_at(ui, rect);
                }
                (None, Some(path)) => {
                    ui.painter().rect_filled(rect, 6.0, Color32::BLACK);
                    if let animation::Frame::Ready(texture) =
                        animation::frame(ui, path, rect, false)
                    {
                        paint_texture(
                            ui,
                            fit_within(texture.size_vec2(), rect),
                            texture.id(),
                            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            6.0,
                        );
                    }
                }
                (None, None) => {}
            },
        }
        let state = status.as_ref().map(|status| status.state);
        if state != Some(State::Playing) {
            ui.painter()
                .rect_filled(rect, 6.0, Color32::from_black_alpha(40));
            let disc = Rect::from_center_size(rect.center(), Vec2::splat(48.0));
            ui.painter()
                .circle_filled(disc.center(), 24.0, Color32::from_black_alpha(140));
            match (&media.path, &media.state) {
                (Some(_), _)
                    if state == Some(State::Loading)
                        || matches!(playing, Some(animation::Frame::Pending)) =>
                {
                    theme::paint_spinner(ui, disc, 24.0, Color32::WHITE)
                }
                (Some(_), _) if gif => {
                    theme::paint_icon(ui, Icon::ExternalLink, disc, 22.0, Color32::WHITE)
                }
                (None, MediaState::Downloading) => {
                    theme::paint_spinner(ui, disc, 24.0, Color32::WHITE)
                }
                (None, MediaState::Failed(_)) => {
                    theme::paint_icon(ui, Icon::CircleAlert, disc, 22.0, palette.danger)
                }
                (Some(_), _) | (None, MediaState::Idle) => {
                    theme::paint_icon(ui, Icon::Play, disc, 22.0, Color32::WHITE)
                }
            }
        }
        match (&status, &media.path) {
            (Some(status), Some(path)) => {
                // Controls show while paused and while the pointer is over
                // the video, as in other players.
                if status.state == State::Paused
                    || (status.state == State::Playing && ui.rect_contains_pointer(rect))
                {
                    video_controls(
                        ui,
                        &VideoControls {
                            player: view.video,
                            locale: view.locale,
                            accent: view.palette.accent,
                            expanded: false,
                        },
                        &message.id,
                        path,
                        rect,
                        status,
                        actions,
                    );
                }
            }
            _ => {
                let mut label = Vec::new();
                if gif {
                    label.push("GIF".to_owned());
                }
                if let Some(seconds) = seconds {
                    label.push(crate::util::duration(seconds));
                }
                if media.path.is_none() {
                    label.push(crate::util::bytes(media.size));
                }
                if !label.is_empty() {
                    let galley = ui.painter().layout_no_wrap(
                        label.join(" · "),
                        theme::medium(11.5),
                        Color32::WHITE,
                    );
                    let chip = Rect::from_min_size(
                        pos2(rect.left() + 8.0, rect.bottom() - galley.size().y - 14.0),
                        galley.size() + vec2(12.0, 6.0),
                    );
                    ui.painter().rect_filled(
                        chip,
                        chip.height() / 2.0,
                        Color32::from_black_alpha(140),
                    );
                    ui.painter()
                        .galley(chip.min + vec2(6.0, 3.0), galley, Color32::WHITE);
                }
            }
        }
    }
    let auto = ui.is_rect_visible(rect)
        && media.path.is_none()
        && matches!(media.state, MediaState::Idle)
        && view.auto_download
        && media.is_within_download_limit();
    if auto {
        actions.push(Action::Download {
            manual: false,
            card: None,
            chat: view.chat.id.clone(),
            message: message.id.clone(),
        });
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        video_clicked(view, message, media, gif, auto, actions);
    }
    // The two clicks of a double-click play and pause between them; the
    // video then opens over the window as it was.
    if response.double_clicked()
        && !gif
        && let Some(path) = &media.path
    {
        actions.push(expand_video_action(&message.id, path));
    }
    size.x
}

/// What "Fill the window" does: the video covers the window, starting it
/// when it is not the one loaded.
fn expand_video_action(message: &str, path: &Path) -> Action {
    Action::ExpandVideo {
        message: message.to_owned(),
        path: path.to_owned(),
    }
}

/// What a click on a video does: a downloaded video plays or pauses, a GIF
/// that cannot play here opens in the system viewer, and a video still on
/// WhatsApp's servers downloads and then plays. The full-window viewer is a
/// separate way in, from the message's own menu, so a clip keeps playing in
/// its bubble here. `downloading` is set when this frame already asked for the
/// download.
fn video_clicked(
    view: &View<'_>,
    message: &Message,
    media: &Media,
    gif: bool,
    downloading: bool,
    actions: &mut Vec<Action>,
) {
    match &media.path {
        Some(path) if gif => actions.push(Action::OpenFile(path.clone())),
        Some(path) => actions.push(Action::PlayVideo {
            message: message.id.clone(),
            path: path.clone(),
        }),
        None => {
            if !downloading && !matches!(media.state, MediaState::Downloading) {
                actions.push(Action::Download {
                    manual: true,
                    card: None,
                    chat: view.chat.id.clone(),
                    message: message.id.clone(),
                });
            }
            if !gif {
                actions.push(Action::PlayVideoWhenDownloaded(message.id.clone()));
            }
        }
    }
}

/// What the video controls need besides the video: the bubble and the view
/// covering the window share them.
pub(crate) struct VideoControls<'a> {
    pub player: &'a crate::video::Player,
    pub locale: crate::i18n::Locale,
    pub accent: Color32,
    /// Whether the video covers the window; its button then puts it back.
    pub expanded: bool,
}

/// Play/pause, the time, a seek bar, a sound switch, and the switch between
/// the message and the whole window, along the bottom of a playing video.
pub(crate) fn video_controls(
    ui: &mut egui::Ui,
    controls: &VideoControls<'_>,
    message: &str,
    path: &Path,
    rect: Rect,
    status: &crate::video::Status,
    actions: &mut Vec<Action>,
) {
    let bar = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 32.0), rect.max);
    ui.painter().rect_filled(
        bar,
        CornerRadius {
            nw: 0,
            ne: 0,
            sw: 6,
            se: 6,
        },
        Color32::from_black_alpha(150),
    );
    let id = ui.id().with(("video-controls", message));
    let toggle = Rect::from_center_size(pos2(bar.left() + 18.0, bar.center().y), Vec2::splat(26.0));
    let playing = status.state == crate::video::State::Playing;
    theme::paint_icon(
        ui,
        if playing { Icon::Pause } else { Icon::Play },
        toggle,
        16.0,
        Color32::WHITE,
    );
    let tooltip = if playing {
        crate::i18n::gettext(controls.locale, "Pause")
    } else {
        crate::i18n::gettext(controls.locale, "Play")
    };
    if ui
        .interact(toggle, id.with("toggle"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tooltip.as_ref())
        .clicked()
    {
        actions.push(Action::PlayVideo {
            message: message.to_owned(),
            path: path.to_owned(),
        });
    }
    let window =
        Rect::from_center_size(pos2(bar.right() - 18.0, bar.center().y), Vec2::splat(26.0));
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::new(("video-window", message)), window));
    theme::paint_icon(
        ui,
        if controls.expanded {
            Icon::Minimize
        } else {
            Icon::Maximize
        },
        window,
        16.0,
        Color32::WHITE,
    );
    let tooltip = if controls.expanded {
        crate::i18n::gettext(controls.locale, "Back to the message (Esc)")
    } else {
        crate::i18n::gettext(controls.locale, "Fill the window")
    };
    if ui
        .interact(window, id.with("window"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tooltip.as_ref())
        .clicked()
    {
        actions.push(if controls.expanded {
            Action::CloseImagePreview
        } else {
            expand_video_action(message, path)
        });
    }
    let sound = Rect::from_center_size(
        pos2(window.left() - 15.0, bar.center().y),
        Vec2::splat(26.0),
    );
    let muted = controls.player.muted();
    theme::paint_icon(
        ui,
        if muted { Icon::VolumeX } else { Icon::Volume2 },
        sound,
        16.0,
        Color32::WHITE,
    );
    let tooltip = if muted {
        crate::i18n::gettext(controls.locale, "Unmute")
    } else {
        crate::i18n::gettext(controls.locale, "Mute")
    };
    if ui
        .interact(sound, id.with("sound"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tooltip.as_ref())
        .clicked()
    {
        actions.push(Action::ToggleVideoSound);
    }
    // The playback speed, cycled by a click as the voice message chip is.
    let speed_label = crate::audio::speed_label(controls.player.speed());
    let speed_galley = ui.painter().layout_no_wrap(
        speed_label,
        theme::tabular(fastframe_fonts::Weight::Medium, 11.5),
        Color32::WHITE,
    );
    let speed = Rect::from_center_size(
        pos2(
            sound.left() - 4.0 - (speed_galley.size().x + 10.0) / 2.0,
            bar.center().y,
        ),
        vec2(speed_galley.size().x + 10.0, 18.0),
    );
    ui.painter()
        .rect_filled(speed, 9.0, Color32::from_white_alpha(40));
    ui.painter().galley(
        speed.center() - speed_galley.size() / 2.0,
        speed_galley,
        Color32::WHITE,
    );
    if ui
        .interact(speed, id.with("speed"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        actions.push(Action::CycleVideoSpeed);
    }
    let time = format!(
        "{} / {}",
        crate::util::duration(status.position.as_secs() as u32),
        crate::util::duration(status.total.as_secs() as u32)
    );
    let galley = ui.painter().layout_no_wrap(
        time,
        theme::tabular(fastframe_fonts::Weight::Medium, 11.5),
        Color32::WHITE,
    );
    let text = pos2(toggle.right() + 4.0, bar.center().y - galley.size().y / 2.0);
    let track = Rect::from_min_max(
        pos2(text.x + galley.size().x + 10.0, bar.center().y - 8.0),
        pos2(speed.left() - 6.0, bar.center().y + 8.0),
    );
    ui.painter().galley(text, galley, Color32::WHITE);
    if track.width() < 12.0 {
        return;
    }
    let response = ui
        .interact(track, id.with("seek"), Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let pointed = response
        .interact_pointer_pos()
        .map(|pointer| ((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0));
    // Seeking restarts the decoder, so a drag seeks once, where it ends.
    let fraction = match pointed {
        Some(fraction) if response.dragged() => fraction,
        _ => status.fraction(),
    };
    let line = Rect::from_center_size(track.center(), vec2(track.width(), 3.0));
    ui.painter()
        .rect_filled(line, 1.5, Color32::from_white_alpha(90));
    let played = pos2(line.left() + fraction * line.width(), line.center().y);
    ui.painter().rect_filled(
        Rect::from_min_max(line.min, pos2(played.x, line.bottom())),
        1.5,
        controls.accent,
    );
    ui.painter().circle_filled(played, 5.0, controls.accent);
    if (response.clicked() || response.drag_stopped())
        && let Some(fraction) = pointed
    {
        actions.push(Action::SeekVideo {
            message: message.to_owned(),
            fraction,
        });
    }
}

/// Side of a round video message.
const NOTE_SIDE: f32 = 220.0;

/// Draws a round video message (PTV) as a circle that plays in place when
/// clicked, with a ring for the progress. Returns its width.
fn video_note(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    media: &Media,
    seconds: Option<u32>,
    actions: &mut Vec<Action>,
) -> f32 {
    use crate::video::State;
    let palette = view.palette;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(NOTE_SIDE), Sense::click());
    let status = media
        .path
        .as_ref()
        .and_then(|_| view.video.status(&message.id));
    if ui.is_rect_visible(rect) {
        if status.is_some() {
            view.video.saw(&message.id);
        }
        let center = rect.center();
        // Leave room around the picture for the progress ring.
        let picture = rect.shrink(5.0);
        let frame = status
            .as_ref()
            .and_then(|status| status.frame.as_ref())
            .map(|frame| (frame.id(), frame.size_vec2()));
        let poster = || {
            let bytes = message.thumbnail.as_deref()?;
            let uri = thumbnail_uri(ui.ctx(), &message.chat, &message.id, bytes);
            match egui::Image::new(uri).load_for_size(ui.ctx(), picture.size()) {
                Ok(egui::load::TexturePoll::Ready { texture }) => Some((texture.id, texture.size)),
                _ => None,
            }
        };
        match frame.or_else(poster) {
            Some((texture, size)) => paint_texture(
                ui,
                picture,
                texture,
                crate::video::square_uv(size.x, size.y),
                picture.width() / 2.0,
            ),
            None => {
                ui.painter()
                    .circle_filled(center, picture.width() / 2.0, palette.surface);
            }
        }
        let state = status.as_ref().map(|status| status.state);
        let ring = NOTE_SIDE / 2.0 - 2.0;
        if let Some(status) = &status {
            ui.painter().circle_stroke(
                center,
                ring,
                Stroke::new(3.0, palette.secondary.gamma_multiply(0.35)),
            );
            ui.painter().add(egui::Shape::line(
                crate::video::arc(center, ring, status.fraction()),
                Stroke::new(3.0, palette.accent),
            ));
        }
        let hovered = ui.rect_contains_pointer(rect);
        let disc = Rect::from_center_size(center, Vec2::splat(48.0));
        let waiting = matches!(
            (&media.path, &media.state, state),
            (Some(_), _, Some(State::Loading)) | (None, MediaState::Downloading, _)
        );
        let icon = match (&media.path, &media.state, state) {
            _ if waiting => None,
            // Only a pause sign under the pointer covers a playing video.
            (Some(_), _, Some(State::Playing)) => hovered.then_some(Icon::Pause),
            (None, MediaState::Failed(_), _) => Some(Icon::CircleAlert),
            _ => Some(Icon::Play),
        };
        if state != Some(State::Playing) {
            ui.painter().circle_filled(
                center,
                picture.width() / 2.0,
                Color32::from_black_alpha(40),
            );
        }
        if waiting || icon.is_some() {
            ui.painter()
                .circle_filled(center, 24.0, Color32::from_black_alpha(140));
        }
        if waiting {
            theme::paint_spinner(ui, disc, 24.0, Color32::WHITE);
        } else if let Some(icon) = icon {
            let color = if icon == Icon::CircleAlert {
                palette.danger
            } else {
                Color32::WHITE
            };
            theme::paint_icon(ui, icon, disc, 22.0, color);
        }
        // The time while it plays, otherwise its length.
        let label = match &status {
            Some(status) => Some(crate::util::duration(status.position.as_secs() as u32)),
            None => seconds
                .map(crate::util::duration)
                .or_else(|| media.path.is_none().then(|| crate::util::bytes(media.size))),
        };
        if let Some(label) = label {
            let galley = ui.painter().layout_no_wrap(
                label,
                theme::tabular(fastframe_fonts::Weight::Medium, 11.5),
                Color32::WHITE,
            );
            let chip = Rect::from_center_size(
                pos2(center.x, picture.bottom() - galley.size().y / 2.0 - 16.0),
                galley.size() + vec2(12.0, 6.0),
            );
            ui.painter()
                .rect_filled(chip, chip.height() / 2.0, Color32::from_black_alpha(140));
            ui.painter()
                .galley(chip.min + vec2(6.0, 3.0), galley, Color32::WHITE);
        }
    }
    let auto = ui.is_rect_visible(rect)
        && media.path.is_none()
        && matches!(media.state, MediaState::Idle)
        && view.auto_download
        && media.is_within_download_limit();
    if auto {
        actions.push(Action::Download {
            manual: false,
            card: None,
            chat: view.chat.id.clone(),
            message: message.id.clone(),
        });
    }
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        video_clicked(view, message, media, false, auto, actions);
    }
    NOTE_SIDE
}

/// Paints a texture into `rect` with rounded corners; a radius of half the
/// side makes a circle.
fn paint_texture(ui: &egui::Ui, rect: Rect, texture: egui::TextureId, uv: Rect, radius: f32) {
    ui.painter().add(
        egui::epaint::RectShape::filled(
            rect,
            CornerRadius::same(radius.round().clamp(0.0, 255.0) as u8),
            Color32::WHITE,
        )
        .with_texture(texture, uv),
    );
}

/// The largest rect with the proportions of `size` centred in `rect`.
fn fit_within(size: Vec2, rect: Rect) -> Rect {
    if size.x <= 0.0 || size.y <= 0.0 {
        return rect;
    }
    let scale = (rect.width() / size.x).min(rect.height() / size.y);
    Rect::from_center_size(rect.center(), size * scale)
}

#[allow(clippy::too_many_arguments)]
fn attachment(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    media: &Media,
    icon: Icon,
    title: &str,
    detail: &str,
    width: f32,
    actions: &mut Vec<Action>,
) {
    let palette = view.palette;
    let response = Frame::new()
        .fill(palette.window.gamma_multiply(0.35))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            // Include margins in the settled row width.
            let card = (width - 20.0).max(0.0);
            ui.set_width(card);

            let disc = |ui: &mut egui::Ui| {
                let (disc, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
                ui.painter().circle_filled(
                    disc.center(),
                    18.0,
                    palette.accent.gamma_multiply(0.25),
                );
                theme::paint_icon(ui, icon, disc, 18.0, palette.accent);
            };
            let action = |ui: &mut egui::Ui| match (&media.path, &media.state) {
                (Some(_), _) => {
                    theme::icon(ui, Icon::ExternalLink, 18.0, palette.secondary);
                }
                (None, MediaState::Downloading) => {
                    theme::spinner(ui, 18.0, palette.accent);
                }
                (None, _) => {
                    theme::icon(ui, Icon::Download, 18.0, palette.secondary);
                }
            };
            let column = |ui: &mut egui::Ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    // Reserve 70 points for the icon, action, and gaps.
                    ui.set_width((card - 70.0).max(0.0));
                    widgets::rich_text(ui, title, theme::medium(14.0), palette.text);
                    let detail = match &media.state {
                        MediaState::Failed(error) => format!("{error}. Click to retry."),
                        _ => detail.to_owned(),
                    };
                    theme::text(ui, detail, theme::regular(12.0), palette.secondary);
                });
            };
            // Fix the row left-to-right at the card width in own bubbles.
            ui.allocate_ui_with_layout(
                vec2(card, 52.0),
                Layout::left_to_right(egui::Align::Center),
                |ui| {
                    disc(ui);
                    column(ui);
                    action(ui);
                },
            );
        })
        .response;
    let auto = ui.is_rect_visible(response.rect)
        && media.path.is_none()
        && matches!(media.state, MediaState::Idle)
        && auto_download_allowed(media, false, view.auto_download);
    if auto {
        actions.push(Action::Download {
            manual: false,
            card: None,
            chat: view.chat.id.clone(),
            message: message.id.clone(),
        });
    }
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            bubble_id(&view.chat.id, &message.id).with("card"),
            response.rect,
        );
    });
    let response = ui
        .interact(
            response.rect,
            ui.id().with(("attachment", &message.id)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() && !auto {
        match &media.path {
            Some(path) => actions.push(Action::OpenFile(path.clone())),
            None if !matches!(media.state, MediaState::Downloading) => {
                actions.push(Action::Download {
                    manual: true,
                    card: None,
                    chat: view.chat.id.clone(),
                    message: message.id.clone(),
                })
            }
            None => {}
        }
    }
}

/// The space above a conversation's messages that keeps a short one at the
/// bottom of the view: what `natural`, the height the messages took last
/// frame, leaves of the viewport. Unmeasured, there is none.
fn bottom_filler(viewport: f32, natural: Option<f32>, spacing: f32) -> f32 {
    natural.map_or(0.0, |natural| (viewport - natural - spacing).max(0.0))
}

/// Draws the in-chat player with a waveform for voice notes or a track and music badge
/// for audio files. Playback, seeking and speed controls remain shared between both types.
#[allow(clippy::too_many_arguments)]
fn voice_player(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    media: &Media,
    seconds: Option<u32>,
    waveform: &[u8],
    width: f32,
    reserve: f32,
    actions: &mut Vec<Action>,
) -> Option<Rect> {
    use crate::audio::State;
    let palette = view.palette;
    let status = view.player.status(&message.id);
    let button = 36.0;
    let bar_height = 30.0;
    let chip = 44.0;
    let voice_note = matches!(
        message.content,
        Content::Audio {
            voice_note: true,
            ..
        }
    );
    // The chip appears with the playable clip; the waveform takes its space
    // back while the audio is still downloading.
    let shows_chip = media.path.is_some();
    let wave_width = (width
        - button
        - 10.0
        - if shows_chip { chip + 10.0 } else { 0.0 }
        - if voice_note { 0.0 } else { button + 10.0 })
    .max(0.0);
    let bars: Vec<u8> = if !waveform.is_empty() {
        waveform.to_vec()
    } else if let Some(bars) = view.player.bars(&message.id) {
        bars.to_vec()
    } else {
        vec![12; crate::voice::BARS]
    };
    let fill = palette.accent.gamma_multiply(0.22);
    let hover = palette.accent.gamma_multiply(0.38);
    let waiting = |ui: &mut egui::Ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(button), Sense::hover());
        ui.painter()
            .circle_filled(rect.center(), button / 2.0, fill);
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.centered_and_justified(|ui| {
                theme::spinner(ui, 18.0, palette.accent);
            });
        });
    };
    // Force left-to-right layout at the player's width inside own bubbles.
    let mut duration = None;
    let row = ui.allocate_ui_with_layout(
        vec2(width.max(0.0), button),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            match (&media.path, &media.state) {
                (None, MediaState::Downloading) => waiting(ui),
                (None, _) => {
                    if theme::circle_button(
                        ui,
                        Icon::Download,
                        button,
                        fill,
                        hover,
                        palette.accent,
                        "Download",
                    )
                    .clicked()
                    {
                        actions.push(Action::Download {
                            manual: true,
                            card: None,
                            chat: view.chat.id.clone(),
                            message: message.id.clone(),
                        });
                    }
                }
                (Some(path), _) => match status.state {
                    State::Loading => waiting(ui),
                    State::Playing | State::Paused | State::Idle => {
                        let (icon, tooltip) = if status.state == State::Playing {
                            (Icon::Pause, "Pause")
                        } else {
                            (Icon::Play, "Play")
                        };
                        if theme::circle_button(
                            ui,
                            icon,
                            button,
                            fill,
                            hover,
                            palette.accent,
                            tooltip,
                        )
                        .clicked()
                        {
                            actions.push(Action::PlayVoice {
                                message: message.id.clone(),
                                path: path.clone(),
                            });
                        }
                    }
                },
            }
            let mut wave_middle = None;
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let (rect, response) =
                    ui.allocate_exact_size(vec2(wave_width, bar_height), Sense::click());
                wave_middle = Some(rect.center().y);
                let pitch = 3.0;
                let count = (rect.width() / pitch).floor() as usize;
                let fraction = if status.total > Duration::ZERO {
                    status.position.as_secs_f32() / status.total.as_secs_f32()
                } else {
                    0.0
                };
                let played_until = rect.left() + fraction * rect.width();
                let quiet = palette.secondary.gamma_multiply(0.7);
                let kind = if voice_note {
                    "voice-waveform"
                } else {
                    "audio-track"
                };
                ui.ctx().data_mut(|data| {
                    data.insert_temp(bubble_id(&view.chat.id, &message.id).with(kind), rect);
                });
                if !voice_note {
                    let track = Rect::from_center_size(rect.center(), vec2(rect.width(), 4.0));
                    ui.painter().rect_filled(track, 2.0, quiet);
                    if fraction > 0.0 {
                        ui.painter().rect_filled(
                            Rect::from_min_max(track.min, pos2(played_until, track.bottom())),
                            2.0,
                            palette.accent,
                        );
                    }
                } else if count > 0 {
                    for index in 0..count {
                        let level = f32::from(bars[index * bars.len() / count]) / 100.0;
                        let height = (2.0 + level * (bar_height - 4.0)).max(2.0);
                        let x = rect.left() + index as f32 * pitch + 1.0;
                        let colour = if status.state != State::Idle && x <= played_until {
                            palette.accent
                        } else {
                            quiet
                        };
                        ui.painter().rect_filled(
                            Rect::from_center_size(
                                egui::pos2(x, rect.center().y),
                                vec2(2.0, height),
                            ),
                            1.0,
                            colour,
                        );
                    }
                }
                if (!voice_note || matches!(status.state, State::Playing | State::Paused))
                    && rect.width() >= 10.0
                {
                    let knob = played_until.clamp(rect.left() + 5.0, rect.right() - 5.0);
                    ui.painter().circle_filled(
                        egui::pos2(knob, rect.center().y),
                        5.0,
                        palette.accent,
                    );
                }
                if let Some(path) = &media.path {
                    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                    if response.clicked()
                        && let Some(pointer) = response.interact_pointer_pos()
                    {
                        let fraction = if rect.width() > 0.0 {
                            ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        actions.push(Action::SeekVoice {
                            message: message.id.clone(),
                            path: path.clone(),
                            fraction,
                        });
                    }
                }
                // Show playback position while active, otherwise total duration.
                let shown = match status.state {
                    State::Playing | State::Paused => {
                        crate::util::duration(status.position.as_secs() as u32)
                    }
                    _ => seconds
                        .or_else(|| {
                            (status.total > Duration::ZERO).then_some(status.total.as_secs() as u32)
                        })
                        .map(crate::util::duration)
                        .unwrap_or_else(|| crate::util::bytes(media.size)),
                };
                let (text, font) = match &media.state {
                    MediaState::Failed(error) => {
                        (format!("{error}. Click to retry."), theme::regular(11.5))
                    }
                    _ => (
                        shown,
                        theme::tabular(fastframe_fonts::Weight::Regular, 11.5),
                    ),
                };
                let shown = theme::text(ui, text, font, palette.secondary).rect;
                duration = Some(shown);
                // A dot beside the length until their voice message is heard,
                // here or on another device.
                if voice_note
                    && !message.from_me
                    && message.status != Delivery::Played
                    && !matches!(media.state, MediaState::Failed(_))
                {
                    let dot = Rect::from_center_size(
                        pos2(shown.right() + 7.0, shown.center().y),
                        Vec2::splat(6.0),
                    );
                    ui.painter()
                        .circle_filled(dot.center(), 3.0, palette.accent);
                    // The time beside it keeps clear of the dot too.
                    duration = Some(shown.union(dot));
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(
                            bubble_id(&view.chat.id, &message.id).with("unheard"),
                            dot,
                        );
                    });
                }
            });
            if !voice_note {
                let (slot, response) = ui.allocate_exact_size(Vec2::splat(button), Sense::hover());
                let at = Rect::from_center_size(
                    pos2(slot.center().x, wave_middle.unwrap_or(slot.center().y)),
                    Vec2::splat(button),
                );
                ui.painter().circle_filled(at.center(), button / 2.0, fill);
                theme::paint_icon(ui, Icon::Music, at, 20.0, palette.accent);
                let label = crate::i18n::gettext(view.locale, "Audio");
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label.as_ref())
                });
                ui.ctx().data_mut(|data| {
                    data.insert_temp(bubble_id(&view.chat.id, &message.id).with("audio-file"), at);
                });
            }
            // Speed chip, cycling 1x, 1.5x, and 2x like the phone. The
            // message menu lists every speed, including 1.25x and 1.75x.
            if shows_chip {
                let speed = view.voice_speed;
                // The label follows the click at once, faded until this
                // clip actually plays at that speed.
                let preparing = view.player.preparing_speed(&message.id);
                // Reserve the chip's place in the row, then draw it level
                // with the middle of the waveform rather than the whole row.
                let size = vec2(chip, 20.0);
                let (slot, _) = ui.allocate_exact_size(size, Sense::hover());
                let at = Rect::from_center_size(
                    egui::pos2(slot.center().x, wave_middle.unwrap_or(slot.center().y)),
                    size,
                );
                let response = ui
                    .scope_builder(egui::UiBuilder::new().max_rect(at), |ui| {
                        speed_pill(ui, view, size, speed, speed > 1.0, preparing)
                    })
                    .inner;
                ui.ctx().data_mut(|data| {
                    data.insert_temp(speed_chip_id(&view.chat.id, &message.id), response.rect);
                });
                if response.clicked() {
                    actions.push(speed_action(
                        view.chat,
                        crate::audio::next_cycled_speed(speed),
                    ));
                }
                response.on_hover_text(if preparing {
                    "Preparing playback speed"
                } else {
                    "Playback speed. Right-click for every speed."
                });
            }
        },
    );
    let auto = media.path.is_none()
        && matches!(media.state, MediaState::Idle)
        && view.auto_download
        && media.is_within_download_limit();
    if auto {
        actions.push(Action::Download {
            manual: false,
            card: None,
            chat: view.chat.id.clone(),
            message: message.id.clone(),
        });
    }
    // The time and ticks share the duration's line, as on the phone. A long
    // error leaves them no room, so they keep a line of their own.
    let right = row.response.rect.right();
    duration
        .filter(|duration| duration.right() + 8.0 + reserve <= right)
        .map(|duration| {
            Rect::from_min_max(
                pos2(right - reserve, duration.center().y - 7.5),
                pos2(right, duration.center().y + 7.5),
            )
        })
}

/// Voice-recording controls and live waveform.
fn recording_strip(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let (elapsed, levels, listening) = match app.recording.as_ref() {
        Some(recorder) => (recorder.elapsed(), recorder.levels(), recorder.listening()),
        None => return,
    };
    // As tall as the one-line composer it replaces, so the field keeps its
    // height while recording.
    let line_height = ui
        .painter()
        .layout_no_wrap("x".to_owned(), theme::regular(BODY_SIZE), palette.text)
        .size()
        .y;
    let row_height = (line_height + COMPOSER_PADDING)
        .round()
        .max(COMPOSER_CONTROL);
    let button = row_height;
    // As in WhatsApp: discard at the start, the light and the time, the
    // waveform across the rest, and send at the end.
    ui.allocate_ui_with_layout(
        vec2(ui.available_width().max(0.0), row_height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if theme::circle_button(
                ui,
                Icon::Trash,
                button,
                palette.surface,
                palette.surface_hover,
                palette.secondary,
                "Discard",
            )
            .clicked()
            {
                app.actions.push(Action::CancelRecording);
            }
            // The light pulses red once sound arrives; while the microphone
            // wakes it stays dim, and the time at zero.
            let (dot, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
            let light = if listening {
                let pulse = 0.55 + 0.45 * (elapsed.as_secs_f32() * 3.0).sin().abs();
                palette.danger.gamma_multiply(pulse)
            } else {
                palette.secondary.gamma_multiply(0.5)
            };
            ui.painter().circle_filled(dot.center(), 5.0, light);
            theme::text(
                ui,
                crate::util::duration(elapsed.as_secs() as u32),
                theme::tabular(fastframe_fonts::Weight::Medium, 14.0),
                palette.text,
            );
            // Recent audio levels, newest on the right against send.
            let spacing = ui.spacing().item_spacing.x;
            let wave_width = (ui.available_width() - button - spacing).max(0.0);
            let (rect, _) = ui.allocate_exact_size(vec2(wave_width, 28.0), Sense::hover());
            ui.ctx()
                .data_mut(|data| data.insert_temp(recording_wave_id(), rect));
            let pitch = 3.0;
            let count = (rect.width() / pitch).floor() as usize;
            let shown = &levels[levels.len().saturating_sub(count)..];
            for (index, level) in shown.iter().enumerate() {
                let height = 2.0_f32 + (level * 4.0).min(1.0) * 24.0;
                let x = rect.right() - (shown.len() - index) as f32 * pitch + 1.0;
                ui.painter().rect_filled(
                    Rect::from_center_size(egui::pos2(x, rect.center().y), vec2(2.0, height)),
                    1.0,
                    palette.accent,
                );
            }
            if theme::circle_button(
                ui,
                Icon::Send,
                button,
                palette.accent,
                palette.accent_hover,
                palette.on_accent,
                "Send",
            )
            .clicked()
            {
                app.actions.push(Action::SendRecording);
            }
        },
    );
}

/// Where the recorder's waveform was drawn, for layout tests.
pub(crate) fn recording_wave_id() -> egui::Id {
    egui::Id::new("recording-wave")
}

/// A selecting row's check box: an outline, or ticked in the accent.
fn select_box(ui: &egui::Ui, palette: &Palette, rect: Rect, checked: bool) {
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

/// Replaces the composer while messages are selected.
fn selection_bar(app: &mut App, ui: &mut egui::Ui, chat: &str, selected: &[String]) {
    let palette = app.palette;
    let locale = app.locale;
    ui.horizontal(|ui| {
        if theme::icon_button(
            ui,
            Icon::X,
            18.0,
            palette.secondary,
            palette.text,
            &crate::i18n::gettext(locale, "Cancel selection"),
        )
        .clicked()
            || ui.input(|input| input.key_pressed(Key::Escape))
        {
            app.actions.push(Action::CancelSelection);
        }
        let count = crate::i18n::ngettext(
            locale,
            "{count} selected",
            "{count} selected",
            selected.len() as u32,
        )
        .replace("{count}", &selected.len().to_string());
        theme::text(ui, &count, theme::medium(14.5), palette.text);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // The selection stays open with nothing in it, as its check
            // boxes do; there is nothing to forward until one is ticked.
            let forward = ui
                .add_enabled_ui(!selected.is_empty(), |ui| {
                    theme::pill_button(
                        ui,
                        &palette,
                        &crate::i18n::gettext(locale, "Forward…"),
                        true,
                    )
                })
                .inner;
            if forward.clicked() {
                app.actions.push(Action::ShowDialog(Dialog::Forward {
                    chat: chat.to_owned(),
                    messages: selected.to_vec(),
                }));
            }
            let delete = ui
                .add_enabled_ui(!selected.is_empty(), |ui| {
                    theme::pill_button(
                        ui,
                        &palette,
                        &crate::i18n::gettext(locale, "Delete…"),
                        false,
                    )
                })
                .inner;
            // Demo/test: hold this menu open for a screenshot.
            #[cfg(any(test, feature = "demo"))]
            if app.open_selection_delete_menu {
                egui::Popup::open_id(ui.ctx(), delete.id.with("popup"));
            }
            let can_revoke = app.conversations.get(chat).is_some_and(|conversation| {
                selection_can_revoke(&conversation.messages, selected, crate::util::now())
            });
            let labels = [
                crate::i18n::gettext(app.locale, "Delete for everyone"),
                crate::i18n::gettext(app.locale, "Delete for me"),
            ];
            let width = widgets::menu_width(ui, &[&*labels[0], &*labels[1]], true);
            let mut actions = Vec::new();
            egui::Popup::menu(&delete)
                .width(width)
                .frame(widgets::menu_frame(&palette))
                .show(|ui| {
                    selection_delete_items(
                        ui,
                        &palette,
                        app.locale,
                        chat,
                        can_revoke,
                        selected,
                        &mut actions,
                    );
                });
            app.actions.extend(actions);
        });
    });
}

/// The file name to suggest when saving an attachment: the sender's name for
/// documents, the cached file's name otherwise. Path separators are dropped so
/// a crafted name cannot point the dialog somewhere else.
fn attachment_name(content: &Content, path: &Path) -> String {
    let name = match content {
        Content::Document { file_name, .. } if !file_name.trim().is_empty() => file_name.clone(),
        _ => path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "attachment".to_owned()),
    };
    name.replace(['/', '\\'], "_")
}

/// Whether a conversation has visible content. Used by tests.
#[allow(dead_code)]
pub fn has_messages(conversation: &Conversation) -> bool {
    !conversation.messages.is_empty()
}

#[allow(dead_code)]
fn status_label(status: Delivery) -> &'static str {
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
fn chat_of(chat: &ChatId) -> &str {
    chat
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each reason the keyboard menu speaks for an unavailable card button
    /// goes through gettext as its own literal, so extraction finds it and
    /// translators get it. No catalog translates them yet, and the template
    /// is regenerated on main, so this file's own gettext calls prove the
    /// literal. The test walks the same table the menu reads.
    #[test]
    fn unavailable_reasons_are_spoken_through_gettext() {
        const SOURCE: &str = include_str!("conversation.rs");
        let locale = crate::i18n::Locale::German;
        for blocker in ButtonBlocker::ALL {
            let shown = blocker.shown();
            let spoken = blocker.said(locale);
            assert_eq!(spoken, crate::i18n::gettext(locale, shown), "{blocker:?}");
            assert!(
                SOURCE.contains(&format!("gettext(locale, \"{shown}\")")),
                "{blocker:?}: {shown:?} is not a gettext literal"
            );
        }
    }

    /// "Delete for everyone" is for our own messages, not yet deleted, up to
    /// WhatsApp's time limit, as the menu has always offered it; each other
    /// case has its reason.
    #[test]
    fn delete_for_everyone_is_for_our_own_recent_messages() {
        let window = crate::app::REVOKE_WINDOW.as_secs() as i64;
        let ours = message(Content::text("Synthetic message"));
        assert_eq!(revoke_blocker(&ours, window), None, "at the limit");
        assert_eq!(
            revoke_blocker(&ours, window + 1),
            Some(RevokeBlocker::TooOld)
        );
        let theirs = Message {
            from_me: false,
            ..ours.clone()
        };
        assert_eq!(revoke_blocker(&theirs, 0), Some(RevokeBlocker::NotOurs));
        let deleted = Message {
            content: Content::Revoked,
            ..ours
        };
        assert_eq!(revoke_blocker(&deleted, 0), Some(RevokeBlocker::Deleted));
    }

    /// Each reason Shift+Delete gives goes through gettext as its own
    /// literal, so extraction finds it, and is said after the item's name as
    /// the keyboard menu names an unavailable entry.
    #[test]
    fn revoke_reasons_are_spoken_through_gettext() {
        const SOURCE: &str = include_str!("conversation.rs");
        let locale = crate::i18n::Locale::English;
        for blocker in RevokeBlocker::ALL {
            let reason = blocker.said(locale);
            assert!(
                SOURCE.contains(&format!("gettext(locale, \"{reason}\")")),
                "{blocker:?}: {reason:?} is not a gettext literal"
            );
            assert_eq!(
                revoke_refusal_said(locale, blocker),
                format!("Delete for everyone, unavailable: {reason}")
            );
        }
    }

    #[test]
    fn account_read_receipt_privacy_hides_read_ticks_only_in_outgoing_dms() {
        use crate::model::ChatKind;

        assert_eq!(
            display_delivery(Delivery::Read, true, ChatKind::Direct, true),
            Delivery::Delivered
        );
        assert_eq!(
            display_delivery(Delivery::Played, true, ChatKind::Direct, true),
            Delivery::Played
        );
        assert_eq!(
            display_delivery(Delivery::Read, true, ChatKind::Group, true),
            Delivery::Read
        );
        assert_eq!(
            display_delivery(Delivery::Read, false, ChatKind::Direct, true),
            Delivery::Read
        );
        assert_eq!(
            display_delivery(Delivery::Read, true, ChatKind::Direct, false),
            Delivery::Read
        );
    }

    #[test]
    fn account_privacy_preserves_non_read_states_and_broadcasts() {
        use crate::model::ChatKind;
        for status in [
            Delivery::None,
            Delivery::Pending,
            Delivery::Sent,
            Delivery::Delivered,
            Delivery::Played,
            Delivery::Failed,
        ] {
            assert_eq!(
                display_delivery(status, true, ChatKind::Direct, true),
                status
            );
        }
        assert_eq!(
            display_delivery(Delivery::Read, true, ChatKind::Broadcast, true),
            Delivery::Read
        );
    }

    #[test]
    fn batch_revoke_requires_every_selected_message_to_be_eligible() {
        let now = crate::util::now();
        let first = Message {
            id: "first".into(),
            chat: "synthetic-chat".into(),
            sender: "me".into(),
            sender_name: None,
            from_me: true,
            timestamp: now,
            history_order: None,
            content: Content::text("Synthetic message"),
            status: Delivery::Sent,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: Vec::new(),
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        };
        let mut second = first.clone();
        second.id = "second".into();
        let selected = vec!["first".into(), "second".into()];
        assert!(selection_can_revoke(
            &[first.clone(), second.clone()],
            &selected,
            now
        ));
        second.from_me = false;
        assert!(!selection_can_revoke(
            &[first.clone(), second.clone()],
            &selected,
            now
        ));
        second.from_me = true;
        second.timestamp = now - crate::app::REVOKE_WINDOW.as_secs() as i64 - 1;
        assert!(!selection_can_revoke(
            &[first.clone(), second.clone()],
            &selected,
            now
        ));
        second.timestamp = now;
        second.content = Content::Revoked;
        assert!(!selection_can_revoke(
            &[first.clone(), second],
            &selected,
            now
        ));
        assert!(!selection_can_revoke(&[first], &selected, now));
        assert!(!selection_can_revoke(&[], &[], now));
    }

    #[test]
    fn sender_pictures_show_in_groups_only() {
        let chat = |id: &str| Chat::new(id.into(), "Chat".into());
        assert!(shows_sender_pictures(&chat("120363012345678901@g.us")));
        assert!(!shows_sender_pictures(&chat("393331234567@s.whatsapp.net")));
        assert!(!shows_sender_pictures(&chat(
            "120363055566677788@newsletter"
        )));
    }

    fn contact(name: &str, number: &str, account: Option<&str>) -> Option<SharedContact> {
        Some(SharedContact {
            name: name.to_owned(),
            number: number.to_owned(),
            account: account.map(str::to_owned),
            first_name: None,
        })
    }

    #[test]
    fn a_shared_contact_keeps_the_first_name_of_its_card() {
        let first = |card: &str| {
            shared_contact_details(card, "Fallback")
                .expect("a contact")
                .first_name
        };
        // A first name of two words stays whole, as the card has it (#314).
        assert_eq!(
            first("BEGIN:VCARD\nN:;My Dih;;;\nFN:My Dih\nTEL:+15550101234\nEND:VCARD").as_deref(),
            Some("My Dih")
        );
        assert_eq!(
            first("BEGIN:VCARD\nN:Evans;Mary;Ann;;\nTEL:+15550101234\nEND:VCARD").as_deref(),
            Some("Mary Ann")
        );
        assert_eq!(
            first("BEGIN:VCARD\nN:Smith;Ada\\;Jo;;\nTEL:+15550101234\nEND:VCARD").as_deref(),
            Some("Ada;Jo")
        );
        assert_eq!(
            first("BEGIN:VCARD\nN:山田;太郎;;;\nTEL:+15550101234\nEND:VCARD").as_deref(),
            Some("太郎")
        );
        // Without a structured first name nothing is known.
        assert_eq!(
            first("BEGIN:VCARD\nN:Evans;;;;\nTEL:+15550101234\nEND:VCARD"),
            None
        );
        assert_eq!(
            first("BEGIN:VCARD\nFN:Ada Lovelace\nTEL:+15550101234\nEND:VCARD"),
            None
        );
    }

    #[test]
    fn shared_contact_accepts_case_insensitive_vcard_property_names() {
        assert_eq!(
            shared_contact_details(
                "BEGIN:VCARD\nversion:3.0\nitem1.fn:Case-insensitive name\nitem1.tel;type=cell;waid=1234567:+1 234567\nEND:VCARD",
                "Fallback",
            ),
            contact("Case-insensitive name", "+1 234567", Some("1234567"))
        );
    }

    #[test]
    fn shared_contact_uses_first_valid_telephone_when_multiple_are_present() {
        assert_eq!(
            shared_contact_details(
                "BEGIN:VCARD\nFN:Two numbers\nTEL;TYPE=HOME:+1111111\nTEL;TYPE=CELL:+2222222\nEND:VCARD",
                "Fallback",
            ),
            contact("Two numbers", "+1111111", Some("1111111"))
        );
    }

    #[test]
    fn shared_contact_prefers_lowest_vcard_tel_preference() {
        assert_eq!(
            shared_contact_details(
                "BEGIN:VCARD\nFN:Preferred number\nTEL;TYPE=HOME:+1111111\nitem1.TEL;TYPE=CELL;PREF=2:+2222222\nTEL;TYPE=WORK;PREF=1:+3333333\nEND:VCARD",
                "Fallback",
            ),
            contact("Preferred number", "+3333333", Some("3333333"))
        );
    }

    #[test]
    fn shared_contact_prefers_vcard_name_and_reads_the_account_from_waid() {
        assert_eq!(
            shared_contact_details(
                "BEGIN:VCARD\nVERSION:3.0\nFN:Alice Example\nitem1.TEL;waid=15550101234:+1 (555) 010-1234\nEND:VCARD",
                "Contact from sender",
            ),
            contact("Alice Example", "+1 (555) 010-1234", Some("15550101234"))
        );
    }

    #[test]
    fn a_local_number_names_no_account_and_stays_readable() {
        // Without a country code the digits are not a WhatsApp account.
        assert_eq!(
            shared_contact_details(
                "BEGIN:VCARD\nFN:Local\nTEL;TYPE=CELL:0171 1234567\nEND:VCARD",
                "Fallback",
            ),
            contact("Local", "0171 1234567", None)
        );
    }

    #[test]
    fn shared_contact_without_vcard_name_uses_message_name_and_rejects_missing_phone() {
        assert_eq!(
            shared_contact_details("BEGIN:VCARD\nTEL:+1234567\nEND:VCARD", "Shared person"),
            contact("Shared person", "+1234567", Some("1234567"))
        );
        assert_eq!(
            shared_contact_details("BEGIN:VCARD\nFN:No phone\nEND:VCARD", "Fallback"),
            None
        );
    }

    #[test]
    fn shared_contact_details_use_the_first_vcard_when_multiple_contacts_are_shared() {
        assert_eq!(
            shared_contact_details(
                "BEGIN:VCARD\nFN:Alice\nTEL:+15550101234\nEND:VCARD\nBEGIN:VCARD\nFN:Bob\nTEL:+15550105678\nEND:VCARD",
                "Several contacts",
            ),
            contact("Alice", "+15550101234", Some("15550101234"))
        );
    }

    #[test]
    fn saved_attachments_suggest_a_plain_file_name() {
        let document = |file_name: &str| Content::Document {
            media: media(None, None),
            file_name: file_name.into(),
            caption: None,
            pages: None,
        };
        let cached = Path::new("/cache/media/abc123.pdf");
        assert_eq!(attachment_name(&document("Notes.pdf"), cached), "Notes.pdf");
        assert_eq!(
            attachment_name(&document("../../.bashrc"), cached),
            ".._.._.bashrc"
        );
        assert_eq!(attachment_name(&document("  "), cached), "abc123.pdf");
    }

    /// Builds synthetic attachment metadata for media-state and album regression tests.
    fn media(w: Option<u32>, h: Option<u32>) -> Media {
        Media {
            mime: "image/jpeg".into(),
            size: 1,
            width: w,
            height: h,
            album: None,
            path: None,
            state: MediaState::Idle,
        }
    }

    /// Builds a synthetic picture belonging to a specified protocol album.
    fn album_picture(id: &str, album: Option<&str>, sender: &str) -> Message {
        let mut attachment = media(Some(640), Some(480));
        attachment.album = album.map(str::to_owned);
        Message {
            id: id.into(),
            chat: "chat@example".into(),
            sender: sender.into(),
            sender_name: None,
            from_me: false,
            timestamp: 1,
            history_order: None,
            content: Content::Image {
                motion: None,
                caption: None,
                media: attachment,
            },
            status: Delivery::None,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: Vec::new(),
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        }
    }

    /// Checks that consecutive compatible media sharing an album parent are displayed together.
    #[test]
    fn consecutive_items_with_the_same_protocol_album_are_grouped() {
        let messages = vec![
            album_picture("one", Some("album-1"), "alice"),
            album_picture("two", Some("album-1"), "alice"),
            album_picture("three", Some("album-2"), "alice"),
        ];
        assert_eq!(album_run(&messages, 0).map(<[Message]>::len), Some(2));
        assert!(album_run(&messages, 2).is_none());
    }

    /// A protocol album splits at the local day boundary even when its identity stays the same.
    #[test]
    fn album_runs_stop_at_the_local_calendar_day_boundary() {
        let (_, midnight) = crate::util::day_bounds(jiff::civil::date(2026, 9, 20)).unwrap();
        let mut messages: Vec<_> = ["one", "two", "three", "four"]
            .iter()
            .map(|id| album_picture(id, Some("album-1"), "alice"))
            .collect();
        for (message, offset) in messages.iter_mut().zip([-2, -1, 1, 2]) {
            message.timestamp = midnight + offset;
        }
        assert_eq!(album_run(&messages, 0).map(<[Message]>::len), Some(2));
        assert_eq!(album_run(&messages, 2).map(<[Message]>::len), Some(2));
        assert!(album_run(&messages[1..3], 0).is_none());
    }

    /// Checks that grouping never combines unrelated pictures or messages from different senders.
    #[test]
    fn unrelated_or_different_sender_pictures_are_not_grouped() {
        let unrelated = vec![
            album_picture("one", None, "alice"),
            album_picture("two", None, "alice"),
        ];
        assert!(album_run(&unrelated, 0).is_none());
        let senders = vec![
            album_picture("one", Some("album-1"), "alice"),
            album_picture("two", Some("album-1"), "bob"),
        ];
        assert!(album_run(&senders, 0).is_none());
    }

    fn message(content: Content) -> Message {
        Message {
            id: "m1".into(),
            chat: "peer@s.whatsapp.net".into(),
            sender: "me".into(),
            sender_name: None,
            from_me: true,
            timestamp: 0,
            content,
            status: Delivery::None,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: Vec::new(),
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
            history_order: None,
        }
    }

    /// The menu offers the viewer for a message the album takes and a file
    /// that is still on disk. A recorded path whose file was deleted elsewhere
    /// offers nothing, or the click would fall back to opening a file that is
    /// not there.
    #[test]
    fn the_viewer_item_needs_a_file_that_is_still_here() {
        let dir = tempfile::tempdir().unwrap();
        let picture = |path: &Path| {
            message(Content::Image {
                media: Media {
                    path: Some(path.to_path_buf()),
                    ..media(None, None)
                },
                caption: None,
                motion: None,
            })
        };

        let photo = dir.path().join("photo.png");
        std::fs::write(&photo, b"jpeg").unwrap();
        let downloaded = picture(&photo);
        assert_eq!(viewer_file(&downloaded), Some(photo.as_path()));

        let deleted = dir.path().join("deleted.png");
        let gone = picture(&deleted);
        assert_eq!(
            viewer_file(&gone),
            None,
            "a path with no file offers nothing"
        );

        // A GIF plays inline as an animation, so the album leaves it out.
        let animation = dir.path().join("animation.gif");
        std::fs::write(&animation, b"gif").unwrap();
        let inline = picture(&animation);
        assert_eq!(viewer_file(&inline), None);

        // Neither does a message whose file was never downloaded.
        let pending = message(Content::Image {
            media: media(None, None),
            caption: None,
            motion: None,
        });
        assert_eq!(viewer_file(&pending), None);
    }

    #[test]
    fn picture_placeholder_matches_decoded_dimensions() {
        for limit in [200.0, PICTURE_WIDTH] {
            for (width, height) in [(900, 1200), (600, 1600), (1600, 900)] {
                assert_eq!(
                    frame_size(
                        &media(Some(width), Some(height)),
                        None,
                        limit,
                        PICTURE_HEIGHT
                    ),
                    fit_picture(width as f32, height as f32, limit, PICTURE_HEIGHT),
                    "decoding a {width}x{height} photo must not shift the transcript"
                );
            }
        }
    }

    #[test]
    fn reaction_affordance_action_targets_the_message_it_belongs_to() {
        let action = open_reaction_picker_action("chat@example", "message-42", false);
        assert!(matches!(
            action,
            Action::OpenReactionPicker { chat, message, beside_menu: false, from_list: false }
                if chat == "chat@example" && message == "message-42"
        ));
    }

    #[test]
    fn reaction_affordance_sits_beside_the_bubble_without_clipping() {
        let bounds = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 300.0));
        let incoming = Rect::from_min_max(pos2(20.0, 20.0), pos2(140.0, 80.0));
        let outgoing = Rect::from_min_max(pos2(260.0, 20.0), pos2(380.0, 80.0));

        let incoming_button = reaction_affordance_rect(incoming, bounds, false);
        let outgoing_button = reaction_affordance_rect(outgoing, bounds, true);

        assert!(incoming_button.left() > incoming.right());
        assert!(outgoing_button.right() < outgoing.left());
        assert!(bounds.contains_rect(incoming_button));
        assert!(bounds.contains_rect(outgoing_button));

        let edge = Rect::from_min_max(pos2(360.0, 260.0), pos2(398.0, 298.0));
        assert!(bounds.contains_rect(reaction_affordance_rect(edge, bounds, false)));
    }

    #[test]
    fn reaction_affordance_stays_visible_while_crossing_to_its_button() {
        let bubble = Rect::from_min_max(pos2(20.0, 20.0), pos2(140.0, 80.0));
        let button = Rect::from_min_max(pos2(146.0, 24.0), pos2(172.0, 50.0));

        assert!(reaction_affordance_visible(
            Some(bubble.center()),
            bubble,
            button
        ));
        assert!(reaction_affordance_visible(
            Some(button.center()),
            bubble,
            button
        ));
        assert!(!reaction_affordance_visible(
            Some(pos2(300.0, 200.0)),
            bubble,
            button
        ));
        assert!(!reaction_affordance_visible(None, bubble, button));
    }

    /// One pinned row, as the archive hands it over.
    fn pinned(chat: &str, id: &str) -> crate::archive::Pinned {
        crate::archive::Pinned {
            chat: chat.to_owned(),
            id: id.to_owned(),
            pinned_at: 1,
            expires_at: i64::MAX,
            text: format!("message {id}"),
            from_me: false,
            sent_at: 1,
            pinner: crate::archive::Pinner {
                by_me: true,
                sender: String::new(),
            },
        }
    }

    #[test]
    fn a_pin_notice_names_who_pinned_the_message() {
        let names = |id: &str, _: Option<&str>| id.split('@').next().unwrap_or(id).to_owned();
        let mut row = pinned("1@s.whatsapp.net", "m1");
        assert_eq!(
            pin_notice_label(crate::i18n::Locale::English, &row, &names),
            "You pinned a message"
        );

        row.pinner = crate::archive::Pinner {
            by_me: false,
            sender: "bob@s.whatsapp.net".to_owned(),
        };
        assert_eq!(
            pin_notice_label(crate::i18n::Locale::English, &row, &names),
            "bob pinned a message"
        );

        // An archive that does not say who pinned it still shows the pin.
        row.pinner.sender.clear();
        assert_eq!(
            pin_notice_label(crate::i18n::Locale::English, &row, &names),
            "A message was pinned"
        );
    }

    #[test]
    fn an_expired_pin_leaves_the_line_under_the_header() {
        let mut gone = pinned("1@s.whatsapp.net", "m2");
        gone.expires_at = 1_000;
        let mut live = pinned("1@s.whatsapp.net", "m1");
        live.expires_at = 2_000;

        let rows = live_pins(&[gone, live.clone()], 1_500);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "m1");
        assert!(
            live_pins(&[live], 2_000).is_empty(),
            "the moment it runs out, it leaves"
        );
    }

    #[test]
    fn an_open_pin_asks_for_a_frame_when_it_expires() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(directory.path()),
            crate::settings::Settings::default(),
        );
        let chat = Chat::new("1@s.whatsapp.net".into(), "Ada".into());
        let mut row = pinned(&chat.id, "m1");
        row.expires_at = crate::util::now() + 30;
        app.chat_pins.insert(chat.id.clone(), vec![row]);
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            pin_banner(&mut app, ui, &chat);
        });
        output.textures_delta.clear();
        let delay = output
            .viewport_output
            .values()
            .map(|viewport| viewport.repaint_delay)
            .min()
            .unwrap_or(std::time::Duration::MAX);
        assert!(
            delay.as_secs() <= 31,
            "the banner wakes when the pin runs out"
        );
    }

    #[test]
    fn a_pin_notice_sits_above_the_message_that_followed_its_pin() {
        // `chat_pins` hands the newest pin over first. A pin at 100 and one at
        // 500 belong above the messages at 200 and 600, one each.
        let mut newest_first = vec![
            pinned("1@s.whatsapp.net", "c"),
            pinned("1@s.whatsapp.net", "a"),
        ];
        newest_first[0].pinned_at = 500;
        newest_first[1].pinned_at = 100;

        let ordered = pin_notices_in_time_order(&newest_first);
        assert_eq!(
            ordered.iter().map(|row| row.pinned_at).collect::<Vec<_>>(),
            [100, 500],
            "the list goes back into time order before it is placed"
        );
        assert_eq!(notices_above(&ordered[..], 200), 1, "one notice above 200");
        assert_eq!(notices_above(&ordered[1..], 600), 1, "and one above 600");

        // Handed over unsorted, both would pile above the later message, which
        // is the bug this keeps out.
        assert_eq!(notices_above(&newest_first[..], 200), 0);
    }

    #[test]
    fn the_pin_line_shows_the_newest_pin_and_a_click_walks_the_rest() {
        let rows = vec![
            pinned("1@s.whatsapp.net", "newest"),
            pinned("1@s.whatsapp.net", "middle"),
            pinned("1@s.whatsapp.net", "oldest"),
        ];
        // Nothing shown yet, or the shown pin gone: the newest.
        assert_eq!(shown_pin(&rows, None), 0);
        assert_eq!(shown_pin(&rows, Some("unpinned")), 0);
        assert_eq!(shown_pin(&rows, Some("middle")), 1);
        // Each click moves on to the next older pin and wraps to the newest.
        assert_eq!(next_pin(3, 0), 1);
        assert_eq!(next_pin(3, 1), 2);
        assert_eq!(next_pin(3, 2), 0);
        // One pin is its own next pin.
        assert_eq!(next_pin(1, 0), 0);
    }

    #[test]
    fn the_pin_bar_has_a_segment_per_pin_inside_the_line() {
        for count in 1..=3 {
            let segments = pin_bar_segments(10.0, 28.0, count);
            assert_eq!(segments.len(), count);
            assert!((segments[0].0 - 10.0).abs() < 0.01);
            assert!((segments[count - 1].1 - 38.0).abs() < 0.01, "{segments:?}");
            for pair in segments.windows(2) {
                assert!(pair[1].0 > pair[0].1, "segments do not touch: {segments:?}");
            }
        }
    }

    #[test]
    fn a_click_on_the_pin_line_opens_the_pin_and_shows_the_next() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(directory.path()),
            crate::settings::Settings::default(),
        );
        let chat = Chat::new("1@s.whatsapp.net".into(), "Ada".into());
        app.chat_pins.insert(
            chat.id.clone(),
            vec![pinned(&chat.id, "newest"), pinned(&chat.id, "older")],
        );
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let screen = Rect::from_min_size(egui::Pos2::ZERO, vec2(600.0, 400.0));
        // Clicks a second apart, so none of them counts as a double click.
        let clock = std::cell::Cell::new(0.0);
        let frame = |events: Vec<egui::Event>, app: &mut App, step: f64| {
            clock.set(clock.get() + step);
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(clock.get()),
                    events,
                    ..Default::default()
                },
                |ui| pin_banner(app, ui, &chat),
            );
            output.textures_delta.clear();
        };
        let at = pos2(300.0, PIN_ROW_HEIGHT / 2.0);
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        // The line has to be laid out once before it can take a click.
        frame(vec![egui::Event::PointerMoved(at)], &mut app, 1.0);
        let mut opened = Vec::new();
        for _ in 0..3 {
            frame(
                vec![egui::Event::PointerMoved(at), press(true)],
                &mut app,
                1.0,
            );
            frame(vec![press(false)], &mut app, 0.05);
            for action in app.actions.drain(..) {
                if let Action::OpenMessage { message, .. } = action {
                    opened.push(message);
                }
            }
        }
        assert_eq!(opened, ["newest", "older", "newest"]);
    }

    #[test]
    fn the_footer_takes_a_slot_only_for_the_marks_on_the_bubble() {
        let ctx = egui::Context::default();
        let mut clock = 0.0;
        let mut plain = 0.0;
        let mut pinned = 0.0;
        let mut both = 0.0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 200.0))),
                ..Default::default()
            },
            |ui| {
                let message = crate::archive::tests::message("1@s.whatsapp.net", "m1", 0, false);
                clock = clock_width(ui, &message);
                plain = footer_width(ui, &message, false, false);
                pinned = footer_width(ui, &message, false, true);
                both = footer_width(ui, &message, true, true);
            },
        );
        output.textures_delta.clear();
        // Measured against the clock the reader's own format produces, not a
        // number that only holds on a 24-hour one: the same footer is a third
        // wider where the system shows "12:00 AM".
        assert!(
            (plain - clock).abs() < 0.01,
            "a message without a mark is just the clock, {plain} vs {clock}"
        );
        assert!(
            (pinned - clock - FOOTER_MARK_SLOT).abs() < 0.5,
            "a pin takes one slot, {pinned} vs {clock}"
        );
        assert!(
            (both - clock - FOOTER_MARK_SLOT * 2.0).abs() < 0.5,
            "a star and a pin take one slot each, {both} vs {clock}"
        );
    }

    #[test]
    fn the_footer_paints_a_pin_mark_when_the_message_is_pinned() {
        let ctx = egui::Context::default();
        with_still_view(|view| {
            let shapes = |pinned: bool| {
                let message = crate::archive::tests::message("1@s.whatsapp.net", "m1", 0, false);
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            egui::Pos2::ZERO,
                            vec2(400.0, 200.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        footer(ui, view, &message, None, false, pinned, &mut Vec::new());
                    },
                );
                output.textures_delta.clear();
                output.shapes.len()
            };
            assert_eq!(
                shapes(true),
                shapes(false) + 1,
                "a pinned message paints one extra mark"
            );
        });
    }

    #[test]
    fn picture_frames_keep_their_shape_within_the_limit() {
        let landscape = frame_size(&media(Some(1600), Some(1200)), None, 340.0, PICTURE_HEIGHT);
        assert!((landscape.x - 340.0).abs() < 0.01);
        assert!((landscape.y - 255.0).abs() < 0.01);
        let tall = frame_size(&media(Some(600), Some(1200)), None, 340.0, PICTURE_HEIGHT);
        assert!(tall.y > 340.0 && tall.y <= PICTURE_HEIGHT);
        let exact = fit_picture(900.0, 1600.0, PICTURE_WIDTH, PICTURE_HEIGHT);
        assert!((exact.y - PICTURE_HEIGHT).abs() < 0.01);
        assert!(exact.x < PICTURE_WIDTH);
        let unknown = frame_size(&media(None, None), Some((16, 9)), 340.0, PICTURE_HEIGHT);
        assert!(unknown.x > unknown.y);
        let tiny = frame_size(&media(Some(40), Some(40)), None, 340.0, PICTURE_HEIGHT);
        assert!(tiny.x >= 120.0);
    }

    #[test]
    fn the_footer_reserves_a_slot_only_when_the_message_is_starred() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut clock = 0.0;
        let mut incoming = 0.0;
        let mut starred = 0.0;
        let mut own = 0.0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 200.0))),
                ..Default::default()
            },
            |ui| {
                let from_them = crate::archive::tests::message("1@s.whatsapp.net", "m1", 0, false);
                let from_me = crate::archive::tests::message("1@s.whatsapp.net", "m1", 0, true);
                clock = clock_width(ui, &from_them);
                incoming = footer_width(ui, &from_them, false, false);
                starred = footer_width(ui, &from_them, true, false);
                own = footer_width(ui, &from_me, false, false);
            },
        );
        output.textures_delta.clear();
        // Measured against the clock the reader's own format produces, not a
        // number that only holds on a 24-hour one: the same footer is a third
        // wider where the system shows "12:00 AM".
        assert!(
            (incoming - clock).abs() < 0.01,
            "a plain incoming footer is just the clock, {incoming} vs {clock}"
        );
        assert!(
            (starred - clock - FOOTER_MARK_SLOT).abs() < 0.5,
            "a star adds one mark slot, {starred} vs {clock}"
        );
        assert!(
            (own - incoming - 19.0).abs() < 0.5,
            "our own messages add the delivery ticks, {own} vs {incoming}"
        );
    }

    /// Runs `check` with a still view of one plain chat, as a footer test
    /// needs one to paint into.
    fn with_still_view(check: impl FnOnce(&View<'_>)) {
        let directory = tempfile::tempdir().unwrap();
        let (app, _events) = App::headless(
            crate::paths::AppDirs::under(directory.path()),
            crate::settings::Settings::default(),
        );
        let chat = Chat::new("1@s.whatsapp.net".into(), "Ada".into());
        let names_or = |id: &str, hint: Option<&str>| app.display_name_or(id, hint);
        let mention_names = |id: &str| app.mention_name(id);
        let avatars = HashMap::new();
        let keyboard_navigation = std::cell::Cell::new(false);
        let view = View {
            palette: Palette::dark(),
            locale: app.locale,
            chat: &chat,
            me: None,
            auto_download: app.settings.auto_download,
            connected: false,
            poll_voting: &app.poll_voting,
            interactive_pending: &app.interactive_sending,
            anchor: None,
            open_menu: None,
            reaction: None,
            reaction_menu: false,
            reaction_emoji: &app.settings.reaction_emoji,
            keyboard_navigation: &keyboard_navigation,
            names_or: &names_or,
            mention_names: &mention_names,
            avatars: &avatars,
            contacts: &app.contacts,
            now: 0,
            animate: false,
            player: &app.player,
            voice_speed: app.settings.voice_speed,
            video: &app.video,
            account_receipts_off: false,
            copy_rows: Some(app.copy_rows.as_ref()),
            selecting: false,
            motion_playing: None,
            starred: None,
            pins: None,
            pin_notices: &[],
        };
        check(&view);
    }

    #[test]
    fn the_footer_paints_a_star_mark_when_the_message_is_starred() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        with_still_view(|view| {
            let shapes = |starred: bool| {
                let message = crate::archive::tests::message("1@s.whatsapp.net", "m1", 0, false);
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            egui::Pos2::ZERO,
                            vec2(400.0, 200.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        footer(ui, view, &message, None, starred, false, &mut Vec::new());
                    },
                );
                output.textures_delta.clear();
                output.shapes.len()
            };
            assert_eq!(shapes(true), shapes(false) + 1, "a star paints one mark");
        });
    }

    #[test]
    fn a_picture_without_a_caption_paints_the_marks_it_carries() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        with_still_view(|view| {
            let shapes = |starred: bool, pinned: bool| {
                let message = crate::archive::tests::message("1@s.whatsapp.net", "m1", 0, false);
                let picture = Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 180.0));
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            egui::Pos2::ZERO,
                            vec2(400.0, 200.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        footer_over_picture(
                            ui,
                            view,
                            &message,
                            picture,
                            starred,
                            pinned,
                            &mut Vec::new(),
                        );
                    },
                );
                output.textures_delta.clear();
                output.shapes.len()
            };
            assert_eq!(
                shapes(true, false),
                shapes(false, false) + 1,
                "a star on a picture paints one mark"
            );
            assert_eq!(
                shapes(false, true),
                shapes(false, false) + 1,
                "a pin on a picture paints one mark"
            );
            assert_eq!(
                shapes(true, true),
                shapes(false, false) + 2,
                "both marks on a picture paint both"
            );
        });
    }

    #[test]
    fn the_star_mark_tint_is_not_the_secondary() {
        let palette = Palette::dark();
        assert_ne!(FOOTER_STAR, palette.secondary);
    }

    #[test]
    fn a_failed_footer_reserves_room_for_its_label() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut message = Message {
            id: "fixture".into(),
            chat: "1@s.whatsapp.net".into(),
            sender: "me@s.whatsapp.net".into(),
            sender_name: None,
            from_me: true,
            timestamp: 1000,
            content: Content::text("Fixture"),
            status: Delivery::Sent,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: Vec::new(),
            history_order: None,
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        };
        let mut widths = Vec::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            widths.push(footer_width(ui, &message, false, false));
            message.status = Delivery::Failed;
            widths.push(footer_width(ui, &message, false, false));
            // Only our own messages can fail to send.
            message.from_me = false;
            widths.push(footer_width(ui, &message, false, false) + 19.0);
        });
        output.textures_delta.clear();
        assert!(widths[1] > widths[0] + 30.0, "{widths:?}");
        assert_eq!(widths[2], widths[0]);
    }

    #[test]
    fn visible_stickers_download_automatically_with_the_attachment_setting_off() {
        let mut sticker = media(Some(180), Some(180));
        assert!(auto_download_allowed(&sticker, true, false));
        assert!(!auto_download_allowed(&sticker, false, false));
        sticker.size = crate::model::ATTACHMENT_DOWNLOAD_LIMIT + 1;
        assert!(!auto_download_allowed(&sticker, true, false));
    }

    /// A document reads as not downloaded only when its card will not fetch
    /// it by itself: with auto-download on and within the size limit it will.
    #[test]
    fn a_document_reads_as_not_downloaded_only_when_its_card_waits() {
        let document = |size: u64| Content::Document {
            media: Media {
                mime: "application/pdf".into(),
                size,
                ..media(None, None)
            },
            file_name: "Notes.pdf".into(),
            caption: None,
            pages: None,
        };
        let state = |content: &Content, auto_download: bool| {
            crate::a11y::attachment_state(
                crate::i18n::Locale::English,
                content,
                download_starts_by_itself(content, auto_download),
            )
        };
        let within = document(482_113);
        let over = document(crate::model::ATTACHMENT_DOWNLOAD_LIMIT + 1);
        assert_eq!(state(&within, true), None);
        assert_eq!(state(&within, false).as_deref(), Some("not downloaded"));
        assert_eq!(state(&over, true).as_deref(), Some("not downloaded"));
        assert_eq!(state(&over, false).as_deref(), Some("not downloaded"));
    }

    /// The typing dots animate at the display's rate, not a fixed timer.
    #[test]
    fn typing_dots_repaint_every_frame() {
        let ctx = egui::Context::default();
        let palette = crate::theme::Palette::dark();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| typing_dots(ui, &palette));
        output.textures_delta.clear();
        let delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        assert_eq!(delay, std::time::Duration::ZERO);
    }

    /// With a screen reader attached the dots ask for a frame a step later,
    /// and each stepped frame draws exactly what an every-frame one would at
    /// that moment, so the wave keeps its speed and only skips frames.
    #[test]
    fn typing_dots_step_with_a_screen_reader_and_keep_their_phase() {
        let palette = crate::theme::Palette::dark();
        let draw = |reader: bool, time: f64| {
            let ctx = crate::motion::tests::reading(reader);
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ui| typing_dots(ui, &palette),
            );
            output.textures_delta.clear();
            let delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
            let dots: Vec<_> = output
                .shapes
                .into_iter()
                .filter_map(|clipped| match clipped.shape {
                    egui::Shape::Circle(circle) => Some((circle.center, circle.fill)),
                    _ => None,
                })
                .collect();
            (delay, dots)
        };
        let mut seen = Vec::new();
        for step in 0..12 {
            let time = 5.0 + f64::from(step) * 0.1;
            let (plain_delay, plain) = draw(false, time);
            let (stepped_delay, stepped) = draw(true, time);
            assert_eq!(plain_delay, std::time::Duration::ZERO);
            assert!(
                stepped_delay > std::time::Duration::from_millis(75)
                    && stepped_delay <= crate::motion::STEP,
                "{stepped_delay:?}"
            );
            assert_eq!(stepped.len(), 3);
            assert_eq!(plain, stepped, "at {time}");
            seen.push(format!("{stepped:?}"));
        }
        // Over one 1.2 s wave the steps show it moving: only while all three
        // dots rest at the bottom do two steps look alike.
        seen.sort();
        seen.dedup();
        assert!(seen.len() >= 9, "{} different steps", seen.len());
    }

    #[test]
    fn composer_triggers_only_start_at_word_boundaries() {
        assert_eq!(standalone_trigger(":", 1, ':'), Some(0));
        assert_eq!(standalone_trigger("hello :", 7, ':'), Some(6));
        assert_eq!(standalone_trigger("hello @", 7, '@'), Some(6));
        assert_eq!(standalone_trigger("19:30", 3, ':'), None);
        assert_eq!(standalone_trigger("mail@example.com", 5, '@'), None);
    }

    #[test]
    fn mention_query_runs_from_the_at_to_the_cursor() {
        assert_eq!(active_mention("hello @mi", Some(6), 9), Some((9, "mi")));
        assert_eq!(active_mention("hello @mi\n", Some(6), 10), None);
        assert_eq!(active_mention("hello @mi", Some(99), 9), None);
    }

    #[test]
    fn emoji_query_ends_at_spaces_and_punctuation() {
        assert_eq!(active_emoji("hello :gri", Some(6), 10), Some((10, "gri")));
        assert_eq!(
            active_emoji("hello :gri_ning", Some(6), 15),
            Some((15, "gri_ning"))
        );
        assert_eq!(active_emoji("hello :gri ", Some(6), 11), None);
        assert_eq!(active_emoji("hello :gri!", Some(6), 11), None);
        assert_eq!(active_emoji("hello gri", Some(6), 9), None);
    }

    #[test]
    fn emoji_shortcodes_rank_ahead_of_name_matches() {
        let grinning = emojis::get("😀").expect("known emoji");
        assert_eq!(emoji_match_score(grinning, "grinning"), Some(0));
        assert_eq!(emoji_match_score(grinning, "grin"), Some(1));
        assert_eq!(emoji_match_score(grinning, "face"), Some(3));
        assert_eq!(emoji_match_score(grinning, "rocket"), None);
    }

    #[test]
    fn completion_offers_skin_tone_capable_emoji() {
        let directory = tempfile::tempdir().unwrap();
        let (app, _events) = App::headless(
            crate::paths::AppDirs::under(directory.path()),
            crate::settings::Settings::default(),
        );
        let offers = |query: &str, emoji: &str| {
            emoji_candidates(&app, query)
                .iter()
                .any(|suggestion| suggestion.emoji == emoji)
        };
        assert!(offers("pregnant", "🤰"));
        assert!(offers("thumbsup", "👍"));
    }
}

#[cfg(test)]
mod reaction_tests {
    use super::*;
    use crate::model::{Delivery, Reaction};

    fn with_reactions(reactions: Vec<Reaction>) -> Message {
        Message {
            id: "m1".into(),
            chat: "a@s.whatsapp.net".into(),
            sender: "a@s.whatsapp.net".into(),
            sender_name: None,
            from_me: false,
            timestamp: 0,
            content: Content::text("hi"),
            status: Delivery::None,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions,
            history_order: None,
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        }
    }

    fn reaction(from_me: bool, emoji: &str) -> Reaction {
        Reaction {
            sender: if from_me { "me" } else { "them" }.into(),
            from_me,
            emoji: emoji.into(),
        }
    }

    /// The tail goes to a run's last bubble, past the messages drawn without
    /// one after it, and only to one in the same run.
    #[test]
    fn the_tail_goes_to_the_last_bubble_of_a_run() {
        let photo = Message {
            id: "photo".into(),
            content: Content::Image {
                caption: None,
                media: crate::model::Media {
                    album: None,
                    mime: "image/jpeg".into(),
                    size: 1,
                    width: None,
                    height: None,
                    path: None,
                    state: crate::model::MediaState::Idle,
                },
                motion: None,
            },
            ..with_reactions(Vec::new())
        };
        let text = with_reactions(Vec::new());
        let mine = Message {
            from_me: true,
            ..with_reactions(Vec::new())
        };
        let messages = Palette::messages();
        // Text, then a lone photo in the same run: the text keeps the tail.
        let run = [text.clone(), photo.clone()];
        assert!(last_bubble_of_run(&messages, &run, 0));
        // Text, text: only the second.
        let run = [text.clone(), text.clone()];
        assert!(!last_bubble_of_run(&messages, &run, 0));
        assert!(last_bubble_of_run(&messages, &run, 1));
        // A photo, then text of the other side: the run ends, the text tails.
        let run = [text.clone(), photo.clone(), mine, text.clone()];
        assert!(last_bubble_of_run(&messages, &run, 0));
        // WhatsApp frames the photo, so there it is the last bubble.
        assert!(!last_bubble_of_run(
            &Palette::dark(),
            &[text.clone(), photo.clone()],
            0
        ));
    }

    /// The avatar Messages puts beside a run goes to its last message,
    /// bubble or not, and a run ends with another sender or another day.
    #[test]
    fn a_run_ends_with_its_last_message() {
        let from = |sender: &str, timestamp: i64| Message {
            sender: sender.into(),
            timestamp,
            ..with_reactions(Vec::new())
        };
        let run = [
            from("a", 0),
            from("a", 60),
            from("b", 120),
            from("b", 2 * 86_400),
        ];
        assert!(!last_of_run(&run, 0));
        assert!(last_of_run(&run, 1));
        // Another day starts another run, from the same sender too.
        assert!(last_of_run(&run, 2));
        assert!(last_of_run(&run, 3));
    }

    /// Messages draws a photo alone in its message without a bubble, with
    /// a bubble's corners; anything else in the message keeps the bubble.
    /// A quote on your own Messages balloon is white on its blue, in both
    /// appearances; on others' balloons and in WhatsApp's style it keeps the
    /// quoted sender's colour.
    #[test]
    fn quotes_on_own_messages_balloons_are_white() {
        let tint = Color32::from_rgb(0x80, 0x40, 0xc0);
        for palette in [Palette::messages(), Palette::messages_light()] {
            let own = quote_colours(&palette.on_bubble(true), true, || tint);
            assert_eq!(own.name, Color32::WHITE);
            assert_eq!(own.text, Color32::WHITE.gamma_multiply(0.6));
            assert_eq!(own.bar, Color32::WHITE.gamma_multiply(0.6));
            assert_eq!(own.fill, Color32::WHITE.gamma_multiply(0.15));
            let theirs = quote_colours(&palette.on_bubble(false), false, || tint);
            assert_eq!((theirs.name, theirs.bar), (tint, tint));
        }
        for palette in [Palette::dark(), Palette::light()] {
            let own = quote_colours(&palette.on_bubble(true), true, || tint);
            assert_eq!((own.name, own.bar), (tint, tint));
            assert_eq!(own.fill, palette.window.gamma_multiply(0.35));
        }
    }

    #[test]
    fn only_lone_pictures_stand_without_a_bubble_in_messages() {
        let photo = |caption: Option<&str>| Message {
            content: Content::Image {
                caption: caption.map(Into::into),
                media: crate::model::Media {
                    album: None,
                    mime: "image/jpeg".into(),
                    size: 1,
                    width: None,
                    height: None,
                    path: None,
                    state: crate::model::MediaState::Idle,
                },
                motion: None,
            },
            ..with_reactions(Vec::new())
        };
        let messages = Palette::messages();
        assert!(bare_picture(&messages, &photo(None)));
        assert_eq!(
            picture_radius(&messages, &photo(None)),
            f32::from(widgets::MESSAGES_BUBBLE_RADIUS)
        );
        assert!(bare_picture(&Palette::messages_light(), &photo(None)));
        // WhatsApp's bubbles keep their frame around every picture.
        assert!(!bare_picture(&Palette::dark(), &photo(None)));
        assert_eq!(picture_radius(&Palette::dark(), &photo(None)), 6.0);
        // A caption, a quote or the forwarded label keep the bubble.
        assert!(!bare_picture(&messages, &photo(Some("Look"))));
        let mut forwarded = photo(None);
        forwarded.forwarded = true;
        assert!(!bare_picture(&messages, &forwarded));
        let mut quoting = photo(None);
        quoting.quoted = Some(crate::model::Quoted {
            labelled: None,
            chat: None,
            id: "q".into(),
            sender: "a@s.whatsapp.net".into(),
            sender_name: None,
            summary: "hi".into(),
            mentions: Vec::new(),
        });
        assert!(!bare_picture(&messages, &quoting));
        assert!(!bare_picture(&messages, &with_reactions(Vec::new())));
    }

    #[test]
    fn only_our_own_reaction_counts_as_chosen() {
        let message = with_reactions(vec![reaction(false, "😂"), reaction(true, "❤️")]);
        assert_eq!(own_reaction(&message), Some("❤️"));
        assert_eq!(quick_reactions(&message, &[]), QUICK_REACTIONS.to_vec());
        assert_eq!(
            own_reaction(&with_reactions(vec![reaction(false, "😂")])),
            None
        );
    }

    #[test]
    fn quick_reactions_start_with_preferences_and_fill_with_unique_defaults() {
        let message = with_reactions(Vec::new());
        let preferred = vec![("🦀".into(), 5), ("👍".into(), 2), ("invalid".into(), 1)];
        let quick = quick_reactions(&message, &preferred);
        assert_eq!(&quick[..2], &["🦀", "👍"]);
        assert_eq!(quick.len(), 6);
        assert_eq!(quick.iter().filter(|&&emoji| emoji == "👍").count(), 1);
    }

    #[test]
    fn an_unusual_reaction_of_ours_joins_the_quick_row() {
        let message = with_reactions(vec![reaction(true, "🦀")]);
        let quick = quick_reactions(&message, &[]);
        assert_eq!(quick.len(), QUICK_REACTIONS.len() + 1);
        assert_eq!(quick.last(), Some(&"🦀"));
    }

    #[test]
    fn reaction_choice_toggles_or_replaces_our_emoji() {
        assert_eq!(reaction_choice(None, "🦀"), "🦀");
        assert_eq!(reaction_choice(Some("🦀"), "🦀"), "");
        assert_eq!(reaction_choice(Some("👍"), "🦀"), "🦀");
        assert_eq!(reaction_choice(Some("❤️"), "❤️"), "");
    }

    #[test]
    fn another_senders_custom_reaction_is_kept_on_the_message() {
        let message = with_reactions(vec![reaction(false, "🏆")]);
        assert_eq!(own_reaction(&message), None);
        assert_eq!(message.reactions[0].emoji, "🏆");
        assert!(!message.reactions[0].from_me);
        assert_eq!(quick_reactions(&message, &[]), QUICK_REACTIONS.to_vec());
    }

    /// A message as a list row draws its menu: starred or not, pinned or not.
    struct MenuRow {
        message: Message,
        marks: Marks,
        pinned: bool,
    }

    /// The accessible labels of the message menu for `row`, drawn in
    /// `locale` the way a right-click draws it.
    fn drawn_menu_labels(locale: crate::i18n::Locale, row: &MenuRow) -> Vec<String> {
        let message = &row.message;
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(directory.path()),
            crate::settings::Settings::default(),
        );
        app.locale = locale;
        if row.pinned {
            app.pins
                .insert(message.chat.clone(), HashSet::from([message.id.clone()]));
        }
        let chat = Chat::new(message.chat.clone(), "Ada".into());
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::theme::install(&ctx);
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            list_menu(&app, ui, &chat, message, row.marks, &mut actions);
        });
        output.textures_delta.clear();
        let tree = output
            .platform_output
            .accesskit_update
            .expect("accessibility tree");
        tree.nodes
            .iter()
            .filter_map(|(_, node)| node.label().map(str::to_owned))
            .collect()
    }

    /// The labels of the message menu made of words: quick reactions are emoji.
    fn drawn_menu_words(locale: crate::i18n::Locale, row: &MenuRow) -> Vec<String> {
        drawn_menu_labels(locale, row)
            .into_iter()
            .filter(|label| label.chars().any(|c| c.is_alphabetic()))
            .collect()
    }

    /// Rows whose menus between them show every entry a list row can: our
    /// own fresh delivered text, starred and pinned, photos with and without
    /// their file and one downloading, a video and a sticker.
    fn menu_fixtures(file: &Path) -> Vec<MenuRow> {
        use MediaState::{Downloading, Idle};
        let mut own = with_reactions(Vec::new());
        own.from_me = true;
        own.timestamp = crate::util::now();
        own.status = Delivery::Delivered;
        let path = Some(file.to_path_buf());
        let media = |mime: &str, path: Option<PathBuf>, state| Media {
            mime: mime.into(),
            size: 1,
            width: None,
            height: None,
            album: None,
            path,
            state,
        };
        let row = |content: Content| {
            let mut message = with_reactions(Vec::new());
            message.content = content;
            MenuRow {
                message,
                marks: Marks { starred: false },
                pinned: false,
            }
        };
        let photo = |path: Option<PathBuf>, state| {
            row(Content::Image {
                caption: None,
                media: media("image/jpeg", path, state),
                motion: None,
            })
        };
        vec![
            MenuRow {
                message: own,
                marks: Marks { starred: true },
                pinned: true,
            },
            photo(path.clone(), Idle),
            photo(None, Idle),
            photo(None, Downloading),
            row(Content::Video {
                caption: None,
                media: media("video/mp4", path.clone(), Idle),
                seconds: None,
                gif: false,
                note: false,
            }),
            row(Content::Sticker {
                media: media("image/webp", path, Idle),
                animated: false,
            }),
        ]
    }

    #[test]
    fn the_message_menu_speaks_the_interface_language() {
        // #552: the menu came out in English whatever the language.
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("photo.jpg");
        std::fs::write(&file, b"jpeg").unwrap();
        let mut labels = Vec::new();
        for row in menu_fixtures(&file) {
            let spanish = drawn_menu_words(crate::i18n::Locale::Spanish, &row);
            for english in drawn_menu_words(crate::i18n::Locale::English, &row) {
                assert!(!spanish.contains(&english), "{english} in {spanish:?}");
            }
            labels.extend(spanish);
        }
        for spanish in [
            "Responder",
            "Copiar texto",
            "Eliminar para mí",
            "Abrir archivo",
            "Mostrar en la carpeta",
            "Descargar",
        ] {
            assert!(
                labels.iter().any(|label| label == spanish),
                "{spanish} in {labels:?}"
            );
        }
    }

    #[test]
    fn every_message_menu_label_can_be_translated() {
        const TEMPLATE: &str = include_str!("../../assets/i18n/zapfast.pot");
        // Line by line: a Windows checkout may end the template's lines in CRLF.
        let translatable = |text: &str| {
            let msgid = format!("msgid \"{text}\"");
            TEMPLATE.lines().any(|line| line == msgid)
        };
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("photo.jpg");
        std::fs::write(&file, b"jpeg").unwrap();
        let mut labels: Vec<String> = menu_fixtures(&file)
            .iter()
            .flat_map(|row| drawn_menu_words(crate::i18n::Locale::English, row))
            .collect();
        // The menu is as wide as the longest label it measures. The
        // reactions row above its items is named for screen readers, but
        // it is no item the width measures.
        let reactions_row = ["React with any emoji"];
        let measured = message_menu_labels(crate::i18n::Locale::English);
        for label in labels
            .iter()
            .filter(|label| !reactions_row.contains(&label.as_str()))
        {
            assert!(measured.contains(label), "menu width leaves out {label}");
        }
        // The fixtures reach every entry a list row's menu can show. A private
        // reply starts from the group on screen, so only the chat offers it.
        let undrawn: Vec<&str> = measured
            .iter()
            .map(String::as_str)
            .filter(|label| !labels.iter().any(|drawn| drawn == label))
            .collect();
        assert_eq!(undrawn, ["Reply privately"]);
        // Hover texts are not in the accessibility tree (React with any
        // emoji is, as its button's name).
        labels.extend(["React with any emoji".into(), "Remove your reaction".into()]);
        labels.sort();
        labels.dedup();
        let untranslatable: Vec<_> = labels.iter().filter(|label| !translatable(label)).collect();
        assert!(
            untranslatable.is_empty(),
            "not translatable: {untranslatable:?}"
        );
        for label in message_menu_labels(crate::i18n::Locale::English) {
            assert!(
                translatable(&label),
                "menu width uses untranslatable {label}"
            );
        }
    }

    #[test]
    fn the_selection_delete_menu_speaks_the_interface_language() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::theme::install(&ctx);
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            selection_delete_items(
                ui,
                &crate::theme::Palette::dark(),
                crate::i18n::Locale::Spanish,
                "chat",
                true,
                &["message".to_owned()],
                &mut actions,
            );
        });
        output.textures_delta.clear();
        let tree = output
            .platform_output
            .accesskit_update
            .expect("accessibility tree");
        let labels: Vec<_> = tree
            .nodes
            .iter()
            .filter_map(|(_, node)| node.label())
            .collect();
        assert!(labels.contains(&"Eliminar para todos"), "{labels:?}");
        assert!(labels.contains(&"Eliminar para mí"), "{labels:?}");
    }

    #[test]
    fn the_message_menu_measures_star_and_unstar() {
        let labels = message_menu_labels(crate::i18n::Locale::German);
        assert!(labels.iter().any(|label| label == "Markierung entfernen"));
        assert!(labels.iter().any(|label| label == "Markieren"));
    }
}

/// Pending attachment tiles above the composer.
fn pending_strip(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let tile = 72.0;
    let mut remove = None;
    let mut open = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        for (index, item) in app.pending.iter_mut().enumerate() {
            // A picture can be cropped before it goes out. Anything else
            // cannot, and does not pretend to be clickable.
            let editable = match &item.item {
                crate::app::Pending::Picture { .. } => true,
                crate::app::Pending::File(path) => crate::app::Pending::is_picture_file(path),
                // A document goes as the file it is, so it is not edited here.
                crate::app::Pending::Document(_) => false,
            };
            let sense = if editable {
                Sense::click()
            } else {
                Sense::hover()
            };
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(tile), sense);
            if ui.is_rect_visible(rect) {
                // Lifted off the chat like a bubble, with a bubble's corners.
                let radius = CornerRadius::same(widgets::BUBBLE_RADIUS);
                ui.painter()
                    .add(palette.bubble_shadow().as_shape(rect, radius));
                ui.painter().rect_filled(
                    rect.translate(vec2(0.0, -theme::RAISED_EDGE)),
                    radius,
                    palette.raised_edge(palette.surface),
                );
                ui.painter().rect_filled(rect, radius, palette.surface);
                // A document shows as the file it arrives as, even a picture.
                let document = matches!(item.item, crate::app::Pending::Document(_));
                match &mut item.item {
                    crate::app::Pending::Picture {
                        width,
                        height,
                        rgba,
                        texture,
                    } => {
                        let handle = texture.get_or_insert_with(|| {
                            // Limit thumbnails to the GPU's maximum texture size.
                            let image = if *width > 1024 || *height > 1024 {
                                let scale = 1024.0 / (*width).max(*height) as f32;
                                let (w, h) = (
                                    ((*width as f32 * scale) as u32).max(1),
                                    ((*height as f32 * scale) as u32).max(1),
                                );
                                match image::RgbaImage::from_raw(
                                    *width as u32,
                                    *height as u32,
                                    rgba.to_vec(),
                                ) {
                                    Some(full) => {
                                        let small = image::imageops::resize(
                                            &full,
                                            w,
                                            h,
                                            image::imageops::FilterType::Triangle,
                                        );
                                        egui::ColorImage::from_rgba_unmultiplied(
                                            [w as usize, h as usize],
                                            &small,
                                        )
                                    }
                                    None => egui::ColorImage::example(),
                                }
                            } else {
                                egui::ColorImage::from_rgba_unmultiplied([*width, *height], rgba)
                            };
                            ui.ctx().load_texture(
                                format!("pending-picture-{index}"),
                                image,
                                egui::TextureOptions::LINEAR,
                            )
                        });
                        // Preserve aspect ratio while filling the tile.
                        let side = tile - 8.0;
                        let scale =
                            (side / (*width).max(1) as f32).min(side / (*height).max(1) as f32);
                        let fitted = vec2(*width as f32 * scale, *height as f32 * scale);
                        let inner = Rect::from_center_size(rect.center(), fitted);
                        egui::Image::from_texture((handle.id(), fitted))
                            .corner_radius(6.0)
                            .paint_at(ui, inner);
                    }
                    crate::app::Pending::File(path) | crate::app::Pending::Document(path) => {
                        if !document && crate::app::Pending::is_picture_file(path) {
                            widgets::file_image(ui, path)
                                .fit_to_exact_size(Vec2::splat(tile - 8.0))
                                .corner_radius(6.0)
                                .paint_at(ui, rect.shrink(4.0));
                        } else {
                            let icon = Rect::from_center_size(
                                rect.center() - vec2(0.0, 10.0),
                                Vec2::splat(24.0),
                            );
                            theme::paint_icon(ui, Icon::FileText, icon, 22.0, palette.secondary);
                            let name = path
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            let line = widgets::line(
                                ui,
                                &name,
                                theme::regular(10.5),
                                palette.text,
                                tile - 8.0,
                                1,
                            );
                            line.paint(
                                ui,
                                egui::pos2(
                                    rect.center().x - line.size().x / 2.0,
                                    rect.bottom() - 18.0,
                                ),
                                palette.text,
                            );
                        }
                    }
                }
                if editable && response.hovered() {
                    // A picture that can be cropped says so, rather than
                    // looking like every other tile.
                    let dim = palette.shadow.gamma_multiply(1.4);
                    ui.painter().rect_filled(rect, radius, dim);
                    theme::paint_icon(
                        ui,
                        Icon::Pencil,
                        Rect::from_center_size(rect.center(), Vec2::splat(22.0)),
                        20.0,
                        palette.text,
                    );
                }
                // Remove button in the corner.
                let close =
                    Rect::from_center_size(rect.right_top() + vec2(-10.0, 10.0), Vec2::splat(18.0));
                let close_response =
                    ui.interact(close, ui.id().with(("unstage", item.id)), Sense::click());
                ui.painter()
                    .circle_filled(close.center(), 9.0, palette.overlay);
                theme::paint_icon(ui, Icon::X, close, 12.0, palette.text);
                if close_response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    remove = Some(index);
                } else if response.clicked() {
                    open = Some(item.id);
                }
            }
        }
    });
    if let Some(index) = remove {
        app.actions.push(Action::RemovePending(index));
    }
    if let Some(target) = open {
        app.actions.push(Action::EditPicture(target));
    }
    ui.add_space(4.0);
}

/// A photo's view-once choice stays beside its caption and Send, with the
/// same dashed-circle mark as on the phone. Its state is exposed to readers
/// and keyboard users as a checkbox, even though it is drawn as an icon.
fn view_once_button(app: &App, ui: &mut egui::Ui) -> egui::Response {
    let palette = app.palette;
    let selected = app.pending_view_once;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(COMPOSER_CONTROL), Sense::click());
    theme::reveal_focus(&response);
    theme::focus_outline(ui, response.id, rect, COMPOSER_CONTROL / 2.0);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Checkbox,
            ui.is_enabled(),
            selected,
            crate::i18n::gettext(app.locale, "View once"),
        )
    });
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered() || response.has_focus();
        let fill = if selected {
            if hovered {
                palette.accent_hover
            } else {
                palette.accent
            }
        } else if hovered {
            palette.surface_hover
        } else {
            Color32::TRANSPARENT
        };
        ui.painter()
            .circle_filled(rect.center(), COMPOSER_CONTROL / 2.0, fill);
        let color = if selected {
            palette.on_accent
        } else {
            palette.secondary
        };
        theme::paint_icon(ui, Icon::ViewOnce, rect, 24.0, color);
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(crate::i18n::gettext(
            app.locale,
            "The recipient can open this photo only once, on their phone.",
        ))
}
