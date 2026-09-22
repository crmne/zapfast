//! Audio playback and voice-message recording.
//!
//! Input and output devices are opened on demand and released when idle.

use std::collections::HashMap;
use std::num::NonZero;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rodio::Source;
use rodio::buffer::SamplesBuffer;

use crate::backend::Waker;
use crate::voice;

/// Maximum recording length. The phone uses a shorter limit.
const LONGEST_RECORDING: Duration = Duration::from_secs(15 * 60);

fn mono() -> NonZero<u16> {
    NonZero::<u16>::MIN
}

fn rate() -> NonZero<u32> {
    NonZero::new(voice::RATE).expect("48 kHz is not zero")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Loading,
    Playing,
    Paused,
}

/// Playback state for one message.
#[derive(Clone, Copy, Debug)]
pub struct Status {
    pub state: State,
    pub position: Duration,
    pub total: Duration,
}

impl Status {
    const IDLE: Self = Self {
        state: State::Idle,
        position: Duration::ZERO,
        total: Duration::ZERO,
    };
}

/// Playback speeds, in the order the speed button cycles them.
pub const SPEEDS: [f32; 3] = [1.0, 1.5, 2.0];

/// Label for a playback speed, like `1x` or `1.5x`.
pub fn speed_label(speed: f32) -> String {
    if speed.fract() == 0.0 {
        format!("{}x", speed as i32)
    } else {
        format!("{speed:.1}x")
    }
}

type Decoded = Arc<Mutex<Option<Result<Vec<f32>, String>>>>;

/// Plays one clip at a time through the default output device.
pub struct Player {
    waker: Waker,
    output: Option<(rodio::MixerDeviceSink, rodio::Player)>,
    loaded: Option<Loaded>,
    decoding: Option<Decoding>,
    /// Playback speed applied to the current clip and to later ones.
    speed: f32,
    /// Time-compressed copies of the loaded clip, one per speed already
    /// built, dropped when the clip changes.
    stretches: Vec<(f32, Arc<Vec<f32>>)>,
    /// Compression being built for the loaded message.
    stretching: Option<Stretching>,
    /// Generated waveforms for clips that did not include one.
    bars: HashMap<String, Vec<u8>>,
}

struct Loaded {
    message: String,
    samples: Arc<Vec<f32>>,
    /// Samples queued in the sink: the clip itself or its compression.
    buffer: Arc<Vec<f32>>,
    /// Speed the queued buffer represents; 1 plays the clip as recorded.
    factor: f32,
    /// Restart position in the clip's own timeline.
    base: Duration,
    paused: bool,
    done: bool,
}

struct Stretching {
    factor: f32,
    slot: StretchedSlot,
    /// Set when this job is replaced or the clip changes, so the worker
    /// stops instead of piling up behind the next one.
    cancelled: Arc<AtomicBool>,
}

impl Drop for Stretching {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

type StretchedSlot = Arc<Mutex<Option<Arc<Vec<f32>>>>>;

struct Decoding {
    message: String,
    /// Requested start position after decoding, from 0 to 1.
    start: f32,
    slot: Decoded,
}

impl Player {
    pub fn new(waker: Waker) -> Self {
        Self {
            waker,
            output: None,
            loaded: None,
            decoding: None,
            speed: SPEEDS[0],
            stretches: Vec::new(),
            stretching: None,
            bars: HashMap::new(),
        }
    }

    /// Current playback speed multiplier.
    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// Sets the playback speed for the clip playing now and for later ones.
    ///
    /// Speeds above 1x play a time-compressed copy of the clip, once it has
    /// been built, so the voice keeps its pitch. Until then playback
    /// continues at the speed already queued. Speeds outside 1x to 2x, such
    /// as a hand-edited setting, are clamped, and non-finite ones play at 1x.
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = if speed.is_finite() {
            speed.clamp(SPEEDS[0], SPEEDS[SPEEDS.len() - 1])
        } else {
            SPEEDS[0]
        };
        self.apply_speed();
        self.ensure_stretch();
    }

    /// Whether `message` is still playing at an earlier speed while the
    /// compression for the chosen one builds.
    pub fn preparing_speed(&self, message: &str) -> bool {
        self.loaded.as_ref().is_some_and(|loaded| {
            loaded.message == message && !loaded.done && loaded.factor != self.speed
        })
    }

    /// Cycles 1x, 1.5x, and 2x, wrapping back to 1x.
    pub fn cycle_speed(&mut self) -> f32 {
        let next = SPEEDS
            .iter()
            .copied()
            .find(|&candidate| candidate > self.speed)
            .unwrap_or(SPEEDS[0]);
        self.set_speed(next);
        self.speed
    }

    /// The samples that play at `speed` and the speed they represent: the
    /// clip itself at 1x, its compression once built, and otherwise whatever
    /// is queued, so a speed still building does not drop playback to 1x.
    fn buffer_for(
        loaded: &Loaded,
        stretches: &[(f32, Arc<Vec<f32>>)],
        speed: f32,
    ) -> (Arc<Vec<f32>>, f32) {
        if speed <= 1.0 {
            return (Arc::clone(&loaded.samples), 1.0);
        }
        match stretches.iter().find(|(factor, _)| *factor == speed) {
            Some((factor, compressed)) => (Arc::clone(compressed), *factor),
            None => (Arc::clone(&loaded.buffer), loaded.factor),
        }
    }

    /// Restarts playback on the buffer for the current speed, keeping the
    /// position, when it differs from what is queued.
    fn apply_speed(&mut self) {
        let Some(loaded) = self.loaded.as_ref() else {
            return;
        };
        let (wanted, _) = Self::buffer_for(loaded, &self.stretches, self.speed);
        if self.output.is_none() || Arc::ptr_eq(&wanted, &loaded.buffer) {
            return;
        }
        let total = clip_length(loaded.samples.len());
        let fraction = if total > Duration::ZERO {
            (self.status(&loaded.message).position.as_secs_f64() / total.as_secs_f64()) as f32
        } else {
            0.0
        }
        .clamp(0.0, 1.0);
        let paused = loaded.paused;
        if self.restart(fraction).is_ok() && paused {
            if let Some((_, sink)) = &self.output {
                sink.pause();
            }
            if let Some(loaded) = self.loaded.as_mut() {
                loaded.paused = true;
            }
        }
    }

    /// Builds the compression for the current speed in the background, if it
    /// is still missing. Replacing an outstanding job cancels it, and so does
    /// going back to 1x, which needs none.
    fn ensure_stretch(&mut self) {
        let factor = self.speed;
        if factor <= 1.0 {
            self.stretching = None;
            return;
        }
        if self.stretches.iter().any(|(built, _)| *built == factor)
            || self
                .stretching
                .as_ref()
                .is_some_and(|job| job.factor == factor)
        {
            return;
        }
        let Some(loaded) = &self.loaded else {
            return;
        };
        let samples = Arc::clone(&loaded.samples);
        let waker = self.waker.clone();
        let slot: StretchedSlot = Default::default();
        let cancelled = Arc::new(AtomicBool::new(false));
        let thread_slot = Arc::clone(&slot);
        let thread_cancelled = Arc::clone(&cancelled);
        let spawned = std::thread::Builder::new()
            .name("voice-stretch".to_owned())
            .spawn(move || {
                let Some(compressed) =
                    crate::timestretch::speed_up_unless(&samples, factor, &thread_cancelled)
                else {
                    return;
                };
                *thread_slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(Arc::new(compressed));
                waker.wake();
            });
        if spawned.is_ok() {
            self.stretching = Some(Stretching {
                factor,
                slot,
                cancelled,
            });
        }
    }

    /// Plays or pauses a message. Finished clips restart; new clips decode first.
    pub fn toggle(&mut self, message: &str, path: &Path) -> Result<(), String> {
        match self.loaded.as_mut() {
            Some(loaded) if loaded.message == message => {
                if loaded.done {
                    return self.restart(0.0);
                }
                if let Some((_, sink)) = &self.output {
                    if loaded.paused {
                        sink.play();
                    } else {
                        sink.pause();
                    }
                    loaded.paused = !loaded.paused;
                }
                Ok(())
            }
            _ => self.load(message, path, 0.0),
        }
    }

    /// Seeks to a fraction from 0 to 1 and starts playback.
    pub fn seek(&mut self, message: &str, path: &Path, fraction: f32) -> Result<(), String> {
        match &self.loaded {
            Some(loaded) if loaded.message == message => self.restart(fraction),
            _ => self.load(message, path, fraction),
        }
    }

    /// Clears the loaded clip and releases the output device.
    pub fn stop(&mut self) {
        self.output = None;
        self.loaded = None;
        self.decoding = None;
        self.stretches.clear();
        self.stretching = None;
    }

    /// Whether audio is currently playing.
    pub fn is_playing(&self) -> bool {
        self.decoding.is_some()
            || self
                .loaded
                .as_ref()
                .is_some_and(|loaded| !loaded.paused && !loaded.done)
    }

    pub fn status(&self, message: &str) -> Status {
        if let Some(decoding) = &self.decoding
            && decoding.message == message
        {
            return Status {
                state: State::Loading,
                ..Status::IDLE
            };
        }
        match &self.loaded {
            Some(loaded) if loaded.message == message => {
                let total = clip_length(loaded.samples.len());
                if loaded.done {
                    return Status {
                        state: State::Idle,
                        position: Duration::ZERO,
                        total,
                    };
                }
                let position = self
                    .output
                    .as_ref()
                    .map(|(_, sink)| loaded.base + sink.get_pos().mul_f32(loaded.factor))
                    .unwrap_or(loaded.base)
                    .min(total);
                Status {
                    state: if loaded.paused {
                        State::Paused
                    } else {
                        State::Playing
                    },
                    position,
                    total,
                }
            }
            _ => Status::IDLE,
        }
    }

    /// Generated waveform for a decoded clip.
    pub fn bars(&self, message: &str) -> Option<&[u8]> {
        self.bars.get(message).map(Vec::as_slice)
    }

    /// Handles completed decodes and finished playback once per frame.
    pub fn poll(&mut self) -> Result<(), String> {
        let decoded = self.decoding.as_ref().and_then(|decoding| {
            decoding
                .slot
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take()
        });
        if let Some(result) = decoded {
            let Decoding { message, start, .. } = self.decoding.take().expect("just seen");
            let samples = result?;
            if samples.is_empty() {
                return Err("The clip is empty".to_owned());
            }
            let samples = Arc::new(samples);
            self.bars
                .entry(message.clone())
                .or_insert_with(|| voice::waveform(&samples));
            self.loaded = Some(Loaded {
                message,
                buffer: Arc::clone(&samples),
                factor: 1.0,
                samples,
                base: Duration::ZERO,
                paused: false,
                done: false,
            });
            self.restart(start)?;
            self.ensure_stretch();
        }
        let compressed = self
            .stretching
            .as_ref()
            .and_then(|job| job.slot.lock().unwrap_or_else(|p| p.into_inner()).take());
        if let Some(samples) = compressed {
            let factor = self.stretching.take().expect("just seen").factor;
            self.stretches.push((factor, samples));
            self.apply_speed();
            // The speed may have moved on while this compression built.
            self.ensure_stretch();
        }
        let ended = match (&mut self.loaded, &self.output) {
            (Some(loaded), Some((_, sink))) if !loaded.done && !loaded.paused && sink.empty() => {
                loaded.done = true;
                true
            }
            _ => false,
        };
        if ended {
            // Release the device after playback ends.
            self.output = None;
        }
        Ok(())
    }

    fn load(&mut self, message: &str, path: &Path, start: f32) -> Result<(), String> {
        self.stop();
        let slot: Decoded = Default::default();
        let path = path.to_owned();
        let waker = self.waker.clone();
        let thread_slot = Arc::clone(&slot);
        let spawned = std::thread::Builder::new()
            .name("voice-decode".to_owned())
            .spawn(move || {
                let result = decode_file(&path);
                *thread_slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(result);
                waker.wake();
            });
        if let Err(error) = spawned {
            return Err(format!("Could not decode audio: {error}"));
        }
        self.decoding = Some(Decoding {
            message: message.to_owned(),
            start,
            slot,
        });
        Ok(())
    }

    /// Plays the loaded clip from a fraction from 0 to 1.
    fn restart(&mut self, fraction: f32) -> Result<(), String> {
        let Some(loaded) = self.loaded.as_mut() else {
            return Ok(());
        };
        // The sink always plays at 1x: running it faster sharpens the voice,
        // so speeds above 1x queue a time-compressed copy of the clip.
        let (buffer, factor) = Self::buffer_for(loaded, &self.stretches, self.speed);
        let total = clip_length(loaded.samples.len());
        let offset = ((fraction.clamp(0.0, 1.0) * buffer.len() as f32) as usize).min(buffer.len());
        if self.output.is_none() {
            let device = rodio::DeviceSinkBuilder::open_default_sink()
                .map_err(|error| format!("No sound output: {error}"))?;
            let sink = rodio::Player::connect_new(device.mixer());
            self.output = Some((device, sink));
        }
        let (_, sink) = self.output.as_ref().expect("just opened");
        sink.clear();
        sink.append(SamplesBuffer::new(
            mono(),
            rate(),
            buffer[offset..].to_vec(),
        ));
        sink.play();
        loaded.buffer = buffer;
        loaded.factor = factor;
        loaded.base =
            Duration::from_secs_f64(fraction.clamp(0.0, 1.0) as f64 * total.as_secs_f64());
        loaded.paused = false;
        loaded.done = false;
        Ok(())
    }
}

fn clip_length(samples: usize) -> Duration {
    Duration::from_secs_f64(samples as f64 / f64::from(voice::RATE))
}

/// Decodes a file to mono 48 kHz samples. OGG/Opus uses `voice`; other
/// supported formats use rodio.
fn decode_file(path: &Path) -> Result<Vec<f32>, String> {
    let bytes =
        std::fs::read(path).map_err(|error| format!("Could not read the audio: {error}"))?;
    if bytes.starts_with(b"OggS")
        && let Ok(samples) = voice::decode(&bytes)
    {
        return Ok(samples);
    }
    let file =
        std::fs::File::open(path).map_err(|error| format!("Could not read the audio: {error}"))?;
    let decoder = rodio::Decoder::new(std::io::BufReader::new(file))
        .map_err(|error| format!("Could not decode the audio: {error}"))?;
    let channels = decoder.channels().get();
    let rate = decoder.sample_rate().get();
    let interleaved: Vec<f32> = decoder.collect();
    Ok(voice::mono_at_rate(&interleaved, channels, rate))
}

type Outcome = Arc<Mutex<Option<Result<Vec<f32>, String>>>>;

/// Records from the default microphone until told to stop.
pub struct Recorder {
    started: Instant,
    stop: Arc<AtomicBool>,
    /// Loudness for each recorded 50 ms segment.
    levels: Arc<Mutex<Vec<f32>>>,
    outcome: Outcome,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Recorder {
    pub fn start(waker: Waker) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let levels: Arc<Mutex<Vec<f32>>> = Default::default();
        let outcome: Outcome = Default::default();
        let spawned = {
            let stop = Arc::clone(&stop);
            let levels = Arc::clone(&levels);
            let outcome = Arc::clone(&outcome);
            std::thread::Builder::new()
                .name("voice-record".to_owned())
                .spawn(move || {
                    let result = record(&stop, &levels, &waker);
                    *outcome.lock().unwrap_or_else(|p| p.into_inner()) = Some(result);
                    waker.wake();
                })
        };
        let thread = match spawned {
            Ok(thread) => Some(thread),
            Err(error) => {
                *outcome.lock().unwrap_or_else(|p| p.into_inner()) = Some(Err(error.to_string()));
                None
            }
        };
        Self {
            started: Instant::now(),
            stop,
            levels,
            outcome,
            thread,
        }
    }

    /// Simulated recorder for demos and tests.
    #[cfg(any(test, feature = "demo"))]
    pub fn rehearsal() -> Self {
        let levels: Vec<f32> = (0..90)
            .map(|index| 0.05 + 0.2 * ((index as f32 * 0.6).sin().abs()))
            .collect();
        Self {
            started: Instant::now() - Duration::from_millis(4_500),
            stop: Arc::new(AtomicBool::new(true)),
            levels: Arc::new(Mutex::new(levels)),
            outcome: Default::default(),
            thread: None,
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn levels(&self) -> Vec<f32> {
        self.levels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Error that stopped recording early.
    pub fn failure(&self) -> Option<String> {
        match self
            .outcome
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            Some(Err(error)) => Some(error.clone()),
            _ => None,
        }
    }

    /// Stops and returns mono 48 kHz samples.
    pub fn finish(mut self) -> Result<Vec<f32>, String> {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.outcome
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
            .unwrap_or_else(|| Err("No audio was recorded".to_owned()))
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn record(stop: &AtomicBool, levels: &Mutex<Vec<f32>>, waker: &Waker) -> Result<Vec<f32>, String> {
    let mut microphone = rodio::microphone::MicrophoneBuilder::new()
        .default_device()
        .map_err(|error| format!("No microphone available: {error}"))?
        .default_config()
        .map_err(|error| format!("The microphone has no supported format: {error}"))?
        .open_stream()
        .map_err(|error| format!("Could not open the microphone: {error}"))?;
    let channels = microphone.channels().get();
    let rate = microphone.sample_rate().get();
    let chunk = (rate as usize * usize::from(channels) / 20).max(1);
    let started = Instant::now();
    let mut heard = Vec::new();
    while !stop.load(Ordering::Relaxed) && started.elapsed() < LONGEST_RECORDING {
        let before = heard.len();
        heard.extend(microphone.by_ref().take(chunk));
        let taken = &heard[before..];
        if taken.is_empty() {
            break;
        }
        let loudness = (taken.iter().map(|s| s * s).sum::<f32>() / taken.len() as f32).sqrt();
        levels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(loudness);
        waker.wake();
        if taken.len() < chunk {
            // The device disappeared before recording stopped.
            break;
        }
    }
    if heard.is_empty() {
        return Err("The microphone did not record any audio".to_owned());
    }
    Ok(voice::mono_at_rate(&heard, channels, rate))
}

/// Temporary recording path used before sending and archiving.
#[allow(dead_code)]
pub fn recording_path(dir: &Path) -> PathBuf {
    dir.join(format!("voice-{}.ogg", crate::util::now()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_labels_match_the_button() {
        assert_eq!(speed_label(SPEEDS[0]), "1x");
        assert_eq!(speed_label(1.5), "1.5x");
        assert_eq!(speed_label(SPEEDS[2]), "2x");
    }

    #[test]
    fn cycling_wraps_through_every_speed() {
        let mut player = Player::new(Waker::default());
        assert_eq!(player.speed(), SPEEDS[0]);
        assert_eq!(player.cycle_speed(), 1.5);
        assert_eq!(player.cycle_speed(), 2.0);
        assert_eq!(player.cycle_speed(), 1.0);
        // A speed set by hand still cycles up to the next known one.
        player.set_speed(1.75);
        assert_eq!(player.cycle_speed(), 2.0);
        assert_eq!(player.speed(), 2.0);
    }

    #[test]
    fn a_speed_still_building_keeps_the_queued_one() {
        let samples = Arc::new(vec![0.0; 12]);
        let one_and_a_half = Arc::new(vec![0.0; 8]);
        let double = Arc::new(vec![0.0; 6]);
        let loaded = Loaded {
            message: "clip".to_owned(),
            samples: Arc::clone(&samples),
            buffer: Arc::clone(&one_and_a_half),
            factor: 1.5,
            base: Duration::ZERO,
            paused: false,
            done: false,
        };
        let mut stretches = vec![(1.5, Arc::clone(&one_and_a_half))];

        let (buffer, factor) = Player::buffer_for(&loaded, &stretches, 2.0);
        assert!(Arc::ptr_eq(&buffer, &one_and_a_half));
        assert_eq!(factor, 1.5);

        stretches.push((2.0, Arc::clone(&double)));
        let (buffer, factor) = Player::buffer_for(&loaded, &stretches, 2.0);
        assert!(Arc::ptr_eq(&buffer, &double));
        assert_eq!(factor, 2.0);
        // Both built speeds stay available when cycling back.
        let (buffer, _) = Player::buffer_for(&loaded, &stretches, 1.5);
        assert!(Arc::ptr_eq(&buffer, &one_and_a_half));
        let (buffer, factor) = Player::buffer_for(&loaded, &stretches, 1.0);
        assert!(Arc::ptr_eq(&buffer, &samples));
        assert_eq!(factor, 1.0);
    }

    #[test]
    fn a_speed_is_preparing_until_the_clip_plays_at_it() {
        let samples = Arc::new(vec![0.0; 12]);
        let mut player = Player::new(Waker::default());
        player.loaded = Some(Loaded {
            message: "clip".to_owned(),
            buffer: Arc::clone(&samples),
            samples,
            factor: 1.0,
            base: Duration::ZERO,
            paused: true,
            done: false,
        });
        assert!(!player.preparing_speed("clip"));
        player.speed = 2.0;
        assert!(player.preparing_speed("clip"));
        assert!(!player.preparing_speed("another clip"));
        if let Some(loaded) = player.loaded.as_mut() {
            loaded.factor = 2.0;
        }
        assert!(!player.preparing_speed("clip"));
    }

    #[test]
    fn unusable_speeds_are_kept_in_range() {
        let mut player = Player::new(Waker::default());
        player.set_speed(f32::NAN);
        assert_eq!(player.speed(), 1.0);
        player.set_speed(f32::INFINITY);
        assert_eq!(player.speed(), 1.0);
        player.set_speed(50.0);
        assert_eq!(player.speed(), 2.0);
        player.set_speed(-3.0);
        assert_eq!(player.speed(), 1.0);
    }

    #[test]
    fn going_back_to_one_x_cancels_the_outstanding_compression() {
        let mut player = Player::new(Waker::default());
        let cancelled = Arc::new(AtomicBool::new(false));
        player.set_speed(2.0);
        player.stretching = Some(Stretching {
            factor: 2.0,
            slot: Default::default(),
            cancelled: Arc::clone(&cancelled),
        });
        player.set_speed(1.0);
        assert!(player.stretching.is_none());
        assert!(cancelled.load(Ordering::Relaxed));
    }

    /// Plays a one-second test tone:
    /// `cargo test audio::tests::plays -- --ignored --nocapture`.
    #[test]
    #[ignore = "makes a sound on this machine"]
    fn plays_a_clip_on_this_machine() {
        let dir = std::env::temp_dir();
        let path = dir.join("zapfast-audio-test.ogg");
        let tone: Vec<f32> = (0..voice::RATE)
            .map(|i| (i as f32 * 330.0 * std::f32::consts::TAU / voice::RATE as f32).sin() * 0.3)
            .collect();
        std::fs::write(&path, voice::encode(&tone).expect("encodes")).expect("written");
        let mut player = Player::new(Waker::default());
        player.toggle("clip", &path).expect("starts decoding");
        assert_eq!(player.status("clip").state, State::Loading);
        let started = Instant::now();
        let mut seen_playing = false;
        while started.elapsed() < Duration::from_secs(3) {
            player.poll().expect("plays");
            let status = player.status("clip");
            if status.state == State::Playing && status.position > Duration::from_millis(300) {
                seen_playing = true;
                eprintln!("playing at {:?} of {:?}", status.position, status.total);
            }
            if seen_playing && status.state == State::Idle {
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        assert!(seen_playing, "never heard it playing");
        assert_eq!(player.status("clip").state, State::Idle, "ends on its own");
        assert_eq!(player.bars("clip").map(<[u8]>::len), Some(voice::BARS));
        let _ = std::fs::remove_file(path);
    }

    /// Plays a two-second tone at double speed and checks the position
    /// outruns the clock:
    /// `cargo test audio::tests::doubles -- --ignored --nocapture`.
    #[test]
    #[ignore = "makes a sound on this machine"]
    fn doubles_the_position_rate_on_this_machine() {
        let dir = std::env::temp_dir();
        let path = dir.join("zapfast-audio-speed-test.ogg");
        let tone: Vec<f32> = (0..voice::RATE * 2)
            .map(|i| (i as f32 * 330.0 * std::f32::consts::TAU / voice::RATE as f32).sin() * 0.3)
            .collect();
        std::fs::write(&path, voice::encode(&tone).expect("encodes")).expect("written");
        let mut player = Player::new(Waker::default());
        player.set_speed(2.0);
        player.toggle("clip", &path).expect("starts decoding");
        let started = Instant::now();
        let mut seen: Vec<(Duration, Duration)> = Vec::new();
        while started.elapsed() < Duration::from_secs(6) {
            player.poll().expect("plays");
            let status = player.status("clip");
            // The clip starts at 1x while its compression builds; measure
            // only after the compressed buffer has taken over.
            if status.state == State::Playing && status.position > Duration::from_millis(600) {
                seen.push((started.elapsed(), status.position));
            }
            if status.state == State::Idle && !seen.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        let (first_wall, first_position) = seen.first().expect("played");
        let (last_wall, last_position) = seen.last().expect("played");
        let wall = *last_wall - *first_wall;
        let advanced = *last_position - *first_position;
        assert!(wall > Duration::from_millis(200), "played for {wall:?}");
        assert!(
            advanced.as_secs_f32() >= 1.5 * wall.as_secs_f32(),
            "position advanced {advanced:?} over {wall:?} of wall time"
        );
        let _ = std::fs::remove_file(path);
    }

    /// Records one second from the default microphone:
    /// `cargo test audio::tests::records -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs a microphone"]
    fn records_a_second_on_this_machine() {
        let recorder = Recorder::start(Waker::default());
        std::thread::sleep(Duration::from_millis(1_000));
        assert!(recorder.failure().is_none(), "{:?}", recorder.failure());
        let levels = recorder.levels();
        let heard = recorder.finish().expect("something was heard");
        eprintln!("{} samples, {} level readings", heard.len(), levels.len());
        assert!(
            heard.len() > voice::RATE as usize * 8 / 10,
            "{}",
            heard.len()
        );
        assert!(levels.len() >= 15, "{}", levels.len());
    }
}
