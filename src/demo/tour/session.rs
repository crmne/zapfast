//! Local replies to commands emitted by the real UI during an offline tour.

use crate::{
    app::App,
    backend::Command,
    model::{Content, LastMessage, Message, Quoted, Reaction},
};

pub fn respond(app: &mut App) {
    for command in app.host.take_demo_commands() {
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
                let quoted = quoting.and_then(|id| quote(app, &chat, id));
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
            Command::SendSticker {
                chat,
                path,
                quoting,
            } => {
                let mut media = super::super::media(
                    "image/webp",
                    path.metadata().map_or(0, |meta| meta.len()),
                    Some(192),
                    Some(192),
                );
                media.path = Some(path);
                let mut row = outgoing(
                    app,
                    &chat,
                    Content::Sticker {
                        media,
                        animated: false,
                    },
                );
                row.quoted = quoting.and_then(|id| quote(app, &chat, id));
                append(app, row);
            }
            Command::SearchGifs { .. } => {
                // The offline results were rendered before the tour began.
                app.gif_pending = false;
                app.gif_error = None;
            }
            Command::RecentStickers => app.stickers_pending = false,
            Command::SearchChatMessages {
                chat,
                query,
                from,
                until,
            } => {
                // Newest first, as the archive answers.
                let hits: Vec<_> = app
                    .conversations
                    .get(&chat)
                    .map(|conversation| {
                        conversation
                            .messages
                            .iter()
                            .rev()
                            .filter(|row| query.is_empty() || row.text_matching(&query).is_some())
                            .filter(|row| from.is_none_or(|from| row.timestamp >= from))
                            .filter(|row| until.is_none_or(|until| row.timestamp < until))
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                if app.open_chat.as_deref() == Some(chat.as_str())
                    && query == app.chat_search.trim()
                {
                    app.chat_search_hits = hits;
                    app.chat_search_truncated = false;
                    app.chat_search_pending = false;
                    app.chat_search_selected = None;
                }
            }
            Command::WatchReceipts(Some((chat, message))) => {
                app.message_receipts = Some(crate::model::MessageReceipts {
                    chat,
                    message,
                    recipients: super::super::sample_recipients(crate::util::now()),
                });
            }
            Command::SendVoice {
                chat,
                samples,
                quoting,
            } => {
                let path = app.dirs.media_cache_dir().join("tour-voice.ogg");
                let Ok(bytes) = crate::voice::encode(&samples) else {
                    continue;
                };
                if std::fs::write(&path, &bytes).is_err() {
                    continue;
                }
                let mut media =
                    super::super::media("audio/ogg; codecs=opus", bytes.len() as u64, None, None);
                media.path = Some(path);
                let seconds = (samples.len() as f64 / f64::from(crate::voice::RATE)).round();
                let mut row = outgoing(
                    app,
                    &chat,
                    Content::Audio {
                        media,
                        seconds: Some(seconds.max(1.0) as u32),
                        voice_note: true,
                        waveform: crate::voice::waveform(&samples),
                    },
                );
                row.id = "tour-voice".into();
                row.quoted = quoting.and_then(|id| quote(app, &chat, id));
                append(app, row);
            }
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

fn quote(app: &App, chat: &str, id: String) -> Option<Quoted> {
    let row = app
        .conversations
        .get(&crate::model::ChatId::whatsapp(chat))?
        .message(&id)?;
    Some(Quoted {
        sender: row.sender.clone(),
        sender_name: row.sender_name.clone(),
        summary: row.summary(),
        mentions: row.mentions.clone(),
        id,
    })
}

fn outgoing(app: &App, chat: &str, content: Content) -> Message {
    let count = app
        .conversations
        .get(&crate::model::ChatId::whatsapp(chat))
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
            full: row.content.full_summary(),
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
