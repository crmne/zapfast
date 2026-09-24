//! Favorite chats and their sync with the phone.
//!
//! The phone keeps one `favorites` app-state value (RegularHigh) holding the
//! whole ordered list. An update from the phone replaces ours, with changes
//! made here and not yet sent replayed on top (see `archive::favorites`). A
//! change made here sends the whole list back, one request at a time with a
//! capped backoff, and never before the phone's list is known: sending first
//! would overwrite favorites this computer has not seen yet.
//!
//! The phone's list may have arrived before ZapFast read it, and a phone that
//! never had favorites sends no update at all, so the first connection reads
//! RegularHigh once as a snapshot. Its completion travels on the same queue as
//! the replayed mutations: once it arrives, a list the phone had would already
//! have been applied, and a list still unknown means the phone has none.

use super::*;
use crate::archive::Favorite;
use whatsapp_rust::schemas;

/// One request at a time: the list is a single value, and a failure blocks
/// every later change as well.
#[derive(Default)]
pub(super) struct FavoriteChats {
    /// The snapshot that learns the phone's list is running.
    reading: bool,
    /// A list is on its way to the phone.
    sending: bool,
    /// The phone's list arrived while ours was being sent, so ours may lack
    /// its change: the queue is kept and the merged list goes out again.
    overtaken: bool,
    retry_at: Option<Instant>,
    failures: u32,
}

impl FavoriteChats {
    fn ready(&self, now: Instant) -> bool {
        !self.reading && !self.sending && self.retry_at.is_none_or(|retry| now >= retry)
    }

    fn settle(&mut self, success: bool, now: Instant) {
        if success {
            self.failures = 0;
            self.retry_at = None;
        } else {
            self.failures = self.failures.saturating_add(1);
            let seconds = (30 * (1_u64 << (self.failures - 1).min(5))).min(15 * 60);
            self.retry_at = Some(now + Duration::from_secs(seconds));
            log::warn!("favorite chats sync paused; retrying in {seconds} seconds");
        }
    }
}

/// The app-state value for a whole favorites list, made at `at` (ms).
pub(super) fn favorites_value(list: &[Favorite], at: i64) -> wa::SyncActionValue {
    wa::SyncActionValue {
        favorites_action: MessageField::some(wa::sync_action_value::FavoritesAction {
            favorites: list
                .iter()
                .map(
                    |favorite| wa::sync_action_value::favorites_action::Favorite {
                        id: Some(favorite.jid.clone()),
                    },
                )
                .collect(),
        }),
        timestamp: Some(at),
        ..Default::default()
    }
}

/// Channels cannot be favorites, as on the phone.
fn is_channel(chat: &str) -> bool {
    chat.ends_with("@newsletter")
}

impl Worker {
    /// The phone's list, which replaces ours.
    pub(super) fn favorite_chats_update(&mut self, update: &wa_events::FavoritesUpdate) {
        let mut list = Vec::new();
        for favorite in &update.action.favorites {
            let Some(jid) = favorite.id.as_deref().and_then(|id| id.parse::<Jid>().ok()) else {
                continue;
            };
            // A privacy id learned later moves the favorite with its chat.
            let chat = self.canonical(&jid);
            if is_channel(&chat) {
                continue;
            }
            list.push(Favorite {
                chat,
                jid: jid.to_non_ad_string(),
            });
        }
        let at = update.timestamp.timestamp_millis();
        match self.archive.apply_phone_favorites(&list, at) {
            Ok(Some(touched)) => {
                if self.favorite_chats.sending {
                    self.favorite_chats.overtaken = true;
                }
                for chat in touched {
                    self.emit_chat(&chat);
                }
                self.pump_favorite_chats();
            }
            Ok(None) => log::debug!("ignored an older favorites list"),
            Err(error) => log::warn!("could not apply the phone's favorites: {error}"),
        }
    }

    /// Marks or unmarks a chat here and queues the change for the phone.
    pub(super) fn set_favorite_chat(&mut self, chat: &str, favorite: bool) {
        if is_channel(chat) {
            return;
        }
        match self.archive.set_favorite(chat, favorite) {
            Ok(true) => {
                // Every favorite's place may have shifted after a removal.
                for favorite in self.archive.favorites().unwrap_or_default() {
                    if favorite.chat != chat {
                        self.emit_chat(&favorite.chat);
                    }
                }
                self.emit_chat(chat);
                self.pump_favorite_chats();
            }
            Ok(false) => self.emit_chat(chat),
            Err(error) => self.emit(Event::Error(error.to_string())),
        }
    }

    /// The list to send and the newest queued change it holds, when the
    /// phone's list is known and something waits for it.
    pub(super) fn favorites_to_send(&self) -> Option<(i64, Vec<Favorite>)> {
        self.archive.favorites_synced_at().ok().flatten()?;
        let through = self.archive.pending_favorites().ok().flatten()?;
        Some((through, self.archive.favorites().ok()?))
    }

    /// Learns the phone's list once, then sends queued changes.
    pub(super) fn pump_favorite_chats(&mut self) {
        let now = Instant::now();
        if !matches!(self.status, LinkStatus::Connected) || !self.favorite_chats.ready(now) {
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };
        if self.archive.favorites_synced_at().ok().flatten().is_none() {
            self.favorite_chats.reading = true;
            let sender = self.wa_sender.clone();
            let generation = self.privacy_generation;
            tokio::spawn(async move {
                use whatsapp_rust::WAPatchName;
                let complete = match client
                    .resync_app_state(
                        [WAPatchName::RegularHigh],
                        whatsapp_rust::AppStateResyncMode::Snapshot,
                    )
                    .await
                {
                    Ok(report) => report.synced.contains(&WAPatchName::RegularHigh),
                    Err(error) => {
                        log::warn!("could not read favorite chats from the phone: {error}");
                        false
                    }
                };
                let _ = sender.send(RuntimeEvent::FavoriteChatsRead {
                    generation,
                    complete,
                });
            });
            return;
        }
        let Some((through, list)) = self.favorites_to_send() else {
            return;
        };
        self.favorite_chats.sending = true;
        self.favorite_chats.overtaken = false;
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let at = jiff::Timestamp::now().as_millisecond();
            let value = favorites_value(&list, at);
            let result = client
                .send_app_state_action(&schemas::FAVORITES, &[], &value)
                .await;
            if let Err(error) = &result {
                log::debug!("favorite chats not synced: {error}");
            }
            let _ = commands.send(Command::FavoritesSent {
                through,
                at,
                success: result.is_ok(),
            });
        });
    }

    /// The snapshot finished. Every list the phone had is applied by now, so
    /// a list still unknown is an empty one.
    pub(super) fn favorite_chats_read(&mut self, generation: u64, complete: bool) {
        if generation != self.privacy_generation {
            return;
        }
        self.favorite_chats.reading = false;
        self.favorite_chats.settle(complete, Instant::now());
        if !complete {
            return;
        }
        if self.archive.favorites_synced_at().ok().flatten().is_none() {
            match self.archive.apply_phone_favorites(&[], 0) {
                Ok(touched) => {
                    for chat in touched.unwrap_or_default() {
                        self.emit_chat(&chat);
                    }
                }
                Err(error) => log::warn!("could not record the favorites sync: {error}"),
            }
        }
        self.pump_favorite_chats();
    }

    pub(super) fn favorites_sent(&mut self, through: i64, at: i64, success: bool) {
        if !self.favorite_chats.sending {
            return;
        }
        self.favorite_chats.sending = false;
        self.favorite_chats.settle(success, Instant::now());
        if !success {
            return;
        }
        if std::mem::take(&mut self.favorite_chats.overtaken) {
            // The merged list still holds our changes; send it again.
            self.pump_favorite_chats();
            return;
        }
        if let Err(error) = self.archive.favorites_sent(through, at) {
            log::warn!("could not record the favorites sync: {error}");
        }
        self.pump_favorite_chats();
    }
}

#[cfg(test)]
mod tests {
    use super::super::receipt_tests::{PEER, worker};
    use super::*;

    const GROUP: &str = "120363000000000042@g.us";
    const CHANNEL: &str = "120363000000000099@newsletter";

    fn phone_list(worker: &mut Worker, ids: &[&str], at: i64) {
        let update = wa_events::FavoritesUpdate::builder()
            .timestamp(whatsapp_rust::wacore::time::from_millis_or_now(at))
            .action(Box::new(wa::sync_action_value::FavoritesAction {
                favorites: ids
                    .iter()
                    .map(|id| wa::sync_action_value::favorites_action::Favorite {
                        id: Some((*id).into()),
                    })
                    .collect(),
            }))
            .from_full_sync(false)
            .build();
        worker.favorite_chats_update(&update);
    }

    fn chats(worker: &Worker) -> Vec<String> {
        let favorites = worker.archive.favorites().unwrap();
        favorites
            .into_iter()
            .map(|favorite| favorite.chat)
            .collect()
    }

    #[tokio::test]
    async fn the_phone_list_replaces_ours_under_phone_numbers_and_in_order() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.learn_lid("167650256810092", "4917663430455");
        worker.archive.ensure_chat(PEER, "Ada").unwrap();
        worker.archive.ensure_chat(GROUP, "Plans").unwrap();
        phone_list(&mut worker, &["15550000002@s.whatsapp.net"], 1_000);
        phone_list(
            &mut worker,
            &[GROUP, "167650256810092@lid", CHANNEL, GROUP],
            2_000,
        );
        assert_eq!(chats(&worker), [GROUP, PEER]);
        let favorites = worker.archive.favorites().unwrap();
        assert_eq!(
            favorites[1].jid, "167650256810092@lid",
            "the phone's own name goes back to it"
        );
        let peer = worker.archive.chat(PEER).unwrap().unwrap();
        assert!(peer.favorite);
        assert_eq!(peer.favorite_position, 1);
        assert!(worker.favorites_to_send().is_none(), "nothing to send back");
    }

    #[tokio::test]
    async fn a_favorite_under_a_privacy_id_moves_to_the_phone_number() {
        let (mut worker, _events, _inbox, _wa) = worker();
        phone_list(&mut worker, &["167650256810092@lid"], 1_000);
        assert_eq!(chats(&worker), ["167650256810092@lid"]);
        worker.learn_lid("167650256810092", "4917663430455");
        assert_eq!(chats(&worker), [PEER]);
    }

    #[tokio::test]
    async fn an_older_list_from_the_phone_is_ignored() {
        let (mut worker, _events, _inbox, _wa) = worker();
        phone_list(&mut worker, &[GROUP], 2_000);
        phone_list(&mut worker, &[PEER], 1_000);
        assert_eq!(chats(&worker), [GROUP]);
    }

    #[tokio::test]
    async fn a_favorite_made_before_the_phone_list_waits_and_joins_it() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.archive.ensure_chat(PEER, "Ada").unwrap();
        worker.set_favorite_chat(PEER, true);
        assert!(worker.archive.chat(PEER).unwrap().unwrap().favorite);
        assert!(
            worker.favorites_to_send().is_none(),
            "the phone's list is unknown, so ours cannot replace it"
        );
        phone_list(&mut worker, &[GROUP], 1_000);
        assert_eq!(chats(&worker), [GROUP, PEER]);
        let (through, list) = worker.favorites_to_send().expect("the merged list");
        let value = favorites_value(&list, 5_000);
        let ids: Vec<_> = value
            .favorites_action
            .as_option()
            .expect("the action")
            .favorites
            .iter()
            .map(|favorite| favorite.id.clone().unwrap_or_default())
            .collect();
        assert_eq!(ids, [GROUP, PEER], "the whole list, in order");
        assert_eq!(value.timestamp, Some(5_000));
        worker.favorite_chats.sending = true;
        worker.favorites_sent(through, 5_000, true);
        assert!(worker.favorites_to_send().is_none());
        phone_list(&mut worker, &[], 4_000);
        assert_eq!(chats(&worker), [GROUP, PEER], "older than what we sent");
    }

    #[tokio::test]
    async fn a_phone_without_favorites_lets_ours_go_out() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.set_favorite_chat(PEER, true);
        worker.favorite_chats.reading = true;
        worker.favorite_chats_read(worker.privacy_generation, true);
        assert_eq!(worker.archive.favorites_synced_at().unwrap(), Some(0));
        let (_, list) = worker.favorites_to_send().expect("ours");
        assert_eq!(list.len(), 1);
    }

    #[tokio::test]
    async fn a_list_that_overtakes_ours_is_sent_again_merged() {
        let (mut worker, _events, _inbox, _wa) = worker();
        phone_list(&mut worker, &[], 1_000);
        worker.set_favorite_chat(PEER, true);
        let (through, _) = worker.favorites_to_send().expect("ours");
        worker.favorite_chats.sending = true;
        phone_list(&mut worker, &[GROUP], 3_000);
        worker.favorites_sent(through, 2_000, true);
        let (_, list) = worker.favorites_to_send().expect("still queued");
        let list: Vec<_> = list.into_iter().map(|favorite| favorite.chat).collect();
        assert_eq!(list, [GROUP, PEER]);
    }

    #[tokio::test]
    async fn channels_are_never_favorites() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.archive.ensure_chat(CHANNEL, "News").unwrap();
        worker.set_favorite_chat(CHANNEL, true);
        phone_list(&mut worker, &[CHANNEL], 1_000);
        assert!(chats(&worker).is_empty());
        assert!(worker.archive.pending_favorites().unwrap().is_none());
    }
}
