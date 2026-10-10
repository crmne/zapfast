//! The window, not the client, owns the account's online presence (#461).
//!
//! whatsapp-rust's default `PresencePolicy::Automatic` announces `available`
//! on its own whenever the client connects or the push name syncs. For an
//! app that stays connected in the tray that showed the account online with
//! the window closed, and WhatsApp holds back the phone's own notifications
//! while any linked device is available. `Worker::start_bot` builds every
//! bot through `zapfast::backend::host_owned_presence`; this test drives
//! that same builder step against a real client and store.
//!
//! whatsapp-rust exposes no wire-level test hook (its capturing transports
//! are crate-internal), so the assertions sit at the client surface: the
//! policy the lifecycle consults before every automatic announcement, and
//! the validation outcome of the explicit announcements zapfast sends.

use whatsapp_rust::features::{PresenceError, PresencePolicy};
use whatsapp_rust::prelude::Bot;
use whatsapp_rust::store::SqliteStore;
use whatsapp_rust::store::commands::DeviceCommand;

async fn store(directory: &tempfile::TempDir) -> SqliteStore {
    SqliteStore::new(directory.path().join("session.db").to_str().unwrap())
        .await
        .expect("an in-file store builds")
}

#[tokio::test]
async fn zapfast_bots_leave_presence_to_the_window() {
    let directory = tempfile::tempdir().unwrap();
    // Building a client is enough; never run or connect this bot.
    let bot =
        zapfast::backend::host_owned_presence(Bot::builder().with_backend(store(&directory).await))
            .build()
            .await
            .expect("the bot builds");
    let client = bot.client();
    assert_eq!(
        client.presence_policy(),
        PresencePolicy::Manual,
        "an automatic announcement would show a trayed window as online (#461)"
    );

    // The explicit announcements the worker sends on window focus changes
    // still validate under the manual policy: with a push name set and no
    // connection, the failure is the connection, never the policy or the
    // missing push name.
    client
        .persistence_manager()
        .process_command(DeviceCommand::SetPushName("ZapFast".to_owned()))
        .await;
    for online in [true, false] {
        let presence = client.presence();
        let result = if online {
            presence.set_available().await
        } else {
            presence.set_unavailable().await
        };
        assert!(
            matches!(result, Err(PresenceError::Client(_))),
            "explicit presence must pass validation and fail only on the \
             missing connection, got {result:?}"
        );
    }
}

#[tokio::test]
async fn whatsapp_rust_announces_on_its_own_without_the_override() {
    // The behavior behind #461: a bot built without the override keeps the
    // default Automatic policy, so connecting would announce "available"
    // regardless of the window.
    let directory = tempfile::tempdir().unwrap();
    let bot = Bot::builder()
        .with_backend(store(&directory).await)
        .build()
        .await
        .expect("the bot builds");
    assert_eq!(
        bot.client().presence_policy(),
        PresencePolicy::Automatic,
        "if whatsapp-rust changes its default, the override above needs a second look"
    );
}
