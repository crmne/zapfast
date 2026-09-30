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
            let forum = matches!(
                &dialog.peer,
                grammers_client::peer::Peer::Channel(channel) if channel.raw.forum
            );
            let parent = chat.id.clone();
            chats.push(chat);
            if forum {
                match dialog.peer.to_ref().await {
                    Ok(Some(reference)) => {
                        match forum_topics(&client, reference, &parent, account).await {
                            Ok(mut topics) => chats.append(&mut topics),
                            Err(error) => log::warn!("Telegram topics could not load: {error}"),
                        }
                    }
                    Ok(None) => log::warn!("Telegram topics need an access hash"),
                    Err(error) => log::warn!("Telegram topics could not resolve: {error}"),
                }
            }
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

/// The topic rows of one forum. The General topic stays on the group's own
/// row because its messages carry no topic header.
async fn forum_topics(
    client: &Client,
    reference: PeerRef,
    parent: &ChatId,
    account: AccountId,
) -> Result<Vec<crate::model::Chat>> {
    let mut rows = Vec::new();
    let mut offset_topic = 0;
    for _ in 0..TOPIC_PAGES {
        let grammers_client::tl::enums::messages::ForumTopics::Topics(page) = client
            .invoke(&grammers_client::tl::functions::messages::GetForumTopics {
                peer: reference.into(),
                q: None,
                offset_date: 0,
                offset_id: 0,
                offset_topic,
                limit: TOPIC_PAGE,
            })
            .await?;
        let count = page.topics.len();
        let mut last = 0;
        for topic in page.topics {
            let grammers_client::tl::enums::ForumTopic::Topic(topic) = topic else {
                continue;
            };
            if topic.id == 1 {
                continue;
            }
            last = topic.id;
            if let Some(chat) = project::topic_chat(account, reference.id, parent, &topic) {
                rows.push(chat);
            }
        }
        if count < TOPIC_PAGE as usize {
            break;
        }
        offset_topic = last;
    }
    Ok(rows)
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
    project::message(account, message, Some(me), quoted, None)
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
    let topic = project::parse_chat(chat.peer()).and_then(|(_, topic)| topic);
    let mut items: Vec<(Option<i64>, crate::model::Message)> = Vec::new();
    if let Some(topic) = topic {
        // A topic page reads the group's recent history and keeps this
        // topic's rows; grammers has no topic paging of its own, and a busy
        // group's messages for other topics are left behind.
        let mut iter = client.iter_messages(reference).limit(TOPIC_SCAN);
        if let Some(before) = before {
            iter = iter.offset_id(before);
        }
        while items.len() <= PAGE {
            let Some(message) = iter.next().await? else {
                break;
            };
            if project::topic_of(&message.raw) != Some(topic) {
                continue;
            }
            if let Some(projected) = project_message(account, &message, me).await {
                items.push((message.grouped_id(), projected));
            }
        }
    } else {
        let mut iter = client.iter_messages(reference).limit(PAGE + 1);
        if let Some(before) = before {
            iter = iter.offset_id(before);
        }
        while let Some(message) = iter.next().await? {
            if let Some(projected) = project_message(account, &message, me).await {
                items.push((message.grouped_id(), projected));
            }
        }
    }
    let older = items.len() > PAGE;
    items.truncate(PAGE);
    items.reverse();
    let messages = project::album_bubbles(items);
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

/// How many topic rows one forum page carries, and how many pages the topics
/// of one forum are read through.
const TOPIC_PAGE: i32 = 100;
const TOPIC_PAGES: usize = 10;

/// How many recent group messages a topic page scans for its own rows.
const TOPIC_SCAN: usize = 500;

/// Where a send goes: the forum topic when the chat is one, and the message
/// it replies to.
#[derive(Clone, Copy)]
struct Placement {
    topic: Option<i64>,
    reply_to: Option<i32>,
}

impl Placement {
    /// Reads the placement out of a chat key and an optional quoted id.
    fn of(chat: &ChatId, quoting: Option<&str>) -> Self {
        let topic = project::parse_chat(chat.peer()).and_then(|(_, topic)| topic);
        let reply_to = quoting.and_then(|id| id.parse::<i32>().ok());
        Self { topic, reply_to }
    }
}

/// The reply target a send carries: a topic root, a quoted message, or both.
fn reply_to(
    topic: Option<i64>,
    reply: Option<i32>,
) -> Option<grammers_client::tl::enums::InputReplyTo> {
    if topic.is_none() && reply.is_none() {
        return None;
    }
    Some(
        grammers_client::tl::types::InputReplyToMessage {
            reply_to_msg_id: reply.unwrap_or(0),
            top_msg_id: topic.map(|topic| topic as i32),
            reply_to_peer_id: None,
            quote_text: None,
            quote_entities: None,
            quote_offset: None,
            monoforum_peer_id: None,
            todo_item_id: None,
            poll_option: None,
        }
        .into(),
    )
}

/// A fresh id Telegram uses to recognize duplicate sends.
fn random_id() -> i64 {
    use std::sync::atomic::{AtomicI64, Ordering};
    static LAST: AtomicI64 = AtomicI64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as i64)
        .unwrap_or(0);
    now.wrapping_add(LAST.fetch_add(1, Ordering::Relaxed))
}

/// Sends text. A forum topic needs the raw call because the high-level
/// client has no topic parameter; the stream echoes the sent row back.
async fn send_text(
    client: &Client,
    reference: PeerRef,
    text: String,
    placement: Placement,
) -> Result<Option<TelegramMessage>> {
    if placement.topic.is_some() {
        client
            .invoke(&grammers_client::tl::functions::messages::SendMessage {
                no_webpage: false,
                silent: false,
                background: false,
                clear_draft: false,
                noforwards: false,
                update_stickersets_order: false,
                invert_media: false,
                allow_paid_floodskip: false,
                peer: reference.into(),
                reply_to: reply_to(placement.topic, placement.reply_to),
                message: text,
                random_id: random_id(),
                reply_markup: None,
                entities: None,
                schedule_date: None,
                schedule_repeat_period: None,
                send_as: None,
                quick_reply_shortcut: None,
                effect: None,
                allow_paid_stars: None,
                suggested_post: None,
                rich_message: None,
            })
            .await?;
        return Ok(None);
    }
    let mut input = grammers_client::message::InputMessage::new().text(text);
    if let Some(id) = placement.reply_to {
        input = input.reply_to(Some(id));
    }
    Ok(Some(client.send_message(reference, input).await?))
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
    placement: Placement,
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
    if placement.topic.is_some() {
        client
            .invoke(&grammers_client::tl::functions::messages::SendMedia {
                silent: false,
                background: false,
                clear_draft: false,
                noforwards: false,
                update_stickersets_order: false,
                invert_media: false,
                allow_paid_floodskip: false,
                peer: reference.into(),
                reply_to: reply_to(placement.topic, placement.reply_to),
                media,
                message: caption.unwrap_or_default(),
                random_id: random_id(),
                reply_markup: None,
                entities: None,
                schedule_date: None,
                schedule_repeat_period: None,
                send_as: None,
                quick_reply_shortcut: None,
                effect: None,
                allow_paid_stars: None,
                suggested_post: None,
            })
            .await?;
        return Ok(());
    }
    let mut input = grammers_client::message::InputMessage::new().media(media);
    if let Some(caption) = caption {
        input = input.text(caption);
    }
    if let Some(id) = placement.reply_to {
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
            let placement = Placement::of(&chat, quoting.as_deref());
            if let Some(sent) = send_text(client, reference, text, placement).await?
                && let Some(message) = project_message(account, &sent, me).await
            {
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
            let placement = Placement::of(&chat, quoting.as_deref());
            for (index, path) in paths.iter().enumerate() {
                let photo = mime_of(path).starts_with("image/");
                let caption = (index == 0).then(|| caption.clone()).flatten();
                send_upload(client, reference, path, photo, None, caption, placement).await?;
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
            let placement = Placement::of(&chat, quoting.as_deref());
            send_upload(client, reference, &path, true, None, caption, placement).await?;
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
            let placement = Placement::of(&chat, quoting.as_deref());
            send_upload(
                client,
                reference,
                &path,
                false,
                Some(duration),
                None,
                placement,
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
            let placement = Placement::of(&chat, quoting.as_deref());
            send_upload(client, reference, &path, false, None, None, placement).await?;
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
            let topic = project::parse_chat(chat.peer()).and_then(|(_, topic)| topic);
            let action = match topic {
                Some(topic) => action.topic_id(topic as i32),
                None => action,
            };
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
        Command::VotePoll {
            chat,
            message,
            choices,
        } => {
            let Some(reference) = peer_ref(session, sink, &chat).await else {
                return Ok(());
            };
            let Ok(id) = message.parse::<i32>() else {
                return Ok(());
            };
            let options = match poll_options(client, reference, id, &choices).await {
                Ok(options) => options,
                Err(error) => {
                    sink.send(Event::PollVoted {
                        chat,
                        message,
                        error: Some(error.to_string()),
                    });
                    return Ok(());
                }
            };
            match client
                .invoke(&grammers_client::tl::functions::messages::SendVote {
                    peer: reference.into(),
                    msg_id: id,
                    options,
                })
                .await
            {
                Ok(_) => {
                    sink.send(Event::PollVoted {
                        chat: chat.clone(),
                        message: message.clone(),
                        error: None,
                    });
                    let found = client.get_messages_by_id(reference, &[id]).await?;
                    if let Some(Some(original)) = found.into_iter().next()
                        && let Some(projected) = project_message(account, &original, me).await
                    {
                        sink.send(Event::MessageUpdated(Box::new(projected)));
                    }
                }
                Err(error) => {
                    sink.send(Event::PollVoted {
                        chat,
                        message,
                        error: Some(error.to_string()),
                    });
                }
            }
        }
        Command::CreatePoll { chat, draft } => {
            let Some(reference) = peer_ref(session, sink, &chat).await else {
                return Ok(());
            };
            let placement = Placement::of(&chat, None);
            let media = poll_media(&draft);
            match client
                .invoke(&grammers_client::tl::functions::messages::SendMedia {
                    silent: false,
                    background: false,
                    clear_draft: false,
                    noforwards: false,
                    update_stickersets_order: false,
                    invert_media: false,
                    allow_paid_floodskip: false,
                    peer: reference.into(),
                    reply_to: reply_to(placement.topic, placement.reply_to),
                    media,
                    message: String::new(),
                    random_id: random_id(),
                    reply_markup: None,
                    entities: None,
                    schedule_date: None,
                    schedule_repeat_period: None,
                    send_as: None,
                    quick_reply_shortcut: None,
                    effect: None,
                    allow_paid_stars: None,
                    suggested_post: None,
                })
                .await
            {
                Ok(_) => sink.send(Event::PollCreated { chat, error: None }),
                Err(error) => sink.send(Event::PollCreated {
                    chat,
                    error: Some(error.to_string()),
                }),
            }
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

/// The option bytes behind the chosen answer indices of a poll message.
async fn poll_options(
    client: &Client,
    reference: PeerRef,
    id: i32,
    choices: &[usize],
) -> Result<Vec<Vec<u8>>> {
    let found = client.get_messages_by_id(reference, &[id]).await?;
    let Some(Some(original)) = found.into_iter().next() else {
        anyhow::bail!("The poll message is gone");
    };
    let Some(grammers_client::media::Media::Poll(poll)) = original.media() else {
        anyhow::bail!("The message is not a poll");
    };
    let answers: Vec<Vec<u8>> = poll
        .iter_answers()
        .filter_map(|answer| match answer {
            grammers_client::tl::enums::PollAnswer::Answer(answer) => Some(answer.option.clone()),
            _ => None,
        })
        .collect();
    let options: Vec<Vec<u8>> = choices
        .iter()
        .filter_map(|choice| answers.get(*choice).cloned())
        .collect();
    if options.is_empty() {
        anyhow::bail!("The vote does not name a poll answer");
    }
    Ok(options)
}

/// A fresh Telegram poll carrying the interface's draft.
fn poll_media(draft: &crate::model::PollDraft) -> grammers_client::tl::enums::InputMedia {
    let text = |text: &str| {
        grammers_client::tl::enums::TextWithEntities::Entities(
            grammers_client::tl::types::TextWithEntities {
                text: text.to_owned(),
                entities: Vec::new(),
            },
        )
    };
    let poll = grammers_client::tl::types::Poll {
        id: 0,
        closed: false,
        public_voters: false,
        multiple_choice: draft.multiple,
        quiz: false,
        open_answers: false,
        revoting_disabled: false,
        shuffle_answers: false,
        hide_results_until_close: false,
        creator: false,
        subscribers_only: false,
        question: text(&draft.question),
        answers: draft
            .options
            .iter()
            .map(|option| {
                grammers_client::tl::enums::PollAnswer::Answer(
                    grammers_client::tl::types::PollAnswer {
                        text: text(option),
                        option: random_id().to_le_bytes().to_vec(),
                        media: None,
                        added_by: None,
                        date: None,
                    },
                )
            })
            .collect(),
        close_period: None,
        close_date: None,
        countries_iso2: None,
        hash: 0,
    };
    grammers_client::tl::types::InputMediaPoll {
        poll: grammers_client::tl::enums::Poll::Poll(poll),
        correct_answers: None,
        attached_media: None,
        solution: None,
        solution_entities: None,
        solution_media: None,
    }
    .into()
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
