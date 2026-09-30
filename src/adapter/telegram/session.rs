//! Telegram session storage backed by the account's own SQLite file.
//!
//! The Telegram client can use any [`Session`] implementation, so the account directory
//! holds a plain rusqlite database instead of the storage the crate ships with.
//! That keeps a single SQLite build in the binary and puts the Telegram
//! authorization keys next to the account they belong to. The file holds
//! secrets: never log its contents, and delete it with its account.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use anyhow::{Context as _, Result, anyhow};
use grammers_session::types::{
    ChannelState, DcOption, PeerId, PeerInfo, UpdateState, UpdatesState,
};
use grammers_session::{BoxFuture, Session, SessionData};
use rusqlite::{Connection, OptionalExtension, params};

/// The Telegram session for one account, stored in the account directory.
pub struct TelegramSession {
    path: PathBuf,
    inner: Mutex<Inner>,
}

struct Inner {
    connection: Connection,
    data: SessionData,
}

/// The session's own error type. grammers requires `std::error::Error`, which
/// anyhow does not implement, so the session wraps anyhow errors here.
#[derive(Debug)]
pub struct SessionError(anyhow::Error);

impl std::fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl std::error::Error for SessionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

impl From<anyhow::Error> for SessionError {
    fn from(error: anyhow::Error) -> Self {
        Self(error)
    }
}

impl From<serde_json::Error> for SessionError {
    fn from(error: serde_json::Error) -> Self {
        Self(error.into())
    }
}

impl From<rusqlite::Error> for SessionError {
    fn from(error: rusqlite::Error) -> Self {
        Self(error.into())
    }
}

impl TelegramSession {
    /// Opens (or creates) the session file and loads what was stored.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            crate::paths::create_private_dir(parent)?;
        }
        let connection = Connection::open(path).with_context(|| {
            format!("Could not open the Telegram session at {}", path.display())
        })?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS options (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS peers (id TEXT PRIMARY KEY, info TEXT NOT NULL);",
        )?;
        let mut data = SessionData::default();
        if let Some(home_dc) = load_option(&connection, "home_dc")? {
            data.home_dc = home_dc;
        }
        if let Some(options) = load_option::<Vec<DcOption>>(&connection, "dc_options")? {
            data.dc_options = options
                .into_iter()
                .map(|option| (option.id, option))
                .collect();
        }
        if let Some(state) = load_option(&connection, "updates_state")? {
            data.updates_state = state;
        }
        {
            let mut statement = connection.prepare("SELECT id, info FROM peers")?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (id, info) = row?;
                let peer: PeerId = serde_json::from_str(&id).with_context(|| {
                    format!(
                        "The Telegram session at {} has an unreadable peer id",
                        path.display()
                    )
                })?;
                let info: PeerInfo = serde_json::from_str(&info).with_context(|| {
                    format!(
                        "The Telegram session at {} has an unreadable cached peer",
                        path.display()
                    )
                })?;
                data.peer_infos.insert(peer, info);
            }
        }
        Ok(Self {
            path: path.to_owned(),
            inner: Mutex::new(Inner { connection, data }),
        })
    }

    /// The file backing the session.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn lock(&self) -> Result<MutexGuard<'_, Inner>> {
        self.inner
            .lock()
            .map_err(|_| anyhow!("The Telegram session lock was poisoned"))
    }

    fn store_option<T: serde::Serialize>(inner: &Inner, key: &str, value: &T) -> Result<()> {
        let value = serde_json::to_string(value)?;
        inner.connection.execute(
            "INSERT INTO options (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

fn load_option<T: serde::de::DeserializeOwned>(
    connection: &Connection,
    key: &str,
) -> Result<Option<T>> {
    let value: Option<String> = connection
        .query_row(
            "SELECT value FROM options WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()?;
    match value {
        Some(value) => Ok(Some(serde_json::from_str(&value)?)),
        None => Ok(None),
    }
}

impl Session for TelegramSession {
    type Error = SessionError;

    fn home_dc_id(&self) -> Result<i32, SessionError> {
        Ok(self.lock()?.data.home_dc)
    }

    fn set_home_dc_id(&self, dc_id: i32) -> BoxFuture<'_, Result<(), SessionError>> {
        Box::pin(async move {
            let mut inner = self.lock()?;
            inner.data.home_dc = dc_id;
            Ok(Self::store_option(&inner, "home_dc", &dc_id)?)
        })
    }

    fn dc_option(&self, dc_id: i32) -> Result<Option<DcOption>, SessionError> {
        Ok(self.lock()?.data.dc_options.get(&dc_id).cloned())
    }

    fn set_dc_option(&self, dc_option: &DcOption) -> BoxFuture<'_, Result<(), SessionError>> {
        let dc_option = dc_option.clone();
        Box::pin(async move {
            let mut inner = self.lock()?;
            inner
                .data
                .dc_options
                .insert(dc_option.id, dc_option.clone());
            let options: Vec<&DcOption> = inner.data.dc_options.values().collect();
            Ok(Self::store_option(&inner, "dc_options", &options)?)
        })
    }

    fn peer(&self, peer: PeerId) -> BoxFuture<'_, Result<Option<PeerInfo>, SessionError>> {
        Box::pin(async move { Ok(self.lock()?.data.peer_infos.get(&peer).cloned()) })
    }

    fn cache_peer(&self, peer: PeerInfo) -> BoxFuture<'_, Result<(), SessionError>> {
        Box::pin(async move {
            let mut inner = self.lock()?;
            let id = peer.id();
            inner.data.peer_infos.insert(id, peer.clone());
            let id = serde_json::to_string(&id)?;
            let info = serde_json::to_string(&peer)?;
            inner.connection.execute(
                "INSERT INTO peers (id, info) VALUES (?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET info = excluded.info",
                params![id, info],
            )?;
            Ok(())
        })
    }

    fn updates_state(&self) -> BoxFuture<'_, Result<UpdatesState, SessionError>> {
        Box::pin(async move { Ok(self.lock()?.data.updates_state.clone()) })
    }

    fn set_update_state(&self, update: UpdateState) -> BoxFuture<'_, Result<(), SessionError>> {
        Box::pin(async move {
            let mut inner = self.lock()?;
            match update {
                UpdateState::All(state) => inner.data.updates_state = state,
                UpdateState::Primary { pts, date, seq } => {
                    inner.data.updates_state.pts = pts;
                    inner.data.updates_state.date = date;
                    inner.data.updates_state.seq = seq;
                }
                UpdateState::Secondary { qts } => inner.data.updates_state.qts = qts,
                UpdateState::Channel { id, pts } => {
                    match inner
                        .data
                        .updates_state
                        .channels
                        .iter_mut()
                        .find(|state| state.id == id)
                    {
                        Some(state) => state.pts = pts,
                        None => inner
                            .data
                            .updates_state
                            .channels
                            .push(ChannelState { id, pts }),
                    }
                }
            }
            let state = inner.data.updates_state.clone();
            Ok(Self::store_option(&inner, "updates_state", &state)?)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opened() -> (tempfile::TempDir, TelegramSession) {
        let directory = tempfile::tempdir().unwrap();
        let session = TelegramSession::open(&directory.path().join("grammers/session.db")).unwrap();
        (directory, session)
    }

    #[tokio::test]
    async fn a_new_session_starts_from_the_known_datacenters() {
        let (_directory, session) = opened();
        let defaults = SessionData::default();
        assert_eq!(session.home_dc_id().unwrap(), defaults.home_dc);
        for dc_id in defaults.dc_options.keys() {
            assert!(session.dc_option(*dc_id).unwrap().is_some());
        }
        assert!(session.peer(PeerId::self_user()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn what_is_cached_comes_back_after_reopening() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("grammers/session.db");
        let session = TelegramSession::open(&path).unwrap();
        let self_user = PeerInfo::User {
            id: 7,
            auth: None,
            bot: None,
            is_self: Some(true),
        };
        session.cache_peer(self_user.clone()).await.unwrap();
        session.set_home_dc_id(4).await.unwrap();
        session
            .set_update_state(UpdateState::Primary {
                pts: 11,
                date: 22,
                seq: 33,
            })
            .await
            .unwrap();
        session
            .set_update_state(UpdateState::Secondary { qts: 44 })
            .await
            .unwrap();
        session
            .set_update_state(UpdateState::Channel { id: 55, pts: 66 })
            .await
            .unwrap();
        drop(session);

        let reopened = TelegramSession::open(&path).unwrap();
        assert_eq!(reopened.home_dc_id().unwrap(), 4);
        assert_eq!(
            reopened.peer(PeerId::user(7).unwrap()).await.unwrap(),
            Some(self_user)
        );
        let state = reopened.updates_state().await.unwrap();
        assert_eq!(
            (state.pts, state.date, state.seq, state.qts),
            (11, 22, 33, 44)
        );
        assert_eq!(state.channels, [ChannelState { id: 55, pts: 66 }]);
    }

    #[tokio::test]
    async fn a_datacenter_option_survives_reopening() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("grammers/session.db");
        let session = TelegramSession::open(&path).unwrap();
        let mut option = session.dc_option(2).unwrap().unwrap();
        option.auth_key = Some([7; 256]);
        session.set_dc_option(&option).await.unwrap();
        drop(session);

        let reopened = TelegramSession::open(&path).unwrap();
        assert_eq!(
            reopened.dc_option(2).unwrap().unwrap().auth_key,
            Some([7; 256])
        );
    }
}
