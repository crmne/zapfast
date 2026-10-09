//! Keyboard shortcuts.

use egui::{Key, Modifiers};

use crate::app::App;
use crate::model::{Action, Chat, ChatId, Dialog, InfoView, Page, Scroll};

/// Whether Home, End and Ctrl+End scroll the open chat rather than move the
/// cursor of the focused text field (#456).
fn jump_keys_reach_chat(editing_text: bool, composer_focused: bool, composer_empty: bool) -> bool {
    !editing_text || (composer_focused && composer_empty)
}

/// Routes keyboard commands to actions, respecting overlays and text focus.
pub fn handle(app: &mut App, ctx: &egui::Context) {
    // A focused menu takes what is typed before any shortcut sees it.
    super::widgets::menu_letters(ctx);
    if app.picture_edit.is_some() {
        editor_keys(app, ctx);
        return;
    }
    if app.image_preview.is_some() {
        video_keys(app, ctx);
        preview_keys(app, ctx);
        return;
    }
    if app.sticker_view.is_some() {
        sticker_view_keys(app, ctx);
        return;
    }
    let editing_text = ctx.text_edit_focused();
    let find = find_action(app);
    let archive = archive_action(app);
    let next_account = next_account_action(app);
    let composer_focused = ctx.memory(|memory| memory.has_focus(egui::Id::new("composer-text")));
    // Home/End (and Ctrl+End) move the text cursor in a field with text in
    // it, as in any editor; they scroll the chat only from an empty composer
    // or with no field focused.
    let jump_allowed =
        jump_keys_reach_chat(editing_text, composer_focused, app.composer.is_empty());
    let mut actions = Vec::new();
    ctx.input_mut(|input| {
        // The message list's key counts only with its modifier alone:
        // `consume_key` would also take Alt+Shift+2, and on macOS
        // Command+Control+2, which is Command+2's chat.
        let messages_key_free = input.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key: Key::Num2,
                    pressed: true,
                    modifiers,
                    ..
                } if messages_modifier_alone(*modifiers)
            )
        });
        let mut key = |modifiers: Modifiers, key: Key, action: Action| {
            if input.consume_key(modifiers, key) {
                actions.push(action);
            }
        };
        // Checked first: a plain Ctrl+F binding also matches Ctrl+Shift+F.
        key(
            Modifiers::COMMAND | Modifiers::SHIFT,
            Key::F,
            Action::FocusSearch,
        );
        key(Modifiers::COMMAND, Key::F, find);
        // Before Ctrl+L, which would also match it with Shift held.
        if app.settings.app_lock_hash.is_some() {
            key(
                Modifiers::COMMAND | Modifiers::SHIFT,
                Key::L,
                Action::LockApp,
            );
        }
        key(Modifiers::COMMAND, Key::K, Action::FocusSearch);
        if let Some(next_account) = next_account {
            key(Modifiers::COMMAND | Modifiers::SHIFT, Key::A, next_account);
        }
        if app.is_linked() {
            key(
                Modifiers::COMMAND,
                Key::N,
                Action::ShowDialog(Dialog::NewChat),
            );
        }
        if !editing_text {
            key(
                Modifiers::NONE,
                Key::Questionmark,
                Action::ShowDialog(Dialog::Shortcuts),
            );
        }
        if app.chat_view()
            && app.open_chat.is_some()
            && app.dialog.is_none()
            && !app.show_update
            && app.picker.is_none()
            && app.reaction_target.is_none()
            && app.recording.is_none()
        {
            key(Modifiers::COMMAND, Key::L, Action::FocusComposer);
        }
        if let Some(archive) = archive {
            key(Modifiers::COMMAND, Key::E, archive);
        }
        key(Modifiers::COMMAND, Key::B, Action::ToggleSidebar);
        key(Modifiers::COMMAND, Key::Comma, Action::ToggleSettings);
        key(Modifiers::COMMAND, Key::Q, Action::Quit);
        key(Modifiers::COMMAND, Key::W, Action::CloseWindow);
        key(
            Modifiers::COMMAND,
            Key::Slash,
            Action::ShowDialog(Dialog::Shortcuts),
        );
        key(Modifiers::COMMAND, Key::Plus, Action::ZoomBy(0.1));
        key(Modifiers::COMMAND, Key::Equals, Action::ZoomBy(0.1));
        key(Modifiers::COMMAND, Key::Minus, Action::ZoomBy(-0.1));
        key(Modifiers::COMMAND, Key::Num0, Action::ResetZoom);
        if jump_allowed {
            key(Modifiers::COMMAND, Key::End, Action::ScrollToBottom);
        }
        // Alt+2 goes to the message list. On macOS it is Control+2:
        // Command+1 to 9 open chats there, and Option with a digit types
        // characters some keyboard layouts need, such as # and €.
        if app.dialog.is_none()
            && !app.show_update
            && app.picker.is_none()
            && app.reaction_target.is_none()
            && app.recording.is_none()
            // The call screen covers the chats.
            && (app.call.is_none() || app.call_surface_hidden)
            && messages_key_free
        {
            key(
                if cfg!(target_os = "macos") {
                    Modifiers::CTRL
                } else {
                    Modifiers::ALT
                },
                Key::Num2,
                Action::FocusMessages,
            );
        }
        // The rail's Chats and Calls, by the digit's place on the keyboard:
        // with Shift held, layouts report the digits as their symbols.
        // Ctrl+1..9 without Shift open chats by their place in the list.
        if app.is_linked() {
            match take_destination(input) {
                Some(1) => actions.extend(super::rail::Destination::Chats.actions(app)),
                Some(2) => actions.extend(super::rail::Destination::Calls.actions(app)),
                _ => {}
            }
        }
    });
    drop_shortcut_text(ctx);
    // Escape cancels the topmost state. Menus handle Escape themselves.
    let menu_open = egui::Popup::is_any_open(ctx);
    let search_focused = ctx.memory(|memory| memory.has_focus(egui::Id::new("chat-search")));
    if app.chat_view()
        && app.selection.as_ref().is_some_and(|(chat, selected)| {
            app.open_chat.as_ref() == Some(chat) && !selected.is_empty()
        })
        && app.dialog.is_none()
        && !app.show_update
        && app.picker.is_none()
        && app.reaction_target.is_none()
        && app.recording.is_none()
        && !app.chat_search_calendar
        && !menu_open
        // The selection bar replaces the composer. Its previous frame's
        // focus must not swallow Copy before egui drops that hidden field.
        && (!editing_text || composer_focused)
        && ctx.input(|input| input.events.iter().any(is_copy_event))
        && let Some(text) = super::conversation::selected_text(app)
    {
        // Native clipboard commands arrive as Event::Copy, including Copy
        // from the macOS menu. Consume only when we have text to export;
        // otherwise leave Copy available to egui's text-selection hook.
        ctx.input_mut(|input| {
            input.events.retain(|event| !is_copy_event(event));
        });
        actions.push(Action::CopyText(text));
    }
    // The focused message list takes the arrows, Home, End and the page keys
    // to move its current message, ahead of the scrolling below.
    if app.chat_view()
        && app.dialog.is_none()
        && !app.show_update
        && app.picker.is_none()
        && app.reaction_target.is_none()
        && (app.call.is_none() || app.call_surface_hidden)
        && !menu_open
        && let Some(chat) = app.open_chat.clone()
        && ctx.memory(|memory| memory.has_focus(crate::ui::conversation::messages_list_id(&chat)))
        && let Some(step) =
            crate::ui::list_nav::take_step(ctx, || app.message_page_rows(ctx, &chat))
    {
        actions.push(Action::MessageCursor(step));
    }
    // The focused list's current message takes the keys that act on it.
    if app.dialog.is_none()
        && !app.show_update
        && app.picker.is_none()
        && app.reaction_target.is_none()
        && app.recording.is_none()
        && !menu_open
        // The call screen covers the chats.
        && (app.call.is_none() || app.call_surface_hidden)
        && let Some((chat, id)) = focused_message(app, ctx)
    {
        let taken = message_keys(app, ctx, &chat, &id);
        if !taken.is_empty() {
            // Acting from the keyboard is keyboard navigation, as Tab is:
            // the focus these keys move (into the menu, the picker, the
            // input, a dialog) is shown.
            ctx.data_mut(|data| data.insert_temp(crate::theme::keyboard_focus_id(), true));
            actions.extend(taken);
        }
    }
    // PgUp/PgDn/Home/End scroll the open chat. PgUp/PgDn also work while the
    // composer has focus, since egui's TextEdit does not handle them itself;
    // Home/End keep moving the text cursor in a non-empty field, as ↑ keeps
    // its normal meaning outside an empty composer.
    if app.chat_view()
        && app.open_chat.is_some()
        && app.dialog.is_none()
        && !app.show_update
        && app.picker.is_none()
        && app.reaction_target.is_none()
        && !menu_open
    {
        ctx.input_mut(|input| {
            if take_plain(input, Key::PageUp) {
                actions.push(Action::ScrollPage(Scroll::PageUp));
            }
            if take_plain(input, Key::PageDown) {
                actions.push(Action::ScrollPage(Scroll::PageDown));
            }
            if jump_allowed {
                if take_plain(input, Key::Home) {
                    actions.push(Action::ScrollPage(Scroll::Top));
                }
                if take_plain(input, Key::End) {
                    actions.push(Action::ScrollPage(Scroll::Bottom));
                }
            }
        });
    }
    let escape = (!menu_open || app.reaction_target.is_some())
        && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape));
    if escape {
        if app.show_update {
            actions.push(Action::CloseUpdate);
        } else if app.dialog.is_some() {
            actions.push(Action::CloseDialog);
        } else if app.recording.is_some() {
            actions.push(Action::CancelRecording);
        } else if app.picker.is_some() || app.reaction_target.is_some() {
            actions.push(Action::ClosePicker);
        } else if app.selection.is_some() {
            // Taken here, the key never reaches the selection bar, and
            // would otherwise fall through to closing the chat.
            actions.push(Action::CancelSelection);
        } else if app.show_starred {
            actions.push(Action::ToggleStarred);
        } else if app.chat_search_visible() && app.chat_search_calendar {
            // The day filter first, then the pane it hangs from.
            app.chat_search_calendar = false;
        } else if app.chat_search_visible() && !composer_focused {
            actions.push(Action::CloseChatSearch);
        } else if app.emoji_start.is_some() {
            actions.push(Action::CloseEmojiSuggestions);
        } else if app.mention_start.is_some() {
            actions.push(Action::CloseMentions);
        } else if !app.pending.is_empty() {
            actions.push(Action::ClearPending);
        } else if app.editing.is_some() {
            actions.push(Action::CancelEdit);
        } else if app.reply_to.is_some() {
            actions.push(Action::CancelReply);
        } else if app.call_fullscreen {
            // The window shrinks back first, and only then does the next Escape put the surface
            // aside. Neither one ends the call; only the hang-up button does that.
            actions.push(Action::ToggleCallFullscreen);
        } else if app.call_surface_open() {
            // The same move the surface's own button makes: the call keeps running, and the bar at
            // the bottom of the chat offers the way back.
            actions.push(Action::LeaveCallSurface);
        } else if app.info_visible() && app.group_name_edit.is_some() {
            // Cancels the group rename and keeps the panel open.
            actions.push(Action::CloseGroupName);
        } else if app.info_visible() && app.group_description_edit.is_some() {
            actions.push(Action::CloseGroupDescription);
        } else if app.info_visible() && app.contact_edit.is_some() {
            actions.push(Action::CloseContactEdit);
        } else if app.info_visible() {
            // After what the composer was in the middle of: the media view
            // goes back to the overview, and the overview closes the panel.
            let media = matches!(
                app.info.as_ref().map(|info| info.view),
                Some(InfoView::Media(_))
            );
            actions.push(if media {
                Action::ShowInfo(InfoView::Overview)
            } else {
                Action::CloseInfo
            });
        } else if app.page == Page::Wallpaper {
            actions.push(Action::Open(Page::Settings));
        } else if app.page == Page::Settings && !app.settings_search.is_empty() {
            actions.push(Action::SearchSettings(String::new()));
        } else if app.page == Page::Calls && app.new_call {
            actions.push(Action::ShowNewCall(false));
        } else if app.page == Page::Calls && super::narrow(ctx) && app.call_picked() {
            // A narrow window shows the picked call's chat over the history:
            // Escape goes back to it, as the chat's Back does.
            actions.push(Action::CloseChat);
        } else if app.page == Page::Settings || app.page == Page::Calls {
            actions.push(Action::Open(Page::Chats));
        } else if search_focused || !app.search.is_empty() {
            if !app.search.is_empty() {
                actions.push(Action::Search(String::new()));
            }
            if app.open_chat.is_some() {
                actions.push(Action::FocusComposer);
            }
        } else if app.chat_search_visible() {
            actions.push(Action::CloseChatSearch);
        } else if app.open_chat.is_some() {
            actions.push(Action::CloseChat);
        } else if app.locked_folder {
            actions.push(Action::CloseLockedFolder);
        }
    }
    if escape {
        keep_focus_on_escape(app, ctx);
    }
    // Enter sends a recording because the text field is hidden.
    if app.recording.is_some()
        && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter))
    {
        actions.push(Action::SendRecording);
    }
    // Alt+Up/Down switches chats without leaving the composer, and so do
    // Ctrl+Shift+[ and Ctrl+Shift+], WhatsApp's own keys. With Shift held,
    // US-style layouts report the brackets as braces, so both spellings
    // count; a layout with another character on the shifted key has Alt.
    let step = ctx.input_mut(|input| {
        let brackets = Modifiers::COMMAND | Modifiers::SHIFT;
        let mut pressed = |modifiers: Modifiers, keys: &[Key]| {
            keys.iter().any(|key| input.consume_key(modifiers, *key))
        };
        if pressed(Modifiers::ALT, &[Key::ArrowDown])
            || pressed(brackets, &[Key::CloseBracket, Key::CloseCurlyBracket])
        {
            1
        } else if pressed(Modifiers::ALT, &[Key::ArrowUp])
            || pressed(brackets, &[Key::OpenBracket, Key::OpenCurlyBracket])
        {
            -1
        } else {
            0
        }
    });
    // Numbered shortcuts use the same ordering and filters as the sidebar.
    let number = if app.page == Page::Chats
        && app.dialog.is_none()
        && !app.show_update
        && app.picker.is_none()
        && app.reaction_target.is_none()
        && app.recording.is_none()
        && !menu_open
    {
        ctx.input_mut(take_chat_number)
    } else {
        None
    };
    if step != 0 || number.is_some() {
        // Switching chats from the keyboard is keyboard navigation, as Tab
        // is: a chat without a message field then hands its focus to the
        // message list rather than to nothing.
        ctx.data_mut(|data| data.insert_temp(crate::theme::keyboard_focus_id(), true));
        let visible = app.visible_chats();
        if !visible.is_empty() {
            let current = app
                .open_chat
                .as_ref()
                .and_then(|open| visible.iter().position(|chat| chat.id == *open));
            let next = number.unwrap_or_else(|| match current {
                Some(index) => (index as i64 + step).rem_euclid(visible.len() as i64) as usize,
                None => 0,
            });
            if let Some(next) = visible.get(next).map(|chat| chat.id.clone()) {
                if number.is_some() {
                    app.search_selected = None;
                }
                // Stepping through the Unread list must not shift it underfoot.
                if app.search.trim().is_empty() && !app.show_archived {
                    actions.push(Action::KeepUnread(next.clone()));
                }
                app.scroll_chat_into_view = Some(next.clone());
                actions.push(Action::OpenChat(next));
            }
        }
    }
    // Arrow Up in an empty, focused composer edits the user's most recent
    // message, as WhatsApp does. The key keeps its normal meaning everywhere
    // else: it navigates open overlays and moves the cursor in a non-empty
    // field.
    let edit_previous = composer_focused
        && app.chat_view()
        && app.open_chat.is_some()
        && app
            .open_chat
            .as_deref()
            .and_then(|id| app.chat(id))
            .is_some_and(Chat::can_send)
        && app.composer.is_empty()
        && app.pending.is_empty()
        && app.editing.is_none()
        && app.picker.is_none()
        && app.reaction_target.is_none()
        && app.dialog.is_none()
        && !app.show_update
        && app.recording.is_none()
        && !menu_open
        && ctx.input_mut(|input| take_plain(input, Key::ArrowUp));
    if let Some(id) = edit_previous.then(|| app.previous_own_editable()).flatten() {
        actions.push(Action::Edit(id));
    }
    app.actions.extend(actions);
}

/// Recognizes native Copy and plain Command/Ctrl+C presses.
fn is_copy_event(event: &egui::Event) -> bool {
    matches!(event, egui::Event::Copy)
        || matches!(event, egui::Event::Key {
            key: Key::C, pressed: true, modifiers, ..
        } if (modifiers.command || modifiers.ctrl)
            && !modifiers.alt && !modifiers.shift)
}

/// The open chat's current message while its message list has keyboard
/// focus: the message the cursor names, or the newest when it names none
/// loaded, as the list itself resolves it. An album is one row, named by
/// its first item. The Calls tab shows the open chat beside its history, so
/// its list counts there too; a narrow window showing only the history has
/// no list on screen, and so none with focus.
pub fn focused_message(app: &App, ctx: &egui::Context) -> Option<(ChatId, String)> {
    use crate::ui::conversation::{row_of, row_starts};
    if !app.chat_view() {
        return None;
    }
    let chat = app.open_chat.as_ref()?;
    if !ctx.memory(|memory| memory.has_focus(crate::ui::conversation::messages_list_id(chat))) {
        return None;
    }
    let messages = &app.conversations.get(chat)?.messages;
    let index = app
        .message_cursor
        .get(chat)
        .and_then(|id| messages.iter().position(|message| &message.id == id))
        .or(messages.len().checked_sub(1))?;
    let starts = row_starts(messages, app.selecting_in(chat));
    let first = starts.get(row_of(&starts, index)).copied().unwrap_or(index);
    Some((chat.clone(), messages.get(first)?.id.clone()))
}

/// Takes this frame's key for the focused message `id`, if any, and returns
/// what it does: each key is the message menu's item of the same name, with
/// the same conditions, so the keys never offer what the menu does not.
/// Enter does what Space does, and Shift+F10 opens the menu itself: egui
/// never reports the Applications key. Shift+Delete, where the menu offers
/// no "Delete for everyone", says why.
fn message_keys(app: &mut App, ctx: &egui::Context, chat: &str, id: &str) -> Vec<Action> {
    use crate::model::Content;
    use crate::ui::conversation as view;
    // Every key below is a press: a frame without one has nothing to take,
    // and makes no copy of the message. On Windows, Shift+Delete arrives as
    // Cut alone.
    if !ctx.input(|input| {
        input.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key { pressed: true, .. } | egui::Event::Cut
            )
        })
    }) {
        return Vec::new();
    }
    // While this chat selects messages, a key acting on one message would
    // act behind the selection's back, and the bubbles draw no menu then: one
    // asked for would open only once the selection ends. The keys that move
    // the list still move it.
    if app.selecting_in(chat) {
        return Vec::new();
    }
    // The account's field directly, as `voting` below: the keys change the
    // window's own state while they read the row. An album is one row with
    // several messages, its items.
    let selecting = app.selecting_in(chat);
    let Some(members) = app.accounts[app.active]
        .conversations
        .get(chat)
        .and_then(|conversation| {
            let index = conversation
                .messages
                .iter()
                .position(|message| message.id == id)?;
            Some(view::row_members(&conversation.messages, selecting, index))
        })
    else {
        return Vec::new();
    };
    let row = id;
    // The links, cards and files inside the row, and the one the item
    // cursor is on: the first, unless the keys moved it on this row.
    let (targets, owners) = view::row_targets(members, &app.contacts);
    // The item is found again by itself and the album item it belongs to,
    // so an edit or a saved contact that moves the items under the cursor
    // never makes Space or Enter act on another one, nor on the same link
    // in another item's caption.
    // An item that is gone leaves the cursor lost until it moves again.
    let owner_of = |index: usize| members[owners[index]].id.as_str();
    let (current, lost) = match app
        .message_item
        .as_ref()
        .filter(|(item_chat, item_message, ..)| item_chat == chat && item_message == row)
    {
        None => (0, false),
        Some((.., index, owner, item))
            if targets.get(*index) == Some(item) && owner_of(*index) == owner =>
        {
            (*index, false)
        }
        Some((.., owner, item)) => match targets
            .iter()
            .enumerate()
            .position(|(index, target)| target == item && owner_of(index) == owner)
        {
            Some(index) => (index, false),
            None => (0, true),
        },
    };
    // The message the keys act on: the row's own, or the album item the
    // cursor is on.
    let message = &members[owners.get(current).copied().unwrap_or(0)];
    let id = message.id.as_str();
    // A call in the chat is a record: its menu only calls back, so Space and
    // Enter open that menu and the other keys that act on a message leave it
    // alone.
    let record = matches!(message.content, Content::Call { .. });
    // Left and Right move the item cursor. One target leaves them alone.
    let walks = targets.len() > 1;
    let step = |delta: isize| {
        current
            .saturating_add_signed(delta)
            .min(targets.len().saturating_sub(1))
    };
    let voting = view::Voting {
        connected: app.link.is_connected(),
        // The account's field directly: the keys below change the window's
        // own state while the votes on their way are borrowed.
        pending: &app.accounts[app.active].poll_voting,
    };
    let primary = view::target_actions(ctx, chat, message, targets.get(current), &voting);
    // A poll option that takes no vote now: Space or Enter says why instead.
    let option = match targets.get(current) {
        Some(crate::a11y::KeyTarget::PollOption(option)) => Some(*option),
        _ => None,
    };
    let blocked = option.and_then(|option| view::option_blocked(message, option, &voting));
    // A carousel card with no picture has only its buttons: Space or Enter
    // opens the message menu on them, as Shift+F10 opens it on its first
    // item. A card with neither takes neither key.
    let card_menu = match (targets.get(current), &message.content) {
        (
            Some(crate::a11y::KeyTarget::Card(index)),
            Content::Interactive {
                card: Some(card), ..
            },
        ) => card
            .carousel
            .get(*index)
            .filter(|card| card.image.is_none() && !card.buttons.is_empty())
            .map(|_| *index),
        _ => None,
    };
    // A phone number: Space or Enter opens its menu, as clicking it does,
    // with focus on its first item.
    let phone_menu = match targets.get(current) {
        Some(crate::a11y::KeyTarget::Phone { url, .. }) => Some(url.clone()),
        _ => None,
    };
    let mut opened_card = None;
    let mut actions = Vec::new();
    let mut moved = None;
    let mut refused = None;
    let mut revoke_refused = None;
    // In an album, the message a key acts on is the item heard: with that
    // item gone, the first one would be acted on unheard.
    let album_lost = lost && members.len() > 1;
    ctx.input_mut(|input| {
        if album_lost
            && (take_exact(input, Modifiers::SHIFT, Key::F10)
                || [Key::R, Key::G, Key::E, Key::C, Key::F]
                    .into_iter()
                    .any(|key| take_plain(input, key))
                || take_delete(input, Modifiers::SHIFT)
                || take_delete(input, Modifiers::NONE))
        {
            moved = Some(0);
        } else if take_exact(input, Modifiers::SHIFT, Key::F10) {
            actions.push(Action::OpenMessageMenu(id.to_owned()));
        } else if record {
            // Nothing else acts on a call.
            if take_space_or_enter(input) {
                actions.push(Action::OpenMessageMenu(id.to_owned()));
            }
        } else if lost && !targets.is_empty() && take_space_or_enter(input) {
            // The item heard last is gone: say where the cursor is now
            // rather than act on something else.
            moved = Some(0);
        } else if let Some(index) = card_menu
            && take_space_or_enter(input)
        {
            actions.push(Action::OpenMessageMenu(id.to_owned()));
            opened_card = Some(index);
        } else if let Some(url) = &phone_menu
            && take_space_or_enter(input)
        {
            actions.push(Action::OpenPhoneMenu {
                message: id.to_owned(),
                url: url.clone(),
            });
        } else if !primary.is_empty() && take_space_or_enter(input) {
            // What clicking the current target does. Play and pause are not
            // announced: the player's own state is the feedback.
            actions.extend(primary.iter().cloned());
        } else if blocked.is_some() && take_space_or_enter(input) {
            refused = blocked;
        } else if walks && take_plain(input, Key::ArrowLeft) {
            moved = Some(step(-1));
        } else if walks && take_plain(input, Key::ArrowRight) {
            moved = Some(step(1));
        } else if take_plain(input, Key::R) {
            if view::can_reply(&message.content) {
                actions.push(Action::Reply(id.to_owned()));
                actions.push(Action::FocusComposer);
            }
        } else if let Some(quoted) = &message.quoted
            && take_plain_once(input, Key::G)
        {
            // The quote's jump: the list moves to the replied message, once
            // for a key held down.
            actions.push(view::quote_key_action(quoted));
        } else if take_plain(input, Key::E) {
            actions.push(view::open_reaction_picker_action(chat, id, true));
        } else if take_plain(input, Key::C) {
            let names = |mention: &str| app.mention_name(mention);
            actions.extend(view::copy_key_action(message, &names));
        } else if take_plain(input, Key::F) {
            if crate::app::can_select(&message.content) {
                actions.push(view::forward_action(chat, id));
            }
        } else if take_delete(input, Modifiers::SHIFT) {
            // Never a deletion: the confirmation that "Delete for everyone"
            // opens, where the menu offers it.
            match view::revoke_blocker(message, crate::util::now()) {
                None => actions.push(view::delete_for_everyone_action(chat, id)),
                Some(blocker) => revoke_refused = Some(blocker),
            }
        } else if take_delete(input, Modifiers::NONE) {
            // Never a deletion: the confirmation that "Delete for me" opens.
            actions.push(view::delete_for_me_action(chat, id));
        }
    });
    if let Some(index) = opened_card {
        ctx.data_mut(|data| data.insert_temp(view::menu_card_id(chat, id), index));
    }
    // A download that Space or Enter asked for is spoken if it fails.
    if let Some((message, card)) = actions.iter().find_map(|action| match action {
        Action::Download { message, card, .. } => Some((message.clone(), *card)),
        _ => None,
    }) {
        app.download_heard.insert((chat.to_owned(), message, card));
    }
    // Each move says where the cursor is; at either end it says the end item
    // again, as the list does.
    if let Some(index) = moved
        && let Some(target) = targets.get(index)
    {
        let owner = &members[owners[index]];
        let text = view::target_label(app.locale, owner, &targets, index, &voting);
        // The list stays on this row while its items are walked: a message
        // arriving meanwhile does not become the current one.
        app.message_cursor.insert(chat.to_owned(), row.to_owned());
        app.message_item = Some((
            chat.to_owned(),
            row.to_owned(),
            index,
            owner.id.clone(),
            target.clone(),
        ));
        app.announcer.say(crate::announcer::Category::Item, text);
    }
    // A vote says the choice it sends, as the ballot will show it once the
    // vote lands.
    if let (Some(option), Some(Action::VotePoll { choices, .. })) = (option, actions.first()) {
        let text = view::vote_said(app.locale, message, option, choices);
        app.announcer.say(crate::announcer::Category::Item, text);
    }
    if let Some(reason) = refused {
        let text = crate::a11y::fill(
            &crate::i18n::gettext(app.locale, "unavailable: {reason}"),
            &[("reason", &view::blocker_said(app.locale, reason))],
        );
        app.announcer.say(crate::announcer::Category::Status, text);
    }
    if let Some(blocker) = revoke_refused {
        let text = view::revoke_refusal_said(app.locale, blocker);
        app.announcer.say(crate::announcer::Category::Status, text);
    }
    actions
}

/// Takes this frame's first press of the Delete key with exactly
/// `modifiers`. A Mac's Delete key is Backspace, and Fn with it is Delete.
/// On Windows egui reports Shift+Delete as Cut, with no key, so a Cut while
/// Shift alone is held is that press; Ctrl+X, also Cut, is left alone.
fn take_delete(input: &mut egui::InputState, modifiers: Modifiers) -> bool {
    if take_exact(input, modifiers, Key::Delete)
        || (cfg!(target_os = "macos") && take_exact(input, modifiers, Key::Backspace))
    {
        return true;
    }
    if !cfg!(target_os = "windows") || modifiers != Modifiers::SHIFT || input.modifiers != modifiers
    {
        return false;
    }
    let mut taken = false;
    input.events.retain(|event| {
        let cut = !taken && matches!(event, egui::Event::Cut);
        taken |= cut;
        !cut
    });
    taken
}

/// Takes an exact Command/Ctrl+1..9 press, leaving modified number keys alone.
fn take_chat_number(input: &mut egui::InputState) -> Option<usize> {
    let keys = [
        Key::Num1,
        Key::Num2,
        Key::Num3,
        Key::Num4,
        Key::Num5,
        Key::Num6,
        Key::Num7,
        Key::Num8,
        Key::Num9,
    ];
    let mut number = None;
    input.events.retain(|event| {
        if number.is_none()
            && let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            && modifiers.matches_exact(Modifiers::COMMAND)
            && let Some(index) = keys.iter().position(|candidate| candidate == key)
        {
            number = Some(index);
            return false;
        }
        true
    });
    number
}

/// Takes Ctrl+Shift+1 or Ctrl+Shift+2 (Cmd on macOS), the rail's Chats and
/// Calls, and says which. It goes by the key's place on the keyboard: with
/// Shift held, each layout reports the digit as a symbol of its own.
fn take_destination(input: &mut egui::InputState) -> Option<u8> {
    let mut destination = None;
    input.events.retain(|event| {
        if destination.is_none()
            && let egui::Event::Key {
                key,
                physical_key,
                pressed: true,
                modifiers,
                ..
            } = event
            && modifiers.matches_exact(Modifiers::COMMAND | Modifiers::SHIFT)
        {
            destination = match physical_key.unwrap_or(*key) {
                Key::Num1 => Some(1),
                Key::Num2 => Some(2),
                _ => None,
            };
            return destination.is_none();
        }
        true
    });
    destination
}

/// Removes this frame's first plain (unmodified) press of `key`, if any, and
/// reports whether one was found. `consume_key` is unsuitable here: it also
/// matches the key with Shift or Alt held, and a plain binding must leave
/// those combinations, such as Shift+Home for text selection, alone.
pub(crate) fn take_plain(input: &mut egui::InputState, key: Key) -> bool {
    take_exact(input, Modifiers::NONE, key)
}

/// [`take_plain`] for a key that acts once however long it is held: its
/// first press counts, and a repeat is taken without acting, so nothing
/// else acts on it either.
fn take_plain_once(input: &mut egui::InputState, key: Key) -> bool {
    let mut taken = None;
    input.events.retain(|event| {
        if taken.is_some() {
            return true;
        }
        if let egui::Event::Key {
            key: found,
            pressed: true,
            repeat,
            modifiers,
            ..
        } = event
            && *found == key
            && *modifiers == Modifiers::NONE
        {
            taken = Some(!repeat);
            return false;
        }
        true
    });
    taken.unwrap_or(false)
}

/// Removes this frame's first press of `key` with exactly `modifiers`, if
/// any, and reports whether one was found.
pub(crate) fn take_exact(input: &mut egui::InputState, modifiers: Modifiers, key: Key) -> bool {
    let wanted = modifiers;
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
            } if *found == key && *modifiers == wanted
        );
        taken |= matches;
        !matches
    });
    taken
}

/// Takes this frame's first plain Space or, failing that, its first plain
/// Enter: the message list's current message takes the two alike, a held
/// key acting on each repeat.
fn take_space_or_enter(input: &mut egui::InputState) -> bool {
    take_plain(input, Key::Space) || take_plain(input, Key::Enter)
}

/// Handles keys while the image preview is open. No chat shortcut runs, and
/// typing and clipboard input are swallowed; Left/Right browse photos, while
/// Tab, Enter, Space and Up/Down stay for the preview's own controls.
fn preview_keys(app: &mut App, ctx: &egui::Context) {
    let mut actions = Vec::new();
    ctx.input_mut(|input| {
        if input.consume_key(Modifiers::NONE, Key::Escape) {
            actions.push(Action::CloseImagePreview);
        }
        let copy_shortcut = input.consume_key(Modifiers::COMMAND, Key::C)
            || input.consume_key(Modifiers::CTRL, Key::C);
        // The header leaves the copy action out while a clip is on screen, so
        // the shortcut does the same: copying a clip would decode a video as an
        // image and fail.
        if copy_shortcut
            && let Some(preview) = &app.image_preview
            && let Some(path) = preview.path()
            && !preview.shows_video(&app.viewer_media)
        {
            actions.push(Action::CopyImage(path.to_owned()));
        }
        let mut event_actions = Vec::new();
        for event in &input.events {
            if let egui::Event::Key {
                key,
                modifiers,
                pressed: true,
                ..
            } = event
            {
                event_actions.extend(crate::image_preview::preview_action(*key, *modifiers));
            }
        }
        // A clip has no zoom, as its header has no zoom controls.
        let clip = app
            .image_preview
            .as_ref()
            .is_some_and(|preview| preview.shows_video(&app.viewer_media));
        if clip {
            event_actions.retain(|action| {
                !matches!(
                    action,
                    Action::ZoomImageIn
                        | Action::ZoomImageOut
                        | Action::FitImage
                        | Action::ImageActualSize
                )
            });
        }
        actions.extend(event_actions);
        input
            .events
            .retain(|event| !crate::image_preview::consumes_key(event));
    });
    app.actions.extend(actions);
}

/// Keeps keyboard focus where it was on an Escape in the window itself,
/// anywhere in the app, after keyboard navigation or with a screen reader
/// attached; a dialog or the update notice keep their own handling of it.
/// egui lets go of focus on a plain Escape at the start of the pass, before
/// anything is drawn, unless the focused widget claims the key, and a screen
/// reader then hears focus fall to the window. Given back in this pass,
/// which draws the widget again, focus stays on it while Escape still does
/// what `handle` gives it to do: it closes or clears the topmost thing, up
/// to the open chat itself. When that takes the focused widget away, as
/// closing the chat does, focus goes with it once the widget is no longer
/// drawn, as before; with nothing to close, focus stays where it was.
fn keep_focus_on_escape(app: &App, ctx: &egui::Context) {
    if app.dialog.is_some() || app.show_update || ctx.memory(|memory| memory.focused().is_some()) {
        return;
    }
    // Only for the keyboard: a pointer user who clicked into a field sees
    // it let go on Escape, as egui does.
    if escape_keeps_focus(ctx)
        && let Some(id) = focus_before_escape(ctx)
    {
        ctx.memory_mut(|memory| memory.request_focus(id));
    }
}

/// The widget that had focus when this pass began, among those the last
/// pass drew.
fn focus_before_escape(ctx: &egui::Context) -> Option<egui::Id> {
    let drawn: Vec<egui::Id> = ctx.viewport(|viewport| {
        viewport
            .prev_pass
            .widgets
            .layers()
            .flat_map(|(_, widgets)| widgets.iter().map(|widget| widget.id))
            .collect()
    });
    ctx.memory(|memory| {
        drawn
            .into_iter()
            .find(|id| memory.had_focus_last_frame(*id))
    })
}

/// Whether Escape keeps focus where it was: after keyboard navigation, or
/// with a screen reader attached, which always hears where focus goes.
fn escape_keeps_focus(ctx: &egui::Context) -> bool {
    keyboard_or_reader(ctx)
}

/// Whether focus is followed: after keyboard navigation, or with a screen
/// reader attached, which always hears where focus goes. A pointer user
/// sees focus move only on a text field, as its caret and outline.
pub(crate) fn keyboard_or_reader(ctx: &egui::Context) -> bool {
    let keyboard = ctx.data(|data| {
        data.get_temp::<bool>(crate::theme::keyboard_focus_id())
            .unwrap_or(false)
    });
    // A node builder exists only while AccessKit is on.
    let screen_reader = ctx
        .accesskit_node_builder(egui::accesskit_root_id(), |_| ())
        .is_some();
    keyboard || screen_reader
}

/// Drops the text of a digit shortcut taken this frame: a digit with Alt,
/// or with Control on macOS, such as Alt+2. Windows delivers Alt+2 as the
/// key and as the text "2": with the key taken by its shortcut, the text
/// would still reach a focused field and be typed there. Control with a
/// digit types nothing.
fn drop_shortcut_text(ctx: &egui::Context) {
    let held = |modifiers: &Modifiers| {
        if cfg!(target_os = "macos") {
            modifiers.ctrl
        } else {
            modifiers.alt
        }
    };
    ctx.input_mut(|input| {
        // Taken: in this frame's input, but no longer among its events.
        let taken: Vec<&str> = input
            .raw
            .events
            .iter()
            .filter(|event| !input.events.contains(event))
            .filter_map(|event| match event {
                egui::Event::Key {
                    key:
                        key @ (Key::Num0
                        | Key::Num1
                        | Key::Num2
                        | Key::Num3
                        | Key::Num4
                        | Key::Num5
                        | Key::Num6
                        | Key::Num7
                        | Key::Num8
                        | Key::Num9),
                    pressed: true,
                    modifiers,
                    ..
                } if held(modifiers) => Some(key.name()),
                _ => None,
            })
            .collect();
        for digit in taken {
            if let Some(index) = input
                .events
                .iter()
                .position(|event| matches!(event, egui::Event::Text(text) if text == digit))
            {
                input.events.remove(index);
            }
        }
    });
}

/// Handles keys while the cropper covers the window: Escape leaves the
/// staged picture alone, Enter keeps the crop. Enter on a focused button
/// the reader can reach, the cropper's own or the window's above it,
/// presses that button instead, as anywhere else, so Enter on Cancel
/// cancels and Enter on Turn left turns the picture. A control under the
/// cropper, such as the message field it was opened over, leaves Enter to
/// the crop, also in the cropper's first frame, before egui knows it covers
/// that control.
fn editor_keys(app: &mut App, ctx: &egui::Context) {
    let focused = ctx.memory(|memory| memory.focused()).and_then(|focused| {
        ctx.viewport(|viewport| {
            viewport
                .prev_pass
                .widgets
                .get(focused)
                .map(|widget| (widget.layer_id, widget.sense.senses_click()))
        })
    });
    let cropper = egui::LayerId::new(egui::Order::Foreground, super::image_editor::area_id());
    let on_button = focused.is_some_and(|(layer, clicks)| {
        clicks
            && ctx.memory(|memory| {
                // The cropper is the modal layer from the pass after its
                // first: until then everything takes the pointer and the
                // keys, the message field under it too, so only a layer
                // drawn above every Foreground one is surely above it.
                memory.allows_interaction(layer)
                    && (memory.top_modal_layer() == Some(cropper)
                        || layer.order > egui::Order::Foreground)
            })
    });
    let mut actions = Vec::new();
    ctx.input_mut(|input| {
        if input.consume_key(Modifiers::NONE, Key::Escape) {
            actions.push(Action::CancelPictureEdit);
        }
        if !on_button && input.consume_key(Modifiers::NONE, Key::Enter) {
            actions.push(Action::ApplyPictureEdit);
        }
    });
    app.actions.extend(actions);
}

/// Handles a playing clip's keys while the viewer shows it: Space plays or
/// pauses, M mutes, S changes the speed, and the arrows jump five seconds, as
/// they step through the album only while a picture is on the stage. Escape
/// closes the viewer, which puts a video back in its message.
fn video_keys(app: &mut App, ctx: &egui::Context) {
    let Some((message, path)) = app
        .video
        .message()
        .map(str::to_owned)
        .zip(app.video.path().map(std::path::Path::to_owned))
    else {
        return;
    };
    // Only the clip on the viewer: anything else was stopped as it opened.
    let on_screen = app.image_preview.as_ref().is_some_and(|preview| {
        preview.message() == message
            && preview.shows_video(&app.viewer_media)
            && preview.clip_path() == Some(path.as_path())
    });
    if !on_screen {
        return;
    }
    let status = app.video.status(&message);
    let jump = |seconds: f32| {
        let status = status.as_ref()?;
        let total = status.total.as_secs_f32();
        (total > 0.0).then(|| Action::SeekVideo {
            message: message.clone(),
            fraction: ((status.position.as_secs_f32() + seconds) / total).clamp(0.0, 1.0),
        })
    };
    let mut actions = Vec::new();
    ctx.input_mut(|input| {
        if input.consume_key(Modifiers::NONE, Key::Space) {
            actions.push(Action::PlayVideo {
                message: message.clone(),
                path: path.clone(),
            });
        }
        if input.consume_key(Modifiers::NONE, Key::M) {
            actions.push(Action::ToggleVideoSound);
        }
        if input.consume_key(Modifiers::NONE, Key::S) {
            actions.push(Action::CycleVideoSpeed);
        }
        if input.consume_key(Modifiers::NONE, Key::ArrowLeft) {
            actions.extend(jump(-5.0));
        }
        if input.consume_key(Modifiers::NONE, Key::ArrowRight) {
            actions.extend(jump(5.0));
        }
    });
    app.actions.extend(actions);
}

/// Handles keys while a sticker is shown large: Escape closes it, and no
/// chat shortcut or typing reaches the chat behind it.
fn sticker_view_keys(app: &mut App, ctx: &egui::Context) {
    let close = ctx.input_mut(|input| {
        let close = input.consume_key(Modifiers::NONE, Key::Escape);
        input
            .events
            .retain(|event| !crate::image_preview::consumes_key(event));
        close
    });
    if close {
        app.actions.push(Action::CloseStickerPreview);
    }
}

/// Ctrl+F searches the open chat, as in WhatsApp, the chat list when no
/// chat is open, and the settings on the Settings page.
fn find_action(app: &App) -> Action {
    match app.page {
        Page::Settings => Action::FocusSettingsSearch,
        Page::Chats | Page::Calls if app.open_chat.is_some() => Action::OpenChatSearch,
        _ => Action::FocusSearch,
    }
}

/// Ctrl+E archives the open chat, or unarchives an archived one, as in
/// WhatsApp. Archiving closes the chat, as the chat menus do.
fn archive_action(app: &App) -> Option<Action> {
    if !app.chat_view()
        || app.dialog.is_some()
        || app.show_update
        || app.picker.is_some()
        || app.reaction_target.is_some()
        || app.recording.is_some()
    {
        return None;
    }
    let chat = app.chat(app.open_chat.as_deref()?)?;
    Some(Action::SetArchived(chat.id.clone(), !chat.archived))
}

/// Ctrl+Shift+A shows the next linked number, in the switcher's order and
/// wrapping after the last. An account still being added is left through
/// its own Cancel, not by this shortcut.
fn next_account_action(app: &App) -> Option<Action> {
    if !app.has_several_accounts()
        || app.adding_account
        || app.dialog.is_some()
        || app.show_update
        || app.picker.is_some()
        || app.reaction_target.is_some()
        || app.recording.is_some()
    {
        return None;
    }
    let next = (app.active + 1) % app.accounts.len();
    Some(Action::SwitchAccount(app.accounts[next].id.clone()))
}

/// Control held with no other modifier: on macOS, Command+Control is
/// Command's, which `consume_key` would also match as Control.
fn control_alone(modifiers: Modifiers) -> bool {
    modifiers.ctrl && !modifiers.command && !modifiers.mac_cmd && !modifiers.alt && !modifiers.shift
}

/// The message list's modifier held alone: Control on macOS, Alt elsewhere.
fn messages_modifier_alone(modifiers: Modifiers) -> bool {
    if cfg!(target_os = "macos") {
        control_alone(modifiers)
    } else {
        modifiers.matches_exact(Modifiers::ALT)
    }
}

/// The message list's shortcut: Alt+2, but Control+2 on macOS, where
/// Command with a digit opens a chat. `label` shows "Control" as ⌃ there.
const MESSAGES_KEYS: &str = if cfg!(target_os = "macos") {
    "Control+2"
} else {
    "Alt+2"
};

/// Deletes the current message for me. A Mac's Delete key is Backspace, and
/// Fn+Delete is Delete.
const DELETE_KEYS: &str = if cfg!(target_os = "macos") {
    "Delete / Fn+Delete"
} else {
    "Delete"
};

/// Deletes the current message for everyone, the same keys with Shift.
const DELETE_FOR_EVERYONE_KEYS: &str = if cfg!(target_os = "macos") {
    "Shift+Delete / Shift+Fn+Delete"
} else {
    "Shift+Delete"
};

/// Shortcuts shown in the help dialog.
pub const SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+K / Ctrl+Shift+F", "Search chats"),
    ("Ctrl+F", "Search the open chat or the settings"),
    (
        "↑ / ↓, Enter",
        "Walk the open chat's search results and jump to one",
    ),
    ("Ctrl+L", "Focus the message input"),
    ("Alt+↑ / Alt+↓", "Previous / next chat"),
    ("Ctrl+Shift+[ / ]", "Previous / next chat, as in WhatsApp"),
    ("Ctrl+1..9", "Open a chat by its position in the chat list"),
    ("Ctrl+Shift+1", "Chats"),
    ("Ctrl+Shift+2", "Calls"),
    ("↑", "Edit the previous message (when the input is empty)"),
    ("Enter", "Send (Shift+Enter for a new line)"),
    (
        "Escape",
        "Dismiss the current action, return from search, Calls or Starred, or close the chat",
    ),
    ("Ctrl+N", "New chat or message yourself"),
    ("Ctrl+E", "Archive or unarchive the open chat"),
    ("Ctrl+Shift+A", "Switch to the next linked number"),
    (
        "Ctrl+V",
        "Paste text, or stage a picture from the clipboard",
    ),
    ("Ctrl+C", "Copy the selected messages"),
    ("Ctrl+B", "Collapse or expand the chat list"),
    (
        "Ctrl+End",
        "Jump to the newest message (when the input is empty)",
    ),
    (MESSAGES_KEYS, "Go to the message list"),
    (
        "↑ / ↓, Home / End, PgUp / PgDn",
        "Move through messages (in the message list)",
    ),
    ("Shift+F10", "Message menu (in the message list)"),
    ("R", "Reply to the message (in the message list)"),
    ("E", "React to the message (in the message list)"),
    ("C", "Copy the message (in the message list)"),
    ("F", "Forward the message (in the message list)"),
    (
        DELETE_KEYS,
        "Delete the message for me (in the message list)",
    ),
    (
        DELETE_FOR_EVERYONE_KEYS,
        "Delete the message for everyone (in the message list)",
    ),
    (
        "Space / Enter",
        "Play or pause, open or vote for the selected item, downloading it first when needed (in the message list)",
    ),
    (
        "← / →",
        "Move between the items of a message: its file, links, phone numbers, cards or poll options (in the message list)",
    ),
    ("G", "Go to the replied message (in the message list)"),
    ("PgUp / PgDn", "Scroll the open chat by a page"),
    (
        "Home / End",
        "Top / bottom of the open chat (when the input is empty)",
    ),
    ("Ctrl+,", "Settings"),
    ("Ctrl++ / Ctrl+-", "Zoom in / out"),
    ("Ctrl+0", "Reset zoom"),
    ("← / →", "Previous / next item in the media viewer"),
    ("? / Ctrl+/", "Keyboard shortcuts (? when not typing)"),
    ("Ctrl+Shift+L", "Lock ZapFast (with an app lock password)"),
    ("Ctrl+W", "Close the window (ZapFast remains in the tray)"),
    ("Ctrl+Q", "Quit"),
];

/// Uses Command and Option labels on macOS.
pub fn label(keys: &str) -> String {
    if cfg!(target_os = "macos") {
        keys.replace("Control", "⌃")
            .replace("Ctrl", "⌘")
            .replace("Strg", "⌘")
            .replace("Alt", "⌥")
    } else {
        keys.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_f_searches_the_open_chat_the_list_or_the_settings() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        assert!(matches!(find_action(&app), Action::FocusSearch));
        app.open_chat = Some("1@s.whatsapp.net".into());
        app.page = Page::Chats;
        assert!(matches!(find_action(&app), Action::OpenChatSearch));
        app.page = Page::Settings;
        assert!(matches!(find_action(&app), Action::FocusSettingsSearch));
        app.page = Page::Wallpaper;
        assert!(matches!(find_action(&app), Action::FocusSearch));
    }

    /// An app whose open chat holds `messages`, by id and content, in order,
    /// with the folder it keeps its files in, which goes when dropped.
    fn app_with(messages: Vec<(&str, crate::model::Content)>) -> (tempfile::TempDir, App, String) {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        let chat = "1@s.whatsapp.net".to_owned();
        let conversation = app.conversations.entry(chat.clone()).or_default();
        for (index, (id, content)) in messages.into_iter().enumerate() {
            conversation.messages.push(crate::model::Message {
                id: id.into(),
                chat: chat.clone(),
                sender: chat.clone(),
                sender_name: None,
                from_me: false,
                timestamp: 1 + index as i64,
                history_order: None,
                content,
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
        app.open_chat = Some(chat.clone());
        (root, app, chat)
    }

    fn media(mime: &str, path: Option<&str>) -> crate::model::Media {
        crate::model::Media {
            mime: mime.into(),
            size: 10_000,
            width: None,
            height: None,
            path: path.map(Into::into),
            state: Default::default(),
            album: None,
        }
    }

    /// Runs `message_keys` on one press of `key`, and what it announced.
    fn message_key(app: &mut App, chat: &str, id: &str, key: Key) -> (Vec<Action>, String) {
        message_key_with(app, chat, id, key, Modifiers::NONE)
    }

    /// Command on macOS, Ctrl elsewhere.
    const CTRL: Modifiers = Modifiers::COMMAND;

    /// [`message_key`] with `modifiers` held.
    fn message_key_with(
        app: &mut App,
        chat: &str,
        id: &str,
        key: Key,
        modifiers: Modifiers,
    ) -> (Vec<Action>, String) {
        app.announcer = Default::default();
        let ctx = egui::Context::default();
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            },
            |ui| {
                actions = message_keys(app, ui.ctx(), chat, id);
                app.announcer.show(ui.ctx(), true);
            },
        );
        output.textures_delta.clear();
        let spoken = app.announcer.spoken().concat();
        (actions, spoken)
    }

    fn three_links() -> crate::model::Content {
        crate::model::Content::text("see https://a.org, b.org/path and https://c.org")
    }

    fn opens(actions: &[Action]) -> Vec<&str> {
        actions
            .iter()
            .filter_map(|action| match action {
                Action::OpenUrl(url) => Some(url.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Left and Right walk a text's links one by one and say where they are,
    /// with no wrap: at either end they say the end link again. Space opens
    /// the link the cursor is on.
    #[test]
    fn arrows_walk_the_links_of_a_text_and_space_opens_the_current_one() {
        let (_root, mut app, chat) = app_with(vec![("text", three_links())]);
        let mut key = |key: Key| message_key(&mut app, &chat, "text", key);
        let (actions, spoken) = key(Key::Space);
        assert_eq!(opens(&actions), ["https://a.org"], "the first link");
        assert_eq!(spoken, "");
        for said in [
            "Link 2 of 3, b.org/path",
            "Link 3 of 3, https://c.org",
            "Link 3 of 3, https://c.org",
        ] {
            assert_eq!(key(Key::ArrowRight).1, said);
        }
        assert_eq!(opens(&key(Key::Space).0), ["https://c.org"]);
        for said in [
            "Link 2 of 3, b.org/path",
            "Link 1 of 3, https://a.org",
            "Link 1 of 3, https://a.org",
        ] {
            assert_eq!(key(Key::ArrowLeft).1, said);
        }
        // One link leaves the arrows alone.
        let (_root, mut app, chat) =
            app_with(vec![("one", crate::model::Content::text("https://a.org"))]);
        let (actions, spoken) = message_key(&mut app, &chat, "one", Key::ArrowRight);
        assert!(actions.is_empty() && spoken.is_empty(), "{spoken}");
    }

    /// An album's photo, downloaded to `path` or not.
    fn album_photo(path: Option<&str>, caption: Option<&str>) -> crate::model::Content {
        let mut media = media("image/png", path);
        media.album = Some("album-fixture".into());
        crate::model::Content::Image {
            motion: None,
            caption: caption.map(Into::into),
            media,
        }
    }

    /// An album is one message of the list whose items are its photos and
    /// videos, each followed by the links of its caption: the arrows walk
    /// them, Space does what clicking the tile does, and the other keys act
    /// on the item the cursor is on, as the pointer's menu does on the tile
    /// it was opened over.
    #[test]
    fn the_arrows_walk_an_albums_items_and_the_keys_act_on_the_current_one() {
        let (_root, mut app, chat) = app_with(vec![
            ("one", album_photo(Some("one.png"), None)),
            ("two", album_photo(None, Some("see https://a.org"))),
            ("three", album_photo(Some("three.png"), None)),
        ]);
        let (actions, _) = message_key(&mut app, &chat, "one", Key::Space);
        assert_eq!(
            actions,
            [Action::PreviewImage {
                path: "one.png".into(),
                chat: chat.clone(),
                message: "one".into(),
                card: None,
            }],
            "the first photo opens in the viewer"
        );
        let (_, spoken) = message_key(&mut app, &chat, "one", Key::ArrowRight);
        assert_eq!(spoken, "Photo 2 of 4, not downloaded");
        let (actions, _) = message_key(&mut app, &chat, "one", Key::Space);
        assert!(
            matches!(
                actions.as_slice(),
                [Action::Download { message, manual: true, .. }] if message == "two"
            ),
            "the second photo is fetched: {actions:?}"
        );
        let (_, spoken) = message_key(&mut app, &chat, "one", Key::ArrowRight);
        assert_eq!(spoken, "Link 3 of 4, https://a.org");
        let (actions, _) = message_key(&mut app, &chat, "one", Key::R);
        assert_eq!(
            actions,
            [Action::Reply("two".into()), Action::FocusComposer],
            "the caption's item replies"
        );
        let (_, spoken) = message_key(&mut app, &chat, "one", Key::ArrowRight);
        assert_eq!(spoken, "Photo 4 of 4");
        let (actions, _) = message_key_with(&mut app, &chat, "one", Key::F10, Modifiers::SHIFT);
        assert_eq!(actions, [Action::OpenMessageMenu("three".into())]);
        assert_eq!(
            app.message_cursor.get(&chat).map(String::as_str),
            Some("one"),
            "the list stays on the album"
        );
    }

    /// Two album items can caption the same link. The cursor keeps the one
    /// it is on, not the same link of another item, when the items before
    /// it change.
    #[test]
    fn an_albums_cursor_keeps_its_item_when_the_items_before_it_change() {
        let (_root, mut app, chat) = app_with(vec![
            (
                "one",
                album_photo(Some("one.png"), Some("see https://b.org or https://c.org")),
            ),
            (
                "two",
                album_photo(Some("two.png"), Some("see https://a.org")),
            ),
            (
                "three",
                album_photo(Some("three.png"), Some("see https://a.org")),
            ),
        ]);
        for _ in 0..4 {
            message_key(&mut app, &chat, "one", Key::ArrowRight);
        }
        let (actions, _) = message_key(&mut app, &chat, "one", Key::R);
        assert_eq!(
            actions,
            [Action::Reply("two".into()), Action::FocusComposer]
        );
        // The first photo's caption loses its links: two items fewer before
        // the cursor's, which now has the third photo's link at its place.
        app.conversations
            .get_mut(&chat)
            .unwrap()
            .message_mut("one")
            .unwrap()
            .content = album_photo(Some("one.png"), None);
        let (actions, _) = message_key(&mut app, &chat, "one", Key::R);
        assert_eq!(
            actions,
            [Action::Reply("two".into()), Action::FocusComposer],
            "still the second photo's link"
        );
    }

    /// The album item under the cursor is gone: the keys that act on an item
    /// say where the cursor is now, as Space does, rather than act on the
    /// album's first item unheard.
    #[test]
    fn the_keys_on_an_album_item_that_is_gone_say_the_first_instead() {
        let keys = [
            (Key::R, Modifiers::NONE),
            (Key::E, Modifiers::NONE),
            (Key::F, Modifiers::NONE),
            (Key::Delete, Modifiers::NONE),
            (Key::F10, Modifiers::SHIFT),
        ];
        for (key, modifiers) in keys {
            let (_root, mut app, chat) = app_with(vec![
                ("one", album_photo(Some("one.png"), None)),
                ("two", album_photo(Some("two.png"), None)),
                ("three", album_photo(Some("three.png"), None)),
            ]);
            message_key(&mut app, &chat, "one", Key::ArrowRight);
            app.conversations
                .get_mut(&chat)
                .unwrap()
                .messages
                .retain(|message| message.id != "two");
            let (actions, spoken) = message_key_with(&mut app, &chat, "one", key, modifiers);
            assert!(actions.is_empty(), "{key:?}: {actions:?}");
            assert_eq!(spoken, "Photo 1 of 2", "{key:?}");
            let (actions, _) = message_key_with(&mut app, &chat, "one", key, modifiers);
            assert!(!actions.is_empty(), "{key:?} acts once heard");
        }
    }

    /// Phone numbers are items between the links, in the order the text has
    /// them; Space on one asks for its menu, never a `tel:` address to open.
    #[test]
    fn space_on_a_phone_number_asks_for_its_menu() {
        let (_root, mut app, chat) = app_with(vec![(
            "text",
            crate::model::Content::text("see https://a.org or call +1 (555) 010-2040"),
        )]);
        let (_, spoken) = message_key(&mut app, &chat, "text", Key::ArrowRight);
        assert_eq!(spoken, "Phone number 2 of 2, +1 (555) 010-2040");
        let (actions, spoken) = message_key(&mut app, &chat, "text", Key::Space);
        assert_eq!(
            actions,
            [Action::OpenPhoneMenu {
                message: "text".into(),
                url: "tel:+15550102040".into(),
            }]
        );
        assert_eq!(spoken, "", "the menu speaks for itself");
        let (actions, _) = message_key(&mut app, &chat, "text", Key::ArrowLeft);
        assert!(actions.is_empty());
        assert_eq!(
            opens(&message_key(&mut app, &chat, "text", Key::Space).0),
            ["https://a.org"]
        );
    }

    /// Moving to another message and back starts its items over.
    #[test]
    fn the_item_cursor_starts_over_on_another_message() {
        let (_root, mut app, chat) =
            app_with(vec![("first", three_links()), ("second", three_links())]);
        app.message_cursor.insert(chat.clone(), "first".into());
        assert_eq!(
            message_key(&mut app, &chat, "first", Key::ArrowRight).1,
            "Link 2 of 3, b.org/path"
        );
        let ctx = egui::Context::default();
        for step in [
            crate::ui::list_nav::Step::Next,
            crate::ui::list_nav::Step::Previous,
        ] {
            app.actions.push(Action::MessageCursor(step));
            app.background_frame(&ctx);
        }
        assert_eq!(app.message_cursor[&chat], "first");
        assert_eq!(
            message_key(&mut app, &chat, "first", Key::ArrowRight).1,
            "Link 2 of 3, b.org/path",
            "the cursor started over"
        );
    }

    const SAMPLE_VIDEO: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/video/sample.mp4"
    );

    /// A poll of three options taking `selectable` answers, with `selected`
    /// chosen and one voter per chosen option.
    fn poll(selectable: usize, selected: Vec<usize>) -> crate::model::Content {
        let mut counts = vec![0; 3];
        for &option in &selected {
            counts[option] += 1;
        }
        crate::model::Content::Poll {
            question: "When?".into(),
            options: vec!["Morning".into(), "Afternoon".into(), "Evening".into()],
            state: crate::model::PollState {
                selectable,
                can_vote: true,
                history_complete: true,
                voters: selected.len(),
                counts,
                selected,
                ..Default::default()
            },
        }
    }

    fn votes(actions: &[Action]) -> Vec<Vec<usize>> {
        actions
            .iter()
            .filter_map(|action| match action {
                Action::VotePoll { choices, .. } => Some(choices.clone()),
                _ => None,
            })
            .collect()
    }

    /// The arrows walk a poll's options, each with its count and whether it
    /// is chosen, then "Show votes" once someone voted; Space on an option
    /// votes as its click does and says the choice it sends.
    #[test]
    fn the_arrows_walk_a_polls_options_and_space_votes_as_a_click() {
        let (_root, mut app, chat) = app_with(vec![("poll", poll(1, vec![0]))]);
        app.link = crate::backend::LinkStatus::Connected;
        let mut key = |key: Key| message_key(&mut app, &chat, "poll", key);
        // Untouched, Space votes for the first option: here it takes the
        // chosen answer back, as clicking it does.
        let (actions, spoken) = key(Key::Space);
        assert_eq!(votes(&actions), [Vec::<usize>::new()]);
        assert_eq!(spoken, "Morning, not selected");
        for said in [
            "Afternoon, 0 votes, not selected, 2 of 4",
            "Evening, 0 votes, not selected, 3 of 4",
            "Show votes 4 of 4",
            "Show votes 4 of 4",
        ] {
            assert_eq!(key(Key::ArrowRight).1, said);
        }
        let (actions, _) = key(Key::Space);
        assert!(
            matches!(
                actions.as_slice(),
                [Action::ShowDialog(crate::model::Dialog::PollResults { .. })]
            ),
            "{actions:?}"
        );
        assert_eq!(
            key(Key::ArrowLeft).1,
            "Evening, 0 votes, not selected, 3 of 4"
        );
        assert_eq!(
            key(Key::ArrowLeft).1,
            "Afternoon, 0 votes, not selected, 2 of 4"
        );
        assert_eq!(key(Key::ArrowLeft).1, "Morning, 1 vote, selected, 1 of 4");
        // One answer: choosing another replaces it, as a click does.
        key(Key::ArrowRight);
        let (actions, spoken) = key(Key::Space);
        assert_eq!(votes(&actions), [vec![1]]);
        assert_eq!(spoken, "Afternoon, selected");
    }

    /// A poll taking several answers adds to them, and stops at its limit,
    /// where the option says why and Space sends nothing.
    #[test]
    fn a_multi_answer_poll_votes_as_a_click_and_says_why_it_stops() {
        let (_root, mut app, chat) = app_with(vec![("poll", poll(2, vec![0, 2]))]);
        app.link = crate::backend::LinkStatus::Connected;
        let (_, spoken) = message_key(&mut app, &chat, "poll", Key::ArrowRight);
        assert_eq!(
            spoken,
            "Afternoon, 0 votes, not selected, 2 of 4, unavailable: Remove an answer first to choose another"
        );
        let (actions, spoken) = message_key(&mut app, &chat, "poll", Key::Space);
        assert!(votes(&actions).is_empty(), "{actions:?}");
        assert_eq!(
            spoken,
            "unavailable: Remove an answer first to choose another"
        );
        let (_, _) = message_key(&mut app, &chat, "poll", Key::ArrowRight);
        let (actions, spoken) = message_key(&mut app, &chat, "poll", Key::Space);
        assert_eq!(votes(&actions), [vec![0]], "taking Evening back");
        assert_eq!(spoken, "Evening, not selected");
        let (_root2, app2, chat2) = app_with(vec![("poll", poll(2, vec![0]))]);
        let mut app2 = app2;
        app2.link = crate::backend::LinkStatus::Connected;
        message_key(&mut app2, &chat2, "poll", Key::ArrowRight);
        let (actions, _) = message_key(&mut app2, &chat2, "poll", Key::Space);
        assert_eq!(votes(&actions), [vec![0, 1]], "adding Afternoon");
    }

    /// Offline, the options leave out whether they are chosen and say why
    /// they take no vote; Space sends nothing.
    #[test]
    fn a_poll_that_takes_no_vote_says_why() {
        let (_root, mut app, chat) = app_with(vec![("poll", poll(1, vec![0]))]);
        let (_, spoken) = message_key(&mut app, &chat, "poll", Key::ArrowRight);
        assert_eq!(
            spoken,
            "Afternoon, 0 votes, 2 of 4, unavailable: Reconnect to vote"
        );
        let (actions, spoken) = message_key(&mut app, &chat, "poll", Key::Space);
        assert!(actions.is_empty(), "{actions:?}");
        assert_eq!(spoken, "unavailable: Reconnect to vote");
    }

    /// A shared contact's Chat and Add are two items; a saved contact has
    /// no Add, as its bubble shows none.
    #[test]
    fn a_contacts_chat_and_add_are_items() {
        let contact = crate::model::Content::Contact {
            display_name: "Sara".into(),
            vcard: "BEGIN:VCARD\nFN:Sara Ali\nTEL;waid=15550101234:+1 555 010 1234\nEND:VCARD"
                .into(),
        };
        let (_root, mut app, chat) = app_with(vec![("contact", contact)]);
        let (_, spoken) = message_key(&mut app, &chat, "contact", Key::ArrowRight);
        assert_eq!(spoken, "Add contact 2 of 2");
        let (actions, _) = message_key(&mut app, &chat, "contact", Key::Space);
        assert!(
            matches!(
                actions.as_slice(),
                // Filled as the bubble's Add fills it: a card without a
                // first name of its own keeps the whole name there.
                [Action::NewContact { phone, first, .. }]
                    if phone == "15550101234" && first == "Sara Ali"
            ),
            "{actions:?}"
        );
        let (_, spoken) = message_key(&mut app, &chat, "contact", Key::ArrowLeft);
        assert_eq!(spoken, "Chat 1 of 2");
        let (actions, _) = message_key(&mut app, &chat, "contact", Key::Space);
        assert!(
            matches!(
                actions.as_slice(),
                [Action::StartChat { id, .. }] if id == "15550101234@s.whatsapp.net"
            ),
            "{actions:?}"
        );
        app.contacts.insert(
            "15550101234@s.whatsapp.net".into(),
            crate::model::Contact {
                id: "15550101234@s.whatsapp.net".into(),
                full_name: Some("Sara Ali".into()),
                first_name: None,
                push_name: None,
            },
        );
        let (actions, spoken) = message_key(&mut app, &chat, "contact", Key::ArrowRight);
        assert!(actions.is_empty() && spoken.is_empty(), "{spoken}");
    }

    /// What a key did on a message: what it queued, what it said, and the
    /// download it listens for.
    type KeyOutcome = (
        Vec<Action>,
        String,
        std::collections::HashSet<(ChatId, String, Option<usize>)>,
    );

    /// A kind of message for Space and Enter: its name, the chat's messages,
    /// the keys that move the item cursor first, and whether WhatsApp is
    /// connected.
    type KeyCase<'a> = (
        &'a str,
        Vec<(&'a str, crate::model::Content)>,
        Vec<Key>,
        bool,
    );

    /// What one press of Space and one of Enter each do on a message, after
    /// `walk` moved the item cursor.
    fn space_and_enter(
        messages: &[(&str, crate::model::Content)],
        id: &str,
        walk: &[Key],
        connected: bool,
    ) -> [KeyOutcome; 2] {
        [Key::Space, Key::Enter].map(|key| {
            let (_root, mut app, chat) = app_with(messages.to_vec());
            if connected {
                app.link = crate::backend::LinkStatus::Connected;
            }
            for &step in walk {
                message_key(&mut app, &chat, id, step);
            }
            let (actions, spoken) = message_key(&mut app, &chat, id, key);
            (actions, spoken, app.download_heard.clone())
        })
    }

    /// Enter does what Space does on every kind of message and on each of
    /// its items, and nothing where Space does nothing.
    #[test]
    fn enter_does_what_space_does_on_every_kind_of_message() {
        use crate::model::Content;
        let document = |path| Content::Document {
            media: media("application/pdf", path),
            file_name: "a.pdf".into(),
            caption: None,
            pages: None,
        };
        let photo = |path| Content::Image {
            motion: None,
            caption: None,
            media: media("image/png", path),
        };
        let contact = Content::Contact {
            display_name: "Sara".into(),
            vcard: "BEGIN:VCARD\nFN:Sara Ali\nTEL;waid=15550101234:+1 555 010 1234\nEND:VCARD"
                .into(),
        };
        let album = [
            ("m", album_photo(Some("one.png"), None)),
            ("two", album_photo(None, Some("see https://a.org"))),
        ];
        let cases: Vec<KeyCase> = vec![
            ("a link", vec![("m", three_links())], vec![], false),
            (
                "another link",
                vec![("m", three_links())],
                vec![Key::ArrowRight],
                false,
            ),
            (
                "a downloaded document",
                vec![("m", document(Some("a.pdf")))],
                vec![],
                false,
            ),
            (
                "a document to fetch",
                vec![("m", document(None))],
                vec![],
                false,
            ),
            ("a photo", vec![("m", photo(Some("a.png")))], vec![], false),
            ("a photo to fetch", vec![("m", photo(None))], vec![], false),
            (
                "a sticker",
                vec![(
                    "m",
                    Content::Sticker {
                        media: media("image/webp", Some("a.webp")),
                        animated: false,
                    },
                )],
                vec![],
                false,
            ),
            (
                "a video",
                vec![(
                    "m",
                    Content::Video {
                        caption: None,
                        media: media("video/mp4", Some("a.mp4")),
                        seconds: Some(3),
                        gif: false,
                        note: false,
                    },
                )],
                vec![],
                false,
            ),
            (
                "a voice message",
                vec![(
                    "m",
                    Content::Audio {
                        media: media("audio/ogg", Some("voice.ogg")),
                        seconds: Some(6),
                        voice_note: true,
                        waveform: Vec::new(),
                    },
                )],
                vec![],
                false,
            ),
            (
                "a location",
                vec![(
                    "m",
                    Content::Location {
                        latitude: 24.7,
                        longitude: 46.7,
                        name: Some("Office".into()),
                        address: None,
                    },
                )],
                vec![],
                false,
            ),
            (
                "a contact's chat",
                vec![("m", contact.clone())],
                vec![],
                false,
            ),
            (
                "a contact's Add",
                vec![("m", contact)],
                vec![Key::ArrowRight],
                false,
            ),
            (
                "a poll option",
                vec![("m", poll(1, vec![]))],
                vec![Key::ArrowRight],
                true,
            ),
            (
                "a poll option that takes no vote",
                vec![("m", poll(1, vec![]))],
                vec![],
                false,
            ),
            ("an album's first item", album.to_vec(), vec![], false),
            (
                "an album item to fetch",
                album.to_vec(),
                vec![Key::ArrowRight],
                false,
            ),
            (
                "a phone number",
                vec![(
                    "m",
                    Content::text("see https://a.org or call +1 (555) 010-2040"),
                )],
                vec![Key::ArrowRight],
                false,
            ),
        ];
        for (kind, messages, walk, connected) in cases {
            let [space, enter] = space_and_enter(&messages, "m", &walk, connected);
            assert!(
                !space.0.is_empty() || !space.1.is_empty(),
                "{kind}: Space does something"
            );
            assert_eq!(enter, space, "{kind}");
        }
        // A plain text message: Space does nothing, and Enter neither.
        let [space, enter] =
            space_and_enter(&[("m", Content::text("no links here"))], "m", &[], false);
        assert_eq!(space, (Vec::new(), String::new(), Default::default()));
        assert_eq!(enter, space, "a plain text message");
    }

    /// A saved contact loses its Add: the cursor that stood on Add does not
    /// start a chat instead, but says where it stands now.
    #[test]
    fn space_on_an_item_that_is_gone_says_the_first_instead() {
        let vcard = "BEGIN:VCARD\nFN:Sara Ali\nTEL;waid=15550101234:+1 555 010 1234\nEND:VCARD";
        let contact = crate::model::Content::Contact {
            display_name: "Sara".into(),
            vcard: vcard.into(),
        };
        let (_root, mut app, chat) = app_with(vec![("contact", contact)]);
        assert_eq!(
            message_key(&mut app, &chat, "contact", Key::ArrowRight).1,
            "Add contact 2 of 2"
        );
        app.contacts.insert(
            "15550101234@s.whatsapp.net".into(),
            crate::model::Contact {
                id: "15550101234@s.whatsapp.net".into(),
                full_name: Some("Sara Ali".into()),
                first_name: None,
                push_name: None,
            },
        );
        let (actions, spoken) = message_key(&mut app, &chat, "contact", Key::Space);
        assert!(actions.is_empty(), "{actions:?}");
        assert_eq!(spoken, "Chat 1 of 1");
        let (actions, _) = message_key(&mut app, &chat, "contact", Key::Space);
        assert!(
            matches!(actions.as_slice(), [Action::StartChat { .. }]),
            "{actions:?}"
        );
    }

    /// An item moved under the cursor is found again: Space acts on the item
    /// that was heard.
    #[test]
    fn the_item_cursor_follows_its_item_when_the_message_changes() {
        let (_root, mut app, chat) = app_with(vec![("text", three_links())]);
        assert_eq!(
            message_key(&mut app, &chat, "text", Key::ArrowRight).1,
            "Link 2 of 3, b.org/path"
        );
        // An edit puts a new link first.
        app.conversations.get_mut(&chat).unwrap().messages[0].content = crate::model::Content::text(
            "new https://z.org then https://a.org, b.org/path and https://c.org",
        );
        let (actions, _) = message_key(&mut app, &chat, "text", Key::Space);
        assert_eq!(opens(&actions), ["https://b.org/path"]);
    }

    /// Walking a message's items keeps the list on it: Down at the bottom,
    /// where the list stood on the newest message without a cursor of its
    /// own, does not start the items over.
    #[test]
    fn the_item_cursor_survives_a_list_move_that_stays_on_its_message() {
        let (_root, mut app, chat) = app_with(vec![("text", three_links())]);
        assert!(!app.message_cursor.contains_key(&chat));
        message_key(&mut app, &chat, "text", Key::ArrowRight);
        assert_eq!(
            app.message_cursor.get(&chat).map(String::as_str),
            Some("text")
        );
        let ctx = egui::Context::default();
        app.message_cursor.remove(&chat);
        for step in [
            crate::ui::list_nav::Step::Next,
            crate::ui::list_nav::Step::Last,
        ] {
            app.actions.push(Action::MessageCursor(step));
            app.background_frame(&ctx);
        }
        let (actions, _) = message_key(&mut app, &chat, "text", Key::Space);
        assert_eq!(opens(&actions), ["https://b.org/path"]);
    }

    /// A GIF plays in place: plain Left and Right walk its caption's links.
    #[test]
    fn plain_arrows_walk_a_gifs_items() {
        let (_root, mut app, chat) = app_with(vec![(
            "gif",
            crate::model::Content::Video {
                caption: Some("from https://a.org".into()),
                media: media("video/mp4", Some(SAMPLE_VIDEO)),
                seconds: Some(3),
                gif: true,
                note: false,
            },
        )]);
        assert_eq!(
            message_key(&mut app, &chat, "gif", Key::ArrowRight).1,
            "Link 2 of 2, https://a.org"
        );
        assert_eq!(
            message_key(&mut app, &chat, "gif", Key::ArrowLeft).1,
            "GIF 1 of 2"
        );
    }

    /// A carousel's items go as the bubble draws them: the links in its
    /// text, above the cards, then each card followed by its own links.
    #[test]
    fn a_carousel_walks_its_text_then_each_card_and_its_links() {
        let card = |body: &str| crate::model::InteractiveCard {
            body: body.into(),
            ..Default::default()
        };
        let carousel = crate::model::Content::Interactive {
            text: String::new(),
            card: Some(Box::new(crate::model::InteractiveCard {
                body: "All at https://all.org".into(),
                carousel: vec![card("One https://one.org"), card("Two")],
                ..Default::default()
            })),
        };
        let (_root, mut app, chat) = app_with(vec![("cards", carousel)]);
        for said in [
            "Card 2 of 4, One https://one.org",
            "Link 3 of 4, https://one.org",
            "Card 4 of 4, Two",
        ] {
            assert_eq!(
                message_key(&mut app, &chat, "cards", Key::ArrowRight).1,
                said
            );
        }
    }

    /// G takes no key from a message that replies to nothing.
    #[test]
    fn g_is_left_alone_without_a_quote() {
        let (_root, mut app, chat) = app_with(vec![("text", three_links())]);
        let ctx = egui::Context::default();
        let mut left = 0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: Key::G,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| {
                assert!(message_keys(&mut app, ui.ctx(), &chat, "text").is_empty());
                left = ui.input(|input| input.events.len());
            },
        );
        output.textures_delta.clear();
        assert_eq!(left, 1, "G is still there for others");
    }

    /// A poll option blocked by the voting key says why, translated and
    /// without the ballot's middle dot.
    #[test]
    fn a_blocked_option_says_why_in_words() {
        let mut content = poll(1, vec![0]);
        if let crate::model::Content::Poll { state, .. } = &mut content {
            state.can_vote = false;
        }
        let (_root, mut app, chat) = app_with(vec![("poll", content)]);
        app.link = crate::backend::LinkStatus::Connected;
        let (_, spoken) = message_key(&mut app, &chat, "poll", Key::Space);
        assert_eq!(
            spoken,
            "unavailable: Voting key unavailable, use your phone"
        );
    }

    /// What the item cursor says (a link, a number, a poll option) is what
    /// the message holds: it goes from the tree behind the lock, as the
    /// messages themselves do, and once another account is on screen.
    #[test]
    fn what_the_items_say_is_private() {
        let (_root, mut app, chat) = app_with(vec![("text", three_links())]);
        let (_, spoken) = message_key(&mut app, &chat, "text", Key::ArrowRight);
        assert_eq!(spoken, "Link 2 of 3, b.org/path");
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            // Locked: private text is not shown and leaves the tree.
            app.announcer.show(ui.ctx(), false);
        });
        output.textures_delta.clear();
        assert_eq!(app.announcer.spoken().concat(), "", "gone behind the lock");
        let (_root, mut app, chat) = app_with(vec![("poll", poll(1, vec![0]))]);
        app.link = crate::backend::LinkStatus::Connected;
        let (_, spoken) = message_key(&mut app, &chat, "poll", Key::Space);
        assert_eq!(spoken, "Morning, not selected");
        app.announcer.forget_private();
        assert_eq!(app.announcer.spoken().concat(), "", "a vote's choice too");
    }

    /// An app with three texts in its open chat, the message list focused.
    fn focused_list() -> (tempfile::TempDir, App, String, egui::Context) {
        let (root, mut app, chat) = app_with(vec![
            ("one", crate::model::Content::text("One")),
            (
                "two",
                crate::model::Content::text("Two https://a.org https://b.org"),
            ),
            ("three", crate::model::Content::text("Three")),
        ]);
        app.page = Page::Chats;
        app.message_cursor.insert(chat.clone(), "two".into());
        (root, app, chat, egui::Context::default())
    }

    /// `press` with the message list of `chat` focused, and the actions it
    /// queued.
    fn press_in_list(app: &mut App, ctx: &egui::Context, chat: &str, key: Key) -> Vec<Action> {
        app.actions.clear();
        let list = crate::ui::conversation::messages_list_id(chat);
        ctx.memory_mut(|memory| memory.request_focus(list));
        press(app, ctx, key, Modifiers::NONE);
        std::mem::take(&mut app.actions)
    }

    /// The keys that act on one message (Enter, Space, R, E, C, F, Delete,
    /// the item arrows) wait while the chat selects messages: the bubbles
    /// draw no menu then, and the keys would act behind the selection's back.
    /// Up and Down still move the list.
    #[test]
    fn a_selection_holds_the_keys_that_act_on_a_message() {
        let (_root, mut app, chat, ctx) = focused_list();
        let keys = [
            Key::Enter,
            Key::Space,
            Key::R,
            Key::E,
            Key::C,
            Key::F,
            Key::Delete,
        ];
        for key in keys {
            assert!(
                !press_in_list(&mut app, &ctx, &chat, key).is_empty(),
                "{key:?} acts without a selection"
            );
        }
        press_in_list(&mut app, &ctx, &chat, Key::ArrowRight);
        assert!(app.message_item.is_some(), "Right walks the items");
        app.message_item = None;
        app.selection = Some((chat.clone(), Vec::new()));
        for key in keys {
            let actions = press_in_list(&mut app, &ctx, &chat, key);
            assert!(actions.is_empty(), "{key:?} while selecting: {actions:?}");
        }
        press_in_list(&mut app, &ctx, &chat, Key::ArrowRight);
        assert!(app.message_item.is_none(), "nor do the items move");
        assert_eq!(
            press_in_list(&mut app, &ctx, &chat, Key::ArrowUp),
            [Action::MessageCursor(crate::ui::list_nav::Step::Previous)]
        );
    }

    /// The call screen covers the chats: the message keys leave the list
    /// behind it alone, and come back once the screen is put aside.
    #[test]
    fn the_call_screen_holds_the_message_keys() {
        let (_root, mut app, chat, ctx) = focused_list();
        app.call = Some(crate::calls::CallUpdate {
            generation: 1,
            chat: chat.clone(),
            direction: crate::model::CallDirection::Outgoing,
            video: false,
            phase: crate::calls::CallPhase::Active,
            started: Some(std::time::Instant::now()),
            muted: false,
            camera_on: false,
            remote_video: false,
            video_requested: false,
            outcome: None,
            peer_audio: None,
            lost_devices: Vec::new(),
            microphone: None,
            speaker: None,
            camera: None,
            group: None,
        });
        assert_eq!(press_in_list(&mut app, &ctx, &chat, Key::R), []);
        app.call_surface_hidden = true;
        assert_eq!(
            press_in_list(&mut app, &ctx, &chat, Key::R),
            [Action::Reply("two".into()), Action::FocusComposer]
        );
    }

    /// One frame with the message list of `chat` focused, `events` in it and
    /// `modifiers` held: the actions it queued, and what it said.
    fn in_list_with(
        app: &mut App,
        ctx: &egui::Context,
        chat: &str,
        modifiers: Modifiers,
        events: Vec<egui::Event>,
    ) -> (Vec<Action>, String) {
        app.actions.clear();
        app.announcer = Default::default();
        let list = crate::ui::conversation::messages_list_id(chat);
        ctx.memory_mut(|memory| memory.request_focus(list));
        let held = std::iter::once(egui::Event::ModifiersChanged(modifiers));
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: held.chain(events).collect(),
                ..Default::default()
            },
            |ui| {
                handle(app, ui.ctx());
                app.announcer.show(ui.ctx(), true);
            },
        );
        output.textures_delta.clear();
        let spoken = app.announcer.spoken().concat();
        (std::mem::take(&mut app.actions), spoken)
    }

    /// Shift+Delete, as egui reports it outside Windows.
    fn shift_delete() -> egui::Event {
        egui::Event::Key {
            key: Key::Delete,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::SHIFT,
        }
    }

    /// The focused list's current message made ours, sent now.
    fn own_recent(app: &mut App, chat: &str) {
        let conversation = app.conversations.get_mut(chat).unwrap();
        let message = conversation
            .messages
            .iter_mut()
            .find(|message| message.id == "two")
            .unwrap();
        message.from_me = true;
        message.timestamp = crate::util::now();
    }

    /// Shift+Delete on our own recent message opens the confirmation that
    /// the menu's "Delete for everyone" opens, and deletes nothing itself.
    /// On Windows egui reports the press as Cut alone, with Shift held;
    /// Ctrl+X, also Cut, deletes nothing.
    #[test]
    fn shift_delete_asks_to_delete_the_message_for_everyone() {
        let (_root, mut app, chat, ctx) = focused_list();
        own_recent(&mut app, &chat);
        let everyone = crate::ui::conversation::delete_for_everyone_action(&chat, "two");
        let (actions, spoken) = in_list_with(
            &mut app,
            &ctx,
            &chat,
            Modifiers::SHIFT,
            vec![shift_delete()],
        );
        assert_eq!(actions, std::slice::from_ref(&everyone));
        assert_eq!(spoken, "");
        assert!(
            app.dialog.is_none(),
            "the dialog opens from the action, not before"
        );
        let (actions, _) = in_list_with(
            &mut app,
            &ctx,
            &chat,
            Modifiers::SHIFT,
            vec![egui::Event::Cut],
        );
        if cfg!(target_os = "windows") {
            assert_eq!(actions, [everyone]);
        } else {
            assert_eq!(actions, []);
        }
        let (actions, _) = in_list_with(
            &mut app,
            &ctx,
            &chat,
            Modifiers::COMMAND,
            vec![egui::Event::Cut],
        );
        assert_eq!(actions, [], "Ctrl+X");
        assert!(
            app.conversations[&chat]
                .message("two")
                .is_some_and(|message| !matches!(message.content, crate::model::Content::Revoked)),
            "nothing deleted"
        );
    }

    /// Where the menu offers no "Delete for everyone", Shift+Delete opens
    /// nothing and says why, as the keyboard menu names an unavailable entry.
    #[test]
    fn shift_delete_says_why_a_message_cannot_be_deleted_for_everyone() {
        let (_root, mut app, chat, ctx) = focused_list();
        let said = |app: &mut App| {
            let (actions, spoken) =
                in_list_with(app, &ctx, &chat, Modifiers::SHIFT, vec![shift_delete()]);
            assert_eq!(actions, []);
            spoken
        };
        assert_eq!(
            said(&mut app),
            "Delete for everyone, unavailable: Only messages you sent"
        );
        own_recent(&mut app, &chat);
        let two = app.conversations[&chat].message("two").unwrap().clone();
        let mut old = two.clone();
        old.timestamp = 1;
        let mut deleted = two;
        deleted.content = crate::model::Content::Revoked;
        for (message, reason) in [
            (old, "The time limit has passed"),
            (deleted, "This message was deleted"),
        ] {
            let messages = &mut app.conversations.get_mut(&chat).unwrap().messages;
            messages[1] = message;
            assert_eq!(
                said(&mut app),
                format!("Delete for everyone, unavailable: {reason}")
            );
        }
        assert!(app.dialog.is_none());
    }

    /// Shift+Delete waits during a selection and under the call screen, as
    /// the other message keys do, and while the message field has focus it
    /// is the field's.
    #[test]
    fn shift_delete_waits_as_the_other_message_keys_do() {
        let (_root, mut app, chat, ctx) = focused_list();
        own_recent(&mut app, &chat);
        app.selection = Some((chat.clone(), Vec::new()));
        let (actions, spoken) = in_list_with(
            &mut app,
            &ctx,
            &chat,
            Modifiers::SHIFT,
            vec![shift_delete()],
        );
        assert_eq!((actions, spoken.as_str()), (Vec::new(), ""), "selecting");
        app.selection = None;
        app.call_surface_hidden = false;
        app.call = Some(crate::calls::CallUpdate {
            generation: 1,
            chat: chat.clone(),
            direction: crate::model::CallDirection::Outgoing,
            video: false,
            phase: crate::calls::CallPhase::Active,
            started: Some(std::time::Instant::now()),
            muted: false,
            camera_on: false,
            remote_video: false,
            video_requested: false,
            outcome: None,
            peer_audio: None,
            lost_devices: Vec::new(),
            microphone: None,
            speaker: None,
            camera: None,
            group: None,
        });
        let (actions, spoken) = in_list_with(
            &mut app,
            &ctx,
            &chat,
            Modifiers::SHIFT,
            vec![shift_delete()],
        );
        assert_eq!((actions, spoken.as_str()), (Vec::new(), ""), "call screen");
        app.call = None;
        // Focus on the message field, not the list: no message is current.
        app.actions.clear();
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::ModifiersChanged(Modifiers::SHIFT),
                    shift_delete(),
                ],
                ..Default::default()
            },
            |ui| {
                ui.memory_mut(|memory| memory.request_focus(egui::Id::new("composer-text")));
                handle(&mut app, ui.ctx());
            },
        );
        output.textures_delta.clear();
        assert_eq!(app.actions, [], "typing");
    }

    /// A Mac's Delete key is Backspace: there it asks to delete the message
    /// for me, as Delete does; elsewhere Backspace is left alone.
    #[test]
    fn backspace_deletes_for_me_on_macos_only() {
        let (_root, mut app, chat) = app_with(vec![("text", three_links())]);
        let (actions, _) = message_key(&mut app, &chat, "text", Key::Backspace);
        if cfg!(target_os = "macos") {
            assert_eq!(
                actions,
                [crate::ui::conversation::delete_for_me_action(&chat, "text")]
            );
        } else {
            assert!(actions.is_empty(), "{actions:?}");
        }
        let (actions, _) = message_key_with(&mut app, &chat, "text", Key::Backspace, CTRL);
        assert!(actions.is_empty(), "only plain: {actions:?}");
    }

    #[test]
    fn escape_clears_the_settings_search_before_leaving_settings() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.page = Page::Settings;
        app.settings_search = "sound".into();
        let ctx = egui::Context::default();
        escape(&mut app, &ctx);
        assert!(
            matches!(app.actions.as_slice(), [Action::SearchSettings(text)] if text.is_empty())
        );
        app.actions.clear();
        app.settings_search.clear();
        escape(&mut app, &ctx);
        assert!(matches!(
            app.actions.as_slice(),
            [Action::Open(Page::Chats)]
        ));
    }

    #[test]
    fn escape_leaves_the_calls_page() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.page = Page::Calls;
        escape(&mut app, &egui::Context::default());
        assert!(matches!(
            app.actions.as_slice(),
            [Action::Open(Page::Chats)]
        ));
    }

    #[test]
    fn escape_closes_the_new_call_list_then_returns_to_the_chats() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.page = Page::Calls;
        app.new_call = true;
        app.open_chat = Some("ada@s.whatsapp.net".into());
        app.picked_call = Some(("ada@s.whatsapp.net".into(), "call-1".into()));
        let ctx = egui::Context::default();
        escape(&mut app, &ctx);
        assert!(matches!(
            app.actions.as_slice(),
            [Action::ShowNewCall(false)]
        ));
        app.actions.clear();
        app.new_call = false;
        // Beside the history, the picked chat stays open on the way out.
        escape(&mut app, &ctx);
        assert!(matches!(
            app.actions.as_slice(),
            [Action::Open(Page::Chats)]
        ));
    }

    /// Escape leaves the call screen rather than the call.
    ///
    /// The window shrinks back first and only the next Escape puts the surface aside, and neither
    /// one hangs up: a call the reader stepped away from is still running behind the bar. This is
    /// the rule the whole call surface turns on, so it is checked through the real key handler
    /// rather than only through the actions it pushes.
    #[test]
    fn escape_leaves_the_call_screen_without_ending_the_call() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.call = Some(crate::calls::CallUpdate {
            generation: 1,
            chat: "1@s.whatsapp.net".to_owned(),
            direction: crate::model::CallDirection::Outgoing,
            video: false,
            phase: crate::calls::CallPhase::Active,
            started: Some(std::time::Instant::now()),
            muted: false,
            camera_on: false,
            remote_video: false,
            video_requested: false,
            outcome: None,
            peer_audio: None,
            lost_devices: Vec::new(),
            microphone: None,
            speaker: None,
            camera: None,
            group: None,
        });
        app.call_fullscreen = true;
        let ctx = egui::Context::default();
        escape(&mut app, &ctx);
        assert!(
            matches!(app.actions.as_slice(), [Action::ToggleCallFullscreen]),
            "the window shrinks back first: {:?}",
            app.actions
        );
        app.actions.clear();
        // The window is back at its normal size, which is what that action does, so the next
        // Escape reaches the surface rather than the full-screen state.
        app.call_fullscreen = false;
        escape(&mut app, &ctx);
        assert!(
            matches!(app.actions.as_slice(), [Action::LeaveCallSurface]),
            "and only then does the surface step aside: {:?}",
            app.actions
        );
        assert!(
            !app.actions
                .iter()
                .any(|action| matches!(action, Action::HangupCall)),
            "neither Escape hangs up"
        );
        assert_eq!(
            app.call.as_ref().map(|call| call.phase),
            Some(crate::calls::CallPhase::Active),
            "Escape never ends the call"
        );
    }

    /// Copy is for a picture. The header leaves the action out while a clip is
    /// on screen, and the shortcut follows the same rule rather than decoding a
    /// video as an image and failing.
    #[test]
    fn ctrl_c_does_not_copy_a_clip_from_the_viewer() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        let ctx = egui::Context::default();
        let copy = |app: &mut App, ctx: &egui::Context| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: Key::C,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Modifiers::CTRL,
                    }],
                    ..Default::default()
                },
                |ui| handle(app, ui.ctx()),
            );
            output.textures_delta.clear();
        };
        let showing = |path: &str, message: &str| {
            Some(crate::image_preview::PreviewState::new(
                Some(std::path::PathBuf::from(path)),
                "fixture".into(),
                message.into(),
            ))
        };
        app.image_preview = showing("/cache/photo.png", "m1");
        copy(&mut app, &ctx);
        assert!(matches!(app.actions.as_slice(), [Action::CopyImage(_)]));
        app.actions.clear();
        app.image_preview = showing("/cache/clip.mp4", "m2");
        copy(&mut app, &ctx);
        assert!(app.actions.is_empty(), "a clip is played, not copied");
    }

    /// Under the cropper, Enter on a focused button the reader can reach
    /// presses that button, here one above the cropper, as ZapFast's own
    /// window buttons are; with focus on a control under the cropper, such
    /// as the message field it was opened over, Enter keeps the crop.
    #[test]
    fn the_croppers_enter_presses_a_reachable_button_and_keeps_the_crop_otherwise() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        let ctx = egui::Context::default();
        let cropper =
            egui::LayerId::new(egui::Order::Foreground, crate::ui::image_editor::area_id());
        let above = egui::Id::new("fixture-window-button");
        let under = egui::Id::new("fixture-field-under");
        let place = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(40.0, 40.0));
        let enter = |app: &mut App, focus: egui::Id| {
            app.actions.clear();
            for events in [
                Vec::new(),
                vec![egui::Event::Key {
                    key: Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
            ] {
                let pressed = !events.is_empty();
                ctx.memory_mut(|memory| memory.request_focus(focus));
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        if pressed {
                            editor_keys(app, ui.ctx());
                        }
                        let ctx = ui.ctx().clone();
                        ctx.memory_mut(|memory| memory.set_modal_layer(cropper));
                        egui::Area::new(egui::Id::new("fixture-under"))
                            .order(egui::Order::Middle)
                            .show(&ctx, |ui| {
                                ui.interact(place, under, egui::Sense::click());
                            });
                        egui::Area::new(egui::Id::new("fixture-above"))
                            .order(egui::Order::Tooltip)
                            .show(&ctx, |ui| {
                                ui.interact(place, above, egui::Sense::click());
                            });
                    },
                );
                output.textures_delta.clear();
            }
            std::mem::take(&mut app.actions)
        };
        assert!(
            enter(&mut app, above).is_empty(),
            "the button above the cropper takes Enter"
        );
        assert_eq!(enter(&mut app, under), [Action::ApplyPictureEdit]);
    }

    fn escape(app: &mut App, ctx: &egui::Context) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| handle(app, ui.ctx()),
        );
        output.textures_delta.clear();
    }

    #[test]
    fn escape_folds_the_day_filter_then_the_search_pane_then_the_chat() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.page = Page::Chats;
        app.open_chat = Some("fixture".into());
        app.chat_search_open = true;
        app.chat_search_calendar = true;
        let ctx = egui::Context::default();
        escape(&mut app, &ctx);
        assert!(!app.chat_search_calendar);
        assert!(app.actions.is_empty(), "the pane stays for now");
        escape(&mut app, &ctx);
        assert!(matches!(app.actions.as_slice(), [Action::CloseChatSearch]));
        app.actions.clear();
        // Settings hide the pane, so Escape leaves them instead.
        app.page = Page::Settings;
        escape(&mut app, &ctx);
        assert!(matches!(
            app.actions.as_slice(),
            [Action::Open(Page::Chats)]
        ));
    }

    #[test]
    fn escape_returns_from_the_media_view_then_closes_the_info_panel() {
        use crate::model::{InfoPanel, InfoView, MediaTab};
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.page = Page::Chats;
        app.open_chat = Some("fixture".into());
        app.info = Some(InfoPanel {
            chat: "fixture".into(),
            view: InfoView::Media(MediaTab::Links),
        });
        let ctx = egui::Context::default();
        escape(&mut app, &ctx);
        assert!(matches!(
            app.actions.as_slice(),
            [Action::ShowInfo(InfoView::Overview)]
        ));
        app.actions.clear();
        app.info.as_mut().unwrap().view = InfoView::Overview;
        // A group rename in the panel is cancelled before the panel closes.
        app.group_name_edit = Some("Rust".into());
        escape(&mut app, &ctx);
        assert!(matches!(app.actions.as_slice(), [Action::CloseGroupName]));
        app.actions.clear();
        app.group_name_edit = None;
        escape(&mut app, &ctx);
        assert!(matches!(app.actions.as_slice(), [Action::CloseInfo]));
    }

    /// What the composer is in the middle of goes first: a reply, an edit,
    /// staged files and suggestions are cancelled before the panel closes,
    /// and a focused composer with nothing to cancel still closes the panel
    /// rather than the chat.
    #[test]
    fn escape_cancels_composer_state_before_closing_the_info_panel() {
        use crate::model::{InfoPanel, InfoView};
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.page = Page::Chats;
        app.open_chat = Some("fixture".into());
        app.info = Some(InfoPanel {
            chat: "fixture".into(),
            view: InfoView::Overview,
        });
        let ctx = egui::Context::default();
        app.reply_to = Some("reply".into());
        escape(&mut app, &ctx);
        assert!(matches!(app.actions.as_slice(), [Action::CancelReply]));
        assert!(app.info.is_some(), "the panel stays");
        app.actions.clear();
        app.reply_to = None;
        app.emoji_start = Some(0);
        escape(&mut app, &ctx);
        assert!(matches!(
            app.actions.as_slice(),
            [Action::CloseEmojiSuggestions]
        ));
        app.actions.clear();
        app.emoji_start = None;
        app.stage(crate::app::Pending::File("unsent.png".into()));
        escape(&mut app, &ctx);
        assert!(matches!(app.actions.as_slice(), [Action::ClearPending]));
        app.actions.clear();
        app.pending.clear();
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("composer-text")));
        escape(&mut app, &ctx);
        assert!(
            matches!(app.actions.as_slice(), [Action::CloseInfo]),
            "the panel, not the chat: {:?}",
            app.actions
        );
    }

    #[test]
    fn escape_cancels_a_contact_rename_and_keeps_the_info_panel() {
        use crate::model::{InfoPanel, InfoView};
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.page = Page::Chats;
        app.open_chat = Some("fixture".into());
        app.info = Some(InfoPanel {
            chat: "fixture".into(),
            view: InfoView::Overview,
        });
        app.contact_edit = Some(("Ada".into(), String::new()));
        let ctx = egui::Context::default();
        escape(&mut app, &ctx);
        assert!(matches!(app.actions.as_slice(), [Action::CloseContactEdit]));
    }

    #[test]
    fn focus_input_shortcut_only_targets_an_available_composer() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        let ctx = egui::Context::default();
        for (page, chat, dialog, expected) in [
            (Page::Chats, Some("fixture"), None, true),
            (Page::Chats, None, None, false),
            (Page::Settings, Some("fixture"), None, false),
            (Page::Chats, Some("fixture"), Some(Dialog::Shortcuts), false),
        ] {
            app.page = page;
            app.open_chat = chat.map(str::to_owned);
            app.dialog = dialog;
            app.actions.clear();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: Key::L,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Modifiers::COMMAND,
                    }],
                    ..Default::default()
                },
                |ui| handle(&mut app, ui.ctx()),
            );
            output.textures_delta.clear();
            assert_eq!(
                matches!(app.actions.as_slice(), [Action::FocusComposer]),
                expected
            );
        }
    }

    #[test]
    fn escape_returns_from_search_and_reply_before_closing_the_chat() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.page = Page::Chats;
        app.open_chat = Some("fixture".into());
        let ctx = egui::Context::default();
        app.reply_to = Some("reply".into());
        escape(&mut app, &ctx);
        assert!(matches!(app.actions.as_slice(), [Action::CancelReply]));
        app.actions.clear();
        app.reply_to = None;
        app.search = "Ada".into();
        escape(&mut app, &ctx);
        assert!(
            matches!(app.actions.as_slice(), [Action::Search(text), Action::FocusComposer] if text.is_empty())
        );
        app.actions.clear();
        app.search.clear();
        escape(&mut app, &ctx);
        assert!(matches!(app.actions.as_slice(), [Action::CloseChat]));
    }

    #[test]
    fn escape_closes_the_update_before_touching_an_unfinished_message() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.show_update = true;
        app.page = Page::Settings;
        app.reply_to = Some("reply-fixture".into());
        app.stage(crate::app::Pending::File("unsent.png".into()));
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| handle(&mut app, ui.ctx()),
        );
        output.textures_delta.clear();
        assert!(matches!(app.actions.as_slice(), [Action::CloseUpdate]));
    }

    fn app_with_chats(count: usize) -> (tempfile::TempDir, App, Vec<String>) {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        let ids: Vec<String> = (0..count)
            .map(|index| format!("49170000{index:04}@s.whatsapp.net"))
            .collect();
        for (index, id) in ids.iter().enumerate() {
            let mut chat = Chat::new(id.clone(), format!("Chat {index:02}"));
            chat.last_activity = 100 - index as i64;
            app.chats.push(chat);
        }
        app.page = Page::Chats;
        (root, app, ids)
    }

    /// Runs `handle` on one key press and reports whether the press was left
    /// for the views, as every press that is not a shortcut must be.
    fn press(app: &mut App, ctx: &egui::Context, key: Key, modifiers: Modifiers) -> bool {
        let mut survived = false;
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            },
            |ui| {
                handle(app, ui.ctx());
                survived = ui.ctx().input(|input| {
                    input.events.iter().any(|event| {
                        matches!(
                            event,
                            egui::Event::Key {
                                key: found,
                                pressed: true,
                                ..
                            } if *found == key
                        )
                    })
                });
            },
        );
        output.textures_delta.clear();
        survived
    }

    /// The modifiers a real Ctrl+Shift press carries on Linux and Windows,
    /// and a real Cmd+Shift press on macOS.
    fn ctrl_shift() -> [Modifiers; 3] {
        [
            Modifiers::COMMAND | Modifiers::SHIFT,
            Modifiers {
                ctrl: true,
                command: true,
                shift: true,
                ..Modifiers::NONE
            },
            Modifiers {
                mac_cmd: true,
                command: true,
                shift: true,
                ..Modifiers::NONE
            },
        ]
    }

    #[test]
    fn bracket_shortcuts_step_to_the_previous_and_next_chat() {
        let (_root, mut app, ids) = app_with_chats(3);
        app.open_chat = Some(ids[1].clone());
        let ctx = egui::Context::default();
        // With Shift held, US-style layouts report the brackets as braces.
        for modifiers in ctrl_shift() {
            for (key, expected) in [
                (Key::CloseBracket, &ids[2]),
                (Key::CloseCurlyBracket, &ids[2]),
                (Key::OpenBracket, &ids[0]),
                (Key::OpenCurlyBracket, &ids[0]),
            ] {
                app.actions.clear();
                app.scroll_chat_into_view = None;
                let survived = press(&mut app, &ctx, key, modifiers);
                assert!(!survived, "{key:?} with {modifiers:?} reached the views");
                assert!(
                    app.actions.contains(&Action::OpenChat(expected.clone())),
                    "{key:?} with {modifiers:?} opens {expected}: {:?}",
                    app.actions
                );
                assert_eq!(app.scroll_chat_into_view.as_ref(), Some(expected));
            }
        }
    }

    #[test]
    fn only_control_alone_counts_as_the_macos_messages_key() {
        assert!(control_alone(Modifiers::CTRL));
        // Command+Control+2 stays Command+2's chat.
        assert!(!control_alone(Modifiers {
            ctrl: true,
            command: true,
            ..Modifiers::NONE
        }));
        assert!(!control_alone(Modifiers {
            ctrl: true,
            mac_cmd: true,
            command: true,
            ..Modifiers::NONE
        }));
        assert!(!control_alone(Modifiers::CTRL | Modifiers::SHIFT));
        assert!(!control_alone(Modifiers::NONE));
    }

    #[test]
    fn numbered_shortcuts_open_each_position_with_platform_command_modifiers() {
        let (_root, mut app, ids) = app_with_chats(9);
        let ctx = egui::Context::default();
        for modifiers in ctrl_shift().map(|modifiers| Modifiers {
            shift: false,
            ..modifiers
        }) {
            for (key, expected) in [
                Key::Num1,
                Key::Num2,
                Key::Num3,
                Key::Num4,
                Key::Num5,
                Key::Num6,
                Key::Num7,
                Key::Num8,
                Key::Num9,
            ]
            .into_iter()
            .zip(&ids)
            {
                app.actions.clear();
                app.scroll_chat_into_view = None;
                assert!(!press(&mut app, &ctx, key, modifiers));
                assert!(app.actions.contains(&Action::OpenChat(expected.clone())));
                assert_eq!(app.scroll_chat_into_view.as_ref(), Some(expected));
            }
        }
    }

    #[test]
    fn numbered_shortcuts_follow_pins_filters_search_and_archived_order() {
        let (_root, mut app, ids) = app_with_chats(4);
        let ctx = egui::Context::default();
        app.chats[3].pinned = true;
        app.chats[2].archived = true;
        app.chats[1].locked = true;
        assert!(!press(&mut app, &ctx, Key::Num1, Modifiers::COMMAND));
        assert!(app.actions.contains(&Action::OpenChat(ids[3].clone())));
        app.actions.clear();
        assert!(!press(&mut app, &ctx, Key::Num2, Modifiers::COMMAND));
        assert!(app.actions.contains(&Action::OpenChat(ids[0].clone())));

        app.chat_filter = crate::model::ChatFilter::Unread;
        app.chats[0].unread = 1;
        app.actions.clear();
        press(&mut app, &ctx, Key::Num1, Modifiers::COMMAND);
        assert!(app.actions.contains(&Action::OpenChat(ids[0].clone())));
        assert!(app.actions.contains(&Action::KeepUnread(ids[0].clone())));

        app.search = "Chat 02".into();
        app.search_selected = Some(ids[0].clone());
        app.actions.clear();
        press(&mut app, &ctx, Key::Num9, Modifiers::COMMAND);
        assert!(app.actions.is_empty());
        assert_eq!(app.search_selected.as_ref(), Some(&ids[0]));
        press(&mut app, &ctx, Key::Num1, Modifiers::COMMAND);
        assert_eq!(app.actions, [Action::OpenChat(ids[2].clone())]);
        assert!(app.search_selected.is_none());

        app.search.clear();
        app.show_archived = true;
        app.actions.clear();
        press(&mut app, &ctx, Key::Num1, Modifiers::COMMAND);
        assert_eq!(app.actions, [Action::OpenChat(ids[2].clone())]);
    }

    #[test]
    fn numbered_shortcuts_do_not_wrap_missing_positions_or_change_zero_zoom() {
        let (_root, mut app, ids) = app_with_chats(2);
        let ctx = egui::Context::default();
        app.open_chat = Some(ids[0].clone());
        for key in [Key::Num3, Key::Num9] {
            assert!(!press(&mut app, &ctx, key, Modifiers::COMMAND));
            assert!(app.actions.is_empty());
            assert!(app.scroll_chat_into_view.is_none());
        }
        app.chats.clear();
        press(&mut app, &ctx, Key::Num1, Modifiers::COMMAND);
        assert!(app.actions.is_empty());
        press(&mut app, &ctx, Key::Num0, Modifiers::COMMAND);
        assert_eq!(app.actions, [Action::ResetZoom]);
    }

    #[test]
    fn numbered_shortcuts_leave_plain_and_modified_numbers_alone() {
        let (_root, mut app, _ids) = app_with_chats(2);
        let ctx = egui::Context::default();
        for modifiers in [
            Modifiers::NONE,
            Modifiers::SHIFT,
            Modifiers::ALT,
            Modifiers::COMMAND | Modifiers::ALT,
        ] {
            assert!(press(&mut app, &ctx, Key::Num1, modifiers));
            assert!(app.actions.is_empty());
        }
    }

    /// Ctrl+Shift+1 and Ctrl+Shift+2 (Cmd on macOS) lead to the rail's
    /// Chats and Calls by where the digit sits, whatever symbol the layout
    /// puts on it with Shift held; Ctrl+1 and Ctrl+2 still open chats.
    #[test]
    fn shifted_digits_lead_to_the_chats_and_the_calls() {
        let (_root, mut app, ids) = app_with_chats(2);
        app.link = crate::backend::LinkStatus::Connected;
        let ctx = egui::Context::default();
        let shifted = |app: &mut App, key: Key, physical: Key| {
            app.actions.clear();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key,
                        physical_key: Some(physical),
                        pressed: true,
                        repeat: false,
                        modifiers: Modifiers::COMMAND | Modifiers::SHIFT,
                    }],
                    ..Default::default()
                },
                |ui| handle(app, ui.ctx()),
            );
            output.textures_delta.clear();
            std::mem::take(&mut app.actions)
        };
        // US layouts report Shift+2 as "@", which egui has no key for, and
        // Shift+1 as "!".
        assert_eq!(
            shifted(&mut app, Key::Num2, Key::Num2),
            [Action::Open(Page::Calls)]
        );
        app.page = Page::Calls;
        assert!(
            shifted(&mut app, Key::Num2, Key::Num2).is_empty(),
            "Calls is not a toggle"
        );
        assert_eq!(
            shifted(&mut app, Key::Exclamationmark, Key::Num1),
            [Action::Open(Page::Chats)]
        );
        app.page = Page::Chats;
        app.show_starred = true;
        assert_eq!(
            shifted(&mut app, Key::Exclamationmark, Key::Num1),
            [Action::ToggleStarred]
        );
        app.show_starred = false;
        assert!(!press(&mut app, &ctx, Key::Num2, Modifiers::COMMAND));
        assert!(app.actions.contains(&Action::OpenChat(ids[1].clone())));
    }

    #[test]
    fn numbered_shortcuts_leave_settings_and_overlays_alone() {
        let (_root, mut app, _ids) = app_with_chats(2);
        let ctx = egui::Context::default();
        app.page = Page::Settings;
        assert!(press(&mut app, &ctx, Key::Num1, Modifiers::COMMAND));
        app.page = Page::Chats;
        app.dialog = Some(Dialog::Shortcuts);
        assert!(press(&mut app, &ctx, Key::Num1, Modifiers::COMMAND));
        app.dialog = None;
        app.show_update = true;
        assert!(press(&mut app, &ctx, Key::Num1, Modifiers::COMMAND));
        app.show_update = false;
        app.reaction_target = Some(("chat-fixture".into(), "message-fixture".into()));
        assert!(press(&mut app, &ctx, Key::Num1, Modifiers::COMMAND));
        assert!(app.actions.is_empty());
    }

    /// One number on screen and a second linked beside it.
    fn app_with_two_numbers() -> (tempfile::TempDir, App) {
        let root = tempfile::tempdir().unwrap();
        let dirs = crate::paths::AppDirs::under(root.path());
        let mut app = App::headless(dirs.clone(), crate::settings::Settings::default()).0;
        let second = crate::account::Account::detached(
            &dirs,
            crate::model::AccountId::parse("2").unwrap(),
            crate::settings::AccountSettings::default(),
        )
        .unwrap()
        .0;
        app.accounts.push(second);
        app.active = 0;
        app.page = Page::Chats;
        (root, app)
    }

    #[test]
    fn ctrl_shift_a_steps_through_linked_numbers_and_wraps() {
        let (_root, mut app) = app_with_two_numbers();
        let ids: Vec<_> = app
            .accounts
            .iter()
            .map(|account| account.id.clone())
            .collect();
        let ctx = egui::Context::default();
        for modifiers in ctrl_shift() {
            for (active, expected) in [(0, &ids[1]), (1, &ids[0])] {
                app.active = active;
                app.actions.clear();
                assert!(!press(&mut app, &ctx, Key::A, modifiers));
                assert_eq!(app.actions, [Action::SwitchAccount(expected.clone())]);
            }
        }
    }

    #[test]
    fn ctrl_shift_a_needs_a_second_number_that_is_not_being_added() {
        let (_root, mut single, _ids) = app_with_chats(1);
        let ctx = egui::Context::default();
        let ctrl_shift_a = Modifiers::COMMAND | Modifiers::SHIFT;
        assert!(press(&mut single, &ctx, Key::A, ctrl_shift_a));
        assert!(
            single.actions.is_empty(),
            "one number has nothing to switch to"
        );
        let (_root, mut app) = app_with_two_numbers();
        app.adding_account = true;
        assert!(press(&mut app, &ctx, Key::A, ctrl_shift_a));
        assert!(
            app.actions.is_empty(),
            "a number being added keeps its Cancel"
        );
    }

    #[test]
    fn ctrl_shift_a_leaves_dialogs_and_other_combinations_alone() {
        let (_root, mut app) = app_with_two_numbers();
        let ctx = egui::Context::default();
        app.dialog = Some(Dialog::Shortcuts);
        assert!(press(
            &mut app,
            &ctx,
            Key::A,
            Modifiers::COMMAND | Modifiers::SHIFT
        ));
        app.dialog = None;
        for modifiers in [Modifiers::NONE, Modifiers::SHIFT, Modifiers::COMMAND] {
            assert!(press(&mut app, &ctx, Key::A, modifiers));
        }
        assert!(
            !app.actions
                .iter()
                .any(|action| matches!(action, Action::SwitchAccount(_))),
            "{:?}",
            app.actions
        );
    }

    #[test]
    fn ctrl_e_archives_and_unarchives_the_open_chat() {
        let (_root, mut app, ids) = app_with_chats(2);
        app.open_chat = Some(ids[1].clone());
        let ctx = egui::Context::default();
        for modifiers in ctrl_shift().map(|modifiers| Modifiers {
            shift: false,
            ..modifiers
        }) {
            app.chats[1].archived = false;
            app.actions.clear();
            assert!(!press(&mut app, &ctx, Key::E, modifiers));
            assert_eq!(app.actions, [Action::SetArchived(ids[1].clone(), true)]);
            app.chats[1].archived = true;
            app.actions.clear();
            assert!(!press(&mut app, &ctx, Key::E, modifiers));
            assert_eq!(app.actions, [Action::SetArchived(ids[1].clone(), false)]);
        }
    }

    #[test]
    fn ctrl_e_needs_an_open_chat_and_no_overlay() {
        let (_root, mut app, ids) = app_with_chats(1);
        let ctx = egui::Context::default();
        assert!(press(&mut app, &ctx, Key::E, Modifiers::COMMAND));
        assert!(app.actions.is_empty(), "no chat is open");
        app.open_chat = Some(ids[0].clone());
        app.dialog = Some(Dialog::Shortcuts);
        assert!(press(&mut app, &ctx, Key::E, Modifiers::COMMAND));
        app.dialog = None;
        app.page = Page::Settings;
        assert!(press(&mut app, &ctx, Key::E, Modifiers::COMMAND));
        app.page = Page::Chats;
        assert!(press(&mut app, &ctx, Key::E, Modifiers::NONE));
        assert!(app.actions.is_empty());
    }

    #[test]
    fn a_bracket_without_ctrl_and_shift_together_stays_typed() {
        let (_root, mut app, ids) = app_with_chats(3);
        app.open_chat = Some(ids[1].clone());
        let ctx = egui::Context::default();
        let typed = [
            Modifiers::NONE,
            Modifiers::SHIFT,
            Modifiers::COMMAND,
            Modifiers {
                ctrl: true,
                command: true,
                ..Modifiers::NONE
            },
            Modifiers {
                mac_cmd: true,
                command: true,
                ..Modifiers::NONE
            },
            Modifiers::ALT,
            Modifiers::ALT | Modifiers::SHIFT,
        ];
        for modifiers in typed {
            for key in [
                Key::OpenBracket,
                Key::OpenCurlyBracket,
                Key::CloseBracket,
                Key::CloseCurlyBracket,
            ] {
                app.actions.clear();
                let survived = press(&mut app, &ctx, key, modifiers);
                assert!(survived, "{key:?} with {modifiers:?} was consumed");
                assert!(
                    !app.actions
                        .iter()
                        .any(|action| matches!(action, Action::OpenChat(_))),
                    "{key:?} with {modifiers:?} switched chats"
                );
            }
        }
    }

    #[test]
    fn page_keys_scroll_the_open_chat_and_are_consumed() {
        let (_root, mut app, ids) = app_with_chats(1);
        app.open_chat = Some(ids[0].clone());
        let ctx = egui::Context::default();
        for (key, expected) in [
            (Key::PageUp, Action::ScrollPage(Scroll::PageUp)),
            (Key::PageDown, Action::ScrollPage(Scroll::PageDown)),
            (Key::Home, Action::ScrollPage(Scroll::Top)),
            (Key::End, Action::ScrollPage(Scroll::Bottom)),
        ] {
            app.actions.clear();
            let survived = press(&mut app, &ctx, key, Modifiers::NONE);
            assert!(!survived, "{key:?} reached the views");
            assert_eq!(
                app.actions,
                [expected],
                "{key:?} did not push the expected action"
            );
        }
    }

    /// The focused message list takes the arrows, Home, End and the page
    /// keys for its cursor; unfocused, the page keys still scroll.
    #[test]
    fn a_focused_message_list_takes_the_navigation_keys() {
        use crate::ui::list_nav::Step;
        let (_root, mut app, ids) = app_with_chats(1);
        app.open_chat = Some(ids[0].clone());
        let ctx = egui::Context::default();
        let list = crate::ui::conversation::messages_list_id(&ids[0]);
        for (key, expected) in [
            (Key::ArrowUp, Step::Previous),
            (Key::ArrowDown, Step::Next),
            (Key::Home, Step::First),
            (Key::End, Step::Last),
            (Key::PageUp, Step::PageUp(8)),
            (Key::PageDown, Step::PageDown(8)),
        ] {
            app.actions.clear();
            ctx.memory_mut(|memory| memory.request_focus(list));
            let survived = press(&mut app, &ctx, key, Modifiers::NONE);
            assert!(!survived, "{key:?} reached the views");
            assert_eq!(app.actions, [Action::MessageCursor(expected)], "{key:?}");
        }
        // Shift+Home and the like are not list keys.
        app.actions.clear();
        ctx.memory_mut(|memory| memory.request_focus(list));
        assert!(press(&mut app, &ctx, Key::Home, Modifiers::SHIFT));
        assert!(app.actions.is_empty(), "{:?}", app.actions);
        // Unfocused, PgUp scrolls as before and the arrows are left alone.
        ctx.memory_mut(|memory| memory.surrender_focus(list));
        app.actions.clear();
        assert!(!press(&mut app, &ctx, Key::PageUp, Modifiers::NONE));
        assert_eq!(app.actions, [Action::ScrollPage(Scroll::PageUp)]);
        app.actions.clear();
        assert!(press(&mut app, &ctx, Key::ArrowDown, Modifiers::NONE));
        assert!(app.actions.is_empty());
    }

    #[test]
    fn the_messages_shortcut_focuses_the_list() {
        let (_root, mut app, ids) = app_with_chats(1);
        app.open_chat = Some(ids[0].clone());
        let ctx = egui::Context::default();
        let modifiers = if cfg!(target_os = "macos") {
            Modifiers::CTRL
        } else {
            Modifiers::ALT
        };
        assert!(!press(&mut app, &ctx, Key::Num2, modifiers));
        assert_eq!(app.actions, [Action::FocusMessages]);
        // The call screen covers the list, so the shortcut leaves it be.
        app.actions.clear();
        app.call = Some(crate::calls::CallUpdate {
            generation: 1,
            chat: ids[0].clone(),
            direction: crate::model::CallDirection::Outgoing,
            video: false,
            phase: crate::calls::CallPhase::Active,
            started: Some(std::time::Instant::now()),
            muted: false,
            camera_on: false,
            remote_video: false,
            video_requested: false,
            outcome: None,
            peer_audio: None,
            lost_devices: Vec::new(),
            microphone: None,
            speaker: None,
            camera: None,
            group: None,
        });
        press(&mut app, &ctx, Key::Num2, modifiers);
        assert!(app.actions.is_empty(), "{:?}", app.actions);
        // Stepped away from the call, the chats are back, and so is the key.
        app.call_surface_hidden = true;
        assert!(!press(&mut app, &ctx, Key::Num2, modifiers));
        assert_eq!(app.actions, [Action::FocusMessages]);
    }

    /// The list's shortcut and the rail's Ctrl+Shift+2 (Cmd on macOS) never
    /// take each other's keys, and Shift does not turn one into the other.
    #[test]
    fn the_messages_shortcut_and_the_rail_keys_stay_apart() {
        let (_root, mut app, ids) = app_with_chats(2);
        app.link = crate::backend::LinkStatus::Connected;
        app.open_chat = Some(ids[0].clone());
        let ctx = egui::Context::default();
        let messages = if cfg!(target_os = "macos") {
            Modifiers::CTRL
        } else {
            Modifiers::ALT
        };
        press(&mut app, &ctx, Key::Num2, messages);
        assert_eq!(app.actions, [Action::FocusMessages]);
        app.actions.clear();
        press(
            &mut app,
            &ctx,
            Key::Num2,
            Modifiers::COMMAND | Modifiers::SHIFT,
        );
        assert_eq!(app.actions, [Action::Open(Page::Calls)]);
        app.actions.clear();
        app.page = Page::Chats;
        press(&mut app, &ctx, Key::Num2, messages | Modifiers::SHIFT);
        assert!(
            !app.actions.contains(&Action::Open(Page::Calls)),
            "{:?}",
            app.actions
        );
        assert!(
            !app.actions.contains(&Action::FocusMessages),
            "{:?}",
            app.actions
        );
    }

    #[test]
    fn page_keys_leave_shift_and_ctrl_combinations_alone() {
        let (_root, mut app, ids) = app_with_chats(1);
        app.open_chat = Some(ids[0].clone());
        let ctx = egui::Context::default();
        for (key, modifiers) in [
            (Key::Home, Modifiers::SHIFT),
            (Key::PageUp, Modifiers::SHIFT),
            (Key::Home, Modifiers::COMMAND),
        ] {
            app.actions.clear();
            let survived = press(&mut app, &ctx, key, modifiers);
            assert!(survived, "{key:?} with {modifiers:?} was consumed");
            assert!(app.actions.is_empty(), "{key:?} with {modifiers:?} acted");
        }
        // Ctrl+End (Cmd+End on macOS) still jumps to the newest message.
        app.actions.clear();
        let survived = press(&mut app, &ctx, Key::End, Modifiers::COMMAND);
        assert!(!survived);
        assert_eq!(app.actions, [Action::ScrollToBottom]);
    }

    #[test]
    fn ctrl_end_and_home_end_leave_a_field_with_text_alone() {
        // No field focused, or an empty composer: the keys scroll the chat.
        assert!(jump_keys_reach_chat(false, false, false));
        assert!(jump_keys_reach_chat(true, true, true));
        // A composer with text, or any other field (search, dialogs): the
        // keys move the text cursor (#456).
        assert!(!jump_keys_reach_chat(true, true, false));
        assert!(!jump_keys_reach_chat(true, false, true));
        assert!(!jump_keys_reach_chat(true, false, false));
    }

    #[test]
    fn page_keys_do_nothing_without_an_open_chat_a_dialog_or_off_the_chats_page() {
        let (_root, mut app, ids) = app_with_chats(1);
        let ctx = egui::Context::default();
        let keys = [Key::PageUp, Key::PageDown, Key::Home, Key::End];
        // No chat open.
        for key in keys {
            app.actions.clear();
            assert!(
                press(&mut app, &ctx, key, Modifiers::NONE),
                "{key:?} survived with no chat open"
            );
            assert!(app.actions.is_empty());
        }
        app.open_chat = Some(ids[0].clone());
        // A dialog is open.
        app.dialog = Some(Dialog::Shortcuts);
        for key in keys {
            app.actions.clear();
            assert!(press(&mut app, &ctx, key, Modifiers::NONE));
            assert!(app.actions.is_empty());
        }
        app.dialog = None;
        // Off the Chats page.
        app.page = Page::Settings;
        for key in keys {
            app.actions.clear();
            assert!(press(&mut app, &ctx, key, Modifiers::NONE));
            assert!(app.actions.is_empty());
        }
    }
}
