use std::collections::HashMap;

pub use crate::account::{Account, AccountManager, AccountSyncState};
use crate::app::App;
use crate::backend::Command;
use crate::model::{AccountId, Action, Chat, ChatId, Contact};

/// Contextual developer utility for inspecting and managing accounts on an active App instance.
pub struct AccountScope<'a> {
    app: &'a mut App,
}

pub type AccountToolkit<'a> = AccountScope<'a>;
pub type AccountUtils<'a> = AccountScope<'a>;

impl<'a> AccountScope<'a> {
    pub fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    #[inline]
    pub fn active(&self) -> &Account {
        self.app.account()
    }

    #[inline]
    pub fn active_mut(&mut self) -> &mut Account {
        self.app.account_mut()
    }

    #[inline]
    pub fn active_id(&self) -> AccountId {
        self.app.account().id.clone()
    }

    #[inline]
    pub fn all(&self) -> &[Account] {
        &self.app.accounts
    }

    #[inline]
    pub fn all_mut(&mut self) -> &mut [Account] {
        &mut self.app.accounts
    }

    #[inline]
    pub fn get(&self, id: &AccountId) -> Option<&Account> {
        self.app.accounts.iter().find(|account| &account.id == id)
    }

    #[inline]
    pub fn get_mut(&mut self, id: &AccountId) -> Option<&mut Account> {
        self.app.accounts.iter_mut().find(|account| &account.id == id)
    }

    #[inline]
    pub fn is_linked(&self) -> bool {
        self.active().is_linked()
    }

    #[inline]
    pub fn is_syncing(&self) -> bool {
        self.active().syncing
    }

    #[inline]
    pub fn is_any_syncing(&self) -> bool {
        self.app.accounts.iter().any(|account| account.syncing)
    }

    #[inline]
    pub fn sync_progress(&self) -> Option<u32> {
        self.active().sync_percent
    }

    #[inline]
    pub fn my_phone(&self) -> Option<String> {
        self.active().me.clone()
    }

    #[inline]
    pub fn my_name(&self) -> Option<&str> {
        self.active().me_name.as_deref()
    }

    #[inline]
    pub fn my_about(&self) -> Option<&str> {
        self.active().me_about.as_deref()
    }

    #[inline]
    pub fn contacts(&self) -> &HashMap<String, Contact> {
        &self.active().contacts
    }

    #[inline]
    pub fn chats(&self) -> &[Chat] {
        &self.active().chats
    }

    #[inline]
    pub fn open_chat_id(&self) -> Option<&str> {
        self.active().open_chat.as_deref()
    }

    pub fn unread_chat_count_total(&self) -> u32 {
        let mut total: u32 = 0;
        for account in &self.app.accounts {
            total = total.saturating_add(account.unread_chat_count());
        }
        total
    }

    pub fn switch_to(&mut self, id: AccountId) {
        self.app.actions.push(Action::SwitchAccount(id));
    }

    pub fn add_new(&mut self) {
        self.app.actions.push(Action::AddAccount);
    }

    pub fn remove(&mut self, id: AccountId) {
        self.app.actions.push(Action::RemoveAccount(id));
    }

    pub fn open_chat(&mut self, chat_id: impl Into<ChatId>) {
        self.app.actions.push(Action::OpenChat(chat_id.into()));
    }

    pub fn clear_media_cache(&mut self, id: AccountId) {
        self.app.actions.push(Action::ClearMediaCache(id));
    }

    pub fn set_notifications(&mut self, id: AccountId, enabled: bool) {
        self.app.actions.push(Action::SetAccountNotifications(id, enabled));
    }

    pub fn set_background_sync(&mut self, enabled: bool) {
        self.active_mut().settings.background_sync = enabled;
        self.active_mut().mark_settings_dirty();
    }

    /// Triggers immediate prefetch and presence synchronization across all accounts.
    pub fn sync_all(&mut self) {
        let active = self.app.active;
        for (index, account) in self.app.accounts.iter_mut().enumerate() {
            let is_active = index == active;
            let wanted = (
                account.settings.history_prefetch
                    || (account.settings.background_sync && account.syncing),
                is_active.then(|| account.open_chat.clone()).flatten(),
            );
            if account.prefetch_sent.as_ref() == Some(&wanted) {
                continue;
            }
            account.backend.send(Command::SetHistoryPrefetch {
                on: wanted.0,
                focused: wanted.1.clone(),
            });
            account.prefetch_sent = Some(wanted);
        }
    }

    /// Reconnects disconnected accounts in the background.
    pub fn reconnect_all(&mut self) {
        for account in &self.app.accounts {
            if matches!(account.link, crate::backend::LinkStatus::Disconnected { .. }) {
                account.backend.send(Command::Reconnect);
            }
        }
    }
}
