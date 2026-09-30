//! The Discord bot adapter.
//!
//! One gateway shard and one REST client per account, both over rustls. The
//! bot token comes from the OS keyring; only bot accounts are supported, never
//! personal user tokens. Projection into the window's models lives in
//! [`project`].

pub mod project;
mod runtime;

pub(crate) use runtime::run;
