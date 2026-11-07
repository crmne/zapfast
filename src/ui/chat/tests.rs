use super::*;

#[cfg(test)]
mod tests {
    /// Glass buttons leave the name no room in a narrow header (#626).
    #[test]
    fn a_vibrant_header_folds_its_buttons_rather_than_cover_the_name() {
        assert!(super::controls_width(4.0, true) > super::controls_width(4.0, false));
        assert!(super::header_fits(500.0, 4.0, 0.0, true));
        assert!(!super::header_fits(360.0, 4.0, 0.0, true));
        // Folded, More alone leaves the name room.
        assert!(super::header_fits(280.0, 1.0, 0.0, true));
        assert!(!super::header_fits(460.0, 4.0, 120.0, true));
    }

    /// The plain header beside an info panel: four buttons take about 164
    /// points there, and counting less slid the name under Search.
    #[test]
    fn a_plain_header_folds_its_buttons_beside_the_info_panel() {
        assert!(!super::header_fits(353.0, 4.0, 0.0, false));
        assert!(super::header_fits(353.0, 1.0, 0.0, false));
        assert!(super::header_fits(500.0, 4.0, 0.0, false));
    }

    #[test]
    fn more_stands_clear_of_drawn_caption_buttons_only() {
        assert_eq!(super::caption_gap(0.0), 0.0);
        assert!(super::caption_gap(120.0) > 0.0);
    }

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

    /// The phone groups a run of pictures that carry no album id (#733).
    #[test]
    fn four_pictures_in_a_row_group_without_an_album_id() {
        let mut messages: Vec<_> = ["one", "two", "three", "four", "five"]
            .iter()
            .map(|id| album_picture(id, None, "alice"))
            .collect();
        assert_eq!(album_run(&messages, 0).map(<[Message]>::len), Some(5));
        assert!(album_run(&messages, 1).is_none(), "drawn with the first");
        assert!(album_continues(&messages, 3));
        assert!(!album_continues(&messages, 0));
        assert_eq!(row_starts(&messages, false), [0]);
        assert_eq!(row_starts(&messages, true).len(), 5);
        // Three are not enough, and a caption stands alone.
        assert!(album_run(&messages[..3], 0).is_none());
        if let Content::Image { caption, .. } = &mut messages[2].content {
            *caption = Some("Look".into());
        }
        assert!(album_run(&messages, 0).is_none());
        assert_eq!(row_starts(&messages, false).len(), 5);
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

    /// The unread divider skips the rows the unread count leaves out, so the
    /// count-th real incoming message is the right one.
    #[test]
    fn the_unread_divider_skips_rows_that_are_not_counted() {
        let incoming = |id: &str, content: Content| {
            let mut row = message(content);
            row.id = id.into();
            row.from_me = false;
            row
        };
        let rows = vec![
            incoming("old", Content::text("old")),
            incoming(
                "zapfast-timer-1",
                Content::DisappearingTimer { seconds: 86_400 },
            ),
            incoming(
                "zapfast-call-1",
                Content::Call {
                    video: false,
                    status: crate::model::CallStatus::Missed,
                    duration: 0,
                    group: false,
                    participants: None,
                },
            ),
            incoming("newest", Content::text("newest")),
        ];
        assert_eq!(unread_divider_message(&rows, 1).unwrap().id, "newest");
        // Neither local row is counted, so the second real message is chosen.
        assert_eq!(unread_divider_message(&rows, 2).unwrap().id, "old");
    }

    #[test]
    fn timer_notices_break_message_runs_on_both_sides() {
        for from_me in [false, true] {
            let text = Message {
                from_me,
                ..message(Content::text("Hello"))
            };
            let notice = Message {
                content: Content::DisappearingTimer { seconds: 86_400 },
                ..text.clone()
            };
            assert!(same_run(&text, &text));
            assert!(!same_run(&text, &notice));
            assert!(!same_run(&notice, &text));
            assert!(!same_run(&notice, &notice));
            let rows = [text.clone(), notice, text];
            assert!(last_of_run(&rows, 0));
            for palette in [Palette::dark(), Palette::messages()] {
                assert!(last_bubble_of_run(&palette, &rows, 0));
                assert!(last_bubble_of_run(&palette, &rows, 2));
            }
        }
    }

    #[test]
    fn timer_notices_return_a_navigation_response_without_selection() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        with_still_view(|view| {
            let notice = message(Content::DisappearingTimer { seconds: 86_400 });
            let mut actions = Vec::new();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(600.0, 300.0))),
                    ..Default::default()
                },
                |ui| {
                    let response = bubble(
                        ui,
                        view,
                        &notice,
                        None,
                        RunPlace {
                            show_sender: false,
                            show_avatar: false,
                            tailed: false,
                        },
                        &mut actions,
                    )
                    .expect("the notice needs a response for scroll and focus handling");
                    assert!(response.rect.is_positive());
                    assert!(!response.sense.senses_click());
                    assert!(!response.sense.senses_drag());
                },
            );
            output.textures_delta.clear();
            assert!(!crate::app::can_select(&notice.content));
            assert!(actions.is_empty());
            assert!(view.copy_rows.unwrap().lock().unwrap().is_empty());
        });
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
        let arrivals = HashMap::new();
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
            download_progress: None,
            selecting: false,
            motion_playing: None,
            starred: None,
            pins: None,
            pin_notices: &[],
            arrivals: &arrivals,
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

    #[test]
    fn reaction_groups_keep_every_person_and_rank_counts_with_stable_ties() {
        let message = with_reactions(vec![
            reaction(false, "❤️"),
            reaction(true, "👍"),
            reaction(false, "😂"),
            reaction(false, "👍"),
            reaction(false, "🙏"),
        ]);
        let groups = reaction_groups(&message);
        assert_eq!(
            groups
                .iter()
                .map(|(emoji, people)| (*emoji, people.len()))
                .collect::<Vec<_>>(),
            [("👍", 2), ("❤️", 1), ("😂", 1), ("🙏", 1)]
        );
        assert!(groups[0].1.iter().any(|reaction| reaction.from_me));
        assert_eq!(
            groups.iter().map(|(_, people)| people.len()).sum::<usize>(),
            5
        );
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
