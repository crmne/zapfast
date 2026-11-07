//! The tones a call makes on this side: the ringback heard while the peer's phone rings, and the
//! ringtone of an incoming call.
//!
//! Both are synthesised here rather than bundled. Pidgin, whose message sounds ZapFast plays,
//! ships no call or ring sound (its `share/sounds/` holds only alert, login, logout, receive and
//! send), and a tone made in code needs no licence note of its own.
//!
//! - The ringback is the ETSI one (ETSI ES 201 970, ITU-T E.180): 425 Hz, one second on and four
//!   off. It is what most of Europe hears from a ringing line, and WhatsApp's own clients play a
//!   ringback of that shape while the other phone rings.
//! - The ringtone is two soft bell notes, E5 then B4, played twice and followed by a rest, every
//!   [`RINGTONE_CYCLE`]: the cadence the ring of an unanswered call already had.
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
/// How long one ring of the ringtone takes, rest included.
pub const RINGTONE_CYCLE: Duration = Duration::from_secs(3);
/// The ringtone's notes: E5 and B4, a fourth apart, and when each starts within the cycle.
const RINGTONE_NOTES: [(f32, f32); 4] =
    [(0.0, 659.25), (0.4, 493.88), (0.9, 659.25), (1.3, 493.88)];
/// How long each note may ring before it is faded out, in seconds.
const NOTE_LENGTH: f32 = 0.6;
const RINGTONE_LEVEL: f32 = 0.28;
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
    let rate = RATE as f32;
    let total = (RINGTONE_CYCLE.as_secs_f32() * rate) as usize;
    let mut samples = vec![0.0f32; total];
    for (start, frequency) in RINGTONE_NOTES {
        let first = (start * rate) as usize;
        let length = (NOTE_LENGTH * rate) as usize;
        for (offset, sample) in samples[first..(first + length).min(total)]
            .iter_mut()
            .enumerate()
        {
            let t = offset as f32 / rate;
            // A struck bell: a quick rise, a decay, and a fade at the end of its slot so the
            // next note starts from silence. Two quiet overtones give it some body.
            let attack = (t / 0.008).min(1.0);
            let decay = (-t / 0.18).exp();
            let envelope = attack * decay * edge(t, NOTE_LENGTH);
            let phase = TAU * frequency * t;
            let bell = phase.sin() + 0.25 * (2.0 * phase).sin() + 0.08 * (3.0 * phase).sin();
            *sample += RINGTONE_LEVEL * envelope * bell / 1.33;
        }
    }
    samples
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
            assert!(samples.iter().all(|s| s.abs() < 0.5), "{tone:?} stays soft");
            // No step between neighbouring samples is large enough to click.
            let jump = samples
                .windows(2)
                .map(|pair| (pair[1] - pair[0]).abs())
                .fold(0.0f32, f32::max);
            assert!(jump < 0.05, "{tone:?} jumps by {jump}");
        }
        assert_eq!(
            cycle(Tone::Ringtone).len() as f32,
            RINGTONE_CYCLE.as_secs_f32() * RATE as f32
        );
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
