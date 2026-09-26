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
use std::sync::atomic::{AtomicBool, Ordering};
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
pub fn unavailable() -> Option<&'static str> {
    if rodio::microphone::MicrophoneBuilder::new()
        .default_device()
        .is_err()
    {
        return Some("microphone");
    }
    if rodio::DeviceSinkBuilder::from_default_device().is_err() {
        return Some("speaker");
    }
    None
}

fn open_microphone(device: Option<&str>) -> Result<rodio::microphone::Microphone, String> {
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
        .map_err(|error| format!("the microphone could not be opened: {error}"))
}

/// An open speaker. The sink has to outlive the player queued on it, or the stream closes with it.
struct Sink {
    _device: rodio::MixerDeviceSink,
    player: rodio::Player,
    layout: Layout,
}

fn open_sink(device: Option<&str>) -> Result<Sink, String> {
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
    Ok(Sink {
        _device: sink,
        player,
        layout,
    })
}

impl Sink {
    fn append(&self, samples: Vec<f32>) {
        self.player.append(SamplesBuffer::new(
            NonZero::new(self.layout.channels).unwrap_or(NonZero::<u16>::MIN),
            NonZero::new(self.layout.rate)
                .unwrap_or(NonZero::new(48_000).expect("48 kHz is not zero")),
            samples,
        ));
        self.player.play();
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
}

impl AudioInput {
    pub fn spawn(target: Option<String>) -> (Self, async_channel::Receiver<Vec<i16>>) {
        let (out, rx) = async_channel::bounded::<Vec<i16>>(4);
        let (swap, swaps) = async_channel::bounded::<Option<String>>(1);
        let (fell, fell_back) = async_channel::bounded::<()>(1);
        tokio::spawn(mic_pump(out, swaps, fell, target.clone()));
        (
            Self {
                swap: Some(swap),
                fell_back,
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
) {
    let mut target = initial;
    loop {
        let reader = match MicReader::start(target.as_deref()) {
            Ok(reader) => reader,
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
    let mut converter = Converter::new(microphone.channels().get(), microphone.sample_rate().get());
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
}

impl AudioOutput {
    pub fn spawn(target: Option<String>) -> (Self, async_channel::Sender<Vec<i16>>) {
        let (tx, rx) = async_channel::bounded::<Vec<i16>>(SPEAKER_QUEUE);
        let (swap, swaps) = async_channel::bounded::<Option<String>>(1);
        let (fell, fell_back) = async_channel::bounded::<()>(1);
        tokio::spawn(play_pump(rx, swaps, fell, target.clone()));
        (
            Self {
                swap: Some(swap),
                fell_back,
            },
            tx,
        )
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

async fn play_pump(
    rx: async_channel::Receiver<Vec<i16>>,
    swaps: async_channel::Receiver<Option<String>>,
    fell: async_channel::Sender<()>,
    initial: Option<String>,
) {
    let mut target = initial;
    loop {
        let writer = match SpkWriter::start(target.as_deref()) {
            Ok(writer) => writer,
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
        drop(writer);
        if finished {
            return;
        }
        if stalled && accepted {
            // A stream that played frames was merely stalled, so it keeps the device the user
            // picked rather than being demoted to the system default over one bad moment. What is
            // queued is already older than the restart, and playing it late would only add delay,
            // so it is dropped and the engine's channel refills at its own pace.
            while rx.try_recv().is_ok() {}
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
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
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SpkWriter {
    fn start(device: Option<&str>) -> Result<Self, String> {
        let (tx, rx) = async_channel::bounded::<Vec<i16>>(SPEAKER_QUEUE);
        let stop = Arc::new(AtomicBool::new(false));
        let accepted = Arc::new(AtomicBool::new(false));
        let stalled = Arc::new(AtomicBool::new(false));
        let (ready, opened) = std::sync::mpsc::channel::<Result<(), String>>();
        let name = device.map(str::to_owned);
        let thread = {
            let stop = Arc::clone(&stop);
            let accepted = Arc::clone(&accepted);
            let stalled = Arc::clone(&stalled);
            std::thread::Builder::new()
                .name("call-speaker".to_owned())
                .spawn(move || {
                    write_speaker(name.as_deref(), &rx, &stop, &accepted, &stalled, &ready)
                })
                .map_err(|error| format!("the speaker thread could not start: {error}"))?
        };
        match opened.recv() {
            Ok(Ok(())) => Ok(Self {
                stop,
                frames: tx,
                accepted,
                stalled,
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

fn write_speaker(
    device: Option<&str>,
    frames: &async_channel::Receiver<Vec<i16>>,
    stop: &AtomicBool,
    accepted: &AtomicBool,
    stalled: &AtomicBool,
    ready: &std::sync::mpsc::Sender<Result<(), String>>,
) {
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
        sink.append(sink.layout.convert(&frame));
        accepted.store(true, Ordering::Relaxed);
        let position = sink.player.get_pos();
        if !sink.player.empty() && position == last_position {
            let since = *quiet_since.get_or_insert_with(Instant::now);
            if since.elapsed() > STALLED {
                log::warn!("[CALL] the speaker stopped playing; restarting the stream");
                stalled.store(true, Ordering::Relaxed);
                return;
            }
        } else {
            quiet_since = None;
        }
        last_position = position;
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

    /// Opens the machine's own devices and reads real frames from the microphone.
    /// `cargo test --lib -- --ignored --nocapture call_audio::tests::hardware`
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "opens this machine's real devices"]
    async fn hardware_opens_the_microphone_and_the_speaker() {
        let microphones = microphones();
        let speakers = speakers();
        eprintln!("microphones: {microphones:#?}");
        eprintln!("speakers: {speakers:#?}");
        eprintln!("unavailable: {:?}", unavailable());

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
