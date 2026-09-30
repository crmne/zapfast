//! Zulip account adapter: one server per account.
//!
//! The account signs in with the server URL, an email address, and the user's
//! API key, all kept in the OS keyring. Events arrive through a registered
//! queue and a long poll, which needs no public URL and no inbound port, and
//! messages are projected through [`project`] into the same models every
//! other account uses.

pub mod project;
mod runtime;
pub(crate) use runtime::run;
