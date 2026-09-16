//! Local replies to commands emitted by the real UI during an offline tour.

use crate::{
    app::App,
    backend::Command,
    model::{Content, LastMessage, Message, Quoted, Reaction},
};

pub fn respond(app: &mut App) {
    for command in app.backend.take_demo_commands() {
        match command {
            Command::CreatePoll { chat, draft } => {
                let state = crate::model::PollState {
                    selectable: draft.selectable(),
                    counts: vec![0; draft.options.len()],
                    can_vote: true,
                    history_complete: true,
                    ..Default::default()
                };
                let row = outgoing(
                    app,
                    &chat,
                    Content::Poll {
                        question: draft.question,
                        options: draft.options,
                        state,
                    },
                );
                append(app, row);
                app.poll_creating = false;
                app.dialog = None;
            }
            Command::VotePoll {
                chat,
                message,
                choices,
            } => {
                if let Some(row) = app
                    .conversations
                    .get_mut(&chat)
                    .and_then(|chat| chat.message_mut(&message))
                    && let Content::Poll { state, .. } = &mut row.content
                {
                    for &old in &state.selected {
                        if let Some(count) = state.counts.get_mut(old) {
                            *count = count.saturating_sub(1);
                        }
                    }
                    if !state.selected.is_empty() {
                        state.voters = state.voters.saturating_sub(1);
                    }
                    for &new in &choices {
                        if let Some(count) = state.counts.get_mut(new) {
                            *count += 1;
                        }
                    }
                    if !choices.is_empty() {
                        state.voters += 1;
                    }
                    state.selected = choices;
                }
                app.poll_voting.remove(&(chat, message));
            }
            Command::SendText {
                chat,
                text,
                quoting,
                mentions,
            } => {
                let quoted = quoting.and_then(|id| {
                    app.conversations
                        .get(&chat)?
                        .message(&id)
                        .map(|row| Quoted {
                            id,
                            sender: row.sender.clone(),
                            sender_name: row.sender_name.clone(),
                            summary: row.summary(),
                            mentions: row.mentions.clone(),
                        })
                });
                let mut row = outgoing(app, &chat, Content::text(text));
                row.quoted = quoted;
                row.mentions = mentions
                    .into_iter()
                    .map(|id| crate::model::MentionRef {
                        user: id.split('@').next().unwrap_or_default().to_owned(),
                        id,
                    })
                    .collect();
                append(app, row);
            }
            Command::SendSticker { chat, path } => {
                let mut media = super::super::media(
                    "image/webp",
                    path.metadata().map_or(0, |meta| meta.len()),
                    Some(192),
                    Some(192),
                );
                media.path = Some(path);
                let row = outgoing(
                    app,
                    &chat,
                    Content::Sticker {
                        media,
                        animated: false,
                    },
                );
                append(app, row);
            }
            Command::SearchGifs { .. } => {
                // The offline results were rendered before the tour began.
                app.gif_pending = false;
                app.gif_error = None;
            }
            Command::RecentStickers => app.stickers_pending = false,
            Command::React {
                chat,
                message,
                emoji,
            } => {
                if let Some(row) = app
                    .conversations
                    .get_mut(&chat)
                    .and_then(|chat| chat.message_mut(&message))
                {
                    row.reactions.retain(|reaction| !reaction.from_me);
                    if !emoji.is_empty() {
                        row.reactions.push(Reaction {
                            sender: super::super::ME.into(),
                            from_me: true,
                            emoji,
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

fn outgoing(app: &App, chat: &str, content: Content) -> Message {
    let count = app
        .conversations
        .get(chat)
        .map_or(0, |chat| chat.messages.len());
    super::super::message(
        chat,
        &format!("tour-{count}"),
        true,
        crate::util::now(),
        content,
    )
}

fn append(app: &mut App, row: Message) {
    if let Some(chat) = app.chats.iter_mut().find(|chat| chat.id == row.chat) {
        chat.last_activity = row.timestamp;
        chat.last = Some(LastMessage {
            from_me: row.from_me,
            sender: row.sender.clone(),
            sender_name: row.sender_name.clone(),
            summary: row.summary(),
            status: row.status,
        });
    }
    app.conversations
        .get_mut(&row.chat)
        .expect("sample chat")
        .messages
        .push(row);
    app.scroll_to_bottom = true;
}
