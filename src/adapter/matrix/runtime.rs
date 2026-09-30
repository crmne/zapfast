//! Matrix account runtime: login, rooms, sync, verification, and commands.
//!
//! One account talks to one Matrix homeserver as one user. The runtime owns
//! the matrix-sdk client, projects events through [`super::project`], and
//! speaks the same [`Command`] and [`Event`] types as the other workers so the
//! host can route both the same way.
//!
//! In encrypted rooms the window only paints decrypted content once this
//! session's device is verified; until then those messages show a placeholder.
//! Verification uses the emoji SAS method on the to-device channel.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use futures_util::future::BoxFuture;
use matrix_sdk::attachment::AttachmentConfig;
use matrix_sdk::config::SyncSettings;
use matrix_sdk::encryption::VerificationState;
use matrix_sdk::encryption::verification::{
    SasState, SasVerification, VerificationRequest, VerificationRequestState,
};
use matrix_sdk::latest_events::LatestEventValue;
use matrix_sdk::media::{MediaFormat, MediaRequestParameters};
use matrix_sdk::room::edit::EditedContent;
use matrix_sdk::room::reply::{EnforceThread, Reply};
use matrix_sdk::room::{MessagesOptions, ParentSpace, Room};
use matrix_sdk::ruma::api::client::receipt::create_receipt::v3::ReceiptType;
use matrix_sdk::ruma::events::key::verification::request::ToDeviceKeyVerificationRequestEvent;
use matrix_sdk::ruma::events::reaction::{ReactionEventContent, SyncReactionEvent};
use matrix_sdk::ruma::events::receipt::ReceiptThread;
use matrix_sdk::ruma::events::relation::Annotation;
use matrix_sdk::ruma::events::room::MediaSource;
use matrix_sdk::ruma::events::room::message::{
    AddMentions, OriginalSyncRoomMessageEvent, Relation, RoomMessageEventContent,
    RoomMessageEventContentWithoutRelation, SyncRoomMessageEvent, TextMessageEventContent,
};
use matrix_sdk::ruma::events::room::redaction::SyncRoomRedactionEvent;
use matrix_sdk::ruma::events::sticker::SyncStickerEvent;
use matrix_sdk::ruma::events::typing::SyncTypingEvent;
use matrix_sdk::ruma::events::{AnySyncMessageLikeEvent, AnySyncTimelineEvent};
use matrix_sdk::ruma::{OwnedEventId, UInt, UserId};
use matrix_sdk::{Client, EncryptionState};
use mime::Mime;
use tokio::sync::mpsc;

use crate::account::{AccountId, AuthState};
use crate::backend::{Command, Event, LoginStep, VerificationPrompt, VerifyAction, Waker};
use crate::model::{
    Chat, ChatId, ChatKind, Content, Delivery, LastMessage, MediaState, Message, Quoted,
};
use crate::paths::AppDirs;

use super::project;

/// One page of messages. One extra row is fetched to learn whether older
/// messages exist.
const PAGE: usize = 50;

/// How long a verification request waits for the other side to be ready.
const VERIFY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

/// Sends events and asks the window to repaint, like the other workers.
#[derive(Clone)]
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

/// One reaction row per emoji and sender, keyed by chat and message id.
type ReactionRows = HashMap<(ChatId, String), Vec<(String, String)>>;

/// What every task of one account shares: the client, the rooms it knows,
/// and the small caches the projections need.
#[derive(Clone)]
struct Shared {
    account: AccountId,
    me: matrix_sdk::ruma::OwnedUserId,
    client: Client,
    sink: Sink,
    rooms: Arc<Mutex<HashMap<ChatId, Room>>>,
    /// One row per emoji and sender, keyed by chat and message id.
    reactions: Arc<Mutex<ReactionRows>>,
    /// Where a message's media can be fetched from.
    sources: Arc<Mutex<HashMap<(ChatId, String), MediaSource>>>,
    /// The newest event id seen per chat, for read receipts.
    latest: Arc<Mutex<HashMap<ChatId, OwnedEventId>>>,
    /// Chats whose encrypted messages are waiting for verification.
    gated: Arc<Mutex<HashSet<ChatId>>>,
    /// The token that pages further back per chat.
    tokens: Arc<Mutex<HashMap<ChatId, String>>>,
    /// The last user that typed per chat, so a stop can be emitted.
    typers: Arc<Mutex<HashMap<ChatId, String>>>,
    /// Where a running verification accepts match, mismatch, or cancel.
    slot: Arc<tokio::sync::Mutex<Option<mpsc::UnboundedSender<VerifyAction>>>>,
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
        log::warn!("Matrix account {account} stopped: {error}");
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
    let store = dirs.account_dir(account).join("matrix");
    crate::paths::create_private_dir(&store)
        .with_context(|| format!("Could not create {}", store.display()))?;
    let homeserver_file = store.join("homeserver.txt");
    let mut homeserver = std::fs::read_to_string(&homeserver_file)
        .ok()
        .map(|saved| saved.trim().to_owned())
        .filter(|saved| !saved.is_empty());
    let mut client = match &homeserver {
        Some(homeserver) => match build_client(&store, homeserver).await {
            Ok(client) => Some(client),
            Err(error) => {
                log::warn!("Matrix could not open its store: {error}");
                None
            }
        },
        None => None,
    };
    let mut auth = match &client {
        Some(client) if client.matrix_auth().logged_in() => AuthState::Ready,
        _ => AuthState::SignedOut,
    };
    sink.send(Event::Auth {
        account,
        state: auth.clone(),
    });

    while auth != AuthState::Ready {
        let Some(command) = commands.recv().await else {
            return Ok(());
        };
        match command {
            Command::Shutdown => return Ok(()),
            Command::Login {
                account: login_account,
                step,
            } if login_account == account => {
                let LoginStep::MatrixPassword {
                    homeserver: server,
                    user,
                    password,
                } = step
                else {
                    continue;
                };
                let server = server.trim().to_owned();
                if server.is_empty() {
                    auth = AuthState::Failed {
                        reason: "Enter the homeserver address".to_owned(),
                    };
                    sink.send(Event::Auth {
                        account,
                        state: auth.clone(),
                    });
                    continue;
                }
                if homeserver.as_deref() != Some(server.as_str()) {
                    match build_client(&store, &server).await {
                        Ok(built) => {
                            client = Some(built);
                            homeserver = Some(server.clone());
                        }
                        Err(error) => {
                            auth = failed(&error);
                            sink.send(Event::Auth {
                                account,
                                state: auth.clone(),
                            });
                            continue;
                        }
                    }
                }
                let Some(built) = client.as_ref() else {
                    continue;
                };
                match built
                    .matrix_auth()
                    .login_username(user.trim(), &password)
                    .send()
                    .await
                {
                    Ok(_) => {
                        auth = AuthState::Ready;
                        if let Err(error) = std::fs::write(&homeserver_file, format!("{server}\n"))
                        {
                            log::warn!("Matrix could not remember its homeserver: {error}");
                        }
                    }
                    Err(error) => auth = failed(&error),
                }
            }
            _ => {}
        }
        sink.send(Event::Auth {
            account,
            state: auth.clone(),
        });
    }

    let Some(client) = client else {
        return Ok(());
    };
    let me = client
        .user_id()
        .context("The homeserver did not return a user id")?
        .to_owned();
    let name = client.account().get_display_name().await.ok().flatten();
    sink.send(Event::Me {
        id: me.to_string(),
        lid: None,
        name,
        about: None,
    });

    let shared = Shared {
        account,
        me,
        client: client.clone(),
        sink: sink.clone(),
        rooms: Arc::new(Mutex::new(HashMap::new())),
        reactions: Arc::new(Mutex::new(HashMap::new())),
        sources: Arc::new(Mutex::new(HashMap::new())),
        latest: Arc::new(Mutex::new(HashMap::new())),
        gated: Arc::new(Mutex::new(HashSet::new())),
        tokens: Arc::new(Mutex::new(HashMap::new())),
        typers: Arc::new(Mutex::new(HashMap::new())),
        slot: Arc::new(tokio::sync::Mutex::new(None)),
    };
    let (verify_tx, mut verify_rx) = mpsc::unbounded_channel();

    {
        let inbox = verify_tx.clone();
        client.add_event_handler(move |event: ToDeviceKeyVerificationRequestEvent| {
            let inbox = inbox.clone();
            async move {
                on_verify_request(event, &inbox);
            }
        });
    }
    {
        let shared = shared.clone();
        client.add_event_handler(move |event: SyncRoomMessageEvent, room: Room| {
            let shared = shared.clone();
            async move {
                on_message(&shared, event, room).await;
            }
        });
    }
    {
        let shared = shared.clone();
        client.add_event_handler(move |event: SyncStickerEvent, room: Room| {
            let shared = shared.clone();
            async move {
                on_sticker(&shared, event, room).await;
            }
        });
    }
    {
        let shared = shared.clone();
        client.add_event_handler(move |event: SyncReactionEvent, room: Room| {
            let shared = shared.clone();
            async move {
                on_reaction(&shared, event, room).await;
            }
        });
    }
    {
        let shared = shared.clone();
        client.add_event_handler(move |event: SyncRoomRedactionEvent, room: Room| {
            let shared = shared.clone();
            async move {
                on_redaction(&shared, event, room).await;
            }
        });
    }
    {
        let shared = shared.clone();
        client.add_event_handler(move |event: SyncTypingEvent, room: Room| {
            let shared = shared.clone();
            async move {
                on_typing(&shared, event, room).await;
            }
        });
    }

    if let Err(error) = client.sync_once(SyncSettings::default()).await {
        log::warn!("Matrix first sync failed: {error}");
    }
    emit_chats(&shared).await;
    spawn_sync(client.clone());

    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else {
                    return Ok(());
                };
                if matches!(command, Command::Shutdown) {
                    return Ok(());
                }
                if let Command::VerifySession { account: target, action } = &command {
                    if *target == account {
                        match *action {
                            VerifyAction::Start => {
                                tokio::spawn(start_verification(&shared));
                            }
                            reply => {
                                let slot = shared.slot.lock().await;
                                if let Some(sender) = slot.as_ref() {
                                    let _ = sender.send(reply);
                                }
                            }
                        }
                    }
                    continue;
                }
                if let Err(error) = handle_command(&dirs, &shared, command).await {
                    log::warn!("Matrix command failed: {error}");
                }
            }
            request = verify_rx.recv() => {
                let Some((sender, transaction)) = request else {
                    continue;
                };
                tokio::spawn(accept_verification(&shared, sender, transaction));
            }
        }
    }
}

async fn build_client(store: &Path, homeserver: &str) -> Result<Client> {
    Client::builder()
        .homeserver_url(homeserver)
        .sqlite_store(store, None::<&str>)
        .build()
        .await
        .context("The Matrix client could not start")
}

/// Keeps receiving updates for as long as the account is signed in. The sync
/// future is driven on its own thread with a current-thread runtime, so the
/// SDK's recursive decrypt future never needs a `Send` proof.
fn spawn_sync(client: Client) {
    let spawned = std::thread::Builder::new()
        .name("zapfast-matrix-sync".to_owned())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    log::warn!("Matrix sync could not start: {error}");
                    return;
                }
            };
            runtime.block_on(async move {
                if let Err(error) = client.sync(SyncSettings::default()).await {
                    log::warn!("Matrix sync stopped: {error}");
                }
            });
        });
    if let Err(error) = spawned {
        log::warn!("Matrix sync could not start: {error}");
    }
}

fn failed(error: &impl std::fmt::Display) -> AuthState {
    AuthState::Failed {
        reason: error.to_string(),
    }
}

fn verified(shared: &Shared) -> bool {
    matches!(
        shared.client.encryption().verification_state().get(),
        VerificationState::Verified
    )
}

fn encrypted(room: &Room) -> bool {
    matches!(room.encryption_state(), EncryptionState::Encrypted)
}

/// What an encrypted message shows before this session is verified.
fn gated_content() -> Content {
    Content::Unsupported {
        what: "Encrypted message. Verify this session to read it.".to_owned(),
    }
}

fn chat_room(shared: &Shared, chat: &ChatId) -> Option<Room> {
    if let Some(room) = shared.rooms.lock().unwrap().get(chat) {
        return Some(room.clone());
    }
    let room_id = project::parse_room(chat)?;
    shared.client.get_room(&room_id)
}

fn reactions_of(shared: &Shared, chat: &ChatId, id: &str) -> Vec<crate::model::Reaction> {
    let rows = shared
        .reactions
        .lock()
        .unwrap()
        .get(&(chat.clone(), id.to_owned()))
        .cloned()
        .unwrap_or_default();
    project::reactions(&rows, &shared.me)
}

fn media_source(content: &RoomMessageEventContent) -> Option<MediaSource> {
    match &content.msgtype {
        matrix_sdk::ruma::events::room::message::MessageType::Image(image) => {
            Some(image.source.clone())
        }
        matrix_sdk::ruma::events::room::message::MessageType::File(file) => {
            Some(file.source.clone())
        }
        matrix_sdk::ruma::events::room::message::MessageType::Audio(audio) => {
            Some(audio.source.clone())
        }
        matrix_sdk::ruma::events::room::message::MessageType::Video(video) => {
            Some(video.source.clone())
        }
        _ => None,
    }
}

async fn member_name(_shared: &Shared, room: &Room, user: &UserId) -> Option<String> {
    match room.get_member_no_sync(user).await {
        Ok(Some(member)) => member.display_name().map(str::to_owned),
        _ => None,
    }
}

/// Builds the reply preview by reading the message it replies to. A missing
/// original still leaves a reply marker behind.
async fn quoted_for(
    shared: &Shared,
    room: &Room,
    content: &RoomMessageEventContent,
) -> Option<Quoted> {
    let Some(Relation::Reply(reply)) = &content.relates_to else {
        return None;
    };
    let id = reply.in_reply_to.event_id.clone();
    let event = room.event(&id, None).await.ok()?;
    let summary = summary_of(shared, room, &event)
        .await
        .unwrap_or_else(|| "Message".to_owned());
    Some(Quoted {
        id: id.to_string(),
        sender: event
            .sender()
            .map(|sender| sender.to_string())
            .unwrap_or_default(),
        sender_name: None,
        summary,
        mentions: Vec::new(),
    })
}

/// The one-line summary of a timeline event, if it is a room message.
async fn summary_of(
    shared: &Shared,
    room: &Room,
    event: &matrix_sdk::deserialized_responses::TimelineEvent,
) -> Option<String> {
    let raw = event.raw().deserialize().ok()?;
    let AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(message)) = raw
    else {
        return None;
    };
    let content = project_message_content(shared, room, &message)?;
    Some(content.summary())
}

/// The message content after edits are applied, or the placeholder in an
/// encrypted room this session cannot read yet.
fn project_message_content(
    shared: &Shared,
    room: &Room,
    message: &SyncRoomMessageEvent,
) -> Option<Content> {
    let SyncRoomMessageEvent::Original(original) = message else {
        return Some(Content::Revoked);
    };
    if encrypted(room) && !verified(shared) {
        return Some(gated_content());
    }
    let mut content = original.content.clone();
    if let Some(Relation::Replacement(replacement)) = &original.content.relates_to {
        content.apply_replacement(replacement.new_content.clone());
    }
    Some(project::content(&content))
}

#[allow(clippy::too_many_arguments)]
async fn project_room_message(
    shared: &Shared,
    room: &Room,
    original: &OriginalSyncRoomMessageEvent,
) -> Option<Message> {
    let chat = project::chat_id(shared.account, room.room_id());
    let (id, edited) = match &original.content.relates_to {
        Some(Relation::Replacement(replacement)) => (replacement.event_id.to_string(), true),
        _ => (original.event_id.to_string(), false),
    };
    if !(encrypted(room) && !verified(shared))
        && let Some(source) = media_source(&original.content)
    {
        shared
            .sources
            .lock()
            .unwrap()
            .insert((chat.clone(), id.clone()), source);
    }
    let member = member_name(shared, room, &original.sender).await;
    let quoted = quoted_for(shared, room, &original.content).await;
    let reactions = reactions_of(shared, &chat, &id);
    let mut content = original.content.clone();
    if let Some(Relation::Replacement(replacement)) = &original.content.relates_to {
        content.apply_replacement(replacement.new_content.clone());
    }
    let content = if encrypted(room) && !verified(shared) {
        shared.gated.lock().unwrap().insert(chat.clone());
        gated_content()
    } else {
        project::content(&content)
    };
    let from_me = original.sender == shared.me;
    let mut message = project::message(
        shared.account,
        room.room_id(),
        &original.event_id,
        &original.sender,
        member,
        u64::from(original.origin_server_ts.get()),
        content,
        from_me,
        quoted,
        reactions,
    );
    message.id = id.clone();
    message.edited = edited;
    shared
        .latest
        .lock()
        .unwrap()
        .insert(chat, original.event_id.clone());
    Some(message)
}

async fn on_message(shared: &Shared, event: SyncRoomMessageEvent, room: Room) {
    let chat = project::chat_id(shared.account, room.room_id());
    shared
        .rooms
        .lock()
        .unwrap()
        .insert(chat.clone(), room.clone());
    match &event {
        SyncRoomMessageEvent::Redacted(redacted) => {
            shared.sink.send(Event::MessageDeleted {
                chat,
                id: redacted.event_id.to_string(),
            });
        }
        SyncRoomMessageEvent::Original(original) => {
            if let Some(message) = project_room_message(shared, &room, original).await {
                shared.sink.send(Event::Incoming {
                    chat,
                    message: Box::new(message),
                });
            }
        }
    }
}

async fn on_sticker(shared: &Shared, event: SyncStickerEvent, room: Room) {
    let chat = project::chat_id(shared.account, room.room_id());
    shared
        .rooms
        .lock()
        .unwrap()
        .insert(chat.clone(), room.clone());
    let SyncStickerEvent::Original(original) = event else {
        return;
    };
    let id = original.event_id.to_string();
    let source: MediaSource = original.content.source.clone().into();
    let content = if encrypted(&room) && !verified(shared) {
        shared.gated.lock().unwrap().insert(chat.clone());
        gated_content()
    } else {
        shared
            .sources
            .lock()
            .unwrap()
            .insert((chat.clone(), id.clone()), source.clone());
        project::sticker_content(&source, Some(&original.content.info))
    };
    let member = member_name(shared, &room, &original.sender).await;
    let from_me = original.sender == shared.me;
    let mut message = project::message(
        shared.account,
        room.room_id(),
        &original.event_id,
        &original.sender,
        member,
        u64::from(original.origin_server_ts.get()),
        content,
        from_me,
        None,
        Vec::new(),
    );
    message.id = id;
    shared
        .latest
        .lock()
        .unwrap()
        .insert(chat.clone(), original.event_id.clone());
    shared.sink.send(Event::Incoming {
        chat,
        message: Box::new(message),
    });
}

async fn on_reaction(shared: &Shared, event: SyncReactionEvent, room: Room) {
    let SyncReactionEvent::Original(original) = event else {
        return;
    };
    let chat = project::chat_id(shared.account, room.room_id());
    let target = original.content.relates_to.event_id.clone();
    let sender = original.sender.to_string();
    let emoji = original.content.relates_to.key.clone();
    {
        let mut map = shared.reactions.lock().unwrap();
        let rows = map.entry((chat.clone(), target.to_string())).or_default();
        rows.retain(|(_, existing)| existing != &sender);
        rows.push((emoji, sender.clone()));
    }
    if let Ok(event) = room.event(&target, None).await
        && let Ok(AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(
            SyncRoomMessageEvent::Original(message),
        ))) = event.raw().deserialize()
        && let Some(message) = project_room_message(shared, &room, &message).await
    {
        shared.sink.send(Event::MessageUpdated(Box::new(message)));
    }
}

async fn on_redaction(shared: &Shared, event: SyncRoomRedactionEvent, room: Room) {
    let SyncRoomRedactionEvent::Original(original) = event else {
        return;
    };
    let Some(target) = original.redacts else {
        return;
    };
    let chat = project::chat_id(shared.account, room.room_id());
    shared.sink.send(Event::MessageDeleted {
        chat,
        id: target.to_string(),
    });
}

async fn on_typing(shared: &Shared, event: SyncTypingEvent, room: Room) {
    let chat = project::chat_id(shared.account, room.room_id());
    let other = event
        .content
        .user_ids
        .iter()
        .find(|user| **user != shared.me)
        .map(ToString::to_string);
    match other {
        Some(sender) => {
            shared
                .typers
                .lock()
                .unwrap()
                .insert(chat.clone(), sender.clone());
            shared.sink.send(Event::Typing {
                chat,
                sender,
                composing: true,
            });
        }
        None => {
            if let Some(sender) = shared.typers.lock().unwrap().remove(&chat) {
                shared.sink.send(Event::Typing {
                    chat,
                    sender,
                    composing: false,
                });
            }
        }
    }
}

fn on_verify_request(
    event: ToDeviceKeyVerificationRequestEvent,
    inbox: &mpsc::UnboundedSender<(matrix_sdk::ruma::OwnedUserId, String)>,
) {
    let _ = inbox.send((event.sender, event.content.transaction_id.to_string()));
}

/// The room list the window shows. Spaces are kept too: the window draws them
/// in the space rail instead of the chat list.
async fn emit_chats(shared: &Shared) {
    let mut chats = Vec::new();
    for room in shared.client.rooms() {
        let name = match room.cached_display_name() {
            Some(name) => name.to_string(),
            None => match room.name() {
                Some(name) => name,
                None => continue,
            },
        };
        let chat_id = project::chat_id(shared.account, room.room_id());
        let mut chat = Chat::new(chat_id.clone(), name);
        chat.space = room.is_space();
        chat.unread = room.num_unread_messages().min(u32::MAX as u64) as u32;
        chat.kind = if room.is_direct().await.unwrap_or(false) {
            ChatKind::Direct
        } else {
            ChatKind::Group
        };
        if let LatestEventValue::Remote(event) = room.latest_event() {
            chat.last_activity = event
                .timestamp()
                .map(|timestamp| (u64::from(timestamp.get()) / 1000) as i64)
                .unwrap_or(0);
            if let Some((from_me, sender, summary)) = preview_of(shared, &room, &event) {
                let full = summary.clone();
                chat.last = Some(LastMessage {
                    from_me,
                    sender,
                    sender_name: None,
                    summary,
                    full,
                    status: if from_me {
                        Delivery::Sent
                    } else {
                        Delivery::None
                    },
                });
            }
        }
        if !chat.space
            && let Ok(spaces) = room.parent_spaces().await
        {
            let mut spaces = Box::pin(spaces);
            while let Some(space) = spaces.next().await {
                if let Ok(ParentSpace::Reciprocal(parent)) = space {
                    chat.parent = Some(project::chat_id(shared.account, parent.room_id()));
                    break;
                }
            }
        }
        shared.rooms.lock().unwrap().insert(chat_id, room);
        chats.push(chat);
    }
    shared.sink.send(Event::Chats(chats));
}

fn preview_of(
    shared: &Shared,
    room: &Room,
    event: &matrix_sdk::deserialized_responses::TimelineEvent,
) -> Option<(bool, String, String)> {
    let raw = event.raw().deserialize().ok()?;
    let AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(message)) = raw
    else {
        return None;
    };
    let sender = event.sender()?;
    let content = project_message_content(shared, room, &message)?;
    Some((sender == shared.me, sender.to_string(), content.summary()))
}

fn reply_for(quoting: Option<&str>) -> Option<Reply> {
    let event_id: OwnedEventId = quoting?.parse().ok()?;
    Some(Reply {
        event_id,
        enforce_thread: EnforceThread::Unthreaded,
        add_mentions: AddMentions::No,
    })
}

fn mime_for(path: &Path) -> Mime {
    let fallback: Mime = "application/octet-stream".parse().unwrap();
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png".parse().unwrap_or(fallback),
        Some("jpg" | "jpeg") => "image/jpeg".parse().unwrap_or(fallback),
        Some("gif") => "image/gif".parse().unwrap_or(fallback),
        Some("webp") => "image/webp".parse().unwrap_or(fallback),
        Some("mp4") => "video/mp4".parse().unwrap_or(fallback),
        Some("ogg" | "opus") => "audio/ogg".parse().unwrap_or(fallback),
        Some("mp3") => "audio/mpeg".parse().unwrap_or(fallback),
        Some("pdf") => "application/pdf".parse().unwrap_or(fallback),
        Some("txt") => "text/plain".parse().unwrap_or(fallback),
        _ => fallback,
    }
}

fn safe_name(id: &str) -> String {
    id.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '-'
            }
        })
        .collect()
}

/// Paints a page of a room, oldest first, and keeps the token that pages
/// further back.
async fn page(shared: &Shared, chat: &ChatId, before: Option<String>) -> Result<()> {
    let Some(room) = chat_room(shared, chat) else {
        shared.sink.send(Event::Messages {
            chat: chat.clone(),
            messages: Vec::new(),
            older: false,
            complete: true,
        });
        return Ok(());
    };
    shared
        .rooms
        .lock()
        .unwrap()
        .insert(chat.clone(), room.clone());
    let mut options = MessagesOptions::backward();
    options.limit = UInt::from((PAGE + 1) as u32);
    if let Some(token) = before.as_deref() {
        options = options.from(token);
    }
    let messages = room
        .messages(options)
        .await
        .context("The room history could not be read")?;
    collect_relations(shared, chat, &messages.chunk);
    let mut rows = Vec::new();
    for event in &messages.chunk {
        let Ok(raw) = event.raw().deserialize() else {
            continue;
        };
        let AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(
            SyncRoomMessageEvent::Original(original),
        )) = raw
        else {
            continue;
        };
        if let Some(message) = project_room_message(shared, &room, &original).await {
            rows.push(message);
        }
    }
    let older = rows.len() > PAGE;
    rows.truncate(PAGE);
    rows.reverse();
    shared
        .tokens
        .lock()
        .unwrap()
        .insert(chat.clone(), messages.start);
    shared.sink.send(Event::Messages {
        chat: chat.clone(),
        messages: rows,
        older,
        complete: !older,
    });
    Ok(())
}

fn collect_relations(
    shared: &Shared,
    chat: &ChatId,
    chunk: &[matrix_sdk::deserialized_responses::TimelineEvent],
) {
    for event in chunk {
        let Ok(raw) = event.raw().deserialize() else {
            continue;
        };
        if let AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::Reaction(
            SyncReactionEvent::Original(original),
        )) = raw
        {
            let target = original.content.relates_to.event_id.to_string();
            let emoji = original.content.relates_to.key.clone();
            let sender = original.sender.to_string();
            let mut map = shared.reactions.lock().unwrap();
            let rows = map.entry((chat.clone(), target)).or_default();
            if !rows.iter().any(|(_, existing)| existing == &sender) {
                rows.push((emoji, sender));
            }
        }
    }
}

async fn handle_command(dirs: &AppDirs, shared: &Shared, command: Command) -> Result<()> {
    let account = shared.account;
    match command {
        Command::SendText {
            chat,
            text,
            quoting,
            ..
        } => {
            let Some(room) = chat_room(shared, &chat) else {
                return Ok(());
            };
            let content = RoomMessageEventContent::text_plain(&text);
            match reply_for(quoting.as_deref()) {
                Some(reply) => {
                    let without: RoomMessageEventContentWithoutRelation = content.into();
                    let content = room
                        .make_reply_event(without, reply)
                        .await
                        .context("The reply could not be built")?;
                    room.send(content)
                        .await
                        .context("The message could not be sent")?;
                }
                None => {
                    room.send(content)
                        .await
                        .context("The message could not be sent")?;
                }
            }
        }
        Command::SendFiles {
            chat,
            paths,
            caption,
            quoting,
            ..
        } => {
            let Some(room) = chat_room(shared, &chat) else {
                return Ok(());
            };
            for (index, path) in paths.iter().enumerate() {
                let data = std::fs::read(path)
                    .with_context(|| format!("Could not read {}", path.display()))?;
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("file")
                    .to_owned();
                let mut config = AttachmentConfig::new();
                if index == 0 {
                    if let Some(caption) = &caption {
                        config =
                            config.caption(Some(TextMessageEventContent::plain(caption.clone())));
                    }
                    config = config.reply(reply_for(quoting.as_deref()));
                }
                room.send_attachment(name, &mime_for(path), data, config)
                    .await
                    .context("The file could not be sent")?;
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
            let Some(room) = chat_room(shared, &chat) else {
                return Ok(());
            };
            let image = image::RgbaImage::from_raw(width, height, rgba)
                .context("The image was malformed")?;
            let mut png = Vec::new();
            image::DynamicImage::ImageRgba8(image)
                .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .context("The image could not be encoded")?;
            let mut config = AttachmentConfig::new();
            if let Some(caption) = &caption {
                config = config.caption(Some(TextMessageEventContent::plain(caption.clone())));
            }
            config = config.reply(reply_for(quoting.as_deref()));
            room.send_attachment("image.png", &"image/png".parse().unwrap(), png, config)
                .await
                .context("The image could not be sent")?;
        }
        Command::SendVoice {
            chat,
            samples,
            quoting,
        } => {
            let Some(room) = chat_room(shared, &chat) else {
                return Ok(());
            };
            let ogg = crate::voice::encode(&samples).map_err(|error| anyhow::anyhow!(error))?;
            let config = AttachmentConfig::new().reply(reply_for(quoting.as_deref()));
            room.send_attachment("voice.ogg", &"audio/ogg".parse().unwrap(), ogg, config)
                .await
                .context("The voice note could not be sent")?;
        }
        Command::SendSticker {
            chat,
            path,
            quoting,
        } => {
            let Some(room) = chat_room(shared, &chat) else {
                return Ok(());
            };
            let data = std::fs::read(&path)
                .with_context(|| format!("Could not read {}", path.display()))?;
            let config = AttachmentConfig::new().reply(reply_for(quoting.as_deref()));
            room.send_attachment("sticker.webp", &mime_for(&path), data, config)
                .await
                .context("The sticker could not be sent")?;
        }
        Command::Download { chat, message, .. } => {
            let Some(source) = shared
                .sources
                .lock()
                .unwrap()
                .get(&(chat.clone(), message.clone()))
                .cloned()
            else {
                anyhow::bail!("There is nothing to download for this message");
            };
            let Some(room) = chat_room(shared, &chat) else {
                return Ok(());
            };
            let event_id: OwnedEventId = message.parse().context("The message id was malformed")?;
            let event = room
                .event(&event_id, None)
                .await
                .context("The message could not be read back")?;
            let path = dirs
                .media_cache_dir()
                .join(format!("matrix-{account}-{}", safe_name(&message)));
            let file = shared
                .client
                .media()
                .get_media_file(
                    &MediaRequestParameters {
                        source,
                        format: MediaFormat::File,
                    },
                    None,
                    &"application/octet-stream".parse().unwrap(),
                    false,
                    None,
                )
                .await
                .context("The media could not be fetched")?;
            let failed = match file.persist(&path) {
                Ok(_) => None,
                Err(error) => {
                    log::warn!("Matrix could not store a download: {error}");
                    Some(error.to_string())
                }
            };
            if let Ok(AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(
                SyncRoomMessageEvent::Original(original),
            ))) = event.raw().deserialize()
                && let Some(mut message) = project_room_message(shared, &room, &original).await
            {
                if let Some(media) = message.content.media_mut() {
                    match &failed {
                        None => {
                            media.path = Some(path);
                            media.state = MediaState::Idle;
                        }
                        Some(reason) => media.state = MediaState::Failed(reason.clone()),
                    }
                }
                shared.sink.send(Event::MessageUpdated(Box::new(message)));
            }
        }
        Command::LoadChat { chat, before } => {
            let token = before.map(|(_, token)| token);
            page(shared, &chat, token).await?;
        }
        Command::FetchOlder(chat) => {
            let token = shared.tokens.lock().unwrap().get(&chat).cloned();
            page(shared, &chat, token).await?;
        }
        Command::MarkRead { chat, .. } => {
            let Some(room) = chat_room(shared, &chat) else {
                return Ok(());
            };
            let latest = shared.latest.lock().unwrap().get(&chat).cloned();
            if let Some(latest) = latest {
                room.send_single_receipt(ReceiptType::Read, ReceiptThread::Unthreaded, latest)
                    .await
                    .context("The read receipt could not be sent")?;
            }
        }
        Command::Composing { chat, composing } => {
            if let Some(room) = chat_room(shared, &chat) {
                let _ = room.typing_notice(composing).await;
            }
        }
        Command::EditText { chat, id, text, .. } => {
            let Some(room) = chat_room(shared, &chat) else {
                return Ok(());
            };
            let event_id: OwnedEventId = id.parse().context("The message id was malformed")?;
            let content = room
                .make_edit_event(
                    &event_id,
                    EditedContent::RoomMessage(RoomMessageEventContent::text_plain(&text).into()),
                )
                .await
                .context("The edit could not be built")?;
            room.send(content)
                .await
                .context("The edit could not be sent")?;
        }
        Command::Revoke { chat, id } => {
            let Some(room) = chat_room(shared, &chat) else {
                return Ok(());
            };
            let event_id: OwnedEventId = id.parse().context("The message id was malformed")?;
            room.redact(&event_id, None, None)
                .await
                .context("The message could not be deleted")?;
        }
        Command::React {
            chat,
            message,
            emoji,
        } => {
            let Some(room) = chat_room(shared, &chat) else {
                return Ok(());
            };
            let event_id: OwnedEventId = message.parse().context("The message id was malformed")?;
            room.send(ReactionEventContent::new(Annotation::new(event_id, emoji)))
                .await
                .context("The reaction could not be sent")?;
        }
        _ => {}
    }
    Ok(())
}

/// Accepts a verification request another device started, then runs the SAS
/// flow.
fn accept_verification(
    shared: &Shared,
    sender: matrix_sdk::ruma::OwnedUserId,
    transaction: String,
) -> BoxFuture<'static, ()> {
    let shared = shared.clone();
    Box::pin(async move {
        let Some(request) = shared
            .client
            .encryption()
            .get_verification_request(&sender, transaction)
            .await
        else {
            log::warn!("Matrix could not find a verification request");
            return;
        };
        if let Err(error) = request.accept().await {
            log::warn!("Matrix could not accept the verification request: {error}");
            return;
        }
        run_verification(&shared, request).await;
    })
}

/// Starts a self-verification from this device, then runs the SAS flow.
fn start_verification(shared: &Shared) -> BoxFuture<'static, ()> {
    let shared = shared.clone();
    Box::pin(async move {
        let identity = match shared
            .client
            .encryption()
            .get_user_identity(&shared.me)
            .await
        {
            Ok(Some(identity)) => identity,
            Ok(None) => {
                log::warn!("Matrix has no identity for this account yet");
                return;
            }
            Err(error) => {
                log::warn!("Matrix could not read its identity: {error}");
                return;
            }
        };
        let request = match identity.request_verification().await {
            Ok(request) => request,
            Err(error) => {
                log::warn!("Matrix could not start verification: {error}");
                return;
            }
        };
        run_verification(&shared, request).await;
    })
}

async fn run_verification(shared: &Shared, request: VerificationRequest) {
    let ready = tokio::time::timeout(VERIFY_TIMEOUT, async {
        let mut changes = Box::pin(request.changes());
        while let Some(state) = changes.next().await {
            match state {
                VerificationRequestState::Ready { .. } => return true,
                VerificationRequestState::Done | VerificationRequestState::Cancelled(_) => {
                    return false;
                }
                _ => {}
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    if !ready {
        return;
    }
    let sas = match request.start_sas().await {
        Ok(Some(sas)) => sas,
        Ok(None) => return,
        Err(error) => {
            log::warn!("Matrix could not start the SAS flow: {error}");
            return;
        }
    };
    let sas = match sas.accept().await {
        Ok(()) => sas,
        Err(error) => {
            log::warn!("Matrix could not accept the SAS flow: {error}");
            return;
        }
    };
    let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<VerifyAction>();
    *shared.slot.lock().await = Some(reply_tx);
    let mut finished = false;
    let mut states = Box::pin(sas.changes());
    while !finished {
        tokio::select! {
            state = states.next() => match state {
                Some(SasState::KeysExchanged { .. }) => {
                    shared.sink.send(Event::Verification {
                        account: shared.account,
                        prompt: Some(prompt_of(&sas)),
                    });
                }
                Some(SasState::Done { .. }) => {
                    shared.sink.send(Event::Verification {
                        account: shared.account,
                        prompt: None,
                    });
                    refresh_gated(shared).await;
                    finished = true;
                }
                Some(SasState::Cancelled(_)) | None => {
                    shared.sink.send(Event::Verification {
                        account: shared.account,
                        prompt: None,
                    });
                    finished = true;
                }
                _ => {}
            },
            action = reply_rx.recv() => match action {
                Some(VerifyAction::Confirm) => {
                    let _ = sas.confirm().await;
                }
                Some(VerifyAction::Mismatch) => {
                    let _ = sas.mismatch().await;
                }
                Some(VerifyAction::Cancel) | None => {
                    let _ = sas.cancel().await;
                    finished = true;
                }
                Some(VerifyAction::Start) => {}
            },
        }
    }
    *shared.slot.lock().await = None;
}

fn prompt_of(sas: &SasVerification) -> VerificationPrompt {
    VerificationPrompt {
        emojis: sas
            .emoji()
            .map(|list| {
                list.iter()
                    .map(|emoji| (emoji.symbol.to_owned(), emoji.description.to_owned()))
                    .collect()
            })
            .unwrap_or_default(),
        decimals: sas.decimals(),
    }
}

/// Once the session is verified, the rooms that showed placeholders are
/// paged again so their decrypted content paints.
async fn refresh_gated(shared: &Shared) {
    let chats: Vec<ChatId> = shared.gated.lock().unwrap().drain().collect();
    for chat in chats {
        if let Err(error) = page(shared, &chat, None).await {
            log::warn!("Matrix could not refresh a verified room: {error}");
        }
    }
}
