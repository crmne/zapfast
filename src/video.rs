//! In-app playback for video messages and round video notes.
//!
//! One video plays at a time. A thread demuxes the MP4 with `mp4`, decodes
//! its H.264 track with `openh264` (as `animation` does for GIFs), and hands
//! over scaled RGBA frames tagged with their presentation time. The interface
//! thread shows the newest frame that is due and uploads it into a single
//! texture. Sound plays through rodio, whose symphonia backend decodes the
//! AAC track, and its position steers the clock while it lasts. Other codecs
//! are reported so the video can open in the system player instead.

use std::cell::Cell;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError};
use std::time::{Duration, Instant};

use egui::{ColorImage, TextureHandle, TextureOptions};
use rodio::Source;

use crate::backend::Waker;

/// Longest side of a decoded frame in pixels: about twice the widest bubble.
const MAX_SIDE: u32 = 720;
/// Frames decoded ahead of the clock.
const AHEAD: usize = 4;
/// How far the clock may drift from the sound before it follows it.
const DRIFT: Duration = Duration::from_millis(60);
/// How long a playing video may stay off screen before it pauses.
const UNSEEN: Duration = Duration::from_millis(1500);
/// Longest wait between repaints while a video plays, for the progress.
const TICK: Duration = Duration::from_millis(40);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Waiting for the first frame after starting or seeking.
    Loading,
    Playing,
    Paused,
}

/// Playback state of the video being shown.
#[derive(Clone)]
pub struct Status {
    pub state: State,
    pub position: Duration,
    /// Length of the video, or zero until the file has been read.
    pub total: Duration,
    /// The frame to draw, once one has been decoded.
    pub frame: Option<TextureHandle>,
}

impl Status {
    /// Played fraction, from 0 to 1.
    pub fn fraction(&self) -> f32 {
        if self.total.is_zero() {
            0.0
        } else {
            (self.position.as_secs_f32() / self.total.as_secs_f32()).clamp(0.0, 1.0)
        }
    }
}

/// Something the app has to act on after [`Player::poll`].
#[derive(Debug, PartialEq, Eq)]
pub enum Notice {
    /// ZapFast cannot decode this video, so it should open in the system player.
    Unsupported(PathBuf),
}

/// What the decoder thread hands to the interface thread.
enum Delivery {
    /// The movie's length, sent before any frame.
    Length(Duration),
    Frame(Duration, ColorImage),
    /// Every frame has been sent.
    End,
    /// The file is not an H.264 MP4 this player can read.
    Unsupported(String),
}

/// Playback time: wall time while no sound steers it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Clock {
    /// Position when the clock last stopped or jumped.
    anchor: Duration,
    /// When it started running from `anchor`, while it runs.
    since: Option<Instant>,
}

impl Clock {
    pub fn position(&self, now: Instant) -> Duration {
        self.anchor
            + self
                .since
                .map_or(Duration::ZERO, |since| now.saturating_duration_since(since))
    }

    pub fn running(&self) -> bool {
        self.since.is_some()
    }

    pub fn pause(&mut self, now: Instant) {
        self.anchor = self.position(now);
        self.since = None;
    }

    pub fn resume(&mut self, now: Instant) {
        if self.since.is_none() {
            self.since = Some(now);
        }
    }

    /// Jumps to `to`, keeping the clock running or stopped.
    pub fn seek(&mut self, to: Duration, now: Instant) {
        self.anchor = to;
        if self.since.is_some() {
            self.since = Some(now);
        }
    }

    /// Follows the sound's position once it drifts further than 60 ms.
    /// Small differences are left alone: the sound's position advances in
    /// device-sized steps and would make the picture stutter.
    pub fn follow(&mut self, sound: Duration, now: Instant) {
        if sound.abs_diff(self.position(now)) > DRIFT {
            self.seek(sound, now);
        }
    }
}

/// Removes every frame that is due at `position` and returns the newest of
/// them. Frames that were late are skipped rather than shown in a burst.
pub fn take_due<T>(queue: &mut VecDeque<(Duration, T)>, position: Duration) -> Option<T> {
    let mut due = None;
    while queue.front().is_some_and(|(at, _)| *at <= position) {
        due = queue.pop_front().map(|(_, frame)| frame);
    }
    due
}

/// The part of a `width` by `height` picture that fills a circle: its
/// centred square, in texture coordinates.
pub fn square_uv(width: f32, height: f32) -> egui::Rect {
    if width <= 0.0 || height <= 0.0 {
        return egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
    }
    let side = width.min(height);
    let (u, v) = (side / width, side / height);
    egui::Rect::from_min_max(
        egui::pos2((1.0 - u) / 2.0, (1.0 - v) / 2.0),
        egui::pos2((1.0 + u) / 2.0, (1.0 + v) / 2.0),
    )
}

/// Points along a circle from twelve o'clock clockwise, covering `fraction`
/// of it: the progress ring around a video note.
pub fn arc(center: egui::Pos2, radius: f32, fraction: f32) -> Vec<egui::Pos2> {
    let fraction = fraction.clamp(0.0, 1.0);
    // About one point every three degrees.
    let steps = ((fraction * 120.0).ceil() as usize).max(1);
    (0..=steps)
        .map(|step| {
            let angle = std::f32::consts::TAU * fraction * step as f32 / steps as f32;
            egui::pos2(
                center.x + radius * angle.sin(),
                center.y - radius * angle.cos(),
            )
        })
        .collect()
}

/// The sound of the playing video on the default output device.
struct Sound {
    device: rodio::MixerDeviceSink,
    sink: rodio::Player,
    /// Video time the queued decoder started from.
    base: Duration,
}

impl Sound {
    /// Opens the file's sound, paused at `from`. A video without a sound
    /// track, or a computer without an output device, plays silently.
    fn open(path: &Path, from: Duration, muted: bool) -> Option<Self> {
        let decoder = sound_decoder(path, from)?;
        let device = match rodio::DeviceSinkBuilder::open_default_sink() {
            Ok(device) => device,
            Err(error) => {
                log::warn!("video plays without sound: {error}");
                return None;
            }
        };
        let sink = rodio::Player::connect_new(device.mixer());
        sink.pause();
        sink.set_volume(if muted { 0.0 } else { 1.0 });
        sink.append(decoder);
        Some(Self {
            device,
            sink,
            base: from,
        })
    }

    /// Queues the sound again from `from`, paused. A fresh player keeps the
    /// seek off the audio thread, which `Player::try_seek` would wait for.
    fn restart(&mut self, path: &Path, from: Duration, muted: bool) {
        let sink = rodio::Player::connect_new(self.device.mixer());
        sink.pause();
        sink.set_volume(if muted { 0.0 } else { 1.0 });
        if let Some(decoder) = sound_decoder(path, from) {
            sink.append(decoder);
        }
        self.sink = sink;
        self.base = from;
    }

    /// Position of the sound in the video, while there is sound left.
    fn position(&self) -> Option<Duration> {
        (!self.sink.empty()).then(|| self.base + self.sink.get_pos())
    }
}

fn sound_decoder(
    path: &Path,
    from: Duration,
) -> Option<rodio::Decoder<std::io::BufReader<std::fs::File>>> {
    let file = std::fs::File::open(path).ok()?;
    // Fails for a video without a sound track: the MP4's H.264 track has no
    // codec symphonia knows, so there is nothing to pick.
    let mut decoder = rodio::Decoder::try_from(file).ok()?;
    if !from.is_zero() {
        decoder.try_seek(from).ok()?;
    }
    Some(decoder)
}

struct Session {
    message: String,
    path: PathBuf,
    state: State,
    /// Whether playback starts once the first frame arrives.
    resume: bool,
    frames: Receiver<Delivery>,
    queue: VecDeque<(Duration, ColorImage)>,
    /// Every frame of the current decode has arrived.
    decoded: bool,
    texture: Option<TextureHandle>,
    clock: Clock,
    sound: Option<Sound>,
    total: Duration,
}

impl Session {
    fn pause(&mut self, now: Instant) {
        match self.state {
            State::Loading => self.resume = false,
            State::Playing => {
                self.state = State::Paused;
                self.clock.pause(now);
                if let Some(sound) = &self.sound {
                    sound.sink.pause();
                }
            }
            State::Paused => {}
        }
    }

    fn play(&mut self, now: Instant) {
        match self.state {
            State::Loading => self.resume = true,
            State::Paused => {
                self.state = State::Playing;
                self.clock.resume(now);
                if let Some(sound) = &self.sound {
                    sound.sink.play();
                }
            }
            State::Playing => {}
        }
    }

    fn show(&mut self, ctx: &egui::Context, image: ColorImage) {
        match &mut self.texture {
            Some(texture) => texture.set(image, TextureOptions::LINEAR),
            None => {
                self.texture = Some(ctx.load_texture("video-frame", image, TextureOptions::LINEAR))
            }
        }
    }
}

/// Plays one video at a time inside its message.
pub struct Player {
    waker: Waker,
    session: Option<Session>,
    muted: bool,
    /// Whether videos open the sound device. Tests and demo screenshots
    /// play silently.
    audible: bool,
    /// When the playing video's message was last drawn on screen.
    seen: Cell<Instant>,
}

impl Player {
    pub fn new(waker: Waker) -> Self {
        Self {
            waker,
            session: None,
            muted: false,
            audible: true,
            seen: Cell::new(Instant::now()),
        }
    }

    /// Plays videos without opening the sound device.
    #[cfg(any(test, feature = "demo"))]
    pub fn silence(&mut self) {
        self.audible = false;
    }

    /// Plays or pauses a message's video, starting it when another one (or
    /// none) is loaded.
    pub fn toggle(&mut self, message: &str, path: &Path) {
        let now = Instant::now();
        match self.session.as_mut() {
            Some(session) if session.message == message && session.path == path => {
                let playing = match session.state {
                    State::Loading => session.resume,
                    State::Playing => true,
                    State::Paused => false,
                };
                if playing {
                    session.pause(now);
                } else {
                    session.play(now);
                }
            }
            _ => self.start(message, path, Duration::ZERO),
        }
    }

    /// Jumps to a fraction from 0 to 1 of the playing video, keeping it
    /// playing or paused.
    pub fn seek(&mut self, message: &str, fraction: f32) {
        let muted = self.muted;
        let Some(session) = self
            .session
            .as_mut()
            .filter(|session| session.message == message && !session.total.is_zero())
        else {
            return;
        };
        let now = Instant::now();
        let to = session.total.mul_f32(fraction.clamp(0.0, 1.0));
        session.resume = match session.state {
            State::Loading => session.resume,
            State::Playing => true,
            State::Paused => false,
        };
        session.state = State::Loading;
        session.clock.pause(now);
        session.clock.seek(to, now);
        session.queue.clear();
        session.decoded = false;
        session.frames = spawn_decoder(&session.path, to, self.waker.clone());
        if let Some(sound) = &mut session.sound {
            sound.restart(&session.path, to, muted);
        }
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        if let Some(sound) = self
            .session
            .as_ref()
            .and_then(|session| session.sound.as_ref())
        {
            sound.sink.set_volume(if self.muted { 0.0 } else { 1.0 });
        }
    }

    /// Stops playback and releases the decoder, the texture, and the device.
    pub fn stop(&mut self) {
        self.session = None;
    }

    /// Whether a video is playing or about to.
    pub fn is_active(&self) -> bool {
        self.session.as_ref().is_some_and(|session| {
            session.state == State::Playing || (session.state == State::Loading && session.resume)
        })
    }

    /// The message whose video is loaded, playing or not.
    pub fn message(&self) -> Option<&str> {
        self.session
            .as_ref()
            .map(|session| session.message.as_str())
    }

    /// Playback state for `message`, if its video is the one loaded.
    pub fn status(&self, message: &str) -> Option<Status> {
        let session = self
            .session
            .as_ref()
            .filter(|session| session.message == message)?;
        let position = session.clock.position(Instant::now());
        Some(Status {
            state: session.state,
            position: if session.total.is_zero() {
                position
            } else {
                position.min(session.total)
            },
            total: session.total,
            frame: session.texture.clone(),
        })
    }

    /// Records that `message` was drawn on screen. A playing video that
    /// goes unseen for a while pauses, such as after scrolling far away.
    pub fn saw(&self, message: &str) {
        if self.message() == Some(message) {
            self.seen.set(Instant::now());
        }
    }

    fn start(&mut self, message: &str, path: &Path, from: Duration) {
        let now = Instant::now();
        self.seen.set(now);
        let mut clock = Clock::default();
        clock.seek(from, now);
        self.session = Some(Session {
            message: message.to_owned(),
            path: path.to_owned(),
            state: State::Loading,
            resume: true,
            frames: spawn_decoder(path, from, self.waker.clone()),
            queue: VecDeque::new(),
            decoded: false,
            texture: None,
            clock,
            sound: self
                .audible
                .then(|| Sound::open(path, from, self.muted))
                .flatten(),
            total: Duration::ZERO,
        });
    }

    /// Takes decoded frames, shows the one that is due, and schedules the
    /// next repaint while the video plays.
    pub fn poll(&mut self, ctx: &egui::Context) -> Option<Notice> {
        let session = self.session.as_mut()?;
        let now = Instant::now();
        if now.saturating_duration_since(self.seen.get()) > UNSEEN {
            session.pause(now);
        }
        while session.queue.len() < AHEAD {
            match session.frames.try_recv() {
                Ok(Delivery::Length(total)) => session.total = total,
                Ok(Delivery::Frame(at, image)) => session.queue.push_back((at, image)),
                Ok(Delivery::End) | Err(TryRecvError::Disconnected) => {
                    session.decoded = true;
                    break;
                }
                Ok(Delivery::Unsupported(reason)) => {
                    log::info!("video opens in the system player: {reason}");
                    let path = session.path.clone();
                    self.session = None;
                    return Some(Notice::Unsupported(path));
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        if session.state == State::Loading {
            if let Some((_, image)) = session.queue.pop_front() {
                session.show(ctx, image);
                session.state = State::Paused;
                if session.resume {
                    session.play(now);
                }
            } else if session.decoded {
                // Not one frame decoded: hand the file to the system player.
                let path = session.path.clone();
                self.session = None;
                return Some(Notice::Unsupported(path));
            } else {
                return None;
            }
        }
        if session.state != State::Playing {
            return None;
        }
        let sound = session.sound.as_ref().and_then(Sound::position);
        if let Some(sound) = sound {
            session.clock.follow(sound, now);
        }
        let position = session.clock.position(now);
        if let Some(image) = take_due(&mut session.queue, position) {
            session.show(ctx, image);
        }
        let finished = session.decoded
            && session.queue.is_empty()
            && sound.is_none()
            && position >= session.total;
        if finished {
            // Back to the poster, ready to play again.
            self.session = None;
            ctx.request_repaint();
            return None;
        }
        let next = session
            .queue
            .front()
            .map_or(TICK, |(at, _)| at.saturating_sub(position));
        ctx.request_repaint_after(next.clamp(Duration::from_millis(4), TICK));
        None
    }
}

/// Starts decoding `path` from `from` on its own thread. Dropping the
/// receiver stops the thread at its next frame.
fn spawn_decoder(path: &Path, from: Duration, waker: Waker) -> Receiver<Delivery> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(AHEAD);
    let path = path.to_owned();
    let spawned = std::thread::Builder::new()
        .name("video-decode".into())
        .spawn(move || {
            // A decoder panic reports the video as unsupported.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                decode(&path, from, &sender, &waker)
            }))
            .unwrap_or_else(|_| Err("the decoder stopped".to_owned()));
            let last = match outcome {
                Ok(()) => Delivery::End,
                Err(reason) => Delivery::Unsupported(reason),
            };
            let _ = sender.send(last);
            waker.wake();
        });
    if let Err(error) = spawned {
        log::warn!("could not start the video decoder: {error}");
    }
    receiver
}

/// Sample timing of a video track, in the track's own time units.
#[derive(Debug, Default)]
struct Timeline {
    timescale: u64,
    /// Decode time of each sample, in sample order.
    starts: Vec<u64>,
    /// Sample numbers, from 1, that decode on their own. Empty when all do.
    sync: Vec<u32>,
    /// Media time the edit list starts showing from.
    shift: u64,
}

impl Timeline {
    fn of(track: &mp4::Mp4Track) -> Self {
        let stbl = &track.trak.mdia.minf.stbl;
        let limit = track.sample_count() as usize;
        let mut starts = Vec::with_capacity(limit);
        let mut time = 0u64;
        'entries: for entry in &stbl.stts.entries {
            for _ in 0..entry.sample_count {
                if starts.len() >= limit {
                    break 'entries;
                }
                starts.push(time);
                time += u64::from(entry.sample_delta);
            }
        }
        let shift = track
            .trak
            .edts
            .as_ref()
            .and_then(|edts| edts.elst.as_ref())
            .and_then(|elst| {
                // An empty edit, a delay before the track starts, has a
                // media time of -1.
                elst.entries
                    .iter()
                    .map(|entry| entry.media_time)
                    .find(|&time| time != u64::from(u32::MAX) && time != u64::MAX)
            })
            .unwrap_or(0);
        Self {
            timescale: u64::from(track.timescale().max(1)),
            starts,
            sync: stbl
                .stss
                .as_ref()
                .map(|stss| stss.entries.clone())
                .unwrap_or_default(),
            shift,
        }
    }

    fn ticks(&self, time: Duration) -> u64 {
        (time.as_nanos() * u128::from(self.timescale) / 1_000_000_000) as u64 + self.shift
    }

    /// Presentation time of a sample's composition time, in video time.
    fn time(&self, ticks: i64) -> Duration {
        let ticks = (ticks - self.shift as i64).max(0) as u64;
        Duration::from_nanos(
            (u128::from(ticks) * 1_000_000_000 / u128::from(self.timescale)) as u64,
        )
    }

    /// The sample to start decoding from to show `time`: the last keyframe
    /// that decodes at or before it.
    fn first_sample(&self, time: Duration) -> u32 {
        let target = self.ticks(time);
        // Samples are numbered from 1.
        let wanted = self.starts.partition_point(|&start| start <= target).max(1) as u32;
        if self.sync.is_empty() {
            return wanted;
        }
        self.sync
            .iter()
            .copied()
            .filter(|&sample| sample <= wanted)
            .max()
            .unwrap_or(1)
    }
}

/// Presentation times of samples sent to the decoder, handed back in
/// display order: H.264 reorders frames, so the decoder returns the
/// earliest pending time next.
#[derive(Default)]
struct Reorder(BinaryHeap<Reverse<i64>>);

impl Reorder {
    fn push(&mut self, time: i64) {
        self.0.push(Reverse(time));
    }

    fn pop(&mut self) -> Option<i64> {
        self.0.pop().map(|Reverse(time)| time)
    }
}

/// How many quarter turns clockwise a track's matrix rotates the picture.
/// The matrix is `[a b; c d]` in 16.16 fixed point.
fn quarter_turns(a: i32, b: i32, c: i32, d: i32) -> u8 {
    match (a.signum(), b.signum(), c.signum(), d.signum()) {
        (0, 1, -1, 0) => 1,
        (-1, 0, 0, -1) => 2,
        (0, -1, 1, 0) => 3,
        _ => 0,
    }
}

/// Size that fits `width` by `height` within `max_side` on its longest side.
fn fitted(width: u32, height: u32, max_side: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= max_side || longest == 0 {
        return (width, height);
    }
    (
        (u64::from(width) * u64::from(max_side) / u64::from(longest)).max(1) as u32,
        (u64::from(height) * u64::from(max_side) / u64::from(longest)).max(1) as u32,
    )
}

/// Decodes `path` from `from` and sends its frames until they run out or
/// the receiver goes away. Errors name why the file cannot play here.
fn decode(
    path: &Path,
    from: Duration,
    frames: &SyncSender<Delivery>,
    waker: &Waker,
) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let size = file.metadata().map_err(|error| error.to_string())?.len();
    let mut mp4 = mp4::Mp4Reader::read_header(std::io::BufReader::new(file), size)
        .map_err(|error| format!("not a readable MP4: {error}"))?;
    let (track_id, count, timeline, sps, pps, turns) = {
        let track = mp4
            .tracks()
            .values()
            .find(|track| track.track_type().ok() == Some(mp4::TrackType::Video))
            .ok_or("no video track")?;
        if track.media_type().ok() != Some(mp4::MediaType::H264) {
            return Err("not H.264".to_owned());
        }
        (
            track.track_id(),
            track.sample_count(),
            Timeline::of(track),
            track
                .sequence_parameter_set()
                .map_err(|error| error.to_string())?
                .to_vec(),
            track
                .picture_parameter_set()
                .map_err(|error| error.to_string())?
                .to_vec(),
            {
                let matrix = &track.trak.tkhd.matrix;
                quarter_turns(matrix.a, matrix.b, matrix.c, matrix.d)
            },
        )
    };
    if frames.send(Delivery::Length(mp4.duration())).is_err() {
        return Ok(());
    }
    waker.wake();
    // Flushing after every packet, the default, breaks streams with
    // B-frames: the decoder then gives up at the first one. Reordered frames
    // come out on their own and the rest with `flush_remaining`.
    let mut decoder = openh264::decoder::Decoder::with_api_config(
        openh264::OpenH264API::from_source(),
        openh264::decoder::DecoderConfig::new()
            .flush_after_decode(openh264::decoder::Flush::NoFlush),
    )
    .map_err(|error| error.to_string())?;
    let mut parameters = Vec::new();
    crate::animation::push_annex_b(&mut parameters, &sps);
    crate::animation::push_annex_b(&mut parameters, &pps);
    let _ = decoder.decode(&parameters);
    let mut order = Reorder::default();
    let mut output = Output {
        from,
        turns,
        held: None,
        frames,
        waker,
    };
    for sample_id in timeline.first_sample(from)..=count {
        let Ok(Some(sample)) = mp4.read_sample(track_id, sample_id) else {
            break;
        };
        order.push(sample.start_time as i64 + i64::from(sample.rendering_offset));
        let mut annex_b = Vec::with_capacity(sample.bytes.len() + 16);
        crate::animation::avcc_to_annex_b(&mut annex_b, &sample.bytes);
        if let Ok(Some(yuv)) = decoder.decode(&annex_b) {
            let at = timeline.time(order.pop().unwrap_or_default());
            if !output.frame(at, &yuv) {
                return Ok(());
            }
        }
    }
    let rest = decoder
        .flush_remaining()
        .map_err(|error| error.to_string())?;
    for yuv in &rest {
        let at = timeline.time(order.pop().unwrap_or_default());
        if !output.frame(at, yuv) {
            return Ok(());
        }
    }
    // A seek past the last frame still shows that frame.
    if let Some(image) = output.held.take() {
        let _ = frames.send(Delivery::Frame(from, image));
    }
    Ok(())
}

/// Frames this close before a seek target are converted, so the picture
/// showing at the target is ready.
const FRAME: Duration = Duration::from_millis(100);

/// Sends decoded frames from a start time on.
struct Output<'a> {
    from: Duration,
    turns: u8,
    /// The newest frame before `from`, which shows at `from` until the next
    /// frame is due.
    held: Option<ColorImage>,
    frames: &'a SyncSender<Delivery>,
    waker: &'a Waker,
}

impl Output<'_> {
    /// Sends a frame shown at `at`. Frames before the start only prime the
    /// decoder after a seek. Returns false once nobody is listening.
    fn frame(&mut self, at: Duration, yuv: &openh264::decoder::DecodedYUV<'_>) -> bool {
        if at < self.from {
            if at + FRAME >= self.from {
                self.held = picture(yuv, self.turns);
            }
            return true;
        }
        if let Some(image) = self.held.take()
            && at > self.from
            && self.frames.send(Delivery::Frame(self.from, image)).is_err()
        {
            return false;
        }
        let Some(image) = picture(yuv, self.turns) else {
            return true;
        };
        let sent = self.frames.send(Delivery::Frame(at, image)).is_ok();
        self.waker.wake();
        sent
    }
}

/// Turns one decoded frame into an upright picture no larger than
/// [`MAX_SIDE`].
fn picture(yuv: &openh264::decoder::DecodedYUV<'_>, turns: u8) -> Option<ColorImage> {
    use openh264::formats::YUVSource;

    let (width, height) = yuv.dimensions();
    if width == 0 || height == 0 {
        return None;
    }
    let (out_width, out_height) = fitted(width as u32, height as u32, MAX_SIDE);
    let planes = Planes {
        y: yuv.y(),
        u: yuv.u(),
        v: yuv.v(),
        strides: yuv.strides(),
        size: (width, height),
    };
    Some(convert(
        &planes,
        (out_width as usize, out_height as usize),
        turns,
    ))
}

/// A YUV 4:2:0 picture as the decoder leaves it.
struct Planes<'a> {
    y: &'a [u8],
    u: &'a [u8],
    v: &'a [u8],
    strides: (usize, usize, usize),
    size: (usize, usize),
}

/// Converts limited-range BT.601 YUV to RGB at `out` size, turned clockwise
/// by `turns` quarter turns. Scaling happens during the conversion, one
/// sample per output pixel, so a large video costs about as much as a small
/// one; when scaling, luma is averaged over the first two by two source
/// pixels each output pixel covers, which keeps downscaled edges smooth.
fn convert(planes: &Planes<'_>, out: (usize, usize), turns: u8) -> ColorImage {
    let (width, height) = planes.size;
    let (out_width, out_height) = out;
    let turned = if turns % 2 == 1 {
        [out_height, out_width]
    } else {
        [out_width, out_height]
    };
    let scaling = out != planes.size;
    let mut pixels = vec![egui::Color32::BLACK; out_width * out_height];
    for oy in 0..out_height {
        let sy = (oy * height / out_height).min(height - 1);
        let below = if scaling {
            (sy + 1).min(height - 1)
        } else {
            sy
        };
        let luma = &planes.y[sy * planes.strides.0..];
        let luma_below = &planes.y[below * planes.strides.0..];
        let u = &planes.u[sy / 2 * planes.strides.1..];
        let v = &planes.v[sy / 2 * planes.strides.2..];
        for ox in 0..out_width {
            let sx = (ox * width / out_width).min(width - 1);
            let right = if scaling { (sx + 1).min(width - 1) } else { sx };
            let y = (u32::from(luma[sx])
                + u32::from(luma[right])
                + u32::from(luma_below[sx])
                + u32::from(luma_below[right])
                + 2)
                / 4;
            let (tx, ty) = match turns {
                1 => (out_height - 1 - oy, ox),
                2 => (out_width - 1 - ox, out_height - 1 - oy),
                3 => (oy, out_width - 1 - ox),
                _ => (ox, oy),
            };
            pixels[ty * turned[0] + tx] = rgb(y as i32, i32::from(u[sx / 2]), i32::from(v[sx / 2]));
        }
    }
    ColorImage::new(turned, pixels)
}

/// One limited-range BT.601 pixel in RGB.
fn rgb(y: i32, u: i32, v: i32) -> egui::Color32 {
    let (c, d, e) = (298 * (y - 16), u - 128, v - 128);
    let channel = |value: i32| ((value + 128) >> 8).clamp(0, 255) as u8;
    egui::Color32::from_rgb(
        channel(c + 409 * e),
        channel(c - 100 * d - 208 * e),
        channel(c + 516 * d),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/video/sample.mp4"
    );

    fn collect(path: &Path, from: Duration) -> Vec<Delivery> {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1024);
        let outcome = decode(path, from, &sender, &Waker::default());
        if let Err(reason) = outcome {
            sender.send(Delivery::Unsupported(reason)).unwrap();
        }
        drop(sender);
        receiver.into_iter().collect()
    }

    fn times(deliveries: &[Delivery]) -> Vec<Duration> {
        deliveries
            .iter()
            .filter_map(|delivery| match delivery {
                Delivery::Frame(at, _) => Some(*at),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_video_plays_to_the_end_and_returns_to_its_poster() {
        let ctx = egui::Context::default();
        let mut player = Player::new(Waker::default());
        player.silence();
        let path = Path::new(SAMPLE);
        player.toggle("clip", path);
        assert_eq!(player.status("clip").unwrap().state, State::Loading);
        assert!(player.status("other").is_none());
        let started = Instant::now();
        let mut shown = Vec::new();
        while player.message().is_some() {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "never finished"
            );
            player.saw("clip");
            assert_eq!(player.poll(&ctx), None);
            if let Some(status) = player.status("clip")
                && status.state == State::Playing
            {
                assert_eq!(status.total, Duration::from_secs(3));
                assert!(status.frame.is_some());
                shown.push(status.position);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // It ran in real time, not all at once.
        assert!(started.elapsed() >= Duration::from_millis(2900));
        assert!(shown.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(shown.last().unwrap() >= &Duration::from_millis(2800));
    }

    #[test]
    fn pausing_holds_the_position_and_an_unseen_video_pauses() {
        let ctx = egui::Context::default();
        let mut player = Player::new(Waker::default());
        player.silence();
        let path = Path::new(SAMPLE);
        player.toggle("clip", path);
        let wait_for = |player: &mut Player, state: State| {
            let started = Instant::now();
            while player.status("clip").unwrap().state != state {
                assert!(started.elapsed() < Duration::from_secs(5));
                player.saw("clip");
                player.poll(&ctx);
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        wait_for(&mut player, State::Playing);
        std::thread::sleep(Duration::from_millis(200));
        player.toggle("clip", path);
        let paused = player.status("clip").unwrap();
        assert_eq!(paused.state, State::Paused);
        std::thread::sleep(Duration::from_millis(100));
        player.poll(&ctx);
        assert_eq!(player.status("clip").unwrap().position, paused.position);
        // Seeking while paused shows the new place and stays paused.
        player.seek("clip", 0.5);
        wait_for(&mut player, State::Paused);
        assert_eq!(
            player.status("clip").unwrap().position,
            Duration::from_millis(1500)
        );
        player.toggle("clip", path);
        wait_for(&mut player, State::Playing);
        // Nobody draws it for a while: it pauses.
        player
            .seen
            .set(Instant::now() - UNSEEN - Duration::from_millis(1));
        player.poll(&ctx);
        assert_eq!(player.status("clip").unwrap().state, State::Paused);
        player.stop();
        assert!(player.message().is_none());
    }

    #[test]
    fn an_unreadable_video_goes_to_the_system_player() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip.mp4");
        std::fs::write(&path, b"not really a video").unwrap();
        let ctx = egui::Context::default();
        let mut player = Player::new(Waker::default());
        player.silence();
        player.toggle("clip", &path);
        let started = Instant::now();
        let notice = loop {
            assert!(started.elapsed() < Duration::from_secs(5));
            if let Some(notice) = player.poll(&ctx) {
                break notice;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(notice, Notice::Unsupported(path));
        assert!(player.message().is_none());
    }

    #[test]
    fn clock_runs_pauses_and_seeks() {
        let start = Instant::now();
        let later = |ms| start + Duration::from_millis(ms);
        let mut clock = Clock::default();
        assert_eq!(clock.position(later(500)), Duration::ZERO);
        clock.resume(start);
        assert_eq!(clock.position(later(500)), Duration::from_millis(500));
        clock.pause(later(500));
        assert_eq!(clock.position(later(900)), Duration::from_millis(500));
        clock.seek(Duration::from_secs(2), later(900));
        assert!(!clock.running());
        assert_eq!(clock.position(later(1000)), Duration::from_secs(2));
        clock.resume(later(1000));
        assert_eq!(clock.position(later(1250)), Duration::from_millis(2250));
    }

    #[test]
    fn clock_follows_the_sound_only_past_the_drift() {
        let start = Instant::now();
        let mut clock = Clock::default();
        clock.resume(start);
        let now = start + Duration::from_millis(1000);
        clock.follow(Duration::from_millis(1030), now);
        assert_eq!(clock.position(now), Duration::from_millis(1000));
        // A late device start leaves the sound well behind: wait for it.
        clock.follow(Duration::from_millis(800), now);
        assert_eq!(clock.position(now), Duration::from_millis(800));
        assert_eq!(
            clock.position(now + Duration::from_millis(100)),
            Duration::from_millis(900)
        );
    }

    #[test]
    fn due_frames_skip_to_the_newest() {
        let ms = Duration::from_millis;
        let mut queue: VecDeque<(Duration, u32)> =
            [(ms(0), 0), (ms(40), 1), (ms(80), 2), (ms(120), 3)].into();
        assert_eq!(take_due(&mut queue, ms(10)), Some(0));
        assert_eq!(take_due(&mut queue, ms(20)), None);
        assert_eq!(take_due(&mut queue, ms(100)), Some(2));
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn square_uv_crops_the_middle() {
        let wide = square_uv(1280.0, 720.0);
        assert!((wide.min.x - 0.21875).abs() < 1e-5);
        assert!((wide.max.x - 0.78125).abs() < 1e-5);
        assert_eq!((wide.min.y, wide.max.y), (0.0, 1.0));
        let tall = square_uv(480.0, 640.0);
        assert_eq!((tall.min.x, tall.max.x), (0.0, 1.0));
        assert!((tall.min.y - 0.125).abs() < 1e-5);
        let square = square_uv(400.0, 400.0);
        assert_eq!(
            square,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0))
        );
    }

    #[test]
    fn arc_runs_clockwise_from_the_top() {
        let center = egui::pos2(100.0, 100.0);
        let quarter = arc(center, 50.0, 0.25);
        let first = quarter[0];
        let last = *quarter.last().unwrap();
        assert!((first - egui::pos2(100.0, 50.0)).length() < 1e-3);
        assert!((last - egui::pos2(150.0, 100.0)).length() < 1e-3);
        assert!(
            quarter
                .iter()
                .all(|point| ((*point - center).length() - 50.0).abs() < 1e-3)
        );
        let full = arc(center, 50.0, 1.0);
        assert!((*full.last().unwrap() - full[0]).length() < 1e-3);
    }

    #[test]
    fn timeline_starts_at_the_keyframe_before_a_seek() {
        let timeline = Timeline {
            timescale: 1000,
            starts: (0..30).map(|index| index * 100).collect(),
            sync: vec![1, 11, 21],
            shift: 0,
        };
        assert_eq!(timeline.first_sample(Duration::ZERO), 1);
        assert_eq!(timeline.first_sample(Duration::from_millis(950)), 1);
        assert_eq!(timeline.first_sample(Duration::from_millis(1000)), 11);
        assert_eq!(timeline.first_sample(Duration::from_millis(2500)), 21);
        let all_sync = Timeline {
            sync: Vec::new(),
            ..timeline
        };
        assert_eq!(all_sync.first_sample(Duration::from_millis(1250)), 13);
    }

    #[test]
    fn edit_list_shift_moves_presentation_times() {
        let timeline = Timeline {
            timescale: 15_360,
            starts: Vec::new(),
            sync: Vec::new(),
            shift: 2048,
        };
        assert_eq!(timeline.time(2048), Duration::ZERO);
        assert_eq!(timeline.time(2048 + 15_360), Duration::from_secs(1));
        assert_eq!(timeline.time(0), Duration::ZERO);
        assert_eq!(timeline.ticks(Duration::from_secs(1)), 2048 + 15_360);
    }

    #[test]
    fn reorder_hands_back_display_order() {
        let mut order = Reorder::default();
        for time in [0, 3, 1, 2] {
            order.push(time);
        }
        assert_eq!(
            std::iter::from_fn(|| order.pop()).collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
    }

    #[test]
    fn quarter_turns_follow_the_track_matrix() {
        assert_eq!(quarter_turns(0x10000, 0, 0, 0x10000), 0);
        assert_eq!(quarter_turns(0, 0x10000, -0x10000, 0), 1);
        assert_eq!(quarter_turns(-0x10000, 0, 0, -0x10000), 2);
        assert_eq!(quarter_turns(0, -0x10000, 0x10000, 0), 3);
    }

    #[test]
    fn fitted_keeps_the_aspect_ratio() {
        assert_eq!(fitted(320, 180, 720), (320, 180));
        assert_eq!(fitted(1920, 1080, 720), (720, 405));
        assert_eq!(fitted(1080, 1920, 720), (405, 720));
    }

    #[test]
    fn sample_video_decodes_in_display_order() {
        let deliveries = collect(Path::new(SAMPLE), Duration::ZERO);
        assert!(matches!(
            deliveries.first(),
            Some(Delivery::Length(length)) if *length == Duration::from_secs(3)
        ));
        let times = times(&deliveries);
        // 3 seconds at 15 frames a second, starting at zero despite the
        // B-frame delay the edit list removes.
        assert_eq!(times.len(), 45);
        assert_eq!(times[0], Duration::ZERO);
        assert!(times.windows(2).all(|pair| pair[0] < pair[1]));
        let Some(Delivery::Frame(_, image)) = deliveries.get(1) else {
            panic!("expected a frame");
        };
        assert_eq!(image.size, [320, 180]);
    }

    #[test]
    fn seeking_starts_near_the_target() {
        let from = Duration::from_millis(1500);
        let times = times(&collect(Path::new(SAMPLE), from));
        // The frame shown at the target, then every later one.
        assert_eq!(times[0], from);
        assert!(times[1] > from);
        // The held frame, then 1.53 s to 2.93 s at 15 frames a second.
        assert_eq!(times.len(), 23);
    }

    #[test]
    fn other_files_are_reported_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip.mp4");
        std::fs::write(&path, b"not really a video").unwrap();
        let deliveries = collect(&path, Duration::ZERO);
        assert!(matches!(deliveries.last(), Some(Delivery::Unsupported(_))));
    }

    #[test]
    fn conversion_scales_and_turns_the_picture() {
        // Four by two: a dark left half and a white right half, neutral colour.
        let y = [16, 16, 235, 235, 16, 16, 235, 235];
        let chroma = [128, 128];
        let planes = Planes {
            y: &y,
            u: &chroma,
            v: &chroma,
            strides: (4, 2, 2),
            size: (4, 2),
        };
        let black = egui::Color32::from_rgb(0, 0, 0);
        let white = egui::Color32::from_rgb(255, 255, 255);
        let full = convert(&planes, (4, 2), 0);
        assert_eq!(full.size, [4, 2]);
        assert_eq!(full.pixels[..4], [black, black, white, white]);
        let half = convert(&planes, (2, 1), 0);
        assert_eq!(half.pixels, [black, white]);
        // A quarter turn clockwise puts the left half on top.
        let turned = convert(&planes, (2, 1), 1);
        assert_eq!(turned.size, [1, 2]);
        assert_eq!(turned.pixels, [black, white]);
        let upside_down = convert(&planes, (2, 1), 2);
        assert_eq!(upside_down.pixels, [white, black]);
        let back = convert(&planes, (2, 1), 3);
        assert_eq!(back.pixels, [white, black]);
    }

    #[test]
    fn primaries_convert_to_their_colours() {
        let near = |a: egui::Color32, b: [u8; 3]| {
            a.r().abs_diff(b[0]) <= 2 && a.g().abs_diff(b[1]) <= 2 && a.b().abs_diff(b[2]) <= 2
        };
        // Limited-range BT.601 red, green, and blue.
        assert!(near(rgb(81, 90, 240), [255, 0, 0]));
        assert!(near(rgb(145, 54, 34), [0, 255, 0]));
        assert!(near(rgb(41, 240, 110), [0, 0, 255]));
    }

    #[test]
    fn sample_video_has_decodable_sound() {
        let mut decoder = sound_decoder(Path::new(SAMPLE), Duration::ZERO).unwrap();
        assert!(decoder.by_ref().take(1000).count() == 1000);
        let seeked = sound_decoder(Path::new(SAMPLE), Duration::from_secs(1)).unwrap();
        assert!(seeked.count() > 0);
    }

    #[test]
    fn sample_video_plays_its_whole_sound() {
        let mut decoder = sound_decoder(Path::new(SAMPLE), Duration::ZERO).unwrap();
        let rate = rodio::Source::sample_rate(&decoder).get();
        let channels = usize::from(rodio::Source::channels(&decoder).get());
        let frames = decoder.by_ref().count() / channels;
        let sound = Duration::from_secs_f64(frames as f64 / f64::from(rate));
        // The clip lasts three seconds and its sound track lasts as long. A
        // decoder that stops at the first packet of another track leaves a
        // tenth of a second, which the listener hears as silence.
        assert!(
            sound > Duration::from_millis(2900),
            "the sound of a three second clip should last about three seconds, not {sound:?}"
        );
    }
}
