//! Maps known choices to whatsapp-rust message types. The library owns quoting,
//! encryption, fanout, stanza classification, receipts, and disappearing timers.

use super::super::{ChatId, Delivery, Event, Message, MessageField, Quoted, send_outgoing};
use super::*;

fn present(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

pub(super) fn buttons(id: Option<&str>, label: Option<&str>) -> Option<wa::Message> {
    use wa::message::buttons_response_message::{Response, Type};
    Some({
        let mut proto = wa::Message::default();
        proto.buttons_response_message = MessageField::some({
            let mut proto = wa::message::ButtonsResponseMessage::default();
            proto.selected_button_id = Some(present(id)?.into());
            proto.response = Some(Response::SelectedDisplayText(present(label)?.into()));
            proto.r#type = Some(Type::DISPLAY_TEXT);
            proto
        });
        proto
    })
}

pub(super) fn template(
    id: Option<&str>,
    label: Option<&str>,
    index: Option<u32>,
) -> Option<wa::Message> {
    Some({
        let mut proto = wa::Message::default();
        proto.template_button_reply_message = MessageField::some({
            let mut proto = wa::message::TemplateButtonReplyMessage::default();
            proto.selected_id = Some(present(id)?.into());
            proto.selected_display_text = Some(present(label)?.into());
            proto.selected_index = Some(index?);
            proto
        });
        proto
    })
}

fn native_reply(name: &str, id: &str, label: &str) -> wa::Message {
    use wa::message::interactive_response_message::{
        Body, InteractiveResponseMessage, NativeFlowResponseMessage,
    };
    {
        let mut proto = wa::Message::default();
        proto.interactive_response_message = MessageField::some({
            let mut proto = wa::message::InteractiveResponseMessage::default();
            proto.body = MessageField::some({
                let mut proto = Body::default();
                proto.text = Some(label.into());
                proto.format =
                    Some(wa::message::interactive_response_message::body::Format::EXTENSIONS_1);
                proto
            });
            proto.interactive_response_message = Some(
                InteractiveResponseMessage::NativeFlowResponseMessage(Box::new({
                    let mut proto = NativeFlowResponseMessage::default();
                    proto.name = Some(name.into());
                    proto.params_json = Some(serde_json::json!({"id": id}).to_string());
                    proto.version = Some(1);
                    proto
                })),
            );
            proto
        });
        proto
    }
}

pub(super) fn native(
    name: Option<&str>,
    label: Option<&str>,
    value: &serde_json::Value,
) -> (InteractiveAction, Vec<wa::Message>) {
    let field = |key| {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .and_then(|v| present(Some(v)))
    };
    match name {
        Some("quick_reply") if present(label).is_some() && field("id").is_some() => (
            InteractiveAction::Reply,
            vec![native_reply(
                "quick_reply",
                field("id").unwrap(),
                label.unwrap(),
            )],
        ),
        Some("cta_copy") if field("copy_code").is_some() => (
            InteractiveAction::Copy(field("copy_code").unwrap().into()),
            Vec::new(),
        ),
        Some("single_select") => {
            let mut options = Vec::new();
            let mut replies = Vec::new();
            if let Some(sections) = value.get("sections").and_then(|v| v.as_array()) {
                for section in sections {
                    let Some(rows) = section.get("rows").and_then(|v| v.as_array()) else {
                        continue;
                    };
                    for row in rows {
                        let title = row.get("title").and_then(|v| v.as_str());
                        let id = row.get("id").and_then(|v| v.as_str());
                        let (Some(title), Some(id)) = (present(title), present(id)) else {
                            continue;
                        };
                        options.push(InteractiveOption {
                            section: section
                                .get("title")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .into(),
                            title: title.into(),
                            description: row
                                .get("description")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .into(),
                        });
                        replies.push(native_reply("single_select", id, title));
                    }
                }
            }
            if options.is_empty() {
                (InteractiveAction::Unavailable, Vec::new())
            } else {
                (InteractiveAction::Select(options), replies)
            }
        }
        _ => (InteractiveAction::Unavailable, Vec::new()),
    }
}

pub(super) fn list(
    message: &wa::message::ListMessage,
) -> (Vec<InteractiveOption>, Vec<wa::Message>) {
    use wa::message::{list_message, list_response_message};
    if message.list_type != Some(list_message::ListType::SINGLE_SELECT) {
        return (Vec::new(), Vec::new());
    }
    let mut options = Vec::new();
    let mut replies = Vec::new();
    for section in &message.sections {
        for row in &section.rows {
            let (Some(title), Some(id)) = (
                present(row.title.as_deref()),
                present(row.row_id.as_deref()),
            ) else {
                continue;
            };
            options.push(InteractiveOption {
                section: section.title.clone().unwrap_or_default(),
                title: title.into(),
                description: row.description.clone().unwrap_or_default(),
            });
            replies.push({
                let mut proto = wa::Message::default();
                proto.list_response_message = MessageField::some({
                    let mut proto = wa::message::ListResponseMessage::default();
                    proto.title = Some(title.into());
                    proto.description = row.description.clone();
                    proto.list_type = Some(list_response_message::ListType::SINGLE_SELECT);
                    proto.single_select_reply = MessageField::some({
                        let mut proto = list_response_message::SingleSelectReply::default();
                        proto.selected_row_id = Some(id.into());
                        proto
                    });
                    proto
                });
                proto
            });
        }
    }
    (options, replies)
}

/// Re-resolve every click against the archived protobuf, never UI-supplied ids or text.
fn response(original: &wa::Message, button: usize, choice: Option<usize>) -> Option<wa::Message> {
    let base = original.get_base_message();
    let parsed = parse(base)?;
    let Content::Interactive {
        card: Some(card), ..
    } = classify(base)?
    else {
        return None;
    };
    let index = match (&card.buttons.get(button)?.action, choice) {
        (InteractiveAction::Reply, None) => 0,
        (InteractiveAction::Select(options), Some(index)) if index < options.len() => index,
        _ => return None,
    };
    parsed.replies.get(button)?.get(index).cloned()
}

fn prepare(
    source: &Message,
    original: &wa::Message,
    jid: &super::super::Jid,
    button: usize,
    choice: Option<usize>,
) -> Option<wa::Message> {
    if source.from_me
        || source.edited
        || !matches!(source.content, Content::Interactive { card: Some(_), .. })
    {
        return None;
    }
    let mut reply = response(original, button, choice)?;
    let sender = Worker::jid_of(&source.sender)?;
    let context = whatsapp_rust::wacore::proto_helpers::build_quote_context_with_info(
        source.id.clone(),
        &sender,
        jid,
        jid,
        original,
    );
    reply.set_context_info(context).then_some(reply)
}

impl Worker {
    /// Re-resolves archived interactive choices and quotes through the normal validated send path.
    pub(in super::super) fn reply_interactive(
        &mut self,
        chat: ChatId,
        source_id: String,
        button: usize,
        choice: Option<usize>,
    ) {
        if self
            .interactive_sending
            .contains_key(&(chat.clone(), source_id.clone()))
        {
            return;
        }
        let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&chat)) else {
            self.emit(Event::Error("Not connected to WhatsApp".into()));
            return;
        };
        let prepared = (|| {
            let row = self.archive.message(&chat, &source_id).ok().flatten()?;
            let raw = self.archive.raw(&chat, &source_id).ok().flatten()?;
            let original = wa::Message::decode_from_slice(&raw).ok()?;
            let reply = prepare(&row, &original, &jid, button, choice)?;
            Some((row, reply))
        })();
        let Some((source, mut message)) = prepared else {
            self.emit(Event::Error("This option is unavailable in ZapFast. Open the message in WhatsApp Web or on your phone.".into()));
            return;
        };
        let expiration = self.apply_ephemeral(&chat, &mut message);
        let id = client.generate_message_id().into_string();
        let content = classify(&message).expect("validated interactive reply");
        let row = Message {
            id: id.clone(),
            chat: chat.clone(),
            sender: self.me(),
            sender_name: None,
            from_me: true,
            timestamp: crate::util::now(),
            content,
            status: Delivery::Pending,
            delivered_at: None,
            read_at: None,
            quoted: Some(Quoted {
                labelled: Some(source.content.labelled()),
                chat: None,
                mentions: source.mentions,
                id: source.id,
                sender_name: source.sender_name.or_else(|| self.name_for(&source.sender)),
                sender: source.sender,
                summary: source.content.summary(),
            }),
            reactions: Vec::new(),
            history_order: None,
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        };
        self.interactive_sending
            .insert((chat.clone(), source_id.clone()), id.clone());
        self.emit(Event::InteractiveReplyState {
            chat: chat.clone(),
            message: source_id,
            pending: true,
        });
        self.store_message(row, Some(message.encode_to_vec()), None);
        tokio::spawn(send_outgoing(
            client,
            self.commands.clone(),
            chat,
            jid,
            id,
            message,
            expiration,
            None,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::receipt_tests::{PEER, own_message, worker};
    use super::*;
    use whatsapp_rust::wacore::send::{media_type_from_message, stanza_type_from_message};

    fn native_request(name: &str, json: &str) -> wa::Message {
        use wa::message::interactive_message::{
            InteractiveMessage, NativeFlowMessage, native_flow_message::NativeFlowButton,
        };
        {
            let mut proto = wa::Message::default();
            proto.interactive_message = MessageField::some({
                let mut proto = wa::message::InteractiveMessage::default();
                proto.body = MessageField::some({
                    let mut proto = wa::message::interactive_message::Body::default();
                    proto.text = Some("Choose an option".into());
                    proto
                });
                proto.interactive_message =
                    Some(InteractiveMessage::NativeFlowMessage(Box::new({
                        let mut proto = NativeFlowMessage::default();
                        proto.buttons = vec![{
                            let mut proto = NativeFlowButton::default();
                            proto.name = Some(name.into());
                            proto.button_params_json = Some(json.into());
                            proto
                        }];
                        proto
                    })));
                proto
            });
            proto
        }
    }

    fn requests() -> Vec<(wa::Message, Option<usize>, Option<&'static str>)> {
        use wa::hydrated_template_button::{HydratedButton, HydratedQuickReplyButton};
        use wa::message::{buttons_message, list_message, template_message};
        vec![
            (
                {
                    let mut proto = wa::Message::default();
                    proto.buttons_message = MessageField::some({
                        let mut proto = wa::message::ButtonsMessage::default();
                        proto.content_text = Some("Choose".into());
                        proto.buttons = vec![{
                            let mut proto = buttons_message::Button::default();
                            proto.button_id = Some("private-choice".into());
                            proto.button_text = MessageField::some({
                                let mut proto = buttons_message::button::ButtonText::default();
                                proto.display_text = Some("Continue".into());
                                proto
                            });
                            proto.r#type = Some(buttons_message::button::Type::RESPONSE);
                            proto
                        }];
                        proto
                    });
                    proto
                },
                None,
                Some("buttons_response"),
            ),
            (
                {
                    let mut proto = wa::Message::default();
                    proto.template_message = MessageField::some({
                        let mut proto = wa::message::TemplateMessage::default();
                        proto.hydrated_template = MessageField::some({
                            let mut proto = template_message::HydratedFourRowTemplate::default();
                            proto.hydrated_content_text = Some("Choose".into());
                            proto.hydrated_buttons = vec![{
                                let mut proto = wa::HydratedTemplateButton::default();
                                proto.index = Some(7);
                                proto.hydrated_button =
                                    Some(HydratedButton::QuickReplyButton(Box::new({
                                        let mut proto = HydratedQuickReplyButton::default();
                                        proto.id = Some("private-choice".into());
                                        proto.display_text = Some("Continue".into());
                                        proto
                                    })));
                                proto
                            }];
                            proto
                        });
                        proto
                    });
                    proto
                },
                None,
                None,
            ),
            (
                native_request(
                    "quick_reply",
                    r#"{"display_text":"Continue","id":"private-choice","secret":"must-not-echo"}"#,
                ),
                None,
                Some("native_flow_response"),
            ),
            (
                native_request(
                    "single_select",
                    r#"{"title":"Choose","sections":[{"title":"Schedule","rows":[{"title":"Missing id"},{"title":"Continue","id":"private-choice","description":"Morning 🎨"}]}]}"#,
                ),
                Some(0),
                Some("native_flow_response"),
            ),
            (
                {
                    let mut proto = wa::Message::default();
                    proto.list_message = MessageField::some({
                        let mut proto = wa::message::ListMessage::default();
                        proto.title = Some("Choose".into());
                        proto.button_text = Some("Choose a time".into());
                        proto.list_type = Some(list_message::ListType::SINGLE_SELECT);
                        proto.sections = vec![{
                            let mut proto = list_message::Section::default();
                            proto.title = Some("Schedule".into());
                            proto.rows = vec![
                                {
                                    let mut proto = list_message::Row::default();
                                    proto.title = Some("Invalid choice".into());
                                    proto
                                },
                                {
                                    let mut proto = list_message::Row::default();
                                    proto.title = Some("Continue".into());
                                    proto.row_id = Some("private-choice".into());
                                    proto
                                },
                            ];
                            proto
                        }];
                        proto
                    });
                    proto
                },
                Some(0),
                Some("list_response"),
            ),
        ]
    }

    #[test]
    fn replies_preserve_choice_and_quote_for_the_library_sender() {
        for (request, choice, media_type) in requests() {
            let request = {
                let mut proto = wa::Message::default();
                proto.view_once_message = MessageField::some({
                    let mut proto = wa::message::FutureProofMessage::default();
                    proto.message = MessageField::some(request);
                    proto
                });
                proto
            };
            let mut source = own_message("original", 123);
            source.from_me = false;
            source.sender = PEER.into();
            source.content = classify(request.get_base_message()).unwrap();
            assert!(
                !serde_json::to_string(&source.content)
                    .unwrap()
                    .contains("private-choice")
            );
            let mut reply =
                prepare(&source, &request, &Worker::jid_of(PEER).unwrap(), 0, choice).unwrap();
            assert!(reply.conversation.is_none());
            assert!(!reply.extended_text_message.is_set());
            assert!(reply.set_ephemeral_expiration(86400));
            let reply = wa::Message::decode_from_slice(&reply.encode_to_vec()).unwrap();
            let context = context(&reply).unwrap();
            assert_eq!(context.stanza_id.as_deref(), Some("original"));
            assert_eq!(context.participant.as_deref(), Some(PEER));
            assert_eq!(context.expiration, Some(86400));
            assert!(is_request(context.quoted_message.get_base_message()));
            assert_eq!(classify(&reply).unwrap().summary(), "Continue");
            assert_eq!(media_type_from_message(&reply), media_type);
            assert_eq!(
                stanza_type_from_message(&reply),
                if media_type.is_some() {
                    "media"
                } else {
                    "text"
                }
            );
            if let Some(button) = reply.buttons_response_message.as_option() {
                assert_eq!(button.selected_button_id.as_deref(), Some("private-choice"));
            }
            if let Some(button) = reply.template_button_reply_message.as_option() {
                assert_eq!(button.selected_id.as_deref(), Some("private-choice"));
                assert_eq!(button.selected_index, Some(7));
            }
            if let Some(list) = reply.list_response_message.as_option() {
                assert_eq!(
                    list.single_select_reply.selected_row_id.as_deref(),
                    Some("private-choice")
                );
            }
            if let Some(button) = reply.interactive_response_message.as_option() {
                let Some(wa::message::interactive_response_message::InteractiveResponseMessage::NativeFlowResponseMessage(flow)) = &button.interactive_response_message else { panic!("native reply") };
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(flow.params_json.as_deref().unwrap())
                        .unwrap(),
                    serde_json::json!({"id":"private-choice"})
                );
                assert_eq!(flow.version, Some(1));
            }
        }
    }

    #[test]
    fn unknown_actions_invalid_choices_and_local_actions_never_send_replies() {
        for (request, choice, _) in requests() {
            assert!(response(&request, 99, choice).is_none());
            assert!(response(&request, 0, Some(99)).is_none());
            if choice.is_some() {
                assert!(response(&request, 0, None).is_none());
            } else {
                assert!(response(&request, 0, Some(0)).is_none());
            }
            let mut source = own_message("original", 123);
            source.content = classify(&request).unwrap();
            let jid = Worker::jid_of(PEER).unwrap();
            assert!(prepare(&source, &request, &jid, 0, choice).is_none());
            source.from_me = false;
            source.edited = true;
            assert!(prepare(&source, &request, &jid, 0, choice).is_none());
            source.edited = false;
            source.content = Content::Revoked;
            assert!(prepare(&source, &request, &jid, 0, choice).is_none());
        }
        for (name, json) in [
            (
                "unknown",
                r#"{"display_text":"Continue","id":"private-choice"}"#,
            ),
            ("quick_reply", r#"{"display_text":"Continue"}"#),
            ("quick_reply", r#"{"display_text":"Continue","id":42}"#),
            (
                "cta_url",
                r#"{"display_text":"Website","url":"https://example.com/"}"#,
            ),
            (
                "cta_copy",
                r#"{"display_text":"Copy code","copy_code":"CEDAR20"}"#,
            ),
            (
                "payment_info",
                r#"{"display_text":"Pay","id":"private-choice"}"#,
            ),
        ] {
            let request = native_request(name, json);
            assert!(response(&request, 0, None).is_none());
        }
    }

    #[test]
    fn existing_cards_gain_actions_without_losing_downloads() {
        let (mut worker, _, _, _) = worker();
        worker.archive.ensure_chat(PEER, "Demo").unwrap();
        let mut request = requests().remove(0).0;
        request.buttons_message.as_option_mut().unwrap().header = Some(
            wa::message::buttons_message::Header::ImageMessage(Box::new({
                let mut proto = wa::message::ImageMessage::default();
                proto.mimetype = Some("image/jpeg".into());
                proto.width = Some(640);
                proto.height = Some(480);
                proto
            })),
        );
        let raw = request.encode_to_vec();
        let mut source = own_message("original", 123);
        source.content = classify(&request).unwrap();
        let path = std::path::PathBuf::from("/tmp/synthetic-interactive.jpg");
        source.content.media_mut().unwrap().path = Some(path.clone());
        if let Content::Interactive {
            card: Some(card), ..
        } = &mut source.content
        {
            card.buttons[0].action = InteractiveAction::Unavailable;
        }
        worker.archive.insert_message(&source, Some(&raw)).unwrap();
        worker.archive.set_meta("interactive_text", "2").unwrap();
        worker.backfill_interactive();
        worker.backfill_interactive();
        let recovered = worker.archive.message(PEER, "original").unwrap().unwrap();
        assert_eq!(
            recovered.content.media().unwrap().path.as_ref(),
            Some(&path)
        );
        let Content::Interactive {
            card: Some(card), ..
        } = recovered.content
        else {
            panic!("card")
        };
        assert_eq!(card.buttons[0].action, InteractiveAction::Reply);
        assert_eq!(worker.archive.raw(PEER, "original").unwrap(), Some(raw));
        assert_eq!(
            worker.archive.meta("interactive_text").unwrap().as_deref(),
            Some("4")
        );
    }

    #[tokio::test]
    async fn failed_send_releases_the_reply_for_another_attempt() {
        let (mut worker, _, _, _) = worker();
        worker
            .interactive_sending
            .insert((PEER.into(), "original".into()), "outgoing".into());
        worker
            .handle_command(super::super::super::Command::Sent {
                chat: PEER.into(),
                id: "outgoing".into(),
                error: Some("offline".into()),
            })
            .await;
        assert!(worker.interactive_sending.is_empty());
    }
    #[test]
    fn mixed_buttons_and_filtered_sections_keep_the_exact_choice_ids() {
        use wa::message::interactive_message::InteractiveMessage as Payload;
        let mut request = native_request("quick_reply", r#"{"display_text":"First","id":"first"}"#);
        let Some(Payload::NativeFlowMessage(flow)) = &mut request
            .interactive_message
            .as_option_mut()
            .unwrap()
            .interactive_message
        else {
            panic!("flow")
        };
        for (name, json) in [
            ("quick_reply", "{broken"),
            (
                "cta_url",
                r#"{"display_text":"Website","url":"https://example.com/"}"#,
            ),
            ("quick_reply", r#"{"display_text":"Second","id":"second"}"#),
            (
                "single_select",
                r#"{"title":"Choose","sections":[{"title":"A","rows":[{"title":"Invalid"},{"title":"Same label","id":"a"}]},{"title":"B","rows":[{"title":"Same label","id":"b"},{"title":"Skip","id":""},{"title":"Last","id":"c"}]}]}"#,
            ),
        ] {
            let extra = native_request(name, json);
            let Some(Payload::NativeFlowMessage(extra)) =
                extra.interactive_message.unwrap().interactive_message
            else {
                panic!("flow")
            };
            flow.buttons.extend(extra.buttons);
        }
        for (button, choice, expected) in [
            (0, None, "first"),
            (2, None, "second"),
            (3, Some(0), "a"),
            (3, Some(1), "b"),
            (3, Some(2), "c"),
        ] {
            let reply = response(&request, button, choice).unwrap();
            let Some(wa::message::interactive_response_message::InteractiveResponseMessage::NativeFlowResponseMessage(flow)) = reply.interactive_response_message.unwrap().interactive_response_message else { panic!("reply") };
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(flow.params_json.as_deref().unwrap())
                    .unwrap(),
                serde_json::json!({"id":expected})
            );
        }
        assert!(response(&request, 1, None).is_none());
        assert!(response(&request, 3, Some(3)).is_none());
    }

    #[tokio::test]
    async fn duplicate_clicks_and_unrelated_send_results_do_not_unlock_other_cards() {
        for error in [None, Some("offline".to_owned())] {
            let (mut worker, events, _, _) = worker();
            worker
                .interactive_sending
                .insert((PEER.into(), "source".into()), "outgoing".into());
            worker.interactive_sending.insert(
                (PEER.into(), "other-source".into()),
                "other-outgoing".into(),
            );
            worker.reply_interactive(PEER.into(), "source".into(), 0, None);
            assert!(
                events.try_recv().is_err(),
                "duplicate must stop before any send or connection error"
            );
            worker
                .handle_command(super::super::super::Command::Sent {
                    chat: "other-chat".into(),
                    id: "outgoing".into(),
                    error: None,
                })
                .await;
            assert_eq!(worker.interactive_sending.len(), 2);
            assert!(
                !events
                    .try_iter()
                    .any(|event| matches!(event, Event::InteractiveReplyState { .. }))
            );
            worker
                .handle_command(super::super::super::Command::Sent {
                    chat: PEER.into(),
                    id: "outgoing".into(),
                    error,
                })
                .await;
            assert_eq!(worker.interactive_sending.len(), 1);
            assert!(events.try_iter().any(|event| matches!(event, Event::InteractiveReplyState { message, pending: false, .. } if message == "source")));
        }
    }
}
