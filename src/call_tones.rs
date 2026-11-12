//! The tones a call makes on this side: the ringback heard while the peer's phone rings, and the
//! ringtone of an incoming call.
//!
//! The ringback is synthesised here; the ringtone is Apple's Reflection, bundled as
//! `assets/sounds/ringtone.wav` (see `assets/sounds/README.md`).
//!
//! - The ringback is the ETSI one (ETSI ES 201 970, ITU-T E.180): 425 Hz, one second on and four
//!   off. It is what most of Europe hears from a ringing line, and WhatsApp's own clients play a
//!   ringback of that shape while the other phone rings.
//! - The ringtone is the bundled clip, 48 kHz mono 16-bit PCM, repeated back to back as a phone
//!   repeats it.
//!
//! One cycle of each is rendered once into a buffer and repeated by rodio on the default output,
//! on a thread of its own, so opening the device never waits on the interface thread. Each tone
//! fades in and out within its cycle, and stopping fades the volume down first, so neither end
//! clicks. Tests and demos never open a device: [`Tones`] only keeps track of what would play.

use std::f32::consts::TAU;
use std::num::NonZero;
use std::time::Duration;

use rodio::Source as _;
use rodio::buffer::SamplesBuffer;

/// The rate the tones are rendered at.
const RATE: u32 = 48_000;
/// The ETSI ringback: frequency, then how long it sounds and how long it rests.
const RINGBACK_HZ: f32 = 425.0;
const RINGBACK_ON: f32 = 1.0;
const RINGBACK_OFF: f32 = 4.0;
/// How loud the ringback is: a line tone, quieter than the ringtone.
const RINGBACK_LEVEL: f32 = 0.18;
/// The ringtone: Apple's Reflection, 48 kHz mono 16-bit PCM WAV.
const RINGTONE_WAV: &[u8] = include_bytes!("../assets/sounds/ringtone.wav");
/// How loud the ringtone plays, against the clip as recorded.
const RINGTONE_LEVEL: f32 = 1.2;
/// The fade at either end of a tone, which keeps it from clicking.
const EDGE: f32 = 0.02;
/// How long a stopped tone takes to fade out.
const FADE_OUT: Duration = Duration::from_millis(60);
/// How often a playing tone looks at its output's health.
const WATCH: Duration = Duration::from_millis(100);
/// How long a tone whose output failed waits before opening it again,
/// doubling up to the longest with each failure.
const RETRY: Duration = Duration::from_secs(1);
const RETRY_LONGEST: Duration = Duration::from_secs(30);

/// A tone a call plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// Our call is ringing at the other end.
    Ringback,
    /// Someone is calling us.
    Ringtone,
}

/// The call tone playing now, if any. At most one plays at a time.
pub struct Tones {
    /// Whether a tone really reaches the speakers. Off in tests and demos.
    audible: bool,
    current: Option<Tone>,
    playing: Option<Playing>,
}

impl Default for Tones {
    /// Silent: what tests and demos get.
    fn default() -> Self {
        Self::new(false)
    }
}

impl Tones {
    /// `audible` is ignored in tests, which never open a sound device.
    pub fn new(audible: bool) -> Self {
        Self {
            audible: audible && !cfg!(test),
            current: None,
            playing: None,
        }
    }

    /// The tone that is playing, or would be in a silent run.
    pub fn current(&self) -> Option<Tone> {
        self.current
    }

    /// Plays `tone`, or stops when `None`. Asking for the tone that is already playing leaves it
    /// alone, so this can be called every frame.
    pub fn set(&mut self, tone: Option<Tone>) {
        if self.current == tone {
            return;
        }
        // Dropping the old handle fades it out on its own thread.
        self.playing = None;
        self.current = tone;
        if let Some(tone) = tone
            && self.audible
        {
            self.playing = Playing::start(tone);
        }
    }
}

/// A tone playing on its own thread; dropping it stops the tone.
struct Playing {
    _stop: std::sync::mpsc::Sender<()>,
}

impl Playing {
    fn start(tone: Tone) -> Option<Self> {
        let (stop, stopped) = std::sync::mpsc::channel::<()>();
        let spawned = std::thread::Builder::new()
            .name("call-tone".into())
            .spawn(move || {
                let mut retry = RETRY;
                loop {
                    let device = match crate::audio::open_output(None) {
                        Ok(device) => device,
                        Err(error) => {
                            log::debug!("call tone not played: {error}");
                            return;
                        }
                    };
                    let player = rodio::Player::connect_new(device.mixer());
                    player.append(
                        SamplesBuffer::new(
                            NonZero::<u16>::MIN,
                            NonZero::new(RATE).expect("48 kHz is not zero"),
                            cycle(tone),
                        )
                        .repeat_infinite(),
                    );
                    // Runs until the handle is dropped, which closes the channel, or until the
                    // output keeps failing (#507): then the device is let go, and opened again
                    // after a wait that doubles each time.
                    if !wait_for_stop(&stopped, || device.failed(), WATCH) {
                        drop(player);
                        drop(device);
                        if !matches!(
                            stopped.recv_timeout(retry),
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                        ) {
                            return;
                        }
                        retry = (retry * 2).min(RETRY_LONGEST);
                        continue;
                    }
                    const STEPS: u32 = 6;
                    for step in (0..STEPS).rev() {
                        player.set_volume(step as f32 / STEPS as f32);
                        std::thread::sleep(FADE_OUT / STEPS);
                    }
                    player.stop();
                    return;
                }
            });
        match spawned {
            Ok(_) => Some(Self { _stop: stop }),
            Err(error) => {
                log::debug!("no thread for a call tone: {error}");
                None
            }
        }
    }
}

/// Waits until the tone's handle is dropped, which returns true, or until
/// `failed`, looked at every `every`, which returns false.
fn wait_for_stop(
    stopped: &std::sync::mpsc::Receiver<()>,
    failed: impl Fn() -> bool,
    every: Duration,
) -> bool {
    use std::sync::mpsc::RecvTimeoutError;
    loop {
        match stopped.recv_timeout(every) {
            Err(RecvTimeoutError::Timeout) if failed() => return false,
            Err(RecvTimeoutError::Timeout) => {}
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return true,
        }
    }
}

/// One cycle of `tone`, mono at 48 kHz, ready to repeat.
pub fn cycle(tone: Tone) -> Vec<f32> {
    match tone {
        Tone::Ringback => ringback(),
        Tone::Ringtone => ringtone(),
    }
}

fn ringback() -> Vec<f32> {
    let rate = RATE as f32;
    let on = (RINGBACK_ON * rate) as usize;
    let total = ((RINGBACK_ON + RINGBACK_OFF) * rate) as usize;
    (0..total)
        .map(|index| {
            if index >= on {
                return 0.0;
            }
            let t = index as f32 / rate;
            let envelope = edge(t, RINGBACK_ON);
            RINGBACK_LEVEL * envelope * (TAU * RINGBACK_HZ * t).sin()
        })
        .collect()
}

fn ringtone() -> Vec<f32> {
    let mut samples = wav_samples(RINGTONE_WAV);
    // Fade both ends so the loop never clicks where it joins.
    let length = samples.len() as f32 / RATE as f32;
    for (index, sample) in samples.iter_mut().enumerate() {
        *sample *= RINGTONE_LEVEL * edge(index as f32 / RATE as f32, length);
    }
    samples
}

/// The samples of a mono 16-bit PCM WAV at [`RATE`], as floats; empty if it is not one.
fn wav_samples(wav: &[u8]) -> Vec<f32> {
    let mut rest = wav.get(12..).unwrap_or_default();
    let mut format_ok = false;
    while rest.len() >= 8 {
        let id = &rest[..4];
        let size = u32::from_le_bytes([rest[4], rest[5], rest[6], rest[7]]) as usize;
        let body = rest.get(8..8 + size).unwrap_or(&rest[8..]);
        match id {
            b"fmt " if body.len() >= 16 => {
                let channels = u16::from_le_bytes([body[2], body[3]]);
                let rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
                let bits = u16::from_le_bytes([body[14], body[15]]);
                format_ok = channels == 1 && rate == RATE && bits == 16;
            }
            b"data" if format_ok => {
                return body
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| f32::from(i16::from_le_bytes(*pair)) / 32_768.0)
                    .collect();
            }
            _ => {}
        }
        // Chunks are padded to an even length.
        rest = rest.get(8 + size + (size & 1)..).unwrap_or_default();
    }
    Vec::new()
}

/// A raised-cosine fade over [`EDGE`] at both ends of a span of `length` seconds.
fn edge(t: f32, length: f32) -> f32 {
    let ramp = |x: f32| 0.5 - 0.5 * (std::f32::consts::PI * (x / EDGE).clamp(0.0, 1.0)).cos();
    ramp(t) * ramp(length - t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ringback_is_one_second_of_425_hz_in_five() {
        let samples = cycle(Tone::Ringback);
        assert_eq!(samples.len(), 5 * RATE as usize);
        let sounding = samples.iter().rposition(|s| s.abs() > 1e-4).unwrap();
        assert!(sounding < RATE as usize, "silent after the first second");
        // Zero crossings in the steady middle of the tone give its frequency.
        let middle = &samples[RATE as usize / 4..RATE as usize * 3 / 4];
        let crossings = middle
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        assert!(
            (211..=214).contains(&crossings),
            "{crossings} in half a second"
        );
    }

    #[test]
    fn the_tones_start_and_end_in_silence_and_never_clip() {
        for tone in [Tone::Ringback, Tone::Ringtone] {
            let samples = cycle(tone);
            assert!(samples[0].abs() < 1e-3, "{tone:?} starts silent");
            assert!(
                samples[samples.len() - 1].abs() < 1e-3,
                "{tone:?} ends silent"
            );
            assert!(
                samples.iter().all(|s| s.abs() < 0.9),
                "{tone:?} never clips"
            );
        }
        // No step between neighbouring samples of the ringback is large enough to click.
        let jump = cycle(Tone::Ringback)
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(jump < 0.05, "the ringback jumps by {jump}");
    }

    #[test]
    fn the_bundled_ringtone_decodes_to_a_few_seconds() {
        let seconds = cycle(Tone::Ringtone).len() as f32 / RATE as f32;
        assert!((2.0..30.0).contains(&seconds), "{seconds} s");
    }

    #[test]
    fn a_silent_run_tracks_the_tone_without_playing_it() {
        let mut tones = Tones::new(true);
        assert!(!tones.audible, "tests never open a device");
        tones.set(Some(Tone::Ringback));
        assert_eq!(tones.current(), Some(Tone::Ringback));
        assert!(tones.playing.is_none());
        tones.set(None);
        assert_eq!(tones.current(), None);
    }
}
