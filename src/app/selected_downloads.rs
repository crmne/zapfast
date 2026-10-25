#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::model::Media;

    fn batch(count: usize) -> (App, tokio::sync::mpsc::UnboundedReceiver<Command>) {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(AppDirs::under(root.path()), Settings::default()).0;
        let (backend, commands) = Backend::recording();
        app.backend = backend;
        app.link = LinkStatus::Connected;
        crate::demo::populate(&mut app);
        app.backend.set_offline(false);
        let template = app.conversations[app.open_chat.as_ref().unwrap()].messages[0].clone();
        app.open_chat = None;
        app.conversations.clear();
        let messages = (0..count)
            .map(|index| {
                let mut row = template.clone();
                row.id = index.to_string();
                row.chat = "synthetic".into();
                row.content = Content::Document {
                    media: Media {
                        mime: "application/pdf".into(),
                        size: 100,
                        width: None,
                        height: None,
                        album: None,
                        path: None,
                        state: MediaState::Idle,
                    },
                    file_name: "synthetic.pdf".into(),
                    caption: None,
                    pages: None,
                };
                row
            })
            .collect();
        app.conversations.insert(
            "synthetic".into(),
            Conversation {
                messages,
                ..Default::default()
            },
        );
        (app, commands)
    }

    fn enqueue(app: &mut App, ids: &[&str]) {
        app.apply(
            Action::DownloadSelected {
                chat: "synthetic".into(),
                messages: ids.iter().map(|id| (*id).into()).collect(),
            },
            &egui::Context::default(),
        );
    }

    #[test]
    fn newly_selected_downloads_wake_an_idle_window_but_repeated_selections_do_not() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let (mut app, mut commands) = batch(2);
        let ctx = egui::Context::default();
        let wakes = std::sync::Arc::new(AtomicUsize::new(0));
        let observed = wakes.clone();
        ctx.set_request_repaint_callback(move |request| {
            if request.delay.is_zero() {
                observed.fetch_add(1, Ordering::SeqCst);
            }
        });
        let idle = || {
            for _ in 0..3 {
                let mut output = ctx.run_ui(Default::default(), |_| {});
                output.textures_delta.clear();
            }
        };
        idle();
        app.waker.attach(&ctx);
        let before = wakes.load(Ordering::SeqCst);
        enqueue(&mut app, &["0", "1"]);
        assert!(
            wakes.load(Ordering::SeqCst) > before,
            "a click after the queue pump schedules its next turn"
        );
        assert!(commands.try_recv().is_err());
        app.pump_selected_downloads();
        assert!(commands.try_recv().is_ok());
        assert!(commands.try_recv().is_ok());

        idle();
        let before = wakes.load(Ordering::SeqCst);
        enqueue(&mut app, &["0", "1"]);
        assert_eq!(
            wakes.load(Ordering::SeqCst),
            before,
            "an empty addition does not keep waking the app"
        );
        app.pump_selected_downloads();
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn failures_and_deleted_messages_release_slots_without_retrying() {
        let (mut app, mut commands) = batch(4);
        enqueue(&mut app, &["0", "1", "2", "3"]);
        app.pump_selected_downloads();
        for id in ["0", "1"] {
            assert!(matches!(commands.try_recv().unwrap(), Command::Download {
                manual: true, card: None, message, ..
            } if message == id));
        }
        assert!(commands.try_recv().is_err());
        assert!(app.viewer_media_state.is_empty());
        // The first running message and the first waiting message disappear.
        app.conversations
            .get_mut("synthetic")
            .unwrap()
            .messages
            .retain(|row| row.id != "0" && row.id != "2");
        app.handle_media("synthetic", "0", None, Err("Synthetic failure".into()));
        app.pump_selected_downloads();
        assert!(
            matches!(commands.try_recv().unwrap(), Command::Download { message, .. } if message == "3")
        );
        app.handle_media("synthetic", "1", None, Err("Synthetic failure".into()));
        app.pump_selected_downloads();
        assert!(commands.try_recv().is_err());
        assert!(matches!(
            app.conversations["synthetic"]
                .message("1")
                .unwrap()
                .content
                .media()
                .unwrap()
                .state,
            MediaState::Failed(_)
        ));
    }

    #[test]
    fn saved_in_flight_unselected_and_non_media_messages_are_skipped() {
        let (mut app, mut commands) = batch(7);
        let rows = &mut app.conversations.get_mut("synthetic").unwrap().messages;
        rows[0].content.media_mut().unwrap().path = Some("synthetic-saved.pdf".into());
        rows[1].content.media_mut().unwrap().state = MediaState::Downloading;
        rows[2].content = Content::text("Synthetic text");
        rows[3].content = Content::Revoked;
        rows[4].content.media_mut().unwrap().state = MediaState::Failed("Retry me".into());
        enqueue(&mut app, &["5", "4", "3", "2", "1", "0", "4", "unknown"]);
        app.pump_selected_downloads();
        for id in ["4", "5"] {
            assert!(
                matches!(commands.try_recv().unwrap(), Command::Download { message, .. } if message == id)
            );
        }
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn overflow_changes_no_media_state_and_sends_nothing() {
        let (mut app, mut commands) = batch(crate::selected_downloads::LIMIT + 1);
        let ids: Vec<_> = (0..=crate::selected_downloads::LIMIT)
            .map(|id| id.to_string())
            .collect();
        enqueue(
            &mut app,
            &ids.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        assert!(!app.toasts.is_empty());
        assert!(
            app.conversations["synthetic"]
                .messages
                .iter()
                .all(|row| matches!(row.content.media().unwrap().state, MediaState::Idle))
        );
        app.pump_selected_downloads();
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn locking_pauses_admission_and_disconnecting_cancels_waiting_files() {
        let (mut app, mut commands) = batch(4);
        enqueue(&mut app, &["0", "1", "2", "3"]);
        app.app_lock.lock();
        app.pump_selected_downloads();
        assert!(commands.try_recv().is_err());
        app.app_lock.unlocked(true);
        app.pump_selected_downloads();
        assert!(commands.try_recv().is_ok());
        assert!(commands.try_recv().is_ok());
        app.viewer_media_chat = Some("synthetic".into());
        app.viewer_media_state
            .insert("2".into(), MediaState::Downloading);
        app.handle_link(LinkStatus::Connecting, true);
        assert!(matches!(
            app.viewer_media_state.get("2"),
            Some(MediaState::Failed(_))
        ));
        app.pump_selected_downloads();
        assert!(matches!(
            app.conversations["synthetic"]
                .message("2")
                .unwrap()
                .content
                .media()
                .unwrap()
                .state,
            MediaState::Failed(_)
        ));
        app.handle_link(LinkStatus::Connected, true);
        app.handle_media("synthetic", "0", None, Err("Synthetic failure".into()));
        app.pump_selected_downloads();
        assert!(commands.try_recv().is_err());
        app.handle_link(LinkStatus::LoggedOut, true);
        assert!(app.selected_downloads.next().is_none());
        assert_eq!(
            app.selected_downloads
                .enqueue(vec![crate::selected_downloads::Target {
                    chat: "synthetic".into(),
                    message: "0".into()
                }])
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn off_screen_account_completion_advances_only_its_own_batch() {
        let (mut app, mut commands) = batch(3);
        let (backend, mut other_commands, events) = Backend::recording_with_events();
        let (mut other, _) = crate::account::Account::detached(
            &app.dirs,
            AccountId::parse("2").unwrap(),
            Default::default(),
        )
        .unwrap();
        std::mem::swap(&mut other.conversations, &mut app.accounts[0].conversations);
        // Both accounts can hold the same chat and message ids. The info
        // panel belongs to the window, while the queued files belong to an account.
        let visible = other.conversations["synthetic"].messages.clone();
        app.conversations.insert(
            "synthetic".into(),
            Conversation {
                messages: visible.clone(),
                ..Default::default()
            },
        );
        app.info_media = Some(ChatMedia::collect(
            "synthetic".into(),
            visible,
            crate::model::MEDIA_LIST_LIMIT,
        ));
        let listing_before = app.info_media.clone();
        other.backend = backend;
        other.link = LinkStatus::Connected;
        app.accounts.push(other);
        app.active = 1;
        enqueue(&mut app, &["0", "1", "2"]);
        app.active = 0;
        app.pump_selected_downloads();
        assert!(other_commands.try_recv().is_ok());
        assert!(other_commands.try_recv().is_ok());
        let hidden_path = PathBuf::from("synthetic-hidden.pdf");
        events
            .send(Event::Media {
                chat: "synthetic".into(),
                message: "0".into(),
                card: None,
                result: Ok(hidden_path.clone()),
            })
            .unwrap();
        app.handle_events();
        app.pump_selected_downloads();
        assert_eq!(app.active, 0);
        assert!(
            app.conversations["synthetic"]
                .message("0")
                .unwrap()
                .content
                .media()
                .unwrap()
                .path
                .is_none()
        );
        assert_eq!(
            app.info_media, listing_before,
            "a hidden completion leaves the visible account's panel alone"
        );
        assert_eq!(
            app.accounts[1].conversations["synthetic"]
                .message("0")
                .unwrap()
                .content
                .media()
                .unwrap()
                .path
                .as_ref(),
            Some(&hidden_path)
        );
        assert!(app.viewer_media_state.is_empty());
        assert!(commands.try_recv().is_err());
        assert!(
            matches!(other_commands.try_recv().unwrap(), Command::Download { message, .. } if message == "2")
        );
        let visible_path = PathBuf::from("synthetic-visible.pdf");
        app.handle_media("synthetic", "0", None, Ok(visible_path.clone()));
        let listed = app
            .info_media
            .as_ref()
            .unwrap()
            .docs
            .iter()
            .find(|row| row.id == "0")
            .unwrap();
        assert_eq!(
            listed.content.media().unwrap().path.as_ref(),
            Some(&visible_path),
            "a visible completion still updates its own panel"
        );
    }
}
