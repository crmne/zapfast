//! The X account runtime: OAuth 2.0 PKCE, polling, and message projection.
//!
//! X has no streaming API for direct messages, so the runtime polls the
//! newest page of every known conversation. Sign-in opens the user's browser
//! to the authorization page and catches the redirect on a fixed loopback
//! port, which X requires to be registered exactly.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::account::{AccountId, AuthState};
use crate::backend::{Command, Event, LoginStep, Waker};
use crate::model::{ChatId, MediaState};
use crate::paths::AppDirs;

use super::project::{self, Event as XEvent, Includes, XMedia};

/// How many events one page holds.
const PAGE: usize = 50;
/// How often known conversations are checked for new events.
const POLL: Duration = Duration::from_secs(60);
/// The loopback port X requires to be registered as the redirect.
const PORT: u16 = 8787;
const REDIRECT: &str = "http://127.0.0.1:8787/callback";
const API: &str = "https://api.twitter.com/2";
const AUTHORIZE: &str = "https://twitter.com/i/oauth2/authorize";
const TOKEN_ENDPOINT: &str = "https://api.twitter.com/2/oauth2/token";
const SCOPES: &str = "dm.read dm.write tweet.read users.read offline.access media.write";
const FIELDS: &str = "id,text,event_type,dm_conversation_id,created_at,sender_id,attachments";

/// Wakes the window after each event.
#[derive(Clone)]
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

/// The tokens kept in the OS keyring.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Tokens {
    access: String,
    refresh: String,
    expires_at: i64,
}

/// Everything the runtime remembers about the account.
#[derive(Default)]
struct State {
    /// Chat id to conversation id.
    chats: HashMap<ChatId, String>,
    /// Event ids already shown, so polling does not repeat them.
    known: HashSet<(ChatId, String)>,
    /// Sender names looked up once per user id.
    users: HashMap<String, String>,
    /// The event, media, and direct url behind a download.
    media: HashMap<(ChatId, String), (XEvent, XMedia, String)>,
    /// The pagination cursor for older pages.
    older: HashMap<ChatId, Option<String>>,
    /// The newest event of each chat, for the list preview.
    latest: HashMap<ChatId, XEvent>,
}

/// One account's HTTP client, refreshing its token when it goes stale.
struct Api {
    client: reqwest::Client,
    identity: String,
    tokens: std::sync::Mutex<Tokens>,
}

impl Api {
    async fn access(&self) -> Result<String> {
        let (access, expires_at) = {
            let tokens = self.tokens.lock().expect("tokens");
            (tokens.access.clone(), tokens.expires_at)
        };
        let now = jiff::Timestamp::now().as_second();
        if expires_at - 60 > now {
            return Ok(access);
        }
        let client_id = client_id()?;
        let refresh = {
            let tokens = self.tokens.lock().expect("tokens");
            tokens.refresh.clone()
        };
        let response = self
            .client
            .post(TOKEN_ENDPOINT)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh.as_str()),
                ("client_id", client_id.as_str()),
            ])
            .send()
            .await
            .context("The X sign-in could not be refreshed")?;
        let value = parse_response(response, "refreshing the sign-in").await?;
        let tokens = tokens_from_value(&value)?;
        let access = tokens.access.clone();
        crate::secrets::save(&self.identity, &serde_json::to_string(&tokens)?)
            .context("The X sign-in could not be saved")?;
        *self.tokens.lock().expect("tokens") = tokens;
        Ok(access)
    }

    async fn get(&self, url: &str) -> Result<Value> {
        let access = self.access().await?;
        let response = self
            .client
            .get(url)
            .bearer_auth(access)
            .send()
            .await
            .with_context(|| format!("X did not answer at {url}"))?;
        parse_response(response, "reading from X").await
    }

    async fn post(&self, url: &str, body: Value) -> Result<Value> {
        let access = self.access().await?;
        let response = self
            .client
            .post(url)
            .bearer_auth(access)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("X did not answer at {url}"))?;
        parse_response(response, "writing to X").await
    }

    async fn me(&self) -> Result<(String, Option<String>)> {
        let value = self.get(&format!("{API}/users/me")).await?;
        let data = &value["data"];
        let id = data["id"]
            .as_str()
            .context("X did not say which account this is")?
            .to_owned();
        let name = data["name"].as_str().map(str::to_owned);
        Ok((id, name))
    }

    async fn conversations(&self) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        let mut token: Option<String> = None;
        for _ in 0..10 {
            let mut url = format!("{API}/dm_conversations?max_results=100");
            if let Some(token) = &token {
                url.push_str(&format!("&pagination_token={token}"));
            }
            let value = self.get(&url).await?;
            for row in value["data"].as_array().into_iter().flatten() {
                if let Some(id) = row["dm_conversation_id"].as_str() {
                    ids.push(id.to_owned());
                }
            }
            token = value["meta"]["next_token"].as_str().map(str::to_owned);
            if token.is_none() {
                break;
            }
        }
        Ok(ids)
    }

    async fn events(&self, conversation: &str, before: Option<&str>) -> Result<project::Page> {
        let mut url = format!(
            "{API}/dm_conversations/{conversation}/dm_events?max_results={PAGE}&dm_event.fields={FIELDS}&expansions=attachments.media_keys&media.fields=url,preview_image_url,type,width,height&user.fields=name,username"
        );
        if let Some(before) = before {
            url.push_str(&format!("&pagination_token={before}"));
        }
        let value = self.get(&url).await?;
        serde_json::from_value(value).context("X sent a page this client cannot read")
    }

    async fn user_name(&self, user_id: &str) -> Result<Option<String>> {
        let value = self
            .get(&format!("{API}/users/{user_id}?user.fields=name,username"))
            .await?;
        let data = &value["data"];
        Ok(data["name"]
            .as_str()
            .or_else(|| data["username"].as_str())
            .map(str::to_owned))
    }

    async fn send(
        &self,
        conversation: &str,
        text: Option<&str>,
        media_id: Option<&str>,
    ) -> Result<Option<XEvent>> {
        let mut body = json!({});
        if let Some(text) = text {
            body["text"] = json!(text);
        }
        if let Some(media_id) = media_id {
            body["attachments"] = json!([{"media_id": media_id}]);
        }
        let value = self
            .post(
                &format!("{API}/dm_conversations/{conversation}/messages"),
                body,
            )
            .await?;
        let data = value["data"].clone();
        if let Ok(event) = serde_json::from_value::<XEvent>(data.clone()) {
            return Ok(Some(event));
        }
        // The send response shape is not fully documented; when it cannot be
        // read as an event, polling picks the message up shortly after.
        let id = data["dm_event_id"]
            .as_str()
            .or_else(|| data["id"].as_str())
            .map(str::to_owned);
        Ok(id.map(|id| XEvent {
            id,
            event_type: "MessageCreate".to_owned(),
            text: None,
            sender_id: None,
            dm_conversation_id: Some(conversation.to_owned()),
            created_at: None,
            attachments: None,
        }))
    }

    async fn upload_image(&self, bytes: Vec<u8>, filename: &str) -> Result<String> {
        let access = self.access().await?;
        let part = reqwest::multipart::Part::bytes(bytes).file_name(filename.to_owned());
        let form = reqwest::multipart::Form::new()
            .part("media", part)
            .text("media_category", "dm_image");
        let response = self
            .client
            .post(format!("{API}/media/upload"))
            .bearer_auth(access)
            .multipart(form)
            .send()
            .await
            .context("X did not answer the media upload")?;
        let value = parse_response(response, "uploading an image to X").await?;
        value["data"]["id"]
            .as_str()
            .map(str::to_owned)
            .context("X did not return a media id")
    }
}

/// Runs the account until it is shut down.
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
        log::warn!("X stopped: {error}");
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
    let identity = format!("x-{}", account.get());
    let Some(tokens) = connect(account, &identity, sink, &mut commands).await? else {
        return Ok(());
    };

    let api = Api {
        client: reqwest::Client::builder()
            .user_agent("ZapFast")
            .timeout(Duration::from_secs(30))
            .build()
            .context("The HTTP client could not start")?,
        identity: identity.clone(),
        tokens: std::sync::Mutex::new(tokens),
    };

    let (me, name) = api.me().await?;
    sink.send(Event::Me {
        id: me.clone(),
        lid: None,
        name,
        about: None,
    });

    let mut state = State::default();
    load_chats(&api, account, &me, &mut state, sink).await?;

    let mut ticker = tokio::time::interval(POLL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await;
    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else {
                    return Ok(());
                };
                if matches!(command, Command::Shutdown) {
                    return Ok(());
                }
                if let Err(error) = handle_command(dirs, &api, account, &me, &mut state, sink, command).await {
                    log::warn!("X command failed: {error}");
                }
            }
            _ = ticker.tick() => {
                if let Err(error) = poll(&api, account, &me, &mut state, sink).await {
                    log::warn!("X polling failed: {error}");
                }
            }
        }
    }
}

/// Loads the token from the keyring, or walks the browser sign-in once the
/// window asks for it.
async fn connect(
    account: AccountId,
    identity: &str,
    sink: &Sink,
    commands: &mut mpsc::UnboundedReceiver<Command>,
) -> Result<Option<Tokens>> {
    if let Some(json) = crate::secrets::load(identity)?
        && let Ok(tokens) = serde_json::from_str::<Tokens>(&json)
    {
        return Ok(Some(tokens));
    }
    sink.send(Event::Auth {
        account,
        state: AuthState::SignedOut,
    });
    loop {
        let Some(command) = commands.recv().await else {
            return Ok(None);
        };
        match command {
            Command::Shutdown => return Ok(None),
            Command::Login {
                account: target,
                step,
            } if target == account => {
                let LoginStep::XConnect = step else {
                    continue;
                };
                let tokens = authorize(account, sink).await?;
                crate::secrets::save(identity, &serde_json::to_string(&tokens)?)
                    .context("The X sign-in could not be saved in the OS keyring")?;
                return Ok(Some(tokens));
            }
            _ => {}
        }
    }
}

/// The OAuth 2.0 PKCE dance: show the link, catch the loopback redirect, and
/// exchange the code for tokens.
async fn authorize(account: AccountId, sink: &Sink) -> Result<Tokens> {
    let client_id = client_id()?;
    let verifier_bytes: [u8; 32] = rand::random();
    let verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state_bytes: [u8; 16] = rand::random();
    let state: String = state_bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let url = format!(
        "{AUTHORIZE}?response_type=code&client_id={client_id}&redirect_uri={REDIRECT}&scope={}&state={state}&code_challenge={challenge}&code_challenge_method=S256",
        SCOPES.replace(' ', "%20")
    );
    sink.send(Event::Auth {
        account,
        state: AuthState::XAuthorize { url },
    });

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", PORT))
        .await
        .context("The X sign-in needs port 8787, which is busy")?;
    let (mut stream, _) = tokio::time::timeout(Duration::from_secs(300), listener.accept())
        .await
        .context("The X sign-in was not finished in time")?
        .context("The X sign-in could not listen for the redirect")?;
    let mut request = vec![0u8; 4096];
    let read = stream.read(&mut request).await.unwrap_or(0);
    let request = String::from_utf8_lossy(&request[..read]).into_owned();
    let page = "<html><body>ZapFast is signed in. You can close this tab.</body></html>";
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{page}",
        page.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;

    let query = request
        .strip_prefix("GET /callback?")
        .and_then(|rest| rest.split_once(' '))
        .map(|(query, _)| query)
        .context("The X sign-in redirect was not understood")?;
    let mut code = None;
    let mut returned_state = None;
    for pair in query.split('&') {
        match pair.split_once('=') {
            Some(("code", value)) => code = Some(value.to_owned()),
            Some(("state", value)) => returned_state = Some(value.to_owned()),
            _ => {}
        }
    }
    if returned_state.as_deref() != Some(state.as_str()) {
        bail!("The X sign-in answer did not match this window");
    }
    let code = code.context("The X sign-in was refused")?;

    let response = reqwest::Client::new()
        .post(TOKEN_ENDPOINT)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", REDIRECT),
            ("client_id", client_id.as_str()),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await
        .context("The X sign-in could not be exchanged for tokens")?;
    let value = parse_response(response, "finishing the X sign-in").await?;
    tokens_from_value(&value)
}

fn client_id() -> Result<String> {
    std::env::var("ZAPFAST_X_CLIENT_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .context("X needs ZAPFAST_X_CLIENT_ID from an app on developer.x.com")
}

fn tokens_from_value(value: &Value) -> Result<Tokens> {
    let access = value["access_token"]
        .as_str()
        .context("The X sign-in did not return a token")?
        .to_owned();
    let refresh = value["refresh_token"]
        .as_str()
        .context("The X sign-in did not return a refresh token")?
        .to_owned();
    let expires_in = value["expires_in"].as_i64().unwrap_or(7200);
    Ok(Tokens {
        access,
        refresh,
        expires_at: jiff::Timestamp::now().as_second() + expires_in,
    })
}

async fn parse_response(response: reqwest::Response, what: &str) -> Result<Value> {
    let status = response.status();
    let body = response
        .text()
        .await
        .context("X sent an answer that could not be read")?;
    let value: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    if !status.is_success() {
        let detail = value["title"]
            .as_str()
            .or_else(|| value["detail"].as_str())
            .or_else(|| value["errors"][0]["title"].as_str())
            .unwrap_or("no detail");
        bail!("X refused {what} ({status}): {detail}");
    }
    Ok(value)
}

async fn load_chats(
    api: &Api,
    account: AccountId,
    me: &str,
    state: &mut State,
    sink: &Sink,
) -> Result<()> {
    let mut rows = Vec::new();
    for conversation in api.conversations().await? {
        let page = api.events(&conversation, None).await?;
        let chat = project::chat_id(account, &conversation);
        for event in &page.data {
            remember(state, &chat, event, &page.includes);
        }
        let name = other_name(api, me, &page, state).await;
        let latest = page.data.first();
        rows.push(project::chat(account, &conversation, name, latest));
        state.chats.insert(chat.clone(), conversation.clone());
        state.older.insert(chat, page.meta.next_token.clone());
    }
    sink.send(Event::Chats(rows));
    Ok(())
}

/// The first name in the page that is not ours.
async fn other_name(
    api: &Api,
    me: &str,
    page: &project::Page,
    state: &mut State,
) -> Option<String> {
    for event in &page.data {
        let Some(sender) = event.sender_id.as_deref() else {
            continue;
        };
        if sender == me {
            continue;
        }
        if let Some(name) = state.users.get(sender) {
            return Some(name.clone());
        }
        if let Ok(Some(name)) = api.user_name(sender).await {
            state.users.insert(sender.to_owned(), name.clone());
            return Some(name);
        }
    }
    None
}

fn remember(state: &mut State, chat: &ChatId, event: &XEvent, includes: &Includes) {
    state.known.insert((chat.clone(), event.id.clone()));
    if let Some(media) = project::media_of(event, includes)
        && let Some(url) = media
            .url
            .clone()
            .or_else(|| media.preview_image_url.clone())
    {
        state.media.insert(
            (chat.clone(), event.id.clone()),
            (event.clone(), media.clone(), url),
        );
    }
    let newer = state
        .latest
        .get(chat)
        .and_then(project::timestamp_of)
        .zip(project::timestamp_of(event))
        .is_none_or(|(old, new)| new >= old);
    if newer {
        state.latest.insert(chat.clone(), event.clone());
    }
}

async fn poll(
    api: &Api,
    account: AccountId,
    me: &str,
    state: &mut State,
    sink: &Sink,
) -> Result<()> {
    for (chat, conversation) in state.chats.clone() {
        let page = api.events(&conversation, None).await?;
        let mut fresh = Vec::new();
        for event in page.data.iter().rev() {
            if state.known.contains(&(chat.clone(), event.id.clone())) {
                continue;
            }
            if let Some(message) = project::message(
                account,
                &conversation,
                event,
                me,
                event
                    .sender_id
                    .as_deref()
                    .and_then(|sender| state.users.get(sender).cloned()),
                &page.includes,
                None,
            ) {
                remember(state, &chat, event, &page.includes);
                fresh.push(message);
            } else {
                state.known.insert((chat.clone(), event.id.clone()));
            }
        }
        for message in fresh {
            sink.send(Event::Incoming {
                chat: chat.clone(),
                message: Box::new(message),
            });
        }
    }
    Ok(())
}

async fn handle_command(
    dirs: &AppDirs,
    api: &Api,
    account: AccountId,
    me: &str,
    state: &mut State,
    sink: &Sink,
    command: Command,
) -> Result<()> {
    match command {
        Command::SendText { chat, text, .. } => {
            let Some(conversation) = state.chats.get(&chat).cloned() else {
                return Ok(());
            };
            if let Some(event) = api.send(&conversation, Some(&text), None).await? {
                echo(account, me, state, sink, &chat, &conversation, &event);
            }
        }
        Command::SendImage {
            chat,
            width,
            height,
            rgba,
            caption,
            ..
        } => {
            let Some(conversation) = state.chats.get(&chat).cloned() else {
                return Ok(());
            };
            let image = image::RgbaImage::from_raw(width, height, rgba)
                .context("The image to send is not valid")?;
            let mut png = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(image)
                .write_to(&mut png, image::ImageFormat::Png)
                .context("The image to send could not be encoded")?;
            let media_id = api.upload_image(png.into_inner(), "image.png").await?;
            if let Some(event) = api
                .send(&conversation, caption.as_deref(), Some(&media_id))
                .await?
            {
                echo(account, me, state, sink, &chat, &conversation, &event);
            }
        }
        Command::Download { chat, message, .. } => {
            let Some((event, media, url)) =
                state.media.get(&(chat.clone(), message.clone())).cloned()
            else {
                return Ok(());
            };
            let bytes = api
                .client
                .get(&url)
                .bearer_auth(api.access().await?)
                .send()
                .await
                .context("X did not send the image")?
                .bytes()
                .await
                .context("The image from X could not be read")?;
            let path =
                dirs.media_cache_dir()
                    .join(format!("x-{}-{}", account.get(), safe_name(&message)));
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let saved = std::fs::write(&path, &bytes);
            let includes = Includes {
                media: vec![media],
                users: Vec::new(),
            };
            if let Some(mut projected) = project::message(
                account,
                state
                    .chats
                    .get(&chat)
                    .map(String::as_str)
                    .unwrap_or_default(),
                &event,
                me,
                event
                    .sender_id
                    .as_deref()
                    .and_then(|sender| state.users.get(sender).cloned()),
                &includes,
                None,
            ) {
                if let Some(media) = projected.content.media_mut() {
                    match &saved {
                        Ok(()) => {
                            media.path = Some(path);
                            media.state = MediaState::Idle;
                        }
                        Err(error) => {
                            media.state = MediaState::Failed(error.to_string());
                        }
                    }
                }
                sink.send(Event::MessageUpdated(Box::new(projected)));
            }
        }
        Command::LoadChat { chat, before } => {
            let older = before.is_some();
            page(api, account, me, state, sink, &chat, older).await?;
        }
        Command::FetchOlder(chat) => {
            page(api, account, me, state, sink, &chat, true).await?;
        }
        // X has no typing notices, receipts, edits, reactions, stickers,
        // polls, or forwarding; those commands stay no-ops on purpose.
        _ => {}
    }
    Ok(())
}

/// Shows a just-sent message right away. The polling pass skips it later.
fn echo(
    account: AccountId,
    me: &str,
    state: &mut State,
    sink: &Sink,
    chat: &ChatId,
    conversation: &str,
    event: &XEvent,
) {
    let includes = Includes::default();
    if let Some(message) = project::message(account, conversation, event, me, None, &includes, None)
    {
        remember(state, chat, event, &includes);
        sink.send(Event::Incoming {
            chat: chat.clone(),
            message: Box::new(message),
        });
    } else {
        state.known.insert((chat.clone(), event.id.clone()));
    }
}

async fn page(
    api: &Api,
    account: AccountId,
    me: &str,
    state: &mut State,
    sink: &Sink,
    chat: &ChatId,
    older: bool,
) -> Result<()> {
    let Some(conversation) = state.chats.get(chat).cloned() else {
        return Ok(());
    };
    let before = if older {
        state.older.get(chat).cloned().flatten()
    } else {
        None
    };
    let page = api.events(&conversation, before.as_deref()).await?;
    let mut messages = Vec::new();
    for event in page.data.iter().rev() {
        if let Some(message) = project::message(
            account,
            &conversation,
            event,
            me,
            event
                .sender_id
                .as_deref()
                .and_then(|sender| state.users.get(sender).cloned()),
            &page.includes,
            None,
        ) {
            remember(state, chat, event, &page.includes);
            messages.push(message);
        } else {
            state.known.insert((chat.clone(), event.id.clone()));
        }
    }
    state
        .older
        .insert(chat.clone(), page.meta.next_token.clone());
    let complete = page.meta.next_token.is_none();
    sink.send(Event::Messages {
        chat: chat.clone(),
        messages,
        older,
        complete,
    });
    Ok(())
}

fn safe_name(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_files_round_trip() {
        let tokens = Tokens {
            access: "a".to_owned(),
            refresh: "r".to_owned(),
            expires_at: 1,
        };
        let json = serde_json::to_string(&tokens).unwrap();
        let back: Tokens = serde_json::from_str(&json).unwrap();
        assert_eq!(back.access, "a");
        assert_eq!(back.expires_at, 1);
    }

    #[test]
    fn safe_names_stay_filesystem_safe() {
        assert_eq!(safe_name("123/../x"), "123_.._x");
    }
}
