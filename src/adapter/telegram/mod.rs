//! The Telegram account: login, updates, and projection into the window's
//! models.
//!
//! The protocol comes from the pinned `grammers` crates. This module keeps the
//! pieces the window needs: the session store, the protocol-to-model
//! projection, and the account runtime that talks to the network.

pub mod project;
mod runtime;
mod session;

pub(crate) use runtime::run;
pub use session::TelegramSession;
