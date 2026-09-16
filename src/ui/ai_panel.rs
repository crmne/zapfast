//! Chat-scoped, local-preview-first assistant. Views only queue actions.
use super::widgets;
use crate::{
    app::App,
    model::{Action, Page},
    theme::{self, Icon},
};
use egui::{Align, Frame, Layout, Margin};

pub fn visible(app: &App) -> bool {
    app.ai.open && app.page == Page::Chats && app.open_chat.is_some()
}

/// Preserve a usable conversation at compact widths by temporarily hiding the list.
pub fn hides_sidebar(app: &App, width: f32) -> bool {
    visible(app) && width < 1160.0
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let available = ui.available_width();
    let compact = available < 760.0;
    let width = if compact {
        (available * 0.48).clamp(240.0, 360.0)
    } else {
        380.0
    };
    let palette = app.palette;
    egui::Panel::right("ai-assistant")
        .default_size(width)
        .size_range(if compact {
            240.0..=360.0
        } else {
            360.0..=420.0
        })
        .resizable(!compact)
        .frame(
            Frame::new()
                .fill(palette.panel)
                .inner_margin(Margin::same(14)),
        )
        .show(ui, |ui| contents(app, ui));
}

fn text(ui: &mut egui::Ui, value: &str, color: egui::Color32) {
    let line = widgets::line(
        ui,
        value,
        theme::regular(13.5),
        color,
        ui.available_width(),
        usize::MAX,
    );
    let (rect, _) = ui.allocate_exact_size(line.size(), egui::Sense::hover());
    line.paint(ui, rect.min, color);
}

fn contents(app: &mut App, ui: &mut egui::Ui) {
    if app.ai.reply_target.is_some() {
        reply_contents(app, ui);
        return;
    }
    let palette = app.palette;
    let title = app
        .current_chat()
        .map(|chat| app.chat_title(chat))
        .unwrap_or_default();
    ui.horizontal(|ui| {
        theme::icon(ui, Icon::Sparkles, 19.0, palette.accent);
        theme::text(ui, "AI assistant", theme::semibold(16.0), palette.text);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if theme::icon_button(
                ui,
                Icon::X,
                17.0,
                palette.secondary,
                palette.text,
                "Close AI assistant",
            )
            .clicked()
            {
                app.actions.push(Action::AiClose);
            }
        });
    });
    widgets::rich_text(ui, &title, theme::medium(13.0), palette.secondary);
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        let count = app.ai.preview.as_ref().map_or(0, |p| p.count);
        theme::text(
            ui,
            format!("{count} messages"),
            theme::medium(12.0),
            palette.secondary,
        );
        if theme::icon_button(
            ui,
            Icon::Refresh,
            14.0,
            palette.secondary,
            palette.text,
            "Refresh local context (clears AI history)",
        )
        .clicked()
            && !app.ai.pending
        {
            app.actions.push(Action::AiPreview(app.ai.scope));
        }
        for scope in crate::ai::SCOPES {
            if ui
                .add_enabled(
                    !app.ai.pending,
                    egui::Button::new(scope.to_string()).selected(app.ai.scope == scope),
                )
                .clicked()
            {
                app.actions.push(Action::AiPreview(scope));
            }
        }
    });
    if app.ai.preview_pending {
        text(ui, "Reading local archive…", palette.secondary);
    }
    egui::CollapsingHeader::new("Review context")
        .id_salt("ai-review-context")
        .default_open(app.ai.review_context)
        .show(ui, |ui| {
            text(ui, "Local messages only · sender pseudonyms · attachments omitted", palette.secondary);
            egui::ScrollArea::vertical().id_salt("ai-context").max_height(130.0).show(ui, |ui| {
                if let Some(preview) = &app.ai.preview {
                    text(ui, &preview.transcript.0, palette.secondary);
                    if preview.truncated { text(ui, "Oversized text was truncated (marked above).", palette.warning); }
                }
            });
            text(ui, "Text can still contain private details. Provider policies and charges apply. AI may be inaccurate.", palette.secondary);
        });
    ui.separator();
    // Footer is laid out first so long context and responses never push Send off screen.
    egui::Panel::bottom("ai-question-footer")
        .frame(
            Frame::new()
                .fill(palette.panel)
                .inner_margin(Margin::symmetric(0, 8)),
        )
        .show(ui, |ui| {
            provider_disclosure(app, ui);
            if app.backend.is_offline() {
                text(ui, "Offline demo · sample responses only", palette.warning);
            }
            if !app.settings.ai.enabled {
                text(
                    ui,
                    "AI assistant is disabled. Enable it in AI settings.",
                    palette.secondary,
                );
                if theme::soft_button(ui, &palette, Some(Icon::Settings), "AI settings", false)
                    .clicked()
                {
                    app.actions.push(Action::Open(Page::Settings));
                }
            }
            ui.add_space(8.0);
            composer(app, ui);
            if app.ai.question.len() > crate::ai::MAX_QUESTION {
                text(
                    ui,
                    "Shorten your question to 2,000 UTF-8 bytes.",
                    palette.warning,
                );
            }
            ui.add_space(6.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new("Enter to send · Shift+Enter for newline")
                        .font(theme::regular(11.0))
                        .color(palette.secondary),
                )
                .wrap(),
            );
        });
    egui::ScrollArea::vertical()
        .id_salt("ai-turns")
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if app.ai.history.is_empty() && app.ai.answer.is_none() && !app.ai.pending {
                ui.add_space(14.0);
                theme::text(
                    ui,
                    "Make sense of this chat",
                    theme::semibold(15.0),
                    palette.text,
                );
                text(
                    ui,
                    "Ask a question using the messages above. Only Send contacts your provider.",
                    palette.secondary,
                );
                ui.add_space(10.0);
                for (label, question) in [
                    ("Summarize", "Summarize this conversation."),
                    (
                        "Decisions & next steps",
                        "What did we decide, and what still needs doing?",
                    ),
                    (
                        "Help me reply",
                        "Suggest a reply to the latest message. Do not send it.",
                    ),
                ] {
                    if theme::soft_button(ui, &palette, None, label, false).clicked() {
                        app.ai.question = question.into();
                    }
                }
                if app.ai.preview.as_ref().is_some_and(|p| p.count == 0) {
                    text(
                        ui,
                        "No messages are stored locally for this chat yet.",
                        palette.secondary,
                    );
                }
            }
            for turn in &app.ai.history {
                turn_text(ui, &turn.question.0, true, palette);
                turn_text(ui, &turn.assistant.0, false, palette);
                if theme::icon_button(
                    ui,
                    Icon::Copy,
                    14.0,
                    palette.secondary,
                    palette.text,
                    "Copy answer",
                )
                .clicked()
                {
                    app.actions.push(Action::CopyText(turn.assistant.0.clone()));
                }
                ui.add_space(8.0);
            }
            if !app.ai.active_question.0.is_empty() {
                turn_text(ui, &app.ai.active_question.0, true, palette);
            }
            if let Some(answer) = &app.ai.answer {
                turn_text(ui, &answer.0, false, palette);
            }
            if let Some(error) = &app.ai.error {
                ui.add_space(8.0);
                text(ui, error, palette.warning);
            }
        });
}

fn provider_disclosure(app: &App, ui: &mut egui::Ui) {
    let endpoint = app.ai.config.endpoint().ok();
    let provider = endpoint
        .as_ref()
        .and_then(|url| url.host_str())
        .unwrap_or("Provider not configured");
    ui.add(egui::Label::new(
        egui::RichText::new(format!("Context shared with {provider}"))
            .font(theme::regular(11.5)).color(app.palette.secondary),
    ).truncate()).on_hover_text(format!(
        "Send shares your question, reviewed context and prior AI turns with:\n{}\nNever sends to WhatsApp.",
        endpoint.as_ref().map_or("Configure a provider in AI settings.", |url| url.as_str()),
    ));
}

fn submission_ready(app: &App) -> bool {
    app.settings.ai.enabled
        && !app.ai.pending
        && !app.ai.preview_pending
        && !app.ai.question.trim().is_empty()
        && app.ai.question.len() <= crate::ai::MAX_QUESTION
        && app.ai.preview.as_ref().is_some_and(|p| p.count > 0)
        && !app.backend.is_offline()
}

/// Remove send keys before TextEdit sees them, including repeats and blocked sends.
/// IME confirmation is never a submission, including commit + Enter in one frame.
fn take_submit_key(ui: &mut egui::Ui) -> bool {
    let id = egui::Id::new("ai-question");
    let ime_id = id.with("composing");
    if !ui.memory(|m| m.has_focus(id)) {
        ui.data_mut(|d| d.remove::<bool>(ime_id));
        return false;
    }
    let mut composing = ui.data_mut(|d| d.get_temp::<bool>(ime_id).unwrap_or(false));
    let mut ime_frame = composing;
    ui.input(|i| {
        for event in &i.events {
            if let egui::Event::Ime(event) = event {
                ime_frame = true;
                match event {
                    egui::ImeEvent::Preedit { text, .. } => composing = !text.is_empty(),
                    egui::ImeEvent::Commit(_) => composing = false,
                    _ => {}
                }
            }
        }
    });
    ui.data_mut(|d| d.insert_temp(ime_id, composing));
    let mut submit = false;
    ui.input_mut(|i| {
        i.events.retain(|event| {
            if let egui::Event::Key {
                key: egui::Key::Enter,
                pressed: true,
                repeat,
                modifiers,
                ..
            } = event
                && !modifiers.shift
                && !modifiers.alt
            {
                // No modifier is plain Enter; Ctrl/Cmd+Enter remain compatible.
                submit |= !repeat && !ime_frame;
                false
            } else {
                true
            }
        })
    });
    submit
}

fn composer(app: &mut App, ui: &mut egui::Ui) {
    let p = app.palette;
    let submit_key = take_submit_key(ui);
    let focused = ui.memory(|m| m.has_focus(egui::Id::new("ai-question")));
    let mut clicked = false;
    Frame::new()
        .fill(p.surface)
        .corner_radius(14)
        .stroke(egui::Stroke::new(
            1.0,
            if focused { p.accent } else { p.outline },
        ))
        .inner_margin(12)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("ai-question-scroll")
                .max_height(88.0)
                .show(ui, |ui| {
                    ui.add_enabled_ui(!app.ai.pending, |ui| {
                        question_editor(ui, &mut app.ai.question, p);
                    });
                });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                // Allocate the action first, keeping its location stable in every state.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let ready = submission_ready(app);
                    let label = if app.ai.pending {
                        "Stop"
                    } else if app.ai.error.is_some() {
                        "Retry"
                    } else {
                        "Send"
                    };
                    let response = ui
                        .add_enabled_ui(ready || app.ai.pending, |ui| {
                            let response = theme::circle_button(
                                ui,
                                if app.ai.pending { Icon::X } else { Icon::Send },
                                36.0,
                                if ready || app.ai.pending {
                                    p.accent
                                } else {
                                    p.surface_active
                                },
                                p.accent_hover,
                                if ready || app.ai.pending {
                                    p.on_accent
                                } else {
                                    p.secondary
                                },
                                label,
                            );
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    ui.is_enabled(),
                                    label,
                                )
                            });
                            if response.has_focus() {
                                ui.painter().circle_stroke(
                                    response.rect.center(),
                                    20.0,
                                    egui::Stroke::new(1.0, p.accent),
                                );
                            }
                            response
                        })
                        .inner;
                    if response.clicked() {
                        if app.ai.pending {
                            app.actions.push(Action::AiStop);
                        } else {
                            clicked = true;
                        }
                    }
                    #[cfg(test)]
                    ui.data_mut(|d| {
                        d.insert_temp(
                            egui::Id::new("ai-submit-bounds"),
                            (response.rect, ui.clip_rect()),
                        )
                    });
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        if app.ai.pending {
                            ui.spinner();
                            text(
                                ui,
                                if app.ai.answer.is_some() {
                                    "Receiving reply…"
                                } else if app.ai.config.keyless {
                                    "Waiting for provider…"
                                } else {
                                    "Reading OS key / connecting…"
                                },
                                p.secondary,
                            );
                        } else {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new("Never sends to WhatsApp")
                                        .font(theme::regular(11.0))
                                        .color(p.secondary),
                                )
                                .wrap(),
                            );
                        }
                    });
                });
            });
        });
    if (submit_key || clicked)
        && submission_ready(app)
        && !app
            .actions
            .iter()
            .any(|action| matches!(action, Action::AiSubmit))
    {
        app.actions.push(Action::AiSubmit);
    }
}

fn reply_contents(app: &mut App, ui: &mut egui::Ui) {
    let p = app.palette;
    let chat = app.ai.chat.clone().unwrap_or_default();
    let message = app.ai.reply_target.clone().unwrap_or_default();
    let generation = app.ai.generation;
    let busy = app.ai.pending || app.ai.preview_pending;
    let provider = app
        .ai
        .config
        .endpoint()
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| "provider not configured".into());
    ui.horizontal(|ui| {
        theme::icon(ui, Icon::Sparkles, 19.0, p.accent);
        theme::text(ui, "AI reply", theme::semibold(16.0), p.text);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if theme::icon_button(ui, Icon::X, 17.0, p.secondary, p.text, "Close AI reply")
                .clicked()
            {
                app.actions.push(Action::AiClose);
            }
        });
    });
    let count = app.ai.preview.as_ref().map_or(0, |preview| preview.count);
    text(ui, &format!("{provider} · {count} messages"), p.secondary);
    ui.add_space(8.0);

    // Controls reserve their own space before any long context, choice or error text.
    egui::Panel::bottom("ai-reply-footer")
        .frame(
            Frame::new()
                .fill(p.panel)
                .inner_margin(Margin::symmetric(0, 8)),
        )
        .show(ui, |ui| {
            if let Some(choice) = app.ai.replace_choice {
                text(
                    ui,
                    "Replace your current draft and attachments with this reply?",
                    p.warning,
                );
                ui.horizontal_wrapped(|ui| {
                    if theme::soft_button(ui, &p, None, "Keep draft", false).clicked() {
                        app.actions.push(Action::AiKeepDraft);
                    }
                    if theme::soft_button(ui, &p, None, "Replace draft", true).clicked() {
                        app.actions.push(Action::AiChooseReply {
                            generation,
                            chat: chat.clone(),
                            message: message.clone(),
                            choice,
                            replace: true,
                        });
                    }
                });
            } else if busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    text(
                        ui,
                        if app.ai.preview_pending {
                            "Reading local context…"
                        } else {
                            "Writing three reply options…"
                        },
                        p.secondary,
                    );
                });
                if theme::soft_button(ui, &p, Some(Icon::X), "Stop", false).clicked() {
                    app.actions.push(Action::AiStop);
                }
            } else if !app.settings.ai.enabled {
                if theme::soft_button(ui, &p, Some(Icon::Settings), "AI settings", false).clicked()
                {
                    app.actions.push(Action::Open(Page::Settings));
                }
            } else {
                ui.horizontal_wrapped(|ui| {
                    if theme::soft_button(ui, &p, Some(Icon::Refresh), "Generate again", false)
                        .clicked()
                    {
                        app.actions.push(Action::AiReply {
                            chat: chat.clone(),
                            message: message.clone(),
                        });
                    }
                    if theme::soft_button(ui, &p, Some(Icon::Settings), "AI settings", false)
                        .clicked()
                    {
                        app.actions.push(Action::Open(Page::Settings));
                    }
                });
            }
            ui.add_space(5.0);
            text(ui, "Choose a reply to edit. Never auto-sends.", p.secondary);
        });
    egui::ScrollArea::vertical().id_salt("ai-reply-choices")
        .auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            text(ui, "Replies to the selected message, using up to 25 local messages at or before it.", p.secondary);
            egui::CollapsingHeader::new("Review shared context").id_salt("ai-reply-context").show(ui, |ui| {
                text(ui, "Sender pseudonyms · attachments omitted. Text may contain private details. AI may be inaccurate; provider policies and charges apply.", p.secondary);
                if let Some(preview) = &app.ai.preview {
                    text(ui, &preview.transcript.0, p.text);
                    if preview.truncated { text(ui, "Oversized text was truncated or omitted.", p.warning); }
                }
            });
            if app.backend.is_offline() { text(ui, "Offline demo · sample replies only", p.warning); }
            if let Some(error) = &app.ai.error {
                ui.add_space(10.0);
                text(ui, error, p.warning);
            }
            for (choice, reply) in app.ai.replies.iter().enumerate() {
                ui.add_space(12.0);
                Frame::new().fill(p.surface).corner_radius(10).inner_margin(12).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    theme::text(ui, format!("Option {}", choice + 1), theme::semibold(12.0), p.secondary);
                    ui.add_space(6.0);
                    text(ui, &reply.0, p.text);
                    ui.add_space(10.0);
                    ui.horizontal_wrapped(|ui| {
                        if theme::soft_button(ui, &p, Some(Icon::Reply), "Use reply", true).clicked() {
                            app.actions.push(Action::AiChooseReply {
                                generation, chat: chat.clone(), message: message.clone(), choice, replace: false,
                            });
                        }
                        if theme::soft_button(ui, &p, Some(Icon::Copy), "Copy", false).clicked() {
                            app.actions.push(Action::CopyText(reply.0.clone()));
                        }
                    });
                });
            }
        });
}

fn turn_text(ui: &mut egui::Ui, value: &str, own: bool, p: theme::Palette) {
    ui.add_space(12.0);
    let width = ui.available_width();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        if own {
            ui.add_space(24.0);
        }
        Frame::new()
            .fill(if own { p.bubble_out } else { p.surface })
            .corner_radius(12)
            .inner_margin(12)
            .show(ui, |ui| {
                ui.set_width((width - 48.0).max(40.0));
                text(ui, value, p.text);
            });
    });
}

fn question_editor(ui: &mut egui::Ui, question: &mut String, palette: theme::Palette) {
    let format = egui::TextFormat::simple(theme::regular(14.0), palette.text);
    let mut clusters = Vec::new();
    let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
        let (mut job, found) = crate::emoji::editor_job(text.as_str(), &format);
        job.wrap.max_width = wrap;
        clusters = found;
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    let output = egui::TextEdit::multiline(question)
        .id(egui::Id::new("ai-question"))
        .hint_text(
            egui::RichText::new("Ask about this chat…")
                .font(theme::regular(14.0))
                .color(palette.secondary),
        )
        .frame(Frame::NONE)
        .margin(0)
        .desired_width(f32::INFINITY)
        .desired_rows(2)
        .char_limit(crate::ai::MAX_QUESTION)
        .layouter(&mut layouter)
        .show(ui);
    for (start, length, cluster) in clusters {
        let left = output
            .galley
            .pos_from_cursor(egui::text::CCursor::new(start));
        let right = output
            .galley
            .pos_from_cursor(egui::text::CCursor::new(start + length));
        if (left.top() - right.top()).abs() < 1.0 {
            let rect =
                egui::Rect::from_min_max(left.left_top(), egui::pos2(right.left(), left.bottom()))
                    .translate(output.galley_pos.to_vec2());
            crate::emoji::paint_cluster(ui, &cluster, rect);
        }
    }
}

#[cfg(all(test, feature = "demo"))]
mod tests {
    use super::*;
    fn editor_app() -> (App, egui::Context) {
        let (mut app, _) = App::headless(
            crate::paths::AppDirs::under(&std::env::temp_dir().join("zapfast-ai-editor-test")),
            crate::settings::Settings::default(),
        );
        crate::demo::populate(&mut app);
        crate::demo::apply_flags(&mut app, Some("ai-preview"));
        // Disconnected handle only: no worker, credentials, or action processing.
        app.backend.set_offline(false);
        app.ai.question = "A question".into();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        editor_frame(&mut app, &ctx, Vec::new(), None);
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("ai-question")));
        (app, ctx)
    }

    fn editor_frame(
        app: &mut App,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        other: Option<&mut String>,
    ) {
        let mut other = other;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(380.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                // Match production ordering: global keys run before panel inputs.
                crate::ui::keys::handle(app, ui.ctx());
                composer(app, ui);
                if let Some(other) = other.as_deref_mut() {
                    ui.add(egui::TextEdit::multiline(other).id(egui::Id::new("other-editor")));
                }
            },
        );
        output.textures_delta.clear();
    }

    fn enter(modifiers: egui::Modifiers, repeat: bool) -> egui::Event {
        egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers,
        }
    }

    #[test]
    fn enter_submits_once_without_newline_and_ignores_repeats() {
        for modifiers in [
            egui::Modifiers::NONE,
            egui::Modifiers::CTRL,
            egui::Modifiers::COMMAND,
        ] {
            let (mut app, ctx) = editor_app();
            assert!(submission_ready(&app));
            editor_frame(
                &mut app,
                &ctx,
                vec![enter(modifiers, false), enter(modifiers, true)],
                None,
            );
            assert_eq!(app.ai.question, "A question");
            assert_eq!(
                app.actions
                    .iter()
                    .filter(|a| matches!(a, Action::AiSubmit))
                    .count(),
                1
            );
            app.actions.clear();
            editor_frame(&mut app, &ctx, vec![enter(modifiers, true)], None);
            assert!(app.actions.is_empty());
            assert_eq!(app.ai.question, "A question");
        }
    }

    #[test]
    fn assistant_enter_does_not_send_a_whatsapp_recording() {
        let (mut app, ctx) = editor_app();
        app.recording = Some(crate::audio::Recorder::rehearsal());
        editor_frame(
            &mut app,
            &ctx,
            vec![enter(egui::Modifiers::NONE, false)],
            None,
        );
        assert_eq!(app.actions, vec![Action::AiSubmit]);
        assert_eq!(app.ai.question, "A question");
        app.actions.clear();
        let mut other = String::from("Other");
        editor_frame(&mut app, &ctx, vec![], Some(&mut other));
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("other-editor")));
        editor_frame(
            &mut app,
            &ctx,
            vec![enter(egui::Modifiers::NONE, false)],
            Some(&mut other),
        );
        assert_eq!(app.actions, vec![Action::SendRecording]);
    }

    #[test]
    fn shift_enter_inserts_newline_and_alt_enter_never_submits() {
        let (mut app, ctx) = editor_app();
        editor_frame(
            &mut app,
            &ctx,
            vec![enter(egui::Modifiers::SHIFT, false)],
            None,
        );
        assert_eq!(app.ai.question.matches('\n').count(), 1);
        assert!(app.actions.is_empty());
        editor_frame(
            &mut app,
            &ctx,
            vec![enter(egui::Modifiers::ALT, false)],
            None,
        );
        assert!(app.actions.is_empty());
    }

    #[test]
    fn enter_in_other_editor_is_untouched() {
        let (mut app, ctx) = editor_app();
        let mut other = String::from("Other");
        editor_frame(&mut app, &ctx, vec![], Some(&mut other));
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("other-editor")));
        editor_frame(
            &mut app,
            &ctx,
            vec![enter(egui::Modifiers::NONE, false)],
            Some(&mut other),
        );
        assert!(other.contains('\n'));
        assert_eq!(app.ai.question, "A question");
        assert!(app.actions.is_empty());
    }

    #[test]
    fn unavailable_enter_neither_submits_nor_inserts_newline() {
        for state in [
            "disabled",
            "pending",
            "preview",
            "empty",
            "oversized",
            "no-context",
            "offline",
        ] {
            let (mut app, ctx) = editor_app();
            match state {
                "disabled" => app.settings.ai.enabled = false,
                "pending" => app.ai.pending = true,
                "preview" => app.ai.preview_pending = true,
                "empty" => app.ai.question = " ".into(),
                "oversized" => app.ai.question = "a".repeat(crate::ai::MAX_QUESTION + 1),
                "no-context" => app.ai.preview = None,
                "offline" => app.backend.set_offline(true),
                _ => unreachable!(),
            }
            let before = app.ai.question.clone();
            assert!(!submission_ready(&app));
            editor_frame(
                &mut app,
                &ctx,
                vec![enter(egui::Modifiers::NONE, false)],
                None,
            );
            assert!(app.actions.is_empty(), "{state}");
            assert_eq!(app.ai.question, before, "{state}");
        }
    }

    #[test]
    fn same_control_sends_when_ready_and_stops_when_streaming() {
        for pending in [false, true] {
            let (mut app, ctx) = editor_app();
            app.ai.pending = pending;
            editor_frame(&mut app, &ctx, vec![], None);
            let (rect, _) = ctx
                .data_mut(|d| {
                    d.get_temp::<(egui::Rect, egui::Rect)>(egui::Id::new("ai-submit-bounds"))
                })
                .unwrap();
            for pressed in [true, false] {
                editor_frame(
                    &mut app,
                    &ctx,
                    vec![
                        egui::Event::PointerMoved(rect.center()),
                        egui::Event::PointerButton {
                            pos: rect.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    None,
                );
            }
            assert_eq!(app.actions.len(), 1);
            assert!(matches!(
                (&app.actions[0], pending),
                (Action::AiStop, true) | (Action::AiSubmit, false)
            ));
        }
    }

    #[test]
    fn ime_confirmation_does_not_submit() {
        let (mut app, ctx) = editor_app();
        editor_frame(
            &mut app,
            &ctx,
            vec![egui::Event::Ime(egui::ImeEvent::Preedit {
                text: "compose".into(),
                active_range_chars: None,
            })],
            None,
        );
        editor_frame(
            &mut app,
            &ctx,
            vec![enter(egui::Modifiers::NONE, false)],
            None,
        );
        assert!(app.actions.is_empty());
        editor_frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::Ime(egui::ImeEvent::Commit("composed".into())),
                enter(egui::Modifiers::NONE, false),
            ],
            None,
        );
        assert!(app.actions.is_empty());
        assert!(!app.ai.question.contains('\n'));
    }

    #[test]
    fn ai_reply_footer_survives_long_choices_and_loading() {
        for state in ["ai-replies", "ai-reply-loading", "replace"] {
            for size in [
                egui::vec2(1280.0, 800.0),
                egui::vec2(900.0, 600.0),
                egui::vec2(720.0, 600.0),
            ] {
                let root = std::env::temp_dir().join("zapfast-ai-reply-layout");
                let (mut app, _) = App::headless(
                    crate::paths::AppDirs::under(&root),
                    crate::settings::Settings::default(),
                );
                crate::demo::populate(&mut app);
                crate::demo::apply_flags(
                    &mut app,
                    Some(if state == "replace" {
                        "ai-replies"
                    } else {
                        state
                    }),
                );
                if state == "replace" {
                    app.ai.replace_choice = Some(0);
                }
                if state != "ai-reply-loading" {
                    app.ai.replies[0].0 =
                        "A realistic longer reply, with emoji ☕ and details to review. "
                            .repeat(20);
                }
                let ctx = egui::Context::default();
                app.attach(&ctx);
                let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
                for frame in 0..4 {
                    let mut output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(screen),
                            ..Default::default()
                        },
                        |ui| crate::ui::show(&mut app, ui),
                    );
                    if frame == 3 {
                        let expected = if state == "replace" {
                            "Replace draft"
                        } else if state == "ai-reply-loading" {
                            "Stop"
                        } else {
                            "Generate again"
                        };
                        let (rect, clip) = output
                            .shapes
                            .iter()
                            .find_map(|shape| {
                                if let egui::epaint::Shape::Text(text) = &shape.shape
                                    && text.galley.job.text == expected
                                {
                                    Some((
                                        text.galley.rect.translate(text.pos.to_vec2()),
                                        shape.clip_rect,
                                    ))
                                } else {
                                    None
                                }
                            })
                            .expect("reply footer control is drawn");
                        assert!(
                            screen.contains_rect(rect),
                            "footer outside screen {state} {rect:?}"
                        );
                        assert!(
                            clip.contains_rect(rect),
                            "footer clipped {state} {rect:?} {clip:?}"
                        );
                    }
                    output.textures_delta.clear();
                }
            }
        }
    }

    #[test]
    fn panel_footer_survives_long_context_and_answers() {
        for size in [
            egui::vec2(1280.0, 800.0),
            egui::vec2(900.0, 600.0),
            egui::vec2(720.0, 600.0),
        ] {
            let root = std::env::temp_dir().join("zapfast-ai-panel-layout");
            let (mut app, _) = App::headless(
                crate::paths::AppDirs::under(&root),
                crate::settings::Settings::default(),
            );
            crate::demo::populate(&mut app);
            crate::demo::apply_flags(&mut app, Some("ai-preview"));
            app.ai.preview.as_mut().unwrap().transcript.0 =
                "Person 1: archived context.\n".repeat(200);
            app.ai.answer = Some(crate::ai::PrivateText("A long answer.\n".repeat(200)));
            app.ai.question = "Question line.\n".repeat(100);
            let ctx = egui::Context::default();
            app.attach(&ctx);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            for frame in 0..6 {
                // Exercise the same anchored control in idle and streaming states.
                app.ai.pending = frame >= 4;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(screen),
                        ..Default::default()
                    },
                    |ui| crate::ui::show(&mut app, ui),
                );
                if frame == 3 || frame == 5 {
                    let (rect, clip) = ctx
                        .data_mut(|d| {
                            d.get_temp::<(egui::Rect, egui::Rect)>(egui::Id::new(
                                "ai-submit-bounds",
                            ))
                        })
                        .expect("Send stays visible");
                    assert!(screen.contains_rect(rect), "footer outside screen {rect:?}");
                    assert!(clip.contains_rect(rect), "footer clipped {rect:?} {clip:?}");
                }
                output.textures_delta.clear();
            }
        }
    }
}
