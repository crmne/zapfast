//! Recording through fastframe-audio: an input whose captured samples a reading thread drains.
//!
//! The input hands each block to a ring buffer on the audio thread, which never waits or
//! allocates; the thread that opened the microphone reads it without blocking and keeps the input
//! working (`maintain`) as it reads. Open and read on that one thread, never the UI thread:
//! opening the default input on PipeWire can wait for its server.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use fastframe_audio::{Capture, Device, Input, InputOptions, Maintained};

/// Seconds of sound the ring holds for a reader that fell behind.
const RING_SECONDS: usize = 2;
/// The ring's size before the input says its format: two seconds of 48 kHz stereo.
const RING_SAMPLES: usize = RING_SECONDS * 48_000 * 2;
/// How often reading keeps the input working.
const MAINTAIN_EVERY: Duration = Duration::from_millis(250);

/// Takes what the input captures, on the audio thread.
struct Tap {
    ring: rtrb::Producer<f32>,
    /// The stream's rate and channel count, as the input last configured them.
    rate: Arc<AtomicU32>,
    channels: Arc<AtomicU32>,
}

impl Capture for Tap {
    fn configure(&mut self, sample_rate: u32, channels: u16) {
        self.rate.store(sample_rate, Ordering::Release);
        self.channels.store(u32::from(channels), Ordering::Release);
    }

    fn capture(&mut self, input: &[f32]) {
        // A reader that fell two seconds behind loses what does not fit, not the audio thread.
        let room = self.ring.slots().min(input.len());
        if let Ok(chunk) = self.ring.write_chunk_uninit(room) {
            chunk.fill_from_iter(input.iter().copied());
        }
    }
}

/// An open microphone. Its samples come interleaved in the channel count and at the rate it
/// opened with; an iterator that answers `None` while nothing new has arrived.
pub struct Microphone {
    input: Input<Tap>,
    ring: rtrb::Consumer<f32>,
    rate: Arc<AtomicU32>,
    channels: Arc<AtomicU32>,
    /// The format the reader was told, which a reopen in another format no longer matches.
    opened: (u32, u16),
    label: &'static str,
    lost: bool,
    maintained: Instant,
}

impl Microphone {
    /// Opens `device` at its own rate and channel count; `call` names it as a call's microphone in
    /// the system's volume mixer. Blocks while the device opens, and may make the system ask the
    /// person for the microphone.
    pub fn open(device: Device, label: &'static str, call: bool) -> Result<Self, String> {
        let follow_default = matches!(device, Device::Default);
        let (producer, ring) = rtrb::RingBuffer::new(RING_SAMPLES);
        let rate = Arc::new(AtomicU32::new(0));
        let channels = Arc::new(AtomicU32::new(0));
        let tap = Tap {
            ring: producer,
            rate: Arc::clone(&rate),
            channels: Arc::clone(&channels),
        };
        let options = InputOptions {
            device,
            follow_default,
            media_name: call.then(|| super::CALL_STREAM.to_owned()),
            role: call.then_some(fastframe_audio::StreamRole::Communication),
            ..InputOptions::default()
        };
        let input = Input::open(options, tap).map_err(|error| match error {
            fastframe_audio::OpenError::NoDevice => "No microphone available".to_owned(),
            error => format!("Could not open the microphone: {error}"),
        })?;
        let opened = (input.sample_rate(), input.channels());
        log::info!(
            "{label}: recording from {} at {} Hz, {} channels",
            input.device_id(),
            opened.0,
            opened.1
        );
        Ok(Self {
            input,
            ring,
            rate,
            channels,
            opened,
            label,
            lost: false,
            maintained: Instant::now(),
        })
    }

    pub fn channels(&self) -> u16 {
        self.opened.1
    }

    pub fn rate(&self) -> u32 {
        self.opened.0
    }

    /// How much the input has captured, and its latency as the device reports it.
    pub fn clock(&self) -> fastframe_audio::Clock {
        self.input.clock()
    }

    /// The id of the device recording, which is the default's when a picked one was missing.
    pub fn device_id(&self) -> &fastframe_audio::DeviceId {
        self.input.device_id()
    }

    /// Whether the microphone stopped recording what its reader expects: it could not be opened
    /// again, reopened in another format, or reported a fatal error. Drop it and open another.
    pub fn failed(&mut self) -> bool {
        if !self.lost && self.maintained.elapsed() >= MAINTAIN_EVERY {
            self.maintained = Instant::now();
            self.keep();
        }
        self.lost
    }

    fn keep(&mut self) {
        let label = self.label;
        match self.input.maintain() {
            Maintained::Unchanged | Maintained::Released => {}
            Maintained::Reopened { device, reason, .. } => {
                log::info!("{label}: the microphone reopened on {device} ({reason:?})");
            }
            Maintained::Failed(error) => {
                log::warn!("{label}: the microphone could not be opened again: {error}");
                self.lost = true;
            }
        }
        let now = (
            self.rate.load(Ordering::Acquire),
            u16::try_from(self.channels.load(Ordering::Acquire)).unwrap_or(0),
        );
        if now != self.opened {
            log::info!("{label}: the microphone reopened in another format");
            self.lost = true;
        }
        for error in self.input.take_errors() {
            if error.is_fatal() {
                log::warn!("{label}: the microphone failed: {error}");
                self.lost = true;
            } else {
                log::debug!("{label}: the microphone reported: {error}");
            }
        }
    }
}

impl Iterator for Microphone {
    type Item = f32;

    /// The next sample captured, or `None` while nothing new has arrived (not the end: wait and
    /// ask again, and look at [`Microphone::failed`]).
    fn next(&mut self) -> Option<f32> {
        match self.ring.pop() {
            Ok(sample) => Some(sample),
            Err(_) => {
                self.failed();
                None
            }
        }
    }
}

/// The microphones this machine offers, as `(id, name)`; nothing is opened.
pub fn microphones() -> Result<Vec<(String, String)>, String> {
    fastframe_audio::input_devices()
        .map(|devices| {
            devices
                .into_iter()
                .map(|device| (device.id.as_str().to_owned(), device.name))
                .collect()
        })
        .map_err(|error| error.to_string())
}
