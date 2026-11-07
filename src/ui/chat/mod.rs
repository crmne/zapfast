//! Modular chat view: header, pinned banner, message transcript, bubbles, media, context menus, and composer.

pub mod bubble;
pub mod composer;
pub mod content;
pub mod header;
pub mod media;
pub mod menu;
pub mod messages;
pub mod strips;
pub mod suggestions;
#[cfg(test)]
mod tests;

pub use self::bubble::*;
pub use self::composer::*;
pub(crate) use self::content::*;
pub use self::header::*;
pub(crate) use self::media::*;
pub(crate) use self::menu::*;
pub use self::messages::*;
pub use self::strips::*;
pub use self::suggestions::*;

pub(crate) use std::collections::{HashMap, HashSet};
pub(crate) use std::path::{Path, PathBuf};
pub(crate) use std::time::{Duration, Instant};

pub(crate) use egui::{
    Align, Align2, Color32, CornerRadius, Frame, Key, KeyboardShortcut, Layout, Margin, Modifiers,
    Rect, Sense, Stroke, Vec2, pos2, vec2,
};

pub(crate) use crate::animation;
pub(crate) use crate::app::{
    App, Conversation, JumpHighlight, KeyScroll, LinkPreviewState, RowHeight,
};
pub(crate) use crate::geo::Spot;
pub(crate) use crate::markup;
pub(crate) use crate::model::{
    Action, Chat, ChatActivity, ChatId, Content, Delivery, Dialog, LinkPreview, Media, MediaState,
    Message, PickerTab, Scroll,
};
pub(crate) use crate::theme::{self, Icon, Palette};
pub(crate) use crate::wallpaper;

use super::focus::{self, Stop, TabStop};
use super::{
    attachment_preview, blocking, calls, caption_room, composer_rect_id, keys, list_nav,
    macos_title_strip, narrow, overlay, polls, rail, titlebar_drag, widgets,
};

pub(crate) const SENDER_AVATAR: f32 = 28.0;
pub(crate) const BODY_SIZE: f32 = widgets::MESSAGE_TEXT_SIZE;
pub(crate) const RUN_GAP: f32 = 5.0;
pub(crate) const MESSAGES_WIDTH_SHARE: f32 = 0.65;
pub(crate) const WHATSAPP_WIDTH_SHARE: f32 = 0.72;
pub(crate) const MESSAGES_SIDE_MARGIN: i8 = 17;
pub(crate) const WHATSAPP_SIDE_MARGIN: i8 = 18;
pub(crate) const MESSAGES_BUBBLE_MARGIN: Margin = Margin {
    left: 13,
    right: 13,
    top: 8,
    bottom: 8,
};
pub(crate) const SELECT_BOX: f32 = 18.0;
pub(crate) const SELECT_GUTTER: f32 = SELECT_BOX + 14.0;
pub(crate) const NOT_SENT: &str = "Not sent";
pub(crate) const NOT_SENT_HINT: &str =
    "This message could not be sent, and ZapFast will not retry it. Send it again yourself.";

/// Shared immutable state snapshot for rendering a conversation frame.
pub(crate) struct View<'a> {
    pub(crate) palette: Palette,
    pub(crate) locale: crate::i18n::Locale,
    pub(crate) chat: &'a Chat,
    pub(crate) me: Option<&'a str>,
    pub(crate) auto_download: bool,
    pub(crate) connected: bool,
    pub(crate) account_receipts_off: bool,
    pub(crate) poll_voting: &'a HashSet<(ChatId, String)>,
    pub(crate) interactive_pending: &'a HashSet<(ChatId, String)>,
    pub(crate) selecting: bool,
    pub(crate) anchor: Option<&'a str>,
    pub(crate) open_menu: Option<&'a str>,
    pub(crate) starred: Option<&'a HashSet<String>>,
    pub(crate) pins: Option<&'a HashSet<String>>,
    pub(crate) pin_notices: &'a [crate::archive::Pinned],
    pub(crate) reaction: Option<&'a str>,
    pub(crate) reaction_menu: bool,
    pub(crate) reaction_emoji: &'a [(String, u32)],
    pub(crate) keyboard_navigation: &'a std::cell::Cell<bool>,
    pub(crate) names_or: &'a dyn Fn(&str, Option<&str>) -> String,
    pub(crate) mention_names: &'a dyn Fn(&str) -> String,
    pub(crate) avatars: &'a HashMap<String, Option<PathBuf>>,
    pub(crate) contacts: &'a HashMap<String, crate::model::Contact>,
    pub(crate) now: i64,
    pub(crate) animate: bool,
    pub(crate) motion_playing: Option<&'a str>,
    pub(crate) player: &'a crate::audio::Player,
    pub(crate) voice_speed: f32,
    pub(crate) video: &'a crate::video::Player,
    pub(crate) copy_rows: Option<&'a std::sync::Mutex<Vec<crate::transcript::Row>>>,
    pub(crate) download_progress: Option<&'a HashMap<String, u8>>,
    pub(crate) arrivals: &'a HashMap<String, Instant>,
}

/// Primary controller for the open conversation pane.
pub struct ChatView;

impl ChatView {
    pub fn show(app: &mut App, ui: &mut egui::Ui) {
        let Some(chat) = app.current_chat().cloned() else {
            super::banner(app, ui);
            super::voice_bar::show(app, ui);
            empty(app, ui);
            return;
        };
        if Composer::reviewing_attachments_for(app, &chat) {
            ui.painter()
                .rect_filled(ui.available_rect_before_wrap(), 0.0, app.palette.surface);
        } else {
            wallpaper::paint(ui, &app.wallpaper());
        }
        if app.palette.vibrant() {
            let layer = egui::LayerId::new(egui::Order::Middle, ui.id().with("chat-header"));
            let bar = ui
                .scope_builder(egui::UiBuilder::new().layer_id(layer), |ui| {
                    let glass = ui.painter().add(egui::Shape::Noop);
                    let veil = ui.painter().add(egui::Shape::Noop);
                    ChatHeader::show(app, ui, &chat);
                    super::banner(app, ui);
                    super::voice_bar::show(app, ui);
                    let bar = ui.min_rect();
                    let frosted = bar.with_max_y(bar.bottom() + HEADER_TAIL);
                    let ease = HEADER_TAIL + bar.height() * HEADER_EASE;
                    ui.painter().set(glass, super::glass::shape(frosted, ease));
                    ui.painter()
                        .set(veil, header_veil(&app.palette, frosted, ease));
                    bar
                })
                .inner;
            let review = attachment_review(app, ui, &chat);
            Composer::show(app, ui, &chat, review);
            if review.is_some() {
                super::attachment_preview::show(app, ui);
                return;
            }
            let list = ui.available_rect_before_wrap().with_min_y(bar.top());
            ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
                MessageList::show(app, ui, &chat, bar.height());
            });
            return;
        }
        let header = ChatHeader::show(app, ui, &chat);
        super::banner(app, ui);
        super::voice_bar::show(app, ui);
        PinBanner::show(app, ui, &chat);
        let review = attachment_review(app, ui, &chat);
        Composer::show(app, ui, &chat, review);
        if review.is_some() {
            super::attachment_preview::show(app, ui);
        } else {
            MessageList::show(app, ui, &chat, 0.0);
        }
        widgets::paint_shadow_below(
            ui,
            &app.palette,
            header.left(),
            header.right(),
            header.bottom(),
        );
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
}

impl Composer {
    pub fn reviewing_attachments_for(app: &App, chat: &Chat) -> bool {
        reviewing_attachments(app, chat)
    }
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    ChatView::show(app, ui);
}
