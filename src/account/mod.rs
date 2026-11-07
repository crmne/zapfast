//! WhatsApp multi-account runtime state and background sync management.

pub mod manager;
pub mod model;
pub(crate) mod wallpaper;

pub use manager::{AccountManager, AccountSyncState};
pub use model::Account;
