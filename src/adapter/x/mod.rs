//! The X direct message adapter.
//!
//! Sign-in is OAuth 2.0 with PKCE against the user's own developer app: the
//! window shows the authorization link, the browser redirect comes back to a
//! loopback address, and the tokens live in the OS keyring. Only the last
//! thirty days of legacy direct messages are available through the API.

pub mod project;
mod runtime;
pub(crate) use runtime::run;
