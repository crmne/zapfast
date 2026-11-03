//! Moving pictures for the call tiles of a demo tour.
//!
//! `ZAPFAST_DEMO_CALL_FEEDS` names H.264 MP4 files as comma-separated
//! `role=path` pairs. The roles are `peer` (the other side of a one-to-one
//! call), `self` (our own camera), and a group call's members, by their place
//! on the call's roster counted from 0 or by their chat id:
//!
//! ```text
//! ZAPFAST_DEMO_CALL_FEEDS=peer=/clips/ada.mp4,self=/clips/me.mp4,0=/clips/jonas.mp4,491702222222@s.whatsapp.net=/clips/mira.mp4
//! ```
//!
//! A clip starts where its tile first shows video, plays by the tour's own
//! clock (so a frame-by-frame capture gets the picture each frame's time asks
//! for), and loops. It reaches the window the way a call's video does, as the
//! frames `Event::CallVideo` carries. Without the variable, and for any role
//! it leaves out, the tiles show the stock stills the screenshots use.

use super::super::stock;
use crate::{app::App, video::Frames};
use egui::ColorImage;
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

/// The environment variable naming the clips.
pub const VARIABLE: &str = "ZAPFAST_DEMO_CALL_FEEDS";

/// Longest side of each picture: the peer fills the window, our own camera
/// and a group's members share it.
const PEER_SIDE: u32 = 1280;
const SELF_SIDE: u32 = 640;
const MEMBER_SIDE: u32 = 640;

/// Who a clip stands in for.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Role {
    Peer,
    Me,
    /// A group call's member by their place on the roster.
    Member(usize),
    /// A group call's member by chat id.
    Id(String),
}

impl Role {
    fn parse(name: &str) -> Self {
        match name {
            "peer" => Self::Peer,
            "self" => Self::Me,
            other => other
                .parse()
                .map(Self::Member)
                .unwrap_or_else(|_| Self::Id(other.to_owned())),
        }
    }
}

/// Reads `role=path` pairs, skipping malformed ones.
fn parse(spec: &str) -> Vec<(Role, PathBuf)> {
    spec.split(',')
        .map(str::trim)
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let parsed = pair
                .split_once('=')
                .filter(|(role, path)| !role.trim().is_empty() && !path.trim().is_empty());
            if parsed.is_none() {
                log::warn!("{VARIABLE}: expected role=path, got {pair:?}");
            }
            let (role, path) = parsed?;
            Some((Role::parse(role.trim()), PathBuf::from(path.trim())))
        })
        .collect()
}

/// A clip decoded a few frames ahead of where it is shown.
struct Clip {
    path: PathBuf,
    side: u32,
    frames: Option<Frames>,
    /// Where the pass being decoded starts, after the loops before it.
    pass: Duration,
    /// Whether the pass being decoded has given a frame yet.
    decoded: bool,
    /// The decoded frame due next, by its time in its pass.
    next: Option<(Duration, ColorImage)>,
    shown: Option<Arc<ColorImage>>,
    failed: bool,
}

impl Clip {
    fn new(path: PathBuf, side: u32) -> Self {
        Self {
            path,
            side,
            frames: None,
            pass: Duration::ZERO,
            decoded: false,
            next: None,
            shown: None,
            failed: false,
        }
    }

    /// Back to the clip's start, for the next call.
    fn rewind(&mut self) {
        *self = Self::new(std::mem::take(&mut self.path), self.side);
    }

    /// The picture showing `elapsed` into the clip, looped.
    fn at(&mut self, elapsed: Duration) -> Option<Arc<ColorImage>> {
        loop {
            if let Some((when, _)) = &self.next {
                if self.pass + *when > elapsed {
                    break;
                }
                if let Some((_, image)) = self.next.take() {
                    self.shown = Some(Arc::new(image));
                }
            }
            if self.failed {
                break;
            }
            let (path, side) = (&self.path, self.side);
            let frames = self.frames.get_or_insert_with(|| Frames::open(path, side));
            match frames.next() {
                Some(frame) => {
                    self.next = Some(frame);
                    self.decoded = true;
                }
                None => {
                    let length = frames.length.unwrap_or_default();
                    self.frames = None;
                    // A clip that gave nothing, or has no length, cannot loop.
                    if !self.decoded || length.is_zero() {
                        self.failed = true;
                        break;
                    }
                    self.pass += length;
                    self.decoded = false;
                }
            }
        }
        self.shown.clone()
    }
}

/// The pictures a demo call's tiles show, moving or still.
pub(super) struct Feeds {
    clips: HashMap<Role, Clip>,
    stills: HashMap<Role, Arc<ColorImage>>,
    /// The tour time each tile first showed video on this call.
    began: HashMap<Role, f32>,
}

impl Feeds {
    /// The clips `ZAPFAST_DEMO_CALL_FEEDS` names, if any.
    pub(super) fn from_env() -> Self {
        Self::new(&std::env::var(VARIABLE).unwrap_or_default())
    }

    fn new(spec: &str) -> Self {
        let clips = parse(spec)
            .into_iter()
            .filter_map(|(role, path)| {
                if !path.is_file() {
                    log::warn!("{VARIABLE}: no file at {}", path.display());
                    return None;
                }
                let side = match role {
                    Role::Peer => PEER_SIDE,
                    Role::Me => SELF_SIDE,
                    _ => MEMBER_SIDE,
                };
                Some((role, Clip::new(path, side)))
            })
            .collect();
        Self {
            clips,
            stills: HashMap::new(),
            began: HashMap::new(),
        }
    }

    /// Hands the call on screen its pictures for tour time `at`.
    pub(super) fn show(&mut self, app: &mut App, at: f32) {
        let Some(call) = app
            .call
            .clone()
            .filter(|call| call.video && call.phase.is_live())
        else {
            // The next call starts its clips from the beginning.
            if !self.began.is_empty() {
                self.began.clear();
                for clip in self.clips.values_mut() {
                    clip.rewind();
                }
            }
            return;
        };
        let local = call
            .camera_on
            .then(|| self.picture(&[Role::Me], at))
            .flatten();
        let remote = (call.group.is_none() && call.remote_video)
            .then(|| self.picture(&[Role::Peer], at))
            .flatten();
        app.handle_call_video(local, remote, None);
        let members = call.group.iter().flat_map(|group| &group.participants);
        for (index, person) in members.enumerate() {
            if person.joined && person.video {
                let roles = [Role::Id(person.id.clone()), Role::Member(index)];
                let picture = self.picture(&roles, at);
                app.handle_call_video(None, picture, Some(person.id.clone()));
            }
        }
    }

    /// The picture for the first of `roles` with a clip, or the still for
    /// the last of them.
    fn picture(&mut self, roles: &[Role], at: f32) -> Option<Arc<ColorImage>> {
        let role = roles
            .iter()
            .find(|role| self.clips.contains_key(*role))
            .or(roles.last())?
            .clone();
        let began = *self.began.entry(role.clone()).or_insert(at);
        let elapsed = Duration::from_secs_f32((at - began).max(0.0));
        self.clips
            .get_mut(&role)
            .and_then(|clip| clip.at(elapsed))
            .or_else(|| Some(Arc::clone(still(&mut self.stills, &role))))
    }
}

/// The stock picture standing in for `role`, as the call screenshots use.
fn still<'a>(stills: &'a mut HashMap<Role, Arc<ColorImage>>, role: &Role) -> &'a Arc<ColorImage> {
    stills.entry(role.clone()).or_insert_with(|| {
        let (photo, height) = match role {
            Role::Peer => (stock::SPACEWALK, 720),
            Role::Me => (stock::DUSK, 320),
            Role::Member(index) => (
                [stock::VENUE, stock::SPACEWALK, stock::LAUNCH, stock::DUSK][index % 4],
                360,
            ),
            Role::Id(_) => (stock::VENUE, 360),
        };
        Arc::new(super::super::stock_frame(photo, height))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_and_paths_are_read_from_pairs() {
        let pairs = parse(" peer=/a.mp4, self=/b.mp4,2=/c.mp4,4917@s.whatsapp.net=/d.mp4,junk,=x,");
        assert_eq!(
            pairs,
            vec![
                (Role::Peer, PathBuf::from("/a.mp4")),
                (Role::Me, PathBuf::from("/b.mp4")),
                (Role::Member(2), PathBuf::from("/c.mp4")),
                (
                    Role::Id("4917@s.whatsapp.net".into()),
                    PathBuf::from("/d.mp4")
                ),
            ]
        );
    }

    /// A clip moves with the tour's clock, keeps its picture between
    /// frames, and starts over past its end.
    #[test]
    fn a_clip_moves_with_the_clock_and_loops() {
        let dir = tempfile::tempdir().unwrap();
        let path = stock::save(dir.path(), "feed.mp4", stock::VIDEO);
        let mut clip = Clip::new(path, 320);
        let first = clip.at(Duration::ZERO).expect("a first frame");
        let again = clip.at(Duration::from_millis(1)).expect("the same frame");
        assert!(Arc::ptr_eq(&first, &again), "no new picture within a frame");
        let later = clip.at(Duration::from_secs(1)).expect("a later frame");
        assert!(!Arc::ptr_eq(&first, &later));
        assert_ne!(first.pixels, later.pixels, "the picture moves");
        assert!(first.size[0] <= 320 && first.size[1] <= 320);
        let length = Duration::from_secs_f64(f64::from(stock::CLIP_SECONDS));
        let looped = clip.at(length * 2 + Duration::from_millis(500));
        assert!(looped.is_some(), "still playing after two loops");
        assert!(!clip.failed);
    }

    /// Without clips, a video call shows the stills, through the same
    /// fields the call's own video fills.
    #[test]
    fn without_clips_a_video_call_shows_stills() {
        let mut app = super::super::super::tests::app();
        super::super::super::group_call_sample(&mut app, false, true);
        app.call_member_frames.clear();
        app.call_local_frame = None;
        let mut feeds = Feeds::new("");
        feeds.show(&mut app, 3.0);
        assert!(app.call_local_frame.is_some(), "our own camera");
        let sending = app.call.as_ref().unwrap().group.as_ref().unwrap();
        let cameras = sending
            .participants
            .iter()
            .filter(|person| person.joined && person.video)
            .count();
        assert!(cameras > 0);
        assert_eq!(app.call_member_frames.len(), cameras, "every camera on");
    }
}
