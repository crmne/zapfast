//! Expiring Status are stored separately from conversations and never create chat receipts.

use super::*;
use crate::model::MediaState;
use crate::status::{Item, LIFETIME, LIMIT, MEDIA_LIMIT};

struct Entry {
    item: Item,
    raw: Arc<wa::Message>,
    token: u64,
    file: Option<tempfile::TempDir>,
    download: Option<tokio::task::AbortHandle>,
}

impl Drop for Entry {
    fn drop(&mut self) {
        if let Some(download) = self.download.take() {
            download.abort();
        }
    }
}

#[derive(Default)]
pub(super) struct Posts {
    entries: Vec<Entry>,
    revoked: HashMap<(String, String), i64>,
    seen: HashSet<(String, String)>,
    restored: bool,
    importing: bool,
    audiences: HashMap<String, (Vec<Jid>, whatsapp_rust::StatusPrivacySetting)>,
    viewers: HashMap<String, Vec<crate::status::Viewer>>,
    sequence: u64,
    publication: Option<(u64, tokio::task::AbortHandle)>,
}

impl Drop for Posts {
    fn drop(&mut self) {
        for entry in &mut self.entries {
            if entry.item.alive(crate::util::now())
                && let Some(file) = entry.file.take()
            {
                let _ = file.keep();
            }
        }
        if let Some((_, task)) = self.publication.take() {
            task.abort();
        }
    }
}

impl Posts {
    pub(super) fn remap(&mut self, from: &str, to: &str) -> bool {
        if !self.entries.iter().any(|entry| entry.item.sender == from)
            && !self.revoked.keys().any(|(sender, _)| sender == from)
            && !self
                .viewers
                .values()
                .any(|views| views.iter().any(|view| view.contact == from))
        {
            return false;
        }
        for entry in &mut self.entries {
            if entry.item.sender == from {
                entry.item.sender = to.into();
            }
        }
        for views in self.viewers.values_mut() {
            for view in views.iter_mut() {
                if view.contact == from {
                    view.contact = to.to_owned();
                }
            }
            let mut seen = HashSet::new();
            views.retain(|view| seen.insert(view.contact.clone()));
        }
        let mut seen = HashSet::new();
        self.entries
            .retain(|entry| seen.insert((entry.item.sender.clone(), entry.item.id.clone())));
        self.seen = std::mem::take(&mut self.seen)
            .into_iter()
            .map(|(sender, id)| {
                (
                    if sender == from {
                        to.to_owned()
                    } else {
                        sender
                    },
                    id,
                )
            })
            .collect();
        let revoked = std::mem::take(&mut self.revoked);
        for ((sender, id), until) in revoked {
            let sender = if sender == from {
                to.to_owned()
            } else {
                sender
            };
            let expires = self.revoked.entry((sender, id)).or_default();
            *expires = (*expires).max(until);
        }
        true
    }

    /// Remembers a deletion so a late delivery cannot restore the update. Memory stays
    /// bounded by forgetting the oldest: first the same author's, so one contact's
    /// flood of deletions never pushes out anyone else's, then the oldest overall.
    fn revoke(&mut self, sender: String, id: String, until: i64) {
        let added = (sender, id);
        // Never the one just added: deletions made in the same second tie.
        let oldest = |revoked: &HashMap<(String, String), i64>, author: Option<&str>| {
            revoked
                .iter()
                .filter(|(key, _)| **key != added && author.is_none_or(|author| key.0 == author))
                .min_by_key(|(_, until)| **until)
                .map(|(key, _)| key.clone())
        };
        let expires = self.revoked.entry(added.clone()).or_default();
        *expires = (*expires).max(until);
        let evict = if self
            .revoked
            .keys()
            .filter(|(from, _)| *from == added.0)
            .count()
            > crate::status::REVOKE_PER_SENDER
        {
            oldest(&self.revoked, Some(&added.0))
        } else if self.revoked.len() > crate::status::REVOKE_LIMIT {
            oldest(&self.revoked, None)
        } else {
            None
        };
        if let Some(key) = evict {
            self.revoked.remove(&key);
        }
    }

    pub(super) fn clear(&mut self) {
        if let Some((_, task)) = self.publication.take() {
            task.abort();
        }
        self.restored = false;
        self.importing = false;
        self.seen.clear();
        self.audiences.clear();
        self.viewers.clear();
        self.entries.clear();
        self.revoked.clear();
        // Keep the sequence: a download from the unlinked account may finish later.
    }
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct SavedDetails {
    audience: Option<(Vec<String>, crate::status::AudienceMode)>,
    viewers: Vec<crate::status::Viewer>,
    seen: bool,
}

impl Worker {
    fn saved_status_record(&self, entry: &Entry) -> Option<crate::archive::StatusRecord> {
        let raw = entry.raw.encode_to_vec();
        if raw.len() > crate::status::RAW_LIMIT {
            return None;
        }
        let audience =
            self.status_posts
                .audiences
                .get(&entry.item.id)
                .and_then(|(recipients, privacy)| {
                    let mode = match privacy {
                        whatsapp_rust::StatusPrivacySetting::Contacts => {
                            crate::status::AudienceMode::Contacts
                        }
                        whatsapp_rust::StatusPrivacySetting::AllowList => {
                            crate::status::AudienceMode::Only
                        }
                        whatsapp_rust::StatusPrivacySetting::DenyList => {
                            crate::status::AudienceMode::Except
                        }
                        _ => return None,
                    };
                    Some((recipients.iter().map(ToString::to_string).collect(), mode))
                });
        let metadata = serde_json::to_string(&SavedDetails {
            audience,
            viewers: self
                .status_posts
                .viewers
                .get(&entry.item.id)
                .cloned()
                .unwrap_or_default(),
            seen: self
                .status_posts
                .seen
                .contains(&(entry.item.sender.clone(), entry.item.id.clone())),
        })
        .ok()?;
        Some(crate::archive::StatusRecord {
            sender: entry.item.sender.clone(),
            id: entry.item.id.clone(),
            posted_at: entry.item.timestamp,
            own: self.is_me(&entry.item.sender),
            name: entry.item.name.clone(),
            raw,
            media_path: entry
                .item
                .content
                .media()
                .and_then(|media| media.path.as_ref())
                .and_then(|path| path.to_str())
                .map(str::to_owned),
            metadata,
        })
    }

    fn save_status_record(&self, sender: &str, id: &str) {
        if self.status_posts.importing {
            return;
        }
        if let Some(record) = self
            .status_posts
            .entries
            .iter()
            .find(|entry| entry.item.sender == sender && entry.item.id == id)
            .and_then(|entry| self.saved_status_record(entry))
            && self
                .archive
                .save_statuses(&[record], crate::util::now())
                .is_err()
        {
            self.emit(Event::Info(
                "Could not save a status update for the next restart".into(),
            ));
        }
    }

    fn trim_statuses(&mut self) {
        let me = self.me();
        let lid = self.me_lid.clone();
        let theirs =
            |entry: &Entry| entry.item.sender != me && Some(&entry.item.sender) != lid.as_ref();
        self.status_posts.entries.sort_by(|a, b| {
            theirs(a)
                .cmp(&theirs(b))
                .then_with(|| b.item.timestamp.cmp(&a.item.timestamp))
                .then_with(|| a.item.id.cmp(&b.item.id))
        });
        let trimmed = self.status_posts.entries.len() > LIMIT;
        self.status_posts.entries.truncate(LIMIT);
        self.status_posts
            .entries
            .sort_by_key(|entry| std::cmp::Reverse((entry.item.timestamp, entry.token)));
        if trimmed {
            self.prune_status_media();
        }
    }

    pub(super) fn restore_statuses(&mut self) {
        if self.status_posts.restored {
            self.emit_status_local_state();
            return;
        }
        let now = crate::util::now();
        let result = (|| -> crate::archive::Result<_> {
            self.archive.save_statuses(&[], now)?;
            Ok((
                self.archive.saved_statuses(now)?,
                self.archive.status_revocations(now)?,
            ))
        })();
        let Ok((records, revoked)) = result else {
            self.emit(Event::Info("Could not restore saved status updates".into()));
            return;
        };
        self.status_posts.restored = true;
        for (sender, id, until) in revoked {
            let sender = self.canonical_str(&sender);
            self.status_posts.revoke(sender, id, until);
        }
        for record in records {
            let sender = self.canonical_str(&record.sender);
            if self
                .status_posts
                .revoked
                .contains_key(&(sender.clone(), record.id.clone()))
                || self
                    .status_posts
                    .entries
                    .iter()
                    .any(|entry| entry.item.sender == sender && entry.item.id == record.id)
            {
                continue;
            }
            if record.raw.len() > crate::status::RAW_LIMIT
                || record.metadata.len() > crate::status::METADATA_LIMIT
            {
                continue;
            }
            let Ok(raw) = wa::Message::decode_from_slice(&record.raw) else {
                continue;
            };
            let Some(mut content) = classify(&raw) else {
                continue;
            };
            if !matches!(
                content,
                Content::Text { .. }
                    | Content::Image { .. }
                    | Content::Video { .. }
                    | Content::Audio { .. }
            ) {
                continue;
            }
            if let Some(media) = content.media_mut() {
                media.path = record.media_path.map(PathBuf::from).filter(|path| {
                    !path
                        .components()
                        .any(|part| part == std::path::Component::ParentDir)
                        && path.starts_with(self.dirs.media_cache_dir().join("status"))
                        && path.is_file()
                });
                media.state = MediaState::Idle;
            }
            let details: SavedDetails = serde_json::from_str(&record.metadata).unwrap_or_default();
            if details.seen {
                self.status_posts
                    .seen
                    .insert((sender.clone(), record.id.clone()));
            }
            if self.is_me(&sender) {
                if let Some((recipients, mode)) = details.audience {
                    let recipients: Option<Vec<_>> = recipients
                        .iter()
                        .map(|id| Self::jid_of(id).filter(|jid| jid.is_pn() || jid.is_lid()))
                        .collect();
                    if let Some(recipients) = recipients.filter(|recipients| !recipients.is_empty())
                    {
                        let privacy = match mode {
                            crate::status::AudienceMode::Contacts => {
                                whatsapp_rust::StatusPrivacySetting::Contacts
                            }
                            crate::status::AudienceMode::Only => {
                                whatsapp_rust::StatusPrivacySetting::AllowList
                            }
                            crate::status::AudienceMode::Except => {
                                whatsapp_rust::StatusPrivacySetting::DenyList
                            }
                        };
                        self.status_posts
                            .audiences
                            .insert(record.id.clone(), (recipients, privacy));
                    }
                }
                self.status_posts
                    .viewers
                    .insert(record.id.clone(), details.viewers);
            }
            self.status_posts.sequence = self.status_posts.sequence.wrapping_add(1);
            self.status_posts.entries.push(Entry {
                item: Item {
                    id: record.id,
                    sender,
                    name: record.name,
                    timestamp: record.posted_at,
                    background: raw
                        .get_base_message()
                        .extended_text_message
                        .as_option()
                        .and_then(|text| text.background_argb),
                    backdrop: status_backdrop(&raw),
                    content,
                },
                raw: Arc::new(raw),
                token: self.status_posts.sequence,
                file: None,
                download: None,
            });
        }
        self.trim_statuses();
        self.emit_statuses();
        self.emit_status_local_state();
        self.prune_status_media();
    }

    fn emit_status_local_state(&self) {
        self.emit(Event::StatusLocalState {
            seen: self.status_posts.seen.iter().cloned().collect(),
            viewers: self
                .status_posts
                .viewers
                .iter()
                .map(|(id, views)| (id.clone(), views.clone()))
                .collect(),
        });
    }

    pub(super) fn ingest_status_history(&mut self, updates: Vec<wa::WebMessageInfo>) {
        if updates.is_empty() {
            return;
        }
        // Tokens only grow, so the entries this batch added are those past the current one.
        let before = self.status_posts.sequence;
        self.status_posts.importing = true;
        for update in updates {
            let Some(key) = update.key.as_option() else {
                continue;
            };
            if key.remote_jid.as_deref() != Some("status@broadcast") {
                continue;
            }
            let Some(id) = key.id.as_deref().filter(|id| !id.is_empty()) else {
                continue;
            };
            let Some(timestamp) = update
                .message_timestamp
                .and_then(|time| i64::try_from(time).ok())
                .filter(|time| {
                    *time <= crate::util::now()
                        && crate::util::now().saturating_sub(*time) < LIFETIME
                })
            else {
                continue;
            };
            let from_me = key.from_me.unwrap_or(false);
            let sender = if from_me {
                Self::jid_of(&self.me())
            } else {
                key.participant
                    .as_deref()
                    .or(update.participant.as_deref())
                    .and_then(Self::jid_of)
            };
            let Some(sender) = sender.filter(|jid| jid.is_pn() || jid.is_lid()) else {
                continue;
            };
            let Some(raw) = update.message.as_option() else {
                continue;
            };
            if raw.encode_to_vec().len() > crate::status::RAW_LIMIT {
                continue;
            }
            let Some(timestamp) = whatsapp_rust::wacore::time::from_secs(timestamp) else {
                continue;
            };
            let info = MessageInfo {
                id: id.into(),
                source: MessageSource {
                    chat: Jid::status_broadcast(),
                    sender,
                    is_from_me: from_me,
                    ..Default::default()
                },
                timestamp,
                push_name: update.push_name.unwrap_or_default().into(),
                ..Default::default()
            };
            self.ingest_status(&Arc::new(raw.clone()), &info);
        }
        self.status_posts.importing = false;
        let records: Vec<_> = self
            .status_posts
            .entries
            .iter()
            .filter(|entry| entry.token > before)
            .filter_map(|entry| self.saved_status_record(entry))
            .collect();
        if self
            .archive
            .save_statuses(&records, crate::util::now())
            .is_err()
        {
            self.emit(Event::Info(
                "Could not save the synchronized status updates".into(),
            ));
        }
        self.emit_statuses();
    }

    pub(super) fn mark_status_seen(&mut self, sender: String, id: String) {
        if !self.privacy_ready
            || self.blocklist.contains(&sender)
            || !matches!(
                self.archive.chat(&sender),
                Ok(None) | Ok(Some(Chat { locked: false, .. }))
            )
        {
            return;
        }
        if self.status_posts.entries.iter().any(|entry| {
            entry.item.sender == sender
                && entry.item.id == id
                && entry.item.alive(crate::util::now())
        }) {
            self.status_posts.seen.insert((sender.clone(), id.clone()));
            self.save_status_record(&sender, &id);
        }
    }

    fn remembered_status_audiences(&self) -> Result<crate::status::AudienceLists, String> {
        if let Some(json) = self
            .archive
            .meta("status_audiences")
            .map_err(|_| "Could not load the saved status audience")?
        {
            if json.len() > crate::status::METADATA_LIMIT {
                return Err("Saved status audience is too large".into());
            }
            let mut lists: crate::status::AudienceLists = serde_json::from_str(&json)
                .map_err(|_| "Could not read the saved status audience")?;
            for selected in [&mut lists.only, &mut lists.except] {
                *selected = selected.iter().map(|id| self.canonical_str(id)).collect();
            }
            if let Some(current) = &mut lists.current {
                current.selected = current
                    .selected
                    .iter()
                    .map(|id| self.canonical_str(id))
                    .collect();
            }
            return Ok(lists);
        }
        let mut lists = crate::status::AudienceLists::default();
        if let Some(action) = self
            .client
            .as_ref()
            .and_then(|client| client.status().audience())
            && action.modes.is_empty()
            && action.custom_lists.is_empty()
            && action.user_jid.len() <= 20_000
        {
            use wa::sync_action_value::status_privacy_action::StatusDistributionMode as Mode;
            let mode = if action.mode == Some(Mode::CONTACTS.into()) {
                Some(crate::status::AudienceMode::Contacts)
            } else if action.mode == Some(Mode::ALLOW_LIST.into()) {
                Some(crate::status::AudienceMode::Only)
            } else if action.mode == Some(Mode::DENY_LIST.into()) {
                Some(crate::status::AudienceMode::Except)
            } else {
                None
            };
            if let Some(mode) = mode {
                lists.choose(crate::status::Audience {
                    mode,
                    selected: action
                        .user_jid
                        .iter()
                        .map(|id| self.canonical_str(id))
                        .collect(),
                });
            }
        }
        Ok(lists)
    }

    pub(super) fn load_status_audiences(&self) {
        match self.remembered_status_audiences() {
            Ok(lists) => self.emit(Event::StatusAudiences(lists)),
            Err(_) => self.emit(Event::Info(
                "Could not restore the status audience. Choose it again before publishing".into(),
            )),
        }
    }

    pub(super) fn save_status_audience(
        &self,
        audience: crate::status::Audience,
    ) -> Result<(), String> {
        if !self.privacy_ready {
            return Err("Unlock this account before saving its status audience".into());
        }
        if audience.selected.len() > 20_000
            || audience
                .selected
                .iter()
                .any(|id| Self::jid_of(id).is_none_or(|jid| !jid.is_pn() && !jid.is_lid()))
        {
            return Err("Invalid status audience".into());
        }
        let mut lists = self.remembered_status_audiences()?;
        lists.choose(audience);
        let json =
            serde_json::to_string(&lists).map_err(|_| "Could not prepare the status audience")?;
        self.archive
            .set_meta("status_audiences", &json)
            .map_err(|_| "Could not save the status audience".into())
    }

    pub(super) fn pick_status_media(&self, token: u64) {
        let events = self.events.clone();
        let waker = self.waker.clone();
        tokio::task::spawn_blocking(move || {
            let path = rfd::FileDialog::new()
                .set_title("Choose a status photo or video")
                .add_filter("Photos and videos", &["jpg", "jpeg", "png", "webp", "mp4"])
                .pick_file();
            let _ = events.send(Event::StatusPicked { token, path });
            waker.wake();
        });
    }

    pub(super) fn publish_status(
        &mut self,
        draft: crate::status::Draft,
        audience: crate::status::Audience,
    ) {
        if self.status_posts.publication.is_some() {
            return;
        }
        let fail =
            |worker: &Self, text: &str| worker.emit(Event::StatusPublished(Err(text.into())));
        if !self.privacy_ready {
            return fail(self, "Unlock the account before publishing");
        }
        if self.blocklist.ids.is_none() {
            return fail(
                self,
                "Wait for the account's blocked contacts before publishing",
            );
        }
        if draft.text.chars().count() > 700 {
            return fail(self, "A status can contain up to 700 characters");
        }
        if draft.text.trim().is_empty() && draft.media.is_none() {
            return fail(self, "Add text, a photo or a video");
        }
        let recipients = self.status_recipients(&audience);
        if recipients.is_empty() {
            return fail(self, "Choose at least one saved contact for this status");
        }
        let Some(client) = self.connected_client() else {
            return fail(self, "Connect to publish your status");
        };
        let privacy = match audience.mode {
            crate::status::AudienceMode::Contacts => whatsapp_rust::StatusPrivacySetting::Contacts,
            crate::status::AudienceMode::Except => whatsapp_rust::StatusPrivacySetting::DenyList,
            crate::status::AudienceMode::Only => whatsapp_rust::StatusPrivacySetting::AllowList,
        };
        self.status_posts.sequence = self.status_posts.sequence.wrapping_add(1);
        let token = self.status_posts.sequence;
        let root = self.dirs.media_cache_dir().join("status");
        let events = self.wa_sender.clone();
        let task = tokio::spawn(async move {
            let result = tokio::time::timeout(
                Duration::from_secs(120),
                publish(&client, draft, &recipients, privacy, &root),
            )
            .await
            .unwrap_or_else(|_| {
                Err("Status publishing timed out. Check your phone before trying again".into())
            });
            let _ = events.send(RuntimeEvent::StatusPublished {
                token,
                recipients,
                privacy,
                result,
            });
        });
        self.status_posts.publication = Some((token, task.abort_handle()));
    }

    fn status_recipients(&self, audience: &crate::status::Audience) -> Vec<Jid> {
        // An unresolved exclusion could name one of the saved phone contacts.
        if audience.mode == crate::status::AudienceMode::Except
            && audience
                .selected
                .iter()
                .any(|id| Self::jid_of(&self.canonical_str(id)).is_none_or(|jid| jid.is_lid()))
        {
            return Vec::new();
        }
        // Normalize exclusions too: learning a privacy ID must not enlarge the audience.
        let audience = crate::status::Audience {
            mode: audience.mode,
            selected: audience
                .selected
                .iter()
                .map(|id| self.canonical_str(id))
                .collect(),
        };
        // Only saved phone contacts qualify; chat history and push names are not an address book.
        let contacts = self
            .contacts
            .values()
            .filter(|contact| {
                contact
                    .full_name
                    .as_ref()
                    .is_some_and(|name| !name.is_empty())
            })
            .map(|contact| self.canonical_str(&contact.id))
            .filter(|id| !self.is_me(id) && !self.blocklist.contains(id));
        audience
            .recipients(contacts)
            .iter()
            .filter_map(|id| Self::jid_of(id))
            .filter(|jid| jid.is_pn() || jid.is_lid())
            .collect()
    }

    pub(super) fn status_published(
        &mut self,
        token: u64,
        recipients: Vec<Jid>,
        privacy: whatsapp_rust::StatusPrivacySetting,
        result: Result<
            (
                whatsapp_rust::send::SendResult,
                Option<(PathBuf, tempfile::TempDir)>,
            ),
            String,
        >,
    ) {
        if self
            .status_posts
            .publication
            .as_ref()
            .is_none_or(|(current, _)| *current != token)
        {
            return;
        }
        self.status_posts.publication = None;
        match result {
            Err(error) => self.emit(Event::StatusPublished(Err(error))),
            Ok((sent, file)) => {
                let Some(mut content) = classify(&sent.message) else {
                    self.emit(Event::StatusPublished(Ok(())));
                    self.emit(Event::Info(
                        "Status published, but its preview could not be loaded".into(),
                    ));
                    return;
                };
                let (path, directory) =
                    file.map_or((None, None), |(path, dir)| (Some(path), Some(dir)));
                if let Some(media) = content.media_mut() {
                    media.path = path;
                }
                let item = Item {
                    backdrop: status_backdrop(&sent.message),
                    id: sent.message_id.to_string(),
                    sender: self.me(),
                    name: None,
                    timestamp: crate::util::now(),
                    background: sent
                        .message
                        .get_base_message()
                        .extended_text_message
                        .as_option()
                        .and_then(|text| text.background_argb),
                    content,
                };
                self.status_posts
                    .audiences
                    .insert(item.id.clone(), (recipients, privacy));
                self.status_posts
                    .entries
                    .retain(|entry| entry.item.id != item.id || entry.item.sender != item.sender);
                self.status_posts.entries.insert(
                    0,
                    Entry {
                        item,
                        raw: sent.message,
                        token,
                        file: directory,
                        download: None,
                    },
                );
                self.trim_statuses();
                let sender = self.me();
                let id = sent.message_id.to_string();
                self.save_status_record(&sender, &id);
                self.status_posts.audiences.retain(|id, _| {
                    self.status_posts
                        .entries
                        .iter()
                        .any(|entry| entry.item.id == *id)
                });
                self.emit_statuses();
                self.emit(Event::StatusPublished(Ok(())));
            }
        }
    }

    pub(super) fn status_receipt(&mut self, receipt: &wa_events::Receipt) {
        if !matches!(receipt.r#type, ReceiptType::Read | ReceiptType::Played)
            || receipt.source.is_from_me
            || self.is_me(&self.canonical(&receipt.source.sender))
        {
            return;
        }
        let reader = self.canonical(&receipt.source.sender);
        if Self::jid_of(&reader).is_none_or(|jid| !jid.is_pn() && !jid.is_lid()) {
            return;
        }
        if receipt
            .source
            .recipient
            .as_ref()
            .is_some_and(|author| !self.is_me(&self.canonical(author)))
        {
            return;
        }
        for id in &receipt.message_ids {
            if !self.status_posts.entries.iter().any(|entry| {
                entry.item.id == id.as_str()
                    && self.is_me(&entry.item.sender)
                    && entry.item.alive(crate::util::now())
            }) {
                continue;
            }
            if self
                .status_posts
                .audiences
                .get(id.as_str())
                .is_some_and(|(recipients, _)| {
                    !recipients.iter().any(|jid| self.canonical(jid) == reader)
                })
            {
                continue;
            }
            let views = self.status_posts.viewers.entry(id.to_string()).or_default();
            if views.iter().any(|view| view.contact == reader) || views.len() >= 20_000 {
                continue;
            }
            views.push(crate::status::Viewer {
                contact: reader.clone(),
                timestamp: receipt.timestamp.timestamp(),
            });
            let viewers = vec![views.last().expect("a new view was added").clone()];
            self.emit(Event::StatusViews {
                id: id.to_string(),
                viewers,
            });
            self.save_status_record(&self.me(), id.as_str());
        }
    }

    pub(super) fn delete_status(&self, id: String) {
        let refuse = |text: &str| {
            self.emit(Event::StatusDeleted {
                id: id.clone(),
                result: Err(text.into()),
            })
        };
        if !self.privacy_ready
            || !self.status_posts.entries.iter().any(|entry| {
                entry.item.id == id
                    && self.is_me(&entry.item.sender)
                    && entry.item.alive(crate::util::now())
            })
        {
            return refuse("This status is no longer available");
        }
        let Some((recipients, privacy)) = self.status_posts.audiences.get(&id).cloned() else {
            return refuse("The original audience for this status is unavailable on this device");
        };
        let Some(client) = self.connected_client() else {
            return refuse("Connect to delete this status");
        };
        let events = self.wa_sender.clone();
        let session = self.account_session;
        tokio::spawn(async move {
            let result = tokio::time::timeout(Duration::from_secs(45), async {
                let target = whatsapp_rust::MessageId::new(id.clone()).map_err(|_| ())?;
                client
                    .status()
                    .revoke(
                        target,
                        &recipients,
                        whatsapp_rust::StatusSendOptions::default().with_privacy(privacy),
                    )
                    .await
                    .map(|_| ())
                    .map_err(|_| ())
            })
            .await
            .map_err(|_| {
                "Deleting the status timed out. Check your phone before retrying".to_owned()
            })
            .and_then(|result| result.map_err(|_| "Could not delete this status".into()));
            let _ = events.send(RuntimeEvent::StatusDeleted {
                session,
                id,
                result,
            });
        });
    }

    pub(super) fn status_deleted(&mut self, id: String, result: Result<(), String>) {
        if result.is_ok() {
            let me = self.me();
            self.status_posts
                .entries
                .retain(|entry| entry.item.id != id || entry.item.sender != me);
            self.status_posts.audiences.remove(&id);
            self.status_posts.viewers.remove(&id);
            let expires = crate::util::now() + LIFETIME;
            if self.archive.revoke_status(&me, &id, expires).is_err() {
                self.emit(Event::Info(
                    "Could not save the status removal locally".into(),
                ));
            }
            self.status_posts.revoke(me, id.clone(), expires);
            self.emit_statuses();
            self.prune_status_media();
        }
        self.emit(Event::StatusDeleted { id, result });
    }

    pub(super) fn reply_status(
        &mut self,
        sender: String,
        id: String,
        text: String,
        reaction: bool,
    ) {
        let refuse = |worker: &Self, error: &str| {
            worker.emit(Event::StatusReplied {
                sender: sender.clone(),
                id: id.clone(),
                text: text.clone(),
                reaction,
                result: Err(error.into()),
            })
        };
        if !self.privacy_ready
            || self.blocklist.contains(&sender)
            || self.is_me(&sender)
            || !matches!(
                self.archive.chat(&sender),
                Ok(None) | Ok(Some(Chat { locked: false, .. }))
            )
        {
            return refuse(self, "This status cannot receive a reply");
        }
        if text.trim().is_empty() || text.chars().count() > 4096 {
            return refuse(self, "Write a reply of up to 4096 characters");
        }
        if reaction && !reaction_emoji(&text) {
            return refuse(self, "Choose an emoji to react with");
        }
        let Some(entry) = self.status_posts.entries.iter().find(|entry| {
            entry.item.sender == sender
                && entry.item.id == id
                && entry.item.alive(crate::util::now())
        }) else {
            return refuse(self, "This status is no longer available");
        };
        let Some(client) = self.connected_client() else {
            return refuse(self, "Connect to reply to this status");
        };
        if receipts_hidden(&client) {
            return refuse(self, RECEIPTS_OFF);
        }
        let Some(author) = Self::jid_of(&sender) else {
            return refuse(self, "This contact is unavailable");
        };
        let raw = Arc::clone(&entry.raw);
        let prepared = if !reaction {
            let context = whatsapp_rust::wacore::proto_helpers::build_quote_context_with_info(
                id.clone(),
                &author,
                &Jid::status_broadcast(),
                &author,
                &raw,
            );
            let mut message = wa::Message::text_with_context(text.clone(), context);
            self.apply_ephemeral(&sender, &mut message);
            let outgoing = client.generate_message_id().into_string();
            let row = Message {
                id: outgoing.clone(),
                chat: sender.clone(),
                sender: self.me(),
                sender_name: None,
                from_me: true,
                timestamp: crate::util::now(),
                content: Content::text(text.clone()),
                status: Delivery::Pending,
                delivered_at: None,
                read_at: None,
                reactions: Vec::new(),
                history_order: None,
                edited: false,
                mentions: Vec::new(),
                forwarded: false,
                thumbnail: None,
                quoted: Some(crate::model::Quoted {
                    chat: Some("status@broadcast".into()),
                    id: id.clone(),
                    sender: sender.clone(),
                    sender_name: None,
                    summary: "Status update".into(),
                    labelled: Some(false),
                    mentions: Vec::new(),
                }),
            };
            self.store_message(row, Some(message.encode_to_vec()), None);
            Some((message, outgoing))
        } else {
            None
        };
        self.dispatch_status_reply(sender, id, text, prepared, client, author);
    }

    fn dispatch_status_reply(
        &self,
        sender: String,
        id: String,
        text: String,
        prepared: Option<(wa::Message, String)>,
        client: Arc<Client>,
        author: Jid,
    ) {
        let reaction = prepared.is_none();
        let outgoing = prepared.as_ref().map(|(_, outgoing)| outgoing.clone());
        let events = self.wa_sender.clone();
        let commands = self.commands.clone();
        let session = self.account_session;
        tokio::spawn(async move {
            let unsent = || "Could not send this reply. Your text has been kept".to_owned();
            let result = tokio::time::timeout(Duration::from_secs(45), async {
                // A reply tells the author it was seen, as a read receipt would.
                if !receipts_allowed(&client, &author, &commands).await {
                    return Err(RECEIPTS_OFF.to_owned());
                }
                match prepared {
                    None => {
                        let status = Jid::status_broadcast();
                        let message_id =
                            whatsapp_rust::MessageId::new(id.clone()).map_err(|_| unsent())?;
                        let target = whatsapp_rust::MessageRef::new(
                            &status,
                            message_id,
                            Some(&author),
                            false,
                        )
                        .map_err(|_| unsent())?;
                        client
                            .send_reaction(&target, &text)
                            .await
                            .map_err(|_| unsent())
                    }
                    Some((reply, outgoing)) => {
                        let message_id =
                            whatsapp_rust::MessageId::new(outgoing).map_err(|_| unsent())?;
                        let options = SendOptions::default().with_message_id(message_id);
                        client
                            .send(
                                whatsapp_rust::SendRequest::new(&author, reply)
                                    .with_options(options),
                            )
                            .await
                            .map_err(|_| unsent())
                    }
                }
            })
            .await
            .unwrap_or_else(|_| Err("Reply timed out. Check your phone before retrying".into()));
            let _ = events.send(RuntimeEvent::StatusReplySent {
                session,
                sender,
                id,
                text,
                reaction,
                outgoing,
                result,
            });
        });
    }

    pub(super) fn reply_status_sticker(&mut self, sender: String, id: String, path: PathBuf) {
        let refuse = |worker: &Self, error: &str| {
            worker.emit(Event::StatusReplied {
                sender: sender.clone(),
                id: id.clone(),
                text: String::new(),
                reaction: false,
                result: Err(error.into()),
            })
        };
        if !self.privacy_ready
            || self.blocklist.contains(&sender)
            || self.is_me(&sender)
            || !matches!(
                self.archive.chat(&sender),
                Ok(None) | Ok(Some(Chat { locked: false, .. }))
            )
        {
            return refuse(self, "This status cannot receive a reply");
        }
        let Some(entry) = self.status_posts.entries.iter().find(|entry| {
            entry.item.sender == sender
                && entry.item.id == id
                && entry.item.alive(crate::util::now())
        }) else {
            return refuse(self, "This status is no longer available");
        };
        let Some(client) = self.connected_client() else {
            return refuse(self, "Connect to reply to this status");
        };
        if receipts_hidden(&client) {
            return refuse(self, RECEIPTS_OFF);
        }
        let Some(author) = Self::jid_of(&sender) else {
            return refuse(self, "This contact is unavailable");
        };
        let context = whatsapp_rust::wacore::proto_helpers::build_quote_context_with_info(
            id.clone(),
            &author,
            &Jid::status_broadcast(),
            &author,
            &entry.raw,
        );
        let shown = Quoted {
            chat: Some("status@broadcast".into()),
            id: id.clone(),
            sender: sender.clone(),
            sender_name: None,
            summary: "Status update".into(),
            labelled: Some(false),
            mentions: Vec::new(),
        };
        let events = self.wa_sender.clone();
        let session = self.account_session;
        let dir = self.dirs.media_cache_dir();
        let me = self.me();
        tokio::spawn(async move {
            let result = tokio::time::timeout(Duration::from_secs(45), async {
                use tokio::io::AsyncReadExt;
                let file = tokio::fs::File::open(path)
                    .await
                    .map_err(|error| error.to_string())?;
                let mut bytes = Vec::new();
                file.take(MEDIA_LIMIT + 1)
                    .read_to_end(&mut bytes)
                    .await
                    .map_err(|error| error.to_string())?;
                if bytes.is_empty() || bytes.len() as u64 > MEDIA_LIMIT {
                    return Err("The sticker exceeds the media limit".into());
                }
                let mut prepared = prepare_sticker(&client, bytes).await?;
                let shown = attach_quote(&mut prepared.message, Some((context, shown)))?;
                let (mut row, raw) =
                    file_outbound(&client, &sender, &me, &dir, prepared, None, Vec::new()).await?;
                row.quoted = shown;
                Ok((Box::new(row), raw))
            })
            .await
            .unwrap_or_else(|_| {
                Err("Sticker reply timed out. Check your phone before retrying".into())
            });
            let _ = events.send(RuntimeEvent::StatusStickerPrepared {
                session,
                sender,
                id,
                result,
            });
        });
    }

    pub(super) fn status_sticker_ready(
        &mut self,
        sender: String,
        id: String,
        result: Result<(Box<Message>, Vec<u8>), String>,
    ) {
        if !self.privacy_ready
            || self.blocklist.contains(&sender)
            || self.is_me(&sender)
            || !matches!(
                self.archive.chat(&sender),
                Ok(None) | Ok(Some(Chat { locked: false, .. }))
            )
            || !self.status_posts.entries.iter().any(|entry| {
                entry.item.sender == sender
                    && entry.item.id == id
                    && entry.item.alive(crate::util::now())
            })
        {
            self.emit(Event::StatusReplied {
                sender,
                id,
                text: String::new(),
                reaction: false,
                result: Err("This status cannot receive a reply".into()),
            });
            return;
        }
        let prepared = result.and_then(|(row, raw)| {
            let mut message = wa::Message::decode_from_slice(&raw)
                .map_err(|_| "Could not prepare the sticker reply".to_owned())?;
            self.apply_ephemeral(&sender, &mut message);
            let raw = message.encode_to_vec();
            let client = self
                .connected_client()
                .ok_or_else(|| "Connect to reply to this status".to_owned())?;
            let author =
                Self::jid_of(&sender).ok_or_else(|| "This contact is unavailable".to_owned())?;
            Ok((row, raw, message, client, author))
        });
        match prepared {
            Ok((row, raw, message, client, author)) => {
                let outgoing = row.id.clone();
                self.store_message(*row, Some(raw), None);
                self.dispatch_status_reply(
                    sender,
                    id,
                    String::new(),
                    Some((message, outgoing)),
                    client,
                    author,
                );
            }
            Err(error) => self.emit(Event::StatusReplied {
                sender,
                id,
                text: String::new(),
                reaction: false,
                result: Err(error),
            }),
        }
    }

    pub(super) async fn status_reply_sent(
        &mut self,
        sender: String,
        id: String,
        text: String,
        reaction: bool,
        outgoing: Option<String>,
        result: Result<whatsapp_rust::send::SendResult, String>,
    ) {
        let result = result.map(|_| ());
        if let Some(outgoing) = outgoing {
            self.handle_command(Command::Sent {
                chat: sender.clone(),
                id: outgoing,
                error: result.as_ref().err().cloned(),
            })
            .await;
        }
        self.emit(Event::StatusReplied {
            sender,
            id,
            text,
            reaction,
            result,
        });
    }

    pub(super) fn prune_status_media(&self) {
        if self.status_posts.publication.is_some()
            || self
                .status_posts
                .entries
                .iter()
                .any(|entry| entry.download.is_some())
        {
            return;
        }
        let root = self.dirs.media_cache_dir().join("status");
        let keep: HashSet<_> = self
            .status_posts
            .entries
            .iter()
            .filter_map(|entry| {
                entry
                    .item
                    .content
                    .media()
                    .and_then(|media| media.path.as_ref())
                    .and_then(|path| path.parent())
                    .map(Path::to_path_buf)
            })
            .collect();
        if let Ok(dirs) = std::fs::read_dir(root) {
            for dir in dirs.flatten() {
                if dir.file_type().is_ok_and(|kind| kind.is_dir())
                    && !keep.contains(&dir.path())
                    && std::fs::remove_dir_all(dir.path()).is_err()
                {
                    log::warn!("could not discard expired status media");
                }
            }
        }
    }

    pub(super) fn emit_statuses(&self) {
        if self.status_posts.importing {
            return;
        }
        self.emit(Event::StatusDeletable(
            self.status_posts.audiences.keys().cloned().collect(),
        ));
        let now = crate::util::now();
        self.emit(Event::Statuses(
            self.status_posts
                .entries
                .iter()
                .filter(|entry| entry.item.alive(now))
                .map(|entry| entry.item.clone())
                .collect(),
        ));
    }

    pub(super) fn expire_statuses(&mut self) {
        let now = crate::util::now();
        let before = self.status_posts.entries.len();
        self.status_posts
            .entries
            .retain(|entry| entry.item.alive(now));
        self.status_posts.revoked.retain(|_, until| *until > now);
        self.status_posts.audiences.retain(|id, _| {
            self.status_posts
                .entries
                .iter()
                .any(|entry| entry.item.id == *id)
        });
        self.status_posts.seen.retain(|(sender, id)| {
            self.status_posts
                .entries
                .iter()
                .any(|entry| entry.item.sender == *sender && entry.item.id == *id)
        });
        self.status_posts.viewers.retain(|id, _| {
            self.status_posts
                .entries
                .iter()
                .any(|entry| entry.item.id == *id)
        });
        if self.status_posts.entries.len() != before {
            if self.archive.save_statuses(&[], now).is_err() {
                log::warn!("could not expire saved status updates");
            }
            self.emit_statuses();
            self.prune_status_media();
        }
    }

    pub(super) fn ingest_status(&mut self, raw: &Arc<wa::Message>, info: &MessageInfo) {
        self.expire_statuses();
        let sender = self.canonical(&info.source.sender);
        let sender = if info.source.is_from_me || self.is_me(&sender) {
            self.me()
        } else {
            sender
        };
        if sender.is_empty()
            || info.id.is_empty()
            || raw.encode_to_vec().len() > crate::status::RAW_LIMIT
        {
            return;
        }
        let base = raw.get_base_message();
        if let Some(protocol) = base.protocol_message.as_option() {
            if protocol.r#type == Some(wa::message::protocol_message::Type::REVOKE)
                && let Some(id) = protocol.key.as_option().and_then(|key| key.id.as_deref())
            {
                // Remembered so a late delivery cannot restore the update; bounded per
                // author first (see `Posts::revoke`), and the archive keeps it too.
                self.status_posts
                    .revoke(sender.clone(), id.into(), crate::util::now() + LIFETIME);
                self.status_posts
                    .entries
                    .retain(|entry| entry.item.sender != sender || entry.item.id != id);
                if self
                    .archive
                    .revoke_status(&sender, id, crate::util::now() + LIFETIME)
                    .is_err()
                {
                    self.emit(Event::Info(
                        "Could not retain a status deletion locally".into(),
                    ));
                }
                self.emit_statuses();
                self.prune_status_media();
            }
            return;
        }
        let key = (sender.clone(), info.id.to_string());
        if self.status_posts.revoked.contains_key(&key)
            || self
                .status_posts
                .entries
                .iter()
                .any(|entry| entry.item.sender == sender && entry.item.id == info.id)
        {
            return;
        }
        let Some(content) = classify(raw).filter(|content| {
            matches!(
                content,
                Content::Text { .. }
                    | Content::Image { .. }
                    | Content::Video { .. }
                    | Content::Audio { .. }
            )
        }) else {
            return;
        };
        let (sender, id) = key;
        let item = Item {
            backdrop: status_backdrop(raw),
            id: id.clone(),
            sender: sender.clone(),
            name: (!info.push_name.is_empty()).then(|| info.push_name.to_string()),
            timestamp: info.timestamp.timestamp(),
            content,
            background: base
                .extended_text_message
                .as_option()
                .and_then(|text| text.background_argb),
        };
        if !item.alive(crate::util::now()) {
            return;
        }
        self.status_posts.sequence = self.status_posts.sequence.wrapping_add(1);
        self.status_posts.entries.push(Entry {
            item,
            raw: Arc::clone(raw),
            token: self.status_posts.sequence,
            file: None,
            download: None,
        });
        self.trim_statuses();
        self.save_status_record(&sender, &id);
        self.emit_statuses();
    }

    pub(super) fn download_status(&mut self, sender: &str, id: &str) {
        self.expire_statuses();
        if !self.privacy_ready
            || self.blocklist.contains(sender)
            || !matches!(
                self.archive.chat(sender),
                Ok(None) | Ok(Some(Chat { locked: false, .. }))
            )
        {
            return;
        }
        let Some(client) = self.connected_client() else {
            return;
        };
        let Some(entry) = self
            .status_posts
            .entries
            .iter_mut()
            .find(|entry| entry.item.sender == sender && entry.item.id == id)
        else {
            return;
        };
        let Some(media) = entry.item.content.media_mut() else {
            return;
        };
        if media.path.as_ref().is_some_and(|path| !path.is_file()) {
            media.path = None;
            entry.file = None;
        }
        if media.path.is_some() || matches!(media.state, MediaState::Downloading) {
            return;
        }
        media.state = MediaState::Downloading;
        let raw = Arc::clone(&entry.raw);
        let token = entry.token;
        let sender = sender.to_owned();
        let id = id.to_owned();
        let dir = self.dirs.media_cache_dir().join("status");
        let events = self.wa_sender.clone();
        self.emit_statuses();
        let download = tokio::spawn(async move {
            let result = fetch_status(&client, &raw, &dir).await;
            let _ = events.send(RuntimeEvent::StatusMedia {
                sender,
                id,
                token,
                result,
            });
        });
        if let Some(entry) = self
            .status_posts
            .entries
            .iter_mut()
            .find(|entry| entry.token == token)
        {
            entry.download = Some(download.abort_handle());
        }
    }

    pub(super) fn status_media_ready(
        &mut self,
        sender: &str,
        id: &str,
        token: u64,
        result: Result<(PathBuf, tempfile::TempDir), String>,
    ) {
        self.expire_statuses();
        let sender = self.canonical_str(sender);
        let Some(entry) = self.status_posts.entries.iter_mut().find(|entry| {
            entry.token == token && entry.item.sender == sender && entry.item.id == id
        }) else {
            return;
        };
        let Some(media) = entry.item.content.media_mut() else {
            return;
        };
        entry.download = None;
        match result {
            Ok((path, dir)) => {
                media.path = Some(path);
                media.state = MediaState::Idle;
                entry.file = Some(dir);
            }
            Err(error) => media.state = MediaState::Failed(error),
        }
        self.save_status_record(&sender, id);
        self.emit_statuses();
    }
}

/// Replies follow the read-receipt setting: refused while it is off.
const RECEIPTS_OFF: &str = "Turn on read receipts to reply to status updates";

/// Whether the library already knows the account hides read receipts. Unknown is
/// settled before sending by `receipts_allowed`, which fails closed.
fn receipts_hidden(client: &Client) -> bool {
    client
        .persistence_manager()
        .get_device_snapshot()
        .read_receipts_disabled
}

/// A reaction is one short emoji, never words smuggled in as a reaction.
fn reaction_emoji(text: &str) -> bool {
    use unicode_segmentation::UnicodeSegmentation;
    !text.is_empty()
        && text.len() <= 32
        && text.graphemes(true).count() <= 4
        && !text
            .chars()
            .any(|c| c.is_whitespace() || c.is_ascii_alphanumeric())
}

fn status_backdrop(raw: &wa::Message) -> Option<Vec<u8>> {
    let base = raw.get_base_message();
    let thumbnail = base
        .image_message
        .as_option()
        .and_then(|image| image.jpeg_thumbnail.as_deref())
        .or_else(|| {
            base.video_message
                .as_option()
                .and_then(|video| video.jpeg_thumbnail.as_deref())
        })?;
    crate::status::blurred_backdrop(thumbnail)
}

async fn publish(
    client: &Client,
    draft: crate::status::Draft,
    recipients: &[Jid],
    privacy: whatsapp_rust::StatusPrivacySetting,
    root: &Path,
) -> Result<
    (
        whatsapp_rust::send::SendResult,
        Option<(PathBuf, tempfile::TempDir)>,
    ),
    String,
> {
    let options = whatsapp_rust::StatusSendOptions::default().with_privacy(privacy);
    let Some(path) = draft.media else {
        use wa::message::extended_text_message::FontType;
        let font = [
            FontType::SYSTEM,
            FontType::SYSTEM_BOLD,
            FontType::COURIERPRIME_BOLD,
        ][draft.font % 3];
        let sent = client
            .status()
            .send_text(
                draft.text.trim(),
                crate::status::BACKGROUNDS[draft.background % crate::status::BACKGROUNDS.len()],
                font,
                recipients,
                options,
            )
            .await
            .map_err(|_| "Could not publish this status. Your draft has been kept".to_owned())?;
        return Ok((sent, None));
    };
    let path_for_read = path.clone();
    let quality = draft.quality;
    let (bytes, mime) = tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let mut head = [0; 8];
        let video = std::fs::File::open(&path_for_read)
            .and_then(|mut file| file.read_exact(&mut head))
            .is_ok()
            && head[4..8] == *b"ftyp";
        if video {
            // Measured before reading, so a huge video is never loaded only to be refused.
            if std::fs::metadata(&path_for_read)
                .map_err(|_| "Could not open this media file".to_owned())?
                .len()
                > MEDIA_LIMIT
            {
                return Err("Status media must be smaller than 16 MiB".to_owned());
            }
            let bytes = crate::photo::read(&path_for_read)?;
            if bytes.len() as u64 > MEDIA_LIMIT {
                return Err("Status media must be smaller than 16 MiB".to_owned());
            }
            Ok((bytes, "video/mp4"))
        } else {
            let bytes = crate::photo::read(&path_for_read)?;
            Ok((crate::photo::encode(&bytes, quality)?, "image/jpeg"))
        }
    })
    .await
    .map_err(|_| "Could not prepare this media file".to_owned())??;
    if bytes.is_empty() || bytes.len() as u64 > MEDIA_LIMIT {
        return Err("Status media must be smaller than 16 MiB".into());
    }
    let mut prepared = prepare_media(client, bytes, mime, None, false, None)
        .await
        .map_err(|_| "Could not prepare this media for your status".to_owned())?;
    if prepared.bytes.len() as u64 > MEDIA_LIMIT {
        return Err("Status media must be smaller than 16 MiB".into());
    }
    let caption = (!draft.text.trim().is_empty()).then_some(draft.text);
    if let Some(image) = prepared.message.image_message.as_option_mut() {
        image.caption = caption.clone();
    }
    if let Some(video) = prepared.message.video_message.as_option_mut() {
        video.caption = caption;
    }
    tokio::fs::create_dir_all(root)
        .await
        .map_err(|_| "Could not stage your status".to_owned())?;
    let dir = tempfile::Builder::new()
        .prefix("own-")
        .tempdir_in(root)
        .map_err(|_| "Could not stage your status".to_owned())?;
    let target = dir
        .path()
        .join(format!("status.{}", extension_for(&prepared.mime, None)));
    tokio::fs::write(&target, prepared.bytes)
        .await
        .map_err(|_| "Could not stage your status".to_owned())?;
    let sent = client
        .status()
        .send_raw(prepared.message, recipients, options)
        .await
        .map_err(|_| "Could not publish this status. Your draft has been kept".to_owned())?;
    Ok((sent, Some((target, dir))))
}

async fn fetch_status(
    client: &Client,
    raw: &wa::Message,
    root: &Path,
) -> Result<(PathBuf, tempfile::TempDir), String> {
    tokio::fs::create_dir_all(root)
        .await
        .map_err(|_| "Could not create temporary Status storage".to_owned())?;
    let dir = tempfile::Builder::new()
        .prefix("received-")
        .tempdir_in(root)
        .map_err(|_| "Could not create temporary Status storage".to_owned())?;
    let base = raw.get_base_message();
    let (downloadable, mime): (&dyn Downloadable, &str) =
        if let Some(image) = base.image_message.as_option() {
            (image, image.mimetype.as_deref().unwrap_or("image/jpeg"))
        } else if let Some(video) = base.video_message.as_option() {
            (video, video.mimetype.as_deref().unwrap_or("video/mp4"))
        } else if let Some(audio) = base.audio_message.as_option() {
            (audio, audio.mimetype.as_deref().unwrap_or("audio/ogg"))
        } else {
            return Err("This Status type cannot be downloaded".into());
        };
    let extension = extension_for(mime, None);
    let path = dir.path().join(format!("status.{extension}"));
    let path = tokio::time::timeout(
        Duration::from_secs(60),
        download_attachment(client, downloadable, dir.path(), &path, MEDIA_LIMIT),
    )
    .await
    .map_err(|_| "Status download timed out. Try again".to_owned())?
    .map_err(|_| {
        "Could not download this Status (expired, unavailable, or over 16 MiB). Try again"
            .to_owned()
    })?;
    Ok((path, dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history_post(sender: &str, id: &str, own: bool, at: i64) -> wa::WebMessageInfo {
        let mut update = wa::WebMessageInfo::default();
        let mut key = wa::MessageKey::default();
        key.remote_jid = Some("status@broadcast".into());
        key.id = Some(id.into());
        key.from_me = Some(own);
        key.participant = (!own).then(|| sender.into());
        update.key = MessageField::some(key);
        update.message = MessageField::some((*text()).clone());
        update.message_timestamp = Some(at as u64);
        update
    }

    #[test]
    fn phone_status_history_imports_own_and_contact_updates_and_restores_on_restart() {
        let (mut worker, events, mut commands, _runtime) = super::super::receipt_tests::worker();
        let now = crate::util::now();
        let me = "15550009999@s.whatsapp.net";
        worker.me_pn = Some(me.into());
        let mut history = wa::HistorySync::default();
        history.sync_type = wa::history_sync::HistorySyncType::INITIAL_STATUS_V3;
        history.status_v3_messages = vec![
            history_post(me, "own", true, now),
            history_post(AUTHOR, "peer", false, now),
        ];
        // One uncompressed zlib block keeps this synthetic fixture dependency-free.
        let raw = history.encode_to_vec();
        let length = u16::try_from(raw.len()).unwrap();
        let mut compressed = vec![0x78, 0x01, 0x01];
        compressed.extend(length.to_le_bytes());
        compressed.extend((!length).to_le_bytes());
        compressed.extend(&raw);
        let (a, b) = raw.iter().fold((1u32, 0u32), |(a, b), byte| {
            let a = (a + u32::from(*byte)) % 65521;
            (a, (b + a) % 65521)
        });
        compressed.extend(((b << 16) | a).to_be_bytes());
        let parsed = parse_history(&compressed).unwrap();
        assert!(parsed.chats.is_empty());
        assert_eq!(parsed.statuses.len(), 2);
        worker.apply_history(parsed, true);
        worker.mark_status_seen(AUTHOR.into(), "peer".into());
        assert_eq!(worker.status_posts.entries.len(), 2);
        assert_eq!(worker.archive.saved_statuses(now).unwrap().len(), 2);
        assert!(worker.archive.chat("status@broadcast").unwrap().is_none());
        assert!(
            commands.try_recv().is_err(),
            "viewing never requests a network receipt"
        );
        events.try_iter().for_each(drop);
        worker.status_posts = Posts::default();
        worker.restore_statuses();
        assert_eq!(worker.status_posts.entries.len(), 2);
        assert!(
            worker
                .status_posts
                .entries
                .iter()
                .any(|entry| entry.item.sender == me && entry.item.id == "own")
        );
        assert!(
            worker
                .status_posts
                .seen
                .contains(&(AUTHOR.into(), "peer".into()))
        );
        assert!(events.try_iter().any(|event| matches!(event, Event::StatusLocalState {seen,..} if seen.contains(&(AUTHOR.into(),"peer".into())))));
        worker.ingest_status(&revoke("peer"), &info(AUTHOR, "revoke", now));
        worker.status_posts = Posts::default();
        worker.restore_statuses();
        worker.ingest_status_history(vec![history_post(AUTHOR, "peer", false, now)]);
        assert_eq!(
            worker.status_posts.entries.len(),
            1,
            "a deleted update stays deleted after restart and late history"
        );
    }

    #[test]
    fn status_history_rejects_expired_future_non_status_and_unattributed_updates() {
        let (mut worker, _events, _commands, _runtime) = super::super::receipt_tests::worker();
        let now = crate::util::now();
        let mut chat = history_post(AUTHOR, "chat", false, now);
        chat.key.as_option_mut().unwrap().remote_jid = Some(AUTHOR.into());
        let mut anonymous = history_post(AUTHOR, "anonymous", false, now);
        anonymous.key.as_option_mut().unwrap().participant = None;
        worker.ingest_status_history(vec![
            history_post(AUTHOR, "expired", false, now - LIFETIME),
            history_post(AUTHOR, "future", false, now + 3600),
            chat,
            anonymous,
        ]);
        assert!(worker.status_posts.entries.is_empty());
        assert!(worker.archive.saved_statuses(now).unwrap().is_empty());
    }

    #[test]
    fn restoring_own_status_retains_original_audience_views_and_media() {
        let (mut worker, _events, _commands, _runtime) = super::super::receipt_tests::worker();
        let me = "15550009999@s.whatsapp.net";
        let now = crate::util::now();
        worker.me_pn = Some(me.into());
        let mut own = info(me, "own", now);
        own.source.is_from_me = true;
        let mut raw = wa::Message::default();
        raw.image_message = MessageField::some(Default::default());
        worker.ingest_status(&Arc::new(raw), &own);
        let root = worker.dirs.media_cache_dir().join("status");
        std::fs::create_dir_all(&root).unwrap();
        let dir = tempfile::tempdir_in(&root).unwrap();
        let path = dir.path().join("preview.png");
        std::fs::write(&path, b"synthetic fixture").unwrap();
        let token = worker.status_posts.entries[0].token;
        worker.status_media_ready(me, "own", token, Ok((path.clone(), dir)));
        worker.status_posts.audiences.insert(
            "own".into(),
            (
                vec![AUTHOR.parse().unwrap()],
                whatsapp_rust::StatusPrivacySetting::AllowList,
            ),
        );
        worker.status_posts.viewers.insert(
            "own".into(),
            vec![crate::status::Viewer {
                contact: AUTHOR.into(),
                timestamp: now,
            }],
        );
        worker.save_status_record(me, "own");
        worker.status_posts = Posts::default();
        assert!(path.is_file(), "shutdown retains unexpired media");
        worker.restore_statuses();
        assert_eq!(
            worker.status_posts.entries[0]
                .item
                .content
                .media()
                .unwrap()
                .path
                .as_ref(),
            Some(&path)
        );
        assert_eq!(worker.status_posts.audiences["own"].0.len(), 1);
        assert_eq!(worker.status_posts.viewers["own"].len(), 1);
        worker.status_deleted("own".into(), Ok(()));
        worker.status_posts = Posts::default();
        worker.restore_statuses();
        assert!(worker.status_posts.entries.is_empty());
        assert!(!path.exists(), "removed media is cleaned on recovery");
        let _ = std::fs::remove_dir_all(worker.dirs.media_cache_dir());
    }

    #[test]
    fn unresolved_status_exclusions_never_expand_the_audience() {
        let (mut worker, _events, _commands, _runtime) = super::super::receipt_tests::worker();
        worker.contacts.insert(
            AUTHOR.into(),
            Contact {
                id: AUTHOR.into(),
                full_name: Some("A saved contact".into()),
                ..Default::default()
            },
        );
        let audience = crate::status::Audience {
            mode: crate::status::AudienceMode::Except,
            selected: ["123456789@lid".to_owned()].into(),
        };
        assert!(worker.status_recipients(&audience).is_empty());
        worker
            .lid_to_pn
            .insert("123456789@lid".into(), AUTHOR.into());
        assert!(
            worker.status_recipients(&audience).is_empty(),
            "the mapped exclusion still excludes that contact"
        );
    }

    #[test]
    fn status_audiences_round_trip_without_losing_the_other_list() {
        let (mut worker, _events, _commands, _runtime) = super::super::receipt_tests::worker();
        worker.privacy_ready = true;
        for (mode, id) in [
            (crate::status::AudienceMode::Only, AUTHOR),
            (crate::status::AudienceMode::Except, OTHER),
        ] {
            worker
                .save_status_audience(crate::status::Audience {
                    mode,
                    selected: [id.to_owned()].into(),
                })
                .unwrap();
        }
        worker
            .save_status_audience(crate::status::Audience {
                mode: crate::status::AudienceMode::Contacts,
                selected: Default::default(),
            })
            .unwrap();
        let restored = worker.remembered_status_audiences().unwrap();
        assert!(restored.only.contains(AUTHOR));
        assert!(restored.except.contains(OTHER));
        assert_eq!(
            restored.current.unwrap().mode,
            crate::status::AudienceMode::Contacts
        );
        worker.privacy_ready = false;
        assert!(
            worker
                .save_status_audience(crate::status::Audience {
                    mode: crate::status::AudienceMode::Only,
                    selected: Default::default()
                })
                .is_err()
        );
        assert!(
            worker
                .remembered_status_audiences()
                .unwrap()
                .only
                .contains(AUTHOR)
        );
    }

    #[test]
    fn own_status_views_do_not_become_chat_receipts_and_deletion_needs_the_original_audience() {
        let (mut worker, events, mut commands, _runtime) = super::super::receipt_tests::worker();
        worker.privacy_ready = true;
        let me = "15550009999@s.whatsapp.net";
        worker.me_pn = Some(me.into());
        let mut own = info(me, "own-post", crate::util::now());
        own.source.is_from_me = true;
        worker.ingest_status(&text(), &own);
        worker.status_posts.audiences.insert(
            "own-post".into(),
            (
                vec![AUTHOR.parse().unwrap()],
                whatsapp_rust::StatusPrivacySetting::AllowList,
            ),
        );
        let read = wa_events::Receipt::builder()
            .message_ids(vec!["own-post".into()])
            .source(MessageSource {
                chat: Jid::status_broadcast(),
                sender: AUTHOR.parse().unwrap(),
                recipient: Some(me.parse().unwrap()),
                ..Default::default()
            })
            .timestamp(whatsapp_rust::wacore::time::now_utc())
            .r#type(ReceiptType::Read)
            .offline(false)
            .build();
        worker.on_receipt(&read);
        worker.on_receipt(&read);
        let mut outsider = read.clone();
        outsider.source.sender = OTHER.parse().unwrap();
        worker.on_receipt(&outsider);
        assert_eq!(worker.status_posts.viewers["own-post"].len(), 1);
        assert!(
            worker
                .archive
                .message("status@broadcast", "own-post")
                .unwrap()
                .is_none()
        );
        assert!(commands.try_recv().is_err());
        worker.status_posts.audiences.clear();
        worker.delete_status("own-post".into());
        assert!(events.try_iter().any(|event|matches!(event,Event::StatusDeleted {result:Err(error),..} if error.contains("original audience"))));
        worker.status_deleted("own-post".into(), Ok(()));
        assert!(worker.status_posts.entries.is_empty());
        worker.ingest_status(&text(), &own);
        assert!(
            worker.status_posts.entries.is_empty(),
            "a late echo cannot restore a deleted status"
        );
    }

    const AUTHOR: &str = "15550002222@s.whatsapp.net";
    const OTHER: &str = "15550003333@s.whatsapp.net";

    fn info(sender: &str, id: &str, at: i64) -> MessageInfo {
        MessageInfo {
            id: id.into(),
            source: MessageSource {
                chat: "status@broadcast".parse().unwrap(),
                sender: sender.parse().unwrap(),
                ..Default::default()
            },
            timestamp: whatsapp_rust::wacore::time::from_secs(at).unwrap(),
            ..Default::default()
        }
    }

    fn text() -> Arc<wa::Message> {
        let mut message = wa::Message::default();
        message.conversation = Some("An afternoon in the park".into());
        Arc::new(message)
    }

    fn revoke(id: &str) -> Arc<wa::Message> {
        let mut message = wa::Message::default();
        let mut protocol = wa::message::ProtocolMessage::default();
        protocol.r#type = Some(wa::message::protocol_message::Type::REVOKE);
        let mut key = wa::MessageKey::default();
        key.id = Some(id.into());
        key.remote_jid = Some("status@broadcast".into());
        protocol.key = MessageField::some(key);
        message.protocol_message = MessageField::some(protocol);
        Arc::new(message)
    }

    #[test]
    fn status_sticker_replies_recheck_privacy_before_sending() {
        let (mut worker, events, mut commands, _runtime) = super::super::receipt_tests::worker();
        worker.privacy_ready = true;
        worker.ingest(&text(), &info(AUTHOR, "one", crate::util::now()));
        worker.blocklist.ids = Some([AUTHOR.to_owned()].into());
        worker.reply_status_sticker(AUTHOR.into(), "one".into(), "missing-sticker.webp".into());
        worker.status_sticker_ready(
            AUTHOR.into(),
            "one".into(),
            Err("prepared before blocking".into()),
        );
        assert_eq!(events.try_iter().filter(|event| matches!(event, Event::StatusReplied { result: Err(error), .. } if error == "This status cannot receive a reply")).count(), 2);
        assert!(
            commands.try_recv().is_err(),
            "blocked replies never reach the send queue"
        );
        worker.blocklist.ids = Some(Default::default());
        worker.reply_status_sticker(
            AUTHOR.into(),
            "expired".into(),
            "missing-sticker.webp".into(),
        );
        assert!(events.try_iter().any(|event| matches!(event, Event::StatusReplied { result: Err(error), .. } if error == "This status is no longer available")));
    }

    #[test]
    fn received_statuses_are_stored_separately_and_never_request_receipts() {
        let (mut worker, events, mut commands, _runtime) = super::super::receipt_tests::worker();
        worker.ingest(&text(), &info(AUTHOR, "one", crate::util::now()));
        assert_eq!(worker.status_posts.entries.len(), 1);
        assert!(
            worker
                .archive
                .message("status@broadcast", "one")
                .unwrap()
                .is_none()
        );
        assert!(worker.archive.chat("status@broadcast").unwrap().is_none());
        assert!(commands.try_recv().is_err());
        assert!(
            events
                .try_iter()
                .all(|event| !matches!(event, Event::Incoming { .. } | Event::Messages { .. }))
        );
    }

    #[test]
    fn publishing_never_falls_back_to_all_contacts_for_an_empty_custom_audience() {
        let (mut worker, events, _commands, _runtime) = super::super::receipt_tests::worker();
        worker.privacy_ready = true;
        worker.blocklist.ids = Some(Default::default());
        worker.publish_status(
            crate::status::Draft {
                text: "A day outside".into(),
                ..Default::default()
            },
            crate::status::Audience {
                mode: crate::status::AudienceMode::Only,
                selected: Default::default(),
            },
        );
        assert!(worker.status_posts.publication.is_none());
        assert!(worker.status_posts.entries.is_empty());
        assert!(events.try_iter().any(|event| matches!(event, Event::StatusPublished(Err(error)) if error.contains("at least one saved contact"))));
    }

    #[test]
    fn revocation_is_scoped_to_its_author_and_wins_over_late_delivery() {
        let (mut worker, _events, _commands, _runtime) = super::super::receipt_tests::worker();
        let now = crate::util::now();
        worker.ingest(&text(), &info(AUTHOR, "one", now));
        worker.ingest(&text(), &info(OTHER, "one", now));
        worker.ingest(&revoke("one"), &info(AUTHOR, "deletion", now));
        worker.ingest(&text(), &info(AUTHOR, "one", now));
        assert_eq!(worker.status_posts.entries.len(), 1);
        assert_eq!(worker.status_posts.entries[0].item.sender, OTHER);
        worker.ingest(&revoke("late"), &info(AUTHOR, "early-deletion", now));
        worker.ingest(&text(), &info(AUTHOR, "late", now));
        assert_eq!(worker.status_posts.entries.len(), 1);
    }

    #[test]
    fn one_contacts_flood_of_deletions_never_hides_anyone_elses_updates() {
        let (mut worker, _events, _commands, _runtime) = super::super::receipt_tests::worker();
        let now = crate::util::now();
        for index in 0..=crate::status::REVOKE_LIMIT {
            worker.ingest(
                &revoke(&format!("fake-{index}")),
                &info(AUTHOR, &format!("deletion-{index}"), now),
            );
        }
        assert!(worker.status_posts.revoked.len() <= crate::status::REVOKE_PER_SENDER);
        worker.ingest(&text(), &info(OTHER, "real", now));
        assert_eq!(worker.status_posts.entries.len(), 1);
        assert_eq!(worker.status_posts.entries[0].item.sender, OTHER);
        let newest = format!("fake-{}", crate::status::REVOKE_LIMIT);
        worker.ingest(&text(), &info(AUTHOR, &newest, now));
        assert_eq!(
            worker.status_posts.entries.len(),
            1,
            "the flooder's newest deletion is still remembered"
        );
    }

    #[test]
    fn reactions_are_short_emoji_only() {
        assert!(reaction_emoji("\u{1f44d}"));
        assert!(reaction_emoji(
            "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}"
        ));
        for text in [
            "",
            "ok",
            "\u{1f44d} hi",
            "\u{1f44d}\u{1f44d}\u{1f44d}\u{1f44d}\u{1f44d}",
        ] {
            assert!(!reaction_emoji(text), "{text:?}");
        }
    }

    #[test]
    fn expired_future_and_duplicate_updates_do_not_fill_the_session() {
        let (mut worker, _events, _commands, _runtime) = super::super::receipt_tests::worker();
        let now = crate::util::now();
        for at in [now - LIFETIME, now + 3600] {
            worker.ingest(&text(), &info(AUTHOR, "invalid", at));
        }
        assert!(worker.status_posts.entries.is_empty());
        for index in 0..LIMIT + 4 {
            let metadata = info(
                AUTHOR,
                &format!("post-{index}"),
                now - LIMIT as i64 - 10 + index as i64,
            );
            worker.ingest(&text(), &metadata);
            worker.ingest(&text(), &metadata);
        }
        assert_eq!(worker.status_posts.entries.len(), LIMIT);
        assert_eq!(
            worker.status_posts.entries[0].item.id,
            format!("post-{}", LIMIT + 3)
        );
    }

    #[test]
    fn expiration_removes_media_and_a_late_download_cannot_restore_a_deleted_post() {
        let (mut worker, _events, _commands, _runtime) = super::super::receipt_tests::worker();
        let now = crate::util::now();
        worker.ingest(&text(), &info(AUTHOR, "one", now));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("status.jpg");
        std::fs::write(&path, b"synthetic fixture").unwrap();
        worker.status_posts.entries[0].file = Some(dir);
        worker.status_posts.entries[0].item.timestamp = now - LIFETIME;
        worker.expire_statuses();
        assert!(!path.exists());
        assert!(worker.status_posts.entries.is_empty());
        worker.status_media_ready(AUTHOR, "one", 1, Err("late result".into()));
        assert!(worker.status_posts.entries.is_empty());
    }

    #[test]
    fn private_statuses_wait_for_privacy_recovery() {
        let (mut worker, events, _commands, _runtime) = super::super::receipt_tests::worker();
        worker.privacy_ready = false;
        worker.ingest(&text(), &info(AUTHOR, "one", crate::util::now()));
        assert!(
            !events
                .try_iter()
                .any(|event| matches!(event, Event::Statuses(_)))
        );
        worker.reveal_private_content();
        assert!(
            events
                .try_iter()
                .any(|event| matches!(event, Event::Statuses(items) if items.len() == 1))
        );
    }
    #[test]
    fn learning_a_privacy_id_keeps_updates_and_deletions_on_one_author() {
        let (mut worker, _events, _commands, _runtime) = super::super::receipt_tests::worker();
        let now = crate::util::now();
        let lid = "9001@lid";
        worker.ingest(&text(), &info(lid, "one", now));
        worker.ingest(&revoke("late"), &info(lid, "delete-late", now));
        worker.learn_lid("9001", "15550002222");
        assert_eq!(worker.status_posts.entries[0].item.sender, AUTHOR);
        worker.ingest(&text(), &info(AUTHOR, "one", now));
        worker.ingest(&text(), &info(AUTHOR, "late", now));
        assert_eq!(worker.status_posts.entries.len(), 1);
        worker.ingest(&revoke("one"), &info(AUTHOR, "delete-one", now));
        assert!(worker.status_posts.entries.is_empty());
    }

    #[test]
    fn audience_exclusions_survive_a_contact_privacy_id_mapping() {
        let (mut worker, _events, _commands, _runtime) = super::super::receipt_tests::worker();
        for id in [AUTHOR, OTHER] {
            worker.contacts.insert(
                id.into(),
                Contact {
                    id: id.into(),
                    full_name: Some("Saved contact".into()),
                    first_name: None,
                    push_name: None,
                },
            );
        }
        worker.learn_lid("9001", "15550002222");
        let mut audience = crate::status::Audience {
            mode: crate::status::AudienceMode::Except,
            selected: ["9001@lid".into()].into(),
        };
        assert_eq!(
            worker
                .status_recipients(&audience)
                .iter()
                .map(Jid::to_non_ad_string)
                .collect::<Vec<_>>(),
            [OTHER]
        );
        audience.mode = crate::status::AudienceMode::Only;
        assert_eq!(
            worker
                .status_recipients(&audience)
                .iter()
                .map(Jid::to_non_ad_string)
                .collect::<Vec<_>>(),
            [AUTHOR]
        );
    }

    #[test]
    fn a_download_from_before_unlink_cannot_replace_a_new_accounts_post() {
        let (mut worker, _events, _commands, _runtime) = super::super::receipt_tests::worker();
        let now = crate::util::now();
        let mut raw = wa::Message::default();
        raw.image_message = MessageField::some(wa::message::ImageMessage::default());
        let raw = Arc::new(raw);
        worker.ingest(&raw, &info(AUTHOR, "photo", now));
        let old = worker.status_posts.entries[0].token;
        worker.status_posts.clear();
        worker.ingest(&raw, &info(AUTHOR, "photo", now));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old-photo.jpg");
        std::fs::write(&path, b"synthetic old account").unwrap();
        worker.status_media_ready(AUTHOR, "photo", old, Ok((path.clone(), dir)));
        assert!(!path.exists());
        assert!(
            worker.status_posts.entries[0]
                .item
                .content
                .media()
                .unwrap()
                .path
                .is_none()
        );
    }
}
