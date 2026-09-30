//! The Delta Chat adapter.
//!
//! Delta Chat is a mail client: the account is an email address on any server
//! that speaks standard mail, and the messages are protected end to end when
//! both sides use Delta Chat. ZapFast does not link the mail library into the
//! window; it runs the official `deltachat-rpc-server` beside it and speaks
//! the JSON-RPC dialect over stdio. One server runs per account, with its
//! state under that account's directory, so deleting an account removes only
//! its own mail store.
//!
//! The server binary is pinned by revision and installed outside this
//! repository; `ZAPFAST_DELTA_RPC` names it, or `deltachat-rpc-server` from
//! the PATH. Everything else in this module is projection and networking.

pub mod project;
mod runtime;
pub(crate) use runtime::run;
