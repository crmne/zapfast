//! Multi-account manager coordinating concurrent background synchronization,
//! active account selection, and aggregated state across all linked accounts.

use std::time::Instant;

use crate::backend::LinkStatus;
use crate::model::AccountId;
use super::model::Account;

/// Tracks background synchronization state for a single account.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountSyncState {
    pub is_syncing: bool,
    pub sync_percent: Option<u32>,
    pub last_sync_progress: Option<Instant>,
}

/// Multi-account manager coordinating concurrent background synchronization,
/// active account selection, and aggregated state across all linked accounts.
pub struct AccountManager {
    accounts: Vec<Account>,
    active: usize,
    background_sync: bool,
}

impl AccountManager {
    /// Creates an account manager wrapping accounts and setting the active account index.
    pub fn new(accounts: Vec<Account>, active: usize) -> Self {
        Self {
            accounts,
            active,
            background_sync: true,
        }
    }

    #[inline]
    pub fn active(&self) -> &Account {
        &self.accounts[self.active]
    }

    #[inline]
    pub fn active_mut(&mut self) -> &mut Account {
        &mut self.accounts[self.active]
    }

    #[inline]
    pub fn active_index(&self) -> usize {
        self.active
    }

    #[inline]
    pub fn set_active_index(&mut self, index: usize) {
        if index >= self.accounts.len() {
            return;
        }
        self.active = index;
    }

    #[inline]
    pub fn active_id(&self) -> AccountId {
        self.active().id.clone()
    }

    #[inline]
    pub fn get(&self, id: &AccountId) -> Option<&Account> {
        self.accounts.iter().find(|account| &account.id == id)
    }

    #[inline]
    pub fn get_mut(&mut self, id: &AccountId) -> Option<&mut Account> {
        self.accounts.iter_mut().find(|account| &account.id == id)
    }

    #[inline]
    pub fn get_at(&self, index: usize) -> Option<&Account> {
        self.accounts.get(index)
    }

    #[inline]
    pub fn get_at_mut(&mut self, index: usize) -> Option<&mut Account> {
        self.accounts.get_mut(index)
    }

    #[inline]
    pub fn find_index(&self, id: &AccountId) -> Option<usize> {
        self.accounts.iter().position(|account| &account.id == id)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.accounts.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty()
    }

    #[inline]
    pub fn iter(&self) -> std::slice::Iter<'_, Account> {
        self.accounts.iter()
    }

    #[inline]
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, Account> {
        self.accounts.iter_mut()
    }

    #[inline]
    pub fn as_slice(&self) -> &[Account] {
        &self.accounts
    }

    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [Account] {
        &mut self.accounts
    }

    #[inline]
    pub fn push(&mut self, account: Account) {
        self.accounts.push(account);
    }

    #[inline]
    pub fn remove(&mut self, index: usize) -> Account {
        let removed = self.accounts.remove(index);
        if self.active >= self.accounts.len() && !self.accounts.is_empty() {
            self.active = self.accounts.len() - 1;
        }
        removed
    }

    #[inline]
    pub fn retain<F>(&mut self, f: F)
    where
        F: FnMut(&Account) -> bool,
    {
        self.accounts.retain(f);
        if self.active >= self.accounts.len() && !self.accounts.is_empty() {
            self.active = self.accounts.len() - 1;
        }
    }

    #[inline]
    pub fn background_sync(&self) -> bool {
        self.background_sync
    }

    #[inline]
    pub fn set_background_sync(&mut self, enabled: bool) {
        self.background_sync = enabled;
    }

    /// Aggregated taskbar unread badge count across all linked accounts.
    pub fn unread_chat_count_total(&self) -> u32 {
        let mut total: u32 = 0;
        for account in &self.accounts {
            total = total.saturating_add(account.unread_chat_count());
        }
        total
    }

    /// Whether any managed account is currently synchronizing history or state.
    #[inline]
    pub fn is_any_syncing(&self) -> bool {
        self.accounts.iter().any(|account| account.syncing)
    }

    /// Drains backend events across all accounts in a single pass.
    pub fn poll_all_events(&mut self) -> Vec<(AccountId, usize, Vec<crate::backend::Event>)> {
        let mut result = Vec::new();
        for (index, account) in self.accounts.iter_mut().enumerate() {
            let events = account.backend.poll();
            if events.is_empty() {
                continue;
            }
            result.push((account.id.clone(), index, events));
        }
        result
    }

    /// Updates presence for all accounts. The active account reflects looking state.
    /// Inactive accounts remain connected in the background, staying marked online only
    /// while actively downloading history to prevent WhatsApp from pausing sync.
    pub fn sync_presence(&mut self, looking: bool) {
        let active = self.active;
        let bg_sync = self.background_sync;
        for (index, account) in self.accounts.iter_mut().enumerate() {
            let online = (looking && index == active) || (bg_sync && account.syncing);
            if account.reported_online == Some(online) {
                continue;
            }
            account.reported_online = Some(online);
            account.backend.send(crate::backend::Command::SetOnline(online));
        }
    }

    /// Syncs history prefetch state across all accounts. When background sync is active,
    /// inactive accounts continue prefetching unarchived and recent chats.
    pub fn sync_prefetch(&mut self) {
        let active = self.active;
        let bg_sync = self.background_sync;
        for (index, account) in self.accounts.iter_mut().enumerate() {
            let is_active = index == active;
            let prefetch_on = account.settings.history_prefetch || (bg_sync && account.syncing);
            let focused = if is_active { account.open_chat.clone() } else { None };
            let wanted = (prefetch_on, focused);
            if account.prefetch_sent.as_ref() == Some(&wanted) {
                continue;
            }
            account.backend.send(crate::backend::Command::SetHistoryPrefetch {
                on: wanted.0,
                focused: wanted.1.clone(),
            });
            account.prefetch_sent = Some(wanted);
        }
    }

    /// Pings or reconnects background accounts that may have dropped connectivity.
    pub fn ensure_background_keepalive(&mut self) {
        if !self.background_sync {
            return;
        }
        for (index, account) in self.accounts.iter_mut().enumerate() {
            if index == self.active {
                continue;
            }
            if matches!(account.link, LinkStatus::Disconnected { .. }) {
                account.backend.send(crate::backend::Command::Reconnect);
            }
        }
    }
}

impl std::ops::Deref for AccountManager {
    type Target = [Account];
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.accounts
    }
}

impl std::ops::DerefMut for AccountManager {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.accounts
    }
}

impl std::ops::Index<usize> for AccountManager {
    type Output = Account;
    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        &self.accounts[index]
    }
}

impl std::ops::IndexMut<usize> for AccountManager {
    #[inline]
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.accounts[index]
    }
}

impl<'a> IntoIterator for &'a AccountManager {
    type Item = &'a Account;
    type IntoIter = std::slice::Iter<'a, Account>;
    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.accounts.iter()
    }
}

impl<'a> IntoIterator for &'a mut AccountManager {
    type Item = &'a mut Account;
    type IntoIter = std::slice::IterMut<'a, Account>;
    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.accounts.iter_mut()
    }
}
