//! The seam between the window and the networks.
//!
//! The host owns the account list that `state/accounts.json` names and the
//! worker each account talks through. WhatsApp is the only network with a
//! worker so far; other networks arrive as adapters beside it, and every
//! command keeps the account it belongs to in its chat id.

use crate::account::Account;
use crate::backend::{Backend, Command, Event, Waker};
use crate::paths::AppDirs;

/// UI handle to every account's worker.
pub struct Host {
    accounts: Vec<Account>,
    backend: Backend,
}

impl Host {
    /// Starts the workers for the accounts `state/accounts.json` names.
    pub fn spawn(dirs: AppDirs, waker: Waker) -> Self {
        let accounts = crate::account::load(&dirs.accounts_file());
        let backend = Backend::spawn(dirs, waker);
        Self { accounts, backend }
    }

    /// Builds a host around an existing worker, for demos and tests.
    pub fn of(accounts: Vec<Account>, backend: Backend) -> Self {
        Self { accounts, backend }
    }

    /// The accounts the switcher can show, in order.
    pub fn accounts(&self) -> &[Account] {
        &self.accounts
    }

    /// Swaps in another worker, for tests that drive commands directly.
    #[cfg(test)]
    pub(crate) fn replace(&mut self, backend: Backend) {
        self.backend = backend;
    }

    /// Disables commands except shutdown.
    pub fn set_offline(&mut self, offline: bool) {
        self.backend.set_offline(offline);
    }

    pub fn is_offline(&self) -> bool {
        self.backend.is_offline()
    }

    pub fn send(&self, command: Command) {
        self.backend.send(command);
    }

    pub fn poll(&self) -> Vec<Event> {
        self.backend.poll()
    }

    pub fn take_startup(&mut self) -> Option<tokio::sync::oneshot::Sender<()>> {
        self.backend.take_startup()
    }

    pub fn shutdown(&mut self) {
        self.backend.shutdown();
    }

    /// Captures real UI commands for an offline demo's local responder.
    #[cfg(any(test, feature = "demo"))]
    pub(crate) fn record_demo_commands(&mut self) {
        self.backend.record_demo_commands();
    }

    #[cfg(any(test, feature = "demo"))]
    pub(crate) fn take_demo_commands(&self) -> Vec<Command> {
        self.backend.take_demo_commands()
    }
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
        assert_eq!(host.accounts()[1].kind, NetworkKind::Telegram);
    }
}
