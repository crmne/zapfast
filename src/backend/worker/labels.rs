//! Synced WhatsApp labels/lists (`label_edit` and `label_jid`).
//!
//! Custom lists and WhatsApp Business labels live in the phone's `regular`
//! app-state collection. An already-linked account replays that collection
//! once via a snapshot so existing tags and chat associations appear without
//! relinking.

use super::*;
use whatsapp_rust::waproto::buffa::Enumeration as _;

/// Archive marker: the phone's labels and chat-label associations were replayed.
pub(super) const LABELS_RECOVERED: &str = "labels_recovered_v1";

impl Worker {
    /// Applies a `label_edit` mutation from the phone.
    pub(super) fn label_edit_update(&mut self, update: &wa_events::LabelEditUpdate) {
        let at = update
            .action_timestamp
            .unwrap_or(update.timestamp)
            .timestamp_millis();
        let phone = crate::archive::PhoneLabel {
            id: &update.label_id,
            name: update.action.name.as_deref(),
            color: update.action.color,
            list_type: update.action.r#type.map(|t| t.to_i32()),
            predefined_id: update.action.predefined_id,
            order_index: update.action.order_index,
            is_active: update.action.is_active,
            deleted: update.action.deleted.unwrap_or(false),
            updated_at: at,
        };
        match self.archive.apply_phone_label(&phone) {
            Ok(true) => {
                self.emit_labels();
                // A label_jid mutation may have arrived before its label_edit,
                // or a deleted label may have been hidden.
                self.emit_chats();
            }
            Ok(false) => {}
            Err(error) => log::warn!("could not store synced label: {error}"),
        }
    }

    /// Applies a `label_jid` mutation from the phone.
    pub(super) fn label_association_update(&mut self, update: &wa_events::LabelAssociationUpdate) {
        let chat = self.canonical(&update.chat_jid);
        let at = update
            .action_timestamp
            .unwrap_or(update.timestamp)
            .timestamp_millis();
        let labeled = update.action.labeled.unwrap_or(false);
        match self
            .archive
            .apply_phone_chat_label(&update.label_id, &chat, labeled, at)
        {
            Ok(true) => self.emit_chat(&chat),
            Ok(false) => {}
            Err(error) => log::warn!("could not store synced chat label: {error}"),
        }
    }

    /// Adds or removes a synced WhatsApp label/list on a chat and tells the phone.
    pub(super) fn set_synced_chat_label(&mut self, chat: &str, label_id: &str, labeled: bool) {
        let Some(raw_id) = crate::archive::synced_label_raw_id(label_id).map(str::to_owned) else {
            return;
        };
        let at = jiff::Timestamp::now().as_millisecond();
        if let Err(error) = self
            .archive
            .apply_phone_chat_label(&raw_id, chat, labeled, at)
        {
            log::warn!("could not update synced chat label: {error}");
        }
        self.emit_chat(chat);
        self.tell_phone(chat, move |client, jid| async move {
            if labeled {
                client.labels().add_chat_label(&raw_id, &jid).await
            } else {
                client.labels().remove_chat_label(&raw_id, &jid).await
            }
            .map_err(|error| error.to_string())
        });
    }

    /// Replays the `regular` collection once for accounts linked before
    /// ZapFast followed WhatsApp labels/lists.
    pub(super) fn recover_labels(&mut self) {
        if self.labels_recovered
            || self.labels_recovering
            || !matches!(self.status, LinkStatus::Connected)
        {
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };
        self.labels_recovering = true;
        log::info!("reading WhatsApp labels and lists from the phone");
        let sender = self.wa_sender.clone();
        let generation = self.privacy_generation;
        tokio::spawn(async move {
            use whatsapp_rust::WAPatchName;
            let complete = match client
                .resync_app_state(
                    [WAPatchName::Regular],
                    whatsapp_rust::AppStateResyncMode::Snapshot,
                )
                .await
            {
                Ok(report) => report.synced.contains(&WAPatchName::Regular),
                Err(error) => {
                    log::warn!("could not read WhatsApp labels from the phone: {error}");
                    false
                }
            };
            // Travels on the same queue as the replayed mutations, so every
            // label and association is in the archive before this arrives.
            let _ = sender.send(RuntimeEvent::LabelsRecovered {
                generation,
                complete,
            });
        });
    }

    /// The one-time label replay finished, or waits for the next connection.
    pub(super) fn labels_recovered(&mut self, generation: u64, complete: bool) {
        if generation != self.privacy_generation {
            return;
        }
        self.labels_recovering = false;
        if !complete {
            log::warn!(
                "WhatsApp labels from the phone not read yet; retrying on the next connection"
            );
            return;
        }
        self.labels_recovered = true;
        if let Err(error) = self.archive.set_meta(LABELS_RECOVERED, "complete") {
            log::warn!("could not record the label replay: {error}");
        }
        self.emit_labels();
        self.emit_chats();
    }
}

#[cfg(test)]
mod tests {
    use super::super::receipt_tests::{PEER, worker};
    use super::*;

    #[test]
    fn label_edit_and_association_updates_reach_archive() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.archive.ensure_chat(PEER, "Ada").unwrap();

        let edit = wa_events::LabelEditUpdate::builder()
            .label_id("42".to_owned())
            .timestamp(whatsapp_rust::wacore::time::from_secs(100).unwrap())
            .action_timestamp(whatsapp_rust::wacore::time::from_secs(100).unwrap())
            .action(Box::new({
                let mut action = wa::sync_action_value::LabelEditAction::default();
                action.name = Some("Customers".to_owned());
                action.r#type = Some(wa::sync_action_value::label_edit_action::ListType::CUSTOM);
                action.is_active = Some(true);
                action.order_index = Some(1);
                action
            }))
            .from_full_sync(false)
            .build();
        worker.label_edit_update(&edit);

        let assoc = wa_events::LabelAssociationUpdate::builder()
            .label_id("42".to_owned())
            .chat_jid(PEER.parse().unwrap())
            .timestamp(whatsapp_rust::wacore::time::from_secs(110).unwrap())
            .action_timestamp(whatsapp_rust::wacore::time::from_secs(110).unwrap())
            .action(Box::new({
                let mut action = wa::sync_action_value::LabelAssociationAction::default();
                action.labeled = Some(true);
                action
            }))
            .from_full_sync(false)
            .build();
        worker.label_association_update(&assoc);

        let labels = worker.archive.labels().unwrap();
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].id, "wa:42");
        assert_eq!(labels[0].name, "Customers");
        assert!(labels[0].synced);
        assert_eq!(worker.archive.chat_labels(PEER).unwrap(), vec!["wa:42"]);
    }
}
