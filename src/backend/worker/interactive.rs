//! Presentation of interactive payloads decoded by whatsapp-rust.
//! Never expose flow JSON, internal ids, or template substitution parameters.

use super::{Content, MessageExt, Worker, forwarded_of, mentioned_of, wa};
use crate::model::{InteractiveAction, InteractiveButton, InteractiveCard, InteractiveOption};
use whatsapp_rust::waproto::buffa::Message as _;

mod replies;

#[derive(Default)]
struct Text {
    parts: Vec<String>,
    body: Vec<String>,
    buttons: Vec<InteractiveButton>,
    /// Kept beside display rows while parsing, never serialized into the model.
    replies: Vec<Vec<wa::Message>>,
    omitted: bool,
}

impl Text {
    fn push(&mut self, value: Option<&str>) {
        if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
            self.parts.push(value.to_owned());
            self.body.push(value.to_owned());
        }
    }

    fn action(
        &mut self,
        value: Option<&str>,
        url: Option<&str>,
        action: InteractiveAction,
        replies: Vec<wa::Message>,
    ) {
        if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
            self.parts.push(format!("• {value}"));
            self.buttons.push(InteractiveButton {
                label: value.to_owned(),
                url: url.and_then(web_url),
                action,
            });
            self.replies.push(replies);
        } else {
            self.omitted = true;
        }
    }

    fn native_button(&mut self, name: Option<&str>, json: Option<&str>) {
        // Only recognized actions are executable. Never echo arbitrary JSON
        // back to the sender or expose private flow fields to the interface.
        let Some(json) = json.filter(|json| json.len() <= 64 * 1024) else {
            self.omitted = true;
            return;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
            self.omitted = true;
            return;
        };
        let label = value
            .get("display_text")
            .and_then(|v| v.as_str())
            .or_else(|| value.get("title").and_then(|v| v.as_str()));
        let (action, replies) = replies::native(name, label, &value);
        self.action(
            label,
            (name == Some("cta_url"))
                .then(|| value.get("url").and_then(|v| v.as_str()))
                .flatten(),
            action,
            replies,
        );
    }

    fn interactive(&mut self, message: &wa::message::InteractiveMessage, depth: usize) {
        use wa::message::interactive_message::InteractiveMessage as Payload;
        // Malformed nested carousels should not grow the call stack indefinitely.
        if depth >= 8 {
            return;
        }
        if let Some(header) = message.header.as_option() {
            self.push(header.title.as_deref());
            self.push(header.subtitle.as_deref());
        }
        self.push(
            message
                .body
                .as_option()
                .and_then(|body| body.text.as_deref()),
        );
        self.push(
            message
                .footer
                .as_option()
                .and_then(|footer| footer.text.as_deref()),
        );
        match message.interactive_message.as_ref() {
            Some(Payload::NativeFlowMessage(flow)) => {
                for button in &flow.buttons {
                    self.native_button(
                        button.name.as_deref(),
                        button.button_params_json.as_deref(),
                    );
                }
            }
            Some(Payload::CarouselMessage(carousel)) => {
                for card in &carousel.cards {
                    self.interactive(card, depth + 1);
                }
            }
            _ => {}
        }
    }

    fn hydrated(&mut self, template: &wa::message::template_message::HydratedFourRowTemplate) {
        use wa::hydrated_template_button::HydratedButton;
        use wa::message::template_message::hydrated_four_row_template::Title;
        if let Some(Title::HydratedTitleText(title)) = &template.title {
            self.push(Some(title));
        }
        self.push(template.hydrated_content_text.as_deref());
        self.push(template.hydrated_footer_text.as_deref());
        for button in &template.hydrated_buttons {
            let label = match &button.hydrated_button {
                Some(HydratedButton::QuickReplyButton(button)) => button.display_text.as_deref(),
                Some(HydratedButton::UrlButton(button)) => button.display_text.as_deref(),
                Some(HydratedButton::CallButton(button)) => button.display_text.as_deref(),
                None => None,
            };
            let url = match &button.hydrated_button {
                Some(HydratedButton::UrlButton(button)) => button.url.as_deref(),
                _ => None,
            };
            let reply = match &button.hydrated_button {
                Some(HydratedButton::QuickReplyButton(reply)) => {
                    replies::template(reply.id.as_deref(), label, button.index)
                }
                _ => None,
            };
            self.reply_button(label, url, reply);
        }
    }

    fn reply_button(&mut self, label: Option<&str>, url: Option<&str>, reply: Option<wa::Message>) {
        let action = if reply.is_some() {
            InteractiveAction::Reply
        } else {
            InteractiveAction::Unavailable
        };
        self.action(label, url, action, reply.into_iter().collect());
    }

    fn finish(self, base: &wa::Message, is_request: bool) -> Content {
        let image = image(base).map(|image| {
            super::media(
                image.mimetype.as_ref(),
                image.file_length,
                image.width,
                image.height,
            )
        });
        let children = carousel_messages(base);
        let carousel = children.is_some();
        let needs_phone = self.omitted
            || (!carousel && self.body.is_empty() && image.is_none())
            || has_unrendered_content(base);
        let card = is_request.then(|| {
            let body = if let Some(parent) = envelope(base).filter(|_| carousel) {
                let mut parent = parent.clone();
                parent.interactive_message = None;
                let mut text = Text::default();
                text.interactive(&parent, 0);
                text.body.join("\n\n")
            } else {
                self.body.join("\n\n")
            };
            Box::new(InteractiveCard {
                body,
                buttons: if carousel { Vec::new() } else { self.buttons },
                image,
                needs_phone,
                carousel: children
                    .unwrap_or_default()
                    .iter()
                    .map(|child| {
                        let mut text = Text::default();
                        text.interactive(child, 1);
                        let base = wa::Message {
                            interactive_message: super::MessageField::some(child.clone()),
                            ..Default::default()
                        };
                        let picture = image_at(&base, None);
                        // Local actions need no carousel reply envelope. Keep other
                        // actions disabled until the library supports that envelope.
                        for button in &mut text.buttons {
                            if matches!(
                                button.action,
                                InteractiveAction::Reply | InteractiveAction::Select(_)
                            ) {
                                button.action = InteractiveAction::Unavailable;
                            }
                        }
                        InteractiveCard {
                            body: text.body.join("\n\n"),
                            buttons: text.buttons,
                            image: picture.map(|m| {
                                super::media(m.mimetype.as_ref(), m.file_length, m.width, m.height)
                            }),
                            thumbnail: picture.and_then(|m| m.jpeg_thumbnail.clone()),
                            needs_phone: text.omitted
                                || has_unrendered_content(&base)
                                || carousel_messages(&base).is_some(),
                            ..Default::default()
                        }
                    })
                    .collect(),
                thumbnail: None,
            })
        });
        Content::Interactive {
            text: if self.parts.is_empty() {
                "Interactive message".into()
            } else {
                self.parts.join("\n\n")
            },
            card,
        }
    }
}

pub(super) fn classify(base: &wa::Message) -> Option<Content> {
    let text = parse(base)?;
    Some(text.finish(base, is_request(base)))
}

fn is_request(base: &wa::Message) -> bool {
    base.interactive_message.is_set()
        || base.buttons_message.is_set()
        || base.list_message.is_set()
        || base.template_message.is_set()
}

fn parse(base: &wa::Message) -> Option<Text> {
    let mut text = Text::default();
    if let Some(message) = base.interactive_message.as_option() {
        text.interactive(message, 0);
    } else if let Some(message) = base.buttons_message.as_option() {
        if let Some(wa::message::buttons_message::Header::Text(title)) = &message.header {
            text.push(Some(title));
        }
        text.push(message.content_text.as_deref());
        text.push(message.footer_text.as_deref());
        for button in &message.buttons {
            let label = button
                .button_text
                .as_option()
                .and_then(|label| label.display_text.as_deref());
            if let Some(flow) = button.native_flow_info.as_option()
                && button.r#type == Some(wa::message::buttons_message::button::Type::NATIVE_FLOW)
            {
                text.native_button(flow.name.as_deref(), flow.params_json.as_deref());
            } else if label.is_some_and(|label| !label.trim().is_empty()) {
                let reply = (button.r#type
                    == Some(wa::message::buttons_message::button::Type::RESPONSE))
                .then(|| replies::buttons(button.button_id.as_deref(), label))
                .flatten();
                text.reply_button(label, None, reply);
            } else if let Some(flow) = button.native_flow_info.as_option() {
                text.native_button(flow.name.as_deref(), flow.params_json.as_deref());
            } else {
                text.omitted = true;
            }
        }
    } else if let Some(message) = base.list_message.as_option() {
        text.push(message.title.as_deref());
        text.push(message.description.as_deref());
        text.push(message.footer_text.as_deref());
        if message.button_text.is_some() {
            let (options, replies) = replies::list(message);
            let action = if options.is_empty() {
                InteractiveAction::Unavailable
            } else {
                InteractiveAction::Select(options)
            };
            text.action(message.button_text.as_deref(), None, action, replies);
        }
        for section in &message.sections {
            if let Some(title) = &section.title {
                text.parts.push(title.clone());
            }
            for row in &section.rows {
                let label = row
                    .title
                    .as_deref()
                    .filter(|title| !title.trim().is_empty())
                    .map(|title| format!("• {title}"));
                if let Some(label) = label {
                    text.parts.push(label);
                }
                if let Some(description) = &row.description {
                    text.parts.push(description.clone());
                }
            }
        }
    } else if let Some(message) = base.template_message.as_option() {
        use wa::message::template_message::Format;
        if let Some(template) = message.hydrated_template.as_option() {
            text.hydrated(template);
        } else {
            match &message.format {
                Some(Format::HydratedFourRowTemplate(template)) => text.hydrated(template),
                Some(Format::InteractiveMessageTemplate(template)) => text.interactive(template, 0),
                // Non-hydrated templates contain references and substitution
                // parameters, not the rendered message. Do not invent text.
                _ => {}
            }
        }
    } else if let Some(message) = base.buttons_response_message.as_option() {
        if let Some(wa::message::buttons_response_message::Response::SelectedDisplayText(label)) =
            &message.response
        {
            text.push(Some(label));
        }
    } else if let Some(message) = base.list_response_message.as_option() {
        text.push(message.title.as_deref());
        text.push(message.description.as_deref());
    } else if let Some(message) = base.interactive_response_message.as_option() {
        text.push(
            message
                .body
                .as_option()
                .and_then(|body| body.text.as_deref()),
        );
    } else {
        let message = base.template_button_reply_message.as_option()?;
        text.push(message.selected_display_text.as_deref());
    }
    Some(text)
}

/// Keep URL actions within the same schemes as ordinary web links.
fn web_url(value: &str) -> Option<String> {
    let url = reqwest::Url::parse(value).ok()?;
    (matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}

fn hydrated(base: &wa::Message) -> Option<&wa::message::template_message::HydratedFourRowTemplate> {
    let message = base.template_message.as_option()?;
    message
        .hydrated_template
        .as_option()
        .or_else(|| match &message.format {
            Some(wa::message::template_message::Format::HydratedFourRowTemplate(template)) => {
                Some(template.as_ref())
            }
            _ => None,
        })
}

fn envelope(base: &wa::Message) -> Option<&wa::message::InteractiveMessage> {
    base.interactive_message.as_option().or_else(|| {
        match &base.template_message.as_option()?.format {
            Some(wa::message::template_message::Format::InteractiveMessageTemplate(message)) => {
                Some(message.as_ref())
            }
            _ => None,
        }
    })
}

/// The library's normal image downloader also handles images inside templates.
/// Do not select an arbitrary carousel card: each card has a separate attachment.
pub(super) fn image(base: &wa::Message) -> Option<&wa::message::ImageMessage> {
    use wa::message::{
        buttons_message::Header, interactive_message::header::Media,
        template_message::hydrated_four_row_template::Title,
    };
    if let Some(message) = envelope(base)
        && let Some(header) = message.header.as_option()
        && let Some(Media::ImageMessage(image)) = &header.media
    {
        return Some(image);
    }
    if let Some(message) = base.buttons_message.as_option()
        && let Some(Header::ImageMessage(image)) = &message.header
    {
        return Some(image);
    }
    if let Some(template) = hydrated(base)
        && let Some(Title::ImageMessage(image)) = &template.title
    {
        return Some(image);
    }
    None
}

fn carousel_messages(base: &wa::Message) -> Option<&[wa::message::InteractiveMessage]> {
    if let Some(wa::message::interactive_message::InteractiveMessage::CarouselMessage(carousel)) =
        &envelope(base)?.interactive_message
    {
        Some(&carousel.cards)
    } else {
        None
    }
}

pub(super) fn image_at(
    base: &wa::Message,
    card: Option<usize>,
) -> Option<&wa::message::ImageMessage> {
    match card {
        None => image(base),
        Some(index) => {
            let child = carousel_messages(base)?.get(index)?;
            if let Some(wa::message::interactive_message::header::Media::ImageMessage(image)) =
                &child.header.as_option()?.media
            {
                Some(image)
            } else {
                None
            }
        }
    }
}

fn has_unrendered_content(base: &wa::Message) -> bool {
    use wa::message::interactive_message::InteractiveMessage as Payload;
    let image = image(base).is_some();
    if let Some(message) = envelope(base) {
        return message.header.as_option().is_some_and(|h| {
            ((h.media.is_some() || h.has_media_attachment == Some(true)) && !image)
                || h.bloks_widget.is_set()
        }) || message
            .footer
            .as_option()
            .is_some_and(|f| f.media.is_some())
            || message.bloks_widget.is_set()
            || matches!(
                message.interactive_message,
                Some(Payload::ShopStorefrontMessage(_) | Payload::CollectionMessage(_))
            );
    }
    if let Some(template) = hydrated(base) {
        return template.title.as_ref().is_some_and(|title| {
            !matches!(
                title,
                wa::message::template_message::hydrated_four_row_template::Title::HydratedTitleText(
                    _
                )
            )
        }) && !image;
    }
    base.buttons_message.as_option().is_some_and(|m| {
        m.header
            .as_ref()
            .is_some_and(|header| !matches!(header, wa::message::buttons_message::Header::Text(_)))
            && !image
    })
}

pub(super) fn context(base: &wa::Message) -> Option<&wa::ContextInfo> {
    base.interactive_message
        .as_option()
        .and_then(|m| m.context_info.as_option())
        .or_else(|| {
            base.buttons_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            base.list_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            base.template_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            base.buttons_response_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            base.list_response_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            base.interactive_response_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
        .or_else(|| {
            base.template_button_reply_message
                .as_option()
                .and_then(|m| m.context_info.as_option())
        })
}

impl Worker {
    pub(super) fn backfill_interactive(&mut self) {
        const KEY: &str = "interactive_text";
        if self.archive.meta(KEY).ok().flatten().as_deref() == Some("4") {
            return;
        }
        let result = (|| -> anyhow::Result<usize> {
            let mut updated = 0;
            for (chat, id, raw) in self.archive.interactive_placeholders()? {
                let Ok(message) = wa::Message::decode_from_slice(&raw) else {
                    continue;
                };
                let base = message.get_base_message();
                let Some(mut content) = classify(base) else {
                    continue;
                };
                if let Some(old) = self.archive.message(&chat, &id)? {
                    content.keep_local_paths(&old.content);
                }
                self.archive.set_derived(
                    &chat,
                    &id,
                    &content,
                    &self.mentions_of(&mentioned_of(base)),
                    image(base).and_then(|image| image.jpeg_thumbnail.as_deref()),
                    forwarded_of(base),
                )?;
                updated += 1;
            }
            self.archive.set_meta(KEY, "4")?;
            Ok(updated)
        })();
        match result {
            Ok(updated) if updated > 0 => {
                log::info!("recovered text for {updated} interactive messages");
                self.emit_chats();
            }
            Err(_) => log::warn!("could not recover interactive text; will retry at next startup"),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        MessageField,
        receipt_tests::{PEER, own_message, worker},
    };
    use super::*;

    fn readable(message: &wa::Message) -> String {
        let wire = message.encode_to_vec();
        let decoded = wa::Message::decode_from_slice(&wire).unwrap();
        match super::super::classify(decoded.get_base_message()) {
            Some(Content::Interactive { text, .. }) => text,
            other => panic!("unexpected classification: {other:?}"),
        }
    }

    fn buttons() -> wa::Message {
        use wa::message::buttons_message::{Button, Header, button::ButtonText};
        wa::Message {
            buttons_message: MessageField::some(wa::message::ButtonsMessage {
                header: Some(Header::Text("Order ready 📦".into())),
                content_text: Some("Collect *today*".into()),
                footer_text: Some("Thank you".into()),
                buttons: vec![Button {
                    button_id: Some("hidden-button-id".into()),
                    button_text: MessageField::some(ButtonText {
                        display_text: Some("View order".into()),
                    }),
                    ..Default::default()
                }],
                context_info: MessageField::some(wa::ContextInfo {
                    mentioned_jid: vec![PEER.into()],
                    is_forwarded: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn buttons_keep_display_text_and_context_without_internal_ids() {
        let message = buttons();
        assert_eq!(
            readable(&message),
            "Order ready 📦\n\nCollect *today*\n\nThank you\n\n• View order"
        );
        assert_eq!(super::super::mentioned_of(&message), vec![PEER]);
        assert!(super::super::forwarded_of(&message));
    }

    #[test]
    fn lists_keep_sections_and_descriptions() {
        use wa::message::list_message::{Row, Section};
        let message = wa::Message {
            list_message: MessageField::some(wa::message::ListMessage {
                title: Some("Pickup".into()),
                description: Some("Choose a time".into()),
                sections: vec![Section {
                    title: Some("Today".into()),
                    rows: vec![Row {
                        title: Some("Morning".into()),
                        description: Some("09:00 to 12:00".into()),
                        row_id: Some("hidden-row-id".into()),
                    }],
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            readable(&message),
            "Pickup\n\nChoose a time\n\nToday\n\n• Morning\n\n09:00 to 12:00"
        );
    }

    #[test]
    fn hydrated_templates_support_both_wire_locations_without_duplication() {
        use wa::hydrated_template_button::{HydratedButton, HydratedURLButton};
        use wa::message::template_message::{
            Format, HydratedFourRowTemplate, hydrated_four_row_template::Title,
        };
        let template = HydratedFourRowTemplate {
            title: Some(Title::HydratedTitleText("Appointment".into())),
            hydrated_content_text: Some("Tomorrow at 10:00".into()),
            hydrated_footer_text: Some("Clinic".into()),
            hydrated_buttons: vec![wa::HydratedTemplateButton {
                hydrated_button: Some(HydratedButton::UrlButton(Box::new(HydratedURLButton {
                    display_text: Some("Manage booking".into()),
                    url: Some("https://example.com/private-token".into()),
                    ..Default::default()
                }))),
                ..Default::default()
            }],
            ..Default::default()
        };
        for hydrated in [false, true] {
            let message = wa::Message {
                template_message: MessageField::some(wa::message::TemplateMessage {
                    hydrated_template: if hydrated {
                        MessageField::some(template.clone())
                    } else {
                        MessageField::none()
                    },
                    format: Some(Format::HydratedFourRowTemplate(Box::new(template.clone()))),
                    ..Default::default()
                }),
                ..Default::default()
            };
            assert_eq!(
                readable(&message),
                "Appointment\n\nTomorrow at 10:00\n\nClinic\n\n• Manage booking"
            );
        }
    }

    #[test]
    fn native_flow_and_carousel_text_survive_wrappers() {
        use wa::message::interactive_message::{
            Body, CarouselMessage, InteractiveMessage as Payload, NativeFlowMessage,
            native_flow_message::NativeFlowButton,
        };
        let card = wa::message::InteractiveMessage {
            body: MessageField::some(Body {
                text: Some("Choose an item 🛍️".into()),
            }),
            interactive_message: Some(Payload::NativeFlowMessage(Box::new(NativeFlowMessage {
                buttons: vec![NativeFlowButton {
                    name: Some("quick_reply".into()),
                    button_params_json: Some(
                        r#"{"display_text":"Select","id":"hidden","token":"secret"}"#.into(),
                    ),
                }],
                ..Default::default()
            }))),
            ..Default::default()
        };
        let message = wa::Message {
            view_once_message: MessageField::some(wa::message::FutureProofMessage {
                message: MessageField::some(wa::Message {
                    interactive_message: MessageField::some(wa::message::InteractiveMessage {
                        body: MessageField::some(Body {
                            text: Some("Catalog".into()),
                        }),
                        interactive_message: Some(Payload::CarouselMessage(Box::new(
                            CarouselMessage {
                                cards: vec![card],
                                ..Default::default()
                            },
                        ))),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        };
        assert_eq!(
            readable(&message),
            "Catalog\n\nChoose an item 🛍️\n\n• Select"
        );
    }

    #[test]
    fn interactive_replies_show_only_the_selected_display_text() {
        use wa::message::buttons_response_message::Response;
        let messages = [
            wa::Message {
                buttons_response_message: MessageField::some(wa::message::ButtonsResponseMessage {
                    response: Some(Response::SelectedDisplayText("Morning".into())),
                    selected_button_id: Some("hidden".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            wa::Message {
                list_response_message: MessageField::some(wa::message::ListResponseMessage {
                    title: Some("Morning".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            wa::Message {
                template_button_reply_message: MessageField::some(
                    wa::message::TemplateButtonReplyMessage {
                        selected_display_text: Some("Morning".into()),
                        selected_id: Some("hidden".into()),
                        ..Default::default()
                    },
                ),
                ..Default::default()
            },
            wa::Message {
                interactive_response_message: MessageField::some(
                    wa::message::InteractiveResponseMessage {
                        body: MessageField::some(wa::message::interactive_response_message::Body {
                            text: Some("Morning".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ),
                ..Default::default()
            },
        ];
        for message in messages {
            assert_eq!(readable(&message), "Morning");
        }
    }

    #[test]
    fn missing_or_unrenderable_text_has_a_phone_fallback() {
        let message = wa::Message {
            template_message: MessageField::some(wa::message::TemplateMessage {
                template_id: Some("hidden-template-id".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(readable(&message), "Interactive message");
        let mut text = Text::default();
        for json in [
            "{broken",
            r#"{"id":"hidden","token":"secret"}"#,
            r#"{"display_text":42}"#,
        ] {
            text.native_button(None, Some(json));
        }
        assert_eq!(
            text.finish(&wa::Message::default(), false),
            Content::Interactive {
                text: "Interactive message".into(),
                card: None
            }
        );
        assert_eq!(classify(&wa::Message::default()), None);
    }

    #[test]
    fn button_labels_are_separate_from_the_body_and_urls_are_checked() {
        let Some(Content::Interactive {
            text,
            card: Some(card),
        }) = classify(&buttons())
        else {
            panic!("card")
        };
        assert!(text.contains("• View order"));
        assert!(!card.body.contains("View order"));
        assert_eq!(card.buttons[0].label, "View order");
        assert_eq!(card.buttons[0].url, None);
        let mut text = Text::default();
        for (name, params) in [
            (
                "cta_url",
                r#"{"display_text":"Website","url":"https://example.com/"}"#,
            ),
            (
                "quick_reply",
                r#"{"display_text":"Reply","url":"https://example.com/"}"#,
            ),
            (
                "cta_url",
                r#"{"display_text":"Bad link","url":"javascript:alert(1)"}"#,
            ),
            (
                "cta_url",
                r#"{"display_text":"Local file","url":"file:///tmp/example"}"#,
            ),
        ] {
            text.native_button(Some(name), Some(params));
        }
        assert_eq!(text.buttons[0].url.as_deref(), Some("https://example.com/"));
        assert!(text.buttons[1..].iter().all(|button| button.url.is_none()));
        assert!(web_url("https://name:password@example.com").is_none());
    }

    #[test]
    fn a_finished_card_download_is_filed_under_that_card_only() {
        let (mut worker, events, _commands, _wa) = worker();
        worker.archive.ensure_chat(PEER, "Demo").unwrap();
        let card = || InteractiveCard {
            image: Some(super::super::media(
                Some(&"image/jpeg".to_owned()),
                Some(1),
                None,
                None,
            )),
            ..Default::default()
        };
        let mut carousel = own_message("carousel", 1);
        carousel.content = Content::Interactive {
            text: String::new(),
            card: Some(Box::new(InteractiveCard {
                carousel: vec![card(), card()],
                ..Default::default()
            })),
        };
        worker.archive.insert_message(&carousel, None).unwrap();
        let key = (PEER.to_owned(), "carousel".to_owned(), Some(1));
        worker.downloads.insert(key.clone());
        let path = std::path::PathBuf::from("/cache/zapfast/media/carousel-card-1.jpg");
        worker.downloaded(PEER.into(), "carousel".into(), Some(1), Ok(path.clone()));
        assert!(!worker.downloads.contains(&key));
        assert_eq!(
            worker.archive.carousel_media_paths().unwrap(),
            [(PEER.to_owned(), "carousel".to_owned(), 1, path)]
        );
        assert!(events.try_iter().any(|event| matches!(
            event,
            crate::backend::Event::Media {
                card: Some(1),
                result: Ok(_),
                ..
            }
        )));
    }

    #[test]
    fn nested_images_keep_the_library_download_metadata_and_archive_paths() {
        use wa::message::{
            buttons_message::Header,
            interactive_message::{Header as InteractiveHeader, header::Media},
            template_message::{HydratedFourRowTemplate, hydrated_four_row_template::Title},
        };
        let picture = wa::message::ImageMessage {
            mimetype: Some("image/jpeg".into()),
            width: Some(640),
            height: Some(480),
            file_length: Some(1234),
            media_key: Some(vec![7; 32]),
            direct_path: Some("/synthetic/image".into()),
            jpeg_thumbnail: Some(vec![1, 2, 3]),
            ..Default::default()
        };
        let messages = [
            wa::Message {
                buttons_message: MessageField::some(wa::message::ButtonsMessage {
                    header: Some(Header::ImageMessage(Box::new(picture.clone()))),
                    content_text: Some("Picture".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            wa::Message {
                interactive_message: MessageField::some(wa::message::InteractiveMessage {
                    header: MessageField::some(InteractiveHeader {
                        media: Some(Media::ImageMessage(Box::new(picture.clone()))),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            wa::Message {
                template_message: MessageField::some(wa::message::TemplateMessage {
                    hydrated_template: MessageField::some(HydratedFourRowTemplate {
                        title: Some(Title::ImageMessage(Box::new(picture.clone()))),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ];
        for base in messages {
            assert_eq!(image(&base), Some(&picture));
            assert_eq!(super::super::thumbnail_of(&base), picture.jpeg_thumbnail);
            let content = classify(&base).unwrap();
            assert_eq!(content.media().unwrap().width, Some(640));
            let (worker, _events, _commands, _wa) = worker();
            worker.archive.ensure_chat(PEER, "Demo").unwrap();
            let mut message = own_message("image", 123);
            message.content = content;
            let raw = base.encode_to_vec();
            worker.archive.insert_message(&message, Some(&raw)).unwrap();
            let path = std::path::Path::new("/tmp/synthetic-interactive.jpg");
            worker.archive.set_media_path(PEER, "image", path).unwrap();
            let paths = worker.archive.media_paths().unwrap();
            assert_eq!(
                paths,
                vec![(PEER.into(), "image".into(), path.to_path_buf())]
            );
            assert_eq!(worker.archive.raw(PEER, "image").unwrap(), Some(raw));
            worker.archive.clear_media_path(PEER, "image").unwrap();
            assert!(worker.archive.media_paths().unwrap().is_empty());
        }
    }

    #[test]
    fn previous_text_only_interactive_rows_upgrade_to_cards() {
        let (mut worker, _events, _commands, _wa) = worker();
        worker.archive.ensure_chat(PEER, "Demo").unwrap();
        let mut old = own_message("old-interactive", 123);
        old.content =
            serde_json::from_str(r#"{"kind":"interactive","text":"Order ready"}"#).unwrap();
        worker
            .archive
            .insert_message(&old, Some(&buttons().encode_to_vec()))
            .unwrap();
        worker.archive.set_meta("interactive_text", "1").unwrap();
        worker.backfill_interactive();
        assert!(matches!(
            worker
                .archive
                .message(PEER, &old.id)
                .unwrap()
                .unwrap()
                .content,
            Content::Interactive { card: Some(_), .. }
        ));
    }

    #[test]
    fn generic_backfill_keeps_edited_interactive_bodies() {
        let (mut worker, _events, _commands, _wa) = worker();
        worker.archive.ensure_chat(PEER, "Demo").unwrap();
        let mut edited = own_message("edited-interactive", 1);
        edited.content = Content::text("edited text");
        edited.edited = true;
        worker
            .archive
            .insert_message(&edited, Some(&buttons().encode_to_vec()))
            .unwrap();
        // An archive that never reached the current derived version.
        worker.backfill();
        worker.backfill_interactive();
        assert_eq!(
            worker
                .archive
                .message(PEER, "edited-interactive")
                .unwrap()
                .unwrap()
                .content,
            Content::text("edited text")
        );
    }

    #[test]
    fn backfill_recovers_search_and_preview_preserving_local_message_state() {
        let (mut worker, _events, _commands, _wa) = worker();
        worker.archive.ensure_chat(PEER, "Demo").unwrap();
        let raw = buttons().encode_to_vec();
        let mut placeholder = own_message("interactive", 123);
        placeholder.content = Content::Unsupported {
            what: "interactive message".into(),
        };
        placeholder.read_at = Some(456);
        placeholder.thumbnail = Some(vec![1, 2]);
        worker
            .archive
            .insert_message(&placeholder, Some(&raw))
            .unwrap();
        worker
            .archive
            .set_reaction(PEER, &placeholder.id, PEER, false, "👍")
            .unwrap();
        for (id, content) in [
            ("revoked", Content::Revoked),
            ("edited", Content::text("edited text")),
            (
                "other",
                Content::Unsupported {
                    what: "product".into(),
                },
            ),
        ] {
            let mut message = own_message(id, 1);
            message.content = content;
            worker.archive.insert_message(&message, Some(&raw)).unwrap();
        }
        let mut missing = own_message("no-raw", 1);
        missing.content = placeholder.content.clone();
        worker.archive.insert_message(&missing, None).unwrap();
        let mut edited_placeholder = placeholder.clone();
        edited_placeholder.id = "edited-placeholder".into();
        edited_placeholder.edited = true;
        edited_placeholder.timestamp = 1;
        worker
            .archive
            .insert_message(&edited_placeholder, Some(&raw))
            .unwrap();
        worker.archive.set_meta("derived", "3").unwrap();
        worker.backfill();
        worker.backfill_interactive();
        let recovered = worker
            .archive
            .message(PEER, "interactive")
            .unwrap()
            .unwrap();
        assert_eq!(recovered.content, classify(&buttons()).unwrap());
        assert_eq!(recovered.read_at, placeholder.read_at);
        assert_eq!(recovered.thumbnail, placeholder.thumbnail);
        assert_eq!(recovered.reactions[0].emoji, "👍");
        assert_eq!(worker.archive.raw(PEER, "interactive").unwrap(), Some(raw));
        assert_eq!(
            worker
                .archive
                .message(PEER, "revoked")
                .unwrap()
                .unwrap()
                .content,
            Content::Revoked
        );
        assert_eq!(
            worker
                .archive
                .message(PEER, "edited")
                .unwrap()
                .unwrap()
                .content,
            Content::text("edited text")
        );
        assert_eq!(
            worker
                .archive
                .message(PEER, "no-raw")
                .unwrap()
                .unwrap()
                .content,
            missing.content
        );
        assert_eq!(
            worker
                .archive
                .message(PEER, "edited-placeholder")
                .unwrap()
                .unwrap()
                .content,
            placeholder.content
        );
        assert!(matches!(
            worker
                .archive
                .message(PEER, "other")
                .unwrap()
                .unwrap()
                .content,
            Content::Unsupported { .. }
        ));
        assert_eq!(
            worker.archive.search_messages("Collect", 10).unwrap().len(),
            1
        );
        assert_eq!(
            worker.archive.chats().unwrap()[0]
                .last
                .as_ref()
                .unwrap()
                .summary,
            "Order ready 📦"
        );
        // A second startup must leave even a subsequently edited body intact.
        worker
            .archive
            .set_derived(
                PEER,
                "interactive",
                &Content::text("changed"),
                &[],
                None,
                false,
            )
            .unwrap();
        worker.backfill_interactive();
        assert_eq!(
            worker
                .archive
                .message(PEER, "interactive")
                .unwrap()
                .unwrap()
                .content,
            Content::text("changed")
        );
    }
    #[test]
    fn carousel_cards_keep_their_own_actions_images_and_cached_paths() {
        use wa::message::interactive_message::{
            Body, CarouselMessage, Header, InteractiveMessage as Payload, NativeFlowMessage,
            header::Media as HeaderMedia, native_flow_message::NativeFlowButton,
        };
        let cards = (0..2).map(|index| wa::message::InteractiveMessage {
            body: MessageField::some(Body { text: Some(format!("Card {index}")) }),
            header: MessageField::some(Header {
                media: Some(HeaderMedia::ImageMessage(Box::new(wa::message::ImageMessage {
                    direct_path: Some(format!("/image-{index}")), media_key: Some(vec![index as u8; 32]), jpeg_thumbnail: Some(vec![index as u8]), ..Default::default()
                }))), ..Default::default()
            }),
            interactive_message: Some(Payload::NativeFlowMessage(Box::new(NativeFlowMessage {
                buttons: vec![
                    NativeFlowButton { name: Some("cta_copy".into()), button_params_json: Some(format!(r#"{{"display_text":"Copy","copy_code":"CODE{index}"}}"#)) },
                    NativeFlowButton { name: Some("cta_url".into()), button_params_json: Some(format!(r#"{{"display_text":"Open","url":"https://example.com/{index}"}}"#)) },
                    NativeFlowButton { name: Some("quick_reply".into()), button_params_json: Some(r#"{"display_text":"Reply","id":"hidden"}"#.into()) },
                ], ..Default::default()
            }))), ..Default::default()
        }).collect();
        let raw = wa::Message {
            interactive_message: MessageField::some(wa::message::InteractiveMessage {
                body: MessageField::some(Body {
                    text: Some("Collection".into()),
                }),
                interactive_message: Some(Payload::CarouselMessage(Box::new(CarouselMessage {
                    cards,
                    ..Default::default()
                }))),
                ..Default::default()
            }),
            ..Default::default()
        };
        let content = classify(&raw).unwrap();
        let Content::Interactive {
            card: Some(card), ..
        } = &content
        else {
            panic!("card")
        };
        assert_eq!(card.body, "Collection");
        assert!(!card.needs_phone);
        assert_eq!(card.carousel.len(), 2);
        assert!(image_at(&raw, Some(2)).is_none());
        for (index, child) in card.carousel.iter().enumerate() {
            assert_eq!(child.body, format!("Card {index}"));
            assert_eq!(
                child.buttons[0].action,
                InteractiveAction::Copy(format!("CODE{index}"))
            );
            assert_eq!(
                child.buttons[1].url,
                Some(format!("https://example.com/{index}"))
            );
            assert_eq!(child.buttons[2].action, InteractiveAction::Unavailable);
            assert_eq!(
                image_at(&raw, Some(index)).unwrap().direct_path,
                Some(format!("/image-{index}"))
            );
            assert_eq!(child.thumbnail, Some(vec![index as u8]));
        }
        let (mut worker, _, _, _) = worker();
        worker.archive.ensure_chat(PEER, "Demo").unwrap();
        let mut row = own_message("carousel", 123);
        row.content = content;
        worker
            .archive
            .insert_message(&row, Some(&raw.encode_to_vec()))
            .unwrap();
        for index in 0..2 {
            worker
                .archive
                .put_media_path_at(
                    PEER,
                    &row.id,
                    Some(index),
                    Some(std::path::Path::new(&format!("/tmp/card-{index}.jpg"))),
                )
                .unwrap();
        }
        assert_eq!(worker.archive.carousel_media_paths().unwrap().len(), 2);
        worker.archive.set_meta("interactive_text", "3").unwrap();
        worker.backfill_interactive();
        let mut row = worker.archive.message(PEER, &row.id).unwrap().unwrap();
        assert_eq!(
            row.content.media_at_mut(Some(1)).unwrap().path.as_deref(),
            Some(std::path::Path::new("/tmp/card-1.jpg"))
        );
        assert!(
            worker
                .archive
                .put_media_path_at(PEER, &row.id, Some(9), None)
                .unwrap()
                .is_none()
        );
        worker
            .archive
            .put_media_path_at(PEER, &row.id, Some(0), None)
            .unwrap();
        let paths = worker.archive.carousel_media_paths().unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].2, 1);
    }

    #[test]
    fn list_choices_belong_to_the_menu_not_the_message_body() {
        let raw = wa::Message {
            list_message: MessageField::some(wa::message::ListMessage {
                title: Some("Workshop sessions".into()),
                description: Some("Choose a session".into()),
                button_text: Some("Browse".into()),
                list_type: Some(wa::message::list_message::ListType::SINGLE_SELECT),
                sections: vec![wa::message::list_message::Section {
                    title: Some("Morning".into()),
                    rows: vec![wa::message::list_message::Row {
                        title: Some("Drawing".into()),
                        description: Some("Materials included".into()),
                        row_id: Some("private".into()),
                    }],
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let Content::Interactive {
            text,
            card: Some(card),
        } = classify(&raw).unwrap()
        else {
            panic!("card")
        };
        assert_eq!(card.body, "Workshop sessions\n\nChoose a session");
        assert!(text.contains("Drawing"));
        assert!(!text.contains("private"));
        assert!(
            matches!(&card.buttons[0].action, InteractiveAction::Select(options) if options[0].description == "Materials included")
        );
    }
}
