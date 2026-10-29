//! Playback through fastframe-audio: one shared output that every sound joins, and outputs of
//! their own for what plays on a device of its own (a call's speaker).
//!
//! The shared output lives on a thread of its own, which opens the device off the UI thread,
//! keeps it working (`maintain`) and pauses it while nothing plays: a sound costs callbacks only
//! while it sounds, and the device is let go after a long pause.

use std::num::NonZero;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak, mpsc};
use std::time::Duration;

use fastframe_audio::{Device, Maintained, Render};

/// Plays a rodio mixer, made at the stream's rate and channel count.
///
/// A stand-in for fastframe-audio's own `MixerRender` until its `rodio` feature follows rodio's
/// newer line: that one mixes with rodio 0.21, and ZapFast's players and sources are rodio
/// 0.22's, which cannot join a 0.21 mixer. Switch to it then; this is the same shape.
///
/// A stream in the format the mixer already has keeps it, so a reopen on another device carries
/// on from the same sample. Another rate or channel count makes a new mixer, left in the
/// [`MixSlot`]: what was added to the old one no longer plays. It is idle while the mixer has
/// nothing to play: nothing queued and no player connected.
pub(crate) struct Mix {
    source: Option<rodio::mixer::MixerSource>,
    format: (u32, u16),
    slot: MixSlot,
    idle: bool,
    monitor: Option<Box<dyn Monitor>>,
}

/// Watches what an output hands its device, on the audio thread, so it must never wait or
/// allocate: a call's echo canceller hears the far end's voice through one, as it is played.
pub(crate) trait Monitor: Send {
    /// The stream's rate and channel count, told before its first block and on every reopen.
    fn configure(&mut self, sample_rate: u32, channels: u16);
    /// One block of interleaved samples, as the device is handed them.
    fn rendered(&mut self, out: &[f32]);
}

/// Where [`Mix`] leaves each mixer it makes.
#[derive(Clone, Default)]
struct MixSlot(Arc<Mutex<Option<rodio::mixer::Mixer>>>);

impl MixSlot {
    fn take(&self) -> Option<rodio::mixer::Mixer> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
    }
}

impl Mix {
    fn new(monitor: Option<Box<dyn Monitor>>) -> (Self, MixSlot) {
        let slot = MixSlot::default();
        let mix = Self {
            source: None,
            format: (0, 0),
            slot: slot.clone(),
            idle: true,
            monitor,
        };
        (mix, slot)
    }
}

impl Render for Mix {
    fn configure(&mut self, sample_rate: u32, channels: u16) {
        if let Some(monitor) = &mut self.monitor {
            monitor.configure(sample_rate, channels);
        }
        if self.source.is_some() && self.format == (sample_rate, channels) {
            return;
        }
        let (Some(rate), Some(count)) = (NonZero::new(sample_rate), NonZero::new(channels)) else {
            return;
        };
        let (mixer, source) = rodio::mixer::mixer(count, rate);
        self.source = Some(source);
        self.format = (sample_rate, channels);
        self.idle = true;
        *self.slot.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(mixer);
    }

    fn render(&mut self, out: &mut [f32]) {
        if let Some(source) = &mut self.source {
            // The mixer ends (`None`) while it has nothing playing or waiting, and starts again
            // from the next sample once something is added.
            let mut ended = self.idle;
            for sample in out.iter_mut() {
                let next = source.next();
                ended = next.is_none();
                *sample = next.unwrap_or(0.0);
            }
            self.idle = ended;
        } else {
            out.fill(0.0);
        }
        if let Some(monitor) = &mut self.monitor {
            monitor.rendered(out);
        }
    }

    fn idle(&self) -> bool {
        self.idle
    }
}

/// An output playing a rodio mixer, with the mixer to add to.
pub(crate) struct MixOutput {
    output: fastframe_audio::Output<Mix>,
    slot: MixSlot,
    mixer: rodio::mixer::Mixer,
}

/// What [`MixOutput::maintain`] found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Kept {
    /// What was playing no longer plays: the mixer was made again for another format, the stream
    /// could not be opened again, or the device reported a fatal error. Add it again.
    pub lost: bool,
    /// The device was let go after a long pause; the next `resume` opens it again.
    pub released: bool,
}

impl MixOutput {
    /// Opens `device` in stereo at 48 kHz where the device takes it (the rate voice messages
    /// decode at), so the mixer seldom has to be made again. `call` names it as a call's sound in
    /// the system's volume mixer, which lowers other sound while it plays; `monitor` is handed
    /// every block the device plays. Opening asks the device, which can take a moment: never on
    /// the UI thread.
    pub(crate) fn open(
        device: Device,
        pause_when_idle: Option<Duration>,
        call: bool,
        monitor: Option<Box<dyn Monitor>>,
    ) -> Result<Self, String> {
        let follow_default = matches!(device, Device::Default);
        let options = fastframe_audio::OutputOptions {
            device,
            sample_rate: Some(crate::voice::RATE),
            follow_default,
            pause_when_idle,
            media_name: call.then(|| super::CALL_STREAM.to_owned()),
            role: call.then_some(fastframe_audio::StreamRole::Communication),
            ..fastframe_audio::OutputOptions::default()
        };
        let (mix, slot) = Mix::new(monitor);
        let output =
            fastframe_audio::Output::open(options, mix).map_err(|error| error.to_string())?;
        let mixer = slot
            .take()
            .ok_or_else(|| "the output opened without a format".to_owned())?;
        Ok(Self {
            output,
            slot,
            mixer,
        })
    }

    pub(crate) fn mixer(&self) -> &rodio::mixer::Mixer {
        &self.mixer
    }

    /// The stream's rate and channel count, which the mixer mixes at.
    pub(crate) fn format(&self) -> (u32, u16) {
        (self.output.sample_rate(), self.output.channels())
    }

    pub(crate) fn device_id(&self) -> &fastframe_audio::DeviceId {
        self.output.device_id()
    }

    /// How far the output has played, and its latency as the device reports it.
    pub(crate) fn clock(&self) -> fastframe_audio::Clock {
        self.output.clock()
    }

    pub(crate) fn resume(&mut self) {
        self.output.resume();
    }

    fn is_paused(&self) -> bool {
        self.output.is_paused()
    }

    /// Keeps the output on a working device: call it at least every second or so while it plays,
    /// and every few seconds while it is paused.
    pub(crate) fn maintain(&mut self, label: &str) -> Kept {
        let mut kept = Kept::default();
        match self.output.maintain() {
            Maintained::Unchanged => {}
            Maintained::Reopened {
                device,
                sample_rate,
                channels,
                reason,
            } => log::info!(
                "{label}: the output reopened on {device} at {sample_rate} Hz, {channels} channels ({reason:?})"
            ),
            Maintained::Released => kept.released = true,
            Maintained::Failed(error) => {
                log::warn!("{label}: the output could not be opened again: {error}");
                kept.lost = true;
            }
        }
        if let Some(mixer) = self.slot.take() {
            self.mixer = mixer;
            kept.lost = true;
        }
        for error in self.output.take_errors() {
            if error.is_fatal() {
                log::warn!("{label}: the output failed: {error}");
                kept.lost = true;
            } else {
                log::debug!("{label}: the output reported: {error}");
            }
        }
        kept
    }
}

/// How long the shared output may have nothing to play before it pauses: longer than any
/// device's latency, so the end of the last sound has played.
const IDLE_PAUSE: Duration = Duration::from_secs(2);
/// How often the shared output is kept while it plays.
const PLAYING_TICK: Duration = Duration::from_millis(500);
/// How often the shared output is kept while it is paused, so its device is let go in time.
const PAUSED_TICK: Duration = Duration::from_secs(5);
/// How long an open may wait for the shared output's thread, which may be opening the device.
const OPEN_WAIT: Duration = Duration::from_secs(5);

/// A place on the shared output: its mixer, to connect players to, while it plays.
pub struct Output {
    mixer: rodio::mixer::Mixer,
    generation: u64,
    current: Arc<AtomicU64>,
    _wake: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Output {
    pub fn mixer(&self) -> &rodio::mixer::Mixer {
        &self.mixer
    }

    /// Whether what this place plays has stopped playing: the device failed, or was reopened in
    /// another format. Drop it and open another.
    pub fn failed(&self) -> bool {
        self.current.load(Ordering::Acquire) != self.generation
    }
}

struct Request {
    reply: mpsc::Sender<Result<(rodio::mixer::Mixer, u64), String>>,
    wake: Option<Weak<dyn Fn() + Send + Sync>>,
}

struct Shared {
    requests: mpsc::Sender<Request>,
    generation: Arc<AtomicU64>,
}

fn shared() -> Result<&'static Shared, String> {
    static SHARED: OnceLock<Result<Shared, String>> = OnceLock::new();
    SHARED
        .get_or_init(|| {
            let (requests, received) = mpsc::channel();
            let generation = Arc::new(AtomicU64::new(0));
            let counter = Arc::clone(&generation);
            std::thread::Builder::new()
                .name("sound-output".to_owned())
                .spawn(move || run_shared(&received, &counter))
                .map_err(|error| format!("the sound thread could not start: {error}"))?;
            Ok(Shared {
                requests,
                generation,
            })
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// A place on the shared output, opening the default device if nothing plays yet. `wake` is
/// called when the place fails, so a player that waits for frames notices. Blocks while the
/// device opens.
pub fn open_output(wake: Option<Arc<dyn Fn() + Send + Sync>>) -> Result<Output, String> {
    let shared = shared()?;
    let (reply, answer) = mpsc::channel();
    shared
        .requests
        .send(Request {
            reply,
            wake: wake.as_ref().map(Arc::downgrade),
        })
        .map_err(|_| "the sound thread has stopped".to_owned())?;
    let (mixer, generation) = answer
        .recv_timeout(OPEN_WAIT)
        .map_err(|_| "the sound output did not answer".to_owned())??;
    Ok(Output {
        mixer,
        generation,
        current: Arc::clone(&shared.generation),
        _wake: wake,
    })
}

/// The shared output's thread: opens the device on the first request, resumes it for each, and
/// keeps it until the process ends. While its device is let go it waits for the next request.
fn run_shared(requests: &mpsc::Receiver<Request>, generation: &AtomicU64) {
    let mut output: Option<MixOutput> = None;
    let mut released = false;
    let mut wakers: Vec<Weak<dyn Fn() + Send + Sync>> = Vec::new();
    loop {
        let wait = match &output {
            None => None,
            Some(_) if released => None,
            Some(output) if output.is_paused() => Some(PAUSED_TICK),
            Some(_) => Some(PLAYING_TICK),
        };
        let request = match wait {
            None => match requests.recv() {
                Ok(request) => Some(request),
                Err(_) => return,
            },
            Some(wait) => match requests.recv_timeout(wait) {
                Ok(request) => Some(request),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            },
        };
        if let Some(Request { reply, wake }) = request {
            wakers.retain(|wake| wake.strong_count() > 0);
            wakers.extend(wake);
            if output.is_none() {
                match MixOutput::open(Device::Default, Some(IDLE_PAUSE), false, None) {
                    Ok(opened) => {
                        log::info!(
                            "sound output: playing through {} ({})",
                            fastframe_audio::host(),
                            opened.device_id()
                        );
                        output = Some(opened);
                        generation.fetch_add(1, Ordering::AcqRel);
                    }
                    Err(error) => {
                        let _ = reply.send(Err(format!("No sound output: {error}")));
                        continue;
                    }
                }
            }
            let opened = output.as_mut().expect("opened above");
            opened.resume();
            released = false;
            let _ = reply.send(Ok((
                opened.mixer().clone(),
                generation.load(Ordering::Acquire),
            )));
        }
        let Some(opened) = output.as_mut() else {
            continue;
        };
        let kept = opened.maintain("sound output");
        released |= kept.released;
        if kept.lost {
            generation.fetch_add(1, Ordering::AcqRel);
            wakers.retain(|wake| match wake.upgrade() {
                Some(wake) => {
                    wake();
                    true
                }
                None => false,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::buffer::SamplesBuffer;

    fn tone(samples: usize) -> SamplesBuffer {
        SamplesBuffer::new(
            NonZero::new(2).expect("two"),
            NonZero::new(48_000).expect("48 kHz"),
            vec![0.5_f32; samples],
        )
    }

    #[test]
    fn the_mix_plays_what_is_added_and_is_idle_once_it_ends() {
        let (mut mix, slot) = Mix::new(None);
        let mut out = [1.0_f32; 64];
        mix.render(&mut out);
        assert!(
            out.iter().all(|sample| *sample == 0.0),
            "silence before a format"
        );
        mix.configure(48_000, 2);
        let mixer = slot.take().expect("configuring makes a mixer");
        mix.render(&mut out);
        assert!(mix.idle());
        mixer.add(tone(32));
        mix.render(&mut out);
        assert!(out.contains(&0.5), "the added sound plays");
        mix.render(&mut out);
        assert!(mix.idle(), "nothing is left to play");
        assert!(out.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn a_connected_player_keeps_the_mix_busy_until_it_is_dropped() {
        let (mut mix, slot) = Mix::new(None);
        mix.configure(48_000, 2);
        let mixer = slot.take().expect("configuring makes a mixer");
        let player = rodio::Player::connect_new(&mixer);
        let mut out = [0.0_f32; 64];
        mix.render(&mut out);
        mix.render(&mut out);
        assert!(!mix.idle(), "a player may queue more at any time");
        drop(player);
        mix.render(&mut out);
        mix.render(&mut out);
        assert!(mix.idle());
    }

    #[test]
    fn pausing_a_clip_releases_the_mix_and_keeps_its_position() {
        use crate::audio::{Loaded, Player, State};

        let (mut mix, slot) = Mix::new(None);
        mix.configure(48_000, 2);
        let mixer = slot.take().expect("a mixer");
        let sink = rodio::Player::connect_new(&mixer);
        sink.append(tone(960_000));
        let samples = Arc::new(vec![0.5; 480_000]);
        let mut player = Player::new(crate::backend::Waker::default());
        player.loaded = Some(Loaded {
            message: "clip".to_owned(),
            buffer: Arc::clone(&samples),
            samples,
            factor: 2.0,
            base: Duration::from_secs(1),
            paused: false,
            done: false,
        });
        player.output = Some((
            Output {
                mixer,
                generation: 0,
                current: Arc::new(AtomicU64::new(0)),
                _wake: None,
            },
            sink,
        ));
        let mut out = [0.0_f32; 9_600];
        mix.render(&mut out);
        let position = player.status("clip").position;
        assert!(position > Duration::from_secs(1), "playback advanced");
        player
            .toggle("clip", std::path::Path::new("unused"))
            .unwrap();
        assert!(player.output.is_none(), "a paused clip releases its player");
        assert_eq!(player.status("clip").state, State::Paused);
        assert_eq!(player.status("clip").position, position);
        mix.render(&mut out);
        mix.render(&mut out);
        assert!(mix.idle(), "the device can pause while the clip is paused");
        player.set_speed(1.0);
        player.poll().unwrap();
        assert!(player.output.is_none(), "changing speed keeps it released");
        assert!(!player.preparing_speed("clip"));
        assert_eq!(player.status("clip").position, position);
        assert!(player.take_finished().is_none(), "pausing is not finishing");
    }

    /// What a monitor was told: the format, and every sample.
    type Told = (Option<(u32, u16)>, Vec<f32>);

    /// Records what a monitor is told, as a call's echo tap would hear it.
    struct Heard(Arc<Mutex<Told>>);

    impl Monitor for Heard {
        fn configure(&mut self, sample_rate: u32, channels: u16) {
            self.0.lock().expect("unpoisoned").0 = Some((sample_rate, channels));
        }

        fn rendered(&mut self, out: &[f32]) {
            self.0.lock().expect("unpoisoned").1.extend_from_slice(out);
        }
    }

    #[test]
    fn a_monitor_hears_every_block_played_silence_included() {
        let heard = Arc::new(Mutex::new((None, Vec::new())));
        let (mut mix, slot) = Mix::new(Some(Box::new(Heard(Arc::clone(&heard)))));
        mix.configure(48_000, 2);
        let mixer = slot.take().expect("configuring makes a mixer");
        let mut out = [0.0_f32; 64];
        mix.render(&mut out);
        mixer.add(tone(32));
        mix.render(&mut out);
        let heard = heard.lock().expect("unpoisoned");
        assert_eq!(heard.0, Some((48_000, 2)));
        assert_eq!(heard.1.len(), 128, "the idle block and the playing one");
        assert!(heard.1[..64].iter().all(|sample| *sample == 0.0));
        assert_eq!(
            &heard.1[64..],
            &out[..],
            "exactly what the device was handed"
        );
    }

    #[test]
    fn only_another_format_makes_a_new_mixer() {
        let (mut mix, slot) = Mix::new(None);
        mix.configure(48_000, 2);
        assert!(slot.take().is_some());
        mix.configure(48_000, 2);
        assert!(
            slot.take().is_none(),
            "a reopen in the same format keeps the mixer"
        );
        mix.configure(44_100, 2);
        assert!(slot.take().is_some());
    }
}
