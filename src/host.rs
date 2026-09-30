//! The seam between the window and the networks.
//!
//! The host owns the account list that `state/accounts.json` names and one
//! worker per account. Commands are routed by the account their chat carries;
//! events from every worker share one stream, and every chat-scoped event
//! already names its account in the chat id.

use crate::account::Account;
use crate::backend::{Backend, Command, Event, Waker};
use crate::paths::AppDirs;

/// UI handle to every account's worker.
pub struct Host {
    accounts: Vec<Account>,
    workers: Vec<(crate::account::AccountId, Backend)>,
    dirs: Option<AppDirs>,
    waker: Option<Waker>,
}

impl Host {
    /// Starts the workers for the accounts `state/accounts.json` names.
    pub fn spawn(dirs: AppDirs, waker: Waker) -> Self {
        let accounts = crate::account::load(&dirs.accounts_file());
        let workers = accounts
            .iter()
            .filter(|account| account.kind.supported())
            .map(|account| {
                (
                    account.id,
                    Backend::spawn_for(dirs.clone(), account.clone(), waker.clone()),
                )
            })
            .collect();
        Self {
            accounts,
            workers,
            dirs: Some(dirs),
            waker: Some(waker),
        }
    }

    /// Builds a host around an existing worker, for demos and tests.
    pub fn of(accounts: Vec<Account>, backend: Backend) -> Self {
        Self {
            accounts,
            workers: vec![(crate::account::AccountId::WHATSAPP, backend)],
            dirs: None,
            waker: None,
        }
    }

    /// Builds a host around one worker per account, for tests that drive
    /// commands directly.
    #[cfg(test)]
    pub(crate) fn of_workers(
        accounts: Vec<Account>,
        workers: Vec<(crate::account::AccountId, Backend)>,
    ) -> Self {
        Self {
            accounts,
            workers,
            dirs: None,
            waker: None,
        }
    }

    /// The accounts the switcher can show, in order.
    pub fn accounts(&self) -> &[Account] {
        &self.accounts
    }

    /// Adds an account of a network that ships an adapter, stores the list,
    /// and starts its worker.
    pub fn add_account(&mut self, kind: crate::account::NetworkKind) -> Option<Account> {
        let account = match kind {
            crate::account::NetworkKind::Telegram => {
                Account::telegram(crate::account::next_id(&self.accounts))
            }
            crate::account::NetworkKind::Matrix => {
                Account::matrix(crate::account::next_id(&self.accounts))
            }
            crate::account::NetworkKind::Slack => {
                Account::slack(crate::account::next_id(&self.accounts))
            }
            crate::account::NetworkKind::Zulip => {
                Account::zulip(crate::account::next_id(&self.accounts))
            }
            crate::account::NetworkKind::DiscordBot => {
                Account::discord(crate::account::next_id(&self.accounts))
            }
            crate::account::NetworkKind::X => Account::x(crate::account::next_id(&self.accounts)),
            crate::account::NetworkKind::DeltaChat => {
                Account::delta(crate::account::next_id(&self.accounts))
            }
            _ => return None,
        };
        self.accounts.push(account.clone());
        if let Some(dirs) = &self.dirs
            && let Err(error) = crate::account::save(&dirs.accounts_file(), &self.accounts)
        {
            log::warn!("could not store the account list: {error:#}");
        }
        if let (Some(dirs), Some(waker)) = (&self.dirs, &self.waker) {
            self.workers.push((
                account.id,
                Backend::spawn_for(dirs.clone(), account.clone(), waker.clone()),
            ));
        }
        Some(account)
    }

    /// Swaps in another worker, for tests that drive commands directly.
    #[cfg(test)]
    pub(crate) fn replace(&mut self, backend: Backend) {
        self.workers = vec![(crate::account::AccountId::WHATSAPP, backend)];
    }

    /// Disables commands except shutdown on every worker.
    pub fn set_offline(&mut self, offline: bool) {
        for (_, worker) in &mut self.workers {
            worker.set_offline(offline);
        }
    }

    pub fn is_offline(&self) -> bool {
        self.workers.iter().all(|(_, worker)| worker.is_offline())
    }

    pub fn send(&self, command: Command) {
        match &command {
            Command::Shutdown => {
                for (_, worker) in &self.workers {
                    worker.send(Command::Shutdown);
                }
            }
            Command::Login { account, .. } => {
                if let Some((_, worker)) = self.workers.iter().find(|(id, _)| id == account) {
                    worker.send(command);
                }
            }
            _ => match command_account(&command) {
                Some(account) => {
                    if let Some((_, worker)) = self.workers.iter().find(|(id, _)| *id == account) {
                        worker.send(command);
                    }
                }
                None => {
                    // Commands without a chat belong to the original WhatsApp
                    // account while it is the only network that defines them.
                    if let Some((_, worker)) = self.workers.first() {
                        worker.send(command);
                    }
                }
            },
        }
    }

    pub fn poll(&self) -> Vec<Event> {
        let mut events = Vec::new();
        for (_, worker) in &self.workers {
            events.extend(worker.poll());
        }
        events
    }

    /// Startup permits for every worker. Each one starts its account once the
    /// first frame acknowledged it.
    pub fn take_startup(&mut self) -> Vec<tokio::sync::oneshot::Sender<()>> {
        self.workers
            .iter_mut()
            .filter_map(|(_, worker)| worker.take_startup())
            .collect()
    }

    pub fn shutdown(&mut self) {
        for (_, worker) in &mut self.workers {
            worker.shutdown();
        }
    }

    /// Captures real UI commands for an offline demo's local responder.
    #[cfg(any(test, feature = "demo"))]
    pub(crate) fn record_demo_commands(&mut self) {
        for (_, worker) in &mut self.workers {
            worker.record_demo_commands();
        }
    }

    #[cfg(any(test, feature = "demo"))]
    pub(crate) fn take_demo_commands(&self) -> Vec<Command> {
        self.workers
            .iter()
            .flat_map(|(_, worker)| worker.take_demo_commands())
            .collect()
    }
}

/// The account a command acts on, when its chat names one.
fn command_account(command: &Command) -> Option<crate::account::AccountId> {
    let chat = match command {
        Command::RefreshPoll { chat, .. }
        | Command::ReplyInteractive { chat, .. }
        | Command::SendText { chat, .. }
        | Command::Forward {
            from_chat: chat, ..
        }
        | Command::Composing { chat, .. }
        | Command::SaveDraft { chat, .. }
        | Command::MarkRead { chat, .. }
        | Command::LoadChat { chat, .. }
        | Command::Download { chat, .. }
        | Command::LoadUntil { chat, .. }
        | Command::SearchChatMessages { chat, .. }
        | Command::EnsureChat { chat, .. }
        | Command::EditText { chat, .. }
        | Command::Revoke { chat, .. }
        | Command::DeleteLocal { chat, .. }
        | Command::SendFiles { chat, .. }
        | Command::SendImage { chat, .. }
        | Command::SendVoice { chat, .. }
        | Command::SendSticker { chat, .. }
        | Command::React { chat, .. } => chat,
        Command::MarkUnread(chat)
        | Command::FetchOlder(chat)
        | Command::PickFiles(chat)
        | Command::SetArchived(chat, _)
        | Command::DeleteChat(chat)
        | Command::ClearChat(chat)
        | Command::SetMuted(chat, _)
        | Command::SetLocked(chat, _) => chat,
        Command::WatchReceipts(Some((chat, _))) => chat,
        _ => return None,
    };
    Some(chat.account())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::{AccountId, NetworkKind};
    use crate::backend::LinkStatus;
    use crate::model::ChatId;

    #[test]
    fn commands_reach_the_whatsapp_worker() {
        let (backend, mut commands) = Backend::recording();
        let host = Host::of(vec![Account::whatsapp()], backend);
        let chat = ChatId::whatsapp("1@s.whatsapp.net");
        host.send(Command::MarkUnread(chat.clone()));
        assert!(matches!(commands.try_recv(), Ok(Command::MarkUnread(id)) if id == chat));
    }

    #[test]
    fn commands_route_to_the_account_that_owns_the_chat() {
        let (first, mut first_commands) = Backend::recording();
        let (second, mut second_commands) = Backend::recording();
        let telegram = Account {
            id: AccountId(2),
            kind: NetworkKind::Telegram,
            name: "Telegram".to_owned(),
        };
        let host = Host::of_workers(
            vec![Account::whatsapp(), telegram.clone()],
            vec![(AccountId::WHATSAPP, first), (telegram.id, second)],
        );
        let chat = ChatId::new(telegram.id, "42");
        host.send(Command::MarkUnread(chat.clone()));
        assert!(first_commands.try_recv().is_err());
        assert!(matches!(second_commands.try_recv(), Ok(Command::MarkUnread(id)) if id == chat));
        host.send(Command::MarkUnread(ChatId::whatsapp("1@s.whatsapp.net")));
        assert!(first_commands.try_recv().is_ok());
        assert!(second_commands.try_recv().is_err());
    }

    #[test]
    fn events_come_back_for_the_accounts_the_host_owns() {
        let (backend, _commands, events) = Backend::recording_with_events();
        let host = Host::of(vec![Account::whatsapp()], backend);
        events.send(Event::Link(LinkStatus::Starting)).unwrap();
        assert!(matches!(
            host.poll().as_slice(),
            [Event::Link(LinkStatus::Starting)]
        ));
    }

    #[test]
    fn the_account_list_keeps_its_order() {
        let (backend, _) = Backend::recording();
        let host = Host::of(
            vec![
                Account::whatsapp(),
                Account {
                    id: AccountId(2),
                    kind: NetworkKind::Telegram,
                    name: "Ada".to_owned(),
                },
            ],
            backend,
        );
        assert_eq!(host.accounts().len(), 2);
        assert_eq!(host.accounts()[0].kind, NetworkKind::WhatsApp);
        assert_eq!(host.accounts()[1].kind, NetworkKind::Telegram);
    }
}
