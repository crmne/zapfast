//! Call audio: the microphone and the speaker through rodio.
//!
//! The engine runs a 16 kHz mono clock, 960 samples to a frame, and talks in `i16`. rodio opens
//! whatever the platform's own audio API offers (PipeWire or ALSA, CoreAudio, WASAPI) at the rate
//! and channel count that device wants, in `f32`, and this module converts between the two.
//!
//! It is the same rodio the rest of the app plays and records through, so a call no longer needs
//! `pw-record` or `pw-play` to exist: the engine's channel stays stable while a device is chosen or
//! lost, and only the reader or the writer behind it is replaced.
//!
//! Device selection is by name, because the name a platform reports is what it also accepts back:
//! the call screen's picker, the saved setting, and the stream all speak the same identifier.

use std::num::NonZero;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rodio::Source;
use rodio::buffer::SamplesBuffer;
use rodio::cpal::traits::{DeviceTrait, HostTrait};

/// The engine's audio clock: 16 kHz mono.
pub const RATE: u32 = 16_000;
/// Samples in one engine frame: 60 ms at [`RATE`].
pub const FRAME_SAMPLES: usize = 960;
/// Device audio is converted a block at a time, so a device that only delivers a little at once
/// still makes progress and the remainder is carried rather than padded.
const BLOCK_MS: usize = 20;
/// How many frames the engine may queue for the speaker before it sheds them.
///
/// The engine writes one 20 ms slice every 20 ms and drops the frame when this channel is full, so
/// whatever is here is the burst the speaker may absorb before the peer's voice starts breaking up.
/// Three hundred milliseconds sits above the engine's own jitter cushion rather than under it.
pub const SPEAKER_QUEUE: usize = 16;
/// How long the sink may hold audio without playing any of it before the stream is restarted.
///
/// A device that stops draining is not a slow consumer to catch up with: the queue only grows and
/// the peer is silent for the rest of the call. Half a second is far above anything a healthy sink
/// needs for 60 ms of audio and far below the point where waiting has any value.
const STALLED: Duration = Duration::from_millis(500);
/// How long a stream that has not played a single sample is given to start playing one.
///
/// A sink that has never moved is not a sink that stopped: a suspended device, a Bluetooth headset
/// still connecting, or a PipeWire graph that a new stream (a camera, a video player) has just
/// relinked all take longer than half a second to play their first sample. Judging them stalled on
/// the same clock as a dead stream tears down the very stream that was about to play, and every
/// restart throws away the audio queued behind it, so the call loses its sound to its own recovery.
/// Two seconds is far above any start-up a working device needs and far below leaving a dead one
/// silent for the rest of the call.
const STARTUP: Duration = Duration::from_millis(2_000);
/// How long the sink is excused from the stall clock after a new media stream relinks the audio
/// graph.
///
/// Turning a camera on opens a second stream on this machine's audio graph, and the graph relinks
/// every stream on it while the new one settles: the sink stops draining for as long as that takes,
/// which is routinely longer than the half-second stall window. Judged by that window the call
/// tore its own player down the moment the camera came on, dropped the peer's audio queued behind
/// it, and reopened the stream into the same relink: the sound did not come back while the
/// microphone, the video and the signaling all kept working. Three seconds is above any relink a
/// working graph needs and below leaving a dead device holding the call, and the window only opens
/// when the call itself says a stream came or went.
const RELINK_GRACE: Duration = Duration::from_millis(3_000);
/// How many times a sink may be restarted without ever having played before the selected device is
/// given up for the system default.
///
/// A device that takes frames and never plays one is a device that is not there any more, whatever
/// its name says. Retrying it forever is a call that is silent for its whole length with no counter
/// moving and nothing on screen to explain it, so the pump stops trusting the name it was given and
/// says so.
const UNPLAYED_RESTARTS_BEFORE_DEFAULT: usize = 3;
/// How long a pump waits before reopening a device that would not open.
const RETRY: Duration = Duration::from_millis(500);
/// How long a reader waits for room in the engine's channel before looking at its stop flag again.
const BACKOFF: Duration = Duration::from_millis(5);

// ---------------------------------------------------------------------------
// Conversion
// ---------------------------------------------------------------------------

/// Mixes interleaved device samples down to one channel.
fn mono(interleaved: &[f32], channels: u16) -> Vec<f32> {
    let channels = usize::from(channels.max(1));
    if channels == 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect()
}

/// Resamples one channel of `f32` between two rates by linear interpolation, keeping pitch.
///
/// The engine's rate and a device's rate are rarely the same (16 kHz against 44.1 or 48 kHz), and
/// handing either side the other's samples would play the peer's voice at the wrong speed. Linear
/// interpolation is enough for speech and adds no dependency beyond the ones already linked.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() || from == 0 || to == 0 {
        return input.to_vec();
    }
    let step = f64::from(from) / f64::from(to);
    let out_len = ((input.len() as f64) / step).floor() as usize;
    (0..out_len)
        .map(|index| {
            let at = index as f64 * step;
            let first = at as usize;
            let fraction = (at - first as f64) as f32;
            let a = input[first];
            let b = input.get(first + 1).copied().unwrap_or(a);
            a + (b - a) * fraction
        })
        .collect()
}

/// Turns device audio into whole engine frames.
///
/// A device delivers whatever it likes per read; the engine only ever wants 960 samples at 16 kHz,
/// so the remainder of a read is carried into the next one rather than padded or dropped, and no
/// frame boundary drifts.
struct Converter {
    layout: Layout,
    /// Samples at [`RATE`] not yet packed into a frame.
    pending: Vec<f32>,
}

impl Converter {
    fn new(channels: u16, rate: u32) -> Self {
        Self {
            layout: Layout { rate, channels },
            pending: Vec::new(),
        }
    }

    /// The interleaved samples one read should pull: one block of them.
    fn block(&self) -> usize {
        let per_channel = (self.layout.rate as usize / (1000 / BLOCK_MS)).max(1);
        per_channel * usize::from(self.layout.channels.max(1))
    }

    /// Converts interleaved device samples and appends every whole engine frame to `out`.
    fn push(&mut self, interleaved: &[f32], out: &mut Vec<Vec<i16>>) {
        let mono = mono(interleaved, self.layout.channels);
        self.pending
            .extend_from_slice(&resample(&mono, self.layout.rate, RATE));
        while self.pending.len() >= FRAME_SAMPLES {
            out.push(
                self.pending
                    .drain(..FRAME_SAMPLES)
                    .map(|sample| (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16)
                    .collect(),
            );
        }
    }
}

/// The rate and channel count one device wants, and the conversion into them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Layout {
    rate: u32,
    channels: u16,
}

impl Layout {
    /// Turns an engine frame into the interleaved samples this device wants.
    fn convert(&self, frame: &[i16]) -> Vec<f32> {
        let mono: Vec<f32> = frame
            .iter()
            .map(|sample| f32::from(*sample) / f32::from(i16::MAX))
            .collect();
        let at_rate = resample(&mono, RATE, self.rate);
        if self.channels <= 1 {
            return at_rate;
        }
        let channels = usize::from(self.channels);
        let mut out = Vec::with_capacity(at_rate.len() * channels);
        for sample in at_rate {
            out.extend(std::iter::repeat_n(sample, channels));
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Devices
// ---------------------------------------------------------------------------

/// The microphones the platform reports, as `(name, label)`.
///
/// The name is what the platform both reports and accepts back, so it is what a saved setting
/// holds and what a stream looks for.
pub fn microphones() -> Vec<(String, String)> {
    match rodio::microphone::available_inputs() {
        Ok(inputs) => dedupe(
            inputs
                .into_iter()
                .map(|input| {
                    let name = input.to_string();
                    let label = if name.is_empty() {
                        "Microphone".to_owned()
                    } else {
                        name.clone()
                    };
                    (name, label)
                })
                .collect(),
        ),
        Err(error) => {
            log::warn!("[CALL] the input device list could not be read: {error}");
            Vec::new()
        }
    }
}

/// The speakers the platform reports, as `(name, label)`.
///
/// A device that exists only to swallow sound (ALSA's `null`, which rodio's own listing hides) is
/// left out: offering it would let a call send the peer's voice nowhere.
pub fn speakers() -> Vec<(String, String)> {
    let devices = match rodio::cpal::default_host().output_devices() {
        Ok(devices) => devices,
        Err(error) => {
            log::warn!("[CALL] the output device list could not be read: {error}");
            return Vec::new();
        }
    };
    dedupe(
        devices
            .filter(|device| {
                device
                    .description()
                    .map(|description| description.driver() != Some("null"))
                    .unwrap_or(true)
            })
            .filter_map(|device| {
                let name = device.description().ok()?.name().to_string();
                let label = if name.is_empty() {
                    "Speaker".to_owned()
                } else {
                    name.clone()
                };
                Some((name, label))
            })
            .collect(),
    )
}

/// Keeps one entry per name, in label order, which is what a picker wants.
fn dedupe(mut found: Vec<(String, String)>) -> Vec<(String, String)> {
    found.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    found.dedup_by(|a, b| a.0 == b.0);
    found
}

/// What a call needs that this machine cannot open, if anything.
///
/// The pumps behind the microphone and the speaker retry an open that fails, which is the right
/// answer to a device that vanished mid-call and the wrong one before a call exists: without this
/// check a machine with no microphone would ring the peer, connect, and carry silence with nothing
/// on screen to explain it. So the devices are looked for first, and their absence fails the call
/// with something the reader can act on instead of a call that looks healthy.
pub fn unavailable(microphone: Option<&str>, speaker: Option<&str>) -> Option<&'static str> {
    if !input_available(microphone) {
        return Some("microphone");
    }
    if !output_available(speaker) {
        return Some("speaker");
    }
    None
}

/// Whether the microphone a call would really open can be had.
fn input_available(selected: Option<&str>) -> bool {
    let listed: Option<Vec<String>> = rodio::microphone::available_inputs()
        .ok()
        .map(|inputs| inputs.into_iter().map(|input| input.to_string()).collect());
    chosen_available(selected, listed.as_deref(), || {
        rodio::microphone::MicrophoneBuilder::new()
            .default_device()
            .is_ok()
    })
}

/// Whether the speaker a call would really open can be had.
fn output_available(selected: Option<&str>) -> bool {
    let listed: Option<Vec<String>> =
        rodio::cpal::default_host()
            .output_devices()
            .ok()
            .map(|devices| {
                devices
                    .into_iter()
                    .filter_map(|device| device.description().ok().map(|d| d.name().to_owned()))
                    .collect()
            });
    chosen_available(selected, listed.as_deref(), || {
        rodio::DeviceSinkBuilder::from_default_device().is_ok()
    })
}

/// Whether the device a call would open can be had: the one the user picked, or the system default
/// when nothing is picked.
///
/// A named device is looked for in what discovery found, so a machine whose *default* is busy still
/// takes a call aimed at a device the user chose. The default is asked for only when nothing is
/// named, which is what the settings mean by `None`. `listed` is `None` when discovery could not
/// run at all, which is not the same as an empty machine: a broken helper must not refuse a call,
/// so a selection stands on it, the way `resolve_devices` keeps one.
fn chosen_available(
    selected: Option<&str>,
    listed: Option<&[String]>,
    default_ok: impl FnOnce() -> bool,
) -> bool {
    match selected {
        Some(name) => match listed {
            Some(names) => names.iter().any(|known| known == name),
            None => true,
        },
        None => default_ok(),
    }
}

/// Where the microphone's samples come from: this machine's device, or a test's stand-in.
///
/// The pump, the fallback rules and the framing above stay exactly as they are whichever this is,
/// so what a test covers is the real behaviour rather than a second implementation of it.
enum Mic {
    Real(rodio::microphone::Microphone),
    /// Installed only by a test, and only while its guard lives.
    #[cfg(test)]
    Fake(fake::Microphone),
}

impl Mic {
    fn channels(&self) -> u16 {
        match self {
            Mic::Real(microphone) => microphone.channels().get(),
            #[cfg(test)]
            Mic::Fake(microphone) => microphone.channels,
        }
    }

    fn rate(&self) -> u32 {
        match self {
            Mic::Real(microphone) => microphone.sample_rate().get(),
            #[cfg(test)]
            Mic::Fake(microphone) => microphone.rate,
        }
    }

    fn next(&mut self) -> Option<f32> {
        match self {
            Mic::Real(microphone) => microphone.next(),
            #[cfg(test)]
            Mic::Fake(microphone) => microphone.next(),
        }
    }
}

fn open_microphone(device: Option<&str>) -> Result<Mic, String> {
    #[cfg(test)]
    if let Some(microphone) = fake::open_microphone(device) {
        return Ok(Mic::Fake(microphone));
    }
    let builder = rodio::microphone::MicrophoneBuilder::new();
    let builder = match device {
        Some(name) => {
            let input = rodio::microphone::available_inputs()
                .map_err(|error| format!("the input devices could not be listed: {error}"))?
                .into_iter()
                .find(|input| input.to_string() == name)
                .ok_or_else(|| format!("the microphone {name} is not available"))?;
            builder
                .device(input)
                .map_err(|error| format!("the microphone {name} could not be used: {error}"))?
        }
        None => builder
            .default_device()
            .map_err(|error| format!("no microphone is available: {error}"))?,
    };
    builder
        .default_config()
        .map_err(|error| format!("the microphone has no supported format: {error}"))?
        .open_stream()
        .map(Mic::Real)
        .map_err(|error| format!("the microphone could not be opened: {error}"))
}

/// An open speaker. The sink has to outlive the player queued on it, or the stream closes with it.
enum Sink {
    Real {
        _device: rodio::MixerDeviceSink,
        player: rodio::Player,
        layout: Layout,
    },
    /// Installed only by a test, and only while its guard lives.
    #[cfg(test)]
    Fake(fake::Speaker),
}

fn open_sink(device: Option<&str>) -> Result<Sink, String> {
    #[cfg(test)]
    if let Some(speaker) = fake::open_speaker(device) {
        return Ok(Sink::Fake(speaker));
    }
    let sink = match device {
        Some(name) => {
            let output = rodio::cpal::default_host()
                .output_devices()
                .map_err(|error| format!("the output devices could not be listed: {error}"))?
                .find(|device| {
                    device
                        .description()
                        .ok()
                        .is_some_and(|description| description.name() == name)
                })
                .ok_or_else(|| format!("the speaker {name} is not available"))?;
            rodio::DeviceSinkBuilder::from_device(output)
                .map_err(|error| format!("the speaker {name} could not be used: {error}"))?
                .open_stream()
                .map_err(|error| format!("the speaker {name} could not be opened: {error}"))?
        }
        None => rodio::DeviceSinkBuilder::open_default_sink()
            .map_err(|error| format!("no speaker is available: {error}"))?,
    };
    let layout = Layout {
        rate: sink.config().sample_rate().get(),
        channels: sink.config().channel_count().get(),
    };
    let player = rodio::Player::connect_new(sink.mixer());
    Ok(Sink::Real {
        _device: sink,
        player,
        layout,
    })
}

impl Sink {
    /// The rate and channel count this device wants, which the engine's frames are converted to.
    fn layout(&self) -> Layout {
        match self {
            Sink::Real { layout, .. } => *layout,
            #[cfg(test)]
            Sink::Fake(speaker) => speaker.layout(),
        }
    }

    fn append(&self, samples: Vec<f32>) {
        match self {
            Sink::Real { player, layout, .. } => {
                player.append(SamplesBuffer::new(
                    NonZero::new(layout.channels).unwrap_or(NonZero::<u16>::MIN),
                    NonZero::new(layout.rate)
                        .unwrap_or(NonZero::new(48_000).expect("48 kHz is not zero")),
                    samples,
                ));
                player.play();
            }
            #[cfg(test)]
            Sink::Fake(speaker) => speaker.append(samples),
        }
    }

    /// How far the stream has played, which is how a stuck device is told from a busy one.
    fn position(&self) -> Duration {
        match self {
            Sink::Real { player, .. } => player.get_pos(),
            #[cfg(test)]
            Sink::Fake(speaker) => speaker.position(),
        }
    }

    /// Whether everything handed to the sink has been played.
    fn empty(&self) -> bool {
        match self {
            Sink::Real { player, .. } => player.empty(),
            #[cfg(test)]
            Sink::Fake(speaker) => speaker.empty(),
        }
    }
}

// ---------------------------------------------------------------------------
// The microphone
// ---------------------------------------------------------------------------

/// The microphone side: a channel the engine owns, refilled from whichever device is selected.
///
/// The channel is created once and handed to the engine once. A device change replaces the reader
/// behind it, still writing into that same channel, so nothing about the call's codec or stream
/// state is rebuilt for a device switch.
pub struct AudioInput {
    /// Asks the pump to restart on another device; dropping it stops the pump.
    swap: Option<async_channel::Sender<Option<String>>>,
    /// Set when the pump gave up on the selected device and reopened on the system default.
    pub fell_back: async_channel::Receiver<()>,
    /// How many readers this call's pump has opened.
    ///
    /// Nothing in a call reads it: it is how a test tells a device change, which rebinds the stream
    /// behind the engine's channel, from a reader that was restarted, and how it sees that a mute
    /// restarts nothing at all.
    pub opens: Arc<AtomicUsize>,
}

impl AudioInput {
    pub fn spawn(target: Option<String>) -> (Self, async_channel::Receiver<Vec<i16>>) {
        let (out, rx) = async_channel::bounded::<Vec<i16>>(4);
        // Unbounded so a device the user picks while the pump is busy is never dropped: a capacity
        // of one would silently discard the newer selection when two arrive before the pump reads
        // the first, and `bind` has no way to report that. The pump drains them in order, so the
        // last one wins and what the picker shows stays what the stream is bound to.
        let (swap, swaps) = async_channel::unbounded::<Option<String>>();
        let (fell, fell_back) = async_channel::bounded::<()>(1);
        // One per call, read by the test above and written by the pump below, so a reader that
        // survives a mute or a rebind is told from one that was started again.
        let opens = Arc::new(AtomicUsize::new(0));
        tokio::spawn(mic_pump(
            out,
            swaps,
            fell,
            target.clone(),
            Arc::clone(&opens),
        ));
        (
            Self {
                swap: Some(swap),
                fell_back,
                opens,
            },
            rx,
        )
    }

    /// Rebinds the microphone, keeping the engine's channel alive.
    pub fn bind(&self, target: Option<String>) {
        if let Some(swap) = &self.swap {
            let _ = swap.try_send(target);
        }
    }

    /// Stops the reader: dropping the swap sender is the pump's stop signal.
    pub fn stop(&mut self) {
        self.swap = None;
    }
}

async fn mic_pump(
    out: async_channel::Sender<Vec<i16>>,
    swaps: async_channel::Receiver<Option<String>>,
    fell: async_channel::Sender<()>,
    initial: Option<String>,
    opens: Arc<AtomicUsize>,
) {
    let mut target = initial;
    loop {
        let reader = match MicReader::start(target.as_deref()) {
            Ok(reader) => {
                opens.fetch_add(1, Ordering::Relaxed);
                reader
            }
            Err(error) => {
                log::error!("[CALL] microphone stream failed: {error}");
                if target.is_some() {
                    // A device that is gone will not come back by being asked again.
                    log::warn!(
                        "[CALL] microphone {target:?} could not be opened; using the default input"
                    );
                    target = None;
                    let _ = fell.try_send(());
                }
                tokio::time::sleep(RETRY).await;
                continue;
            }
        };
        let mut swapped = false;
        loop {
            tokio::select! {
                frame = reader.frames.recv() => {
                    match frame {
                        Ok(frame) => {
                            if out.send(frame).await.is_err() {
                                // The engine dropped its port: the call is over.
                                return;
                            }
                        }
                        Err(_) => break,
                    }
                }
                requested = swaps.recv() => {
                    match requested {
                        Ok(next) => {
                            log::info!("[CALL] microphone device changed to {next:?}");
                            target = next;
                        }
                        Err(_) => return,
                    }
                    swapped = true;
                    break;
                }
            }
        }
        let delivered = reader.delivered.load(Ordering::Relaxed);
        drop(reader);
        // A headset switched off leaves its stream delivering nothing: reopen on the system default
        // so the peer hears the call rather than silence, and let the call say what moved.
        if !delivered && !swapped && target.is_some() {
            log::warn!("[CALL] microphone {target:?} delivered nothing; using the default input");
            target = None;
            let _ = fell.try_send(());
        }
    }
}

/// Reads the microphone on its own thread, turning device frames into engine frames.
struct MicReader {
    stop: Arc<AtomicBool>,
    frames: async_channel::Receiver<Vec<i16>>,
    /// Set once a frame reached the engine, which tells a silent device from a gone one.
    delivered: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl MicReader {
    fn start(device: Option<&str>) -> Result<Self, String> {
        let (tx, rx) = async_channel::bounded::<Vec<i16>>(4);
        let stop = Arc::new(AtomicBool::new(false));
        let delivered = Arc::new(AtomicBool::new(false));
        let (ready, opened) = std::sync::mpsc::channel::<Result<(), String>>();
        let name = device.map(str::to_owned);
        let thread = {
            let stop = Arc::clone(&stop);
            let delivered = Arc::clone(&delivered);
            std::thread::Builder::new()
                .name("call-microphone".to_owned())
                .spawn(move || read_microphone(name.as_deref(), &tx, &stop, &delivered, &ready))
                .map_err(|error| format!("the microphone thread could not start: {error}"))?
        };
        match opened.recv() {
            Ok(Ok(())) => Ok(Self {
                stop,
                frames: rx,
                delivered,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => Err("the microphone thread ended before it opened a stream".to_owned()),
        }
    }
}

impl Drop for MicReader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // The thread owns the only sender. If it is parked waiting for room, the reader waking on
        // its stop flag ends it; nothing here can close the channel from this side.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn read_microphone(
    device: Option<&str>,
    tx: &async_channel::Sender<Vec<i16>>,
    stop: &AtomicBool,
    delivered: &AtomicBool,
    ready: &std::sync::mpsc::Sender<Result<(), String>>,
) {
    let mut microphone = match open_microphone(device) {
        Ok(microphone) => {
            let _ = ready.send(Ok(()));
            microphone
        }
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let mut converter = Converter::new(microphone.channels(), microphone.rate());
    let mut block = vec![0.0_f32; converter.block()];
    let mut frames = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        for slot in &mut block {
            match microphone.next() {
                Some(sample) => *slot = sample,
                // The device went away mid-read.
                None => return,
            }
        }
        frames.clear();
        converter.push(&block, &mut frames);
        for frame in frames.drain(..) {
            let mut pending = frame;
            loop {
                match tx.try_send(pending) {
                    Ok(()) => {
                        delivered.store(true, Ordering::Relaxed);
                        break;
                    }
                    Err(async_channel::TrySendError::Full(frame)) => {
                        if stop.load(Ordering::Relaxed) {
                            return;
                        }
                        pending = frame;
                        std::thread::sleep(BACKOFF);
                    }
                    Err(async_channel::TrySendError::Closed(_)) => return,
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The speaker
// ---------------------------------------------------------------------------

/// The speaker side: the engine's frames written to whichever device is selected.
pub struct AudioOutput {
    swap: Option<async_channel::Sender<Option<String>>>,
    /// Set when the pump gave up on the selected device and reopened on the system default.
    pub fell_back: async_channel::Receiver<()>,
    /// How many writer streams this call's pump has opened.
    ///
    /// Nothing in a call reads it: it is how a test tells a device change, which rebinds the stream
    /// behind the engine's channel, from a writer that was torn down and restarted, and how the
    /// call's own diagnostics tell a sink that is being restarted from one that is simply playing.
    pub opens: Arc<AtomicUsize>,
    /// How many times a stream that had been playing was restarted after it stopped draining.
    pub stalls: Arc<AtomicUsize>,
    /// How many times the sink was torn down and reopened for any reason, played or not.
    ///
    /// Kept apart from `stalls` because the two answer different questions: `stalls` is a device
    /// that was playing and stopped, and this is every restart, so a device that never plays at all
    /// is visible as a run of opens rather than as no stall at all.
    pub restarts: Arc<AtomicUsize>,
    /// When the current grace window closes, if one is open.
    relink: Arc<Mutex<Option<Instant>>>,
}

impl AudioOutput {
    pub fn spawn(target: Option<String>) -> (Self, async_channel::Sender<Vec<i16>>) {
        let (tx, rx) = async_channel::bounded::<Vec<i16>>(SPEAKER_QUEUE);
        // Unbounded for the same reason the microphone's is: a newer device selection must not be
        // dropped because the previous one has not been read yet.
        let (swap, swaps) = async_channel::unbounded::<Option<String>>();
        let (fell, fell_back) = async_channel::bounded::<()>(1);
        let opens = Arc::new(AtomicUsize::new(0));
        let stalls = Arc::new(AtomicUsize::new(0));
        let restarts = Arc::new(AtomicUsize::new(0));
        let relink = Arc::new(Mutex::new(None));
        tokio::spawn(play_pump(
            rx,
            swaps,
            fell,
            target.clone(),
            SpeakerHealth {
                opens: Arc::clone(&opens),
                stalls: Arc::clone(&stalls),
                restarts: Arc::clone(&restarts),
                relink: Arc::clone(&relink),
            },
        ));
        (
            Self {
                swap: Some(swap),
                fell_back,
                opens,
                stalls,
                restarts,
                relink,
            },
            tx,
        )
    }

    /// Whether a relink window is open right now, which is what a call's own test asks.
    #[cfg(test)]
    pub(crate) fn relink_open(&self) -> bool {
        relink_left(&self.relink).is_some()
    }

    /// Says a new media stream has just been opened or closed here, which relinks the streams
    /// already on this machine's audio graph.
    ///
    /// Called by the call when the camera starts or stops, so the sink is not judged stalled while
    /// the graph settles around the new stream. It only ever widens a window; it can neither stop
    /// nor restart anything by itself.
    pub fn relink(&self) {
        let mut until = self
            .relink
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *until = Some(Instant::now() + RELINK_GRACE);
    }

    /// Rebinds the speaker, keeping the engine's channel alive.
    pub fn bind(&self, target: Option<String>) {
        if let Some(swap) = &self.swap {
            let _ = swap.try_send(target);
        }
    }

    /// Stops the writer: dropping the swap sender is the pump's stop signal.
    pub fn stop(&mut self) {
        self.swap = None;
    }
}

/// What one call's speaker keeps about itself, handed to the pump as one thing.
///
/// Three counters and a window, which are four questions about one stream: how many were opened,
/// how many that had played stopped draining, how many were reopened at all, and how long a stream
/// the call just opened or closed is still being excused from the stall clock. Kept together so the
/// pump's signature stays about the audio rather than about bookkeeping.
#[derive(Clone)]
struct SpeakerHealth {
    opens: Arc<AtomicUsize>,
    stalls: Arc<AtomicUsize>,
    restarts: Arc<AtomicUsize>,
    relink: Arc<Mutex<Option<Instant>>>,
}

async fn play_pump(
    rx: async_channel::Receiver<Vec<i16>>,
    swaps: async_channel::Receiver<Option<String>>,
    fell: async_channel::Sender<()>,
    initial: Option<String>,
    health: SpeakerHealth,
) {
    let SpeakerHealth {
        opens,
        stalls,
        restarts,
        relink,
    } = health;
    let mut target = initial;
    // Restarts in a row that never played a sample. Reset by any stream that plays one, so this
    // counts a device that is not there rather than a busy one.
    let mut unplayed = 0_usize;
    loop {
        let writer = match SpkWriter::start(target.as_deref(), Arc::clone(&relink)) {
            Ok(writer) => {
                opens.fetch_add(1, Ordering::Relaxed);
                writer
            }
            Err(error) => {
                log::error!("[CALL] speaker stream failed: {error}");
                if target.is_some() {
                    log::warn!(
                        "[CALL] speaker {target:?} could not be opened; using the default output"
                    );
                    target = None;
                    let _ = fell.try_send(());
                }
                tokio::time::sleep(RETRY).await;
                continue;
            }
        };
        let mut finished = false;
        let mut swapped = false;
        loop {
            tokio::select! {
                frame = rx.recv() => {
                    match frame {
                        Ok(frame) => {
                            if writer.send(frame).await.is_err() {
                                // The writer ended, either from a stall or from its device going away.
                                break;
                            }
                        }
                        Err(_) => {
                            finished = true;
                            break;
                        }
                    }
                }
                requested = swaps.recv() => {
                    match requested {
                        Ok(next) => {
                            log::info!("[CALL] speaker device changed to {next:?}");
                            target = next;
                        }
                        Err(_) => finished = true,
                    }
                    swapped = true;
                    break;
                }
            }
        }
        let accepted = writer.accepted.load(Ordering::Relaxed);
        let stalled = writer.stalled.load(Ordering::Relaxed);
        let played = writer.played.load(Ordering::Relaxed);
        drop(writer);
        if finished {
            return;
        }
        if stalled && accepted {
            restarts.fetch_add(1, Ordering::Relaxed);
            if played {
                unplayed = 0;
                // It had been playing, so this is a device that stopped draining rather than one
                // that never started: counted, because that is the difference between a call whose
                // audio went quiet on its own and one that never had any.
                stalls.fetch_add(1, Ordering::Relaxed);
            } else {
                unplayed += 1;
                log::warn!(
                    "[CALL] speaker stream was reopened without playing anything ({unplayed} in a row)"
                );
            }
            // A device that has taken frames without playing one, three streams running, is not a
            // device that is about to start: it is a name this machine no longer answers to. Going
            // back to the default output is the only thing left that can make the call audible, and
            // saying so is what turns a silent call into a reportable one.
            if unplayed >= UNPLAYED_RESTARTS_BEFORE_DEFAULT && target.is_some() {
                log::warn!(
                    "[CALL] speaker {target:?} has played nothing across {unplayed} streams; using the default output"
                );
                target = None;
                unplayed = 0;
                let _ = fell.try_send(());
            }
            // A stream that played frames was merely stalled, so it keeps the device the user
            // picked rather than being demoted to the system default over one bad moment. What is
            // queued is already older than the restart, and playing it late would only add delay,
            // so it is dropped and the engine's channel refills at its own pace.
            while rx.try_recv().is_ok() {}
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        }
        if played {
            unplayed = 0;
        }
        // A sink that accepted no audio was pointing at a device that is gone: reopen on the system
        // default instead of retrying a target that will never take a frame.
        if !accepted && !swapped && target.is_some() {
            log::warn!("[CALL] speaker {target:?} accepted nothing; using the default output");
            target = None;
            let _ = fell.try_send(());
        }
    }
}

/// Writes engine frames to the speaker on its own thread.
struct SpkWriter {
    stop: Arc<AtomicBool>,
    frames: async_channel::Sender<Vec<i16>>,
    /// Set once a frame reached the sink, which tells a device that is gone from one that is quiet.
    accepted: Arc<AtomicBool>,
    /// Set when the sink stopped draining, so the pump restarts it.
    stalled: Arc<AtomicBool>,
    /// Set once the stream has played, which is what tells a stream that never started from one
    /// that was playing and stopped.
    played: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SpkWriter {
    fn start(device: Option<&str>, relink: Arc<Mutex<Option<Instant>>>) -> Result<Self, String> {
        let (tx, rx) = async_channel::bounded::<Vec<i16>>(SPEAKER_QUEUE);
        let stop = Arc::new(AtomicBool::new(false));
        let accepted = Arc::new(AtomicBool::new(false));
        let stalled = Arc::new(AtomicBool::new(false));
        let played = Arc::new(AtomicBool::new(false));
        let (ready, opened) = std::sync::mpsc::channel::<Result<(), String>>();
        let name = device.map(str::to_owned);
        let thread = {
            let stop = Arc::clone(&stop);
            let accepted = Arc::clone(&accepted);
            let played = Arc::clone(&played);
            let stalled = Arc::clone(&stalled);
            std::thread::Builder::new()
                .name("call-speaker".to_owned())
                .spawn(move || {
                    write_speaker(
                        name.as_deref(),
                        &rx,
                        &stop,
                        SpeakerFlags {
                            accepted: &accepted,
                            stalled: &stalled,
                            played: &played,
                        },
                        &relink,
                        &ready,
                    )
                })
                .map_err(|error| format!("the speaker thread could not start: {error}"))?
        };
        match opened.recv() {
            Ok(Ok(())) => Ok(Self {
                stop,
                frames: tx,
                accepted,
                stalled,
                played,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => Err("the speaker thread ended before it opened a stream".to_owned()),
        }
    }

    async fn send(&self, frame: Vec<i16>) -> Result<(), ()> {
        self.frames.send(frame).await.map_err(|_| ())
    }
}

impl Drop for SpkWriter {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Closing the channel wakes a writer parked in `recv_blocking`.
        self.frames.close();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The three flags one speaker stream reports itself through.
///
/// A struct because each is one fact about the same stream, and the writer's signature is about the
/// audio rather than about how many `AtomicBool`s it takes to describe it.
struct SpeakerFlags<'a> {
    /// Set once a frame reached the sink, which tells a device that is gone from one that is quiet.
    accepted: &'a AtomicBool,
    /// Set when the sink stopped draining, so the pump restarts it.
    stalled: &'a AtomicBool,
    /// Set once the stream has played, which is what tells a stream that never started from one
    /// that was playing and stopped.
    played: &'a AtomicBool,
}

fn write_speaker(
    device: Option<&str>,
    frames: &async_channel::Receiver<Vec<i16>>,
    stop: &AtomicBool,
    flags: SpeakerFlags<'_>,
    relink: &Mutex<Option<Instant>>,
    ready: &std::sync::mpsc::Sender<Result<(), String>>,
) {
    let SpeakerFlags {
        accepted,
        stalled,
        played,
    } = flags;
    let sink = match open_sink(device) {
        Ok(sink) => {
            let _ = ready.send(Ok(()));
            sink
        }
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let mut last_position = Duration::ZERO;
    let mut quiet_since: Option<Instant> = None;
    while !stop.load(Ordering::Relaxed) {
        let frame = match frames.recv_blocking() {
            Ok(frame) => frame,
            // The pump dropped its end: the call is over, or the device is being changed.
            Err(_) => return,
        };
        if stop.load(Ordering::Relaxed) {
            return;
        }
        sink.append(sink.layout().convert(&frame));
        accepted.store(true, Ordering::Relaxed);
        let position = sink.position();
        if position > last_position {
            // The stream is playing, so from here anything that stops it is a stall rather than a
            // start-up that is still in progress.
            played.store(true, Ordering::Relaxed);
            quiet_since = None;
        } else if !sink.empty() {
            // Nothing has played while audio is queued. A stream that has never played gets the
            // whole start-up window; one that was playing gets the stall window, which is where
            // the peer's voice would otherwise be stuck behind a device that stopped draining.
            //
            // Either window is widened while a stream the call just opened or closed is still
            // relinking this machine's audio graph: that pause is the graph's, not the device's,
            // and tearing the stream down over it is what took the peer's voice away with the
            // camera on.
            let base = if played.load(Ordering::Relaxed) {
                STALLED
            } else {
                STARTUP
            };
            let limit = match relink_left(relink) {
                Some(left) => base.max(left),
                None => base,
            };
            let since = *quiet_since.get_or_insert_with(Instant::now);
            if since.elapsed() > limit {
                log::warn!(
                    "[CALL] the speaker has not played for {:?}; reopening the stream",
                    since.elapsed()
                );
                stalled.store(true, Ordering::Relaxed);
                return;
            }
        } else {
            quiet_since = None;
        }
        last_position = position;
    }
}

/// How long the sink is still excused from the stall clock, if a relink is in flight.
fn relink_left(relink: &Mutex<Option<Instant>>) -> Option<Duration> {
    let guard = relink.lock().unwrap_or_else(|error| error.into_inner());
    (*guard)?.checked_duration_since(Instant::now())
}

/// A device backend a test installs in place of this machine's devices.
///
/// Only the device at the bottom is replaced. The pumps, the fallback rules, the framing and the
/// stall handling above stay exactly what a call runs, so these tests cover the real behaviour
/// instead of a second implementation of it, and a machine with no microphone or no speaker no
/// longer decides what they see.
#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::sync::Mutex;
    use std::sync::MutexGuard;
    use std::sync::atomic::AtomicUsize;

    /// The microphone a test supplies: one block of samples, repeated for as long as it is read.
    pub(crate) struct Microphone {
        samples: Vec<f32>,
        pub(super) channels: u16,
        pub(super) rate: u32,
        at: usize,
        /// Set to make the device behave as one that has been taken away.
        gone: bool,
    }

    impl Microphone {
        pub(super) fn next(&mut self) -> Option<f32> {
            if self.gone || self.samples.is_empty() {
                return None;
            }
            let sample = self.samples[self.at % self.samples.len()];
            self.at = self.at.wrapping_add(1);
            Some(sample)
        }
    }

    /// What a test installed, if anything.
    struct Spec {
        samples: Vec<f32>,
        channels: u16,
        rate: u32,
        gone: bool,
    }

    /// The speaker a test supplies: what it was given, how far it played, and whether it drains.
    #[derive(Clone)]
    pub(crate) struct Speaker {
        rate: u32,
        channels: u16,
        /// Every block the engine handed over, already in this device's layout.
        pub(crate) written: Arc<Mutex<Vec<Vec<f32>>>>,
        /// Every open, and the device each one asked for, which tells a rebind from a restart.
        pub(crate) opened: Arc<Mutex<Vec<Option<String>>>>,
        /// Cleared to make the sink stop draining, which is what a stuck device looks like.
        pub(crate) drains: Arc<AtomicBool>,
        /// How many blocks this device has played, so a test can tell a restart from a rebind.
        played_count: Arc<AtomicUsize>,
        /// Where the stream says it has played to.
        played: Arc<Mutex<Duration>>,
        /// How long an open is made to take, so a test can hold the pump still while it sends
        /// device selections and prove none of them is dropped.
        pub(crate) open_pause: Arc<AtomicUsize>,
    }

    impl Speaker {
        /// Every block the engine has handed over so far, in this device's layout.
        pub(crate) fn written(&self) -> Vec<Vec<f32>> {
            self.written
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone()
        }

        /// How many blocks reached this device.
        pub(crate) fn written_count(&self) -> usize {
            self.written
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .len()
        }

        /// How many times a device was opened, which counts a restart but not a rebind.
        pub(crate) fn opened_count(&self) -> usize {
            self.opened
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .len()
        }

        /// The device each open asked for.
        pub(crate) fn opened(&self) -> Vec<Option<String>> {
            self.opened
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone()
        }

        /// How many blocks this device has played.
        pub(crate) fn played_count(&self) -> usize {
            self.played_count.load(Ordering::Relaxed)
        }

        pub(super) fn layout(&self) -> Layout {
            Layout {
                rate: self.rate,
                channels: self.channels,
            }
        }

        pub(super) fn append(&self, samples: Vec<f32>) {
            if self.drains.load(Ordering::Relaxed) {
                // Everything given to a draining device is played, so time moves with the samples.
                let seconds =
                    samples.len() as f64 / (f64::from(self.rate) * f64::from(self.channels.max(1)));
                let mut played = self
                    .played
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                *played += Duration::from_secs_f64(seconds);
                self.played_count.fetch_add(1, Ordering::Relaxed);
            }
            self.written
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(samples);
        }

        pub(super) fn position(&self) -> Duration {
            *self
                .played
                .lock()
                .unwrap_or_else(|error| error.into_inner())
        }

        pub(super) fn empty(&self) -> bool {
            self.drains.load(Ordering::Relaxed)
        }
    }

    /// What is installed right now.
    struct Installed {
        microphone: Option<Spec>,
        speaker: Option<Speaker>,
    }

    static INSTALLED: Mutex<Option<Installed>> = Mutex::new(None);
    /// One test at a time: the backend is process-wide, so this is what keeps two tests from
    /// deciding what the other one's pumps open.
    static ALONE: Mutex<()> = Mutex::new(());

    /// A test's devices, installed for as long as this guard lives.
    pub(crate) struct Fixture {
        _alone: MutexGuard<'static, ()>,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            *INSTALLED.lock().unwrap_or_else(|error| error.into_inner()) = None;
        }
    }

    /// Installs a microphone and a speaker for the length of the returned guard.
    ///
    /// The microphone is `samples` repeated at its own rate and channel count, and the speaker is
    /// whatever a test asks for: `drains` starts set, so the two flags on it are how a stuck device
    /// is staged.
    pub(crate) fn install(
        samples: Vec<f32>,
        channels: u16,
        rate: u32,
        speaker: (u32, u16),
    ) -> (Fixture, Speaker) {
        let alone = ALONE.lock().unwrap_or_else(|error| error.into_inner());
        let speaker = Speaker {
            rate: speaker.0,
            channels: speaker.1,
            written: Arc::new(Mutex::new(Vec::new())),
            opened: Arc::new(Mutex::new(Vec::new())),
            drains: Arc::new(AtomicBool::new(true)),
            played_count: Arc::new(AtomicUsize::new(0)),
            played: Arc::new(Mutex::new(Duration::ZERO)),
            open_pause: Arc::new(AtomicUsize::new(0)),
        };
        *INSTALLED.lock().unwrap_or_else(|error| error.into_inner()) = Some(Installed {
            microphone: Some(Spec {
                samples,
                channels,
                rate,
                gone: false,
            }),
            speaker: Some(speaker.clone()),
        });
        (Fixture { _alone: alone }, speaker)
    }

    /// The microphone to hand a reader, if a test installed one.
    pub(super) fn open_microphone(_device: Option<&str>) -> Option<Microphone> {
        let installed = INSTALLED.lock().unwrap_or_else(|error| error.into_inner());
        let spec = installed.as_ref()?.microphone.as_ref()?;
        Some(Microphone {
            samples: spec.samples.clone(),
            channels: spec.channels,
            rate: spec.rate,
            at: 0,
            gone: spec.gone,
        })
    }

    /// The speaker to hand a writer, if a test installed one. Every open is recorded, which is how
    /// a rebind is told from a restart.
    pub(super) fn open_speaker(device: Option<&str>) -> Option<Speaker> {
        // The install lock is not held across the pause, so a slow open cannot block another
        // thread that needs the fixture.
        let speaker = {
            let installed = INSTALLED.lock().unwrap_or_else(|error| error.into_inner());
            installed.as_ref()?.speaker.clone()?
        };
        // The pause holds the pump inside this open, so a test that sets one can send selections
        // that the pump cannot yet read.
        let pause = speaker.open_pause.load(Ordering::Relaxed);
        if pause > 0 {
            std::thread::sleep(Duration::from_millis(pause as u64));
        }
        speaker
            .opened
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(device.map(str::to_owned));
        Some(speaker)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_mixes_every_channel_in() {
        assert_eq!(mono(&[1.0, 0.0, 0.0, 1.0], 2), vec![0.5, 0.5]);
        assert_eq!(mono(&[0.25, 0.5, 0.75], 1), vec![0.25, 0.5, 0.75]);
        // The part of a frame that is there is averaged with itself, not stretched.
        assert_eq!(mono(&[1.0, 0.0, 1.0], 2), vec![0.5, 1.0]);
        // A zero channel count is treated as the single channel it must be.
        assert_eq!(mono(&[0.5, 0.25], 0), vec![0.5, 0.25]);
    }

    #[test]
    fn resampling_moves_between_the_call_and_device_rates() {
        let up = resample(&vec![0.5_f32; RATE as usize], RATE, 48_000);
        assert_eq!(up.len(), 48_000);
        assert!(up.iter().all(|sample| (*sample - 0.5).abs() < 1e-6));

        let down = resample(&vec![0.5_f32; 48_000], 48_000, RATE);
        assert_eq!(down.len(), RATE as usize);
        assert!(down.iter().all(|sample| (*sample - 0.5).abs() < 1e-6));

        // An odd device rate lands on the call's rate too, without a frame's worth of drift.
        assert_eq!(
            resample(&vec![0.0; 44_100], 44_100, RATE).len(),
            RATE as usize
        );
        // Nothing to convert, and nothing to convert at the same rate.
        assert!(resample(&[], 48_000, RATE).is_empty());
        assert_eq!(resample(&[0.5, 0.25], 48_000, 48_000), vec![0.5, 0.25]);
    }

    #[test]
    fn resampling_keeps_the_pitch_and_the_level() {
        let rate = 48_000;
        let tone: Vec<f32> = (0..rate)
            .map(|index| (index as f32 * 1_000.0 * std::f32::consts::TAU / rate as f32).sin())
            .collect();
        let at_call_rate = resample(&tone, rate, RATE);
        assert_eq!(at_call_rate.len(), RATE as usize);
        // A 1 kHz tone crosses zero on the way up 1,000 times a second at either rate.
        let rising = at_call_rate
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        assert!((995..=1_005).contains(&rising), "{rising} rising crossings");
        let peak = at_call_rate
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        assert!(peak > 0.99, "peak {peak}");
    }

    #[test]
    fn device_audio_becomes_whole_engine_frames() {
        let mut converter = Converter::new(1, 48_000);
        assert_eq!(converter.block(), 960, "20 ms at 48 kHz");
        let mut frames = Vec::new();
        for _ in 0..3 {
            converter.push(&vec![0.5; 960], &mut frames);
        }
        assert_eq!(frames.len(), 1, "three blocks are one 60 ms frame");
        assert_eq!(frames[0].len(), FRAME_SAMPLES);
        let expected = (0.5 * f32::from(i16::MAX)) as i16;
        assert!(frames[0].iter().all(|sample| *sample == expected));
        assert!(converter.pending.is_empty(), "no sample is lost or doubled");
        for _ in 0..3 {
            converter.push(&vec![0.5; 960], &mut frames);
        }
        assert_eq!(frames.len(), 2);
        assert!(converter.pending.is_empty());
    }

    #[test]
    fn a_leftover_read_is_carried_into_the_next_frame() {
        let mut converter = Converter::new(1, 48_000);
        let mut frames = Vec::new();
        // Half a block is not a frame, and nothing is emitted early.
        converter.push(&vec![0.0; 480], &mut frames);
        assert!(frames.is_empty());
        assert_eq!(
            converter.pending.len(),
            160,
            "half a block at the call rate"
        );
        converter.push(&vec![0.0; 480], &mut frames);
        assert!(frames.is_empty());
        assert_eq!(converter.pending.len(), 320);
        // The rest of the frame arrives and comes out as one whole frame.
        for _ in 0..4 {
            converter.push(&vec![0.0; 480], &mut frames);
        }
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].len(), FRAME_SAMPLES);
        assert!(converter.pending.is_empty());
    }

    #[test]
    fn stereo_device_audio_is_mixed_before_it_is_resampled() {
        let mut converter = Converter::new(2, 48_000);
        assert_eq!(converter.block(), 1_920, "20 ms of two channels");
        let mut frames = Vec::new();
        for _ in 0..3 {
            converter.push(&vec![1.0; 1_920], &mut frames);
        }
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0][0], i16::MAX);
    }

    #[test]
    fn the_speaker_gets_the_layout_its_device_wants() {
        let frame: Vec<i16> = (0..FRAME_SAMPLES)
            .map(|index| if index % 2 == 0 { 16_384 } else { -16_384 })
            .collect();
        // A mono device at the call's own rate gets the frame back, scaled to `f32`.
        let same = Layout {
            rate: RATE,
            channels: 1,
        }
        .convert(&frame);
        assert_eq!(same.len(), FRAME_SAMPLES);
        assert!((same[0] - 0.5).abs() < 1e-3 && (same[1] + 0.5).abs() < 1e-3);

        // A 48 kHz stereo device gets three samples per input sample, on both channels.
        let wide = Layout {
            rate: 48_000,
            channels: 2,
        }
        .convert(&frame);
        assert_eq!(wide.len(), FRAME_SAMPLES * 6);
        for pair in wide.chunks(2) {
            assert_eq!(pair[0], pair[1], "both channels carry the same mono sample");
        }

        // An odd rate still yields the samples the device's own rate asks for.
        let odd = Layout {
            rate: 44_100,
            channels: 1,
        }
        .convert(&frame);
        assert_eq!(
            odd.len(),
            (FRAME_SAMPLES as f64 * 44_100.0 / RATE as f64).floor() as usize
        );
    }

    #[test]
    fn a_device_list_keeps_one_entry_per_name() {
        let listed = dedupe(vec![
            ("same".to_owned(), "The same device twice".to_owned()),
            ("same".to_owned(), "The same device twice".to_owned()),
            ("other".to_owned(), "Another device".to_owned()),
        ]);
        assert_eq!(listed.len(), 2, "a name a stream accepts back appears once");
        assert_eq!(listed[0].0, "other", "sorted by the label a person reads");
        assert_eq!(listed[1].0, "same");
    }

    /// The engine's channel carries whole 60 ms frames of the microphone no matter what the device
    /// hands over, and its signal really is in them.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_test_microphone_reaches_the_engine_as_whole_frames() {
        // 48 kHz stereo, which is what most devices give, with a ramp so the frames carry a signal
        // rather than silence nobody could tell from a dead device.
        let samples: Vec<f32> = (0..960).map(|sample| sample as f32 / 1_920.0).collect();
        let (_devices, _speaker) = fake::install(samples, 2, 48_000, (48_000, 2));
        let (input, frames) = AudioInput::spawn(None);
        let mut got: Vec<Vec<i16>> = Vec::new();
        let started = Instant::now();
        while got.len() < 5 && started.elapsed() < Duration::from_secs(5) {
            match tokio::time::timeout(Duration::from_millis(250), frames.recv()).await {
                Ok(Ok(frame)) => got.push(frame),
                // The pump dropped its port, which is the call ending rather than a slow frame.
                Ok(Err(_)) => break,
                Err(_) => {}
            }
        }
        drop(input);
        assert!(got.len() >= 5, "the engine got {} frames", got.len());
        for frame in &got {
            assert_eq!(
                frame.len(),
                FRAME_SAMPLES,
                "every frame is 60 ms at the engine's rate"
            );
        }
        assert!(
            got.iter().flatten().any(|sample| *sample != 0),
            "the frames carry what the microphone delivered"
        );
    }

    /// The engine's frames reach the speaker at the device's own rate and channel count.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn engine_frames_reach_a_test_speaker_at_its_own_shape() {
        let (_devices, speaker) = fake::install(vec![0.0; 8], 1, RATE, (48_000, 2));
        let (output, tx) = AudioOutput::spawn(None);
        let frame: Vec<i16> = (0..FRAME_SAMPLES)
            .map(|at| (at % 101) as i16 * 30)
            .collect();
        let started = Instant::now();
        while speaker.written_count() < 4 && started.elapsed() < Duration::from_secs(5) {
            let _ = tx.try_send(frame.clone());
            std::thread::sleep(Duration::from_millis(20));
        }
        drop(output);
        let written = speaker.written();
        assert!(
            written.len() >= 4,
            "the speaker got {} blocks",
            written.len()
        );
        for block in written.iter().take(4) {
            // 960 samples at 16 kHz are 2,880 at 48 kHz, and two channels carry each of them.
            assert_eq!(
                block.len(),
                FRAME_SAMPLES * 3 * 2,
                "the device's own layout"
            );
            assert!(
                block.iter().any(|sample| *sample != 0.0),
                "what the engine sent arrives, not silence"
            );
        }
    }

    /// A device change rebinds the sink and leaves the channel the engine holds alone.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_device_change_rebinds_the_speaker_and_keeps_the_engine_channel() {
        let (_devices, speaker) = fake::install(vec![0.0; 8], 1, RATE, (48_000, 2));
        let (output, tx) = AudioOutput::spawn(None);
        let frame = vec![100_i16; FRAME_SAMPLES];
        let started = Instant::now();
        while speaker.opened_count() < 1 && started.elapsed() < Duration::from_secs(5) {
            let _ = tx.try_send(frame.clone());
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(speaker.opened_count(), 1);
        output.bind(Some("Second speaker".to_owned()));
        let started = Instant::now();
        while speaker.opened_count() < 2 && started.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            speaker.opened()[1].as_deref(),
            Some("Second speaker"),
            "the device the call asked for is the one opened"
        );
        // The very same sender still reaches the new writer, which is the point of a rebind rather
        // than a restart: the engine's codec and stream state are untouched by a device change.
        let before = speaker.written_count();
        let started = Instant::now();
        while speaker.written_count() <= before && started.elapsed() < Duration::from_secs(5) {
            let _ = tx.try_send(frame.clone());
            std::thread::sleep(Duration::from_millis(20));
        }
        drop(output);
        assert!(
            speaker.written_count() > before,
            "the engine's channel survived the device change"
        );
    }

    /// A device the user picks is validated against what exists, not against the system default.
    ///
    /// A machine whose default microphone is busy — another app holding it, a headset that
    /// switched off — still takes a call aimed at a device the user chose. Asking only about the
    /// default refused the call before it rang, even though the selected device was right there.
    #[test]
    fn a_named_device_is_checked_against_what_exists_not_the_default() {
        let listed = vec!["mic-a".to_owned(), "mic-b".to_owned()];
        // A device the user picked is available when it is there, whatever the default is doing.
        assert!(chosen_available(Some("mic-b"), Some(&listed), || false));
        // And refused when the machine really does not have it.
        assert!(!chosen_available(Some("mic-z"), Some(&listed), || true));
        // The default is consulted only when nothing is named.
        assert!(chosen_available(None, Some(&listed), || true));
        assert!(!chosen_available(None, Some(&listed), || false));
        // Discovery that could not run is not an empty machine: the selection stands on it.
        assert!(chosen_available(Some("mic-z"), None, || false));
    }

    /// A second device selection made before the pump has read the first is not lost.
    ///
    /// The picker can move twice in one stroke — a keyboard-driven list, or an impatient second
    /// click — and the stream must end up on the device the picker shows rather than on the one the
    /// pump happened to read first. With a capacity-one channel and an ignored `try_send`, the
    /// newer selection vanished while `Call::set_microphone` had already published it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_second_device_selection_is_not_lost_before_the_first_is_read() {
        let (_devices, speaker) = fake::install(vec![0.0; 8], 1, RATE, (RATE, 1));
        // Hold the pump inside its first open long enough for two selections to be sent while it
        // cannot read them.
        speaker.open_pause.store(400, Ordering::Relaxed);
        let (output, _tx) = AudioOutput::spawn(None);
        tokio::time::sleep(Duration::from_millis(80)).await;
        output.bind(Some("First".to_owned()));
        output.bind(Some("Second".to_owned()));
        let started = Instant::now();
        while speaker.opened_count() < 3 && started.elapsed() < Duration::from_secs(5) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let opened = speaker.opened();
        drop(output);
        assert_eq!(
            opened.last().and_then(|device| device.as_deref()),
            Some("Second"),
            "the last device the user picked is the one in use: {opened:?}"
        );
    }

    /// A sink that has not played yet is given the start-up window rather than the stall window.
    ///
    /// A suspended device, a Bluetooth headset still connecting, or an audio graph that a new
    /// stream has just relinked is slow, not dead. Judging it by the stall clock tore the stream
    /// down from under the audio that was about to play and reopened it in a loop, so a call lost
    /// its sound to its own recovery while the microphone, the video and the signaling all kept
    /// working. This is that regression: a device that comes up after the stall window is never
    /// restarted, and everything queued behind it plays.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_speaker_that_is_slow_to_start_is_not_restarted() {
        let (_devices, speaker) = fake::install(vec![0.0; 8], 1, RATE, (RATE, 1));
        // Nothing plays yet, which is what a device still coming up looks like.
        speaker.drains.store(false, Ordering::Relaxed);
        let (output, tx) = AudioOutput::spawn(None);
        let frame = vec![100_i16; FRAME_SAMPLES];
        // Well past the stall window that used to decide, and inside the start-up one.
        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(900) {
            let _ = tx.try_send(frame.clone());
            std::thread::sleep(Duration::from_millis(20));
        }
        let opens = output.opens.load(Ordering::Relaxed);
        assert_eq!(
            opens, 1,
            "a stream that has not played yet must not be torn down"
        );
        // It comes up, and what is queued behind it plays on the stream that was already there.
        speaker.drains.store(true, Ordering::Relaxed);
        let before = speaker.played_count();
        let started = Instant::now();
        while speaker.played_count() <= before && started.elapsed() < Duration::from_secs(5) {
            let _ = tx.try_send(frame.clone());
            std::thread::sleep(Duration::from_millis(20));
        }
        let (opens, stalls) = (
            output.opens.load(Ordering::Relaxed),
            output.stalls.load(Ordering::Relaxed),
        );
        drop(output);
        assert!(speaker.played_count() > before, "the slow device played");
        assert_eq!(opens, 1, "and it played on the stream it started with");
        assert_eq!(stalls, 0, "a slow start is not a stall");
    }

    /// A sink that pauses while a new stream relinks this machine's audio graph is left alone.
    ///
    /// This is the reported regression, and it is why the grace window exists: a voice call was
    /// playing, the camera came on, the graph relinked every stream on it, and the sink stopped
    /// draining for longer than the stall window. The call tore its own player down over that
    /// pause, dropped the peer's audio queued behind it, and reopened into the same relink, so the
    /// peer went quiet with the picture and the microphone still working. A sink can still move no
    /// samples for most of a second after the call says a stream came or went, and it must not be
    /// restarted for it: it is the graph, not the device.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_relink_does_not_let_the_call_tear_down_its_own_audio() {
        let (_devices, speaker) = fake::install(vec![0.0; 8], 1, RATE, (RATE, 1));
        let (output, tx) = AudioOutput::spawn(None);
        let frame = vec![100_i16; FRAME_SAMPLES];
        let started = Instant::now();
        while speaker.played_count() < 2 && started.elapsed() < Duration::from_secs(5) {
            let _ = tx.try_send(frame.clone());
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(speaker.played_count() >= 2, "the call was playing");
        // The camera opens: the graph relinks and this stream stops draining, well past the stall
        // window that used to decide.
        output.relink();
        speaker.drains.store(false, Ordering::Relaxed);
        let quiet = Instant::now();
        while quiet.elapsed() < Duration::from_millis(900) {
            let _ = tx.try_send(frame.clone());
            std::thread::sleep(Duration::from_millis(20));
        }
        let (opens, stalls) = (
            output.opens.load(Ordering::Relaxed),
            output.stalls.load(Ordering::Relaxed),
        );
        assert_eq!(opens, 1, "a relink must not reopen the sink");
        assert_eq!(stalls, 0, "and it is not a stall");
        // The graph settles and the same stream plays on, with nothing restarted or rebound.
        speaker.drains.store(true, Ordering::Relaxed);
        let before = speaker.played_count();
        let started = Instant::now();
        while speaker.played_count() <= before && started.elapsed() < Duration::from_secs(5) {
            let _ = tx.try_send(frame.clone());
            std::thread::sleep(Duration::from_millis(20));
        }
        drop(output);
        assert!(speaker.played_count() > before, "the call kept its sound");
        assert_eq!(speaker.opened_count(), 1, "on the stream it already had");
    }

    /// A device that takes frames and never plays one is given up for the system default.
    ///
    /// This is the other half of the same failure: a reopened stream that never plays leaves the
    /// call silent for its whole length while every counter that watches the engine says audio is
    /// arriving. After three streams that played nothing, the pump stops believing the name it was
    /// given rather than retrying it forever.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_device_that_never_plays_is_given_up_for_the_default() {
        let (_devices, speaker) = fake::install(vec![0.0; 8], 1, RATE, (RATE, 1));
        speaker.drains.store(false, Ordering::Relaxed);
        let (output, tx) = AudioOutput::spawn(Some("A speaker that is gone".to_owned()));
        let frame = vec![100_i16; FRAME_SAMPLES];
        let started = Instant::now();
        while !speaker.opened().iter().any(Option::is_none)
            && started.elapsed() < Duration::from_secs(20)
        {
            let _ = tx.try_send(frame.clone());
            std::thread::sleep(Duration::from_millis(20));
        }
        let opened = speaker.opened();
        let restarts = output.restarts.load(Ordering::Relaxed);
        drop(output);
        assert_eq!(
            opened.first().cloned(),
            Some(Some("A speaker that is gone".to_owned())),
            "the selected device was tried first"
        );
        assert_eq!(
            opened.last().cloned(),
            Some(None),
            "and it was given up for the default output: {opened:?}"
        );
        assert!(
            restarts >= UNPLAYED_RESTARTS_BEFORE_DEFAULT,
            "the run of dead streams is counted: {restarts}"
        );
    }

    /// A sink that was playing and then stopped is restarted on the short clock, which is what the
    /// stall window is for: a device that dies mid-call must not hold the peer's voice for seconds.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_speaker_that_played_and_then_stopped_is_restarted_quickly() {
        let (_devices, speaker) = fake::install(vec![0.0; 8], 1, RATE, (RATE, 1));
        let (output, tx) = AudioOutput::spawn(None);
        let frame = vec![100_i16; FRAME_SAMPLES];
        let started = Instant::now();
        while speaker.played_count() < 2 && started.elapsed() < Duration::from_secs(5) {
            let _ = tx.try_send(frame.clone());
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(speaker.played_count() >= 2, "the device was playing");
        // It stops taking anything, mid-call.
        speaker.drains.store(false, Ordering::Relaxed);
        let stopped = Instant::now();
        while output.opens.load(Ordering::Relaxed) < 2
            && stopped.elapsed() < Duration::from_millis(1_500)
        {
            let _ = tx.try_send(frame.clone());
            std::thread::sleep(Duration::from_millis(20));
        }
        let reopened = stopped.elapsed();
        let (opens, stalls) = (
            output.opens.load(Ordering::Relaxed),
            output.stalls.load(Ordering::Relaxed),
        );
        drop(output);
        assert!(
            opens >= 2,
            "a stream that stopped playing is reopened (after {reopened:?})"
        );
        assert_eq!(stalls, 1, "and it is counted as a stall, not a start-up");
        assert!(
            reopened < Duration::from_millis(1_500),
            "on the stall clock, not the start-up one"
        );
    }

    /// A speaker that never plays anything is still restarted, on the start-up clock, and the call
    /// carries on.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_stuck_speaker_is_restarted_and_the_call_survives() {
        let (_devices, speaker) = fake::install(vec![0.0; 8], 1, RATE, (48_000, 2));
        // A device that takes frames and never plays them: its position never moves.
        speaker.drains.store(false, Ordering::Relaxed);
        let (output, tx) = AudioOutput::spawn(None);
        let frame = vec![100_i16; FRAME_SAMPLES];
        let started = Instant::now();
        while speaker.opened_count() < 2 && started.elapsed() < Duration::from_secs(10) {
            let _ = tx.try_send(frame.clone());
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            speaker.opened_count() >= 2,
            "the stuck stream was reopened, not left holding the call"
        );
        // And the restarted stream plays what the engine sends from here on.
        speaker.drains.store(true, Ordering::Relaxed);
        let before = speaker.played_count();
        let started = Instant::now();
        while speaker.played_count() <= before && started.elapsed() < Duration::from_secs(5) {
            let _ = tx.try_send(frame.clone());
            std::thread::sleep(Duration::from_millis(20));
        }
        drop(output);
        assert!(speaker.played_count() > before, "the call kept playing");
    }

    /// Opens the machine's own devices and reads real frames from the microphone.
    /// `cargo test --lib -- --ignored --nocapture call_audio::tests::hardware`
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "opens this machine's real devices"]
    async fn hardware_opens_the_microphone_and_the_speaker() {
        let microphones = microphones();
        let speakers = speakers();
        eprintln!("microphones: {microphones:#?}");
        eprintln!("speakers: {speakers:#?}");
        eprintln!("unavailable: {:?}", unavailable(None, None));

        // The default device first, then every device by name, so a machine whose default input
        // is silent still says which microphone a call should be pointed at.
        let choices =
            std::iter::once(None).chain(microphones.iter().map(|(id, _)| Some(id.clone())));
        let mut tried: Vec<(Option<String>, usize)> = Vec::new();
        for target in choices {
            let (input, frames) = AudioInput::spawn(target.clone());
            let started = Instant::now();
            let mut heard = 0;
            while started.elapsed() < Duration::from_millis(600) {
                match frames.try_recv() {
                    Ok(frame) => {
                        assert_eq!(frame.len(), FRAME_SAMPLES, "the engine's frame size");
                        heard += 1;
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
            drop(input);
            eprintln!("{target:?}: {heard} frames in 600 ms");
            tried.push((target, heard));
        }
        assert!(
            tried.iter().any(|(_, heard)| *heard > 0),
            "no microphone delivered a frame: {tried:?}"
        );

        // The speaker side: send a second of silence through the engine's own channel and check
        // the writer took it on the default output rather than falling back to another device.
        let (output, frames) = AudioOutput::spawn(None);
        for _ in 0..16 {
            frames
                .send(vec![0_i16; FRAME_SAMPLES])
                .await
                .expect("the speaker pump is alive");
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(
            output.fell_back.try_recv().is_err(),
            "the default output accepted the frames"
        );
        drop(output);
        eprintln!("the speaker took 16 frames without falling back");
    }
}
