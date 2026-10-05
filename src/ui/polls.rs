//! Poll composition and voting controls.

use super::widgets;
use crate::app::App;
use crate::model::{Action, Content, Dialog, Message, PollState};
use crate::theme::{self, Icon, Palette};
use egui::{Align, Layout, Sense, Stroke, pos2, vec2};

pub fn create(app: &mut App, ui: &mut egui::Ui, chat: &str) {
    let palette = app.palette;
    let locale = app.locale;
    ui.horizontal(|ui| {
        theme::icon(ui, Icon::ListChecks, 20.0, palette.accent);
        theme::text(
            ui,
            crate::i18n::gettext(locale, "Create poll"),
            theme::bold(18.0),
            palette.text,
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if theme::icon_button(
                ui,
                Icon::X,
                16.0,
                palette.secondary,
                palette.text,
                &crate::i18n::gettext(locale, "Close"),
            )
            .clicked()
            {
                app.actions.push(Action::CloseDialog);
            }
        });
    });
    ui.add_space(8.0);
    ui.add_enabled_ui(!app.poll_creating, |ui| {
        theme::text(
            ui,
            crate::i18n::gettext(locale, "Question"),
            theme::medium(13.5),
            palette.secondary,
        );
        let format = egui::TextFormat::simple(theme::regular(14.0), palette.text);
        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
            crate::bidi::layout_field(ui, text.as_str(), &format, wrap)
        };
        let question_align = if crate::bidi::base_rtl(&app.poll_draft.question) {
            Align::RIGHT
        } else {
            Align::LEFT
        };
        let question_hint = crate::i18n::gettext(locale, "Ask a question");
        ui.add(
            egui::TextEdit::singleline(&mut app.poll_draft.question)
                .id_salt("poll-question")
                .hint_text(question_hint)
                .char_limit(255)
                .font(theme::regular(14.0))
                .desired_width(f32::INFINITY)
                .horizontal_align(question_align)
                .layouter(&mut layouter),
        );
        ui.add_space(8.0);
        theme::text(
            ui,
            crate::i18n::gettext(locale, "Answers"),
            theme::medium(13.5),
            palette.secondary,
        );
        let height = (ui.ctx().content_rect().height() - 320.0).clamp(90.0, 330.0);
        let mut remove = None;
        let removable = app.poll_draft.options.len() > 2;
        egui::ScrollArea::vertical()
            .id_salt("poll-answers")
            .max_height(height)
            .show(ui, |ui| {
                let answer_format = egui::TextFormat::simple(theme::regular(14.0), palette.text);
                let mut answer_layouter =
                    |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
                        crate::bidi::layout_field(ui, text.as_str(), &answer_format, wrap)
                    };
                for (index, answer) in app.poll_draft.options.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        let width = (ui.available_width() - 32.0).max(100.0);
                        let answer_align = if crate::bidi::base_rtl(answer) {
                            Align::RIGHT
                        } else {
                            Align::LEFT
                        };
                        let answer_hint = crate::i18n::gettext(locale, "Answer {index}")
                            .replace("{index}", &(index + 1).to_string());
                        ui.add(
                            egui::TextEdit::singleline(answer)
                                .id_salt(("poll-answer", index))
                                .hint_text(answer_hint)
                                .char_limit(100)
                                .font(theme::regular(14.0))
                                .desired_width(width)
                                .horizontal_align(answer_align)
                                .layouter(&mut answer_layouter),
                        );
                        if removable
                            && theme::icon_button(
                                ui,
                                Icon::X,
                                14.0,
                                palette.dim,
                                palette.text,
                                &crate::i18n::gettext(locale, "Remove answer"),
                            )
                            .clicked()
                        {
                            remove = Some(index);
                        }
                    });
                }
            });
        if let Some(index) = remove {
            app.poll_draft.options.remove(index);
        }
        if app.poll_draft.options.len() < 12
            && theme::soft_button(
                ui,
                &palette,
                Some(Icon::Plus),
                &crate::i18n::gettext(locale, "Add answer"),
                false,
            )
            .clicked()
        {
            app.poll_draft.options.push(String::new());
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            widgets::switch(ui, &palette, &mut app.poll_draft.multiple);
            theme::text(
                ui,
                crate::i18n::gettext(locale, "Allow multiple answers"),
                theme::regular(13.5),
                palette.text,
            );
        });
    });
    ui.add_space(8.0);
    let valid = app.poll_draft.validated();
    if let Err(error) = valid.as_ref() {
        theme::text(ui, *error, theme::regular(12.0), palette.dim);
    }
    ui.horizontal(|ui| {
        if theme::soft_button(
            ui,
            &palette,
            None,
            &crate::i18n::gettext(locale, "Cancel"),
            false,
        )
        .clicked()
        {
            app.actions.push(Action::CloseDialog);
        }
        ui.add_enabled_ui(
            valid.is_ok() && !app.poll_creating && app.link.is_connected(),
            |ui| {
                let button_label = if app.poll_creating {
                    crate::i18n::gettext(locale, "Sending…")
                } else {
                    crate::i18n::gettext(locale, "Send poll")
                };
                if theme::pill_button(ui, &palette, &button_label, true).clicked() {
                    app.actions.push(Action::CreatePoll {
                        chat: chat.into(),
                        draft: app.poll_draft.clone(),
                    });
                }
            },
        );
    });
}

#[allow(clippy::too_many_arguments)]
pub fn ballot(
    ui: &mut egui::Ui,
    palette: &Palette,
    locale: crate::i18n::Locale,
    message: &Message,
    width: f32,
    enabled: bool,
    pending: bool,
    actions: &mut Vec<Action>,
) {
    let Content::Poll {
        question,
        options,
        state,
    } = &message.content
    else {
        return;
    };
    let response =
        ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(width);
            let question = widgets::line(
                ui,
                question,
                theme::semibold(14.0),
                palette.text,
                width,
                usize::MAX,
            );
            let (rect, _) = ui.allocate_exact_size(question.size(), Sense::hover());
            if ui.is_rect_visible(rect) {
                question.paint(ui, rect.min, palette.text);
            }
            theme::text(
                ui,
                if state.selectable == 1 {
                    crate::i18n::gettext(locale, "Select one answer")
                } else {
                    crate::i18n::gettext(locale, "Select answers")
                },
                theme::regular(11.5),
                palette.secondary,
            );
            ui.add_space(4.0);
            for (index, option) in options.iter().enumerate() {
                let selected = state.selected.contains(&index);
                let label = widgets::line(
                    ui,
                    option,
                    theme::regular(13.5),
                    palette.text,
                    (width - 58.0).max(50.0),
                    3,
                );
                let (rect, response) = ui.allocate_exact_size(
                    vec2(width, label.size().y.max(18.0) + 34.0),
                    if enabled && state.can_vote && !pending {
                        Sense::click()
                    } else {
                        Sense::hover()
                    },
                );
                let active = enabled && state.can_vote && !pending;
                if ui.is_rect_visible(rect) {
                    if active && response.hovered() {
                        ui.painter().rect_filled(rect, 5.0, palette.surface_hover);
                    }
                    let center = pos2(rect.left() + 10.0, rect.top() + 16.0);
                    if selected {
                        ui.painter().circle_filled(center, 8.0, palette.accent);
                        theme::paint_icon(
                            ui,
                            Icon::Check,
                            egui::Rect::from_center_size(center, vec2(12.0, 12.0)),
                            12.0,
                            palette.window,
                        );
                    } else {
                        ui.painter().circle_stroke(
                            center,
                            8.0,
                            Stroke::new(1.5, palette.secondary),
                        );
                    }
                    label.paint(ui, pos2(rect.left() + 25.0, rect.top() + 6.0), palette.text);
                    let count = state.counts.get(index).copied().unwrap_or_default();
                    ui.painter().text(
                        pos2(rect.right() - 5.0, center.y),
                        egui::Align2::RIGHT_CENTER,
                        count.to_string(),
                        theme::regular(12.0),
                        palette.secondary,
                    );
                    let track = egui::Rect::from_min_size(
                        pos2(rect.left() + 25.0, rect.bottom() - 12.0),
                        vec2((width - 30.0).max(0.0), 6.0),
                    );
                    ui.painter()
                        .rect_filled(track, 3.0, palette.text.gamma_multiply(0.10));
                    if state.voters > 0 && count > 0 {
                        let fraction = (count as f32 / state.voters as f32).min(1.0);
                        let bar = egui::Rect::from_min_size(
                            track.min,
                            vec2(track.width() * fraction, track.height()),
                        );
                        ui.painter().rect_filled(
                            bar,
                            3.0,
                            if selected {
                                palette.accent
                            } else {
                                palette.secondary
                            },
                        );
                    }
                }
                ui.ctx().data_mut(|data| {
                    data.insert_temp(
                        super::conversation::bubble_id(&message.chat, &message.id)
                            .with(("poll-option", index)),
                        rect,
                    )
                });
                response.widget_info(|| {
                    egui::WidgetInfo::selected(egui::WidgetType::Checkbox, active, selected, option)
                });
                if active
                    && response
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    && let Some(choices) = selection_after_click(state, index)
                {
                    actions.push(Action::VotePoll {
                        chat: message.chat.clone(),
                        message: message.id.clone(),
                        choices,
                    });
                }
            }
            let detail = if pending {
                crate::i18n::gettext(locale, "Sending vote…").into_owned()
            } else if state.refresh_failed {
                crate::i18n::gettext(
                    locale,
                    "Waiting for your phone · earlier votes may be missing",
                )
                .into_owned()
            } else if state.refreshing && !state.history_complete {
                crate::i18n::gettext(locale, "Loading earlier votes from your phone…").into_owned()
            } else if !state.history_complete {
                crate::i18n::gettext(locale, "Earlier votes have not been loaded yet").into_owned()
            } else {
                crate::i18n::ngettext(locale, "1 voter", "{count} voters", state.voters as u32)
                    .replace("{count}", &state.voters.to_string())
            };
            let line = widgets::line(
                ui,
                &detail,
                theme::regular(11.0),
                palette.dim,
                width,
                usize::MAX,
            );
            let (rect, _) = ui.allocate_exact_size(line.size(), Sense::hover());
            if ui.is_rect_visible(rect) {
                line.paint(ui, rect.min, palette.dim);
            }
            if !state.can_vote {
                widgets::rich_text(
                    ui,
                    &crate::i18n::gettext(locale, "Voting key unavailable · use your phone"),
                    theme::regular(11.0),
                    palette.dim,
                );
            } else if !enabled {
                theme::text(
                    ui,
                    crate::i18n::gettext(locale, "Reconnect to vote"),
                    theme::regular(11.0),
                    palette.dim,
                );
            }
        });
    if enabled
        && state.refresh_needed
        && !state.refreshing
        && ui.is_rect_visible(response.response.rect)
    {
        actions.push(Action::RefreshPoll {
            chat: message.chat.clone(),
            message: message.id.clone(),
        });
    }
}

/// The message timestamp stays above this full-width action, like interactive cards.
pub fn results_button(
    ui: &mut egui::Ui,
    palette: &Palette,
    locale: crate::i18n::Locale,
    message: &Message,
    width: f32,
    actions: &mut Vec<Action>,
) {
    let Content::Poll { state, .. } = &message.content else {
        return;
    };
    let enabled = state.voters > 0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, 39.0), Sense::hover());
    let rect = rect.expand2(vec2(10.0, 0.0));
    let rect = egui::Rect::from_min_max(rect.min, rect.max + vec2(0.0, 5.0));
    let response = ui.interact(
        rect,
        super::conversation::bubble_id(&message.chat, &message.id).with("poll-results"),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let button_label = crate::i18n::gettext(locale, "Show votes");
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &button_label)
    });
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            super::conversation::bubble_id(&message.chat, &message.id).with("poll-results-rect"),
            rect,
        )
    });
    if enabled && (response.hovered() || response.has_focus()) {
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius {
                sw: 10,
                se: 10,
                ..Default::default()
            },
            palette.text.gamma_multiply(0.04),
        );
    }
    ui.painter().hline(
        rect.x_range(),
        rect.top(),
        Stroke::new(1.0, palette.secondary.gamma_multiply(0.2)),
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        button_label,
        theme::medium(13.5),
        if enabled {
            palette.link
        } else {
            palette.secondary
        },
    );
    theme::reveal_focus(&response);
    if enabled
        && response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
    {
        actions.push(Action::ShowDialog(Dialog::PollResults {
            chat: message.chat.clone(),
            message: message.id.clone(),
        }));
    }
}

pub fn results(app: &mut App, ui: &mut egui::Ui, chat: &str, id: &str) {
    let palette = app.palette;
    let locale = app.locale;
    ui.horizontal(|ui| {
        theme::text(
            ui,
            crate::i18n::gettext(locale, "Poll results"),
            theme::semibold(18.0),
            palette.text,
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if theme::icon_button(
                ui,
                Icon::X,
                16.0,
                palette.secondary,
                palette.text,
                &crate::i18n::gettext(locale, "Close"),
            )
            .clicked()
            {
                app.actions.push(Action::CloseDialog);
            }
        });
    });
    let Some(message) = app
        .conversations
        .get(chat)
        .and_then(|c| c.message(id))
        .cloned()
    else {
        widgets::rich_text(
            ui,
            &crate::i18n::gettext(locale, "This poll is no longer available."),
            theme::regular(14.0),
            palette.secondary,
        );
        return;
    };
    let Content::Poll {
        question,
        options,
        state,
    } = &message.content
    else {
        return;
    };
    ui.add_space(12.0);
    widgets::rich_text(ui, question, theme::semibold(16.0), palette.text);
    if !state.history_complete {
        widgets::rich_text(
            ui,
            &crate::i18n::gettext(
                locale,
                "Earlier votes may still be missing. Results update as they arrive.",
            ),
            theme::regular(12.0),
            palette.secondary,
        );
    }
    let height = widgets::dialog_scroll_height(ui);
    let area = egui::ScrollArea::vertical()
        .id_salt(("poll-results", chat, id))
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
        .max_height(height)
        .min_scrolled_height(height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (index, option) in options.iter().enumerate() {
                ui.add_space(16.0);
                let count = state.counts.get(index).copied().unwrap_or(0);
                let heading = ui.horizontal(|ui| {
                    let width = (ui.available_width() - 70.0).max(1.0);
                    let label = widgets::line(
                        ui,
                        option,
                        theme::semibold(14.0),
                        palette.text,
                        width,
                        usize::MAX,
                    );
                    let (rect, _) =
                        ui.allocate_exact_size(vec2(width, label.size().y), Sense::hover());
                    label.paint(ui, rect.min, palette.text);
                    theme::text(
                        ui,
                        crate::i18n::ngettext(locale, "1 vote", "{count} votes", count as u32)
                            .replace("{count}", &count.to_string()),
                        theme::regular(12.0),
                        palette.secondary,
                    );
                });
                ui.ctx().data_mut(|data| {
                    data.insert_temp(
                        super::conversation::bubble_id(chat, id)
                            .with(("poll-result-option", index)),
                        heading.response.rect,
                    )
                });
                let voters: Vec<_> = state
                    .votes
                    .iter()
                    .filter(|vote| vote.choices.contains(&index))
                    .collect();
                for voter in &voters {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let picture = app.avatar(&voter.id);
                        widgets::avatar(
                            ui,
                            &palette,
                            &voter.name,
                            &voter.id,
                            34.0,
                            picture.as_deref(),
                        );
                        ui.vertical(|ui| {
                            widgets::rich_text(ui, &voter.name, theme::regular(14.0), palette.text);
                            theme::text(
                                ui,
                                crate::util::moment_stamp(locale, voter.timestamp),
                                theme::regular(12.0),
                                palette.secondary,
                            );
                        });
                    });
                }
                if voters.len() < count {
                    theme::text(
                        ui,
                        crate::i18n::gettext(locale, "Participant details are not available yet"),
                        theme::regular(12.0),
                        palette.secondary,
                    );
                }
            }
        });
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            super::conversation::bubble_id(chat, id).with("poll-results-viewport"),
            area.inner_rect,
        )
    });
}

fn selection_after_click(state: &PollState, index: usize) -> Option<Vec<usize>> {
    let mut choices = state.selected.clone();
    if choices.contains(&index) {
        choices.retain(|&choice| choice != index);
    } else {
        if state.selectable == 1 {
            choices.clear();
        }
        if state.selectable > 0 && choices.len() >= state.selectable {
            return None;
        }
        choices.push(index);
    }
    choices.sort_unstable();
    Some(choices)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn visible_polls_request_history_automatically_without_a_control() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut row = crate::archive::tests::message("chat", "poll", 100, false);
        row.content = Content::Poll {
            question: "Lunch?".into(),
            options: vec!["Pizza".into(), "Pasta".into()],
            state: PollState {
                selectable: 1,
                can_vote: true,
                refresh_needed: true,
                ..Default::default()
            },
        };
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ballot(
                ui,
                &Palette::dark(),
                crate::i18n::Locale::English,
                &row,
                320.0,
                true,
                false,
                &mut actions,
            )
        });
        output.textures_delta.clear();
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, Action::RefreshPoll { .. }))
        );
        let Content::Poll { state, .. } = &mut row.content else {
            panic!("poll")
        };
        state.refreshing = true;
        actions.clear();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ballot(
                ui,
                &Palette::dark(),
                crate::i18n::Locale::English,
                &row,
                320.0,
                true,
                false,
                &mut actions,
            )
        });
        output.textures_delta.clear();
        assert!(
            actions.is_empty(),
            "rendering while waiting cannot submit more requests"
        );
    }

    #[test]
    fn single_and_multiple_choices_can_be_replaced_and_withdrawn() {
        let mut state = PollState {
            selectable: 1,
            selected: vec![0],
            ..Default::default()
        };
        assert_eq!(selection_after_click(&state, 1), Some(vec![1]));
        assert_eq!(selection_after_click(&state, 0), Some(vec![]));
        state.selectable = 2;
        assert_eq!(selection_after_click(&state, 1), Some(vec![0, 1]));
        state.selected.push(1);
        assert_eq!(selection_after_click(&state, 2), None);
        assert_eq!(selection_after_click(&state, 1), Some(vec![0]));
    }
}
