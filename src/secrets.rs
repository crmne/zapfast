//! Small secrets kept in the OS keyring.
//!
//! Network tokens live beside the archive key, not in settings.json. An
//! identity names one secret for one account; nothing here is ever logged.

use anyhow::{Context, Result};
use keyring_core::api::CredentialStoreApi;

#[cfg(target_os = "linux")]
type PlatformStore = zbus_secret_service_keyring_store::Store;
#[cfg(target_os = "macos")]
type PlatformStore = apple_native_keyring_store::keychain::Store;
#[cfg(windows)]
type PlatformStore = windows_native_keyring_store::Store;

fn platform_store() -> Result<std::sync::Arc<PlatformStore>> {
    #[cfg(target_os = "linux")]
    let store = zbus_secret_service_keyring_store::Store::new();
    #[cfg(target_os = "macos")]
    let store = apple_native_keyring_store::keychain::Store::new();
    #[cfg(windows)]
    let store = windows_native_keyring_store::Store::new();
    store
        .map_err(|error| anyhow::anyhow!("{error}"))
        .context("Unlock your OS keyring and restart ZapFast")
}

fn entry_for(store: &impl CredentialStoreApi, identity: &str) -> Result<keyring_core::Entry> {
    store
        .build("rocks.zapfast.ZapFast", identity, None)
        .map_err(|error| anyhow::anyhow!("{error}"))
        .context("The OS keyring could not open this secret")
}

/// Saves a secret for an identity, replacing what was there.
pub fn save(identity: &str, secret: &str) -> Result<()> {
    let store = platform_store()?;
    save_in(&*store, identity, secret)
}

/// The secret for an identity, or `None` when none was saved yet.
pub fn load(identity: &str) -> Result<Option<String>> {
    let store = platform_store()?;
    load_in(&*store, identity)
}

/// Forgets a secret. A missing credential is already gone.
pub fn delete(identity: &str) -> Result<()> {
    let store = platform_store()?;
    delete_in(&*store, identity)
}

pub(crate) fn save_in(store: &impl CredentialStoreApi, identity: &str, secret: &str) -> Result<()> {
    entry_for(store, identity)?
        .set_secret(secret.as_bytes())
        .map_err(|error| anyhow::anyhow!("{error}"))
        .context("Could not save the secret in the OS keyring")
}

pub(crate) fn load_in(store: &impl CredentialStoreApi, identity: &str) -> Result<Option<String>> {
    match entry_for(store, identity)?.get_secret() {
        Ok(secret) => match String::from_utf8(secret) {
            Ok(secret) => Ok(Some(secret)),
            Err(_) => anyhow::bail!("The secret in the OS keyring is not text"),
        },
        Err(keyring_core::Error::NoEntry) => Ok(None),
        Err(error) => {
            Err(anyhow::anyhow!("{error}")).context("Unlock your OS keyring and restart ZapFast")
        }
    }
}

pub(crate) fn delete_in(store: &impl CredentialStoreApi, identity: &str) -> Result<()> {
    match entry_for(store, identity)?.delete_credential() {
        Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
        Err(error) => Err(anyhow::anyhow!("{error}"))
            .context("Could not remove the secret from the OS keyring"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_secret_comes_back_and_can_be_forgotten() {
        let store = keyring_core::mock::Store::new().unwrap();
        assert_eq!(load_in(&*store, "slack-bot-2").unwrap(), None);
        save_in(&*store, "slack-bot-2", "xoxb-fixture").unwrap();
        assert_eq!(
            load_in(&*store, "slack-bot-2").unwrap(),
            Some("xoxb-fixture".to_owned())
        );
        delete_in(&*store, "slack-bot-2").unwrap();
        assert_eq!(load_in(&*store, "slack-bot-2").unwrap(), None);
    }

    #[test]
    fn saving_again_replaces_the_secret() {
        let store = keyring_core::mock::Store::new().unwrap();
        save_in(&*store, "slack-app-2", "xapp-one").unwrap();
        save_in(&*store, "slack-app-2", "xapp-two").unwrap();
        assert_eq!(
            load_in(&*store, "slack-app-2").unwrap(),
            Some("xapp-two".to_owned())
        );
    }
}
