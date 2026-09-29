//! Channel (newsletter) pictures.
//!
//! A contact's or a group's picture comes from a profile-picture lookup, but a
//! channel answers that lookup with nothing: its picture is part of the
//! channel's metadata, as direct paths on WhatsApp's media servers, readable
//! without a media key. The followed channels' metadata arrives in one list
//! after connecting, so pictures asked for before it arrives wait for it
//! instead of querying each channel; a channel missing from the list (one that
//! is not followed) has its own metadata read.

use super::*;
use crate::backend::ChannelPicture;
use whatsapp_rust::download::DEFAULT_MEDIA_HOSTS;
use whatsapp_rust::features::NewsletterMetadata;

/// The followed channels' pictures, as last listed.
#[derive(Default)]
pub(super) struct ChannelPictures {
    /// The followed channels were listed, or listing them failed.
    listed: bool,
    known: HashMap<ChatId, ChannelPicture>,
}

impl ChannelPicture {
    pub(super) fn of(metadata: &NewsletterMetadata) -> Self {
        Self {
            full: metadata.picture_url.clone(),
            preview: metadata.preview_url.clone(),
        }
    }

    /// The direct path for one size, falling back to the other.
    fn path(&self, full: bool) -> Option<&str> {
        let (wanted, other) = if full {
            (&self.full, &self.preview)
        } else {
            (&self.preview, &self.full)
        };
        wanted.as_deref().or(other.as_deref())
    }
}

/// The address a channel picture's direct path is fetched from.
fn picture_url(direct_path: &str) -> String {
    if direct_path.starts_with("https://") {
        direct_path.to_owned()
    } else {
        format!("https://{}{direct_path}", DEFAULT_MEDIA_HOSTS[0])
    }
}

impl Worker {
    /// Takes the followed channels' pictures, refreshing any that changed
    /// since the last list.
    pub(super) fn channel_pictures_listed(&mut self, list: Option<Vec<(ChatId, ChannelPicture)>>) {
        self.channel_pictures.listed = true;
        for (chat, picture) in list.into_iter().flatten() {
            let previous = self
                .channel_pictures
                .known
                .insert(chat.clone(), picture.clone());
            if previous.is_some_and(|previous| previous != picture) {
                let removed = picture.path(true).is_none();
                self.refresh_avatar(chat, removed);
            }
        }
    }

    /// Fetches a channel's picture from its metadata. `fetch_avatar` has
    /// already found no fresh copy on disk.
    pub(super) fn fetch_channel_avatar(&mut self, id: String, full: bool) {
        let known = self.channel_pictures.known.get(&id).cloned();
        let connected = self
            .client
            .as_ref()
            .is_some_and(|client| client.is_connected());
        let client = self.client.clone().filter(|_| connected);
        let Some(client) = client.filter(|_| known.is_some() || self.channel_pictures.listed)
        else {
            // Wait for the connection and the list of followed channels.
            self.pending_avatars.entry((id, full)).or_insert(0);
            return;
        };
        let Some(jid) = Self::jid_of(&id) else {
            self.emit(Event::Avatar {
                id,
                full,
                path: None,
            });
            return;
        };
        let path = self.avatar_file(&id, full);
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let fetched = async {
                let picture = match known {
                    Some(picture) => picture,
                    None => client
                        .newsletter()
                        .get_metadata(&jid)
                        .await
                        .map(|metadata| ChannelPicture::of(&metadata))
                        .map_err(|error| error.to_string())?,
                };
                match picture.path(full) {
                    Some(direct_path) => download_avatar(picture_url(direct_path), path)
                        .await
                        .map(Some),
                    None => Ok(None),
                }
            }
            .await;
            match fetched {
                Ok(path) => {
                    let _ = commands.send(Command::AvatarFetched { id, full, path });
                }
                Err(error) => {
                    log::debug!("no channel picture yet: {error}");
                    let _ = commands.send(Command::AvatarFailed { id, full });
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_direct_path_is_fetched_from_the_media_host() {
        assert_eq!(
            picture_url("/v/t61.24694-24/fixture.jpg?oh=1&oe=2"),
            "https://mmg.whatsapp.net/v/t61.24694-24/fixture.jpg?oh=1&oe=2"
        );
        assert_eq!(
            picture_url("https://example.invalid/fixture.jpg"),
            "https://example.invalid/fixture.jpg"
        );
    }

    #[test]
    fn each_size_falls_back_to_the_other() {
        let both = ChannelPicture {
            full: Some("/full".into()),
            preview: Some("/preview".into()),
        };
        assert_eq!(both.path(true), Some("/full"));
        assert_eq!(both.path(false), Some("/preview"));
        let full_only = ChannelPicture {
            full: Some("/full".into()),
            preview: None,
        };
        assert_eq!(full_only.path(false), Some("/full"));
        assert_eq!(ChannelPicture::default().path(true), None);
    }

    const CHANNEL: &str = "fixture@newsletter";

    fn picture(path: &str) -> ChannelPicture {
        ChannelPicture {
            full: Some(format!("/{path}")),
            preview: Some(format!("/{path}-preview")),
        }
    }

    #[tokio::test]
    async fn a_picture_waits_for_the_connection_and_the_channel_list() {
        let (mut worker, events, _, _) = super::super::receipt_tests::worker();
        worker.fetch_avatar(CHANNEL.into(), false);
        assert!(
            worker
                .pending_avatars
                .contains_key(&(CHANNEL.to_owned(), false))
        );
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_changed_channel_picture_drops_the_cached_copy() {
        let (mut worker, events, _, _) = super::super::receipt_tests::worker();
        let cached = worker.avatar_file(CHANNEL, false);
        std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
        std::fs::write(&cached, b"fixture").unwrap();

        worker.channel_pictures_listed(Some(vec![(CHANNEL.into(), picture("first"))]));
        worker.channel_pictures_listed(Some(vec![(CHANNEL.into(), picture("first"))]));
        assert!(cached.exists(), "an unchanged picture keeps its cache");

        worker.channel_pictures_listed(Some(vec![(CHANNEL.into(), picture("second"))]));
        assert!(!cached.exists());
        for full in [false, true] {
            assert!(
                worker
                    .pending_avatars
                    .contains_key(&(CHANNEL.to_owned(), full))
            );
        }

        worker.channel_pictures_listed(Some(vec![(CHANNEL.into(), ChannelPicture::default())]));
        let removed: Vec<_> = events
            .try_iter()
            .filter(|event| matches!(event, Event::Avatar { path: None, .. }))
            .collect();
        assert_eq!(removed.len(), 2);
    }
}
