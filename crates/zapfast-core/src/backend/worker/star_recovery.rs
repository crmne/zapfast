//! Stars made before ZapFast followed them.
//!
//! A star lives in the phone's RegularHigh app-state collection, and an
//! incremental sync brings only what changed since the last one. Stars made
//! before ZapFast 0.20 kept them were consumed by earlier syncs and never come
//! again, so the Starred list showed only stars made since (#604). Rebuilding
//! RegularHigh once from a snapshot replays every star as a full-sync
//! `StarUpdate`, which the archive files like any other (a replayed star never
//! undoes a newer unstar). A new link receives them with its first sync and
//! skips this.

use super::*;

/// Archive marker: the phone's stars were replayed.
pub(super) const STARS_RECOVERED: &str = "stars_recovered_v1";

impl Worker {
    /// Replays the phone's stars once, so stars made before ZapFast followed
    /// them reach the Starred list.
    pub(super) fn recover_stars(&mut self) {
        if self.stars_recovered
            || self.stars_recovering
            || !matches!(self.status, LinkStatus::Connected)
        {
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };
        self.stars_recovering = true;
        log::info!("reading starred messages from the phone");
        let commands = self.commands.clone();
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
                    log::warn!("could not read starred messages from the phone: {error}");
                    false
                }
            };
            let _ = commands.send(Command::StarsRecovered { complete });
        });
    }

    /// The one-time star replay finished, or waits for the next connection.
    pub(super) fn stars_recovered(&mut self, complete: bool) {
        self.stars_recovering = false;
        if !complete {
            log::warn!(
                "starred messages from the phone not read yet; retrying on the next connection"
            );
            return;
        }
        self.stars_recovered = true;
        if let Err(error) = self.archive.set_meta(STARS_RECOVERED, "complete") {
            log::warn!("could not record the star replay: {error}");
        }
        // Replayed stars were filed one by one; the list shows them all now.
        self.emit_starred();
    }
}

#[cfg(test)]
mod tests {
    use super::super::receipt_tests::{PEER, worker};
    use super::*;

    #[test]
    fn the_snapshot_replay_brings_stars_made_before_they_were_followed() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.stars_recovered = false;
        worker
            .archive
            .insert_message(
                &Message {
                    id: "old-star".into(),
                    chat: PEER.into(),
                    sender: PEER.into(),
                    sender_name: None,
                    from_me: false,
                    timestamp: 100,
                    content: Content::text("Starred long ago"),
                    status: Delivery::None,
                    delivered_at: None,
                    read_at: None,
                    quoted: None,
                    reactions: Vec::new(),
                    history_order: None,
                    edited: false,
                    mentions: Vec::new(),
                    forwarded: false,
                    thumbnail: None,
                },
                None,
            )
            .unwrap();
        // The snapshot replays the star as a full-sync update.
        worker.star_update(
            &wa_events::StarUpdate::builder()
                .chat_jid(PEER.parse().unwrap())
                .message_id("old-star".to_owned())
                .from_me(false)
                .timestamp(whatsapp_rust::wacore::time::from_secs(200).unwrap())
                .action(Box::new({
                    let mut action = wa::sync_action_value::StarAction::default();
                    action.starred = Some(true);
                    action
                }))
                .from_full_sync(true)
                .build(),
        );
        assert!(
            worker
                .archive
                .starred_ids(PEER)
                .unwrap()
                .contains("old-star")
        );

        // An unfinished replay is asked again on the next connection.
        worker.stars_recovering = true;
        worker.stars_recovered(false);
        assert!(!worker.stars_recovered && !worker.stars_recovering);
        assert_eq!(worker.archive.meta(STARS_RECOVERED).unwrap(), None);
        // A finished one is never asked again.
        worker.stars_recovered(true);
        assert!(worker.stars_recovered);
        assert_eq!(
            worker.archive.meta(STARS_RECOVERED).unwrap().as_deref(),
            Some("complete")
        );
    }
}
