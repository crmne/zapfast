//! Zulip account runtime: one queue, one long poll, and the command loop.
//!
//! The account registers an event queue with the user's server, then long
//! polls it. No inbound port or public URL is involved. Streams, topics, and
//! direct messages are projected through [`super::project`] into the same
//! models every other account speaks.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::account::{AccountId, AuthState};
use crate::backend::{Command, Event, LoginStep, Waker};
use crate::model::{Chat, ChatId, ChatKind, LastMessage, Media, MediaState, Message};
use crate::paths::AppDirs;

use super::project::{self, Target};

/// One page of messages. One extra row is fetched to learn whether any older
/// message exists on the first load.
const PAGE: usize = 50;

/// The long poll waits on the server side; give it room beyond that.
const EVENT_TIMEOUT: Duration = Duration::from_secs(120);

/// Ordinary requests should not hang the account loop.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

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

/// What the user typed to sign in. The key is used once here and lives in the
/// OS keyring afterwards, never in the settings file or a log.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Credentials {
    server: String,
    email: String,
    key: String,
}

/// Chat state the runtime keeps while the account is connected.
#[derive(Default)]
struct State {
    /// Our own user id.
    me: i64,
    /// User id to display name.
    users: HashMap<i64, String>,
    /// Stream id to name.
    streams: HashMap<i64, String>,
    /// Every known chat, keyed by peer.
    chats: HashMap<String, Chat>,
    /// Reaction rows, one per emoji and sender, keyed by chat and message.
    reactions: HashMap<(ChatId, String), Vec<(String, String)>>,
    /// The message object behind a bubble, keyed by chat and message id.
    objects: HashMap<(ChatId, String), Value>,
    /// The attachment object of a message, kept for downloads.
    sources: HashMap<(ChatId, String), Value>,
    /// The oldest message id emitted per chat, for older pages.
    tokens: HashMap<ChatId, i64>,
    /// The newest message id seen per chat, for read markers.
    latest: HashMap<ChatId, i64>,
    /// The chat a message id belongs to.
    message_chats: HashMap<i64, ChatId>,
}

/// What one long poll answered.
enum Poll {
    Events(Vec<Value>),
    Expired,
}

/// The account run loop. Errors end the account with an [`AuthState::Failed`]
/// note instead of taking the whole window down.
pub(crate) async fn run(
    dirs: AppDirs,
    account: AccountId,
    events: std::sync::mpsc::Sender<Event>,
    _commands_tx: mpsc::UnboundedSender<Command>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    waker: Waker,
) {
    let sink = Sink { events, waker };
    if let Err(error) = serve(&dirs, account, &sink, &mut commands).await {
        log::warn!("Zulip stopped: {error}");
        sink.send(Event::Auth {
            account,
            state: AuthState::Failed {
                reason: error.to_string(),
            },
        });
    }
}

/// Signs in (or waits for the card), then runs the queue loop.
async fn serve(
    dirs: &AppDirs,
    account: AccountId,
    sink: &Sink,
    commands: &mut mpsc::UnboundedReceiver<Command>,
) -> Result<()> {
    let Some((api, me, name)) = authenticate_account(account, sink, commands).await? else {
        return Ok(());
    };
    sink.send(Event::Me {
        id: me.to_string(),
        lid: None,
        name: Some(name),
        about: None,
    });

    let mut state = State {
        me,
        ..State::default()
    };
    let mut rows = Vec::new();
    for member in api.users().await? {
        let user_id = member.get("user_id").and_then(Value::as_i64).unwrap_or(0);
        let user_name = member
            .get("full_name")
            .and_then(Value::as_str)
            .or_else(|| member.get("email").and_then(Value::as_str))
            .unwrap_or("")
            .to_owned();
        state.users.insert(user_id, user_name);
    }
    for subscription in api.subscriptions().await? {
        if let Some(stream) = subscription.get("stream_id").and_then(Value::as_i64)
            && let Some(stream_name) = subscription.get("name").and_then(Value::as_str)
        {
            state.streams.insert(stream, stream_name.to_owned());
        }
        if let Some(chat) = project::stream_chat(account, &subscription) {
            state.chats.insert(chat.id.peer().to_owned(), chat.clone());
            rows.push(chat);
        }
    }
    sink.send(Event::Chats(rows));

    let mut queue = api.register().await?;
    loop {
        let queue_id = queue.0.clone();
        let last_event_id = queue.1;
        tokio::select! {
            command = commands.recv() => match command {
                None | Some(Command::Shutdown) => return Ok(()),
                Some(command) => {
                    if let Err(error) =
                        handle_command(&api, &mut state, account, sink, dirs, command).await
                    {
                        log::warn!("Zulip command failed: {error}");
                    }
                }
            },
            result = api.events(&queue_id, last_event_id) => match result {
                Ok(Poll::Events(events)) => {
                    for event in events {
                        if let Some(id) = event.get("id").and_then(Value::as_i64) {
                            queue.1 = queue.1.max(id);
                        }
                        if let Err(error) = dispatch(&api, &mut state, account, sink, &event).await {
                            log::warn!("Zulip event failed: {error}");
                        }
                    }
                }
                Ok(Poll::Expired) => {
                    log::warn!("Zulip event queue expired; registering again");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    queue = api.register().await?;
                }
                Err(error) => {
                    log::warn!("Zulip events stopped: {error}");
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    queue = api.register().await?;
                }
            },
        }
    }
}

/// Loads the stored credentials, or waits for them on the sign-in card. A
/// failed sign-in stays on the card with the server's answer.
async fn authenticate_account(
    account: AccountId,
    sink: &Sink,
    commands: &mut mpsc::UnboundedReceiver<Command>,
) -> Result<Option<(Api, i64, String)>> {
    loop {
        if let Some(credentials) = load_credentials(account)? {
            let api = Api::new(&credentials)?;
            match api.me().await {
                Ok((me, name)) => {
                    sink.send(Event::Auth {
                        account,
                        state: AuthState::Ready,
                    });
                    return Ok(Some((api, me, name)));
                }
                Err(error) => {
                    log::warn!("Zulip credentials were refused: {error}");
                    sink.send(Event::Auth {
                        account,
                        state: AuthState::Failed {
                            reason: error.to_string(),
                        },
                    });
                }
            }
        } else {
            sink.send(Event::Auth {
                account,
                state: AuthState::SignedOut,
            });
        }
        match commands.recv().await {
            None | Some(Command::Shutdown) => return Ok(None),
            Some(Command::Login {
                account: login_account,
                step,
            }) => {
                if login_account != account {
                    continue;
                }
                let LoginStep::ZulipCredentials {
                    server,
                    email,
                    api_key,
                } = step
                else {
                    continue;
                };
                let credentials = Credentials {
                    server: normalize_server(&server),
                    email: email.trim().to_owned(),
                    key: api_key.trim().to_owned(),
                };
                let api = Api::new(&credentials)?;
                match api.me().await {
                    Ok((me, name)) => {
                        save_credentials(account, &credentials)?;
                        sink.send(Event::Auth {
                            account,
                            state: AuthState::Ready,
                        });
                        return Ok(Some((api, me, name)));
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

/// `https://` is assumed when the user leaves the scheme out.
fn normalize_server(server: &str) -> String {
    let server = server.trim().trim_end_matches('/');
    if server.starts_with("http://") || server.starts_with("https://") {
        server.to_owned()
    } else {
        format!("https://{server}")
    }
}

fn credentials_identity(account: AccountId) -> String {
    format!("zulip-{}", account.get())
}

fn load_credentials(account: AccountId) -> Result<Option<Credentials>> {
    let Some(stored) = crate::secrets::load(&credentials_identity(account))? else {
        return Ok(None);
    };
    Ok(serde_json::from_str(&stored).ok())
}

fn save_credentials(account: AccountId, credentials: &Credentials) -> Result<()> {
    let stored = serde_json::to_string(credentials)?;
    crate::secrets::save(&credentials_identity(account), &stored)
}

/// One signed-in server connection.
struct Api {
    server: String,
    email: String,
    key: String,
    client: reqwest::Client,
}

impl Api {
    fn new(credentials: &Credentials) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent("ZapFast")
            .timeout(REQUEST_TIMEOUT)
            .build()
            .context("The Zulip connection could not be set up")?;
        Ok(Self {
            server: credentials.server.clone(),
            email: credentials.email.clone(),
            key: credentials.key.clone(),
            client,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/v1/{}", self.server, path)
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.client
            .get(self.url(path))
            .basic_auth(&self.email, Some(&self.key))
    }

    fn post(&self, path: &str) -> reqwest::RequestBuilder {
        self.client
            .post(self.url(path))
            .basic_auth(&self.email, Some(&self.key))
    }

    /// Reads a response and turns a Zulip error into a message that names the
    /// method and the server's own error code, never message content.
    async fn json(request: reqwest::RequestBuilder) -> Result<Value> {
        let response = request
            .send()
            .await
            .context("The Zulip server could not be reached")?;
        let value: Value = response
            .json()
            .await
            .context("The Zulip server answered with something unexpected")?;
        if value.get("result").and_then(Value::as_str) == Some("success") {
            return Ok(value);
        }
        let code = value
            .get("code")
            .and_then(Value::as_str)
            .or_else(|| value.get("msg").and_then(Value::as_str))
            .unwrap_or("unknown error");
        bail!("The Zulip server refused: {code}")
    }

    async fn me(&self) -> Result<(i64, String)> {
        let value = Self::json(self.get("users/me")).await?;
        let id = value.get("user_id").and_then(Value::as_i64).unwrap_or(0);
        let name = value
            .get("full_name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        Ok((id, name))
    }

    async fn users(&self) -> Result<Vec<Value>> {
        let value = Self::json(self.get("users")).await?;
        Ok(value
            .get("members")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    async fn subscriptions(&self) -> Result<Vec<Value>> {
        let value = Self::json(self.get("users/me/subscriptions")).await?;
        Ok(value
            .get("subscriptions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    /// Opens an event queue and returns its id and the last event in it.
    async fn register(&self) -> Result<(String, i64)> {
        let body = json!({
            "event_types": ["message", "reaction", "update_message", "delete_message", "typing"],
            "apply_markdown": false,
            "client_gravatar": false,
            "allow_empty_topic_name": true,
        });
        let value = Self::json(self.post("register").json(&body)).await?;
        let queue_id = value
            .get("queue_id")
            .and_then(Value::as_str)
            .context("The Zulip server did not open an event queue")?
            .to_owned();
        let last_event_id = value
            .get("last_event_id")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        Ok((queue_id, last_event_id))
    }

    /// One long poll. An expired queue is reported so the caller can register
    /// a new one without treating it as a connection failure.
    async fn events(&self, queue_id: &str, last_event_id: i64) -> Result<Poll> {
        let last = last_event_id.to_string();
        let response = self
            .get("events")
            .query(&[
                ("queue_id", queue_id),
                ("last_event_id", last.as_str()),
                ("dont_block", "false"),
            ])
            .timeout(EVENT_TIMEOUT)
            .send()
            .await
            .context("The Zulip server could not be reached")?;
        let value: Value = response
            .json()
            .await
            .context("The Zulip server answered with something unexpected")?;
        if value.get("result").and_then(Value::as_str) == Some("success") {
            let events = value
                .get("events")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            return Ok(Poll::Events(events));
        }
        if value.get("code").and_then(Value::as_str) == Some("BAD_EVENT_QUEUE_ID") {
            return Ok(Poll::Expired);
        }
        let code = value
            .get("code")
            .and_then(Value::as_str)
            .or_else(|| value.get("msg").and_then(Value::as_str))
            .unwrap_or("unknown error");
        bail!("The Zulip event poll was refused: {code}")
    }

    /// A page of history for one chat. Returns the raw message objects in
    /// ascending order and whether the oldest message was reached.
    async fn history(
        &self,
        target: &Target,
        before: Option<i64>,
        limit: usize,
    ) -> Result<(Vec<Value>, bool)> {
        let anchor = before
            .map(|id| id.to_string())
            .unwrap_or_else(|| "newest".to_owned());
        let limit_text = limit.to_string();
        let narrow = match target {
            Target::Stream { stream } => json!([{ "operator": "stream", "operand": stream }]),
            Target::Topic { stream, topic } => json!([
                { "operator": "stream", "operand": stream },
                { "operator": "topic", "operand": topic },
            ]),
            Target::Dm { users } => json!([{ "operator": "dm", "operand": users }]),
        };
        let value = Self::json(
            self.get("messages")
                .query(&[
                    ("anchor", anchor.as_str()),
                    ("num_before", limit_text.as_str()),
                    ("num_after", "0"),
                    ("apply_markdown", "false"),
                    ("allow_empty_topic_name", "true"),
                ])
                .query(&[("narrow", narrow.to_string())]),
        )
        .await?;
        let messages = value
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let found_oldest = value
            .get("found_oldest")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Ok((messages, found_oldest))
    }

    /// Sends text into a stream topic or a direct conversation.
    async fn send_message(&self, target: &Target, text: &str) -> Result<()> {
        let body = match target {
            Target::Stream { stream } => json!({
                "type": "stream",
                "to": stream.to_string(),
                "topic": project::GENERAL_TOPIC,
                "content": text,
            }),
            Target::Topic { stream, topic } => json!({
                "type": "stream",
                "to": stream.to_string(),
                "topic": topic,
                "content": text,
            }),
            Target::Dm { users } => json!({
                "type": "direct",
                "to": users,
                "content": text,
            }),
        };
        Self::json(self.post("messages").json(&body)).await?;
        Ok(())
    }

    async fn message(&self, message_id: i64) -> Result<Value> {
        let value = Self::json(self.get(&format!("messages/{message_id}"))).await?;
        value
            .get("message")
            .cloned()
            .context("The Zulip server did not return the message")
    }

    async fn edit(&self, message_id: i64, text: &str) -> Result<()> {
        Self::json(
            self.patch(&format!("messages/{message_id}"))
                .json(&json!({ "content": text })),
        )
        .await?;
        Ok(())
    }

    async fn delete(&self, message_id: i64) -> Result<()> {
        Self::json(self.delete_request(&format!("messages/{message_id}"))).await?;
        Ok(())
    }

    fn patch(&self, path: &str) -> reqwest::RequestBuilder {
        self.client
            .patch(self.url(path))
            .basic_auth(&self.email, Some(&self.key))
    }

    fn delete_request(&self, path: &str) -> reqwest::RequestBuilder {
        self.client
            .delete(self.url(path))
            .basic_auth(&self.email, Some(&self.key))
    }

    async fn react(&self, message_id: i64, emoji_name: &str, add: bool) -> Result<()> {
        let path = format!("messages/{message_id}/reactions");
        let body = json!({ "emoji_name": emoji_name });
        if add {
            Self::json(self.post(&path).json(&body)).await?;
        } else {
            Self::json(self.delete_request(&path).json(&body)).await?;
        }
        Ok(())
    }

    /// Sends a typing notice. Direct conversations name the people; stream
    /// typing names the stream.
    async fn typing(&self, target: &Target, composing: bool) -> Result<()> {
        let op = if composing { "start" } else { "stop" };
        let to = match target {
            Target::Stream { stream } | Target::Topic { stream, .. } => json!([stream]),
            Target::Dm { users } => json!(users),
        };
        Self::json(self.post("typing").json(&json!({ "op": op, "to": to }))).await?;
        Ok(())
    }

    async fn mark_topic_as_read(&self, stream: i64, topic: &str) -> Result<()> {
        Self::json(
            self.post("mark_topic_as_read")
                .json(&json!({ "stream_id": stream, "topic_name": topic })),
        )
        .await?;
        Ok(())
    }

    async fn flags(&self, messages: &[i64], flag: &str) -> Result<()> {
        Self::json(
            self.post("messages/flags")
                .json(&json!({ "messages": messages, "flag": flag })),
        )
        .await?;
        Ok(())
    }

    /// Uploads a file and returns the path to link it in a message.
    async fn upload(&self, name: &str, bytes: Vec<u8>, mime: &str) -> Result<String> {
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(name.to_owned())
            .mime_str(mime)
            .context("The Zulip upload could not be prepared")?;
        let form = reqwest::multipart::Form::new().part("file", part);
        let value = Self::json(self.post("user_uploads").multipart(form)).await?;
        value
            .get("uri")
            .and_then(Value::as_str)
            .or_else(|| value.get("url").and_then(Value::as_str))
            .context("The Zulip server did not return the upload path")
            .map(str::to_owned)
    }

    /// Downloads an attachment the signed-in user can see.
    async fn download(&self, url: &str) -> Result<Vec<u8>> {
        let full = if url.starts_with("http://") || url.starts_with("https://") {
            url.to_owned()
        } else {
            format!("{}{}", self.server, url)
        };
        let response = self
            .client
            .get(full)
            .basic_auth(&self.email, Some(&self.key))
            .send()
            .await
            .context("The attachment could not be downloaded")?;
        let bytes = response
            .bytes()
            .await
            .context("The attachment could not be downloaded")?;
        Ok(bytes.to_vec())
    }
}

/// Acts on one queue event.
async fn dispatch(
    api: &Api,
    state: &mut State,
    account: AccountId,
    sink: &Sink,
    event: &Value,
) -> Result<()> {
    match project::parse_event(event) {
        project::StoredEvent::Message => {
            let Some(object) = event.get("message") else {
                return Ok(());
            };
            let Some(chat) = project::message_chat(account, object, state.me) else {
                return Ok(());
            };
            ensure_chat(state, account, &chat, sink);
            if let Some(message) = remember(account, state, object) {
                note_message(state, sink, &chat, &message);
                sink.send(Event::Incoming {
                    chat,
                    message: Box::new(message),
                });
            }
        }
        project::StoredEvent::Reaction {
            op,
            message_id,
            emoji_name,
            user_id,
        } => {
            let Some(chat) = state.message_chats.get(&message_id).cloned() else {
                return Ok(());
            };
            let emoji = project::reaction_name(&emoji_name);
            let rows = state
                .reactions
                .entry((chat.clone(), message_id.to_string()))
                .or_default();
            if op == "add" {
                let row = (emoji, user_id.to_string());
                if !rows.contains(&row) {
                    rows.push(row);
                }
            } else {
                let sender = user_id.to_string();
                rows.retain(|(row_emoji, row_sender)| {
                    !(row_emoji == &emoji && row_sender == &sender)
                });
            }
            refresh_message(api, state, account, sink, message_id).await?;
        }
        project::StoredEvent::Update { message_id } => {
            refresh_message(api, state, account, sink, message_id).await?;
        }
        project::StoredEvent::Delete { message_id } => {
            if let Some(chat) = state.message_chats.remove(&message_id) {
                state
                    .objects
                    .remove(&(chat.clone(), message_id.to_string()));
                state
                    .sources
                    .remove(&(chat.clone(), message_id.to_string()));
                sink.send(Event::MessageDeleted {
                    chat,
                    id: message_id.to_string(),
                });
            }
        }
        project::StoredEvent::Typing {
            op,
            sender_id,
            sender_name,
            to,
        } => {
            if sender_id == state.me {
                return Ok(());
            }
            let chat = typing_chat(account, state, event, &to);
            let Some(chat) = chat else {
                return Ok(());
            };
            ensure_chat(state, account, &chat, sink);
            sink.send(Event::Typing {
                chat,
                sender: sender_name,
                composing: op == "start",
            });
        }
        project::StoredEvent::Other => {}
    }
    Ok(())
}

/// The chat a typing notice belongs to: a stream topic or a direct
/// conversation. Stream typing carries the stream and topic next to the
/// recipients list.
fn typing_chat(account: AccountId, state: &State, event: &Value, to: &[i64]) -> Option<ChatId> {
    if let Some(stream) = event.get("stream_id").and_then(Value::as_i64) {
        let topic = event
            .get("topic")
            .and_then(Value::as_str)
            .unwrap_or(project::GENERAL_TOPIC);
        return Some(if topic.is_empty() || topic == project::GENERAL_TOPIC {
            project::stream_id(account, stream)
        } else {
            project::topic_id(account, stream, topic)
        });
    }
    let users: Vec<i64> = to.iter().copied().filter(|id| *id != state.me).collect();
    Some(project::dm_id(account, &users))
}

/// Creates a chat row if it is not known yet, parents first.
fn ensure_chat(state: &mut State, account: AccountId, chat: &ChatId, sink: &Sink) {
    if state.chats.contains_key(chat.peer()) {
        return;
    }
    let Some(target) = project::parse(chat) else {
        return;
    };
    let row = match target {
        Target::Stream { stream } => stream_row(state, account, stream),
        Target::Topic { stream, topic } => {
            let parent = stream_row(state, account, stream);
            if !state.chats.contains_key(parent.id.peer()) {
                state
                    .chats
                    .insert(parent.id.peer().to_owned(), parent.clone());
                sink.send(Event::ChatUpdated(Box::new(parent)));
            }
            project::topic_chat(account, stream, &topic)
        }
        Target::Dm { users } => {
            let names: Vec<String> = users
                .iter()
                .map(|id| {
                    state
                        .users
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| id.to_string())
                })
                .collect();
            project::dm_chat(account, &users, &names)
        }
    };
    state.chats.insert(row.id.peer().to_owned(), row.clone());
    sink.send(Event::ChatUpdated(Box::new(row)));
}

/// The parent row of a stream, by name when known.
fn stream_row(state: &State, account: AccountId, stream: i64) -> Chat {
    let name = state
        .streams
        .get(&stream)
        .cloned()
        .unwrap_or_else(|| format!("Stream {stream}"));
    let mut row = Chat::new(project::stream_id(account, stream), name);
    row.kind = ChatKind::Group;
    row
}

/// Projects one message object and keeps the state it needs later.
fn remember(account: AccountId, state: &mut State, object: &Value) -> Option<Message> {
    let chat = project::message_chat(account, object, state.me)?;
    let id = object.get("id").and_then(Value::as_i64)?.to_string();
    let sender_id = object.get("sender_id").and_then(Value::as_i64).unwrap_or(0);
    let sender_name = state.users.get(&sender_id).cloned();
    let rows = state
        .reactions
        .entry((chat.clone(), id.clone()))
        .or_insert_with(|| project::reaction_rows(object));
    let reactions = project::reactions(rows, state.me);
    let message = project::message(account, object, state.me, sender_name, reactions)?;
    if let Some(attachment) = project::attachment(object) {
        state
            .sources
            .insert((chat.clone(), id.clone()), attachment.clone());
    }
    state
        .objects
        .insert((chat.clone(), id.clone()), object.clone());
    if let Some(number) = object.get("id").and_then(Value::as_i64) {
        state.message_chats.insert(number, chat.clone());
        let entry = state.latest.entry(chat).or_insert(0);
        *entry = (*entry).max(number);
    }
    Some(message)
}

/// Updates the chat's preview and unread count, then announces the row.
fn note_message(state: &mut State, sink: &Sink, chat: &ChatId, message: &Message) {
    let Some(row) = state.chats.get_mut(chat.peer()) else {
        return;
    };
    row.last_activity = message.timestamp;
    row.last = Some(LastMessage {
        from_me: message.from_me,
        sender: message.sender.clone(),
        sender_name: message.sender_name.clone(),
        summary: message.summary(),
        full: message.summary(),
        status: message.status,
    });
    if !message.from_me {
        row.unread = row.unread.saturating_add(1);
    }
    sink.send(Event::ChatUpdated(Box::new(row.clone())));
}

/// Fetches one message again and announces it, after an edit or a reaction.
async fn refresh_message(
    api: &Api,
    state: &mut State,
    account: AccountId,
    sink: &Sink,
    message_id: i64,
) -> Result<()> {
    let object = api.message(message_id).await?;
    if let Some(message) = remember(account, state, &object) {
        sink.send(Event::MessageUpdated(Box::new(message)));
    }
    Ok(())
}

/// One page of history: the newest messages first, older ones as asked.
async fn page(
    api: &Api,
    state: &mut State,
    account: AccountId,
    sink: &Sink,
    chat: &ChatId,
    before: Option<i64>,
) -> Result<()> {
    let Some(target) = project::parse(chat) else {
        return Ok(());
    };
    let limit = if before.is_some() { PAGE } else { PAGE + 1 };
    let (objects, found_oldest) = api.history(&target, before, limit).await?;
    let skip = if before.is_none() {
        objects.len().saturating_sub(PAGE)
    } else {
        0
    };
    let mut messages = Vec::new();
    for object in objects.iter().skip(skip) {
        ensure_chat(state, account, chat, sink);
        if let Some(message) = remember(account, state, object) {
            messages.push(message);
        }
    }
    if let Some(first) = messages.first()
        && let Ok(id) = first.id.parse::<i64>()
    {
        state.tokens.insert(chat.clone(), id);
    }
    sink.send(Event::Messages {
        chat: chat.clone(),
        messages,
        older: before.is_some(),
        complete: found_oldest,
    });
    Ok(())
}

/// Acts on one command from the window.
async fn handle_command(
    api: &Api,
    state: &mut State,
    account: AccountId,
    sink: &Sink,
    dirs: &AppDirs,
    command: Command,
) -> Result<()> {
    match command {
        Command::SendText { chat, text, .. } => {
            let Some(target) = project::parse(&chat) else {
                return Ok(());
            };
            ensure_chat(state, account, &chat, sink);
            api.send_message(&target, &text).await?;
        }
        Command::SendFiles {
            chat,
            paths,
            caption,
            ..
        } => {
            send_files(api, state, account, sink, &chat, &paths, caption).await?;
        }
        Command::SendImage {
            chat,
            width,
            height,
            rgba,
            caption,
            ..
        } => {
            let image = image::RgbaImage::from_raw(width, height, rgba)
                .context("The pasted image could not be read")?;
            let mut bytes = Vec::new();
            image::DynamicImage::ImageRgba8(image)
                .write_to(
                    &mut std::io::Cursor::new(&mut bytes),
                    image::ImageFormat::Png,
                )
                .context("The pasted image could not be encoded")?;
            let uri = api.upload("image.png", bytes, "image/png").await?;
            send_link(api, state, sink, &chat, caption, "image.png", &uri).await?;
        }
        Command::SendVoice { chat, samples, .. } => {
            let bytes = crate::voice::encode(&samples)
                .map_err(|error| anyhow::anyhow!("The voice note could not be encoded: {error}"))?;
            let uri = api.upload("voice.ogg", bytes, "audio/ogg").await?;
            send_link(api, state, sink, &chat, None, "voice.ogg", &uri).await?;
        }
        Command::LoadChat { chat, before } => {
            page(api, state, account, sink, &chat, before.map(|(id, _)| id)).await?;
        }
        Command::FetchOlder(chat) => {
            let before = state.tokens.get(&chat).copied();
            page(api, state, account, sink, &chat, before).await?;
        }
        Command::Download { chat, message, .. } => {
            download(api, state, account, sink, dirs, &chat, &message).await?;
        }
        Command::EditText { id, text, .. } => {
            if let Ok(message_id) = id.parse::<i64>() {
                api.edit(message_id, &text).await?;
            }
        }
        Command::Revoke { id, .. } => {
            if let Ok(message_id) = id.parse::<i64>() {
                api.delete(message_id).await?;
            }
        }
        Command::DeleteLocal { chat, id } => {
            state.objects.remove(&(chat.clone(), id.clone()));
            state.sources.remove(&(chat.clone(), id.clone()));
            sink.send(Event::MessageDeleted { chat, id });
        }
        Command::React { message, emoji, .. } => {
            if let Ok(message_id) = message.parse::<i64>() {
                api.react(message_id, &project::reaction_key(&emoji), true)
                    .await?;
            }
        }
        Command::Composing { chat, composing } => {
            if let Some(target) = project::parse(&chat) {
                api.typing(&target, composing).await?;
            }
        }
        Command::MarkRead { chat, .. } => {
            mark_read(api, state, sink, &chat).await?;
        }
        Command::MarkUnread(chat) => {
            if let Some(id) = state.latest.get(&chat).copied() {
                api.flags(&[id], "unread").await?;
            }
            if let Some(row) = state.chats.get_mut(chat.peer()) {
                row.marked_unread = true;
                sink.send(Event::ChatUpdated(Box::new(row.clone())));
            }
        }
        Command::EnsureChat { chat, .. } => {
            ensure_chat(state, account, &chat, sink);
        }
        _ => {}
    }
    Ok(())
}

/// Uploads files and sends one message with the caption and links.
async fn send_files(
    api: &Api,
    state: &mut State,
    account: AccountId,
    sink: &Sink,
    chat: &ChatId,
    paths: &[std::path::PathBuf],
    caption: Option<String>,
) -> Result<()> {
    let Some(target) = project::parse(chat) else {
        return Ok(());
    };
    ensure_chat(state, account, chat, sink);
    let mut content = caption.unwrap_or_default();
    for path in paths {
        let bytes = std::fs::read(path)
            .with_context(|| format!("{} could not be read", display_name(path)))?;
        let name = display_name(path);
        let mime = mime_for(path);
        let uri = api.upload(&name, bytes, mime).await?;
        content.push_str(&format!("\n[{name}]({uri})"));
    }
    api.send_message(&target, &content).await?;
    Ok(())
}

/// Sends one uploaded file as a link with its caption.
async fn send_link(
    api: &Api,
    state: &mut State,
    sink: &Sink,
    chat: &ChatId,
    caption: Option<String>,
    name: &str,
    uri: &str,
) -> Result<()> {
    let Some(target) = project::parse(chat) else {
        return Ok(());
    };
    ensure_chat(state, chat.account(), chat, sink);
    let mut content = caption.unwrap_or_default();
    content.push_str(&format!("\n[{name}]({uri})"));
    api.send_message(&target, &content).await?;
    Ok(())
}

/// Marks the open chat read on the server and locally.
async fn mark_read(api: &Api, state: &mut State, sink: &Sink, chat: &ChatId) -> Result<()> {
    if let Some(target) = project::parse(chat) {
        match target {
            Target::Stream { stream } => {
                api.mark_topic_as_read(stream, project::GENERAL_TOPIC)
                    .await?;
            }
            Target::Topic { stream, topic } => {
                api.mark_topic_as_read(stream, &topic).await?;
            }
            Target::Dm { .. } => {
                if let Some(id) = state.latest.get(chat).copied() {
                    api.flags(&[id], "read").await?;
                }
            }
        }
    }
    if let Some(row) = state.chats.get_mut(chat.peer()) {
        row.unread = 0;
        row.marked_unread = false;
        sink.send(Event::ChatUpdated(Box::new(row.clone())));
    }
    Ok(())
}

/// Downloads the first attachment of a message into the media cache.
async fn download(
    api: &Api,
    state: &mut State,
    account: AccountId,
    sink: &Sink,
    dirs: &AppDirs,
    chat: &ChatId,
    id: &str,
) -> Result<()> {
    let Some(source) = state.sources.get(&(chat.clone(), id.to_owned())).cloned() else {
        return Ok(());
    };
    let Some(url) = project::attachment_url(&source) else {
        return Ok(());
    };
    match api.download(&url).await {
        Ok(bytes) => {
            let path =
                dirs.media_cache_dir()
                    .join(format!("zulip-{}-{}", account.get(), safe_name(id)));
            std::fs::write(&path, &bytes)
                .context("The downloaded attachment could not be stored")?;
            patch_media(state, account, sink, chat, id, |media| {
                media.path = Some(path);
                media.state = MediaState::Idle;
            });
        }
        Err(error) => {
            let reason = error.to_string();
            patch_media(state, account, sink, chat, id, |media| {
                media.state = MediaState::Failed(reason);
            });
        }
    }
    Ok(())
}

/// Projects a stored message again with one media change and announces it.
fn patch_media(
    state: &mut State,
    account: AccountId,
    sink: &Sink,
    chat: &ChatId,
    id: &str,
    patch: impl FnOnce(&mut Media),
) {
    let Some(object) = state.objects.get(&(chat.clone(), id.to_owned())).cloned() else {
        return;
    };
    if let Some(mut message) = remember(account, state, &object) {
        if let Some(media) = message.content.media_mut() {
            patch(media);
        }
        sink.send(Event::MessageUpdated(Box::new(message)));
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_owned())
}

fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("mp4") => "video/mp4",
        Some("ogg") => "audio/ogg",
        Some("mp3") => "audio/mpeg",
        Some("pdf") => "application/pdf",
        Some("txt") => "text/plain",
        _ => "application/octet-stream",
    }
}

fn safe_name(id: &str) -> String {
    id.chars()
        .map(|letter| {
            if letter.is_ascii_alphanumeric() || matches!(letter, '-' | '_' | '.') {
                letter
            } else {
                '_'
            }
        })
        .collect()
}
