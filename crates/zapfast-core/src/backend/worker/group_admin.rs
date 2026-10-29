//! Running a group from its info panel: the invite link, adding people, the
//! group's permissions, and making members admins or removing them.
//!
//! Each request goes to whatsapp-rust's `groups()` on a task of its own and
//! comes back as `Command::GroupManaged`; `group_managed` files what WhatsApp
//! accepted, never before, and bumps `settings_generation` so metadata asked
//! for earlier cannot undo it. A refusal re-asks the metadata, so the panel
//! stops offering what we may no longer do. The invite link is a secret:
//! it is never logged and never stored.

use super::*;
use crate::backend::{GroupAnswer, MemberAnswer};
use crate::group_admin::{Added, Failure, InviteLink, MemberChange, Permission, Refusal};
use crate::model::GroupSettings;
use whatsapp_rust::{
    GroupError, GroupMetadata, MemberAddMode, MemberLinkMode, MembershipApprovalMode,
    ParticipantChangeResponse,
};

impl Worker {
    /// The archived group, when it is one we are in.
    fn managed_group(&self, chat: &str) -> Option<Chat> {
        self.archive
            .chat(chat)
            .ok()
            .flatten()
            .filter(|row| row.is_group() && !row.left)
    }

    /// The client and the group's JID, or why the request cannot go out.
    fn group_link(&self, chat: &str) -> Result<(Arc<Client>, Jid), Failure> {
        let jid = Self::jid_of(chat).ok_or(Failure::Failed)?;
        let client = self.client.clone().ok_or(Failure::Offline)?;
        Ok((client, jid))
    }

    /// Asks WhatsApp for the group's invite link, or revokes it for a new
    /// one when `reset` (admins only).
    pub(super) fn group_invite_link(&mut self, chat: ChatId, reset: bool) {
        let allowed = self.managed_group(&chat).is_some_and(|row| {
            if reset {
                row.can_manage_group()
            } else {
                row.can_share_invite_link()
            }
        });
        let link = if allowed {
            self.group_link(&chat)
        } else {
            Err(Failure::Refused)
        };
        let (client, jid) = match link {
            Ok(link) => link,
            Err(failure) => {
                return self.group_managed(
                    chat,
                    GroupAnswer::Link {
                        reset,
                        result: Err(failure),
                    },
                );
            }
        };
        let commands = self.commands.clone();
        let waker = self.waker.clone();
        tokio::spawn(async move {
            let result = match client.groups().get_invite_link(jid, reset).await {
                // Only WhatsApp's own links are taken; nothing else is shown.
                Ok(link) => InviteLink::new(link).ok_or(Failure::Failed),
                Err(error) => Err(failure_of(&error)),
            };
            let _ = commands.send(Command::GroupManaged {
                chat,
                answer: GroupAnswer::Link { reset, result },
            });
            waker.wake();
        });
    }

    /// Adds `members` to the group. Ourselves and anyone already in it are
    /// left out.
    pub(super) fn add_group_members(&mut self, chat: ChatId, members: Vec<ChatId>) {
        let Some(row) = self.managed_group(&chat).filter(Chat::can_add_members) else {
            return self.group_managed(chat, GroupAnswer::Added(Err(Failure::Refused)));
        };
        let mut picked: Vec<ChatId> = Vec::new();
        for member in members {
            let member = self.canonical_id(&member);
            if !self.is_me(&member)
                && !row.participants.contains(&member)
                && !picked.contains(&member)
            {
                picked.push(member);
            }
        }
        let jids: Vec<Jid> = picked.iter().filter_map(|id| Self::jid_of(id)).collect();
        if jids.is_empty() {
            return self.group_managed(chat, GroupAnswer::Added(Ok(Vec::new())));
        }
        let (client, jid) = match self.group_link(&chat) {
            Ok(link) => link,
            Err(failure) => return self.group_managed(chat, GroupAnswer::Added(Err(failure))),
        };
        log::info!("adding {} people to a group", jids.len());
        self.emit(Event::GroupSaving {
            chat: chat.clone(),
            saving: true,
        });
        let lids = self.lid_to_pn.clone();
        let commands = self.commands.clone();
        let waker = self.waker.clone();
        tokio::spawn(async move {
            let result = client
                .groups()
                .add_participants(jid, &jids)
                .await
                .map(|answers| member_answers(&answers, &lids))
                .map_err(|error| failure_of(&error));
            let _ = commands.send(Command::GroupManaged {
                chat,
                answer: GroupAnswer::Added(result),
            });
            waker.wake();
        });
    }

    /// Changes one of the group's permissions, for its admins.
    pub(super) fn set_group_permission(
        &mut self,
        chat: ChatId,
        permission: Permission,
        members: bool,
    ) {
        let allowed = self
            .managed_group(&chat)
            .is_some_and(|row| row.can_manage_group());
        let link = if allowed {
            self.group_link(&chat)
        } else {
            Err(Failure::Refused)
        };
        let (client, jid) = match link {
            Ok(link) => link,
            Err(failure) => {
                return self.group_managed(
                    chat,
                    GroupAnswer::Permission {
                        permission,
                        members,
                        result: Err(failure),
                    },
                );
            }
        };
        log::info!("changing a group permission ({})", permission.kind());
        self.emit(Event::GroupSaving {
            chat: chat.clone(),
            saving: true,
        });
        let commands = self.commands.clone();
        let waker = self.waker.clone();
        tokio::spawn(async move {
            let groups = client.groups();
            let result = match permission {
                Permission::EditInfo => groups.set_locked(jid, !members).await,
                Permission::SendMessages => groups.set_announce(jid, !members).await,
                Permission::AddMembers => groups.set_member_add_mode(jid, add_mode(members)).await,
                Permission::ApproveMembers => {
                    groups
                        .set_membership_approval(jid, approval_mode(members))
                        .await
                }
            }
            .map_err(|error| failure_of(&error));
            let _ = commands.send(Command::GroupManaged {
                chat,
                answer: GroupAnswer::Permission {
                    permission,
                    members,
                    result,
                },
            });
            waker.wake();
        });
    }

    /// Makes a member an admin, dismisses one, or removes them.
    pub(super) fn change_group_member(
        &mut self,
        chat: ChatId,
        member: ChatId,
        change: MemberChange,
    ) {
        let member = self.canonical_id(&member);
        let allowed = self.managed_group(&chat).is_some_and(|row| {
            row.can_manage_group()
                && !self.is_me(&member)
                && !row.is_group_creator(&member)
                && row.participants.contains(&member)
        });
        let link = if allowed {
            self.group_link(&chat)
        } else {
            Err(Failure::Refused)
        };
        let (client, jid, target) = match link.and_then(|(client, jid)| {
            Self::jid_of(&member)
                .map(|target| (client, jid, target))
                .ok_or(Failure::Failed)
        }) {
            Ok(link) => link,
            Err(failure) => {
                return self.group_managed(
                    chat,
                    GroupAnswer::Member {
                        member,
                        change,
                        result: Err(failure),
                    },
                );
            }
        };
        log::info!("changing a group member ({})", change.kind());
        self.emit(Event::GroupSaving {
            chat: chat.clone(),
            saving: true,
        });
        let lids = self.lid_to_pn.clone();
        let commands = self.commands.clone();
        let waker = self.waker.clone();
        tokio::spawn(async move {
            let groups = client.groups();
            let targets = [target];
            let result = match change {
                MemberChange::Promote => groups.promote_participants(jid, &targets).await,
                MemberChange::Demote => groups.demote_participants(jid, &targets).await,
                MemberChange::Remove => groups.remove_participants(jid, &targets).await,
            }
            .map(|answers| member_answers(&answers, &lids))
            .map_err(|error| failure_of(&error));
            let _ = commands.send(Command::GroupManaged {
                chat,
                answer: GroupAnswer::Member {
                    member,
                    change,
                    result,
                },
            });
            waker.wake();
        });
    }

    /// Files what WhatsApp accepted and tells the window.
    pub(super) fn group_managed(&mut self, chat: ChatId, answer: GroupAnswer) {
        if !matches!(answer, GroupAnswer::Link { .. }) {
            self.emit(Event::GroupSaving {
                chat: chat.clone(),
                saving: false,
            });
        }
        let refused = match &answer {
            GroupAnswer::Link { result, .. } => result.as_ref().err().copied(),
            GroupAnswer::Added(result) => result.as_ref().err().copied(),
            GroupAnswer::Permission { result, .. } => result.as_ref().err().copied(),
            GroupAnswer::Member { result, .. } => result.as_ref().err().copied(),
        };
        if let Some(failure) = refused {
            log::warn!("a group change did not happen ({})", failure.kind());
            if failure == Failure::Refused {
                // Our rights changed without our knowing: learn them, so the
                // panel stops offering what WhatsApp refuses.
                self.request_group_info(&chat, true);
            }
        }
        match answer {
            GroupAnswer::Link { reset, result } => {
                self.emit(Event::GroupInviteLink {
                    chat,
                    reset,
                    result,
                });
            }
            GroupAnswer::Added(result) => {
                let result = result.map(|answers| self.members_added(&chat, answers));
                self.emit(Event::GroupMembersAdded { chat, result });
            }
            GroupAnswer::Permission {
                permission,
                members,
                result,
            } => {
                if result.is_ok() {
                    self.permission_set(&chat, permission, members);
                }
                self.emit(Event::GroupPermissionSaved {
                    chat,
                    permission,
                    result,
                });
            }
            GroupAnswer::Member {
                member,
                change,
                result,
            } => {
                let answered = result.is_ok();
                let result = result.and_then(|answers| {
                    // The answer names the one person asked about.
                    match answers.iter().find_map(|answer| answer.error.as_deref()) {
                        Some(code) => Err(code
                            .parse()
                            .map(Failure::from_code)
                            .unwrap_or(Failure::Failed)),
                        None => Ok(()),
                    }
                });
                match result {
                    Ok(()) => self.member_changed(&chat, &member, change),
                    // A refusal for the person, in an answer that came back.
                    Err(failure) if answered => {
                        log::warn!("a member change was refused ({})", failure.kind());
                        if failure == Failure::Refused {
                            self.request_group_info(&chat, true);
                        }
                    }
                    Err(_) => {}
                }
                self.emit(Event::GroupMemberChanged {
                    chat,
                    member,
                    change,
                    result,
                });
            }
        }
    }

    /// Files the people WhatsApp added and says who it would not add.
    fn members_added(&mut self, chat: &str, answers: Vec<MemberAnswer>) -> Added {
        let mut added = Vec::new();
        let mut refused = Vec::new();
        for answer in answers {
            match answer.error.as_deref() {
                // 409: already in the group, which is what was asked.
                None | Some("409") => added.push(answer.id),
                Some(code) => refused.push((answer.id, Refusal::from_code(code))),
            }
        }
        log::info!(
            "added {} people to a group; {} were refused",
            added.len(),
            refused.len()
        );
        if !added.is_empty()
            && let Some(row) = self.managed_group(chat)
        {
            let mut participants = row.participants;
            for id in &added {
                if !participants.contains(id) {
                    participants.push(id.clone());
                }
            }
            self.bump_settings(chat);
            let _ = self.archive.set_participants(chat, &participants);
            self.emit_chat(chat);
        }
        Added {
            chat: chat.to_owned(),
            added,
            refused,
        }
    }

    /// Files a permission WhatsApp accepted.
    fn permission_set(&mut self, chat: &str, permission: Permission, members: bool) {
        let Some(row) = self.managed_group(chat) else {
            return;
        };
        self.bump_settings(chat);
        if permission == Permission::EditInfo {
            let _ = self.archive.set_info_locked(chat, !members);
        } else {
            let mut settings = row.group_settings.unwrap_or_default();
            match permission {
                Permission::SendMessages => settings.announce = !members,
                Permission::AddMembers => settings.members_add = members,
                Permission::ApproveMembers => settings.approval = members,
                Permission::EditInfo => {}
            }
            let _ = self.archive.set_group_settings(chat, &settings);
        }
        self.emit_chat(chat);
    }

    /// Files a member change WhatsApp accepted.
    fn member_changed(&mut self, chat: &str, member: &str, change: MemberChange) {
        let Some(row) = self.managed_group(chat) else {
            return;
        };
        self.bump_settings(chat);
        let mut settings = row.group_settings.unwrap_or_default();
        match change {
            MemberChange::Promote => {
                if !settings.admins.iter().any(|id| id == member) {
                    settings.admins.push(member.to_owned());
                }
            }
            MemberChange::Demote => settings.admins.retain(|id| id != member),
            MemberChange::Remove => {
                settings.admins.retain(|id| id != member);
                let participants: Vec<String> = row
                    .participants
                    .into_iter()
                    .filter(|id| id != member)
                    .collect();
                let _ = self.archive.set_participants(chat, &participants);
            }
        }
        let _ = self.archive.set_group_settings(chat, &settings);
        self.emit_chat(chat);
    }

    fn bump_settings(&mut self, chat: &str) {
        *self.settings_generation.entry(chat.to_owned()).or_default() += 1;
    }
}

/// A group's permissions and admins from its metadata, members under
/// canonical chat ids. Only an explicit "all members" lets members add
/// people or share the link: WhatsApp's default is admins only.
pub(super) fn settings_of(
    metadata: &GroupMetadata,
    lids: &HashMap<String, String>,
) -> GroupSettings {
    let mut settings = GroupSettings {
        announce: metadata.is_announcement,
        members_add: metadata.member_add_mode == Some(MemberAddMode::AllMemberAdd),
        approval: metadata.membership_approval,
        members_share_link: metadata.member_link_mode == Some(MemberLinkMode::AllMemberLink),
        ..GroupSettings::default()
    };
    for participant in &metadata.participants {
        if !participant.is_admin() {
            continue;
        }
        let id = participant
            .phone_number
            .as_ref()
            .map(|jid| new_group::canonical_jid(jid, lids))
            .unwrap_or_else(|| new_group::canonical_jid(&participant.jid, lids));
        if participant.is_super_admin() {
            settings.super_admins.push(id.clone());
        }
        settings.admins.push(id);
    }
    settings
}

/// WhatsApp's per-person answers, under canonical chat ids.
fn member_answers(
    answers: &[ParticipantChangeResponse],
    lids: &HashMap<String, String>,
) -> Vec<MemberAnswer> {
    answers
        .iter()
        .map(|answer| {
            member_answer(
                &answer.jid,
                answer.phone_number.as_ref(),
                answer.error.clone(),
                lids,
            )
        })
        .collect()
}

/// One person's answer, under their phone number when it is known.
fn member_answer(
    jid: &Jid,
    phone: Option<&Jid>,
    error: Option<String>,
    lids: &HashMap<String, String>,
) -> MemberAnswer {
    MemberAnswer {
        id: phone
            .map(|jid| new_group::canonical_jid(jid, lids))
            .unwrap_or_else(|| new_group::canonical_jid(jid, lids)),
        error,
    }
}

fn add_mode(members: bool) -> MemberAddMode {
    if members {
        MemberAddMode::AllMemberAdd
    } else {
        MemberAddMode::AdminAdd
    }
}

fn approval_mode(on: bool) -> MembershipApprovalMode {
    if on {
        MembershipApprovalMode::On
    } else {
        MembershipApprovalMode::Off
    }
}

/// What a failed group request comes to, in the window's terms.
fn failure_of(error: &GroupError) -> Failure {
    use whatsapp_rust::IqError;
    match error {
        GroupError::Iq(IqError::Timeout) => Failure::Timeout,
        GroupError::Iq(
            IqError::NotConnected
            | IqError::Disconnected(_)
            | IqError::Socket(_)
            | IqError::ClientState(_)
            | IqError::InternalChannelClosed,
        ) => Failure::Offline,
        GroupError::Iq(IqError::ServerError { code, .. }) => Failure::from_code(*code),
        _ => Failure::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use whatsapp_rust::{GroupParticipant, ParticipantType};

    const GROUP: &str = "120363000000000001@g.us";
    const ME: &str = "15550000000@s.whatsapp.net";
    const ADA: &str = "15550000001@s.whatsapp.net";
    const GRACE: &str = "15550000002@s.whatsapp.net";

    /// A worker with a group we are in, as an admin or not.
    fn group_worker(admin: bool) -> (Worker, std::sync::mpsc::Receiver<Event>) {
        let (mut worker, events, _commands, _wa) = super::super::receipt_tests::worker();
        worker.me_pn = Some(ME.into());
        worker.archive.ensure_chat(GROUP, "Club").unwrap();
        worker
            .archive
            .set_group_info(GROUP, Some("Club"), &[ME.into(), ADA.into()], false)
            .unwrap();
        worker.archive.set_group_rights(GROUP, true, admin).unwrap();
        worker
            .archive
            .set_group_settings(
                GROUP,
                &GroupSettings {
                    admins: vec![ME.into()],
                    ..GroupSettings::default()
                },
            )
            .unwrap();
        (worker, events)
    }

    fn row(worker: &Worker) -> Chat {
        worker.archive.chat(GROUP).unwrap().unwrap()
    }

    fn answer(id: &str, error: Option<&str>) -> MemberAnswer {
        MemberAnswer {
            id: id.into(),
            error: error.map(str::to_owned),
        }
    }

    #[test]
    fn metadata_reads_as_permissions_and_admins() {
        let lids = HashMap::from([("900".to_owned(), "15550000003".to_owned())]);
        let mut metadata = GroupMetadata::new(GROUP.parse().unwrap());
        metadata.is_announcement = true;
        metadata.membership_approval = true;
        metadata.member_add_mode = Some(MemberAddMode::AllMemberAdd);
        metadata.member_link_mode = Some(MemberLinkMode::AdminLink);
        let mut creator = GroupParticipant::new("900@lid".parse().unwrap());
        creator.participant_type = ParticipantType::SuperAdmin;
        let mut admin = GroupParticipant::new("200@lid".parse().unwrap());
        admin.phone_number = Some(ADA.parse().unwrap());
        admin.participant_type = ParticipantType::Admin;
        let member = GroupParticipant::new(GRACE.parse().unwrap());
        metadata.participants = vec![creator, admin, member];
        let settings = settings_of(&metadata, &lids);
        assert_eq!(
            settings,
            GroupSettings {
                announce: true,
                members_add: true,
                approval: true,
                members_share_link: false,
                admins: vec!["15550000003@s.whatsapp.net".into(), ADA.into()],
                super_admins: vec!["15550000003@s.whatsapp.net".into()],
            }
        );
        // Unsaid modes are WhatsApp's default: admins only.
        let bare = settings_of(&GroupMetadata::new(GROUP.parse().unwrap()), &lids);
        assert!(!bare.members_add && !bare.members_share_link);
    }

    #[test]
    fn modes_and_errors_translate_to_the_library_and_back() {
        use whatsapp_rust::IqError;
        assert_eq!(add_mode(true), MemberAddMode::AllMemberAdd);
        assert_eq!(add_mode(false), MemberAddMode::AdminAdd);
        assert_eq!(approval_mode(true), MembershipApprovalMode::On);
        assert_eq!(approval_mode(false), MembershipApprovalMode::Off);
        assert_eq!(
            failure_of(&GroupError::Iq(IqError::Timeout)),
            Failure::Timeout
        );
        assert_eq!(
            failure_of(&GroupError::Iq(IqError::NotConnected)),
            Failure::Offline
        );
        assert_eq!(
            failure_of(&GroupError::InvalidRequest("x".into())),
            Failure::Failed
        );
        let lids = HashMap::from([("200".to_owned(), "15550000001".to_owned())]);
        let lid: Jid = "200@lid".parse().unwrap();
        assert_eq!(
            member_answer(&lid, None, Some("403".into()), &lids),
            answer(ADA, Some("403")),
            "a privacy id is filed under its number"
        );
        let grace: Jid = GRACE.parse().unwrap();
        assert_eq!(
            member_answer(&lid, Some(&grace), None, &HashMap::new()),
            answer(GRACE, None)
        );
    }

    /// Nothing reaches WhatsApp unless the group lets us, and without the
    /// link the window hears so at once.
    #[tokio::test]
    async fn requests_are_gated_by_our_rights_and_the_link() {
        let (mut worker, events) = group_worker(false);
        worker
            .handle_command(Command::GroupInviteLink {
                chat: GROUP.into(),
                reset: false,
            })
            .await;
        worker
            .handle_command(Command::AddGroupMembers {
                chat: GROUP.into(),
                members: vec![GRACE.into()],
            })
            .await;
        worker
            .handle_command(Command::SetGroupPermission {
                chat: GROUP.into(),
                permission: Permission::SendMessages,
                members: false,
            })
            .await;
        worker
            .handle_command(Command::ChangeGroupMember {
                chat: GROUP.into(),
                member: ADA.into(),
                change: MemberChange::Remove,
            })
            .await;
        let refused: Vec<_> = events
            .try_iter()
            .filter_map(|event| match event {
                Event::GroupInviteLink { result, .. } => Some(result.err()),
                Event::GroupMembersAdded { result, .. } => Some(result.err()),
                Event::GroupPermissionSaved { result, .. } => Some(result.err()),
                Event::GroupMemberChanged { result, .. } => Some(result.err()),
                _ => None,
            })
            .collect();
        assert_eq!(refused, [Some(Failure::Refused); 4], "a member may do none");
        assert!(
            worker.group_info_queue.contains(&GROUP.to_owned()),
            "a refusal relearns our rights"
        );

        // An admin is allowed, but there is no link to WhatsApp here.
        let (mut worker, events) = group_worker(true);
        worker
            .handle_command(Command::GroupInviteLink {
                chat: GROUP.into(),
                reset: true,
            })
            .await;
        worker
            .handle_command(Command::ChangeGroupMember {
                chat: GROUP.into(),
                member: ME.into(),
                change: MemberChange::Demote,
            })
            .await;
        let answers: Vec<_> = events
            .try_iter()
            .filter_map(|event| match event {
                Event::GroupInviteLink { result, .. } => Some(result.err()),
                Event::GroupMemberChanged { result, .. } => Some(result.err()),
                _ => None,
            })
            .collect();
        assert_eq!(
            answers,
            [Some(Failure::Offline), Some(Failure::Refused)],
            "offline, and never a change to ourselves"
        );
    }

    #[tokio::test]
    async fn added_people_are_filed_and_the_refused_named_by_reason() {
        let (mut worker, events) = group_worker(true);
        let tom = "15550000003@s.whatsapp.net";
        worker
            .handle_command(Command::GroupManaged {
                chat: GROUP.into(),
                answer: GroupAnswer::Added(Ok(vec![
                    answer(GRACE, None),
                    answer(tom, Some("403")),
                    answer(ADA, Some("409")),
                ])),
            })
            .await;
        assert_eq!(row(&worker).participants, [ME, ADA, GRACE]);
        let added = events.try_iter().find_map(|event| match event {
            Event::GroupMembersAdded { result, .. } => Some(result),
            _ => None,
        });
        assert_eq!(
            added,
            Some(Ok(Added {
                chat: GROUP.into(),
                added: vec![GRACE.into(), ADA.into()],
                refused: vec![(tom.into(), Refusal::Privacy)],
            }))
        );
    }

    /// An accepted permission is filed, and metadata asked for before it
    /// cannot bring the old one back.
    #[tokio::test]
    async fn an_accepted_permission_outlives_older_metadata() {
        let (mut worker, _events) = group_worker(true);
        let before = worker.settings_generation.get(GROUP).copied().unwrap_or(0);
        for (permission, members) in [
            (Permission::EditInfo, true),
            (Permission::SendMessages, false),
            (Permission::AddMembers, true),
            (Permission::ApproveMembers, true),
        ] {
            worker
                .handle_command(Command::GroupManaged {
                    chat: GROUP.into(),
                    answer: GroupAnswer::Permission {
                        permission,
                        members,
                        result: Ok(()),
                    },
                })
                .await;
        }
        let filed = row(&worker);
        assert_eq!(filed.info_locked, Some(false));
        let settings = filed.group_settings.unwrap();
        assert!(settings.announce && settings.members_add && settings.approval);

        worker.request_group_info(GROUP, true);
        let generation = worker
            .group_info_generation
            .get(GROUP)
            .copied()
            .unwrap_or(0);
        worker
            .handle_command(Command::GroupInfo {
                chat: GROUP.into(),
                request_generation: generation,
                name: Some("Club".into()),
                description: String::new(),
                description_generation: 0,
                participants: vec![ME.into()],
                read_only: false,
                ephemeral_expiration: None,
                ephemeral_setting_timestamp: None,
                leave_generation: 0,
                info_locked: true,
                admin: true,
                subject_generation: 0,
                settings: GroupSettings::default(),
                settings_generation: before,
            })
            .await;
        let filed = row(&worker);
        assert_eq!(
            filed.info_locked,
            Some(false),
            "the older snapshot is ignored"
        );
        assert!(filed.group_settings.unwrap().announce);
        assert_eq!(filed.participants, [ME, ADA], "and so are its members");
    }

    #[tokio::test]
    async fn accepted_member_changes_move_admins_and_members() {
        let (mut worker, events) = group_worker(true);
        for change in [
            MemberChange::Promote,
            MemberChange::Demote,
            MemberChange::Promote,
        ] {
            worker
                .handle_command(Command::GroupManaged {
                    chat: GROUP.into(),
                    answer: GroupAnswer::Member {
                        member: ADA.into(),
                        change,
                        result: Ok(vec![answer(ADA, None)]),
                    },
                })
                .await;
        }
        assert!(row(&worker).is_group_admin(ADA));
        worker
            .handle_command(Command::GroupManaged {
                chat: GROUP.into(),
                answer: GroupAnswer::Member {
                    member: ADA.into(),
                    change: MemberChange::Remove,
                    result: Ok(vec![answer(ADA, None)]),
                },
            })
            .await;
        let filed = row(&worker);
        assert_eq!(filed.participants, [ME]);
        assert!(!filed.is_group_admin(ADA));
        events.try_iter().for_each(drop);

        // A per-person refusal is a failure, and nothing is filed.
        worker
            .handle_command(Command::GroupManaged {
                chat: GROUP.into(),
                answer: GroupAnswer::Member {
                    member: ME.into(),
                    change: MemberChange::Demote,
                    result: Ok(vec![answer(ME, Some("403"))]),
                },
            })
            .await;
        assert!(row(&worker).is_group_admin(ME));
        let changed = events.try_iter().find_map(|event| match event {
            Event::GroupMemberChanged { result, .. } => Some(result),
            _ => None,
        });
        assert_eq!(changed, Some(Err(Failure::Refused)));
    }

    #[tokio::test]
    async fn the_invite_link_reaches_the_window_unlogged() {
        let (mut worker, events) = group_worker(true);
        let link = InviteLink::new("https://chat.whatsapp.com/Demo123".into()).unwrap();
        worker
            .handle_command(Command::GroupManaged {
                chat: GROUP.into(),
                answer: GroupAnswer::Link {
                    reset: true,
                    result: Ok(link.clone()),
                },
            })
            .await;
        let event = events
            .try_iter()
            .find(|event| matches!(event, Event::GroupInviteLink { .. }))
            .expect("the link is sent on");
        assert!(!format!("{event:?}").contains("Demo123"));
        assert!(matches!(
            event,
            Event::GroupInviteLink { reset: true, result: Ok(got), .. } if got == link
        ));
    }
}
