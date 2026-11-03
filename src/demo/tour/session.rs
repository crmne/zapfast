//! Local replies to commands emitted by the real UI during an offline tour.

use crate::{
    app::App,
    backend::Command,
    model::{Content, LastMessage, Message, Quoted, Reaction},
};

pub fn respond(app: &mut App) {
    for command in app.backend.take_demo_commands() {
        match command {
            Command::SetArchived(chat, archived) => {
                if let Some(shown) = app.chats.iter_mut().find(|shown| shown.id == chat) {
                    shown.archived = archived;
                    if archived && app.open_chat.as_deref() == Some(chat.as_str()) {
                        app.actions.push(crate::model::Action::CloseChat);
                    }
                }
            }
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
            // No network offline: only the sample's own site has a card.
            Command::FetchLinkPreview { url } => {
                let preview = link_preview(&url);
                app.link_preview_arrived(url, preview);
            }
            Command::SendText {
                chat,
                text,
                quoting,
                mentions,
                preview,
            } => {
                let quoted = quoting.and_then(|id| quote(app, &chat, id));
                let (content, thumbnail) = match preview {
                    Some(preview) => (
                        Content::Text {
                            text,
                            preview: Some(preview.preview),
                        },
                        preview.thumbnail,
                    ),
                    None => (Content::text(text), None),
                };
                let mut row = outgoing(app, &chat, content);
                row.thumbnail = thumbnail;
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
            Command::InspectPicture { target, path } => {
                let size = std::fs::read(&path)
                    .map_err(|error| error.to_string())
                    .and_then(|bytes| crate::backend::picture_edit::inspect(&bytes));
                match size {
                    Ok((width, height)) => {
                        app.picture_edit = Some(crate::model::PictureEdit::new(
                            target,
                            crate::model::PictureSource::File(path),
                            width,
                            height,
                        ));
                    }
                    Err(error) => app.toast_error(error),
                }
            }
            Command::ApplyPictureEdit {
                target,
                source,
                width,
                height,
                crop,
                turns,
            } => {
                // The same work the worker would do, on this thread instead.
                let written = crate::backend::picture_edit::write(
                    &source,
                    width,
                    height,
                    crop,
                    turns,
                    &app.dirs.media_cache_dir().join("edited"),
                );
                match written {
                    // Applied the same way the app applies it: only while that
                    // attachment is still staged.
                    Ok(path) => {
                        if let Some(at) = app.staged_at(target) {
                            app.pending[at].item = crate::app::Pending::File(path);
                            if let Some(origin) = app.picture_applying.remove(&target) {
                                app.picture_origins.insert(target, origin);
                            }
                        } else {
                            app.picture_applying.remove(&target);
                        }
                    }
                    Err(error) => {
                        app.picture_applying.remove(&target);
                        app.toast_error(error);
                    }
                }
                app.picture_edit = None;
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
            // A further page: the same pictures again, as new results.
            Command::SearchGifs {
                query,
                offset,
                request,
                ..
            } if offset > 0 => {
                let gifs = app
                    .gif_results
                    .iter()
                    .rev()
                    .map(|gif| crate::model::Gif {
                        id: format!("{}-{offset}", gif.id),
                        ..gif.clone()
                    })
                    .collect();
                app.backend.deliver_demo_event(crate::backend::Event::Gifs {
                    query,
                    offset,
                    request,
                    results: Ok(crate::model::GifPage {
                        gifs,
                        next_offset: None,
                    }),
                });
            }
            Command::SearchGifs { .. } => {
                // The offline results were rendered before the tour began.
                app.gif_pending = false;
                app.gif_error = None;
            }
            Command::SetBlocked { chat, blocked } => {
                let ids = app.blocklist.ids.get_or_insert_with(Default::default);
                if blocked {
                    ids.insert(chat);
                } else {
                    ids.remove(&chat);
                }
            }
            Command::SetDisappearingTimer { chat, seconds } => {
                if let Some(known) = app.chat_mut(&chat) {
                    known.ephemeral_expiration = Some(seconds);
                }
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
            // The engine's answers to the call screen's own controls: the
            // call as it now is, as the worker reports it.
            Command::SetCallMuted(muted) => update_call(app, |call| call.muted = muted),
            Command::SetCallCamera(on) => update_call(app, |call| call.camera_on = on),
            Command::AnswerCall => update_call(app, |call| {
                call.phase = crate::calls::CallPhase::Active;
                call.started = Some(std::time::Instant::now());
                call.remote_video = call.video;
                call.camera_on = call.video;
            }),
            Command::LoadCallLog => {
                // The sample's history, with the calls made on this run.
                let mut log = super::super::call_entries(crate::util::now());
                log.extend(
                    app.conversations
                        .values()
                        .flat_map(|conversation| &conversation.messages)
                        .filter(|row| row.id.starts_with(super::calls::ENTRY))
                        .cloned(),
                );
                log.sort_by_key(|row| std::cmp::Reverse(row.timestamp));
                app.call_log = log;
                app.call_log_loaded = true;
            }
            _ => {}
        }
    }
}

/// The card for a link to the sample's own site, as the worker would make it.
fn link_preview(url: &str) -> Option<crate::model::ComposedPreview> {
    url.contains("spotifast.rocks")
        .then(|| crate::model::ComposedPreview {
            preview: crate::model::LinkPreview {
                url: url.to_owned(),
                title: Some("spotifast.rocks".into()),
                description: Some(
                    "Spotify, native and fast. A lightweight Spotify client written in Rust with egui."
                        .into(),
                ),
            },
            thumbnail: Some(super::super::stock::thumbnail(super::super::stock::SPOTIFAST)),
        })
}

/// Delivers the call on screen, changed by `change`, as the worker's next
/// word on it.
pub(super) fn update_call(app: &mut App, change: impl FnOnce(&mut crate::calls::CallUpdate)) {
    let Some(mut call) = app.call.clone().filter(|call| call.phase.is_live()) else {
        return;
    };
    change(&mut call);
    app.backend
        .deliver_demo_event(crate::backend::Event::Call(Box::new(call)));
}

/// Resolves an explicit quote source rather than assuming it belongs to the destination chat.
fn quote(app: &App, chat: &str, target: crate::model::ReplyTarget) -> Option<Quoted> {
    let row = app
        .conversations
        .get(target.source_chat(chat))?
        .message(&target.id)?;
    Some(Quoted {
        labelled: Some(row.content.labelled()),
        chat: target.chat,
        sender: row.sender.clone(),
        sender_name: row.sender_name.clone(),
        summary: row.summary(),
        mentions: row.mentions.clone(),
        id: target.id,
    })
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

pub(super) fn append(app: &mut App, row: Message) {
    if let Some(chat) = app.chats.iter_mut().find(|chat| chat.id == row.chat) {
        chat.last_activity = row.timestamp;
        chat.last = Some(LastMessage {
            labelled: row.content.labelled(),
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
