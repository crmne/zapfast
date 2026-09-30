//! Slack account runtime: Socket Mode, Web API calls, and commands.
//!
//! One account talks to one Slack workspace. Socket Mode means no public
//! request URL: the app token opens a WebSocket, the bot token calls the Web
//! API, and both live in the OS keyring. Messages are projected through
//! [`super::project`] and the runtime speaks the same [`Command`] and
//! [`Event`] types as every other account.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_websockets::{ClientBuilder, Message as WsMessage};

use crate::account::{AccountId, AuthState};
use crate::backend::{Command, Event, LoginStep, Waker};
use crate::model::{Chat, ChatId, ChatKind, Message};
use crate::paths::AppDirs;

use super::project;

/// One page of messages. One extra row is fetched to learn whether older
/// messages exist.
const PAGE: usize = 50;

/// Sends events and asks the window to repaint, like the other accounts.
struct Sink {
    events: std::sync::mpsc::Sender<Event>,
    waker: Waker,
}

impl Sink {
    fn send(&self, event: Event) {
        let _ = self.events.send(event);
        self.waker.wake();
    }
}

/// Chat and message state the runtime keeps while the account is connected.
#[derive(Default)]
struct State {
    /// The bot's own user id.
    me: String,
    /// The workspace's name, as `auth.test` reports it.
    team: String,
    /// The bot's username.
    username: String,
    /// The app token that opens the socket.
    app: String,
    /// User id to display name.
    users: HashMap<String, String>,
    /// Every known chat, keyed by peer.
    channels: HashMap<String, Chat>,
    /// The event a message was projected from, keyed by chat and timestamp.
    events: HashMap<(ChatId, String), Value>,
    /// Reaction rows, one per emoji and sender, keyed by chat and message.
    reactions: HashMap<(ChatId, String), Vec<(String, String)>>,
    /// File JSON for messages with attachments.
    sources: HashMap<(ChatId, String), Value>,
    /// The oldest message timestamp emitted per chat, for older pages.
    tokens: HashMap<ChatId, String>,
    /// The chat a channel timestamp belongs to.
    message_chats: HashMap<(String, String), ChatId>,
}

/// What ends one socket session.
enum End {
    Shutdown,
    Reconnect,
}

/// The account run loop. Errors end the account with an [`AuthState::Failed`]
/// note instead of taking the whole window down.
pub(crate) async fn run(
    dirs: AppDirs,
    account: AccountId,
    events: std::sync::mpsc::Sender<Event>,
    _commands_tx: mpsc::UnboundedSender<Command>,
    commands: mpsc::UnboundedReceiver<Command>,
    waker: Waker,
) {
    let sink = Sink { events, waker };
    if let Err(error) = serve(&dirs, account, &sink, commands).await {
        log::warn!("Slack stopped: {error}");
        sink.send(Event::Auth {
            account,
            state: AuthState::Failed {
                reason: error.to_string(),
            },
        });
    }
}

async fn serve(
    dirs: &AppDirs,
    account: AccountId,
    sink: &Sink,
    mut commands: mpsc::UnboundedReceiver<Command>,
) -> Result<()> {
    let mut state = State::default();
    let mut slack = authenticate_account(account, sink, &mut state, &mut commands).await?;
    let Some(slack_ref) = slack.as_mut() else {
        return Ok(());
    };
    sink.send(Event::Me {
        id: state.me.clone(),
        lid: None,
        name: Some(state.username.clone()),
        about: Some(state.team.clone()),
    });
    let chats = fetch_chats(slack_ref, &mut state, account).await?;
    sink.send(Event::Chats(chats));

    // Socket Mode: read envelopes in their own task, which also acknowledges
    // every envelope right away. The main loop only ever calls the Web API.
    let (envelope_tx, mut envelope_rx) = mpsc::unbounded_channel::<String>();
    loop {
        let url = slack_ref.open_socket().await?;
        let uri = url
            .parse::<http::Uri>()
            .context("Slack sent an invalid socket address")?;
        let reader = tokio::spawn(read_socket(uri, envelope_tx.clone()));
        match session(
            dirs,
            slack_ref,
            &mut state,
            sink,
            account,
            &mut commands,
            &mut envelope_rx,
        )
        .await?
        {
            End::Shutdown => {
                reader.abort();
                return Ok(());
            }
            End::Reconnect => {
                reader.abort();
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }
    }
}

/// Takes stored tokens or waits for a login step until one works.
async fn authenticate_account(
    account: AccountId,
    sink: &Sink,
    state: &mut State,
    commands: &mut mpsc::UnboundedReceiver<Command>,
) -> Result<Option<Slack>> {
    loop {
        match stored_tokens(account) {
            Ok(Some((app, bot))) => match Slack::authenticate(&app, &bot).await {
                Ok(slack) => {
                    state.me = slack.id.clone();
                    state.username = slack.username.clone();
                    state.team = slack.team.clone();
                    state.app = app;
                    sink.send(Event::Auth {
                        account,
                        state: AuthState::Ready,
                    });
                    return Ok(Some(slack));
                }
                Err(error) => {
                    sink.send(Event::Auth {
                        account,
                        state: AuthState::Failed {
                            reason: error.to_string(),
                        },
                    });
                }
            },
            Ok(None) => {
                sink.send(Event::Auth {
                    account,
                    state: AuthState::SignedOut,
                });
            }
            Err(error) => {
                sink.send(Event::Auth {
                    account,
                    state: AuthState::Failed {
                        reason: error.to_string(),
                    },
                });
            }
        }
        match commands.recv().await {
            None | Some(Command::Shutdown) => return Ok(None),
            Some(Command::Login {
                step: LoginStep::SlackTokens { app, bot },
                ..
            }) => {
                let (app, bot) = (app.trim().to_owned(), bot.trim().to_owned());
                match Slack::authenticate(&app, &bot).await {
                    Ok(slack) => {
                        let _ = crate::secrets::save(&app_identity(account), &app);
                        let _ = crate::secrets::save(&bot_identity(account), &bot);
                        state.me = slack.id.clone();
                        state.username = slack.username.clone();
                        state.team = slack.team.clone();
                        state.app = app;
                        sink.send(Event::Auth {
                            account,
                            state: AuthState::Ready,
                        });
                        return Ok(Some(slack));
                    }
                    Err(error) => {
                        sink.send(Event::Auth {
                            account,
                            state: AuthState::Failed {
                                reason: error.to_string(),
                            },
                        });
                    }
                }
            }
            Some(_) => {}
        }
    }
}

fn app_identity(account: AccountId) -> String {
    format!("slack-app-{}", account.get())
}

fn bot_identity(account: AccountId) -> String {
    format!("slack-bot-{}", account.get())
}

fn stored_tokens(account: AccountId) -> Result<Option<(String, String)>> {
    let Some(app) = crate::secrets::load(&app_identity(account))? else {
        return Ok(None);
    };
    let Some(bot) = crate::secrets::load(&bot_identity(account))? else {
        return Ok(None);
    };
    Ok(Some((app, bot)))
}

/// The Web API client for one workspace.
struct Slack {
    client: reqwest::Client,
    app: String,
    bot: String,
    id: String,
    username: String,
    team: String,
}

impl Slack {
    /// Checks that both tokens work and reads who the bot is.
    async fn authenticate(app: &str, bot: &str) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("ZapFast/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("Could not start the Slack HTTP client")?;
        let mut slack = Self {
            client,
            app: app.to_owned(),
            bot: bot.to_owned(),
            id: String::new(),
            username: String::new(),
            team: String::new(),
        };
        let value = slack.call("auth.test", json!({})).await?;
        slack.id = value
            .get("user_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        slack.username = value
            .get("user")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        slack.team = value
            .get("team")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        Ok(slack)
    }

    async fn call(&self, method: &str, body: Value) -> Result<Value> {
        self.call_with(&self.bot, method, body).await
    }

    async fn call_with(&self, token: &str, method: &str, body: Value) -> Result<Value> {
        let response = self
            .client
            .post(format!("https://slack.com/api/{method}"))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("Slack could not be reached for {method}"))?;
        let value: Value = response
            .json()
            .await
            .with_context(|| format!("Slack answered {method} with something other than JSON"))?;
        if value.get("ok").and_then(Value::as_bool) != Some(true) {
            let error = value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown_error");
            bail!("Slack refused {method}: {error}");
        }
        Ok(value)
    }

    /// Opens a Socket Mode connection and returns its address.
    async fn open_socket(&self) -> Result<String> {
        let value = self
            .call_with(&self.app, "apps.connections.open", json!({}))
            .await?;
        value
            .get("url")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .context("Slack did not return a socket address")
    }
}

/// Reads envelopes and acknowledges each one immediately, as Socket Mode
/// requires. The main loop does the slower work.
async fn read_socket(uri: http::Uri, envelopes: mpsc::UnboundedSender<String>) -> Result<()> {
    let (mut socket, _) = ClientBuilder::from_uri(uri)
        .connect()
        .await
        .context("Slack's socket could not be opened")?;
    while let Some(message) = socket.next().await {
        let message = message.context("Slack's socket failed")?;
        if message.is_close() {
            break;
        }
        let Some(text) = message.as_text() else {
            continue;
        };
        let Ok(envelope) = serde_json::from_str::<Value>(text) else {
            continue;
        };
        if let Some(id) = envelope.get("envelope_id").and_then(Value::as_str) {
            let ack = json!({ "envelope_id": id, "type": "ack" }).to_string();
            socket
                .send(WsMessage::text(ack))
                .await
                .context("Slack's socket could not acknowledge an envelope")?;
        }
        if envelopes.send(text.to_owned()).is_err() {
            break;
        }
    }
    Ok(())
}

async fn session(
    dirs: &AppDirs,
    slack: &Slack,
    state: &mut State,
    sink: &Sink,
    account: AccountId,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    envelopes: &mut mpsc::UnboundedReceiver<String>,
) -> Result<End> {
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                None | Some(Command::Shutdown) => return Ok(End::Shutdown),
                Some(command) => {
                    if let Err(error) = handle_command(dirs, slack, state, sink, account, command).await {
                        log::warn!("Slack command failed: {error}");
                    }
                }
            },
            raw = envelopes.recv() => {
                let Some(raw) = raw else {
                    return Ok(End::Reconnect);
                };
                let Ok(envelope) = serde_json::from_str::<Value>(&raw) else {
                    continue;
                };
                match envelope.get("type").and_then(Value::as_str).unwrap_or("") {
                    "disconnect" => return Ok(End::Reconnect),
                    "events_api" => {
                        if let Some(event) = envelope.pointer("/payload/event")
                            && let Err(error) = handle_event(slack, state, sink, account, event).await
                        {
                            log::warn!("Slack event failed: {error}");
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

async fn handle_event(
    slack: &Slack,
    state: &mut State,
    sink: &Sink,
    account: AccountId,
    event: &Value,
) -> Result<()> {
    match event.get("type").and_then(Value::as_str).unwrap_or("") {
        "message" => match event.get("subtype").and_then(Value::as_str) {
            None => receive_message(state, sink, account, event),
            Some("message_changed") => {
                let Some(changed) = event.get("message") else {
                    return Ok(());
                };
                if let Some(message) = project_event(state, account, changed) {
                    sink.send(Event::MessageUpdated(Box::new(message)));
                }
                Ok(())
            }
            Some("message_deleted") => {
                let Some(ts) = event.get("deleted_ts").and_then(Value::as_str) else {
                    return Ok(());
                };
                let Some(channel) = event.get("channel").and_then(Value::as_str) else {
                    return Ok(());
                };
                if let Some(chat) = state
                    .message_chats
                    .get(&(channel.to_owned(), ts.to_owned()))
                    .cloned()
                {
                    state.events.remove(&(chat.clone(), ts.to_owned()));
                    state.reactions.remove(&(chat.clone(), ts.to_owned()));
                    state.sources.remove(&(chat.clone(), ts.to_owned()));
                    sink.send(Event::MessageDeleted {
                        chat,
                        id: ts.to_owned(),
                    });
                }
                Ok(())
            }
            _ => Ok(()),
        },
        "reaction_added" | "reaction_removed" => react(slack, state, sink, account, event).await,
        _ => Ok(()),
    }
}

fn receive_message(
    state: &mut State,
    sink: &Sink,
    account: AccountId,
    event: &Value,
) -> Result<()> {
    let Some(message) = project_event(state, account, event) else {
        return Ok(());
    };
    ensure_thread(state, sink, account, event, &message);
    sink.send(Event::Incoming {
        chat: message.chat.clone(),
        message: Box::new(message),
    });
    Ok(())
}

/// Thread replies become their own rows under the channel the first time they
/// are seen. The row is named after the first reply.
fn ensure_thread(
    state: &mut State,
    sink: &Sink,
    account: AccountId,
    event: &Value,
    message: &Message,
) {
    let Some((channel, Some(thread))) = project::parse_chat(&message.chat) else {
        return;
    };
    if state.channels.contains_key(message.chat.peer()) {
        return;
    }
    if !state.channels.contains_key(channel) {
        let mut parent = Chat::new(project::chat_id(account, channel), channel.to_owned());
        parent.kind = ChatKind::Direct;
        state.channels.insert(parent.id.peer().to_owned(), parent);
    }
    let text = event
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let name = if text.is_empty() {
        "Thread".to_owned()
    } else {
        text.chars().take(48).collect()
    };
    let child = project::thread_chat(account, channel, thread, name);
    state
        .channels
        .insert(child.id.peer().to_owned(), child.clone());
    sink.send(Event::ChatUpdated(Box::new(child)));
}

/// A reaction changed: remember it and re-emit the message it belongs to.
async fn react(
    slack: &Slack,
    state: &mut State,
    sink: &Sink,
    account: AccountId,
    event: &Value,
) -> Result<()> {
    let Some(channel) = event.pointer("/item/channel").and_then(Value::as_str) else {
        return Ok(());
    };
    let Some(ts) = event.pointer("/item/ts").and_then(Value::as_str) else {
        return Ok(());
    };
    let Some(chat) = state
        .message_chats
        .get(&(channel.to_owned(), ts.to_owned()))
        .cloned()
    else {
        return Ok(());
    };
    let name = event
        .get("reaction")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let emoji = project::reaction_name(&name);
    let sender = event
        .get("user")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let rows = state
        .reactions
        .entry((chat.clone(), ts.to_owned()))
        .or_default();
    let added = event.get("type").and_then(Value::as_str) == Some("reaction_added");
    if added {
        if !rows
            .iter()
            .any(|(known, who)| known == &emoji && who == &sender)
        {
            rows.push((emoji, sender));
        }
    } else {
        rows.retain(|(known, who)| !(known == &emoji && who == &sender));
    }
    refresh_message(slack, state, sink, account, channel, ts, &chat).await
}

/// Re-reads one message and emits it again with its current reactions.
async fn refresh_message(
    slack: &Slack,
    state: &mut State,
    sink: &Sink,
    account: AccountId,
    channel: &str,
    ts: &str,
    chat: &ChatId,
) -> Result<()> {
    let event = match project::parse_chat(chat) {
        Some((_, Some(thread))) => {
            let value = slack
                .call(
                    "conversations.replies",
                    json!({ "channel": channel, "ts": thread }),
                )
                .await?;
            find_message(&value, ts)
        }
        _ => {
            let value = slack
                .call(
                    "conversations.history",
                    json!({ "channel": channel, "latest": ts, "inclusive": true, "limit": 1 }),
                )
                .await?;
            find_message(&value, ts)
        }
    };
    if let Some(event) = event
        && let Some(message) = project_event(state, account, &event)
    {
        sink.send(Event::MessageUpdated(Box::new(message)));
    }
    Ok(())
}

fn find_message(value: &Value, ts: &str) -> Option<Value> {
    value
        .get("messages")
        .and_then(Value::as_array)?
        .iter()
        .find(|row| row.get("ts").and_then(Value::as_str) == Some(ts))
        .cloned()
}

/// Projects a message event and records everything the runtime needs to
/// re-project or download it later.
fn project_event(state: &mut State, account: AccountId, event: &Value) -> Option<Message> {
    let ts = event.get("ts").and_then(Value::as_str)?;
    let channel = event.get("channel").and_then(Value::as_str)?;
    let sender = event
        .get("user")
        .and_then(Value::as_str)
        .or_else(|| event.get("bot_id").and_then(Value::as_str))
        .unwrap_or("slack");
    let sender_name = state.users.get(sender).cloned();
    let mut message = project::message(account, event, &state.me, sender_name, None, Vec::new())?;
    let key = (message.chat.clone(), ts.to_owned());
    if !state.reactions.contains_key(&key)
        && let Some(rows) = event_rows(event)
    {
        state.reactions.insert(key.clone(), rows);
    }
    let rows = state.reactions.get(&key).cloned().unwrap_or_default();
    message.reactions = project::reactions(&rows, &state.me);
    if let Some(file) = event
        .get("files")
        .and_then(Value::as_array)
        .and_then(|files| files.first())
    {
        state.sources.insert(key.clone(), file.clone());
    }
    state.events.insert(key, event.clone());
    state
        .message_chats
        .insert((channel.to_owned(), ts.to_owned()), message.chat.clone());
    Some(message)
}

/// The reaction summary a message event carries, as rows.
fn event_rows(event: &Value) -> Option<Vec<(String, String)>> {
    let reactions = event.get("reactions").and_then(Value::as_array)?;
    let mut rows = Vec::new();
    for reaction in reactions {
        let Some(name) = reaction.get("name").and_then(Value::as_str) else {
            continue;
        };
        let emoji = project::reaction_name(name);
        for user in reaction
            .get("users")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(user) = user.as_str() {
                rows.push((emoji.clone(), user.to_owned()));
            }
        }
    }
    Some(rows)
}

/// Loads users and conversations, and reports the chats they name.
async fn fetch_chats(slack: &Slack, state: &mut State, account: AccountId) -> Result<Vec<Chat>> {
    let mut cursor = String::new();
    loop {
        let mut body = json!({ "limit": 200 });
        if !cursor.is_empty() {
            body["cursor"] = json!(cursor);
        }
        let value = slack.call("users.list", body).await?;
        for member in value
            .get("members")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let (Some(id), Some(name)) =
                (member.get("id").and_then(Value::as_str), user_name(member))
            else {
                continue;
            };
            state.users.insert(id.to_owned(), name);
        }
        cursor = next_cursor(&value);
        if cursor.is_empty() {
            break;
        }
    }
    let mut chats = Vec::new();
    let mut cursor = String::new();
    loop {
        let mut body = json!({
            "types": "public_channel,private_channel,mpim,im",
            "limit": 200,
        });
        if !cursor.is_empty() {
            body["cursor"] = json!(cursor);
        }
        let value = slack.call("conversations.list", body).await?;
        for raw in value
            .get("channels")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(chat) = project::channel(account, raw, &state.users) {
                state
                    .channels
                    .insert(chat.id.peer().to_owned(), chat.clone());
                chats.push(chat);
            }
        }
        cursor = next_cursor(&value);
        if cursor.is_empty() {
            break;
        }
    }
    Ok(chats)
}

fn next_cursor(value: &Value) -> String {
    value
        .pointer("/response_metadata/next_cursor")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn user_name(member: &Value) -> Option<String> {
    let profile = member.get("profile").unwrap_or(&Value::Null);
    for key in ["display_name", "real_name"] {
        if let Some(name) = profile.get(key).and_then(Value::as_str)
            && !name.is_empty()
        {
            return Some(name.to_owned());
        }
    }
    for key in ["real_name", "name"] {
        if let Some(name) = member.get(key).and_then(Value::as_str)
            && !name.is_empty()
        {
            return Some(name.to_owned());
        }
    }
    None
}

async fn handle_command(
    dirs: &AppDirs,
    slack: &Slack,
    state: &mut State,
    sink: &Sink,
    account: AccountId,
    command: Command,
) -> Result<()> {
    match command {
        Command::SendText { chat, text, .. } => {
            let Some((channel, thread)) = project::parse_chat(&chat) else {
                return Ok(());
            };
            let mut body = json!({ "channel": channel, "text": text });
            if let Some(thread) = thread {
                body["thread_ts"] = json!(thread);
            }
            slack.call("chat.postMessage", body).await?;
        }
        Command::SendFiles {
            chat,
            paths,
            caption,
            ..
        } => {
            let Some((channel, thread)) = project::parse_chat(&chat) else {
                return Ok(());
            };
            for (index, path) in paths.iter().enumerate() {
                let bytes = std::fs::read(path)
                    .with_context(|| format!("Could not read {}", path.display()))?;
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("file");
                let caption = if index == 0 { caption.as_deref() } else { None };
                upload(slack, channel, thread, name, bytes, caption).await?;
            }
        }
        Command::SendImage {
            chat,
            rgba,
            width,
            height,
            ..
        } => {
            let Some((channel, thread)) = project::parse_chat(&chat) else {
                return Ok(());
            };
            let Some(image) = image::RgbaImage::from_raw(width, height, rgba) else {
                return Ok(());
            };
            let mut bytes = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(image)
                .write_to(&mut bytes, image::ImageFormat::Png)
                .context("Could not encode the image for Slack")?;
            upload(
                slack,
                channel,
                thread,
                "image.png",
                bytes.into_inner(),
                None,
            )
            .await?;
        }
        Command::SendVoice { chat, samples, .. } => {
            let Some((channel, thread)) = project::parse_chat(&chat) else {
                return Ok(());
            };
            let bytes = crate::voice::encode(&samples).map_err(|error| anyhow::anyhow!(error))?;
            upload(slack, channel, thread, "voice.ogg", bytes, None).await?;
        }
        Command::LoadChat { chat, before } => {
            page(slack, state, sink, account, &chat, before.map(|key| key.0)).await?;
        }
        Command::FetchOlder(chat) => {
            let before = state
                .tokens
                .get(&chat)
                .and_then(|token| token.parse::<f64>().ok())
                .map(|seconds| seconds as i64);
            page(slack, state, sink, account, &chat, before).await?;
        }
        Command::Download { chat, message, .. } => {
            download(dirs, slack, state, sink, account, &chat, &message).await?;
        }
        Command::EditText { chat, id, text, .. } => {
            let Some((channel, _)) = project::parse_chat(&chat) else {
                return Ok(());
            };
            slack
                .call(
                    "chat.update",
                    json!({ "channel": channel, "ts": id, "text": text }),
                )
                .await?;
        }
        Command::Revoke { chat, id } => {
            let Some((channel, _)) = project::parse_chat(&chat) else {
                return Ok(());
            };
            slack
                .call("chat.delete", json!({ "channel": channel, "ts": id }))
                .await?;
        }
        Command::React {
            chat,
            message,
            emoji,
        } => {
            let Some((channel, _)) = project::parse_chat(&chat) else {
                return Ok(());
            };
            slack
                .call(
                    "reactions.add",
                    json!({
                        "channel": channel,
                        "timestamp": message,
                        "name": reaction_key(&emoji),
                    }),
                )
                .await?;
        }
        // Slack has no typing indicator, read receipts, drafts, muted state,
        // stickers, or polls through this API, and search, forwarding, and
        // voice notes wait for their own work.
        _ => {}
    }
    Ok(())
}

/// One page of history, newest first from Slack, oldest first to the window.
async fn page(
    slack: &Slack,
    state: &mut State,
    sink: &Sink,
    account: AccountId,
    chat: &ChatId,
    before: Option<i64>,
) -> Result<()> {
    let Some((channel, thread)) = project::parse_chat(chat) else {
        return Ok(());
    };
    let limit = PAGE + 1;
    let value = match thread {
        Some(thread) => {
            slack
                .call(
                    "conversations.replies",
                    json!({ "channel": channel, "ts": thread, "limit": limit }),
                )
                .await?
        }
        None => {
            let mut body = json!({ "channel": channel, "limit": limit });
            if let Some(before) = before {
                body["latest"] = json!(before.to_string());
            }
            slack.call("conversations.history", body).await?
        }
    };
    let rows = value
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let has_more = value
        .get("has_more")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut messages: Vec<Message> = rows
        .iter()
        .take(PAGE)
        .filter_map(|row| project_event(state, account, row))
        .collect();
    messages.reverse();
    if let Some(oldest) = messages.first() {
        state
            .tokens
            .insert(chat.clone(), oldest.timestamp.to_string());
    }
    sink.send(Event::Messages {
        chat: chat.clone(),
        messages,
        older: rows.len() > PAGE,
        complete: !has_more,
    });
    Ok(())
}

/// Downloads one file's bytes with the bot token and points the message at it.
async fn download(
    dirs: &AppDirs,
    slack: &Slack,
    state: &mut State,
    sink: &Sink,
    account: AccountId,
    chat: &ChatId,
    message: &str,
) -> Result<()> {
    let key = (chat.clone(), message.to_owned());
    let Some(file) = state.sources.get(&key).cloned() else {
        return Ok(());
    };
    let Some(url) = file
        .get("url_private_download")
        .and_then(Value::as_str)
        .or_else(|| file.get("url_private").and_then(Value::as_str))
    else {
        return Ok(());
    };
    let path =
        dirs.media_cache_dir()
            .join(format!("slack-{}-{}", account.get(), safe_name(message)));
    let result = fetch_file(slack, url, &path).await;
    if let Some(event) = state.events.get(&key).cloned()
        && let Some(mut projected) = project_event(state, account, &event)
    {
        if let Some(media) = projected.content.media_mut() {
            match &result {
                Ok(()) => {
                    media.path = Some(path);
                    media.state = crate::model::MediaState::Idle;
                }
                Err(error) => {
                    media.state = crate::model::MediaState::Failed(error.to_string());
                }
            }
        }
        sink.send(Event::MessageUpdated(Box::new(projected)));
    }
    result
}

async fn fetch_file(slack: &Slack, url: &str, path: &Path) -> Result<()> {
    let response = slack
        .client
        .get(url)
        .bearer_auth(&slack.bot)
        .send()
        .await
        .context("Slack could not be reached for the file")?;
    let bytes = response
        .bytes()
        .await
        .context("Slack's file did not download")?;
    std::fs::write(path, bytes).with_context(|| format!("Could not write {}", path.display()))
}

/// Uploads one file with Slack's two-step file API and shares it in the chat.
async fn upload(
    slack: &Slack,
    channel: &str,
    thread: Option<&str>,
    name: &str,
    bytes: Vec<u8>,
    caption: Option<&str>,
) -> Result<()> {
    let value = slack
        .call(
            "files.getUploadURLExternal",
            json!({ "filename": name, "length": bytes.len() }),
        )
        .await?;
    let url = value
        .get("upload_url")
        .and_then(Value::as_str)
        .context("Slack did not return an upload address")?;
    let id = value
        .get("file_id")
        .and_then(Value::as_str)
        .context("Slack did not return a file id")?;
    let response = slack
        .client
        .post(url)
        .body(bytes)
        .send()
        .await
        .context("Slack's upload address could not be reached")?;
    if !response.status().is_success() {
        bail!("Slack refused the file upload");
    }
    let mut body = json!({
        "files": [{ "id": id, "title": name }],
        "channel_id": channel,
    });
    if let Some(thread) = thread {
        body["thread_ts"] = json!(thread);
    }
    if let Some(caption) = caption {
        body["initial_comment"] = json!(caption);
    }
    slack.call("files.completeUploadExternal", body).await?;
    Ok(())
}

/// The reaction name Slack knows for a shown emoji.
fn reaction_key(emoji: &str) -> String {
    for (name, shown) in [
        ("thumbsup", "\u{1f44d}"),
        ("thumbsdown", "\u{1f44e}"),
        ("heart", "\u{2764}\u{fe0f}"),
        ("joy", "\u{1f602}"),
        ("tada", "\u{1f389}"),
        ("eyes", "\u{1f440}"),
        ("fire", "\u{1f525}"),
        ("pray", "\u{1f64f}"),
        ("clap", "\u{1f44f}"),
        ("100", "\u{1f4af}"),
        ("ok_hand", "\u{1f44c}"),
        ("raised_hands", "\u{1f64c}"),
        ("muscle", "\u{1f4aa}"),
        ("wink", "\u{1f609}"),
        ("sunglasses", "\u{1f60e}"),
        ("heart_eyes", "\u{1f60d}"),
        ("scream", "\u{1f631}"),
    ] {
        if emoji == shown {
            return name.to_owned();
        }
    }
    emoji.to_owned()
}

/// Attachment file names, with anything that is not clearly filename-safe
/// replaced, so a timestamp can be one.
fn safe_name(id: &str) -> String {
    id.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect()
}
