//! Creating a group from the New chat dialog.
//!
//! `create_group` sends the request through whatsapp-rust's
//! `groups().create_group` on a task of its own. Its answer lists everyone
//! asked for as a member, even those WhatsApp refused to add (the library
//! reads a refused `<participant error="403">` as a member), so the task asks
//! for the new group's metadata once more and compares. `group_created` then
//! files the chat with its name, members and our admin rights the way a
//! metadata answer would, so it opens at once instead of as "Group", sets the
//! picked photo, and tells the window.

use super::*;
use crate::backend::CreatedGroup;
use crate::new_group::CreateFailure;
use whatsapp_rust::{GroupCreateOptions, GroupError, GroupParticipant, GroupParticipantOptions};

impl Worker {
    /// Asks for a picture for a group being created and stages it as the
    /// JPEG a group photo is (a centred square, 640 pixels at most).
    pub(super) fn pick_new_group_picture(&mut self) {
        let dir = self.dirs.media_cache_dir();
        let events = self.events.clone();
        let waker = self.waker.clone();
        tokio::task::spawn_blocking(move || {
            let Some(path) = rfd::FileDialog::new()
                .set_title("Choose a group photo")
                .add_filter("Images", &["jpg", "jpeg", "png", "webp", "gif"])
                .pick_file()
            else {
                return;
            };
            let result = profile_picture_jpeg(&path).and_then(|jpeg| stage_picture(&dir, &jpeg));
            if result.is_err() {
                log::warn!("could not use a picture for a new group");
            }
            let _ = events.send(Event::NewGroupPicture(result));
            waker.wake();
        });
    }

    /// Creates a group of `members` called `subject`, with us as its admin.
    pub(super) fn create_group(
        &mut self,
        request: u64,
        subject: String,
        members: Vec<ChatId>,
        picture: Option<PathBuf>,
    ) {
        let subject = subject.trim().to_owned();
        let mut picked: Vec<ChatId> = Vec::new();
        for member in members {
            let member = self.canonical_id(&member);
            if !self.is_me(&member) && !picked.contains(&member) {
                picked.push(member);
            }
        }
        let refuse = |worker: &mut Self, failure: CreateFailure| {
            log::warn!("could not create a group ({})", failure.kind());
            worker.emit(Event::GroupCreated {
                request,
                result: Err(failure),
            });
        };
        let jids: Vec<Jid> = picked.iter().filter_map(|id| Self::jid_of(id)).collect();
        if jids.is_empty() || jids.len() != picked.len() {
            return refuse(self, CreateFailure::NoMembers);
        }
        if subject.is_empty() {
            return refuse(self, CreateFailure::NoName);
        }
        if whatsapp_rust::GroupSubject::new(subject.clone()).is_err() {
            return refuse(self, CreateFailure::NameTooLong);
        }
        let Some(client) = self.client.clone() else {
            return refuse(self, CreateFailure::Offline);
        };
        log::info!("creating a group with {} members", picked.len());
        let me: Vec<String> = [self.me_pn.clone(), self.me_lid.clone()]
            .into_iter()
            .flatten()
            .collect();
        let lids = self.lid_to_pn.clone();
        let commands = self.commands.clone();
        let waker = self.waker.clone();
        tokio::spawn(async move {
            let options = GroupCreateOptions::new(subject.clone())
                .with_participants(jids.into_iter().map(GroupParticipantOptions::new).collect());
            let result = match client.groups().create_group(options).await {
                Ok(created) => {
                    let jid = created.metadata.id.clone();
                    // The create answer counts refused people as members.
                    let (metadata, confirmed) = match client.groups().fetch_metadata(&jid).await {
                        Ok(metadata) => (metadata, true),
                        Err(_) => {
                            log::warn!("could not confirm a new group's members");
                            (created.metadata, false)
                        }
                    };
                    let (participants, admin) = group_members(&metadata.participants, &me, &lids);
                    let mut settings = super::group_admin::settings_of(&metadata, &lids);
                    if !confirmed {
                        // The creator is an admin whatever the answer says.
                        settings.admins.extend(me.iter().cloned());
                    }
                    let unadded = if confirmed {
                        crate::new_group::unadded(&picked, &participants)
                    } else {
                        Vec::new()
                    };
                    Ok(CreatedGroup {
                        chat: canonical_jid(&metadata.id, &lids),
                        subject: metadata
                            .subject
                            .filter(|name| !name.trim().is_empty())
                            .unwrap_or(subject),
                        participants,
                        // The creator is the group's super admin, even when
                        // the answer leaves our own entry out.
                        admin: admin || !confirmed,
                        info_locked: metadata.is_locked,
                        read_only: metadata.is_announcement && !admin,
                        settings,
                        created_at: metadata
                            .creation_time
                            .and_then(|at| i64::try_from(at).ok())
                            .unwrap_or_else(crate::util::now),
                        unadded,
                    })
                }
                Err(error) => Err(create_failure(&error)),
            };
            let _ = commands.send(Command::GroupCreated {
                request,
                result,
                picture,
            });
            waker.wake();
        });
    }

    /// Files a group WhatsApp created, sets its photo, and tells the window.
    pub(super) fn group_created(
        &mut self,
        request: u64,
        result: Result<CreatedGroup, CreateFailure>,
        picture: Option<PathBuf>,
    ) {
        let group = match result {
            Ok(group) => group,
            Err(failure) => {
                log::warn!("could not create a group ({})", failure.kind());
                self.emit(Event::GroupCreated {
                    request,
                    result: Err(failure),
                });
                return;
            }
        };
        let chat = group.chat.clone();
        log::info!(
            "created a group; {} of the people picked were not added",
            group.unadded.len()
        );
        // As a metadata answer would file it: named, with its members and
        // our rights known, and dated by its creation so it sorts first.
        if self.archive.ensure_chat(&chat, &group.subject).is_err() {
            log::warn!("could not file a new group");
        }
        let _ = self.archive.set_group_info(
            &chat,
            Some(&group.subject),
            &group.participants,
            group.read_only,
        );
        let _ = self
            .archive
            .set_group_rights(&chat, group.info_locked, group.admin);
        let _ = self.archive.set_group_settings(&chat, &group.settings);
        let _ = self.archive.set_group_description(&chat, "");
        let _ = self.archive.touch_activity(&chat, group.created_at);
        self.emit_chat(&chat);
        if let Some(path) = picture {
            match std::fs::read(&path) {
                Ok(jpeg) => self.set_group_picture(chat.clone(), Some(jpeg)),
                Err(_) => log::warn!("could not read a new group's photo"),
            }
        }
        self.emit(Event::GroupCreated {
            request,
            result: Ok(crate::new_group::Created {
                chat,
                unadded: group.unadded,
            }),
        });
    }

    /// A member id as the archive files it.
    pub(super) fn canonical_id(&self, id: &str) -> ChatId {
        match Self::jid_of(id) {
            Some(jid) => canonical_jid(&jid, &self.lid_to_pn),
            None => id.to_owned(),
        }
    }
}

/// A JID under its canonical chat id: a privacy id under its phone number
/// once the mapping is known.
pub(super) fn canonical_jid(jid: &Jid, lids: &HashMap<String, String>) -> String {
    if jid.is_lid()
        && let Some(pn) = lids.get(jid.user_base())
    {
        return format!("{pn}@s.whatsapp.net");
    }
    jid.to_non_ad_string()
}

/// A group's members under canonical chat ids, and whether one of `me`
/// (our phone and privacy ids) is an admin.
pub(super) fn group_members(
    participants: &[GroupParticipant],
    me: &[String],
    lids: &HashMap<String, String>,
) -> (Vec<String>, bool) {
    let mut members = Vec::new();
    let mut admin = false;
    for participant in participants {
        let id = participant
            .phone_number
            .as_ref()
            .map(|jid| canonical_jid(jid, lids))
            .unwrap_or_else(|| canonical_jid(&participant.jid, lids));
        let mine = me.contains(&id)
            || participant
                .lid
                .as_ref()
                .is_some_and(|lid| me.contains(&lid.to_non_ad_string()))
            || me.contains(&participant.jid.to_non_ad_string());
        if mine && participant.is_admin() {
            admin = true;
        }
        members.push(id);
    }
    (members, admin)
}

/// What a failed `create_group` comes to, in the window's terms.
fn create_failure(error: &GroupError) -> CreateFailure {
    use whatsapp_rust::IqError;
    match error {
        GroupError::Iq(IqError::Timeout) => CreateFailure::Timeout,
        GroupError::Iq(
            IqError::NotConnected
            | IqError::Disconnected(_)
            | IqError::Socket(_)
            | IqError::ClientState(_)
            | IqError::InternalChannelClosed,
        ) => CreateFailure::Offline,
        GroupError::Iq(IqError::ServerError { code, .. }) => CreateFailure::from_code(*code),
        GroupError::InvalidRequest(_) => CreateFailure::NotAccepted,
        _ => CreateFailure::Failed,
    }
}

/// Writes a staged photo under its own bytes' name, so a second pick never
/// shows the first one's cached texture.
fn stage_picture(dir: &Path, jpeg: &[u8]) -> Result<PathBuf, String> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    jpeg.hash(&mut hasher);
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let path = dir.join(format!("new-group-{:016x}.jpg", hasher.finish()));
    std::fs::write(&path, jpeg).map_err(|error| error.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::new_group::Created;

    fn worker() -> (Worker, std::sync::mpsc::Receiver<Event>) {
        let (worker, events, _commands, _wa) = super::super::receipt_tests::worker();
        (worker, events)
    }

    fn answers(
        events: &std::sync::mpsc::Receiver<Event>,
    ) -> Vec<(u64, Result<Created, CreateFailure>)> {
        events
            .try_iter()
            .filter_map(|event| match event {
                Event::GroupCreated { request, result } => Some((request, result)),
                _ => None,
            })
            .collect()
    }

    fn create(subject: &str, members: &[&str]) -> Command {
        Command::CreateGroup {
            request: 7,
            subject: subject.into(),
            members: members.iter().map(|id| (*id).to_owned()).collect(),
            picture: None,
        }
    }

    /// Nothing reaches WhatsApp without someone to add and a name it takes,
    /// and without a link the window hears so at once.
    #[tokio::test]
    async fn a_group_needs_members_a_name_and_the_link() {
        let (mut worker, events) = worker();
        worker.me_pn = Some("15550000000@s.whatsapp.net".into());
        let ada = "15550000001@s.whatsapp.net";
        for (command, failure) in [
            (create("Club", &[]), CreateFailure::NoMembers),
            // We are the creator, never a member we add.
            (
                create("Club", &["15550000000@s.whatsapp.net"]),
                CreateFailure::NoMembers,
            ),
            (create("   ", &[ada]), CreateFailure::NoName),
            (
                create(&"x".repeat(crate::model::GROUP_NAME_LIMIT + 1), &[ada]),
                CreateFailure::NameTooLong,
            ),
            (create(" Club ", &[ada, ada]), CreateFailure::Offline),
        ] {
            worker.handle_command(command).await;
            assert_eq!(answers(&events), [(7, Err(failure))]);
        }
    }

    /// The answer files the group named, with its members and our rights,
    /// dated by its creation, and the window hears who was left out.
    #[tokio::test]
    async fn a_created_group_is_filed_ready_to_open() {
        let (mut worker, events) = worker();
        let chat = "120363000000000001@g.us";
        let me = "15550000000@s.whatsapp.net";
        let ada = "15550000001@s.whatsapp.net";
        let grace = "15550000002@s.whatsapp.net";
        worker
            .handle_command(Command::GroupCreated {
                request: 3,
                result: Ok(CreatedGroup {
                    chat: chat.into(),
                    subject: "Book club".into(),
                    participants: vec![me.into(), ada.into()],
                    admin: true,
                    info_locked: false,
                    read_only: false,
                    settings: Default::default(),
                    created_at: 1_700_000_000,
                    unadded: vec![grace.into()],
                }),
                picture: None,
            })
            .await;
        let row = worker.archive.chat(chat).unwrap().unwrap();
        assert_eq!(row.name, "Book club");
        assert!(row.group_subject_known);
        assert_eq!(row.participants, [me, ada]);
        assert!(row.can_edit_info(), "we are its admin");
        assert_eq!(row.last_activity, 1_700_000_000);
        let filed: Vec<_> = events.try_iter().collect();
        let chat_at = filed
            .iter()
            .position(|event| matches!(event, Event::ChatUpdated(row) if row.id == chat));
        let answer_at = filed.iter().position(|event| {
            matches!(
                event,
                Event::GroupCreated { request: 3, result: Ok(Created { chat: id, unadded }) }
                    if id == chat && unadded == &[grace.to_owned()]
            )
        });
        assert!(
            chat_at.is_some() && chat_at < answer_at,
            "the chat reaches the window before the answer that opens it: {filed:?}"
        );
        // The metadata is known, so nothing asks for it again.
        worker.request_group_info(chat, false);
        assert!(worker.group_info_queue.is_empty());
    }

    #[tokio::test]
    async fn a_refusal_reaches_the_window_and_files_nothing() {
        let (mut worker, events) = worker();
        worker
            .handle_command(Command::GroupCreated {
                request: 4,
                result: Err(CreateFailure::RateLimited),
                picture: None,
            })
            .await;
        assert_eq!(answers(&events), [(4, Err(CreateFailure::RateLimited))]);
        assert!(worker.archive.chats().unwrap().is_empty());
    }

    #[test]
    fn library_errors_read_as_plain_reasons() {
        use whatsapp_rust::IqError;
        assert_eq!(
            create_failure(&GroupError::Iq(IqError::Timeout)),
            CreateFailure::Timeout
        );
        assert_eq!(
            create_failure(&GroupError::Iq(IqError::NotConnected)),
            CreateFailure::Offline
        );
        assert_eq!(
            create_failure(&GroupError::InvalidRequest("missing mapping".into())),
            CreateFailure::NotAccepted
        );
        assert_eq!(
            create_failure(&GroupError::Internal(anyhow::anyhow!("store"))),
            CreateFailure::Failed
        );
    }

    /// Members are filed under their phone numbers, and we are the admin
    /// whether the answer names us by number or by privacy id.
    #[test]
    fn members_are_canonical_and_our_admin_role_is_found() {
        use whatsapp_rust::ParticipantType;
        let lids = HashMap::from([("900".to_owned(), "15550000003".to_owned())]);
        let mut us = GroupParticipant::new("100@lid".parse().unwrap());
        us.participant_type = ParticipantType::SuperAdmin;
        let mut ada = GroupParticipant::new("200@lid".parse().unwrap());
        ada.phone_number = Some("15550000001@s.whatsapp.net".parse().unwrap());
        let mapped = GroupParticipant::new("900@lid".parse().unwrap());
        let (members, admin) = group_members(&[us, ada, mapped], &["100@lid".to_owned()], &lids);
        assert_eq!(
            members,
            [
                "100@lid",
                "15550000001@s.whatsapp.net",
                "15550000003@s.whatsapp.net"
            ]
        );
        assert!(admin);
    }

    #[test]
    fn a_staged_photo_is_named_by_its_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let first = stage_picture(dir.path(), b"one").unwrap();
        let again = stage_picture(dir.path(), b"one").unwrap();
        let second = stage_picture(dir.path(), b"two").unwrap();
        assert_eq!(first, again);
        assert_ne!(first, second);
        assert_eq!(std::fs::read(second).unwrap(), b"two");
    }
}
