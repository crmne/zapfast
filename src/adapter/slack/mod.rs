//! Slack accounts over Socket Mode.
//!
//! One account talks to one Slack workspace with two tokens: an app token
//! that opens the socket, and a bot token for the Web API. Tokens live in the
//! OS keyring, conversations project through [`project`], and thread replies
//! become their own rows under their channel.

pub mod project;
mod runtime;
pub(crate) use runtime::run;
