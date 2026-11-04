//! Focus for the overlays a reader opens from the keys or with a screen
//! reader attached: the dialogs, the media viewer, the sticker view, the
//! picture cropper and the emoji and reaction pickers. Opened, an overlay
//! takes focus in the first pass that shows it, on its first control (the
//! editor's on "Keep changes") unless it put focus on one of its own, so
//! the reader never lands on the window in between, nor on the menu item
//! that opened it on the way. Closed, a dialog, the viewer, the sticker view
//! or the cropper gives focus back to where it was when it opened, such as
//! the message the viewer was opened from, rather than to the message field;
//! the pickers keep their own way back. Opened or closed with the pointer,
//! even with a screen reader or another UIA client attached, an overlay is
//! left exactly as it was (`followed`).
//!
//! One overlay over another, such as a picture opened from a dialog, is one
//! step deep: closing the inner one puts focus back on the outer one's
//! first control, not on the control that opened the inner one.

use egui::Id;

use crate::app::App;
use crate::model::{ChatId, Dialog, Page};

/// Which overlay is open: each dialog counts as one of its own, so a
/// dialog that opens another, of any kind, takes focus again.
#[derive(Clone, PartialEq, Debug)]
enum Overlay {
    Dialog(Dialog),
    Viewer,
    Editor,
    Sticker,
    Reactions,
    Picker,
}

impl Overlay {
    /// The layer the overlay is drawn on: its `Modal` or `Area` id.
    fn layer(&self) -> Id {
        match self {
            Self::Dialog(_) => super::dialogs::modal_id(),
            Self::Viewer => super::image_preview::modal_id(),
            Self::Editor => super::image_editor::area_id(),
            Self::Sticker => super::image_preview::sticker_modal_id(),
            Self::Reactions => super::picker::reaction_area_id(),
            Self::Picker => super::picker::area_id(),
        }
    }

    /// Whether closing it gives focus back to its opener. The emoji picker
    /// leaves the reader in the message field its emoji went to, and the
    /// reaction picker on its message, as they always did.
    fn returns(&self) -> bool {
        !matches!(self, Self::Reactions | Self::Picker)
    }

    /// The control focus enters on, when it is not the first one: the
    /// editor's "Keep changes", so Enter keeps the edit as it always has.
    fn entry(&self, ctx: &egui::Context) -> Option<Id> {
        match self {
            Self::Editor => {
                ctx.data(|data| data.get_temp::<Id>(super::image_editor::keep_note_id()))
            }
            _ => None,
        }
    }
}

/// An overlay open now, and what to do about its focus.
#[derive(Clone)]
struct Opened {
    overlay: Overlay,
    /// Whether it follows focus: opened from the keys or with a screen
    /// reader attached.
    followed: bool,
    /// What had focus when it opened.
    opener: Option<Id>,
    /// Where the opener was: a chat or page changed since has moved on.
    chat: Option<ChatId>,
    page: Page,
    /// Whether focus has gone into the overlay.
    entered: bool,
}

fn opened_id() -> Id {
    Id::new("overlay-focus")
}

/// The opener focus went back to as its overlay closed, taken again in
/// the pass after.
fn returned_id() -> Id {
    Id::new("overlay-returned")
}

/// Where a menu the keys opened hands focus back as it closes, and what
/// had focus in it: noted for an overlay its item opens (`hand_back`).
fn handed_back_id() -> Id {
    Id::new("overlay-handed-back")
}

fn current(app: &App) -> Option<Overlay> {
    // The cropper covers everything, and takes the keys first.
    if app.picture_edit.is_some() {
        Some(Overlay::Editor)
    } else if app.image_preview.is_some() {
        Some(Overlay::Viewer)
    } else if app.sticker_view.is_some() {
        Some(Overlay::Sticker)
    } else if let Some(dialog) = &app.dialog {
        Some(Overlay::Dialog(dialog.clone()))
    } else if app.reaction_target.is_some() {
        Some(Overlay::Reactions)
    } else {
        app.picker.map(|_| Overlay::Picker)
    }
}

/// Whether an overlay opened or closed in this pass follows focus: after
/// keyboard navigation, or with a screen reader attached when no pointer
/// button went down or clicked in this pass. A screen reader's own
/// activation (NVDA+Enter, or Space on a control) reaches egui as an
/// AccessKit action, which egui turns into a click without any pointer
/// event; a mouse click is a pointer event, and keeps upstream's behaviour
/// even with a screen reader or another UIA client, such as Magnifier,
/// attached.
pub(crate) fn followed(ctx: &egui::Context) -> bool {
    let keyboard = ctx.data(|data| {
        data.get_temp::<bool>(crate::theme::keyboard_focus_id())
            .unwrap_or(false)
    });
    let pointer = ctx.input(|input| input.pointer.any_pressed() || input.pointer.any_click());
    keyboard || (super::keys::keyboard_or_reader(ctx) && !pointer)
}

/// A menu the keys opened closed on its chosen item, handing focus back to
/// `list`, from its item `item`. Should the item open an overlay, focus
/// stays on the item until the overlay takes it (`settle`), and `list` is
/// where the overlay gives it back: going by way of the list, the reader
/// would hear it before the overlay.
pub fn hand_back(ctx: &egui::Context, list: Id, item: Option<Id>) {
    ctx.memory_mut(|memory| memory.request_focus(list));
    ctx.data_mut(|data| data.insert_temp(handed_back_id(), (list, item)));
}

/// After the pass's actions, which open and close overlays: notes an
/// overlay that opened and what had focus then, and gives focus back to
/// that opener when it closed, in this pass, while the opener is still
/// drawn and the reader has not moved on to another chat or page. egui
/// drops focus on Escape before anything is drawn, and the message field
/// would take the focus a closing overlay asks for it: a reader would hear
/// the window, then the field.
pub fn settle(app: &mut App, ctx: &egui::Context) {
    let now = current(app);
    let before = ctx.data(|data| data.get_temp::<Opened>(opened_id()));
    let handed_back = ctx.data(|data| data.get_temp::<(Id, Option<Id>)>(handed_back_id()));
    ctx.data_mut(|data| data.remove::<(Id, Option<Id>)>(handed_back_id()));
    if before.as_ref().map(|opened| &opened.overlay) == now.as_ref() {
        return;
    }
    if let Some(before) = &before
        && now.is_none()
        && before.followed
        && followed(ctx)
        && before.overlay.returns()
        && before.chat == app.open_chat
        && before.page == app.page
        && let Some(opener) = before.opener
        && ctx.viewport(|viewport| viewport.this_pass.widgets.contains(opener))
    {
        app.focus_composer = false;
        ctx.memory_mut(|memory| memory.request_focus(opener));
        ctx.data_mut(|data| data.insert_temp(returned_id(), opener));
    }
    // The cropper follows focus as it was asked for: a file opens it frames
    // later, once its size is read, with no key or click in that frame.
    let follows = |overlay: &Overlay, app: &mut App| match overlay {
        Overlay::Editor => app
            .picture_edit_followed
            .take()
            .unwrap_or_else(|| followed(ctx)),
        _ => followed(ctx),
    };
    // One overlay in place of another keeps the first one's opener, and
    // where it was, but follows focus as the move that opened it does: a
    // viewer opened from the keys over a dialog the pointer opened takes
    // focus.
    let opened = now.map(|overlay| match &before {
        Some(before) => Opened {
            followed: follows(&overlay, app),
            overlay,
            entered: false,
            ..before.clone()
        },
        None => {
            let opener = ctx.memory(|memory| memory.focused());
            // Opened by a menu's item: focus stays on the item, drawn in this
            // pass, and the menu's list is what focus comes back to.
            if let Some((list, Some(item))) = handed_back
                && opener == Some(list)
                && ctx.viewport(|viewport| viewport.this_pass.widgets.contains(item))
            {
                ctx.memory_mut(|memory| memory.request_focus(item));
            }
            Opened {
                followed: follows(&overlay, app),
                overlay,
                opener,
                chat: app.open_chat.clone(),
                page: app.page.clone(),
                entered: false,
            }
        }
    });
    ctx.data_mut(|data| match opened {
        Some(opened) => {
            data.insert_temp(opened_id(), opened);
        }
        None => data.remove::<Opened>(opened_id()),
    });
}

/// After the overlays are drawn. An opener focus went back to keeps it:
/// drawn under the closed overlay's modal layer one pass more, it lost it
/// as it was drawn. Only when nothing else took focus meanwhile, such as a
/// key in the same frame. An open overlay takes focus in the first pass that
/// shows it, on its first control, unless focus is already in it. A pass
/// that draws it hidden, as egui's first pass of a new overlay does, gives
/// no control focus; the pass that shows it is asked for at once, in the
/// same frame, and focus stays where it was until then.
pub fn finish(ctx: &egui::Context) {
    if let Some(opener) = ctx.data(|data| data.get_temp::<Id>(returned_id())) {
        ctx.data_mut(|data| data.remove::<Id>(returned_id()));
        if ctx.memory(|memory| memory.focused()).is_none()
            && ctx.viewport(|viewport| viewport.this_pass.widgets.contains(opener))
        {
            ctx.memory_mut(|memory| memory.request_focus(opener));
        }
        return;
    }
    let Some(mut opened) = ctx.data(|data| data.get_temp::<Opened>(opened_id())) else {
        return;
    };
    // Entered once, it is entered again only when focus is gone, such as
    // after the lock screen, whose field this pass no longer draws, or when
    // the control focus was on went away.
    let focused = ctx
        .memory(|memory| memory.focused())
        .filter(|id| ctx.viewport(|viewport| viewport.this_pass.widgets.contains(*id)));
    if !opened.followed || (opened.entered && focused.is_some()) {
        return;
    }
    let layer = opened.overlay.layer();
    let drawn: Vec<egui::WidgetRect> = ctx.viewport(|viewport| {
        viewport
            .this_pass
            .widgets
            .layers()
            .filter(|(layer_id, _)| layer_id.id == layer)
            .flat_map(|(_, widgets)| widgets.iter().copied())
            .collect()
    });
    if drawn.is_empty() {
        return;
    }
    let can_focus = |widget: &&egui::WidgetRect| {
        widget.enabled && widget.visible && widget.sense.is_focusable()
    };
    let entry = opened.overlay.entry(ctx);
    let first = drawn
        .iter()
        .filter(can_focus)
        .find(|widget| Some(widget.id) == entry)
        .or_else(|| drawn.iter().find(can_focus))
        .map(|widget| widget.id);
    let inside = drawn
        .iter()
        .any(|widget| Some(widget.id) == focused && widget.visible);
    if inside {
        opened.entered = true;
    } else if let Some(first) = first {
        ctx.memory_mut(|memory| memory.request_focus(first));
        opened.entered = true;
    } else if drawn.iter().all(|widget| !widget.visible) {
        if ctx.current_pass_index() == 0 {
            ctx.request_discard("an overlay the keys opened takes focus in the pass that shows it");
        }
    } else {
        // Shown, with nothing to focus: it is not tried again every frame.
        opened.entered = true;
    }
    ctx.data_mut(|data| data.insert_temp(opened_id(), opened));
}
