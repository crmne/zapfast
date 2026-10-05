//! The window's side of a call: which account it is on, what the call
//! surface shows, the desktop notification, and the actions the surface
//! pushes, routed to the backend of the account the call belongs to.
//!
//! The call's state comes from the worker that owns it (`Event::Call`); the
//! window never decides what a call is doing. One call at a time across
//! every account, since there is one microphone: an offer to another account
//! while a call is up is declined there.

use std::time::{Duration, Instant};

use super::{App, CALL_FAREWELL, notification_sound};
use crate::backend::{Backend, Command};
use crate::calls::{CallPhase, CallUpdate};
use crate::model::{AccountId, Action, Chat};

/// How often an unanswered call rings again.
const RING_EVERY: Duration = Duration::from_secs(3);

impl App {
    /// Whether a call is up or ringing, on any account.
    pub fn call_live(&self) -> bool {
        self.call.as_ref().is_some_and(|call| call.phase.is_live())
    }

    /// Whether the full call screen is what the reader is looking at: a call
    /// the reader has not stepped away from. A ringing call is answered, not
    /// stepped away from, and a finished one is only saying how it ended.
    pub fn call_surface_open(&self) -> bool {
        self.call
            .as_ref()
            .is_some_and(|call| call.phase.is_live() && call.phase != CallPhase::Incoming)
            && !self.call_surface_hidden
    }

    /// Whether this chat of the account on screen has the live call, which is
    /// what the chat header's call button reflects. A finished call keeps its
    /// snapshot for the farewell, but that is no call to hang up.
    pub fn call_live_here(&self, id: &str) -> bool {
        self.call_account.as_ref() == Some(&self.account().id)
            && self
                .call
                .as_ref()
                .is_some_and(|call| call.chat == id && call.phase.is_live())
    }

    /// Runs `f` with the call's account as the active one, so names and
    /// pictures come from the account the call is on.
    fn with_call_account<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let index = self
            .call_account
            .as_ref()
            .and_then(|id| self.accounts.iter().position(|account| &account.id == id))
            .unwrap_or(self.active);
        let real = std::mem::replace(&mut self.active, index);
        let result = f(self);
        self.active = real;
        result
    }

    /// Whether the call belongs to a chat the lock is hiding right now: a
    /// locked chat says nothing until its folder is open, and a folder is only
    /// ever open for the account on screen.
    pub(crate) fn call_is_private(&self) -> bool {
        let (Some(call), Some(account)) = (&self.call, &self.call_account) else {
            return false;
        };
        let Some(owner) = self.accounts.iter().find(|owner| &owner.id == account) else {
            return false;
        };
        let locked = owner
            .chats
            .iter()
            .any(|chat| chat.id == call.chat && chat.locked);
        let on_screen = !self.events_hidden && owner.id == self.account().id;
        locked && !(on_screen && self.locked_folder_open())
    }

    /// The name the call shows for the other side: the chat's own title, or
    /// "Locked chat" while the lock hides it.
    pub fn call_name(&mut self) -> String {
        let Some(chat) = self.call.as_ref().map(|call| call.chat.clone()) else {
            return String::new();
        };
        if self.call_is_private() {
            return crate::i18n::gettext(self.locale, "Locked chat").into_owned();
        }
        self.with_call_account(|app| app.display_name(&chat))
    }

    /// The picture the call shows for the other side: none while the chat is
    /// locked, because a photo is the same disclosure the name is.
    pub fn call_avatar(&mut self) -> Option<std::path::PathBuf> {
        let chat = self.call.as_ref()?.chat.clone();
        if self.call_is_private() {
            return None;
        }
        self.with_call_account(|app| app.avatar(&chat))
    }

    /// The backend of the account the call is on.
    fn call_backend(&self) -> Option<&Backend> {
        let account = self.call_account.as_ref()?;
        self.accounts
            .iter()
            .find(|owner| &owner.id == account)
            .map(|owner| &owner.backend)
    }

    /// Sends a command to the call's own account, whichever is on screen.
    pub(super) fn send_call_command(&self, command: Command) {
        if let Some(backend) = self.call_backend() {
            backend.send(command);
        }
    }

    /// Takes the window in or out of full screen for the call surface. Only
    /// the window moves; the call is not touched.
    fn set_call_fullscreen(&mut self, ctx: &egui::Context, fullscreen: bool) {
        if self.call_fullscreen != fullscreen {
            self.call_fullscreen = fullscreen;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(fullscreen));
        }
    }

    /// Puts a live call back behind the bar when its chat is locked and the
    /// folder closes: otherwise a call opened while the folder was open would
    /// keep painting the whole window after it closed.
    pub(super) fn hide_private_call(&mut self) {
        if self.call_is_private() {
            self.call_surface_hidden = true;
            // No picture lingers for the strip to lift.
            self.drop_call_frames();
        }
    }

    /// Forgets a video call's pictures, so nothing frozen is drawn as live.
    fn drop_call_frames(&mut self) {
        self.call_local_frame = None;
        self.call_remote_frame = None;
        self.call_local_texture = None;
        self.call_remote_texture = None;
    }

    /// A video call's newest pictures, from the account the call is on.
    pub(super) fn handle_call_video(
        &mut self,
        local: Option<std::sync::Arc<egui::ColorImage>>,
        remote: Option<std::sync::Arc<egui::ColorImage>>,
    ) {
        if self.call_account.as_ref() != Some(&self.account().id) || !self.call_live() {
            return;
        }
        if let Some(image) = local {
            self.call_local_frame = Some(image);
        }
        if let Some(image) = remote {
            self.call_remote_frame = Some(image);
        }
    }

    /// Applies one call state from the backend of the account being handled.
    ///
    /// A finished call keeps its surface for a moment so the outcome can be
    /// read, then goes; the backend has already released the media.
    pub(super) fn handle_call_update(&mut self, update: CallUpdate) {
        let account = self.account().id.clone();
        let other_account = self
            .call_account
            .as_ref()
            .is_some_and(|current| current != &account);
        if other_account && self.call_live() {
            // One call at a time: an offer to another account while a call is
            // up is declined there, and its later updates are not this call's.
            if update.phase == CallPhase::Incoming {
                self.backend.send(Command::DeclineCall);
            }
            return;
        }
        if !other_account
            && self
                .call
                .as_ref()
                .is_some_and(|current| current.generation > update.generation)
        {
            return;
        }
        // A call we announced that no longer rings was picked up or given up:
        // its notification goes, and only its own.
        if update.phase != CallPhase::Incoming && self.call_notified == Some(update.generation) {
            self.call_notified = None;
            self.notifications.clear_call(&account, &update.chat);
        }
        let ringing = update.phase == CallPhase::Incoming
            && self
                .call
                .as_ref()
                .is_none_or(|current| current.generation != update.generation);
        let finished = !update.phase.is_live();
        let was_live = self.call_live() && !other_account;
        // A peer who stopped sending video leaves no picture: the last frame
        // is not a live one, so the surface goes back to their photo.
        if !update.remote_video || other_account {
            self.call_remote_frame = None;
        }
        if !update.camera_on || other_account {
            self.call_local_frame = None;
        }
        self.call = Some(update);
        self.call_account = Some(account);
        if finished {
            if !was_live && self.call_surface_until.is_none() {
                // Nothing was drawn for this call, so there is nothing to
                // take down or say farewell to.
                self.call = None;
                self.call_account = None;
                return;
            }
            self.call_surface_until = Some(Instant::now() + CALL_FAREWELL);
            self.drop_call_frames();
            // How a call ended is the reason the surface lingers, but a locked
            // chat still says nothing: its farewell stays behind the bar.
            self.call_surface_hidden = self.call_is_private();
        } else {
            self.call_surface_until = None;
            // A call that has just begun takes the screen, unless its chat is
            // locked: then it waits in the bar under the folder's name.
            if !was_live {
                self.call_surface_hidden = self.call_is_private();
                self.call_rang = None;
            }
        }
        self.request_call_repaint();
        if ringing {
            self.notify_incoming_call();
        }
    }

    fn request_call_repaint(&self) {
        self.waker.wake();
    }

    /// Tells the desktop a call is waiting: the window may be in the tray or
    /// behind another program. The click brings the window up, where the
    /// call surface waits for Accept or Decline.
    fn notify_incoming_call(&mut self) {
        let Some(call) = self.call.clone() else {
            return;
        };
        if !self.account().settings.notifications {
            return;
        }
        let now = crate::util::now();
        // The chat's own rules: a call in a chat that is muted, archived or
        // locked stays as quiet as a message in it. A caller without a chat
        // row is a stranger, and rings with the ordinary sound.
        let chat_sound = match self.chat(&call.chat) {
            Some(chat) if !call_notification_eligible(chat, now) => return,
            Some(chat) => chat.notification_sound.clone(),
            None => None,
        };
        let (title, body, picture) = if self.app_lock.is_locked() {
            // Behind the app lock the desktop learns that a call came, and
            // nothing about who is calling.
            (
                "ZapFast".to_owned(),
                crate::i18n::gettext(self.locale, "Incoming call").into_owned(),
                None,
            )
        } else {
            (
                self.call_name(),
                if call.video {
                    crate::i18n::gettext(self.locale, "Incoming video call").into_owned()
                } else {
                    crate::i18n::gettext(self.locale, "Incoming voice call").into_owned()
                },
                self.call_avatar(),
            )
        };
        let sound = notification_sound(&self.settings, chat_sound, false, false);
        // The ring repeats the notification's sound; the desktop's own sound
        // only plays with a notification, so a call rings with the alert.
        self.call_ring_sound = Some(match &sound {
            crate::settings::NotificationSound::System => crate::settings::NotificationSound::Alert,
            other => other.clone(),
        });
        let waker = self.waker.clone();
        self.call_notified = Some(call.generation);
        self.wants_attention = true;
        self.notifications.show(
            title,
            body,
            picture,
            sound,
            crate::notify::NotificationTarget {
                account: self.account().id.clone(),
                chat: call.chat.clone(),
                message: None,
            },
            std::sync::Arc::clone(&self.notification_opens),
            move || waker.wake(),
            // A ringing call always notifies; the limit is for repeated messages.
            false,
        );
        self.call_rang = Some(Instant::now());
    }

    /// Per-frame upkeep: the farewell's end, full screen following the
    /// surface, the running timer, and the ring of a call nobody answered.
    pub(super) fn tick_call(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if self.call_surface_until.is_some_and(|until| now >= until) {
            self.call_surface_until = None;
            self.call = None;
            self.call_account = None;
            self.drop_call_frames();
        }
        // Full screen belongs to the call surface while the reader is looking
        // at it: however the surface was put aside, the window comes back.
        if self.call_fullscreen && !self.call_surface_open() {
            self.set_call_fullscreen(ctx, false);
        }
        let Some(call) = &self.call else {
            return;
        };
        if call.phase.is_live() {
            // The duration changes every second.
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        if call.phase == CallPhase::Incoming
            && let Some(rang) = self.call_rang
            && now.duration_since(rang) >= RING_EVERY
            && let Some(sound) = self.call_ring_sound.clone()
        {
            // A call rings until it is answered, declined or given up, the
            // way a phone does, with the sound its notification played.
            self.call_rang = Some(now);
            crate::notify::play_sound(sound);
        }
        if let Some(rang) = self.call_rang {
            ctx.request_repaint_after(RING_EVERY.saturating_sub(now.duration_since(rang)));
        }
        if call.phase != CallPhase::Incoming {
            self.call_rang = None;
            self.call_ring_sound = None;
        }
    }

    /// Applies the actions the call surface and the chat header push.
    /// Returns the action back when it is not a call action.
    pub(super) fn apply_call_action(
        &mut self,
        action: Action,
        ctx: &egui::Context,
    ) -> Option<Action> {
        match action {
            Action::StartCall(chat) | Action::StartVideoCall(chat) if self.call_live() => {
                let _ = chat;
                self.toast_error(crate::i18n::gettext(
                    self.locale,
                    "Hang up the current call first",
                ));
            }
            Action::StartCall(chat) => self.backend.send(Command::StartCall { chat, video: false }),
            Action::StartVideoCall(chat) => {
                self.backend.send(Command::StartCall { chat, video: true });
            }
            Action::SetCallCamera(on) => self.send_call_command(Command::SetCallCamera(on)),
            Action::SetCallCameraDevice(device) => {
                self.send_call_command(Command::SetCallCameraDevice(device));
            }
            Action::AnswerCall => self.send_call_command(Command::AnswerCall),
            Action::DeclineCall => self.send_call_command(Command::DeclineCall),
            Action::HangupCall => self.send_call_command(Command::HangupCall),
            Action::SetCallMuted(muted) => self.send_call_command(Command::SetCallMuted(muted)),
            Action::SetCallMicrophone(device) => {
                self.send_call_command(Command::SetCallMicrophone(device));
            }
            Action::SetCallSpeaker(device) => {
                self.send_call_command(Command::SetCallSpeaker(device));
            }
            Action::LeaveCallSurface => self.call_surface_hidden = true,
            // Returning to a locked chat's call cannot lift the lock on its
            // own: it asks for the code instead.
            Action::ReturnToCall if self.call.is_some() => {
                if !self.call_is_private() {
                    self.call_surface_hidden = false;
                } else if self.call_account.as_ref() == Some(&self.account().id) {
                    self.apply(Action::OpenLockedFolder, ctx);
                }
            }
            Action::ReturnToCall => {}
            Action::ToggleCallFullscreen => {
                if self.call.is_some() {
                    let fullscreen = !self.call_fullscreen;
                    self.set_call_fullscreen(ctx, fullscreen);
                }
            }
            other => return Some(other),
        }
        None
    }

    /// Whether the account being removed or unlinked has the call: its
    /// backend ends it, and the surface goes with it.
    pub(super) fn forget_call_of(&mut self, account: &AccountId) {
        if self.call_account.as_ref() == Some(account) {
            self.call = None;
            self.call_account = None;
            self.call_surface_until = None;
            self.call_rang = None;
            self.call_ring_sound = None;
            self.drop_call_frames();
        }
    }
}

/// Whether a call in this chat may raise a notification: the quiet rules a
/// message follows, without the unread one, since a call often comes to a
/// chat with nothing unread.
pub(super) fn call_notification_eligible(chat: &Chat, now: i64) -> bool {
    !chat.archived && !chat.muted(now) && !chat.locked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::Account;
    use crate::backend::LinkStatus;
    use crate::calls::CallOutcome;
    use crate::model::{CallDirection, Dialog};
    use crate::paths::AppDirs;
    use crate::settings::Settings;

    const ADA: &str = "15550001111@s.whatsapp.net";

    /// A window with one account whose commands a test can read.
    fn app(directory: &std::path::Path) -> (App, tokio::sync::mpsc::UnboundedReceiver<Command>) {
        let mut app = App::headless(AppDirs::under(directory), Settings::default()).0;
        let (backend, commands) = Backend::recording();
        app.backend = backend;
        app.chats.push(Chat::new(ADA.into(), "Ada".into()));
        (app, commands)
    }

    /// Adds a second account, not on screen, with its own command queue.
    fn second_account(app: &mut App) -> tokio::sync::mpsc::UnboundedReceiver<Command> {
        let mut second = Account::detached(
            &app.dirs,
            AccountId::parse("2").unwrap(),
            crate::settings::AccountSettings::default(),
        )
        .unwrap()
        .0;
        let (backend, commands) = Backend::recording();
        second.backend = backend;
        second.link = LinkStatus::Connected;
        second
            .chats
            .push(Chat::new(ADA.into(), "Ada at work".into()));
        app.accounts.push(second);
        commands
    }

    fn sent(commands: &mut tokio::sync::mpsc::UnboundedReceiver<Command>) -> Vec<Command> {
        std::iter::from_fn(|| commands.try_recv().ok()).collect()
    }

    fn update(generation: u64, phase: CallPhase) -> CallUpdate {
        CallUpdate {
            generation,
            chat: ADA.to_owned(),
            direction: CallDirection::Incoming,
            video: false,
            phase,
            started: (phase == CallPhase::Active).then(Instant::now),
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
        }
    }

    fn ended(generation: u64, outcome: CallOutcome) -> CallUpdate {
        let mut update = update(generation, CallPhase::Ended);
        update.outcome = Some(outcome);
        update
    }

    #[test]
    fn a_ringing_call_takes_the_screen_and_is_announced_once() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = app(directory.path());
        app.handle_call_update(update(1, CallPhase::Incoming));
        assert!(app.call_live());
        assert!(!app.call_surface_hidden, "a ringing call takes the screen");
        assert!(
            !app.call_surface_open(),
            "but it is answered, not stepped away from"
        );
        assert_eq!(app.notifications.shown.len(), 1, "the desktop is told");
        assert_eq!(app.notifications.shown[0].title, "Ada");
        // The same call again is not a second ring.
        app.handle_call_update(update(1, CallPhase::Incoming));
        assert_eq!(app.notifications.shown.len(), 1);
        // Answered, its notification is taken back and the call stays up.
        app.handle_call_update(update(1, CallPhase::Active));
        assert_eq!(app.call_notified, None);
        assert!(app.call_surface_open(), "the running call is the screen");
    }

    #[test]
    fn a_call_that_ends_says_how_then_goes() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = app(directory.path());
        let ctx = egui::Context::default();
        app.handle_call_update(update(1, CallPhase::Active));
        app.apply(Action::LeaveCallSurface, &ctx);
        app.handle_call_update(ended(1, CallOutcome::Answered));
        assert!(
            !app.call_surface_hidden,
            "a call that ended is never left behind the strip"
        );
        assert!(app.call.is_some(), "the farewell is on screen");
        assert!(!app.call_live_here(ADA), "but the header offers a new call");
        app.call_surface_until = Some(Instant::now() - Duration::from_secs(1));
        app.tick_call(&ctx);
        assert!(app.call.is_none(), "and then it goes");
        assert!(app.call_account.is_none());
    }

    #[test]
    fn a_finished_call_nobody_saw_is_not_drawn() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = app(directory.path());
        app.handle_call_update(ended(3, CallOutcome::Failed));
        assert!(app.call.is_none());
    }

    #[test]
    fn stepping_away_and_full_screen_leave_the_call_alone() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut commands) = app(directory.path());
        let ctx = egui::Context::default();
        app.handle_call_update(update(1, CallPhase::Active));
        app.apply(Action::LeaveCallSurface, &ctx);
        assert!(app.call_surface_hidden);
        assert!(!app.call_surface_open(), "the strip shows instead");
        app.apply(Action::ReturnToCall, &ctx);
        assert!(app.call_surface_open());
        app.apply(Action::ToggleCallFullscreen, &ctx);
        assert!(app.call_fullscreen);
        app.apply(Action::ToggleCallFullscreen, &ctx);
        assert!(!app.call_fullscreen);
        app.apply(Action::OpenChat(ADA.into()), &ctx);
        assert!(
            !sent(&mut commands)
                .iter()
                .any(|command| matches!(command, Command::HangupCall)),
            "none of that hangs up"
        );
        app.apply(Action::HangupCall, &ctx);
        assert!(
            sent(&mut commands)
                .iter()
                .any(|command| matches!(command, Command::HangupCall)),
            "the hang-up button does"
        );
    }

    #[test]
    fn a_call_that_ends_puts_the_window_back_out_of_full_screen() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = app(directory.path());
        let ctx = egui::Context::default();
        app.handle_call_update(update(1, CallPhase::Active));
        app.apply(Action::ToggleCallFullscreen, &ctx);
        app.handle_call_update(ended(1, CallOutcome::Answered));
        app.tick_call(&ctx);
        assert!(!app.call_fullscreen);
    }

    #[test]
    fn hiding_the_window_keeps_the_call_and_drops_full_screen() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut commands) = app(directory.path());
        app.handle_call_update(update(1, CallPhase::Active));
        app.call_fullscreen = true;
        app.window_gone();
        assert!(app.call_live(), "a hidden window is still a call");
        assert!(!app.call_fullscreen);
        assert!(sent(&mut commands).is_empty());
    }

    #[test]
    fn a_locked_chat_says_nothing_on_a_call_until_its_folder_opens() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = app(directory.path());
        let ctx = egui::Context::default();
        app.chats[0].locked = true;
        app.settings.set_chat_lock_code(Some("test-code"));
        app.handle_call_update(update(1, CallPhase::Incoming));
        assert!(
            app.call_surface_hidden,
            "the locked caller waits in the strip"
        );
        assert_eq!(app.call_name(), "Locked chat");
        assert!(app.call_avatar().is_none(), "no photo either");
        assert!(
            app.notifications.shown.is_empty(),
            "a locked chat's call is as quiet as its messages"
        );
        // Returning asks for the code instead of lifting the lock.
        app.apply(Action::ReturnToCall, &ctx);
        assert!(app.call_surface_hidden);
        assert_eq!(app.dialog, Some(Dialog::UnlockLockedChats));
        app.apply(Action::UnlockLockedFolder("test-code".into()), &ctx);
        assert_eq!(app.call_name(), "Ada");
        app.apply(Action::ReturnToCall, &ctx);
        assert!(!app.call_surface_hidden);
        // Closing the folder sends the call back behind the strip.
        app.close_locked_folder();
        assert!(app.call_surface_hidden);
    }

    #[test]
    fn a_call_in_a_muted_chat_is_not_announced() {
        let now = crate::util::now();
        let mut chat = Chat::new(ADA.into(), "Ada".into());
        assert!(call_notification_eligible(&chat, now));
        chat.muted_until = Some(now + 3600);
        assert!(!call_notification_eligible(&chat, now));
        chat.muted_until = None;
        chat.archived = true;
        assert!(!call_notification_eligible(&chat, now));
        chat.archived = false;
        chat.locked = true;
        assert!(!call_notification_eligible(&chat, now));
    }

    #[test]
    fn the_calls_actions_reach_the_account_it_is_on() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut first) = app(directory.path());
        let mut second = second_account(&mut app);
        let ctx = egui::Context::default();
        // The second account rings while the first is on screen.
        app.active = 1;
        app.handle_call_update(update(1, CallPhase::Incoming));
        app.active = 0;
        assert_eq!(app.call_account, AccountId::parse("2"));
        assert_eq!(app.call_name(), "Ada at work", "named by its own account");
        assert!(
            !app.call_live_here(ADA),
            "the account on screen has no call"
        );
        app.apply(Action::AnswerCall, &ctx);
        app.apply(Action::SetCallMuted(true), &ctx);
        let calls = |commands: Vec<Command>| -> Vec<Command> {
            commands
                .into_iter()
                .filter(|command| matches!(command, Command::AnswerCall | Command::SetCallMuted(_)))
                .collect()
        };
        assert!(
            calls(sent(&mut first)).is_empty(),
            "not the account on screen"
        );
        assert!(matches!(
            calls(sent(&mut second)).as_slice(),
            [Command::AnswerCall, Command::SetCallMuted(true)]
        ));
    }

    #[test]
    fn a_second_accounts_offer_is_declined_while_a_call_is_up() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut first) = app(directory.path());
        let mut second = second_account(&mut app);
        let ctx = egui::Context::default();
        app.handle_call_update(update(1, CallPhase::Active));
        app.active = 1;
        app.handle_call_update(update(2, CallPhase::Incoming));
        app.active = 0;
        assert!(matches!(
            sent(&mut second).as_slice(),
            [Command::DeclineCall]
        ));
        assert_eq!(app.call.as_ref().map(|call| call.generation), Some(1));
        assert_eq!(app.call_account, Some(AccountId::first()));
        // And no second call is started from the window either.
        app.apply(Action::StartCall(ADA.into()), &ctx);
        assert!(sent(&mut first).is_empty());
    }

    #[test]
    fn removing_the_calls_account_takes_its_surface_away() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = app(directory.path());
        let _second = second_account(&mut app);
        app.active = 1;
        app.handle_call_update(update(1, CallPhase::Active));
        app.active = 0;
        app.forget_call_of(&AccountId::parse("2").unwrap());
        assert!(app.call.is_none());
    }
}
