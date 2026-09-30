//! The Discord bot adapter: one gateway shard and REST client per account.
//!
//! A bot token from the OS keyring signs in with `GET /users/@me`, then a
//! gateway shard streams events and the REST client sends. Guilds are spaces
//! in the rail, their channels and threads are rows, and direct messages are
//! rows of their own. Only bot tokens are accepted; there is no user-token
//! path.

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use tokio::sync::mpsc;
use twilight_gateway::{EventTypeFlags, Intents, Shard, ShardId, StreamExt as _};
use twilight_http::Client;
use twilight_http::request::channel::reaction::RequestReactionType;
use twilight_model::channel::message::Message as DiscordMessage;
use twilight_model::channel::{Attachment, Channel, ChannelType};
use twilight_model::gateway::event::Event as GatewayEvent;
use twilight_model::id::Id;
use twilight_model::id::marker::{ChannelMarker, MessageMarker};
use twilight_model::user::CurrentUser;

use crate::account::{AccountId, AuthState};
use crate::backend::{Command, Event, LoginStep, Waker};
use crate::model::{Chat, ChatId, MediaState, Quoted};
use crate::paths::AppDirs;

use super::project;

/// How many messages one page holds.
const PAGE: u16 = 50;

/// What the bot cares about: guilds, their channels, messages, content,
/// reactions, and emoji and stickers.
const INTENTS: Intents = Intents::GUILDS
    .union(Intents::GUILD_MESSAGES)
    .union(Intents::DIRECT_MESSAGES)
    .union(Intents::MESSAGE_CONTENT)
    .union(Intents::GUILD_MESSAGE_REACTIONS)
    .union(Intents::GUILD_EMOJIS_AND_STICKERS);

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

/// The signed-in client and who it is.
struct Session {
    client: Client,
    token: String,
    user: CurrentUser,
}

#[derive(Default)]
struct State {
    /// The bot's own user id.
    me: u64,
    /// Rows the window knows, keyed by chat.
    chats: HashMap<ChatId, Chat>,
    /// Discord channel id to row.
    by_channel: HashMap<u64, ChatId>,
    /// Guild names, for channel rows created on the fly.
    guilds: HashMap<u64, String>,
    /// Names learned from authors.
    senders: HashMap<u64, String>,
    /// Raw messages, so history rows can be downloaded later.
    messages: HashMap<(ChatId, String), DiscordMessage>,
    /// The attachment behind a message, for downloads.
    sources: HashMap<(ChatId, String), Attachment>,
    /// Oldest known message id per chat, for older pages.
    tokens: HashMap<ChatId, u64>,
}

/// Runs the Discord account until it is shut down.
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
        log::warn!("Discord stopped: {error}");
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
    let Some(session) = authenticate_account(account, sink, &mut commands).await? else {
        return Ok(());
    };
    let mut state = State {
        me: session.user.id.get(),
        ..State::default()
    };
    let name = session
        .user
        .global_name
        .clone()
        .unwrap_or_else(|| session.user.name.clone());
    sink.send(Event::Me {
        id: state.me.to_string(),
        lid: None,
        name: Some(name),
        about: None,
    });
    load_chats(&session.client, account, &mut state, sink).await?;
    let mut shard = Shard::new(ShardId::ONE, session.token.clone(), INTENTS);
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                None | Some(Command::Shutdown) => return Ok(()),
                Some(command) => {
                    if let Err(error) =
                        handle_command(dirs, &session.client, &mut state, sink, account, command).await
                    {
                        log::warn!("Discord command failed: {error}");
                    }
                }
            },
            event = shard.next_event(EventTypeFlags::all()) => match event {
                Some(Ok(event)) => {
                    if let Err(error) =
                        handle_event(&session.client, &mut state, sink, account, event).await
                    {
                        log::warn!("Discord event failed: {error}");
                    }
                }
                Some(Err(error)) => {
                    log::warn!("Discord gateway error: {error}");
                    return Ok(());
                }
                None => return Ok(()),
            },
        }
    }
}

/// Waits for a bot token, verifies it, and keeps it in the OS keyring.
async fn authenticate_account(
    account: AccountId,
    sink: &Sink,
    commands: &mut mpsc::UnboundedReceiver<Command>,
) -> Result<Option<Session>> {
    let identity = format!("discord-{}", account.get());
    match crate::secrets::load(&identity).unwrap_or(None) {
        Some(token) => match verify_token(&token).await {
            Ok(user) => {
                sink.send(Event::Auth {
                    account,
                    state: AuthState::Ready,
                });
                return Ok(Some(Session {
                    client: Client::new(token.clone()),
                    token,
                    user,
                }));
            }
            Err(error) => {
                log::warn!("Discord token did not work: {error}");
                sink.send(Event::Auth {
                    account,
                    state: AuthState::Failed {
                        reason: "The stored Discord token did not work. Add the account again."
                            .to_owned(),
                    },
                });
            }
        },
        None => sink.send(Event::Auth {
            account,
            state: AuthState::SignedOut,
        }),
    }
    loop {
        match commands.recv().await {
            None | Some(Command::Shutdown) => return Ok(None),
            Some(Command::Login {
                account: target,
                step,
            }) if target == account => {
                let LoginStep::DiscordToken { token } = step else {
                    continue;
                };
                match verify_token(&token).await {
                    Ok(user) => {
                        let _ = crate::secrets::save(&identity, &token);
                        sink.send(Event::Auth {
                            account,
                            state: AuthState::Ready,
                        });
                        return Ok(Some(Session {
                            client: Client::new(token.clone()),
                            token,
                            user,
                        }));
                    }
                    Err(error) => {
                        log::warn!("Discord refused a token: {error}");
                        sink.send(Event::Auth {
                                account,
                                state: AuthState::Failed {
                                    reason: "Discord refused that bot token. Check the token and the bot's intents."
                                        .to_owned(),
                                },
                            });
                    }
                }
            }
            Some(_) => {}
        }
    }
}

async fn verify_token(token: &str) -> Result<CurrentUser> {
    Client::new(token.to_owned())
        .current_user()
        .await
        .context("Discord did not answer")?
        .model()
        .await
        .context("Discord refused the bot token")
}

/// Fetches the guilds and their channels and threads.
async fn load_chats(
    client: &Client,
    account: AccountId,
    state: &mut State,
    sink: &Sink,
) -> Result<()> {
    let guilds = client
        .current_user_guilds()
        .await
        .context("Discord would not list the guilds")?
        .model()
        .await
        .context("Discord would not list the guilds")?;
    let mut chats = Vec::new();
    for guild in guilds.iter() {
        state.guilds.insert(guild.id.get(), guild.name.clone());
        let guild_chat = project::guild_chat(account, guild.id.get(), &guild.name);
        state
            .chats
            .insert(guild_chat.id.clone(), guild_chat.clone());
        chats.push(guild_chat);
        let channels = client
            .guild_channels(guild.id)
            .await
            .context("Discord would not list the channels")?
            .model()
            .await
            .context("Discord would not list the channels")?;
        for channel in channels.iter() {
            if !worth_showing(channel) {
                continue;
            }
            let chat = project::channel_chat(
                account,
                channel,
                Some((guild.id.get(), &guild.name)),
                state.me,
            );
            state.by_channel.insert(channel.id.get(), chat.id.clone());
            state.chats.insert(chat.id.clone(), chat.clone());
            chats.push(chat);
        }
        let threads = client
            .active_threads(guild.id)
            .await
            .context("Discord would not list the threads")?
            .model()
            .await
            .context("Discord would not list the threads")?;
        for thread in threads.threads.iter() {
            let chat = project::channel_chat(
                account,
                thread,
                Some((guild.id.get(), &guild.name)),
                state.me,
            );
            state.by_channel.insert(thread.id.get(), chat.id.clone());
            state.chats.insert(chat.id.clone(), chat.clone());
            chats.push(chat);
        }
    }
    sink.send(Event::Chats(chats));
    Ok(())
}

fn worth_showing(channel: &Channel) -> bool {
    channel.kind == ChannelType::GuildText
        || channel.kind == ChannelType::GuildAnnouncement
        || channel.kind.is_thread()
}

/// The row for a channel, fetching it if the window has not seen it yet.
async fn ensure_chat(
    client: &Client,
    state: &mut State,
    sink: &Sink,
    account: AccountId,
    channel: u64,
) -> Result<ChatId> {
    if let Some(chat) = state.by_channel.get(&channel) {
        return Ok(chat.clone());
    }
    let model = client
        .channel(Id::<ChannelMarker>::new(channel))
        .await
        .context("Discord would not open the channel")?
        .model()
        .await
        .context("Discord would not open the channel")?;
    let guild = model.guild_id.and_then(|guild| {
        state
            .guilds
            .get(&guild.get())
            .map(String::as_str)
            .map(|name| (guild.get(), name))
    });
    let chat = project::channel_chat(account, &model, guild, state.me);
    state.by_channel.insert(channel, chat.id.clone());
    state.chats.insert(chat.id.clone(), chat.clone());
    sink.send(Event::ChatUpdated(Box::new(chat.clone())));
    Ok(chat.id)
}

async fn handle_event(
    client: &Client,
    state: &mut State,
    sink: &Sink,
    account: AccountId,
    event: GatewayEvent,
) -> Result<()> {
    match event {
        GatewayEvent::MessageCreate(payload) => {
            let message = payload.0;
            let chat =
                match ensure_chat(client, state, sink, account, message.channel_id.get()).await {
                    Ok(chat) => chat,
                    Err(error) => {
                        log::warn!("Discord message skipped: {error}");
                        return Ok(());
                    }
                };
            remember(state, &message);
            let Some(projected) = project::message(
                account,
                &chat,
                &message,
                state.me,
                state.senders.get(&message.author.id.get()).cloned(),
                quoted_of(&message),
            ) else {
                return Ok(());
            };
            if let Some(attachment) = message.attachments.first() {
                state
                    .sources
                    .insert((chat.clone(), projected.id.clone()), attachment.clone());
            }
            state
                .messages
                .insert((chat.clone(), projected.id.clone()), message);
            sink.send(Event::Incoming {
                chat,
                message: Box::new(projected),
            });
        }
        GatewayEvent::MessageUpdate(payload) => {
            let message = payload.0;
            let Some(chat) = state.by_channel.get(&message.channel_id.get()).cloned() else {
                return Ok(());
            };
            remember(state, &message);
            if message.content.is_empty() && message.attachments.is_empty() {
                return Ok(());
            }
            let Some(projected) = project::message(
                account,
                &chat,
                &message,
                state.me,
                state.senders.get(&message.author.id.get()).cloned(),
                quoted_of(&message),
            ) else {
                return Ok(());
            };
            state
                .messages
                .insert((chat.clone(), projected.id.clone()), message);
            sink.send(Event::MessageUpdated(Box::new(projected)));
        }
        GatewayEvent::MessageDelete(deleted) => {
            if let Some(chat) = state.by_channel.get(&deleted.channel_id.get()).cloned() {
                sink.send(Event::MessageDeleted {
                    chat,
                    id: deleted.id.get().to_string(),
                });
            }
        }
        GatewayEvent::MessageDeleteBulk(bulk) => {
            if let Some(chat) = state.by_channel.get(&bulk.channel_id.get()).cloned() {
                for id in &bulk.ids {
                    sink.send(Event::MessageDeleted {
                        chat: chat.clone(),
                        id: id.get().to_string(),
                    });
                }
            }
        }
        GatewayEvent::ReactionAdd(reaction) => {
            refresh_message(
                client,
                state,
                sink,
                account,
                reaction.channel_id.get(),
                reaction.message_id.get(),
            )
            .await?;
        }
        GatewayEvent::ReactionRemove(reaction) => {
            refresh_message(
                client,
                state,
                sink,
                account,
                reaction.channel_id.get(),
                reaction.message_id.get(),
            )
            .await?;
        }
        GatewayEvent::TypingStart(typing) => {
            if let Some(chat) = state.by_channel.get(&typing.channel_id.get()).cloned() {
                let sender = typing.user_id.get().to_string();
                sink.send(Event::Typing {
                    chat,
                    sender,
                    composing: true,
                });
            }
        }
        // Typing stops are not reported by Discord, and guild, channel, and
        // interaction events arrive through the listing and message paths.
        _ => {}
    }
    Ok(())
}

/// Re-projects one message, for reactions and edits.
async fn refresh_message(
    client: &Client,
    state: &mut State,
    sink: &Sink,
    account: AccountId,
    channel: u64,
    message: u64,
) -> Result<()> {
    let Some(chat) = state.by_channel.get(&channel).cloned() else {
        return Ok(());
    };
    let rows = client
        .channel_messages(Id::<ChannelMarker>::new(channel))
        .around(Id::<MessageMarker>::new(message))
        .limit(1)
        .await
        .context("Discord would not refresh the message")?
        .model()
        .await
        .context("Discord would not refresh the message")?;
    let Some(message) = rows.iter().find(|row| row.id.get() == message) else {
        return Ok(());
    };
    remember(state, message);
    let Some(projected) = project::message(
        account,
        &chat,
        message,
        state.me,
        state.senders.get(&message.author.id.get()).cloned(),
        quoted_of(message),
    ) else {
        return Ok(());
    };
    state
        .messages
        .insert((chat.clone(), projected.id.clone()), message.clone());
    sink.send(Event::MessageUpdated(Box::new(projected)));
    Ok(())
}

fn remember(state: &mut State, message: &DiscordMessage) {
    state
        .senders
        .entry(message.author.id.get())
        .or_insert_with(|| project::user_name(&message.author));
}

fn quoted_of(message: &DiscordMessage) -> Option<Quoted> {
    let reference = message.reference.as_ref()?;
    let id = reference.message_id?;
    let Some(original) = message.referenced_message.as_deref() else {
        return Some(Quoted {
            id: id.get().to_string(),
            sender: String::new(),
            sender_name: None,
            summary: "Reply".to_owned(),
            mentions: Vec::new(),
        });
    };
    Some(Quoted {
        id: id.get().to_string(),
        sender: original.author.id.get().to_string(),
        sender_name: Some(project::user_name(&original.author)),
        summary: summary_of(original),
        mentions: Vec::new(),
    })
}

fn summary_of(message: &DiscordMessage) -> String {
    if !message.content.is_empty() {
        return message.content.clone();
    }
    if let Some(attachment) = message.attachments.first() {
        let mime = attachment.content_type.clone().unwrap_or_default();
        return if mime.starts_with("image/") {
            "[photo]".to_owned()
        } else if mime.starts_with("video/") {
            "[video]".to_owned()
        } else if mime.starts_with("audio/") {
            "[audio]".to_owned()
        } else {
            "[file]".to_owned()
        };
    }
    if !message.sticker_items.is_empty() {
        return "[sticker]".to_owned();
    }
    "Message".to_owned()
}

async fn handle_command(
    dirs: &AppDirs,
    client: &Client,
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
            let Some(channel) = channel_of(&chat) else {
                bail!("not a Discord channel");
            };
            let request = client
                .create_message(Id::<ChannelMarker>::new(channel))
                .content(&text);
            let request = match reply_to(&quoting) {
                Some(message) => request.reply(message),
                None => request,
            };
            request
                .await
                .context("Discord would not send the message")?;
        }
        Command::SendFiles {
            chat,
            paths,
            caption,
            quoting,
            ..
        } => {
            let Some(channel) = channel_of(&chat) else {
                bail!("not a Discord channel");
            };
            for (index, path) in paths.iter().enumerate() {
                let bytes = std::fs::read(path)
                    .with_context(|| format!("could not read {}", path.display()))?;
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "file".to_owned());
                let attachment =
                    twilight_model::http::attachment::Attachment::from_bytes(name, bytes, 0);
                let attachments = [attachment];
                let request = client
                    .create_message(Id::<ChannelMarker>::new(channel))
                    .attachments(&attachments);
                let request = if index == 0 {
                    match reply_to(&quoting) {
                        Some(message) => request.reply(message),
                        None => request,
                    }
                } else {
                    request
                };
                let request = match index {
                    0 => request.content(caption.as_deref().unwrap_or("")),
                    _ => request,
                };
                request.await.context("Discord would not send the file")?;
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
            let Some(channel) = channel_of(&chat) else {
                bail!("not a Discord channel");
            };
            let image = image::RgbaImage::from_raw(width, height, rgba)
                .context("the image could not be read")?;
            let mut png = Vec::new();
            image::DynamicImage::ImageRgba8(image)
                .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .context("the image could not be encoded")?;
            let attachment = twilight_model::http::attachment::Attachment::from_bytes(
                "image.png".to_owned(),
                png,
                0,
            );
            let attachments = [attachment];
            let request = client
                .create_message(Id::<ChannelMarker>::new(channel))
                .attachments(&attachments)
                .content(caption.as_deref().unwrap_or(""));
            let request = match reply_to(&quoting) {
                Some(message) => request.reply(message),
                None => request,
            };
            request.await.context("Discord would not send the image")?;
        }
        Command::EditText { chat, id, text, .. } => {
            let (Some(channel), Ok(message)) = (channel_of(&chat), id.parse::<u64>()) else {
                bail!("not a Discord message");
            };
            client
                .update_message(
                    Id::<ChannelMarker>::new(channel),
                    Id::<MessageMarker>::new(message),
                )
                .content(Some(&text))
                .await
                .context("Discord would not edit the message")?;
        }
        Command::Revoke { chat, id } => {
            let (Some(channel), Ok(message)) = (channel_of(&chat), id.parse::<u64>()) else {
                bail!("not a Discord message");
            };
            client
                .delete_message(
                    Id::<ChannelMarker>::new(channel),
                    Id::<MessageMarker>::new(message),
                )
                .await
                .context("Discord would not delete the message")?;
        }
        Command::DeleteLocal { chat, id } => {
            sink.send(Event::MessageDeleted { chat, id });
        }
        Command::React {
            chat,
            message,
            emoji,
        } => {
            let (Some(channel), Ok(message)) = (channel_of(&chat), message.parse::<u64>()) else {
                bail!("not a Discord message");
            };
            client
                .create_reaction(
                    Id::<ChannelMarker>::new(channel),
                    Id::<MessageMarker>::new(message),
                    &RequestReactionType::Unicode { name: &emoji },
                )
                .await
                .context("Discord would not react")?;
        }
        Command::EnsureChat { chat, .. } => {
            if let Some(channel) = channel_of(&chat) {
                ensure_chat(client, state, sink, account, channel).await?;
            }
        }
        Command::LoadChat { chat, before } => {
            page(
                client,
                state,
                sink,
                account,
                &chat,
                before.map(|key| key.0 as u64),
            )
            .await?;
        }
        Command::FetchOlder(chat) => {
            let before = state.tokens.get(&chat).copied();
            page(client, state, sink, account, &chat, before).await?;
        }
        Command::Download { chat, message, .. } => {
            download(dirs, state, sink, account, &chat, &message).await?;
        }
        // Deferred: typing (bots cannot), read marks, drafts, forwarding,
        // search, stickers, voice notes, guild mute state, and polls.
        _ => {}
    }
    Ok(())
}

fn channel_of(chat: &ChatId) -> Option<u64> {
    match project::parse(chat) {
        Some(project::Target::Channel(id)) => Some(id),
        _ => None,
    }
}

fn reply_to(quoting: &Option<String>) -> Option<Id<MessageMarker>> {
    quoting
        .as_deref()
        .and_then(|id| id.parse::<u64>().ok())
        .map(Id::<MessageMarker>::new)
}

/// Pages history, newest first from Discord, oldest first to the window.
async fn page(
    client: &Client,
    state: &mut State,
    sink: &Sink,
    account: AccountId,
    chat: &ChatId,
    before: Option<u64>,
) -> Result<()> {
    let Some(channel) = channel_of(chat) else {
        bail!("not a Discord channel");
    };
    let request = client.channel_messages(Id::<ChannelMarker>::new(channel));
    let rows = match before {
        Some(id) => {
            request
                .before(Id::<MessageMarker>::new(id))
                .limit(PAGE + 1)
                .await
        }
        None => request.limit(PAGE + 1).await,
    }
    .context("Discord would not load the history")?
    .model()
    .await
    .context("Discord would not load the history")?;
    let older = rows.len() > usize::from(PAGE);
    let mut messages = Vec::new();
    for raw in rows.iter().take(usize::from(PAGE)) {
        remember(state, raw);
        if let Some(attachment) = raw.attachments.first()
            && let Some(projected) = project::message(
                account,
                chat,
                raw,
                state.me,
                state.senders.get(&raw.author.id.get()).cloned(),
                quoted_of(raw),
            )
        {
            state
                .sources
                .insert((chat.clone(), projected.id.clone()), attachment.clone());
        }
        let Some(projected) = project::message(
            account,
            chat,
            raw,
            state.me,
            state.senders.get(&raw.author.id.get()).cloned(),
            quoted_of(raw),
        ) else {
            continue;
        };
        state
            .messages
            .insert((chat.clone(), projected.id.clone()), raw.clone());
        messages.push(projected);
    }
    if let Some(oldest) = rows.iter().take(usize::from(PAGE)).next_back() {
        state.tokens.insert(chat.clone(), oldest.id.get());
    }
    messages.reverse();
    sink.send(Event::Messages {
        chat: chat.clone(),
        messages,
        older,
        complete: !older,
    });
    Ok(())
}

/// Downloads the attachment behind a message into the media cache.
async fn download(
    dirs: &AppDirs,
    state: &mut State,
    sink: &Sink,
    account: AccountId,
    chat: &ChatId,
    message: &str,
) -> Result<()> {
    let key = (chat.clone(), message.to_owned());
    let Some(attachment) = state.sources.get(&key).cloned() else {
        bail!("nothing to download");
    };
    let Some(raw) = state.messages.get(&key).cloned() else {
        bail!("nothing to download");
    };
    let path =
        dirs.media_cache_dir()
            .join(format!("discord-{}-{}", account.get(), safe_name(message)));
    let mut projected = project::message(
        account,
        chat,
        &raw,
        state.me,
        state.senders.get(&raw.author.id.get()).cloned(),
        quoted_of(&raw),
    )
    .context("the message has nothing to show")?;
    match fetch_to(&attachment.url, &path).await {
        Ok(()) => {
            if let Some(media) = projected.content.media_mut() {
                media.path = Some(path);
                media.state = MediaState::Idle;
            }
        }
        Err(error) => {
            if let Some(media) = projected.content.media_mut() {
                media.state = MediaState::Failed(error.to_string());
            }
        }
    }
    sink.send(Event::MessageUpdated(Box::new(projected)));
    Ok(())
}

async fn fetch_to(url: &str, path: &std::path::Path) -> Result<()> {
    let response = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .context("the download did not start")?
        .error_for_status()
        .context("the download was refused")?;
    let bytes = response
        .bytes()
        .await
        .context("the download was cut short")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    tokio::fs::write(path, &bytes)
        .await
        .context("the download could not be saved")
}

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
