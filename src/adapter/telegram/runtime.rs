//! Telegram account runtime: login, dialogs, updates, and commands.
//!
//! One account talks to one Telegram account. The runtime owns the grammers
//! client, projects protocol types through [`super::project`], and speaks the
//! same [`Command`] and [`Event`] types as the WhatsApp worker so the host can
//! route both the same way.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::Sender;

use anyhow::{Context, Result};
use grammers_client::client::{LoginToken, PasswordToken};
use grammers_client::message::Message as TelegramMessage;
use grammers_client::sender::UpdatesConfiguration;
use grammers_client::{Client, SignInError};
use grammers_session::Session;
use grammers_session::types::{PeerId, PeerRef};
use tokio::sync::mpsc;

use crate::account::{AccountId, AuthState};
use crate::backend::{Command, Event, LoginStep, Waker};
use crate::model::{ChatId, Content, MediaState};
use crate::paths::AppDirs;

use super::TelegramSession;
use super::project::{self, chat_id};

/// One page of messages. One extra row is fetched to learn whether older
/// messages exist.
const PAGE: usize = 50;

/// Telegram redirects downloads at or above this size through a CDN path the
/// pinned grammers release cannot follow, so those files stay undownloaded on
/// purpose instead of panicking inside the client.
const DOWNLOAD_LIMIT: u64 = 10 * 1024 * 1024;

/// Sends events and asks the window to repaint, like the WhatsApp worker.
struct Sink {
    events: Sender<Event>,
    waker: Waker,
}

impl Sink {
    fn send(&self, event: Event) {
        let _ = self.events.send(event);
        self.waker.wake();
    }
}

/// The sender run loop. Errors end the account with an [`AuthState::Failed`]
/// note instead of taking the whole window down.
pub(crate) async fn run(
    dirs: AppDirs,
    account: AccountId,
    events: Sender<Event>,
    _commands_tx: mpsc::UnboundedSender<Command>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    waker: Waker,
) {
    let sink = Sink { events, waker };
    if let Err(error) = serve(dirs, account, &sink, &mut commands).await {
        log::warn!("Telegram account {account} stopped: {error}");
        sink.send(Event::Auth {
            account,
            state: AuthState::Failed {
                reason: error.to_string(),
            },
        });
    }
}

async fn serve(
    dirs: AppDirs,
    account: AccountId,
    sink: &Sink,
    commands: &mut mpsc::UnboundedReceiver<Command>,
) -> Result<()> {
    let session = Arc::new(TelegramSession::open(
        &dirs
            .account_dir(account)
            .join("grammers")
            .join("session.db"),
    )?);
    let (api_id, api_hash) = credentials()?;
    let pool = grammers_client::SenderPool::new(session.clone(), api_id);
    let handle = pool.handle;
    let updates = pool.updates;
    tokio::spawn(pool.runner.run());
    let client = Client::new(handle);

    let mut auth = if client.is_authorized().await? {
        AuthState::Ready
    } else {
        AuthState::SignedOut
    };
    sink.send(Event::Auth {
        account,
        state: auth.clone(),
    });

    let mut token: Option<LoginToken> = None;
    let mut password_token: Option<PasswordToken> = None;
    let mut phone = String::new();
    while auth != AuthState::Ready {
        let Some(command) = commands.recv().await else {
            return Ok(());
        };
        match command {
            Command::Shutdown => return Ok(()),
            Command::Login {
                account: login_account,
                step,
            } if login_account == account => match step {
                LoginStep::TelegramPhone(number) => {
                    match client.request_login_code(&number, &api_hash).await {
                        Ok(code_token) => {
                            token = Some(code_token);
                            phone = number.clone();
                            auth = AuthState::TelegramCode { phone: number };
                        }
                        Err(error) => auth = failed(&error),
                    }
                }
                LoginStep::TelegramCode(code) => {
                    let Some(code_token) = token.as_ref() else {
                        continue;
                    };
                    match client.sign_in(code_token, &code).await {
                        Ok(_) => auth = AuthState::Ready,
                        Err(SignInError::PasswordRequired(password)) => {
                            password_token = Some(password);
                            auth = AuthState::TelegramPassword {
                                phone: phone.clone(),
                            };
                        }
                        Err(error) => auth = failed(&error),
                    }
                }
                LoginStep::TelegramPassword(password) => {
                    let Some(token) = password_token.take() else {
                        continue;
                    };
                    match client.check_password(token, password).await {
                        Ok(_) => auth = AuthState::Ready,
                        Err(SignInError::InvalidPassword(password)) => {
                            password_token = Some(password);
                            auth = AuthState::TelegramPassword {
                                phone: phone.clone(),
                            };
                        }
                        Err(error) => auth = failed(&error),
                    }
                }
            },
            _ => {}
        }
        sink.send(Event::Auth {
            account,
            state: auth.clone(),
        });
    }

    let me = client
        .get_me()
        .await
        .context("The account did not answer")?;
    let me_id = me.id();
    sink.send(Event::Me {
        id: me_id.to_string(),
        lid: None,
        name: Some(me.full_name()),
        about: None,
    });

    let mut stream = client
        .stream_updates(updates, UpdatesConfiguration { catch_up: true })
        .await
        .map_err(|error| anyhow::anyhow!("The Telegram update stream could not start: {error}"))?;

    let mut dialogs = client.iter_dialogs();
    let mut chats = Vec::new();
    while let Some(dialog) = dialogs.next().await? {
        if let Some(chat) = project::chat(account, &dialog, Some(me_id)) {
            chats.push(chat);
        }
    }
    sink.send(Event::Chats(chats));

    // Oldest message id seen per chat, so "fetch older" pages backwards.
    let mut oldest: HashMap<ChatId, i32> = HashMap::new();

    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else {
                    return Ok(());
                };
                if matches!(command, Command::Shutdown) {
                    let _ = stream.sync_update_state().await;
                    return Ok(());
                }
                if let Err(error) = handle_command(
                    &dirs,
                    &session,
                    Identity { account, me: me_id },
                    &client,
                    sink,
                    &mut oldest,
                    command,
                )
                .await
                {
                    log::warn!("Telegram command failed: {error}");
                }
            }
            update = stream.next() => {
                match update {
                    Ok(update) => handle_update(account, sink, me_id, update).await,
                    Err(error) => {
                        log::warn!("Telegram updates stopped: {error}");
                        return Ok(());
                    }
                }
            }
        }
    }
}

fn credentials() -> Result<(i32, String)> {
    let api_id = std::env::var("ZAPFAST_TELEGRAM_API_ID")
        .ok()
        .and_then(|value| value.parse::<i32>().ok());
    let api_hash = std::env::var("ZAPFAST_TELEGRAM_API_HASH").ok();
    match (api_id, api_hash) {
        (Some(api_id), Some(api_hash)) => Ok((api_id, api_hash)),
        _ => anyhow::bail!(
            "Telegram needs ZAPFAST_TELEGRAM_API_ID and ZAPFAST_TELEGRAM_API_HASH from my.telegram.org"
        ),
    }
}

fn failed(error: &impl std::fmt::Display) -> AuthState {
    AuthState::Failed {
        reason: error.to_string(),
    }
}

/// The cached reference for a chat's peer. Sending always goes through an
/// access hash this way, never a bare numeric id.
async fn peer_ref(session: &TelegramSession, sink: &Sink, chat: &ChatId) -> Option<PeerRef> {
    let peer = project::parse_peer(chat.peer())?;
    match session.peer_ref(peer).await {
        Ok(Some(reference)) => Some(reference),
        Ok(None) => {
            log::warn!("Telegram has no access hash cached for a chat");
            sink.send(Event::Messages {
                chat: chat.clone(),
                messages: Vec::new(),
                older: false,
                complete: true,
            });
            None
        }
        Err(error) => {
            log::warn!("Telegram could not read its session: {error}");
            None
        }
    }
}

async fn project_message(
    account: AccountId,
    message: &TelegramMessage,
    me: PeerId,
) -> Option<crate::model::Message> {
    let quoted = if message.reply_to_message_id().is_some() {
        message
            .get_reply()
            .await
            .ok()
            .flatten()
            .map(|reply| project::quoted(&reply))
    } else {
        None
    };
    project::message(account, message, Some(me), quoted)
}

/// The local account and the Telegram user it is signed in as.
#[derive(Clone, Copy)]
struct Identity {
    account: AccountId,
    me: PeerId,
}

async fn page(
    client: &Client,
    session: &TelegramSession,
    sink: &Sink,
    chat: &ChatId,
    before: Option<i32>,
    oldest: &mut HashMap<ChatId, i32>,
    me: PeerId,
) -> Result<()> {
    let account = chat.account();
    let Some(reference) = peer_ref(session, sink, chat).await else {
        return Ok(());
    };
    let mut iter = client.iter_messages(reference).limit(PAGE + 1);
    if let Some(before) = before {
        iter = iter.offset_id(before);
    }
    let mut messages = Vec::new();
    while let Some(message) = iter.next().await? {
        if let Some(message) = project_message(account, &message, me).await {
            messages.push(message);
        }
    }
    let older = messages.len() > PAGE;
    messages.truncate(PAGE);
    messages.reverse();
    if let Some(earliest) = messages
        .first()
        .and_then(|message| message.id.parse::<i32>().ok())
    {
        oldest
            .entry(chat.clone())
            .and_modify(|seen| *seen = (*seen).min(earliest))
            .or_insert(earliest);
    }
    sink.send(Event::Messages {
        chat: chat.clone(),
        messages,
        older,
        complete: !older,
    });
    Ok(())
}

fn mime_of(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("mp4") => "video/mp4",
        Some("ogg" | "oga") => "audio/ogg",
        _ => "application/octet-stream",
    }
}

fn uploaded_photo(
    uploaded: grammers_client::media::Uploaded,
) -> grammers_client::tl::enums::InputMedia {
    grammers_client::tl::types::InputMediaUploadedPhoto {
        spoiler: false,
        live_photo: false,
        file: uploaded.raw,
        stickers: None,
        ttl_seconds: None,
        video: None,
    }
    .into()
}

fn uploaded_document(
    uploaded: grammers_client::media::Uploaded,
    mime: &str,
    file_name: String,
    voice: Option<i32>,
) -> grammers_client::tl::enums::InputMedia {
    let attributes = match voice {
        Some(duration) => vec![
            grammers_client::tl::types::DocumentAttributeAudio {
                voice: true,
                duration,
                title: None,
                performer: None,
                waveform: None,
            }
            .into(),
        ],
        None => vec![grammers_client::tl::types::DocumentAttributeFilename { file_name }.into()],
    };
    grammers_client::tl::types::InputMediaUploadedDocument {
        nosound_video: false,
        force_file: voice.is_some(),
        spoiler: false,
        file: uploaded.raw,
        thumb: None,
        mime_type: mime.to_owned(),
        attributes,
        stickers: None,
        video_cover: None,
        video_timestamp: None,
        ttl_seconds: None,
    }
    .into()
}

async fn send_upload(
    client: &Client,
    reference: PeerRef,
    path: &std::path::Path,
    photo: bool,
    voice: Option<i32>,
    caption: Option<String>,
    reply_to: Option<i32>,
) -> Result<()> {
    let uploaded = client.upload_file(path).await?;
    let media = if let Some(duration) = voice {
        uploaded_document(
            uploaded,
            "audio/ogg",
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            Some(duration),
        )
    } else if photo {
        uploaded_photo(uploaded)
    } else {
        uploaded_document(
            uploaded,
            mime_of(path),
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            None,
        )
    };
    let mut input = grammers_client::message::InputMessage::new().media(media);
    if let Some(caption) = caption {
        input = input.text(caption);
    }
    if let Some(id) = reply_to {
        input = input.reply_to(Some(id));
    }
    client.send_message(reference, input).await?;
    Ok(())
}

async fn handle_command(
    dirs: &AppDirs,
    session: &TelegramSession,
    identity: Identity,
    client: &Client,
    sink: &Sink,
    oldest: &mut HashMap<ChatId, i32>,
    command: Command,
) -> Result<()> {
    let Identity { account, me } = identity;
    match command {
        Command::SendText {
            chat,
            text,
            quoting,
            ..
        } => {
            let Some(reference) = peer_ref(session, sink, &chat).await else {
                return Ok(());
            };
            let mut input = grammers_client::message::InputMessage::new().text(text);
            if let Some(id) = quoting.as_deref().and_then(|id| id.parse::<i32>().ok()) {
                input = input.reply_to(Some(id));
            }
            let sent = client.send_message(reference, input).await?;
            if let Some(message) = project_message(account, &sent, me).await {
                sink.send(Event::Incoming {
                    chat,
                    message: Box::new(message),
                });
            }
        }
        Command::SendFiles {
            chat,
            paths,
            caption,
            quoting,
            ..
        } => {
            let Some(reference) = peer_ref(session, sink, &chat).await else {
                return Ok(());
            };
            let reply_to = quoting.as_deref().and_then(|id| id.parse::<i32>().ok());
            for (index, path) in paths.iter().enumerate() {
                let photo = mime_of(path).starts_with("image/");
                let caption = (index == 0).then(|| caption.clone()).flatten();
                send_upload(client, reference, path, photo, None, caption, reply_to).await?;
            }
        }
        Command::SendImage {
            chat,
            width,
            height,
            rgba,
            caption,
            quoting,
            ..
        } => {
            let Some(reference) = peer_ref(session, sink, &chat).await else {
                return Ok(());
            };
            let Some(image) = image::RgbaImage::from_raw(width, height, rgba) else {
                return Ok(());
            };
            let path = dirs
                .media_cache_dir()
                .join(format!("telegram-upload-{}.png", std::process::id()));
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            image.save(&path)?;
            let reply_to = quoting.as_deref().and_then(|id| id.parse::<i32>().ok());
            send_upload(client, reference, &path, true, None, caption, reply_to).await?;
            let _ = std::fs::remove_file(&path);
        }
        Command::SendVoice {
            chat,
            samples,
            quoting,
        } => {
            let Some(reference) = peer_ref(session, sink, &chat).await else {
                return Ok(());
            };
            let seconds = samples.len() as f32 / crate::voice::RATE as f32;
            let duration = seconds.round().max(0.0) as i32;
            let ogg = match crate::voice::encode(&samples) {
                Ok(ogg) => ogg,
                Err(reason) => {
                    log::warn!("Voice note could not be encoded: {reason}");
                    return Ok(());
                }
            };
            let path = dirs
                .media_cache_dir()
                .join(format!("telegram-upload-{}.ogg", std::process::id()));
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(&path, ogg)?;
            let reply_to = quoting.as_deref().and_then(|id| id.parse::<i32>().ok());
            send_upload(
                client,
                reference,
                &path,
                false,
                Some(duration),
                None,
                reply_to,
            )
            .await?;
            let _ = std::fs::remove_file(&path);
        }
        Command::SendSticker {
            chat,
            path,
            quoting,
        } => {
            let Some(reference) = peer_ref(session, sink, &chat).await else {
                return Ok(());
            };
            // Telegram stickers travel as documents here; the bubble reads as
            // the file it is.
            let reply_to = quoting.as_deref().and_then(|id| id.parse::<i32>().ok());
            send_upload(client, reference, &path, false, None, None, reply_to).await?;
        }
        Command::EditText { chat, id, text, .. } => {
            let Some(reference) = peer_ref(session, sink, &chat).await else {
                return Ok(());
            };
            let Ok(id) = id.parse::<i32>() else {
                return Ok(());
            };
            client
                .edit_message(
                    reference,
                    id,
                    grammers_client::message::InputMessage::new().text(text),
                )
                .await?;
        }
        Command::Revoke { chat, id } | Command::DeleteLocal { chat, id } => {
            let Some(reference) = peer_ref(session, sink, &chat).await else {
                return Ok(());
            };
            let Ok(id) = id.parse::<i32>() else {
                return Ok(());
            };
            client.delete_messages(reference, &[id]).await?;
        }
        Command::Forward {
            from_chat,
            messages,
            to_chat,
        } => {
            let Some(from) = peer_ref(session, sink, &from_chat).await else {
                return Ok(());
            };
            let Some(to) = peer_ref(session, sink, &to_chat).await else {
                return Ok(());
            };
            let ids: Vec<i32> = messages
                .iter()
                .filter_map(|id| id.parse::<i32>().ok())
                .collect();
            if ids.is_empty() {
                return Ok(());
            }
            client.forward_messages(to, &ids, from).await?;
        }
        Command::Composing { chat, composing } => {
            let Some(reference) = peer_ref(session, sink, &chat).await else {
                return Ok(());
            };
            let action = client.action(reference);
            if composing {
                let _ = action
                    .oneshot(grammers_client::tl::enums::SendMessageAction::SendMessageTypingAction)
                    .await;
            } else {
                let _ = action.cancel().await;
            }
        }
        Command::MarkRead { chat, .. } => {
            let Some(reference) = peer_ref(session, sink, &chat).await else {
                return Ok(());
            };
            let _ = client
                .invoke(&grammers_client::tl::functions::messages::ReadHistory {
                    peer: reference.into(),
                    max_id: i32::MAX,
                })
                .await;
        }
        Command::LoadChat { chat, before } => {
            let before = before.and_then(|(_, id)| id.parse::<i32>().ok());
            page(client, session, sink, &chat, before, oldest, me).await?;
        }
        Command::FetchOlder(chat) => {
            let before = oldest.get(&chat).copied();
            page(client, session, sink, &chat, before, oldest, me).await?;
        }
        Command::SearchChatMessages { chat, query, .. } => {
            let Some(reference) = peer_ref(session, sink, &chat).await else {
                return Ok(());
            };
            let mut iter = client.search_messages(reference).query(&query).limit(30);
            let mut messages = Vec::new();
            while let Some(message) = iter.next().await? {
                if let Some(message) = project_message(account, &message, me).await {
                    messages.push(message);
                }
            }
            messages.reverse();
            sink.send(Event::ChatHits {
                chat,
                query,
                from: None,
                until: None,
                messages,
                truncated: false,
            });
        }
        Command::Download { chat, message, .. } => {
            let Some(reference) = peer_ref(session, sink, &chat).await else {
                return Ok(());
            };
            let Ok(id) = message.parse::<i32>() else {
                return Ok(());
            };
            let found = client.get_messages_by_id(reference, &[id]).await?;
            let Some(Some(original)) = found.into_iter().next() else {
                return Ok(());
            };
            if let Some(mut projected) = project_message(account, &original, me).await {
                let size = content_media(&projected.content).map(|media| media.size);
                if size.is_some_and(|size| size > DOWNLOAD_LIMIT) {
                    if let Some(media) = content_media_mut(&mut projected.content) {
                        media.state = MediaState::Failed(
                            "This file is too large to download here".to_owned(),
                        );
                    }
                } else {
                    let path = dirs
                        .media_cache_dir()
                        .join(format!("telegram-{}-{id}", account.get()));
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    original.download_media(&path).await?;
                    if let Some(media) = content_media_mut(&mut projected.content) {
                        media.path = Some(path);
                        media.state = MediaState::Idle;
                    }
                }
                sink.send(Event::MessageUpdated(Box::new(projected)));
            }
        }
        Command::React {
            chat,
            message,
            emoji,
        } => {
            let Some(reference) = peer_ref(session, sink, &chat).await else {
                return Ok(());
            };
            let Ok(id) = message.parse::<i32>() else {
                return Ok(());
            };
            let _ = client
                .invoke(&grammers_client::tl::functions::messages::SendReaction {
                    big: false,
                    add_to_recent: false,
                    peer: reference.into(),
                    msg_id: id,
                    reaction: Some(vec![grammers_client::tl::enums::Reaction::Emoji(
                        grammers_client::tl::types::ReactionEmoji { emoticon: emoji },
                    )]),
                })
                .await;
        }
        _ => {}
    }
    Ok(())
}

fn content_media(content: &Content) -> Option<&crate::model::Media> {
    match content {
        Content::Image { media, .. }
        | Content::Video { media, .. }
        | Content::Audio { media, .. }
        | Content::Document { media, .. }
        | Content::Sticker { media, .. } => Some(media),
        _ => None,
    }
}

fn content_media_mut(content: &mut Content) -> Option<&mut crate::model::Media> {
    match content {
        Content::Image { media, .. }
        | Content::Video { media, .. }
        | Content::Audio { media, .. }
        | Content::Document { media, .. }
        | Content::Sticker { media, .. } => Some(media),
        _ => None,
    }
}

async fn handle_update(
    account: AccountId,
    sink: &Sink,
    me: PeerId,
    update: grammers_client::update::Update,
) {
    match update {
        grammers_client::update::Update::NewMessage(message) => {
            if let Some(projected) = project_message(account, &message, me).await {
                sink.send(Event::Incoming {
                    chat: projected.chat.clone(),
                    message: Box::new(projected),
                });
            }
        }
        grammers_client::update::Update::MessageEdited(message) => {
            if let Some(projected) = project_message(account, &message, me).await {
                sink.send(Event::MessageUpdated(Box::new(projected)));
            }
        }
        grammers_client::update::Update::MessageDeleted(deletion) => {
            if let Some(channel) = deletion.channel_id()
                && let Some(peer) = PeerId::channel(channel)
                && let Some(chat) = chat_id(account, peer)
            {
                for id in deletion.messages() {
                    sink.send(Event::MessageDeleted {
                        chat: chat.clone(),
                        id: id.to_string(),
                    });
                }
            }
        }
        grammers_client::update::Update::Raw(raw) => match &raw.raw {
            grammers_client::tl::enums::Update::UserTyping(typing) => {
                if let Some(peer) = PeerId::user(typing.user_id)
                    && let Some(chat) = chat_id(account, peer)
                {
                    let composing = !matches!(
                        typing.action,
                        grammers_client::tl::enums::SendMessageAction::SendMessageCancelAction
                    );
                    sink.send(Event::Typing {
                        chat,
                        sender: peer.to_string(),
                        composing,
                    });
                }
            }
            grammers_client::tl::enums::Update::ChatUserTyping(typing) => {
                if let Some(peer) = PeerId::chat(typing.chat_id)
                    && let Some(chat) = chat_id(account, peer)
                {
                    let composing = !matches!(
                        typing.action,
                        grammers_client::tl::enums::SendMessageAction::SendMessageCancelAction
                    );
                    sink.send(Event::Typing {
                        chat,
                        sender: String::new(),
                        composing,
                    });
                }
            }
            _ => {}
        },
        _ => {}
    }
}
