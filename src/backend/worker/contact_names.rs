//! First names saved with contacts before ZapFast kept them.
//!
//! The phone sends a contact again only when it changes, so a contact synced
//! before first names were kept has none, and its short name (a group's
//! member line, the sender before a group's last message) falls back to the
//! first word of its full name: "My Dih", saved as a first name, showed as
//! "My" (#314). Rebuilding the contact collection (CriticalUnblockLow) once
//! replays every contact with the first name saved with it. A new link
//! receives them with its first sync and skips this.

use super::*;

/// Archive marker: the contacts were replayed with their first names.
pub(super) const FIRST_NAMES_RECOVERED: &str = "contact_first_names_recovered_v1";

impl Worker {
    /// Replays the phone's contacts once, so each brings its first name.
    pub(super) fn recover_first_names(&mut self) {
        if self.first_names_recovered
            || self.first_names_recovering
            || !matches!(self.status, LinkStatus::Connected)
        {
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };
        self.first_names_recovering = true;
        log::info!("reading contact first names from the phone");
        let commands = self.commands.clone();
        tokio::spawn(async move {
            use whatsapp_rust::WAPatchName;
            let complete = match client
                .resync_app_state(
                    [WAPatchName::CriticalUnblockLow],
                    whatsapp_rust::AppStateResyncMode::Snapshot,
                )
                .await
            {
                Ok(report) => report.synced.contains(&WAPatchName::CriticalUnblockLow),
                Err(error) => {
                    log::warn!("could not read contact first names from the phone: {error}");
                    false
                }
            };
            let _ = commands.send(Command::FirstNamesRecovered { complete });
        });
    }

    /// The one-time contact replay finished, or waits for the next
    /// connection.
    pub(super) fn first_names_recovered(&mut self, complete: bool) {
        self.first_names_recovering = false;
        if !complete {
            log::warn!(
                "contact first names from the phone not read yet; retrying on the next connection"
            );
            return;
        }
        self.first_names_recovered = true;
        if let Err(error) = self.archive.set_meta(FIRST_NAMES_RECOVERED, "complete") {
            log::warn!("could not record the contact first-name sync: {error}");
        }
    }
}
