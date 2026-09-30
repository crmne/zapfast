//! Runtime for one Delta Chat account: a `deltachat-rpc-server` sidecar and
//! the long-poll loop over its events.
//!
//! The window never links the mail stack; the sidecar keeps the account
//! database under the account directory and speaks JSON-RPC over stdio. Only
//! one request may long-poll for events at a time, so a single task owns that
//! call and forwards events to the command loop through a channel.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdout, Command as ProcessCommand};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::account::{AccountId, AuthState};
use crate::backend::{Command, Event, LoginStep, Waker};
use crate::model::{ChatId, MediaState};
use crate::paths::AppDirs;

use super::project::{self, SELF_CONTACT, StoredEvent};

const PAGE: usize = 50;
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
const CONFIGURE_TIMEOUT: Duration = Duration::from_secs(180);
const POLL_TIMEOUT: Duration = Duration::from_secs(120);
const RETRY: Duration = Duration::from_secs(1);

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

/// The JSON-RPC client over the sidecar's stdio.
struct Rpc {
    stdin: Mutex<tokio::process::ChildStdin>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>>,
    next: AtomicU64,
}

impl Rpc {
    async fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        tokio::time::timeout(timeout, self.call(method, params))
            .await
            .with_context(|| format!("The Delta Chat server did not answer {method}"))?
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let request = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": id,
        });
        {
            let mut stdin = self.stdin.lock().await;
            stdin
                .write_all(request.to_string().as_bytes())
                .await
                .context("The Delta Chat server closed its input")?;
            stdin
                .write_all(b"\n")
                .await
                .context("The Delta Chat server closed its input")?;
            stdin
                .flush()
                .await
                .context("The Delta Chat server closed its input")?;
        }
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id, sender);
        receiver.await.context("The Delta Chat server stopped")?
    }
}

/// Reads responses and hands them to whoever is waiting for that id.
async fn read_loop(
    stdout: ChildStdout,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>>,
) {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(id) = value.get("id").and_then(Value::as_u64) else {
            continue;
        };
        let Some(sender) = pending.lock().await.remove(&id) else {
            continue;
        };
        let _ = sender.send(result_of(value));
    }
    let mut pending = pending.lock().await;
    for (_, sender) in pending.drain() {
        let _ = sender.send(Err(anyhow::anyhow!("The Delta Chat server stopped")));
    }
}

fn result_of(value: Value) -> Result<Value> {
    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("The Delta Chat server refused the call");
        bail!("{message}");
    }
    Ok(value.get("result").cloned().unwrap_or(Value::Null))
}

#[derive(Default)]
struct State {
    rpc_account: u32,
    chats: HashMap<ChatId, u64>,
    known: HashSet<(ChatId, String)>,
    older: HashMap<ChatId, u64>,
}

/// Runs the account until the window shuts it down or the server stops.
pub(crate) async fn run(
    dirs: AppDirs,
    account: AccountId,
    events: std::sync::mpsc::Sender<Event>,
    _commands_tx: tokio::sync::mpsc::UnboundedSender<Command>,
    mut commands: tokio::sync::mpsc::UnboundedReceiver<Command>,
    waker: Waker,
) {
    let sink = Sink { events, waker };
    if let Err(error) = serve(&dirs, account, &sink, &mut commands).await {
        log::warn!("Delta Chat stopped: {error:#}");
        sink.send(Event::Auth {
            account,
            state: AuthState::Failed {
                reason: format!("{error:#}"),
            },
        });
    }
}

async fn serve(
    dirs: &AppDirs,
    account: AccountId,
    sink: &Sink,
    commands: &mut mpsc::UnboundedReceiver<Command>,
) -> Result<()> {
    let mut child = spawn_sidecar(dirs, account)?;
    let stdout = child
        .stdout
        .take()
        .context("The Delta Chat server has no output")?;
    let stdin = child
        .stdin
        .take()
        .context("The Delta Chat server has no input")?;
    let rpc = Arc::new(Rpc {
        stdin: Mutex::new(stdin),
        pending: Arc::new(Mutex::new(HashMap::new())),
        next: AtomicU64::new(1),
    });
    tokio::spawn(read_loop(stdout, rpc.pending.clone()));

    let mut state = State {
        rpc_account: rpc_account(&rpc, dirs, account).await?,
        ..State::default()
    };

    let configured = rpc
        .request("is_configured", json!([state.rpc_account]), CALL_TIMEOUT)
        .await?
        .as_bool()
        .unwrap_or(false);
    let mut address = if configured {
        transports(&rpc, state.rpc_account)
            .await
            .ok()
            .and_then(|list| list.into_iter().next())
    } else {
        None
    };
    if configured {
        sink.send(Event::Auth {
            account,
            state: AuthState::Ready,
        });
    } else {
        sink.send(Event::Auth {
            account,
            state: AuthState::SignedOut,
        });
        loop {
            match commands.recv().await {
                None | Some(Command::Shutdown) => {
                    let _ = child.kill().await;
                    return Ok(());
                }
                Some(Command::Login {
                    account: target,
                    step,
                }) if target == account => {
                    let LoginStep::DeltaCredentials { addr, password } = step else {
                        continue;
                    };
                    match configure(&rpc, state.rpc_account, &addr, &password).await {
                        Ok(()) => {
                            address = Some(addr);
                            sink.send(Event::Auth {
                                account,
                                state: AuthState::Ready,
                            });
                            break;
                        }
                        Err(error) => {
                            sink.send(Event::Auth {
                                account,
                                state: AuthState::Failed {
                                    reason: format!("{error:#}"),
                                },
                            });
                        }
                    }
                }
                Some(_) => {}
            }
        }
    }

    rpc.request("start_io", json!([state.rpc_account]), CALL_TIMEOUT)
        .await
        .context("Delta Chat could not start its network")?;
    announce_me(&rpc, sink, account, &state, address).await;
    refresh_chats(&rpc, &mut state, sink, account).await?;

    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let poller = tokio::spawn(event_loop(rpc.clone(), event_tx));
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                None | Some(Command::Shutdown) => break,
                Some(command) => {
                    if let Err(error) = handle_command(dirs, &rpc, &mut state, sink, account, command).await {
                        log::warn!("A Delta Chat command failed: {error:#}");
                    }
                }
            },
            event = event_rx.recv() => {
                if let Some(event) = event
                    && let Err(error) = dispatch(&rpc, &mut state, sink, account, &event).await
                {
                    log::warn!("A Delta Chat event failed: {error:#}");
                }
            }
        }
    }
    poller.abort();
    let _ = child.kill().await;
    Ok(())
}

fn spawn_sidecar(dirs: &AppDirs, account: AccountId) -> Result<Child> {
    let binary =
        std::env::var("ZAPFAST_DELTA_RPC").unwrap_or_else(|_| "deltachat-rpc-server".to_owned());
    let accounts = dirs.account_dir(account).join("delta/accounts");
    crate::paths::create_private_dir(&accounts)?;
    ProcessCommand::new(&binary)
        .env("DC_ACCOUNTS_PATH", &accounts)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| {
            format!(
                "Delta Chat needs deltachat-rpc-server (looked for {binary}); install it with \
                 cargo install --locked --git https://github.com/chatmail/core \
                 --rev c41cac76d284f44094341c5d9f4eb3a847e2c34b deltachat-rpc-server"
            )
        })
}

async fn rpc_account(rpc: &Rpc, dirs: &AppDirs, account: AccountId) -> Result<u32> {
    let path = dirs.account_dir(account).join("delta/rpc-account");
    if let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(id) = text.trim().parse::<u32>()
    {
        return Ok(id);
    }
    let id = rpc
        .request("add_account", json!([]), CALL_TIMEOUT)
        .await?
        .as_u64()
        .context("Delta Chat did not return an account number")?
        .min(u64::from(u32::MAX)) as u32;
    if let Some(parent) = path.parent() {
        crate::paths::create_private_dir(parent)?;
    }
    std::fs::write(&path, id.to_string()).context("Could not remember the Delta Chat account")?;
    Ok(id)
}

async fn transports(rpc: &Rpc, account: u32) -> Result<Vec<String>> {
    let list = rpc
        .request("list_transports", json!([account]), CALL_TIMEOUT)
        .await?;
    Ok(list
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("addr").and_then(Value::as_str).map(str::to_owned))
        .collect())
}

async fn configure(rpc: &Rpc, account: u32, addr: &str, password: &str) -> Result<()> {
    rpc.request(
        "add_or_update_transport",
        json!([account, {"addr": addr, "password": password}]),
        CONFIGURE_TIMEOUT,
    )
    .await
    .context("The Delta Chat server could not use that address and password")?;
    let configured = rpc
        .request("is_configured", json!([account]), CALL_TIMEOUT)
        .await?
        .as_bool()
        .unwrap_or(false);
    if !configured {
        bail!("Delta Chat did not accept that address and password");
    }
    Ok(())
}

async fn announce_me(
    rpc: &Rpc,
    sink: &Sink,
    _account: AccountId,
    state: &State,
    address: Option<String>,
) {
    let display = rpc
        .request("get_account_info", json!([state.rpc_account]), CALL_TIMEOUT)
        .await
        .ok()
        .and_then(|info| {
            info.get("displayName")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    let name = display.or_else(|| address.clone());
    sink.send(Event::Me {
        id: address.unwrap_or_default(),
        lid: None,
        name,
        about: None,
    });
}

async fn event_loop(rpc: Arc<Rpc>, tx: mpsc::UnboundedSender<Value>) {
    loop {
        match rpc.request("get_next_event", json!([]), POLL_TIMEOUT).await {
            Ok(value) => {
                if tx.send(value).is_err() {
                    return;
                }
            }
            Err(_) => tokio::time::sleep(RETRY).await,
        }
    }
}

async fn refresh_chats(
    rpc: &Rpc,
    state: &mut State,
    sink: &Sink,
    account: AccountId,
) -> Result<()> {
    let entries = rpc
        .request(
            "get_chatlist_entries",
            json!([state.rpc_account, null, null, null]),
            CALL_TIMEOUT,
        )
        .await?;
    let ids: Vec<u64> = entries
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .collect();
    if ids.is_empty() {
        sink.send(Event::Chats(Vec::new()));
        return Ok(());
    }
    let items = rpc
        .request(
            "get_chatlist_items_by_entries",
            json!([state.rpc_account, ids]),
            CALL_TIMEOUT,
        )
        .await?;
    let mut rows = Vec::new();
    if let Some(map) = items.as_object() {
        for item in map.values() {
            if let Some(chat) = project::chat(account, item) {
                if let Some(rpc_id) = item.get("id").and_then(Value::as_u64) {
                    state.chats.insert(chat.id.clone(), rpc_id);
                }
                rows.push(chat);
            }
        }
    }
    sink.send(Event::Chats(rows));
    Ok(())
}

async fn dispatch(
    rpc: &Rpc,
    state: &mut State,
    sink: &Sink,
    account: AccountId,
    event: &Value,
) -> Result<()> {
    match project::stored_event(event) {
        StoredEvent::Incoming { chat, message }
        | StoredEvent::Changed { chat, message }
        | StoredEvent::Delivered { chat, message }
        | StoredEvent::Read { chat, message } => {
            load_message(rpc, state, sink, account, chat, message).await
        }
        StoredEvent::Deleted { chat, message } => {
            sink.send(Event::MessageDeleted {
                chat: project::chat_id(account, chat),
                id: message.to_string(),
            });
            Ok(())
        }
        StoredEvent::ChatModified(_) | StoredEvent::Chatlist | StoredEvent::Overflow => {
            refresh_chats(rpc, state, sink, account).await
        }
        StoredEvent::Other(_) => Ok(()),
    }
}

async fn load_message(
    rpc: &Rpc,
    state: &mut State,
    sink: &Sink,
    account: AccountId,
    chat: u64,
    message: u64,
) -> Result<()> {
    if !state.chats.values().any(|id| *id == chat) {
        refresh_chats(rpc, state, sink, account).await?;
    }
    let object = rpc
        .request(
            "get_message",
            json!([state.rpc_account, message]),
            CALL_TIMEOUT,
        )
        .await?;
    let Some(projected) =
        project::message(account, &object, SELF_CONTACT, project::quoted(&object))
    else {
        return Ok(());
    };
    let chat_id = projected.chat.clone();
    let fresh = state.known.insert((chat_id.clone(), projected.id.clone()));
    if fresh {
        sink.send(Event::Incoming {
            chat: chat_id,
            message: Box::new(projected),
        });
    } else {
        sink.send(Event::MessageUpdated(Box::new(projected)));
    }
    Ok(())
}

async fn page(
    rpc: &Rpc,
    state: &mut State,
    sink: &Sink,
    account: AccountId,
    chat: &ChatId,
    before: Option<u64>,
) -> Result<()> {
    let Some(rpc_chat) = state.chats.get(chat).copied() else {
        return Ok(());
    };
    let ids_value = rpc
        .request(
            "get_message_ids",
            json!([state.rpc_account, rpc_chat, false, false]),
            CALL_TIMEOUT,
        )
        .await?;
    let ids: Vec<u64> = ids_value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .filter(|id| *id != 9)
        .collect();
    let end = match before {
        Some(before) => ids.iter().position(|id| *id == before).unwrap_or(0),
        None => ids.len(),
    };
    let start = end.saturating_sub(PAGE);
    let slice = &ids[start..end];
    if slice.is_empty() {
        sink.send(Event::Messages {
            chat: chat.clone(),
            messages: Vec::new(),
            older: before.is_some(),
            complete: start == 0,
        });
        return Ok(());
    }
    let objects = rpc
        .request(
            "get_messages",
            json!([state.rpc_account, slice]),
            CALL_TIMEOUT,
        )
        .await?;
    let mut messages = Vec::new();
    if let Some(map) = objects.as_object() {
        for id in slice {
            let Some(object) = map.get(&id.to_string()) else {
                continue;
            };
            if let Some(projected) =
                project::message(account, object, SELF_CONTACT, project::quoted(object))
            {
                state.known.insert((chat.clone(), projected.id.clone()));
                messages.push(projected);
            }
        }
    }
    if start > 0 {
        state.older.insert(chat.clone(), ids[start - 1]);
    } else {
        state.older.remove(chat);
    }
    sink.send(Event::Messages {
        chat: chat.clone(),
        messages,
        older: before.is_some(),
        complete: start == 0,
    });
    Ok(())
}

async fn handle_command(
    dirs: &AppDirs,
    rpc: &Rpc,
    state: &mut State,
    sink: &Sink,
    account: AccountId,
    command: Command,
) -> Result<()> {
    match command {
        Command::SendText {
            chat,
            text,
            quoting,
            ..
        } => {
            let Some(rpc_chat) = state.chats.get(&chat).copied() else {
                return Ok(());
            };
            let data = outgoing(&text, quoting.as_deref(), None);
            let id = send_msg(rpc, state.rpc_account, rpc_chat, data).await?;
            load_message(rpc, state, sink, account, rpc_chat, id).await
        }
        Command::SendFiles {
            chat,
            paths,
            caption,
            quoting,
            ..
        } => {
            let Some(rpc_chat) = state.chats.get(&chat).copied() else {
                return Ok(());
            };
            for (index, path) in paths.iter().enumerate() {
                let mime = mime_for(path);
                let viewtype = if mime.starts_with("image/") {
                    "Image"
                } else {
                    "File"
                };
                let caption = (index == 0).then(|| caption.clone()).flatten();
                let data = outgoing(
                    caption.as_deref().unwrap_or_default(),
                    quoting.as_deref(),
                    Some((path, viewtype)),
                );
                let id = send_msg(rpc, state.rpc_account, rpc_chat, data).await?;
                load_message(rpc, state, sink, account, rpc_chat, id).await?;
            }
            Ok(())
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
            let Some(rpc_chat) = state.chats.get(&chat).copied() else {
                return Ok(());
            };
            let image = image::RgbaImage::from_raw(width, height, rgba)
                .context("The pasted image has the wrong size")?;
            let mut png = Vec::new();
            image::DynamicImage::ImageRgba8(image)
                .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
            let path = dirs.media_cache_dir().join(format!(
                "delta-image-{}-{}.png",
                account.get(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|since| since.as_millis())
                    .unwrap_or(0)
            ));
            std::fs::write(&path, &png)?;
            let data = outgoing(
                caption.as_deref().unwrap_or_default(),
                quoting.as_deref(),
                Some((&path, "Image")),
            );
            let sent = send_msg(rpc, state.rpc_account, rpc_chat, data).await;
            let _ = std::fs::remove_file(&path);
            let id = sent?;
            load_message(rpc, state, sink, account, rpc_chat, id).await
        }
        Command::SendVoice {
            chat,
            samples,
            quoting,
            ..
        } => {
            let Some(rpc_chat) = state.chats.get(&chat).copied() else {
                return Ok(());
            };
            let bytes = crate::voice::encode(&samples).map_err(|error| anyhow::anyhow!(error))?;
            let path = dirs
                .media_cache_dir()
                .join(format!("delta-voice-{}.ogg", account.get()));
            std::fs::write(&path, &bytes)?;
            let data = outgoing("", quoting.as_deref(), Some((&path, "Voice")));
            let sent = send_msg(rpc, state.rpc_account, rpc_chat, data).await;
            let _ = std::fs::remove_file(&path);
            let id = sent?;
            load_message(rpc, state, sink, account, rpc_chat, id).await
        }
        Command::Download { chat, message, .. } => {
            let Some(rpc_chat) = state.chats.get(&chat).copied() else {
                return Ok(());
            };
            let Ok(msg_id) = message.parse::<u64>() else {
                return Ok(());
            };
            let object = rpc
                .request(
                    "get_message",
                    json!([state.rpc_account, msg_id]),
                    CALL_TIMEOUT,
                )
                .await?;
            let Some(projected) =
                project::message(account, &object, SELF_CONTACT, project::quoted(&object))
            else {
                return Ok(());
            };
            let path = dirs.media_cache_dir().join(format!(
                "delta-{}-{}",
                account.get(),
                safe_name(&message)
            ));
            let saved = rpc
                .request(
                    "save_msg_file",
                    json!([state.rpc_account, msg_id, path.to_string_lossy()]),
                    CALL_TIMEOUT,
                )
                .await;
            let mut updated = projected;
            if let Some(media) = updated.content.media_mut() {
                match saved {
                    Ok(_) => {
                        media.path = Some(path);
                        media.state = MediaState::Idle;
                    }
                    Err(error) => media.state = MediaState::Failed(format!("{error:#}")),
                }
            }
            state.known.insert((chat.clone(), updated.id.clone()));
            let _ = rpc_chat;
            sink.send(Event::MessageUpdated(Box::new(updated)));
            Ok(())
        }
        Command::LoadChat { chat, before } => {
            page(
                rpc,
                state,
                sink,
                account,
                &chat,
                before.map(|key| key.0.max(0) as u64),
            )
            .await
        }
        Command::FetchOlder(chat) => {
            let before = state.older.get(&chat).copied();
            page(rpc, state, sink, account, &chat, before).await
        }
        Command::MarkRead { chat, .. } => {
            let Some(rpc_chat) = state.chats.get(&chat).copied() else {
                return Ok(());
            };
            let ids_value = rpc
                .request(
                    "get_message_ids",
                    json!([state.rpc_account, rpc_chat, false, false]),
                    CALL_TIMEOUT,
                )
                .await?;
            let ids: Vec<u64> = ids_value
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_u64)
                .filter(|id| *id != 9)
                .collect();
            if !ids.is_empty() {
                let _ = rpc
                    .request(
                        "markseen_msgs",
                        json!([state.rpc_account, ids]),
                        CALL_TIMEOUT,
                    )
                    .await;
            }
            let items = rpc
                .request(
                    "get_chatlist_items_by_entries",
                    json!([state.rpc_account, [rpc_chat]]),
                    CALL_TIMEOUT,
                )
                .await?;
            if let Some(item) = items.get(rpc_chat.to_string())
                && let Some(row) = project::chat(account, item)
            {
                sink.send(Event::ChatUpdated(Box::new(row)));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn outgoing(text: &str, quoting: Option<&str>, file: Option<(&Path, &str)>) -> Value {
    let mut data = json!({});
    if !text.is_empty() {
        data["text"] = json!(text);
    }
    if let Some(quoting) = quoting
        && let Ok(id) = quoting.parse::<u64>()
    {
        data["quotedMessageId"] = json!(id);
    }
    if let Some((path, viewtype)) = file {
        data["file"] = json!(path.to_string_lossy());
        data["viewtype"] = json!(viewtype);
        if let Some(name) = path.file_name() {
            data["filename"] = json!(name.to_string_lossy());
        }
    }
    data
}

async fn send_msg(rpc: &Rpc, account: u32, chat: u64, data: Value) -> Result<u64> {
    rpc.request("send_msg", json!([account, chat, data]), CALL_TIMEOUT)
        .await?
        .as_u64()
        .context("Delta Chat did not return the new message")
}

fn mime_for(path: &Path) -> String {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("mp4") => "video/mp4",
        Some("ogg" | "opus") => "audio/ogg",
        Some("mp3") => "audio/mpeg",
        Some("pdf") => "application/pdf",
        Some("txt") => "text/plain",
        _ => "application/octet-stream",
    }
    .to_owned()
}

fn safe_name(id: &str) -> String {
    let cleaned: String = id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "message".to_owned()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_types_come_from_extensions() {
        assert_eq!(mime_for(Path::new("cat.JPG")), "image/jpeg");
        assert_eq!(mime_for(Path::new("note.ogg")), "audio/ogg");
        assert_eq!(mime_for(Path::new("plan.pdf")), "application/pdf");
        assert_eq!(mime_for(Path::new("blob")), "application/octet-stream");
    }

    #[test]
    fn names_keep_only_safe_characters() {
        assert_eq!(safe_name("101"), "101");
        assert_eq!(safe_name("a/b c"), "a_b_c");
        assert_eq!(safe_name(""), "message");
    }

    #[test]
    fn outgoing_data_carries_captions_files_and_quotes() {
        let data = outgoing(
            "look",
            Some("77"),
            Some((Path::new("/tmp/cat.png"), "Image")),
        );
        assert_eq!(data["text"], json!("look"));
        assert_eq!(data["quotedMessageId"], json!(77));
        assert_eq!(data["viewtype"], json!("Image"));
        assert_eq!(data["filename"], json!("cat.png"));
        let bare = outgoing("", None, None);
        assert!(bare.get("text").is_none());
    }
}
