//! Poll composition and voting controls.

use super::widgets;
use crate::app::App;
use crate::i18n::{fill, t};
use crate::model::{Action, Content, Dialog, Message, PollState};
use crate::theme::{self, Icon, Palette};
use egui::{Align, Layout, Sense, Stroke, pos2, vec2};

pub fn create(app: &mut App, ui: &mut egui::Ui, chat: &str) {
    let palette = app.palette;
    ui.horizontal(|ui| {
        theme::icon(ui, Icon::ListChecks, 20.0, palette.accent);
        theme::text(
            ui,
            t(app.settings.language, "poll.create"),
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
                t(app.settings.language, "poll.close"),
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
            t(app.settings.language, "poll.question"),
            theme::medium(13.5),
            palette.secondary,
        );
        ui.add(
            egui::TextEdit::singleline(&mut app.poll_draft.question)
                .id_salt("poll-question")
                .hint_text(t(app.settings.language, "poll.ask_hint"))
                .char_limit(255)
                .font(theme::regular(14.0))
                .desired_width(f32::INFINITY),
        );
        ui.add_space(8.0);
        theme::text(
            ui,
            t(app.settings.language, "poll.answers"),
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
                for (index, answer) in app.poll_draft.options.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        let width = (ui.available_width() - 32.0).max(100.0);
                        ui.add(
                            egui::TextEdit::singleline(answer)
                                .id_salt(("poll-answer", index))
                                .hint_text(fill(
                                    t(app.settings.language, "poll.answer_hint"),
                                    &[("n", &(index + 1).to_string())],
                                ))
                                .char_limit(100)
                                .font(theme::regular(14.0))
                                .desired_width(width),
                        );
                        if removable
                            && theme::icon_button(
                                ui,
                                Icon::X,
                                14.0,
                                palette.dim,
                                palette.text,
                                t(app.settings.language, "poll.remove_answer"),
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
                t(app.settings.language, "poll.add_answer"),
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
                t(app.settings.language, "poll.multiple"),
                theme::regular(13.5),
                palette.text,
            );
        });
    });
    ui.add_space(8.0);
    let valid = app.poll_draft.validated();
    if let Err(error) = valid.as_ref() {
        theme::text(
            ui,
            t(app.settings.language, error),
            theme::regular(12.0),
            palette.dim,
        );
    }
    ui.horizontal(|ui| {
        if theme::soft_button(
            ui,
            &palette,
            None,
            t(app.settings.language, "poll.cancel"),
            false,
        )
        .clicked()
        {
            app.actions.push(Action::CloseDialog);
        }
        ui.add_enabled_ui(
            valid.is_ok() && !app.poll_creating && app.link.is_connected(),
            |ui| {
                if theme::pill_button(
                    ui,
                    &palette,
                    if app.poll_creating {
                        t(app.settings.language, "poll.sending")
                    } else {
                        t(app.settings.language, "poll.send")
                    },
                    true,
                )
                .clicked()
                {
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
    lang: crate::i18n::Language,
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
                    t(lang, "poll.select_one")
                } else {
                    t(lang, "poll.select_many")
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
                    vec2(width, label.size().y.max(18.0) + 20.0),
                    Sense::click(),
                );
                let active = enabled && state.can_vote && !pending;
                if ui.is_rect_visible(rect) {
                    if active && response.hovered() {
                        ui.painter().rect_filled(rect, 5.0, palette.surface_hover);
                    }
                    let center = pos2(rect.left() + 10.0, rect.center().y - 3.0);
                    ui.painter().circle_stroke(
                        center,
                        6.0,
                        Stroke::new(
                            1.5,
                            if selected {
                                palette.accent
                            } else {
                                palette.dim
                            },
                        ),
                    );
                    if selected {
                        ui.painter().circle_filled(center, 3.0, palette.accent);
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
                    if state.voters > 0 {
                        let fraction = count as f32 / state.voters as f32;
                        let bar = egui::Rect::from_min_size(
                            pos2(rect.left() + 25.0, rect.bottom() - 5.0),
                            vec2((width - 30.0) * fraction, 3.0),
                        );
                        ui.painter().rect_filled(bar, 2.0, palette.accent);
                    }
                }
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
                t(lang, "poll.sending_vote").to_owned()
            } else if state.refresh_failed {
                t(lang, "poll.waiting_phone").into()
            } else if state.refreshing && !state.history_complete {
                t(lang, "poll.loading_votes").into()
            } else if !state.history_complete {
                t(lang, "poll.votes_missing").into()
            } else {
                format!(
                    "{} {}",
                    state.voters,
                    if state.voters == 1 {
                        t(lang, "poll.voter_one")
                    } else {
                        t(lang, "poll.voter_many")
                    }
                )
            };
            // Like WhatsApp Web, the vote count opens the list of who voted.
            let voters = state.voters > 0 && !pending && !state.refresh_failed;
            let line = widgets::line(
                ui,
                &detail,
                theme::regular(11.0),
                if voters { palette.accent } else { palette.dim },
                width,
                usize::MAX,
            );
            let (rect, response) = ui.allocate_exact_size(
                line.size(),
                if voters {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            if ui.is_rect_visible(rect) {
                if voters && response.hovered() {
                    ui.painter().rect_filled(
                        rect.expand2(vec2(4.0, 2.0)),
                        4.0,
                        palette.surface_hover,
                    );
                }
                line.paint(
                    ui,
                    rect.min,
                    if voters { palette.accent } else { palette.dim },
                );
            }
            if voters
                && response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(t(lang, "poll.view_votes"))
                    .clicked()
            {
                actions.push(Action::ShowDialog(Dialog::PollVotes {
                    chat: message.chat.clone(),
                    message: message.id.clone(),
                }));
            }
            if !state.can_vote {
                widgets::rich_text(
                    ui,
                    t(lang, "poll.no_key"),
                    theme::regular(11.0),
                    palette.dim,
                );
            } else if !enabled {
                theme::text(
                    ui,
                    t(lang, "poll.reconnect"),
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
                crate::i18n::Language::English,
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
                crate::i18n::Language::English,
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

    #[test]
    fn the_vote_count_opens_the_voter_list_without_new_requests() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut row = crate::archive::tests::message("chat", "poll", 100, false);
        row.content = Content::Poll {
            question: "Lunch?".into(),
            options: vec!["Pizza".into(), "Pasta".into()],
            state: PollState {
                selectable: 1,
                can_vote: true,
                history_complete: true,
                voters: 2,
                counts: vec![1, 1],
                voters_list: vec![
                    crate::model::PollVoter {
                        id: "100@s.whatsapp.net".into(),
                        choices: vec![0],
                        at: 100_000,
                    },
                    crate::model::PollVoter {
                        id: "200@s.whatsapp.net".into(),
                        choices: vec![1],
                        at: 200_000,
                    },
                ],
                ..Default::default()
            },
        };
        let mut actions = Vec::new();
        let mut count = None;
        // Find the painted vote-count text, then click it like a user would.
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(360.0, 240.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input.take(), |ui| {
            ballot(
                ui,
                &Palette::dark(),
                crate::i18n::Language::English,
                &row,
                320.0,
                true,
                false,
                &mut actions,
            );
            let layers: Vec<_> = ctx.memory(|memory| memory.layer_ids().collect());
            for layer in layers {
                ctx.graphics(|graphics| {
                    if let Some(list) = graphics.get(layer) {
                        for clipped in list.all_entries() {
                            if let egui::Shape::Text(text) = &clipped.shape {
                                let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                                if text.galley.text() == "2 voters"
                                    && rect.intersects(clipped.clip_rect)
                                {
                                    count = Some(rect);
                                }
                            }
                        }
                    }
                });
            }
        });
        output.textures_delta.clear();
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, Action::VotePoll { .. })),
            "the footer is not an option row"
        );
        let count = count.expect("vote count is painted");
        for pressed in [true, false] {
            input.events = vec![
                egui::Event::PointerMoved(count.center()),
                egui::Event::PointerButton {
                    pos: count.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ];
            let mut output = ctx.run_ui(input.take(), |ui| {
                ballot(
                    ui,
                    &Palette::dark(),
                    crate::i18n::Language::English,
                    &row,
                    320.0,
                    true,
                    false,
                    &mut actions,
                );
            });
            output.textures_delta.clear();
        }
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, Action::ShowDialog(Dialog::PollVotes { .. }))),
            "clicking the vote count opens the voter list"
        );
    }
}
