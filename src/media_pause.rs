//! Pauses other apps' music while ZapFast records or plays audio, and
//! resumes it afterwards.
//!
//! Callers take a [`Hold`] for as long as they need quiet. The first hold
//! pauses every player that is playing; dropping the last one resumes exactly
//! those players. Players that were already paused stay paused, and a player
//! the user resumes by hand in the meantime is simply played again, which
//! does nothing. Recording, playback and, later, calls can overlap without
//! pausing twice or resuming early.
//!
//! The work runs in order on one worker thread, so taking a hold never delays
//! the caller and a quick release followed by a new hold cannot resume music
//! after the new pause.
//!
//! Linux talks MPRIS over D-Bus directly, as VoxType does, instead of through
//! `playerctl`: `playerctl -l` hides some players that implement MPRIS, and
//! `playerctl --player <name> play` fails silently when the name has gone,
//! which left music paused. Windows uses the system media transport controls.
//! macOS has no public API for other apps' playback, so holds do nothing
//! there; ZapFast does not use the private MediaRemote framework.

use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

/// Whether this platform can pause other apps' media.
pub const SUPPORTED: bool = cfg!(any(target_os = "linux", windows));

/// Keeps other apps' media paused until dropped.
#[must_use = "media resumes as soon as the hold is dropped"]
pub struct Hold(());

/// Pauses playing media unless another hold already did.
pub fn hold() -> Hold {
    let mut shared = shared();
    if let Some(job) = shared.holds.acquire() {
        shared.send(job);
    }
    Hold(())
}

impl Drop for Hold {
    fn drop(&mut self) {
        let mut shared = shared();
        if let Some(job) = shared.holds.release() {
            shared.send(job);
        }
    }
}

/// Waits up to `timeout` for queued pauses and resumes, so music resumes
/// before the process exits.
pub fn settle(timeout: Duration) {
    let (done, finished) = mpsc::channel();
    if shared().send(Job::Settle(done)) {
        let _ = finished.recv_timeout(timeout);
    }
}

enum Job {
    Pause,
    Resume,
    Settle(mpsc::Sender<()>),
}

/// Counts holds. Only the first acquire and the last release do anything.
#[derive(Default)]
struct Holds(usize);

impl Holds {
    fn acquire(&mut self) -> Option<Job> {
        self.0 += 1;
        (self.0 == 1).then_some(Job::Pause)
    }

    fn release(&mut self) -> Option<Job> {
        self.0 = self.0.checked_sub(1)?;
        (self.0 == 0).then_some(Job::Resume)
    }
}

struct Shared {
    holds: Holds,
    worker: Option<mpsc::Sender<Job>>,
}

impl Shared {
    /// Queues a job, starting the worker on first use. Callers keep the lock
    /// while sending, so jobs arrive in the order the holds changed.
    fn send(&mut self, job: Job) -> bool {
        if self.worker.is_none() {
            let (sender, jobs) = mpsc::channel();
            let spawned = std::thread::Builder::new()
                .name("media-pause".to_owned())
                .spawn(move || work(jobs));
            match spawned {
                Ok(_) => self.worker = Some(sender),
                Err(error) => {
                    log::warn!("could not start the media pause worker: {error}");
                    return false;
                }
            }
        }
        self.worker
            .as_ref()
            .is_some_and(|worker| worker.send(job).is_ok())
    }
}

static SHARED: Mutex<Shared> = Mutex::new(Shared {
    holds: Holds(0),
    worker: None,
});

fn shared() -> MutexGuard<'static, Shared> {
    SHARED.lock().unwrap_or_else(|p| p.into_inner())
}

fn work(jobs: mpsc::Receiver<Job>) {
    let mut players = platform::Players::new();
    // Players this worker paused, to resume and nothing else.
    let mut paused: Vec<String> = Vec::new();
    for job in jobs {
        match job {
            Job::Pause => paused.extend(players.pause_playing()),
            Job::Resume => players.resume(std::mem::take(&mut paused)),
            Job::Settle(done) => {
                let _ = done.send(());
            }
        }
    }
}

/// Whether an MPRIS bus name belongs to a player worth pausing.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn pausable_player(bus_name: &str) -> bool {
    let Some(player) = bus_name.strip_prefix("org.mpris.MediaPlayer2.") else {
        return false;
    };
    // playerctld forwards to the real players, which are paused directly;
    // going through it too would pause one of them twice. ZapFast does not
    // publish MPRIS today, but must never pause itself if it starts to.
    let app = player.split('.').next().unwrap_or_default();
    !player.is_empty() && app != "playerctld" && app != "zapfast"
}

#[cfg(target_os = "linux")]
mod platform {
    use zbus::{Connection, Proxy, fdo::DBusProxy};

    const PATH: &str = "/org/mpris/MediaPlayer2";
    const INTERFACE: &str = "org.mpris.MediaPlayer2.Player";

    /// MPRIS players on the session bus.
    pub struct Players {
        runtime: Option<tokio::runtime::Runtime>,
    }

    impl Players {
        pub fn new() -> Self {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .inspect_err(|error| log::warn!("could not start the media pause runtime: {error}"))
                .ok();
            Self { runtime }
        }

        /// Pauses players that are playing and returns their bus names.
        pub fn pause_playing(&mut self) -> Vec<String> {
            let Some(runtime) = &self.runtime else {
                return Vec::new();
            };
            runtime.block_on(async {
                let connection = match Connection::session().await {
                    Ok(connection) => connection,
                    Err(error) => {
                        log::warn!("could not reach the session bus to pause media: {error}");
                        return Vec::new();
                    }
                };
                let names = match list_players(&connection).await {
                    Ok(names) => names,
                    Err(error) => {
                        log::warn!("could not list media players: {error}");
                        return Vec::new();
                    }
                };
                let mut paused = Vec::new();
                for name in names {
                    match status(&connection, &name).await {
                        Ok(status) if status == "Playing" => {
                            match call(&connection, &name, "Pause").await {
                                Ok(()) => paused.push(name),
                                Err(error) => log::warn!("could not pause {name}: {error}"),
                            }
                        }
                        Ok(_) => {}
                        Err(error) => log::debug!("could not read {name}'s status: {error}"),
                    }
                }
                if !paused.is_empty() {
                    log::info!("paused {}", paused.join(", "));
                }
                paused
            })
        }

        /// Plays the players `pause_playing` paused.
        pub fn resume(&mut self, names: Vec<String>) {
            let Some(runtime) = &self.runtime else {
                return;
            };
            if names.is_empty() {
                return;
            }
            runtime.block_on(async {
                let connection = match Connection::session().await {
                    Ok(connection) => connection,
                    Err(error) => {
                        log::warn!("could not reach the session bus to resume media: {error}");
                        return;
                    }
                };
                for name in names {
                    // A player that quit meanwhile, or whose name changed, as
                    // Chromium's per-process names do, cannot be resumed.
                    match call(&connection, &name, "Play").await {
                        Ok(()) => log::info!("resumed {name}"),
                        Err(error) => log::warn!("could not resume {name}: {error}"),
                    }
                }
            });
        }
    }

    /// Lists every MPRIS player on the bus, including ones `playerctl -l`
    /// leaves out.
    pub(super) async fn list_players(connection: &Connection) -> zbus::Result<Vec<String>> {
        let names = DBusProxy::new(connection).await?.list_names().await?;
        Ok(names
            .into_iter()
            .map(|name| name.to_string())
            .filter(|name| super::pausable_player(name))
            .collect())
    }

    async fn status(connection: &Connection, name: &str) -> zbus::Result<String> {
        let proxy = Proxy::new(connection, name, PATH, INTERFACE).await?;
        proxy.get_property::<String>("PlaybackStatus").await
    }

    async fn call(connection: &Connection, name: &str, method: &'static str) -> zbus::Result<()> {
        let proxy = Proxy::new(connection, name, PATH, INTERFACE).await?;
        proxy.call::<_, _, ()>(method, &()).await
    }
}

#[cfg(windows)]
mod platform {
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSession as Session,
        GlobalSystemMediaTransportControlsSessionManager as Manager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
    };

    /// Media sessions in the system media transport controls.
    pub struct Players;

    impl Players {
        pub fn new() -> Self {
            Self
        }

        /// Pauses sessions that are playing and returns their app ids.
        pub fn pause_playing(&mut self) -> Vec<String> {
            let sessions = match sessions() {
                Ok(sessions) => sessions,
                Err(error) => {
                    log::warn!("could not list media sessions: {error}");
                    return Vec::new();
                }
            };
            let mut paused = Vec::new();
            for session in sessions {
                let playing = session
                    .GetPlaybackInfo()
                    .and_then(|info| info.PlaybackStatus())
                    .is_ok_and(|status| status == Status::Playing);
                if !playing {
                    continue;
                }
                let Ok(app) = session.SourceAppUserModelId().map(|id| id.to_string()) else {
                    continue;
                };
                match session.TryPauseAsync().and_then(|pause| pause.join()) {
                    Ok(true) => paused.push(app),
                    Ok(false) => log::warn!("{app} refused to pause"),
                    Err(error) => log::warn!("could not pause {app}: {error}"),
                }
            }
            if !paused.is_empty() {
                log::info!("paused {}", paused.join(", "));
            }
            paused
        }

        /// Plays the sessions `pause_playing` paused.
        pub fn resume(&mut self, apps: Vec<String>) {
            if apps.is_empty() {
                return;
            }
            let sessions = match sessions() {
                Ok(sessions) => sessions,
                Err(error) => {
                    log::warn!("could not list media sessions to resume: {error}");
                    return;
                }
            };
            for session in sessions {
                let Ok(app) = session.SourceAppUserModelId().map(|id| id.to_string()) else {
                    continue;
                };
                if !apps.contains(&app) {
                    continue;
                }
                match session.TryPlayAsync().and_then(|play| play.join()) {
                    Ok(true) => log::info!("resumed {app}"),
                    Ok(false) => log::warn!("{app} refused to resume"),
                    Err(error) => log::warn!("could not resume {app}: {error}"),
                }
            }
        }
    }

    fn sessions() -> windows::core::Result<Vec<Session>> {
        let sessions = Manager::RequestAsync()?.join()?.GetSessions()?;
        (0..sessions.Size()?)
            .map(|index| sessions.GetAt(index))
            .collect()
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    /// macOS offers no public API to pause or resume other apps' media.
    pub struct Players;

    impl Players {
        pub fn new() -> Self {
            Self
        }

        pub fn pause_playing(&mut self) -> Vec<String> {
            Vec::new()
        }

        pub fn resume(&mut self, _apps: Vec<String>) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pauses(job: Option<Job>) -> bool {
        matches!(job, Some(Job::Pause))
    }

    fn resumes(job: Option<Job>) -> bool {
        matches!(job, Some(Job::Resume))
    }

    #[test]
    fn only_the_first_hold_pauses_and_only_the_last_resumes() {
        let mut holds = Holds::default();
        assert!(pauses(holds.acquire()));
        assert!(
            holds.acquire().is_none(),
            "recording while a voice message plays"
        );
        assert!(
            holds.release().is_none(),
            "the other hold still wants quiet"
        );
        assert!(resumes(holds.release()));
        assert!(pauses(holds.acquire()), "the next recording pauses again");
    }

    #[test]
    fn an_extra_release_does_not_resume_again() {
        let mut holds = Holds::default();
        assert!(holds.release().is_none());
        assert!(pauses(holds.acquire()));
        assert!(resumes(holds.release()));
        assert!(holds.release().is_none());
    }

    #[test]
    fn players_are_mpris_names_except_the_aggregator_and_ourselves() {
        assert!(pausable_player("org.mpris.MediaPlayer2.spotify"));
        assert!(pausable_player("org.mpris.MediaPlayer2.cliamp"));
        assert!(pausable_player(
            "org.mpris.MediaPlayer2.chromium.instance1872063"
        ));
        assert!(pausable_player("org.mpris.MediaPlayer2.playerctldish"));
        assert!(!pausable_player("org.mpris.MediaPlayer2.playerctld"));
        assert!(!pausable_player("org.mpris.MediaPlayer2.zapfast"));
        assert!(!pausable_player(
            "org.mpris.MediaPlayer2.zapfast.instance42"
        ));
        assert!(!pausable_player("org.mpris.MediaPlayer2."));
        assert!(!pausable_player("org.freedesktop.Notifications"));
        assert!(!pausable_player(":1.42"));
    }

    /// Pauses and resumes fake players on a private bus:
    /// `dbus-run-session -- cargo test media_pause -- --ignored`.
    /// Refuses to run when any other player is on the bus, so it cannot
    /// pause real music.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs a private D-Bus session"]
    fn pauses_and_resumes_a_player_on_a_private_bus() {
        use super::platform::list_players;
        use std::sync::{Arc, Mutex};
        use zbus::interface;

        struct Fake(Arc<Mutex<Vec<&'static str>>>, &'static str);

        #[interface(name = "org.mpris.MediaPlayer2.Player")]
        impl Fake {
            #[zbus(property)]
            fn playback_status(&self) -> String {
                self.1.to_owned()
            }
            fn pause(&self) {
                self.0.lock().unwrap().push("Pause");
            }
            fn play(&self) {
                self.0.lock().unwrap().push("Play");
            }
        }

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let playing: Arc<Mutex<Vec<&str>>> = Default::default();
        let stopped: Arc<Mutex<Vec<&str>>> = Default::default();
        let connections = runtime.block_on(async {
            let serve = |name: &'static str, fake: Fake| async move {
                zbus::connection::Builder::session()
                    .unwrap()
                    .name(name)
                    .unwrap()
                    .serve_at("/org/mpris/MediaPlayer2", fake)
                    .unwrap()
                    .build()
                    .await
                    .unwrap()
            };
            let music = serve(
                "org.mpris.MediaPlayer2.music",
                Fake(Arc::clone(&playing), "Playing"),
            )
            .await;
            let podcast = serve(
                "org.mpris.MediaPlayer2.podcast",
                Fake(Arc::clone(&stopped), "Paused"),
            )
            .await;
            let mut players = list_players(&music).await.unwrap();
            players.sort();
            assert_eq!(
                players,
                [
                    "org.mpris.MediaPlayer2.music",
                    "org.mpris.MediaPlayer2.podcast"
                ],
                "run under dbus-run-session, away from real players"
            );
            (music, podcast)
        });
        // The fake players answer from the runtime's thread.
        let (quit, quitting) = tokio::sync::oneshot::channel::<()>();
        let serving = std::thread::spawn(move || {
            runtime.block_on(async {
                let _ = quitting.await;
            });
            drop(connections);
        });

        let first = hold();
        let second = hold();
        drop(first);
        settle(Duration::from_secs(5));
        assert_eq!(*playing.lock().unwrap(), ["Pause"]);
        drop(second);
        settle(Duration::from_secs(5));
        assert_eq!(*playing.lock().unwrap(), ["Pause", "Play"]);
        assert!(
            stopped.lock().unwrap().is_empty(),
            "a paused player stays paused"
        );

        let _ = quit.send(());
        serving.join().unwrap();
    }
}
