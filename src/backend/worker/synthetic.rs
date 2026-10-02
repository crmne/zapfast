//! Demos and tests: the real worker and its send queue over a scripted,
//! offline link. Nothing connects to WhatsApp: the archive is in memory, the
//! client is never run, and each send that starts waits in
//! [`SyntheticLink::started`] until the caller says how it ended.

use super::{ChatId, Command, Event, Instant, Message, Worker};
use crate::backend::SendFailure;

pub(crate) struct SyntheticLink {
    runtime: tokio::runtime::Runtime,
    worker: Worker,
    events: std::sync::mpsc::Receiver<Event>,
}

impl SyntheticLink {
    /// A connected worker that knows `chats`, by id and name.
    pub(crate) fn new(chats: &[(&str, &str)]) -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("offline runtime");
        let (mut worker, events, _, _) = super::offline_worker("15550001111@s.whatsapp.net");
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // An in-memory store: the client only names messages. Never run it.
        let store = format!(
            "file:zapfast_synthetic_{}_{n}?mode=memory&cache=shared",
            std::process::id()
        );
        worker.client = Some(runtime.block_on(async {
            let store = whatsapp_rust::store::SqliteStore::new(&store)
                .await
                .expect("in-memory store");
            super::Bot::builder()
                .with_backend(store)
                .build()
                .await
                .expect("offline client")
                .client()
        }));
        worker.outgoing.synthetic = Some(Vec::new());
        for (id, name) in chats {
            // A saved contact, so the worker keeps the chat's name.
            worker.contacts.insert(
                (*id).to_owned(),
                crate::model::Contact {
                    id: (*id).to_owned(),
                    full_name: Some((*name).to_owned()),
                    first_name: None,
                    push_name: None,
                },
            );
            let _ = worker.archive.ensure_chat(id, name);
            // Opening a chat would ask the offline client for presence.
            worker.presence_subscribed.insert((*id).to_owned());
        }
        Self {
            runtime,
            worker,
            events,
        }
    }

    /// Hands one interface command to the worker.
    pub(crate) fn command(&mut self, command: Command) {
        self.runtime.block_on(self.worker.handle_command(command));
    }

    /// Stores a message as earlier history: read, and not announced.
    pub(crate) fn seed(&mut self, message: Message) {
        let (chat, id) = (message.chat.clone(), message.id.clone());
        self.worker.store_message(message, None, None);
        let _ = self.worker.archive.mark_read_to(&chat, &id);
        self.worker.emit_chat(&chat);
    }

    /// A message from a contact that arrives now.
    pub(crate) fn receive(&mut self, message: Message) {
        self.worker.store_message(message, None, None);
    }

    /// The reader saw everything in `chat`.
    pub(crate) fn read(&mut self, chat: &str) {
        if let Ok(rows) = self.worker.archive.messages(chat, None, 1)
            && let Some(newest) = rows.last()
        {
            let _ = self.worker.archive.mark_read_to(chat, &newest.id);
            self.worker.emit_chat(chat);
        }
    }

    /// The sends that started since the last call, oldest first.
    pub(crate) fn started(&mut self) -> Vec<(ChatId, String)> {
        self.worker
            .outgoing
            .synthetic
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
            .into_iter()
            .map(|job| (job.chat, job.id))
            .collect()
    }

    /// Reports how a started send ended, as the transport task does.
    pub(crate) fn finish(&mut self, chat: ChatId, id: String, result: Result<(), SendFailure>) {
        self.command(Command::OutgoingFinished { chat, id, result });
    }

    /// When the queue next needs the worker's timer.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.worker.outgoing_deadline()
    }

    /// Runs the queue as the worker's timer does at `now`.
    pub(crate) fn pump(&mut self, now: Instant) {
        self.worker.pump_outgoing_at(now);
    }

    /// The end of the rate-limit cooldown that runs at `now`, if any.
    pub(crate) fn cooldown_end(&self, now: Instant) -> Option<Instant> {
        self.worker.outgoing.retry_at.filter(|end| *end > now)
    }

    /// The chat of the first message that waits for a cooldown.
    pub(crate) fn waiting_chat(&self) -> Option<ChatId> {
        self.worker
            .outgoing
            .waiting
            .iter()
            .find(|job| job.waited)
            .map(|job| job.chat.clone())
    }

    /// An archived message, as the interface would load it.
    pub(crate) fn message(&self, chat: &str, id: &str) -> Option<Message> {
        self.worker.archive.message(chat, id).ok().flatten()
    }

    /// Everything the worker told the interface since the last call.
    pub(crate) fn events(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }

    /// Loses or restores the link, as a dropped connection would.
    #[cfg(test)]
    pub(crate) fn set_connected(&mut self, connected: bool) {
        self.worker.status = if connected {
            super::LinkStatus::Connected
        } else {
            super::LinkStatus::Disconnected {
                reason: String::new(),
            }
        };
        // As the worker does on a link change: a lost link fails unsent
        // messages, a restored one sends what waits.
        self.worker.pump_outgoing();
    }
}
