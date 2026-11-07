//! 1:1 WhatsApp calling: signaling, media, audio devices, camera, and state.
//!
//! The protocol, the encryption and the media engine are whatsapp-rust's (`Client::voip()`); this
//! module keeps the state the interface draws. Audio goes through rodio, the layer the rest of the
//! app plays and records with, so a call opens the platform's own audio API (PipeWire or ALSA,
//! CoreAudio, WASAPI). The engine's 16 kHz mono frames meet whatever rate and channel count a device
//! wants inside [`crate::call_audio`], and each direction sits behind a *stable* channel, so
//! changing the input or output device replaces only the stream behind it without the engine ever
//! seeing a port close.
//!
//! Video is H.264 Annex-B both ways, because that is what the library transports: it never touches
//! pixels. Capture encodes with the `openh264` encoder the app already links, and the peer's access
//! units decode back to `egui` images with the `openh264` decoder `crate::video` already uses.
//! Frames cross to the UI through [`VideoTick`]; no decode work happens on the UI thread.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use egui::ColorImage;

use crate::model::CallDirection;
use crate::video_rate::{Cadence, Change as RateChange, FrameClock, RateControl, RateShare};
use whatsapp_rust::prelude::{Client, Jid};
use whatsapp_rust::types::call::{CallAction, IncomingCall};
use whatsapp_rust::types::group_call::{GroupCallParticipant, GroupCallUpdate};
use whatsapp_rust::voip::{
    CallEvent, CallHandle, TimedVideoFrame, VideoFrame, VideoSink, VideoSource,
};

/// The pixel budget one camera session encodes within: a landscape frame at most 1280 by 720, a
/// portrait one at most 720 by 1280. It is the largest step of [`crate::video_rate::TIERS`]; a path
/// that carries less is sent one of the smaller ones.
///
/// Following the camera's own aspect is what keeps a portrait camera portrait on the peer's screen
/// and in the corner preview. Stretching it into one fixed landscape frame would either pad it into
/// a letterbox or squash it, and neither is what the camera saw. The budget is 720p because that is
/// what the peer's screen is: a smaller frame is scaled up twice (once by this side's bitrate and
/// again by the phone) and reads as the soft, blocky picture a 640 by 360 call is.
const CAPTURE_LONG_SIDE: usize = TOP_TIER.long_side;
const CAPTURE_SHORT_SIDE: usize = TOP_TIER.short_side;
const TOP_TIER: crate::video_rate::Tier =
    crate::video_rate::TIERS[crate::video_rate::TIERS.len() - 1];
/// The largest the corner preview may be, in the same orientation as the capture. The preview is a
/// thumbnail rather than the call, so it stays the size of one and costs nothing to draw.
const PREVIEW_LONG_SIDE: usize = 320;
const PREVIEW_SHORT_SIDE: usize = 180;
/// The cadence the camera is asked for: what WhatsApp's own clients send at most. A camera that
/// grants fewer frames is used at what it gives; one that grants more is thinned to an even
/// fraction of its own rate (see [`crate::video_rate::decimation`]), and the frames carry their own
/// capture times, so nothing plays back at the wrong speed.
const VIDEO_FPS: u32 = TOP_TIER.fps;
/// RTP video clock (90 kHz) divided by the capture cadence.
const VIDEO_TS_STRIDE: u32 = 90_000 / VIDEO_FPS;
/// How often the decoder, while it waits for the peer's first keyframe, asks for one again.
const FIRST_KEYFRAME_RETRY: std::time::Duration = std::time::Duration::from_secs(1);
/// After a frame failed to reach the engine, how long before another keyframe is tried.
const RESYNC_GAP: std::time::Duration = std::time::Duration::from_millis(500);

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Where a 1:1 call is in its life.
///
/// Every value except [`CallPhase::Placing`] and the terminal ones comes from the backend: the
/// offer that was sent or received, the peer's `<preaccept>` and `<accept>`, and the media plane
/// reporting itself live. Nothing past placing is inferred from the button that started the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallPhase {
    /// The call button was pressed and our offer has not gone out yet: the devices are being
    /// checked and the peer's devices looked up. Local only: the window draws it the moment the
    /// button is pressed, before the backend has answered, so a call starts on screen at once. It
    /// claims nothing about the peer; [`CallPhase::Dialing`] follows once the offer is sent.
    Placing,
    /// Our offer went out and nobody has answered.
    Dialing,
    /// The peer's device is ringing: its `<preaccept>` arrived. Nobody has answered yet.
    Ringing,
    /// Someone else's offer is ringing and waits for the user.
    Incoming,
    /// The peer answered and the media plane is coming up.
    Connecting,
    /// We answered a ringing call and the media plane is coming up.
    ///
    /// Kept apart from [`CallPhase::Connecting`] because the two are not the same claim: there the
    /// peer's `<accept>` arrived, here the user picked up and the media is still being negotiated.
    Accepted,
    /// Media is up. The duration runs from this point, not from the button press.
    Active,
    /// The call is over. [`CallUpdate::outcome`] says how.
    Ended,
    /// The call never came up. [`CallUpdate::outcome`] says why.
    Failed,
}

impl CallPhase {
    /// Whether the call is still doing something; the UI keeps its surface for these.
    pub fn is_live(self) -> bool {
        matches!(
            self,
            Self::Placing
                | Self::Dialing
                | Self::Ringing
                | Self::Incoming
                | Self::Connecting
                | Self::Accepted
                | Self::Active
        )
    }

    /// Whether the microphone, camera and device pickers apply.
    pub fn is_connected(self) -> bool {
        matches!(self, Self::Connecting | Self::Accepted | Self::Active)
    }

    /// Whether the two sides are talking, or are about to: the phases the peer is on the line for.
    pub fn is_answered(self) -> bool {
        matches!(self, Self::Connecting | Self::Accepted | Self::Active)
    }
}

/// How a call turned out: the status a call entry keeps, as data rather than as a finished
/// sentence. The words belong to the call screen and the history, which know the reader's
/// language; each value comes from the peer's signaling or from the media plane, never from the
/// button that was pressed.
pub use crate::model::CallStatus as CallOutcome;

/// Why the peer's audio is not becoming sound, as the engine names the reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SilenceReason {
    /// This build has no decoder for the codec the call settled on.
    NoDecoder,
    /// The peer's media is arriving but does not authenticate.
    AuthenticationFailing,
    /// Packets arrive that are not the payload type the call negotiated.
    UnexpectedPayloadType,
    /// The decoder keeps refusing frames.
    CodecRejectingFrames,
    /// The decode path keeps changing its mind about the payload grammar.
    CodecFlapping,
    /// Something else; the counters are the only detail there is.
    Unknown,
}

/// What the engine reports about the peer's audio when it is not arriving.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerAudio {
    /// Audio is arriving and none of it is becoming sound.
    Silent(SilenceReason),
    /// No audio is arriving at all, which is a transport problem rather than a codec one.
    Stalled,
}

/// One snapshot of the live call, or of the call that just ended.
#[derive(Clone, Debug, PartialEq)]
pub struct CallUpdate {
    /// Changes with every call, so the UI can tell a stale update from the current one.
    pub generation: u64,
    /// The chat the call belongs to, as an archive id.
    pub chat: String,
    /// Which side placed the call.
    pub direction: CallDirection,
    /// Whether this call carries video.
    pub video: bool,
    pub phase: CallPhase,
    /// Set when the call became active; the timer counts from here.
    pub started: Option<Instant>,
    pub muted: bool,
    pub camera_on: bool,
    /// Whether the peer's picture has started arriving.
    pub remote_video: bool,
    /// Whether the peer asked to turn a voice call into a video call and waits for an answer.
    pub video_requested: bool,
    /// How the call ended, once it has. Worded by the call screen, stored by the call history.
    pub outcome: Option<CallOutcome>,
    /// Set when the engine says the peer's audio is missing, so a silent call says why instead of
    /// just being silent.
    pub peer_audio: Option<PeerAudio>,
    /// Devices the user picked that the machine no longer has, so the call screen can name them
    /// instead of quietly using another one.
    pub lost_devices: Vec<LostDevice>,
    /// The selected input, output and camera nodes; `None` means the system default.
    pub microphone: Option<String>,
    pub speaker: Option<String>,
    pub camera: Option<String>,
    /// Who else is on the call, for a group call; `None` for a one-to-one call.
    pub group: Option<GroupCall>,
}

/// The people on a group call besides us, as the call's roster names them.
///
/// The roster is the call service's own: who it says has joined, and who it says was asked. The
/// ids are the library's until the worker files them under the chat ids the rest of the app uses
/// (`Worker::canonical`), and our own entry is left out there.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GroupCall {
    /// Who rang us: the person who started the call, or who added us to one already running.
    pub caller: Option<String>,
    /// Everyone else on the roster, in the order the call service lists them.
    pub participants: Vec<Participant>,
    /// Whether we joined through a call link, which belongs to no chat.
    pub link: bool,
}

/// One person on a group call's roster.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Participant {
    pub id: String,
    /// Whether the call service counts them as in the call, rather than only asked to it.
    pub joined: bool,
    /// Whether their camera's picture is arriving: set by their first frame, cleared when they
    /// say their video stopped or leave the call.
    pub video: bool,
}

impl GroupCall {
    /// The people the call service counts as in the call.
    pub fn joined(&self) -> impl Iterator<Item = &Participant> {
        self.participants.iter().filter(|person| person.joined)
    }
}

impl CallUpdate {
    /// The snapshot of a call whose button was just pressed: ours, not yet offered, with no
    /// devices picked. The window draws it at once; the worker's own updates replace it.
    pub fn placing(generation: u64, chat: String, video: bool) -> Self {
        Self {
            generation,
            chat,
            direction: CallDirection::Outgoing,
            video,
            phase: CallPhase::Placing,
            started: None,
            muted: false,
            camera_on: false,
            remote_video: false,
            video_requested: false,
            outcome: None,
            peer_audio: None,
            lost_devices: Vec::new(),
            microphone: None,
            speaker: None,
            camera: None,
            group: None,
        }
    }

    /// The same call, failed before its offer went out: the screen says it could not connect and
    /// goes, rather than waiting on a call that will never ring.
    pub fn not_placed(mut self) -> Self {
        self.phase = CallPhase::Failed;
        self.outcome = Some(CallOutcome::Failed);
        self
    }
}

/// A microphone or speaker the platform reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioDevice {
    /// The name the platform reports and accepts back when a stream is opened.
    pub id: String,
    /// What the picker shows.
    pub label: String,
}

/// A camera the platform reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CameraDevice {
    /// What [`crate::camera::Source::open`] takes back: an index, or the platform's own id.
    pub id: String,
    /// The name the platform reports.
    pub label: String,
}

/// The devices the call screen offers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceList {
    pub microphones: Vec<AudioDevice>,
    pub speakers: Vec<AudioDevice>,
    pub cameras: Vec<CameraDevice>,
}

/// Which of the three kinds of call device a selection belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Microphone,
    Speaker,
    Camera,
}

/// What a set of selections resolved to against the machine's own list.
#[derive(Clone, Debug, Default)]
pub struct ResolvedDevices {
    pub microphone: Option<String>,
    pub speaker: Option<String>,
    pub camera: Option<String>,
    /// Whatever had to be given up, so the call screen can say so in the reader's language.
    pub lost_devices: Vec<LostDevice>,
}

/// A device the call had to give up because the machine no longer has it.
///
/// Kept as data rather than as a finished sentence: the words belong to the call screen, which is
/// the only place that knows the reader's language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LostDevice {
    pub kind: DeviceKind,
    /// The friendliest name at hand: what the picker showed, or the node itself.
    pub name: String,
}

/// The friendliest name for `id` in `list`: the description the picker showed, or the node itself.
pub fn device_name(list: &DeviceList, kind: DeviceKind, id: &str) -> String {
    let label = match kind {
        DeviceKind::Microphone => list
            .microphones
            .iter()
            .find(|device| device.id == id)
            .map(|device| device.label.clone()),
        DeviceKind::Speaker => list
            .speakers
            .iter()
            .find(|device| device.id == id)
            .map(|device| device.label.clone()),
        DeviceKind::Camera => list
            .cameras
            .iter()
            .find(|device| device.id == id)
            .map(|device| device.label.clone()),
    };
    label.unwrap_or_else(|| id.to_owned())
}

/// Checks selections against what the machine has, falling back to the system default.
///
/// A selection is kept when its list is empty: that means device discovery could not run, not that
/// nothing is plugged in, and dropping the choice over a broken tool would be a lie. The returned
/// note names what moved, so a call that fell back says so instead of quietly using another device.
pub fn resolve_devices(
    list: &DeviceList,
    microphone: Option<String>,
    speaker: Option<String>,
    camera: Option<String>,
) -> ResolvedDevices {
    let microphones: Vec<&str> = list.microphones.iter().map(|d| d.id.as_str()).collect();
    let speakers: Vec<&str> = list.speakers.iter().map(|d| d.id.as_str()).collect();
    let cameras: Vec<&str> = list.cameras.iter().map(|d| d.id.as_str()).collect();
    let mut lost: Vec<LostDevice> = Vec::new();

    let checked =
        |kind: DeviceKind, wanted: Option<String>, known: &[&str], lost: &mut Vec<LostDevice>| {
            match wanted {
                // Nothing is known about a list that came back empty, so the choice stands.
                Some(id) if known.is_empty() => Some(id),
                Some(id) if known.contains(&id.as_str()) => Some(id),
                Some(id) => {
                    lost.push(LostDevice {
                        kind,
                        name: device_name(list, kind, &id),
                    });
                    None
                }
                None => None,
            }
        };
    let microphone = checked(DeviceKind::Microphone, microphone, &microphones, &mut lost);
    let speaker = checked(DeviceKind::Speaker, speaker, &speakers, &mut lost);
    // A camera has no system default the way an audio device does, so the picker's "First camera"
    // has to mean the first camera the machine reports. Without this a fresh install — whose stored
    // camera is `None` — could not start or answer a video call at all, and could not even reach the
    // in-call picker to choose a node. A named camera the machine no longer has falls back the same
    // way, and the fallback is reported, so the screen can say the old one is gone.
    let camera = checked(DeviceKind::Camera, camera, &cameras, &mut lost)
        .or_else(|| default_camera(list, None));

    ResolvedDevices {
        microphone,
        speaker,
        camera,
        lost_devices: lost,
    }
}

/// Reads the picker's "First camera" as a camera that exists.
///
/// A camera has no system default the way a sound card does, so `None` means the first camera the
/// machine reports — the same reading [`resolve_devices`] applies when a call starts. A camera the
/// machine no longer has falls back the same way. `None` comes back only when nothing is named and
/// nothing is listed, and then the caller's own failure path says so rather than this inventing one.
///
/// Split out of [`resolve_devices`] so the two paths that change a camera mid-call — upgrading a
/// voice call to video, and picking a device while one is running — read the default exactly as a
/// call's startup does, and so that reading is testable without a machine.
pub fn default_camera(list: &DeviceList, camera: Option<String>) -> Option<String> {
    let named = camera.filter(|id| !id.is_empty());
    match named {
        // A camera the machine still reports is kept, even when the list is empty: an empty list
        // means discovery could not run, not that nothing is plugged in.
        Some(id) if list.cameras.is_empty() || list.cameras.iter().any(|c| c.id == id) => Some(id),
        // A camera the machine no longer has, or no choice at all: the first one it does have.
        _ => list.cameras.first().map(|camera| camera.id.clone()),
    }
}

/// The corner preview's size for a capture of `capture`, in the capture's own orientation.
fn preview_size(capture: (usize, usize)) -> (usize, usize) {
    let bounds = if capture.1 > capture.0 {
        (PREVIEW_SHORT_SIDE, PREVIEW_LONG_SIDE)
    } else {
        (PREVIEW_LONG_SIDE, PREVIEW_SHORT_SIDE)
    };
    crate::camera::fit_even(capture, bounds)
}

/// Whether an offer announces video.
pub fn offer_is_video(action: &CallAction) -> bool {
    match action {
        CallAction::Offer { is_video, .. } => *is_video,
        _ => false,
    }
}

/// Whether a stanza is an offer, which is the only one that may start ringing.
pub fn is_offer(action: &CallAction) -> bool {
    matches!(action, CallAction::Offer { .. })
}

// ---------------------------------------------------------------------------
// Device discovery
// ---------------------------------------------------------------------------

/// Lists microphones, speakers and cameras for the call screen.
///
/// They come from rodio, the same layer a call opens them through, so a picker offers what a call
/// can actually take, on every platform: PipeWire or ALSA, CoreAudio, WASAPI.
pub fn devices() -> DeviceList {
    DeviceList {
        microphones: audio_devices(crate::call_audio::microphones()),
        speakers: audio_devices(crate::call_audio::speakers()),
        cameras: cameras(),
    }
}

/// Wraps the names rodio reports as the call screen's device entries.
fn audio_devices(found: Vec<(String, String)>) -> Vec<AudioDevice> {
    found
        .into_iter()
        .map(|(id, label)| AudioDevice { id, label })
        .collect()
}

/// Lists the cameras the platform reports, through the camera module.
fn cameras() -> Vec<CameraDevice> {
    crate::camera::cameras()
        .into_iter()
        .map(|(id, label)| CameraDevice { id, label })
        .collect()
}

// ---------------------------------------------------------------------------
// Audio
// ---------------------------------------------------------------------------

/// The engine's audio clock, and the microphone and the speaker the engine talks to.
///
/// Both live in [`crate::call_audio`], which opens them through rodio the way the rest of the app
/// already plays and records. A device change replaces only the reader or the writer behind the
/// engine's channel, so nothing about the call's codec or stream state is rebuilt for it.
pub use crate::call_audio::{AudioInput, AudioOutput, RATE};

/// What a call still cannot open on this machine, if anything.
///
/// Read against the devices this call would really open rather than the system defaults alone: a
/// machine whose default microphone is busy still takes a call aimed at one the user picked, so
/// the check follows the same selections the streams will.
fn missing_audio_device(microphone: Option<&str>, speaker: Option<&str>) -> Option<&'static str> {
    crate::call_audio::unavailable(microphone, speaker)
}

/// The same check, run where it cannot stall the caller.
///
/// `missing_audio_device` opens every device to name it, which is a blocking enumeration. Both
/// `Call::place` and `Call::answer` are awaited on the worker's own async loop, and the worker
/// already moves its other device discovery to `spawn_blocking` for exactly this reason: a slow or
/// wedged audio stack would otherwise hold up message handling and signaling while a call starts.
/// The microphone and speaker names are cloned in, so nothing borrowed crosses the thread.
async fn missing_audio_device_off_thread(
    microphone: Option<String>,
    speaker: Option<String>,
) -> Option<&'static str> {
    tokio::task::spawn_blocking(move || {
        missing_audio_device(microphone.as_deref(), speaker.as_deref())
    })
    .await
    // A panicked check is not a machine without devices: refusing the call over it would be a lie,
    // so it is treated as "nothing missing" and the ordinary open path reports what it finds.
    .unwrap_or(None)
}

/// Whether a requested video path really came up.
///
/// The user pressed the video button, or the peer offered video; connecting with a media type
/// nobody chose and not saying so is worse than refusing. The check runs before any signaling, so a
/// camera or encoder that will not start fails the call instead of quietly downgrading it.
fn require_video(requested: bool, pipeline: bool) -> Result<()> {
    if requested && !pipeline {
        return Err(anyhow!(
            "video could not be started; the camera or its encoder is not available"
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Video
// ---------------------------------------------------------------------------

/// One frame on its way to the UI.
#[derive(Clone, Debug)]
pub enum VideoTick {
    /// Our own camera, for the corner preview.
    Local(Arc<ColorImage>),
    /// The peer's picture, on a one-to-one call.
    Remote(Arc<ColorImage>),
    /// One member's picture on a group call, by the sender the media named, so the call screen
    /// can put it on that member's tile.
    RemoteFrom { sender: Jid, image: Arc<ColorImage> },
    /// The peer's picture could not be decoded: the peer is asked for a keyframe.
    Lost,
}

/// The most group members whose pictures are decoded at once. Each has a decoder of its own; a new
/// sender past the cap takes the place of the one that began first.
const MAX_REMOTE_STREAMS: usize = 16;

/// The largest a group member's picture is converted at, either way round.
const TILE_PICTURE: (usize, usize) = (640, 360);

/// The size a group member's picture of `size` is converted at: its own, or smaller to fit
/// [`TILE_PICTURE`] at its own shape. Never larger than it came.
fn tile_picture(size: (usize, usize)) -> (usize, usize) {
    let (long, short) = (size.0.max(size.1), size.0.min(size.1));
    if long <= TILE_PICTURE.0 && short <= TILE_PICTURE.1 {
        return size;
    }
    crate::camera::fit_even(size, TILE_PICTURE)
}

/// One sender's picture being decoded.
struct RemoteStream {
    decoder: openh264::decoder::Decoder,
    /// Whether a keyframe has arrived, which is where a decoder can start.
    started: bool,
    waiting: FirstKeyframe,
    orientation: u8,
}

/// The camera side of the call: the camera module delivers YUV 4:2:0 at a fixed size, so the
/// encoder and the preview both have a known layout, and the frames the engine wants are complete
/// H.264 Annex-B access units led by an access-unit delimiter.
///
/// A device change starts a fresh thread rather than telling this one to switch: a running encoder
/// carries state the new device's stream cannot inherit.
struct CameraCapture {
    timed: async_channel::Receiver<TimedVideoFrame>,
    /// Cleared when the camera is no longer wanted, and read by the capture loop between frames.
    running: Arc<AtomicBool>,
    /// Set when the engine needs a keyframe; the next frame is encoded as one.
    keyframe: Arc<AtomicBool>,
    /// Set by the capture thread as it leaves, so `stop` can wait for the device to be released
    /// before another camera opens it, without waiting forever on a driver that hangs.
    finished: std::sync::mpsc::Receiver<()>,
}

/// How long a camera is given to open and build its encoder before the video direction gives up.
///
/// The wait is what keeps a busy camera or an encoder that will not start from signaling a video
/// call with no outgoing picture. It covers macOS's permission prompt the first time a camera is
/// used, which the capture thread waits on.
const CAMERA_STARTUP: std::time::Duration = std::time::Duration::from_secs(20);
/// How long a stopping camera is waited for before its thread is left to finish on its own.
const CAMERA_RELEASE: std::time::Duration = std::time::Duration::from_secs(2);

impl CameraCapture {
    /// Opens `device` (the first camera when `None`) on a thread of its own and waits until it
    /// delivers, so a call never signals video it cannot send. Blocking: call it off the async loop.
    ///
    /// `rate` is what the call's rate control last decided; the thread reads it between frames.
    fn start(
        device: Option<String>,
        ticks: async_channel::Sender<VideoTick>,
        rate: Arc<RateShare>,
        diag: Arc<crate::call_diag::CallVideo>,
    ) -> Result<Self> {
        let (frames, timed) = async_channel::bounded::<TimedVideoFrame>(4);
        let running = Arc::new(AtomicBool::new(true));
        let keyframe = Arc::new(AtomicBool::new(false));
        let (ready, came_up) = std::sync::mpsc::channel::<Result<(), String>>();
        let (done, finished) = std::sync::mpsc::channel::<()>();
        let alive = CaptureAlive(Arc::clone(&running));
        let stopping = Arc::clone(&running);
        let forced = Arc::clone(&keyframe);
        std::thread::Builder::new()
            .name("zapfast-camera".to_owned())
            .spawn(move || {
                let _alive = alive;
                let controls = CaptureControls {
                    stopping: &stopping,
                    keyframe: &forced,
                    rate: &rate,
                    diag: &diag,
                };
                capture(device, frames, ticks, controls, ready);
                let _ = done.send(());
            })
            .context("camera thread could not be started")?;
        let capture = Self {
            timed,
            running,
            keyframe,
            finished,
        };
        match came_up.recv_timeout(CAMERA_STARTUP) {
            Ok(Ok(())) => Ok(capture),
            Ok(Err(error)) => Err(anyhow!(error)),
            Err(_) => Err(anyhow!("the camera did not start in time")),
        }
    }

    /// Ends the camera: the loop sees the cleared flag after the frame it is waiting for, and the
    /// device is released before this returns, unless the driver hangs past [`CAMERA_RELEASE`].
    fn stop(&mut self) {
        if self.running.swap(false, Ordering::Relaxed) {
            let _ = self.finished.recv_timeout(CAMERA_RELEASE);
        }
    }

    fn running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

impl Drop for CameraCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Clears a capture's running flag on the way out, including when the thread panics.
struct CaptureAlive(Arc<AtomicBool>);

impl Drop for CaptureAlive {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

/// The `VideoSource` the engine pulls access units from.
struct LocalVideoSource {
    timed: async_channel::Receiver<TimedVideoFrame>,
    /// Never fed: the engine uses the timestamped channel above.
    empty: async_channel::Receiver<Vec<u8>>,
}

impl VideoSource for LocalVideoSource {
    fn frames(&self) -> async_channel::Receiver<Vec<u8>> {
        self.empty.clone()
    }

    fn timed_frames(&self) -> Option<async_channel::Receiver<TimedVideoFrame>> {
        Some(self.timed.clone())
    }

    fn rtp_timestamp_stride(&self) -> u32 {
        VIDEO_TS_STRIDE
    }
}

/// The `VideoSink` the engine hands the peer's access units to.
struct RemoteVideoSink {
    tx: async_channel::Sender<VideoFrame>,
}

impl VideoSink for RemoteVideoSink {
    fn playout(&self) -> async_channel::Sender<VideoFrame> {
        self.tx.clone()
    }
}

/// Everything the video direction owns, plus what a device change or camera toggle needs.
pub struct VideoPipeline {
    ticks: async_channel::Sender<VideoTick>,
    remote_tx: async_channel::Sender<VideoFrame>,
    camera: Option<CameraCapture>,
    device: Option<String>,
    /// How much the camera sends, adapted to the peer's reports. It outlives a camera switch: the
    /// path has not changed because the camera did.
    rate: RateControl,
    rate_share: Arc<RateShare>,
    /// What both directions' pictures did, counted for the heartbeat, and dumped when asked.
    diag: Arc<crate::call_diag::CallVideo>,
    /// What the peer's RTCP feedback has asked of our video, for the heartbeat's log line.
    feedback: crate::video_rate::PeerFeedback,
    /// The WhatsApp extension of the peer's latest report on our video: 24 bytes of transport
    /// statistics after the standard report block, of which only the bytes and the rate received
    /// are decoded ([`crate::video_rate::peer_receiving`]). Logged whole so a call's numbers can be
    /// set beside what we know we sent.
    report_extension: Vec<u8>,
    /// The peer's latest interarrival jitter for our video, in the 90 kHz clock, for the log.
    peer_jitter: Option<u32>,
}

impl VideoPipeline {
    /// Starts the camera and the peer's decoder. The returned receiver carries both directions'
    /// frames to the UI. Waits for the camera, so it runs on a blocking thread.
    pub async fn start(
        device: Option<String>,
        call_id: &str,
    ) -> Result<(Self, async_channel::Receiver<VideoTick>)> {
        let diag = Arc::new(crate::call_diag::CallVideo::new(call_id));
        tokio::task::spawn_blocking(move || Self::start_blocking(device, diag))
            .await
            .map_err(|error| anyhow!("the camera thread failed: {error}"))?
    }

    fn start_blocking(
        device: Option<String>,
        diag: Arc<crate::call_diag::CallVideo>,
    ) -> Result<(Self, async_channel::Receiver<VideoTick>)> {
        // Room for a picture from each of several group members between two drains.
        let (ticks, tick_rx) = async_channel::bounded::<VideoTick>(8);
        let (remote_tx, remote_rx) = async_channel::bounded::<VideoFrame>(8);
        decode_remote(remote_rx, ticks.clone(), Arc::clone(&diag));
        let rate_share = Arc::new(RateShare::default());
        let rate = RateControl::new(Arc::clone(&rate_share));
        // A machine without a camera, or one whose camera is busy, still takes part in a video
        // call: it sees the other side, its own camera reads as off, and the camera button says
        // why when pressed.
        let camera = match CameraCapture::start(
            device.clone(),
            ticks.clone(),
            Arc::clone(&rate_share),
            Arc::clone(&diag),
        ) {
            Ok(camera) => Some(camera),
            Err(error) => {
                log::warn!("[CALL] the video call goes on without our camera: {error}");
                None
            }
        };
        Ok((
            Self {
                ticks,
                remote_tx,
                camera,
                device,
                rate,
                rate_share,
                feedback: Default::default(),
                report_extension: Vec::new(),
                peer_jitter: None,
                diag,
            },
            tick_rx,
        ))
    }

    /// The rate and picture the camera is sent at right now.
    pub fn rate(&self) -> crate::video_rate::Setting {
        self.rate.setting()
    }

    /// One RTCP packet from the peer, read for what it says about our video.
    fn on_rtcp(
        &mut self,
        reports_audio: bool,
        reports_video: bool,
        blocks: &[(u32, u8)],
        estimates: &[u32],
    ) -> Option<RateChange> {
        self.rate.on_rtcp(
            reports_audio,
            reports_video,
            blocks,
            estimates,
            Instant::now(),
        )
    }

    /// What the peer says it receives of our video, from its report's extension.
    fn on_peer_receiving(
        &mut self,
        receiving: crate::video_rate::PeerReceiving,
    ) -> Option<RateChange> {
        self.rate.on_peer_receiving(receiving, Instant::now())
    }

    /// The relay connection could not take all the video queued for it.
    fn on_send_drops(&mut self) -> Option<RateChange> {
        self.rate.on_send_drops(Instant::now())
    }

    /// The camera thread could not hand all its frames to the engine.
    fn on_capture_drops(&mut self) -> Option<RateChange> {
        self.rate.on_capture_drops(Instant::now())
    }

    /// The FEC bytes the engine has sent so far in the call, which the encoder's share of the rate
    /// is measured against.
    fn on_fec_sent(&mut self, fec_bytes_total: u64) {
        self.rate.on_fec_sent(fec_bytes_total);
    }

    fn source(&self) -> LocalVideoSource {
        let (empty_tx, empty) = async_channel::bounded::<Vec<u8>>(1);
        drop(empty_tx);
        let timed = match &self.camera {
            Some(camera) => camera.timed.clone(),
            // No camera running: a closed timestamped channel, so the feed simply sees no frames.
            None => {
                let (unused, closed) = async_channel::bounded::<TimedVideoFrame>(1);
                drop(unused);
                closed
            }
        };
        LocalVideoSource { timed, empty }
    }

    fn sink(&self) -> RemoteVideoSink {
        RemoteVideoSink {
            tx: self.remote_tx.clone(),
        }
    }

    /// Releases the camera. A resume builds a new source, so nothing here is kept.
    pub fn pause_camera(&mut self) {
        if let Some(mut camera) = self.camera.take() {
            camera.stop();
        }
    }

    /// Starts capturing from `device` again, reusing the frame channel the UI already drains.
    /// Waits for the camera, so it runs on a blocking thread.
    pub async fn resume_camera(&mut self, device: Option<String>) -> Result<()> {
        let previous = self.camera.take();
        self.device = device.clone();
        let ticks = self.ticks.clone();
        let rate = Arc::clone(&self.rate_share);
        let diag = Arc::clone(&self.diag);
        // The old camera is released first, on the same blocking thread, so the new one never
        // races it for the device.
        let camera = tokio::task::spawn_blocking(move || {
            drop(previous);
            CameraCapture::start(device, ticks, rate, diag)
        })
        .await
        .map_err(|error| anyhow!("the camera thread failed: {error}"))??;
        self.camera = Some(camera);
        Ok(())
    }

    /// Whether the camera is capturing right now. A capture thread that ended (an unplugged
    /// camera, one another program took) reads as off, so the frame channel and the controls agree.
    pub fn camera_running(&self) -> bool {
        self.camera.as_ref().is_some_and(CameraCapture::running)
    }

    /// Asks the encoder for a keyframe, which the engine needs whenever the peer's decoder has
    /// lost track (a lost packet, a resumed camera).
    pub fn force_keyframe(&self) {
        if let Some(camera) = &self.camera {
            camera.keyframe.store(true, Ordering::Relaxed);
        }
    }

    /// Ends the video direction: the capture thread stops and the device is released.
    pub fn shutdown(&mut self) {
        if let Some(mut camera) = self.camera.take() {
            camera.stop();
        }
    }
}

/// Captures from the camera, encodes each frame to an Annex-B access unit, and feeds the preview.
///
/// Runs on its own thread: the platforms' capture objects are not all `Send`, the camera read is
/// blocking, and the `openh264` encoder is not shared. Everything it hands out goes through a
/// bounded channel with `try_send`, so a stalled consumer drops a frame instead of stalling the
/// camera.
fn capture(
    device: Option<String>,
    timed: async_channel::Sender<TimedVideoFrame>,
    ticks: async_channel::Sender<VideoTick>,
    controls: CaptureControls<'_>,
    ready: std::sync::mpsc::Sender<Result<(), String>>,
) {
    // The camera's own shape decides the frame the encoder runs at, within the budget either way
    // round, so a portrait camera is encoded portrait.
    let budget = (CAPTURE_LONG_SIDE, CAPTURE_SHORT_SIDE);
    let mut source = match crate::camera::Source::open(device.as_deref(), budget, VIDEO_FPS) {
        Ok(source) => source,
        Err(error) => {
            log::error!("[CALL] camera capture could not start: {error}");
            let _ = ready.send(Err(error));
            return;
        }
    };
    let plan = SendPlan::new(source.size(), f64::from(source.fps()), controls.rate.load());
    let encoder = match CameraEncoder::new(&plan) {
        Ok(encoder) => encoder,
        Err(error) => {
            log::error!("[CALL] camera encoder could not start: {error}");
            source.stop();
            ticks.close();
            let _ = ready.send(Err(format!("the camera encoder could not start: {error}")));
            return;
        }
    };
    // The camera and the encoder are both up: only now is a video call worth signaling.
    let _ = ready.send(Ok(()));
    capture_frames(&mut source, controls, &timed, &ticks, plan, encoder);
    source.stop();
}

/// What the camera thread reads between frames.
struct CaptureControls<'a> {
    /// Cleared when the camera is no longer wanted.
    stopping: &'a AtomicBool,
    /// Set when the engine needs a keyframe; the next frame sent is encoded as one.
    keyframe: &'a AtomicBool,
    /// The rate and the picture size the call's rate control last chose.
    rate: &'a RateShare,
    /// Where what is sent is counted, and dumped when asked.
    diag: &'a crate::call_diag::CallVideo,
}

/// How the camera's frames are being sent right now: at what size, how many of them, and how fast.
#[derive(Debug)]
struct SendPlan {
    /// The camera's own picture, within the capture budget.
    full: (usize, usize),
    /// The camera's frame rate as last measured (or as the driver granted it, to begin with).
    camera_fps: f64,
    setting: crate::video_rate::Setting,
    /// The picture sent: the camera's own shape, within the setting's step.
    size: (usize, usize),
    /// One frame sent for every this many the camera gives.
    every: u32,
}

impl SendPlan {
    fn new(full: (usize, usize), camera_fps: f64, setting: crate::video_rate::Setting) -> Self {
        let tier = setting.tier();
        let rounded = camera_fps.round().clamp(1.0, f64::from(u32::MAX)) as u32;
        Self {
            full,
            camera_fps,
            setting,
            size: crate::camera::fit_even(full, (tier.long_side, tier.short_side)),
            every: crate::video_rate::decimation(rounded, tier.fps),
        }
    }

    /// Frames a second actually sent.
    fn sent_fps(&self) -> f64 {
        self.camera_fps / f64::from(self.every)
    }

    /// Bytes in one YUV 4:2:0 picture of the size sent.
    fn frame_len(&self) -> usize {
        let (width, height) = self.size;
        width * height + 2 * (width / 2) * (height / 2)
    }

    fn describe(&self) -> String {
        format!(
            "{}x{} at {:.1} fps (one in {} camera frames), {} kbps",
            self.size.0,
            self.size.1,
            self.sent_fps(),
            self.every,
            self.setting.bitrate / 1000
        )
    }
}

/// The encoder for one step of picture size and frame rate, and the rate it runs at.
struct CameraEncoder {
    encoder: openh264::encoder::Encoder,
    /// The rate the encoder was last moved to; `None` until the first frame has initialised it.
    applied: Option<u32>,
    wanted: u32,
    /// Whether moving the rate has failed once, so the log says it once.
    refused: bool,
}

impl CameraEncoder {
    /// Built at the step's ceiling. openh264 takes its rate limit from the configuration when the
    /// first frame initialises it, and a rate set later cannot pass that limit, so the wanted rate
    /// is applied once the encoder is running and can then move anywhere under the ceiling.
    fn new(plan: &SendPlan) -> std::result::Result<Self, openh264::Error> {
        Ok(Self {
            encoder: video_encoder(plan.sent_fps(), plan.setting.tier().max_bitrate)?,
            applied: None,
            wanted: plan.setting.bitrate,
            refused: false,
        })
    }

    fn set_bitrate(&mut self, bitrate: u32) {
        self.wanted = bitrate;
    }

    /// Encodes one picture into an access unit led by an access-unit delimiter, so the peer's
    /// depacketizer sees the boundaries an encoder would have produced.
    fn encode(
        &mut self,
        planes: &Yuv420<'_>,
        keyframe: bool,
    ) -> std::result::Result<Vec<u8>, openh264::Error> {
        if keyframe {
            self.encoder.force_intra_frame();
        }
        let mut unit = vec![0x00, 0x00, 0x00, 0x01, 0x09, 0xF0];
        self.encoder.encode(planes)?.write_vec(&mut unit);
        if self.applied != Some(self.wanted) {
            if set_encoder_bitrate(&mut self.encoder, self.wanted) {
                self.applied = Some(self.wanted);
            } else if !self.refused {
                self.refused = true;
                log::warn!("[CALL] the camera encoder would not change its bitrate");
            }
        }
        Ok(unit)
    }
}

/// How often the camera sends a keyframe unasked.
///
/// A keyframe is several times a delta frame's size, and it leaves at once: on a path near its
/// limit it is what gets lost. Every two seconds took a large share of a call's rate, and the peer
/// asks for one whenever its decoder loses its reference (a real call's peer asked forty times in
/// four minutes), so the unasked ones are only a backstop.
const KEYFRAME_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

/// The H.264 encoder a call's camera feeds: Constrained Baseline, which is what WhatsApp's clients
/// decode, with a keyframe every [`KEYFRAME_EVERY`] so a peer that lost one recovers on its own.
///
/// Rate control keeps to the bitrate rather than to a quality: on a call the bitrate is what the
/// path carries, and an encoder that overshoots it on a busy scene is sending loss.
fn video_encoder(
    fps: f64,
    bitrate: u32,
) -> std::result::Result<openh264::encoder::Encoder, openh264::Error> {
    use openh264::encoder::{
        BitRate, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, Profile, RateControlMode,
    };
    let fps = fps.clamp(1.0, f64::from(VIDEO_FPS));
    let config = EncoderConfig::new()
        .profile(Profile::Baseline)
        .bitrate(BitRate::from_bps(bitrate))
        .rate_control_mode(RateControlMode::Bitrate)
        .max_frame_rate(FrameRate::from_hz(fps as f32))
        .intra_frame_period(IntraFramePeriod::from_num_frames(
            (fps * KEYFRAME_EVERY.as_secs_f64()).round() as u32,
        ))
        .skip_frames(false);
    Encoder::with_api_config(openh264::OpenH264API::from_source(), config)
}

/// Moves a running encoder to `bitrate` without restarting it, so a rate change costs no keyframe.
/// `false` if openh264 refused.
fn set_encoder_bitrate(encoder: &mut openh264::encoder::Encoder, bitrate: u32) -> bool {
    let mut info = openh264_sys2::SBitrateInfo {
        iLayer: openh264_sys2::SPATIAL_LAYER_ALL,
        iBitrate: std::os::raw::c_int::try_from(bitrate).unwrap_or(std::os::raw::c_int::MAX),
    };
    // SAFETY: `ENCODER_OPTION_BITRATE` reads one `SBitrateInfo` through the pointer during the
    // call, and `info` lives across it. The encoder has encoded a frame, so it is initialised,
    // which `SetOption` requires. The openh264 crate re-applies its configured rate only when the
    // picture size changes, and a camera encoder never sees a new size: a new size gets a new
    // encoder.
    let status = unsafe {
        encoder.raw_api().set_option(
            openh264_sys2::ENCODER_OPTION_BITRATE,
            (&raw mut info).cast::<std::os::raw::c_void>(),
        )
    };
    status == 0
}

/// Reads frames from one camera until the call stops it, encoding and previewing the ones it sends.
///
/// The camera is thinned to an even fraction of its own rate, every `n`th frame, so the frames
/// sent are evenly spaced, and each carries the time the camera took it, snapped to that spacing.
/// The frames in between are let go without being converted. The picture size, the frame rate and
/// the bitrate follow the call's rate control, read between frames.
fn capture_frames(
    source: &mut crate::camera::Source,
    controls: CaptureControls<'_>,
    timed: &async_channel::Sender<TimedVideoFrame>,
    ticks: &async_channel::Sender<VideoTick>,
    mut plan: SendPlan,
    mut encoder: CameraEncoder,
) {
    let mut bytes = vec![0u8; plan.frame_len()];
    let mut clock = FrameClock::new(Instant::now(), plan.sent_fps());
    let mut cadence = Cadence::new(plan.camera_fps);
    // Camera frames since the last one sent; the next is sent when this reaches zero.
    let mut countdown = 0u32;
    // Set when a frame did not make it to the engine: the peer's decoder has lost its reference,
    // so nothing but a frame it can start from is worth sending, and that not too often, or a
    // queue that is full because the path is short is filled with keyframes.
    let mut resync = false;
    let mut resynced_at: Option<Instant> = None;
    log::info!(
        "[CALL] camera enabled size={}x{} sending {}",
        plan.full.0,
        plan.full.1,
        plan.describe()
    );
    loop {
        if !controls.stopping.load(Ordering::Relaxed) {
            log::info!("[CALL] camera disabled");
            return;
        }
        let wanted = controls.rate.load();
        if wanted.tier != plan.setting.tier {
            let next = SendPlan::new(plan.full, plan.camera_fps, wanted);
            match CameraEncoder::new(&next) {
                Ok(fresh) => {
                    encoder = fresh;
                    clock.respace(next.sent_fps());
                    plan = next;
                    bytes.resize(plan.frame_len(), 0);
                    countdown = 0;
                    log::info!("[CALL] camera now sends {}", plan.describe());
                }
                // The step stays as it was; the next frame tries again.
                Err(error) => log::warn!("[CALL] the camera encoder could not restart: {error}"),
            }
        }
        plan.setting.bitrate = wanted.bitrate;
        // The rate chosen is what goes on the wire, and the engine's FEC takes its share of it.
        encoder.set_bitrate(controls.rate.encoder_bitrate(wanted));

        let send = countdown == 0;
        let read = if send {
            source.read(&mut bytes, plan.size)
        } else {
            source.skip()
        };
        let captured = match read {
            crate::camera::Read::Frame(captured) => captured,
            // Not a picture: the frame that was due goes out with the next one instead.
            crate::camera::Read::Idle => continue,
            crate::camera::Read::Ended => {
                log::warn!("[CALL] the camera stopped delivering frames");
                return;
            }
        };
        countdown = if send { plan.every - 1 } else { countdown - 1 };
        // A camera that slows down in the dark, or never ran at the rate it claimed, gets a new
        // fraction, so the frames sent stay close to the step's rate.
        if let Some(measured) = cadence.observe(captured) {
            let next = SendPlan::new(plan.full, measured, plan.setting);
            if next.every != plan.every {
                match CameraEncoder::new(&next) {
                    Ok(fresh) => {
                        encoder = fresh;
                        clock.respace(next.sent_fps());
                        plan = next;
                        countdown = 0;
                        log::info!(
                            "[CALL] camera runs at {measured:.1} fps, sending {}",
                            plan.describe()
                        );
                    }
                    Err(error) => {
                        log::warn!("[CALL] the camera encoder could not restart: {error}")
                    }
                }
            } else {
                plan.camera_fps = measured;
                clock.respace(plan.sent_fps());
            }
        }
        if !send {
            continue;
        }

        let (width, height) = plan.size;
        let (luma_len, chroma_len) = (width * height, (width / 2) * (height / 2));
        let luma = &bytes[..luma_len];
        let chroma_u = &bytes[luma_len..luma_len + chroma_len];
        let chroma_v = &bytes[luma_len + chroma_len..luma_len + 2 * chroma_len];
        let planes = Yuv420 {
            y: luma,
            u: chroma_u,
            v: chroma_v,
            size: plan.size,
        };
        let resync_due = resync && resynced_at.is_none_or(|at| at.elapsed() >= RESYNC_GAP);
        // A delta frame the peer cannot decode only adds to a queue that is already full.
        let encode = !resync || resync_due;
        let keyframe = encode && (controls.keyframe.swap(false, Ordering::Relaxed) || resync_due);
        if resync_due {
            resynced_at = Some(Instant::now());
        }
        if encode {
            match encoder.encode(&planes, keyframe) {
                Ok(unit) => {
                    controls.diag.counters.sent(&unit);
                    controls.diag.sent.write(&unit);
                    let len = unit.len();
                    // The library's clock shares no origin with ours, so the capture goes over
                    // by its age: the peer reads the time from capture to departure from it.
                    let captured_at =
                        whatsapp_rust::wacore::time::Instant::now() - captured.elapsed();
                    let frame = TimedVideoFrame::builder()
                        .data(unit)
                        .timestamp(clock.stamp(captured))
                        .captured_at(captured_at)
                        .build();
                    // A full queue means the wire is behind. The frame is dropped, which leaves the
                    // peer's decoder without its reference, so the next frame sent starts afresh,
                    // and the rate control hears that the path carries less than is sent.
                    if timed.try_send(frame).is_err() {
                        resync = true;
                        controls.rate.note_queue_drop();
                    } else {
                        controls.rate.note_sent(len);
                        if keyframe {
                            resync = false;
                        }
                    }
                }
                Err(error) => log::warn!("[CALL] camera frame could not be encoded: {error}"),
            }
        }

        let image = crate::video::rgb_image(
            luma,
            chroma_u,
            chroma_v,
            (width, width / 2, width / 2),
            (width, height),
            preview_size(plan.size),
            // Our own frames arrive upright from the camera: there is no rotation to undo.
            0,
        );
        let _ = ticks.try_send(VideoTick::Local(Arc::new(image)));
    }
}

/// A camera frame as `openh264` wants it: three contiguous planes with no row padding.
struct Yuv420<'a> {
    y: &'a [u8],
    u: &'a [u8],
    v: &'a [u8],
    size: (usize, usize),
}

impl openh264::formats::YUVSource for Yuv420<'_> {
    fn dimensions(&self) -> (usize, usize) {
        self.size
    }

    fn strides(&self) -> (usize, usize, usize) {
        (self.size.0, self.size.0 / 2, self.size.0 / 2)
    }

    fn y(&self) -> &[u8] {
        self.y
    }

    fn u(&self) -> &[u8] {
        self.u
    }

    fn v(&self) -> &[u8] {
        self.v
    }
}

/// Decodes the peer's access units into pictures for the UI.
///
/// A decoder cannot start on a delta frame, so frames are skipped until the sink marks one as a
/// keyframe. A group call's frames each name their sender, and each sender gets a decoder of its
/// own (at most [`MAX_REMOTE_STREAMS`]), so every member's picture reaches its own tile; a
/// one-to-one call's carry no sender and share one. Runs on its own thread for the same reason the
/// encoder does.
fn decode_remote(
    frames: async_channel::Receiver<VideoFrame>,
    ticks: async_channel::Sender<VideoTick>,
    diag: Arc<crate::call_diag::CallVideo>,
) {
    let spawned = std::thread::Builder::new()
        .name("zapfast-remote-video".to_owned())
        .spawn(move || {
            use openh264::formats::YUVSource as _;

            let mut streams: std::collections::HashMap<Option<Jid>, RemoteStream> =
                std::collections::HashMap::new();
            // The order the senders began in, so the cap lets go of the oldest stream.
            let mut order: std::collections::VecDeque<Option<Jid>> =
                std::collections::VecDeque::new();
            // What the peer's stream says of itself, logged when it changes: which H.264 features
            // the decoder has to handle, and the picture's size.
            let mut profile = None;
            let mut size = (0, 0);
            while let Ok(frame) = frames.recv_blocking() {
                diag.received.write(&frame.data);
                if let Some(seen) = crate::call_diag::stream_profile(&frame.data)
                    && profile != Some(seen)
                {
                    profile = Some(seen);
                    log::info!(
                        "[CALL] the peer's video is H.264 {} (profile_idc={} constraints=0x{:02x} level_idc={})",
                        seen.name(),
                        seen.profile,
                        seen.constraints,
                        seen.level
                    );
                }
                let key = frame.sender.as_ref().map(Jid::to_non_ad);
                if !streams.contains_key(&key) {
                    let Ok(decoder) = openh264::decoder::Decoder::new() else {
                        log::error!("[CALL] remote video decoder could not start");
                        return;
                    };
                    if order.len() >= MAX_REMOTE_STREAMS
                        && let Some(oldest) = order.pop_front()
                    {
                        streams.remove(&oldest);
                    }
                    order.push_back(key.clone());
                    streams.insert(
                        key.clone(),
                        RemoteStream {
                            decoder,
                            started: false,
                            waiting: FirstKeyframe::default(),
                            orientation: 0,
                        },
                    );
                }
                let Some(stream) = streams.get_mut(&key) else {
                    continue;
                };
                if !stream.started {
                    if !frame.keyframe {
                        // The peer's video is flowing and none of it can be decoded: its first
                        // keyframe went missing, or went out before this side could read it. A peer
                        // sends another only when asked, so the screen would otherwise say its
                        // camera is off for the whole call. The engine sends the request (on a
                        // one-to-one call; a group's members send keyframes on their own schedule).
                        if stream.waiting.ask(Instant::now()) {
                            let _ = ticks.try_send(VideoTick::Lost);
                        }
                        continue;
                    }
                    stream.started = true;
                }
                // The peer's camera rotation, which the frames carry so a phone held upright arrives
                // upright. It rides the same bits on a keyframe and on a delta frame, and a peer
                // that turns its phone changes it mid-call, so it is read per frame rather than once.
                let orientation = frame.orientation & 0x03;
                if orientation != stream.orientation {
                    stream.orientation = orientation;
                    log::info!(
                        "[CALL] remote camera orientation is {} degrees",
                        orientation as u32 * 90
                    );
                }
                let turns = crate::video::orientation_turns(orientation);
                match stream.decoder.decode(&frame.data) {
                    Ok(Some(decoded)) => {
                        let (width, height) = decoded.dimensions();
                        if width == 0 || height == 0 {
                            continue;
                        }
                        diag.counters.decoded();
                        if (width, height) != size {
                            size = (width, height);
                            log::info!("[CALL] the peer's video decodes at {width}x{height}");
                        }
                        // A member's picture fills a tile of a grid, never the window, so it is
                        // converted at a tile's size: a dozen full pictures a frame would be more
                        // than the interface thread can upload.
                        let out = if key.is_some() {
                            tile_picture((width, height))
                        } else {
                            (width, height)
                        };
                        let image = Arc::new(crate::video::rgb_image(
                            decoded.y(),
                            decoded.u(),
                            decoded.v(),
                            decoded.strides(),
                            (width, height),
                            out,
                            turns,
                        ));
                        let tick = match key {
                            Some(sender) => VideoTick::RemoteFrom { sender, image },
                            None => VideoTick::Remote(image),
                        };
                        let _ = ticks.try_send(tick);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        diag.counters.undecodable();
                        log::debug!("[CALL] remote video frame was not decodable: {error}");
                        let _ = ticks.try_send(VideoTick::Lost);
                    }
                }
            }
        });
    if let Err(error) = spawned {
        log::error!("[CALL] remote video decoder could not start: {error}");
    }
}

/// RTCP payload-specific feedback (RFC 4585), the packet type a receiver estimate (REMB) rides.
const RTCP_PAYLOAD_SPECIFIC: u8 = 206;

/// Says in the log how the camera's rate moved and why. Numbers only.
///
/// The slow climb on a clean path moves the rate every second or so, so only a new picture size
/// or a cut makes the shipped log; the climb is there at debug level.
fn log_rate_change(call_id: &str, change: RateChange) {
    use crate::video_rate::Reason;
    let reason = match change.reason {
        Reason::Loss(loss) => format!("the peer lost {:.0}% of it", loss * 100.0),
        Reason::Clean => "the peer reports a clean path".to_owned(),
        Reason::SendDrops => "video was dropped before it was sent".to_owned(),
        Reason::CaptureDrops => "camera frames backed up before the engine".to_owned(),
        Reason::Estimate(bitrate) => format!("the peer can take {} kbps", bitrate / 1000),
        Reason::Receiving(bitrate) => {
            format!("the peer receives only {} kbps", bitrate / 1000)
        }
        Reason::Delay(queued) => format!(
            "the round trip stood {} ms above its shortest",
            queued.as_millis()
        ),
    };
    let level = if change.from.tier != change.to.tier || change.reason != Reason::Clean {
        log::Level::Info
    } else {
        log::Level::Debug
    };
    log::log!(
        level,
        "[CALL] video rate call_id={call_id} {} kbps {} -> {} kbps {} ({reason})",
        change.from.bitrate / 1000,
        change.from.tier().name,
        change.to.bitrate / 1000,
        change.to.tier().name
    );
}

/// While the decoder waits for the peer's first keyframe, when to ask for one again.
#[derive(Debug, Default)]
struct FirstKeyframe {
    asked: Option<Instant>,
}

impl FirstKeyframe {
    /// A frame arrived that the decoder cannot start on: whether to ask the peer for a keyframe
    /// now. The first time at once, then once a [`FIRST_KEYFRAME_RETRY`], since a request can be
    /// lost on the way like anything else.
    fn ask(&mut self, now: Instant) -> bool {
        if self
            .asked
            .is_some_and(|at| now.duration_since(at) < FIRST_KEYFRAME_RETRY)
        {
            return false;
        }
        if self.asked.is_none() {
            log::info!("[CALL] the peer's video began without a keyframe; asking for one");
        }
        self.asked = Some(now);
        true
    }
}

// ---------------------------------------------------------------------------
// The call
// ---------------------------------------------------------------------------

/// The reader hung up a call that was still being placed, before its offer went out: nobody was
/// rung, so there is nothing to end with the peer and nothing to file.
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the call was cancelled before it was placed")
    }
}

impl std::error::Error for Cancelled {}

/// Whether ZapFast takes part in group calls: starts them from a group's header or the Calls
/// page, rings for them, answers them, and joins call links.
///
/// On since 0.20, as an experiment: a three-person test carried audio every way and video both
/// ways, though our video froze every few seconds without video FEC in group calls. Turned off, a
/// group has no call button, a call link is not joined, and an incoming group call does not take
/// the screen: the window only says who is calling and to answer on the phone, and sends WhatsApp
/// nothing, so the phone keeps ringing. Both ways stay compiled and tested.
/// Read it through [`group_calls_enabled`], which tests can override on their own thread.
pub const GROUP_CALLS_ENABLED: bool = true;

#[cfg(test)]
thread_local! {
    /// A test's own setting of [`GROUP_CALLS_ENABLED`], for this thread only.
    static GROUP_CALLS_OVERRIDE: std::cell::Cell<Option<bool>> =
        const { std::cell::Cell::new(None) };
}

/// Whether group calls are on: [`GROUP_CALLS_ENABLED`], or in a test what
/// `override_group_calls` set.
pub fn group_calls_enabled() -> bool {
    #[cfg(test)]
    if let Some(enabled) = GROUP_CALLS_OVERRIDE.with(std::cell::Cell::get) {
        return enabled;
    }
    GROUP_CALLS_ENABLED
}

/// Sets group calls on or off on this thread, whatever [`GROUP_CALLS_ENABLED`] says, until the
/// guard it returns is dropped, so the tests of either behaviour keep running with the switch in
/// either position.
#[cfg(test)]
#[must_use]
pub fn override_group_calls(enabled: bool) -> GroupCallsOverride {
    GroupCallsOverride(GROUP_CALLS_OVERRIDE.with(|cell| cell.replace(Some(enabled))))
}

/// Puts back the setting [`override_group_calls`] replaced.
#[cfg(test)]
pub struct GroupCallsOverride(Option<bool>);

#[cfg(test)]
impl Drop for GroupCallsOverride {
    fn drop(&mut self) {
        GROUP_CALLS_OVERRIDE.with(|cell| cell.set(self.0));
    }
}

/// What the window says when a call link is clicked while group calls are off: the English
/// catalog key, translated where it is shown.
pub const CALL_LINKS_OFF: &str = "ZapFast can't join group calls yet: open the link on your phone.";

/// What the window says when the call service refused a group call we started with 427: the
/// English catalog key, which the window translates when the worker hands it over as an error.
pub const GROUP_CALLS_NOT_ENABLED: &str = "WhatsApp hasn't turned on group calls from a computer for your account yet. Ask someone in the group to call it, and answer here.";

/// Why a group call we started did not go out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupRefusal {
    /// WhatsApp has not turned on group calls from linked devices for this account: the call
    /// service refuses such an offer with 427 ("group call not enabled"). Another member can
    /// start the call, and this device answer it.
    pub not_enabled: bool,
    /// The library's own words, for the log and the generic failure.
    pub detail: String,
}

impl GroupRefusal {
    /// Reads a failed group offer. `gate` is the account's `enable_web_group_calling` flag, when
    /// the server has sent it.
    ///
    /// whatsapp-rust does not hand over the code the call service refused an offer with: a 427
    /// ack carries a `group_info` without the counters an accepted one has, and the library reports
    /// only that it could not read it, as a call-service response error. So such an error reads as
    /// the 427 when the account's gate is off, or when the library names the code itself (as its
    /// other call acks do, "... rejected with error 427"). Anything else is an ordinary failure.
    pub fn of(error: &whatsapp_rust::CallError, gate: Option<bool>) -> Self {
        let detail = error.to_string();
        let not_enabled = match error {
            whatsapp_rust::CallError::Response(message) => {
                message.contains("error 427") || gate == Some(false)
            }
            _ => false,
        };
        Self {
            not_enabled,
            detail,
        }
    }
}

/// Counter behind [`CallUpdate::generation`].
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// A generation no call has had yet. The window takes one when the call button is pressed, so the
/// screen it draws at once and the worker's later updates are the same call.
pub fn next_generation() -> u64 {
    GENERATION.fetch_add(1, Ordering::Relaxed) + 1
}

/// A live 1:1 call: its signaling identity, its media, and the state the UI renders.
///
/// One per worker. The worker refuses to start or accept a second call while this exists, which is
/// what keeps a double click from opening two.
pub struct Call {
    generation: u64,
    call_id: String,
    chat: String,
    direction: CallDirection,
    video: bool,
    phase: CallPhase,
    started: Option<Instant>,
    outcome: Option<CallOutcome>,
    peer_audio: Option<PeerAudio>,
    /// The offer of an incoming call that has not been answered. Held because `accept` borrows it.
    incoming: Option<Box<IncomingCall>>,
    handle: Option<Arc<CallHandle>>,
    /// Whether the media plane has reported itself live. Kept apart from the phase because the
    /// relay is allocated as soon as our offer is acked, long before anyone answers: a call is
    /// Active only once the peer has answered *and* there is a media path to talk over.
    media_ready: bool,
    mic: Option<AudioInput>,
    speaker: Option<AudioOutput>,
    video_pipe: Option<VideoPipeline>,
    remote_video: bool,
    microphone: Option<String>,
    speaker_device: Option<String>,
    camera: Option<String>,
    /// The devices the user picked that the machine no longer has, newest check last.
    lost_devices: Vec<LostDevice>,
    /// Whether the call asked the camera to capture. Kept apart from whether frames are really
    /// arriving, so a camera that stopped on its own is told from one the user switched off.
    camera_wanted: bool,
    /// The peer's request to turn this voice call into a video call, which turning the camera on
    /// answers.
    peer_video_request: Option<whatsapp_rust::voip::VideoUpgradeToken>,
    /// When the call was placed or began to ring, Unix seconds: the time its
    /// entry in the chat carries.
    began: i64,
    /// The roster of a group call; `None` for a one-to-one call.
    group: Option<GroupRoster>,
    /// The task joining a call link, until it has joined: hanging up stops it.
    join_task: Option<tokio::task::AbortHandle>,
}

/// What a group call's own signaling says about who is on it.
///
/// Kept in the library's terms (its JIDs and its participant records) and translated for the window
/// in [`Call::update`]; the worker then files each id under the app's own chat ids.
#[derive(Clone, Debug, Default)]
struct GroupRoster {
    /// Who rang us: the offer's sender, or the member who added us to a running call.
    caller: Option<Jid>,
    /// The transaction of the newest snapshot applied. A snapshot is the whole roster, and the call
    /// service numbers them, so one that arrives late never replaces a newer one.
    transaction: Option<u32>,
    participants: Vec<GroupCallParticipant>,
    /// The members whose pictures are arriving, by their roster JID.
    video: Vec<Jid>,
    /// Whether we joined through a call link.
    link: bool,
    /// Us (our privacy id and phone number), on a call we started: the roster lists our own entry
    /// as on the call from the start, which is not anyone answering.
    own: Vec<Jid>,
    /// The people the service has reported as on the call, one record per person, as the snapshot
    /// that first named them did, so someone who leaves before the end is still counted as having
    /// been on it.
    seen: Vec<GroupCallParticipant>,
}

/// Whether two roster records are the same person. The service names one person by phone number
/// in one snapshot and by privacy id in another, or by both at once.
fn same_person(first: &GroupCallParticipant, second: &GroupCallParticipant) -> bool {
    let ids = |person: &GroupCallParticipant| {
        std::iter::once(person.jid.to_non_ad()).chain(person.pn.as_ref().map(Jid::to_non_ad))
    };
    let theirs: Vec<Jid> = ids(second).collect();
    ids(first).any(|id| theirs.contains(&id))
}

impl GroupRoster {
    /// Whether anyone but us is on the call: what answers a group call we started.
    fn others_joined(&self) -> bool {
        self.participants.iter().any(|person| {
            let ours = |own: &Jid| {
                person.jid.user == own.user
                    || person.pn.as_ref().is_some_and(|pn| pn.user == own.user)
            };
            person.is_connected() && !self.own.iter().any(ours)
        })
    }

    /// Notes everyone a newer snapshot puts on the call. A snapshot is the whole roster, so a
    /// member who has left is in none that follows: they are remembered here, where the call's
    /// history entry is what needs them, while `participants` stays the newest word on who is on
    /// the call now. Our own entry is remembered with the rest; the worker, which knows which id
    /// is ours, leaves it out of the count.
    fn note_joined(&mut self, update: &GroupCallUpdate) {
        for person in &update.participants {
            if !person.is_connected() {
                continue;
            }
            if self.seen.iter().any(|known| same_person(known, person)) {
                continue;
            }
            self.seen.push(person.clone());
        }
    }

    /// The roster an offer rang with: `None` unless the offer is a group call's.
    ///
    /// A group offer names its group (`group-jid`), carries the call's roster, or both; a call
    /// among several people with no group behind it carries the roster alone.
    fn of_offer(incoming: &IncomingCall) -> Option<Self> {
        let named = matches!(
            &incoming.action,
            CallAction::Offer {
                group_jid: Some(_),
                ..
            }
        );
        if incoming.group.is_none() && !named {
            return None;
        }
        let mut roster = Self {
            caller: Some(offer_caller(incoming)),
            ..Self::default()
        };
        if let Some(update) = incoming.group.as_deref() {
            roster.apply(update);
        }
        Some(roster)
    }

    /// Takes a newer snapshot of the roster; an older one is ignored. Returns whether it changed.
    fn apply(&mut self, update: &GroupCallUpdate) -> bool {
        if self
            .transaction
            .is_some_and(|applied| update.transaction_id < applied)
        {
            return false;
        }
        self.transaction = Some(update.transaction_id);
        self.note_joined(update);
        if self.participants == update.participants {
            return false;
        }
        self.participants = update.participants.clone();
        // A member who left sends no picture; one who comes back says so with a new first frame.
        let participants = &self.participants;
        self.video.retain(|jid| {
            participants
                .iter()
                .any(|person| &person.jid.to_non_ad() == jid && person.is_connected())
        });
        true
    }

    /// The roster entry a media sender or a signaling source belongs to: the same user, whatever
    /// the device, by privacy id or by phone number.
    fn member(&self, sender: &Jid) -> Option<&GroupCallParticipant> {
        let same = |jid: &Jid| jid.user == sender.user && jid.server == sender.server;
        self.participants
            .iter()
            .find(|person| same(&person.jid) || person.pn.as_ref().is_some_and(same))
    }

    /// Notes a picture from `sender`: the id its tile is drawn under, and whether it is the first
    /// since their video began. `None` for a sender the roster does not name, who has no tile.
    fn picture_from(&mut self, sender: &Jid) -> Option<(String, bool)> {
        let person = self.member(sender)?;
        let (jid, id) = (person.jid.to_non_ad(), participant_id(person));
        let first = !self.video.contains(&jid);
        if first {
            self.video.push(jid);
        }
        Some((id, first))
    }

    /// Notes that `source` stopped sending video. Returns whether that changed anything.
    fn video_stopped(&mut self, source: &Jid) -> bool {
        let Some(jid) = self.member(source).map(|person| person.jid.to_non_ad()) else {
            return false;
        };
        let before = self.video.len();
        self.video.retain(|known| known != &jid);
        self.video.len() != before
    }

    fn view(&self) -> GroupCall {
        GroupCall {
            link: self.link,
            caller: self.caller.as_ref().map(Jid::to_non_ad_string),
            participants: self
                .participants
                .iter()
                .map(|person| Participant {
                    id: participant_id(person),
                    joined: person.is_connected(),
                    video: self.video.contains(&person.jid.to_non_ad()),
                })
                .collect(),
        }
    }
}

/// The id a roster entry is drawn under: the phone number when the roster carries it, so the
/// person is named the way their chat is; otherwise their privacy id.
fn participant_id(person: &GroupCallParticipant) -> String {
    person.pn.as_ref().unwrap_or(&person.jid).to_non_ad_string()
}

/// Who rang us with `incoming`: the member named on the call wrapper when the call service sent it
/// (an invitation into a call already running), else the device that sent the offer, else the call's
/// creator.
fn offer_caller(incoming: &IncomingCall) -> Jid {
    let from_service = matches!(incoming.from.server, whatsapp_rust::prelude::Server::Call);
    let caller = if from_service {
        incoming.participant.as_ref()
    } else {
        Some(&incoming.from)
    };
    caller
        .unwrap_or_else(|| incoming.action.call_creator())
        .to_non_ad()
}

/// The group an offer belongs to, when it names one.
pub fn offer_group(incoming: &IncomingCall) -> Option<Jid> {
    match &incoming.action {
        CallAction::Offer {
            group_jid: Some(group),
            ..
        } => Some(group.clone()),
        _ => incoming
            .group
            .as_deref()
            .and_then(|update| update.group_jid.clone()),
    }
}

/// Whether an offer is a group call's, which rings under its group rather than its sender.
pub fn offer_is_group(incoming: &IncomingCall) -> bool {
    GroupRoster::of_offer(incoming).is_some()
}

impl Call {
    /// Places an outgoing 1:1 call. `video` offers video from the first frame, which is what makes
    /// the peer's phone ring as a video call. `generation` is the one the window drew the call
    /// under when its button was pressed.
    ///
    /// `cancel` is read once more right before anything is sent: a call hung up while its devices
    /// were still being checked fails with [`Cancelled`] and never rings the peer. Once the offer
    /// is out the library owns it, so a later hang-up goes through [`Call::hangup`] instead.
    #[allow(clippy::too_many_arguments)]
    pub async fn place(
        client: &Arc<Client>,
        generation: u64,
        chat: String,
        video: bool,
        microphone: Option<String>,
        speaker: Option<String>,
        camera: Option<String>,
        cancel: &AtomicBool,
    ) -> Result<(Self, Option<async_channel::Receiver<VideoTick>>)> {
        let peer: Jid = chat
            .parse()
            .map_err(|error| anyhow!("not a WhatsApp JID: {error}"))?;
        if let Some(missing) =
            missing_audio_device_off_thread(microphone.clone(), speaker.clone()).await
        {
            return Err(anyhow!("no {missing} is available; a call needs one"));
        }
        // Video first: a camera or encoder that will not start refuses the call before any stream
        // exists and before the peer is rung, because a video call must not silently become voice.
        let mut pipe = None;
        if video {
            match VideoPipeline::start(camera.clone(), "").await {
                Ok((pipeline, frames)) => pipe = Some((pipeline, frames)),
                Err(error) => log::warn!("[CALL] video pipeline could not start: {error}"),
            }
        }
        require_video(video, pipe.is_some())?;
        if cancel.load(Ordering::Relaxed) {
            // The camera, if one started, is released as the pipeline drops.
            return Err(Cancelled.into());
        }
        let far_end = crate::call_audio::FarEnd::new();
        let (mic, mic_rx) = AudioInput::spawn(microphone.clone(), &far_end);
        let (output, output_tx) = AudioOutput::spawn(speaker.clone(), &far_end);

        let voip = client.voip();
        let builder = voip.call(&peer).audio(mic_rx, output_tx);
        let placed = match &pipe {
            Some((pipeline, _)) => {
                builder
                    .video(pipeline.source(), pipeline.sink())
                    .start()
                    .await
            }
            None => builder.start().await,
        };
        let handle = match placed {
            Ok(handle) => Arc::new(handle),
            Err(error) => return Err(anyhow!("WhatsApp refused the call: {error}")),
        };
        // The destination is deliberately not logged: a call's chat id is the peer's phone
        // number, and this log ships.
        log::info!(
            "[CALL] outgoing created call_id={} video={}",
            handle.call_id(),
            pipe.is_some()
        );

        let (video_pipe, frames) = match pipe {
            Some((pipeline, frames)) => (Some(pipeline), Some(frames)),
            None => (None, None),
        };
        Ok((
            Self {
                generation,
                call_id: handle.call_id().to_owned(),
                chat,
                direction: CallDirection::Outgoing,
                video: video_pipe.is_some(),
                phase: CallPhase::Dialing,
                started: None,
                outcome: None,
                peer_audio: None,
                incoming: None,
                handle: Some(handle),
                media_ready: false,
                mic: Some(mic),
                speaker: Some(output),
                camera_wanted: video_pipe
                    .as_ref()
                    .is_some_and(VideoPipeline::camera_running),
                peer_video_request: None,
                began: crate::util::now(),
                video_pipe,
                remote_video: false,
                microphone,
                speaker_device: speaker,
                camera,
                lost_devices: Vec::new(),
                group: None,
                join_task: None,
            },
            frames,
        ))
    }

    /// Records an incoming offer so the UI can ask the user. No media exists until they answer,
    /// and answering opens the system's default devices unless others are picked meanwhile.
    pub fn ringing(chat: String, incoming: Box<IncomingCall>, video: bool) -> Self {
        let call_id = incoming.action.call_id().to_owned();
        let group = GroupRoster::of_offer(&incoming);
        log::info!(
            "[CALL] incoming offer call_id={call_id} video={video} group={} participants={}",
            group.is_some(),
            group.as_ref().map_or(0, |roster| roster.participants.len())
        );
        Self {
            generation: next_generation(),
            call_id,
            chat,
            direction: CallDirection::Incoming,
            video,
            phase: CallPhase::Incoming,
            started: None,
            outcome: None,
            peer_audio: None,
            incoming: Some(incoming),
            handle: None,
            media_ready: false,
            mic: None,
            speaker: None,
            video_pipe: None,
            remote_video: false,
            microphone: None,
            speaker_device: None,
            camera: None,
            lost_devices: Vec::new(),
            camera_wanted: false,
            peer_video_request: None,
            began: crate::util::now(),
            group,
            join_task: None,
        }
    }

    /// Joins a call link: opens the devices as placing a call does, then hands the join to a task
    /// of its own, since the call service may keep us in its waiting room for as long as it likes
    /// and the worker must not wait with it. The task's answer arrives on `done`, and
    /// [`Call::link_joined`] applies it. Until then the call reads as connecting.
    #[allow(clippy::too_many_arguments)]
    pub async fn join_link(
        client: &Arc<Client>,
        generation: u64,
        token: String,
        video: bool,
        camera: Option<String>,
        cancel: &AtomicBool,
        done: async_channel::Sender<std::result::Result<CallHandle, String>>,
    ) -> Result<(Self, Option<async_channel::Receiver<VideoTick>>)> {
        if let Some(missing) = missing_audio_device_off_thread(None, None).await {
            return Err(anyhow!("no {missing} is available; a call needs one"));
        }
        let mut pipe = None;
        if video {
            match VideoPipeline::start(camera.clone(), "").await {
                Ok((pipeline, frames)) => pipe = Some((pipeline, frames)),
                Err(error) => log::warn!("[CALL] video pipeline could not start: {error}"),
            }
        }
        // A video link's call takes only a video call's media, so one with no video path cannot
        // join it at all.
        require_video(video, pipe.is_some())?;
        if cancel.load(Ordering::Relaxed) {
            return Err(Cancelled.into());
        }
        let far_end = crate::call_audio::FarEnd::new();
        let (mic, mic_rx) = AudioInput::spawn(None, &far_end);
        let (output, output_tx) = AudioOutput::spawn(None, &far_end);
        let endpoints = pipe
            .as_ref()
            .map(|(pipeline, _)| (pipeline.source(), pipeline.sink()));
        let client = Arc::clone(client);
        let task = tokio::spawn(async move {
            let media = if video {
                whatsapp_rust::types::group_call::CallLinkMedia::Video
            } else {
                whatsapp_rust::types::group_call::CallLinkMedia::Audio
            };
            let voip = client.voip();
            let builder = voip.call_link(&token, media).audio(mic_rx, output_tx);
            let joined = match endpoints {
                Some((source, sink)) => builder.video(source, sink).start().await,
                None => builder.start().await,
            };
            let _ = done.send(joined.map_err(|error| error.to_string())).await;
        });
        log::info!("[CALL] joining a call link video={video}");
        let (video_pipe, frames) = match pipe {
            Some((pipeline, frames)) => (Some(pipeline), Some(frames)),
            None => (None, None),
        };
        Ok((
            Self {
                generation,
                call_id: String::new(),
                chat: String::new(),
                direction: CallDirection::Outgoing,
                video: video_pipe.is_some(),
                phase: CallPhase::Connecting,
                started: None,
                outcome: None,
                peer_audio: None,
                incoming: None,
                handle: None,
                media_ready: false,
                mic: Some(mic),
                speaker: Some(output),
                camera_wanted: video_pipe
                    .as_ref()
                    .is_some_and(VideoPipeline::camera_running),
                peer_video_request: None,
                began: crate::util::now(),
                video_pipe,
                remote_video: false,
                microphone: None,
                speaker_device: None,
                camera,
                lost_devices: Vec::new(),
                group: Some(GroupRoster {
                    link: true,
                    ..GroupRoster::default()
                }),
                join_task: Some(task.abort_handle()),
            },
            frames,
        ))
    }

    /// Starts a call in a group, as WhatsApp's own clients do when the group's call button is
    /// pressed: bound to the group, ringing its current members (`group_call_by_id`). `video`
    /// starts it as a video call. The devices open as placing a 1:1 call does, then the offer goes
    /// out on a task of its own, since the library waits for the call service's answer and the
    /// relay before it hands the call back. That answer arrives on `done`, and
    /// [`Call::group_started`] applies it. Until then the call reads as calling.
    #[allow(clippy::too_many_arguments)]
    pub async fn place_group(
        client: &Arc<Client>,
        generation: u64,
        chat: String,
        video: bool,
        camera: Option<String>,
        cancel: &AtomicBool,
        done: async_channel::Sender<std::result::Result<CallHandle, GroupRefusal>>,
    ) -> Result<(Self, Option<async_channel::Receiver<VideoTick>>)> {
        let group: Jid = chat
            .parse()
            .map_err(|error| anyhow!("not a WhatsApp JID: {error}"))?;
        if let Some(missing) = missing_audio_device_off_thread(None, None).await {
            return Err(anyhow!("no {missing} is available; a call needs one"));
        }
        let mut pipe = None;
        if video {
            match VideoPipeline::start(camera.clone(), "").await {
                Ok((pipeline, frames)) => pipe = Some((pipeline, frames)),
                Err(error) => log::warn!("[CALL] video pipeline could not start: {error}"),
            }
        }
        require_video(video, pipe.is_some())?;
        if cancel.load(Ordering::Relaxed) {
            return Err(Cancelled.into());
        }
        let far_end = crate::call_audio::FarEnd::new();
        let (mic, mic_rx) = AudioInput::spawn(None, &far_end);
        let (output, output_tx) = AudioOutput::spawn(None, &far_end);
        let endpoints = pipe
            .as_ref()
            .map(|(pipeline, _)| (pipeline.source(), pipeline.sink()));
        let own = [client.lid(), client.pn()]
            .into_iter()
            .flatten()
            .map(|jid| jid.to_non_ad())
            .collect();
        let client = Arc::clone(client);
        let task = tokio::spawn(async move {
            // Whether WhatsApp lets this account start group calls from a linked device. The
            // button is offered either way: the call service has the last word.
            let gate = client
                .ab_prop_enabled(whatsapp_rust::wacore::iq::abprops::web::ENABLE_WEB_GROUP_CALLING)
                .await;
            log::info!(
                "[CALL] starting a group call video={video} group calling enabled={}",
                gate.map_or("unknown".to_owned(), |gate| gate.to_string())
            );
            let voip = client.voip();
            let builder = voip.group_call_by_id(&group).audio(mic_rx, output_tx);
            let started = match endpoints {
                Some((source, sink)) => builder.video(source, sink).start().await,
                None => builder.start().await,
            };
            let started = started.map_err(|error| {
                let refusal = GroupRefusal::of(&error, gate);
                log::warn!(
                    "[CALL] WhatsApp refused the group call: {} not_enabled={} (whatsapp-rust does not report the call service's error code)",
                    refusal.detail,
                    refusal.not_enabled
                );
                refusal
            });
            let _ = done.send(started).await;
        });
        let (video_pipe, frames) = match pipe {
            Some((pipeline, frames)) => (Some(pipeline), Some(frames)),
            None => (None, None),
        };
        Ok((
            Self {
                generation,
                call_id: String::new(),
                chat,
                direction: CallDirection::Outgoing,
                video: video_pipe.is_some(),
                phase: CallPhase::Dialing,
                started: None,
                outcome: None,
                peer_audio: None,
                incoming: None,
                handle: None,
                media_ready: false,
                mic: Some(mic),
                speaker: Some(output),
                camera_wanted: video_pipe
                    .as_ref()
                    .is_some_and(VideoPipeline::camera_running),
                peer_video_request: None,
                began: crate::util::now(),
                video_pipe,
                remote_video: false,
                microphone: None,
                speaker_device: None,
                camera,
                lost_devices: Vec::new(),
                group: Some(GroupRoster {
                    own,
                    ..GroupRoster::default()
                }),
                join_task: Some(task.abort_handle()),
            },
            frames,
        ))
    }

    /// Applies the call service's answer to a group call we started: the call is out, with the
    /// roster it rang, or it was refused.
    pub fn group_started(
        &mut self,
        started: std::result::Result<CallHandle, GroupRefusal>,
    ) -> Option<CallUpdate> {
        self.join_task = None;
        if !self.phase.is_live() {
            return None;
        }
        match started {
            Ok(handle) => {
                self.call_id = handle.call_id().to_owned();
                if let Some(state) = handle.group_state()
                    && let Some(update) = state.snapshot()
                    && let Some(roster) = self.group.as_mut()
                {
                    roster.apply(update);
                }
                self.handle = Some(Arc::new(handle));
                log::info!(
                    "[CALL] group call started call_id={} listed={}",
                    self.call_id,
                    self.group
                        .as_ref()
                        .map_or(0, |roster| roster.participants.len())
                );
                self.someone_answered().or_else(|| Some(self.update()))
            }
            Err(refusal) => self.fail(format!("the group call was refused: {}", refusal.detail)),
        }
    }

    /// Moves a group call we started on once anyone but us is on it: their joining is the answer,
    /// as the peer's `<accept>` is on a 1:1 call.
    fn someone_answered(&mut self) -> Option<CallUpdate> {
        let roster = self.group.as_ref()?;
        if self.direction != CallDirection::Outgoing
            || roster.link
            || !matches!(self.phase, CallPhase::Dialing | CallPhase::Ringing)
            || !roster.others_joined()
        {
            return None;
        }
        log::info!(
            "[CALL] someone joined the group call call_id={}",
            self.call_id
        );
        self.phase = CallPhase::Connecting;
        self.became_active().or_else(|| Some(self.update()))
    }

    /// Applies the answer of a call link's join: the call is in and its media coming up, or it
    /// could not get in.
    pub fn link_joined(
        &mut self,
        joined: std::result::Result<CallHandle, String>,
    ) -> Option<CallUpdate> {
        self.join_task = None;
        if !self.phase.is_live() {
            return None;
        }
        match joined {
            Ok(handle) => {
                self.call_id = handle.call_id().to_owned();
                // The roster the call service admitted us with, until its next update.
                if let Some(state) = handle.group_state()
                    && let Some(update) = state.snapshot()
                    && let Some(roster) = self.group.as_mut()
                {
                    roster.apply(update);
                }
                self.handle = Some(Arc::new(handle));
                log::info!("[CALL] joined a call link call_id={}", self.call_id);
                self.phase = CallPhase::Accepted;
                self.became_active().or_else(|| Some(self.update()))
            }
            Err(error) => self.fail(format!("the call link could not be joined: {error}")),
        }
    }

    /// A call for a test: a snapshot with no signaling and no engine behind it, so a test can put
    /// one on the worker and check what privacy recovery does with its update.
    #[cfg(test)]
    pub(crate) fn test_snapshot(chat: &str) -> Self {
        Self {
            generation: next_generation(),
            call_id: "test-call".to_owned(),
            chat: chat.to_owned(),
            direction: CallDirection::Incoming,
            video: false,
            phase: CallPhase::Incoming,
            started: None,
            outcome: None,
            peer_audio: None,
            incoming: None,
            handle: None,
            media_ready: false,
            mic: None,
            speaker: None,
            video_pipe: None,
            remote_video: false,
            microphone: None,
            speaker_device: None,
            camera: None,
            lost_devices: Vec::new(),
            camera_wanted: false,
            peer_video_request: None,
            began: crate::util::now(),
            group: None,
            join_task: None,
        }
    }

    /// The WhatsApp call id, which is how signaling for this call is told from any other's.
    pub fn call_id(&self) -> &str {
        &self.call_id
    }

    /// The chat this call belongs to.
    pub fn chat(&self) -> &str {
        &self.chat
    }

    pub fn phase(&self) -> CallPhase {
        self.phase
    }

    pub fn is_video(&self) -> bool {
        self.video
    }

    /// The live handle, once the call has one.
    pub fn handle(&self) -> Option<Arc<CallHandle>> {
        self.handle.clone()
    }

    /// The snapshot the UI renders.
    pub fn update(&self) -> CallUpdate {
        CallUpdate {
            generation: self.generation,
            chat: self.chat.clone(),
            direction: self.direction,
            video: self.video,
            phase: self.phase,
            started: self.started,
            muted: self.handle.as_ref().is_some_and(|handle| handle.is_muted()),
            camera_on: self
                .video_pipe
                .as_ref()
                .is_some_and(VideoPipeline::camera_running),
            remote_video: self.remote_video,
            video_requested: self.peer_video_request.is_some(),
            outcome: self.outcome,
            peer_audio: self.peer_audio,
            lost_devices: self.lost_devices.clone(),
            microphone: self.microphone.clone(),
            speaker: self.speaker_device.clone(),
            camera: self.camera.clone(),
            group: self.group.as_ref().map(GroupRoster::view),
        }
    }

    /// Whether this is a group call: one rung with a group's roster, which more than one other
    /// person can join.
    pub fn is_group(&self) -> bool {
        self.group.is_some()
    }

    /// When the call was placed or began to ring, Unix seconds.
    pub fn began(&self) -> i64 {
        self.began
    }

    /// Whether the call carried video at any point.
    pub fn carried_video(&self) -> bool {
        self.video
    }

    /// Everyone the call service reported as on a group call, over every roster it sent, each
    /// person once and under the id their tile is drawn under: one who joined and left before the
    /// end still counts as having been on the call, and one the service only asked to it does not.
    /// `None` for a one-to-one call. Our own entry is among them when the service lists it, which
    /// it does on a call we started; the caller knows which id is ours and leaves it out.
    pub fn group_people(&self) -> Option<Vec<String>> {
        self.group.as_ref().map(|roster| {
            roster
                .seen
                .iter()
                .map(participant_id)
                .collect::<Vec<String>>()
        })
    }

    /// The id that tells one call from the next, for a caller that keeps the last snapshot.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Says which picked devices are not the ones in use, for the call screen to word.
    pub fn set_lost_devices(&mut self, lost: Vec<LostDevice>) {
        self.lost_devices = lost;
    }

    /// Picks up streams that had to reopen on the system default, and forgets the selections they
    /// were bound to. A Bluetooth headset that switches off mid-call can reach here before device
    /// discovery notices it is gone; either way the call says what moved.
    pub fn take_stream_fallbacks(&mut self) -> Vec<(DeviceKind, String)> {
        let mut lost = Vec::new();
        if self
            .mic
            .as_ref()
            .is_some_and(|mic| mic.fell_back.try_recv().is_ok())
            && let Some(device) = self.microphone.take()
        {
            lost.push((DeviceKind::Microphone, device));
        }
        if self
            .speaker
            .as_ref()
            .is_some_and(|speaker| speaker.fell_back.try_recv().is_ok())
            && let Some(device) = self.speaker_device.take()
        {
            lost.push((DeviceKind::Speaker, device));
        }
        lost
    }

    /// Rebinds anything the machine no longer has: a headset switched off, a camera unplugged.
    ///
    /// Returns what had to change, so the caller can say it. Nothing is reported against an empty
    /// list, which is a discovery tool that could not run rather than an empty machine. The call
    /// itself is never ended here: a lost camera leaves the rest of the call alone, and a lost
    /// microphone or speaker moves to the system default.
    pub async fn verify_devices(&mut self, list: &DeviceList) -> Vec<(DeviceKind, String)> {
        let mut lost = Vec::new();
        if !self.phase.is_live() {
            return lost;
        }
        if let Some(device) = self.microphone.clone()
            && !list.microphones.is_empty()
            && !list.microphones.iter().any(|known| known.id == device)
        {
            self.set_microphone(None);
            lost.push((DeviceKind::Microphone, device));
        }
        if let Some(device) = self.speaker_device.clone()
            && !list.speakers.is_empty()
            && !list.speakers.iter().any(|known| known.id == device)
        {
            self.set_speaker(None);
            lost.push((DeviceKind::Speaker, device));
        }
        if let Some(device) = self.camera.clone()
            && !list.cameras.is_empty()
            && !list.cameras.iter().any(|known| known.id == device)
        {
            self.camera = None;
            lost.push((DeviceKind::Camera, device));
            if self
                .video_pipe
                .as_ref()
                .is_some_and(VideoPipeline::camera_running)
            {
                // The peer is being sent a stream with no source behind it: stop it properly
                // rather than leaving the last frame frozen on their screen.
                if let Err(error) = self.set_camera(false).await {
                    log::warn!("[CALL] the peer was not told the camera is gone: {error}");
                }
            } else {
                self.camera_wanted = false;
            }
        }
        lost
    }

    /// Whether the camera stopped on its own: the call still wants it, and no frames are coming.
    ///
    /// The peer has a video stream open at this point, so a stalled camera is stopped the same way
    /// a switched-off one is instead of being left frozen.
    pub fn camera_stalled(&self) -> bool {
        self.camera_wanted
            && self.handle.is_some()
            && self
                .video_pipe
                .as_ref()
                .is_some_and(|pipe| !pipe.camera_running())
    }

    /// The devices picked so far: what a call that has not started media yet should bind to, so a
    /// microphone chosen while the phone was ringing is the one the call actually uses.
    pub fn selections(&self) -> (Option<String>, Option<String>, Option<String>) {
        (
            self.microphone.clone(),
            self.speaker_device.clone(),
            self.camera.clone(),
        )
    }

    /// Answers a ringing call: sends the real `<accept>`, brings up media, and returns the frame
    /// channel for the UI.
    pub async fn answer(
        &mut self,
        client: &Arc<Client>,
        microphone: Option<String>,
        speaker: Option<String>,
        camera: Option<String>,
    ) -> Result<Option<async_channel::Receiver<VideoTick>>> {
        let Some(incoming) = self.incoming.clone() else {
            return Err(anyhow!("nothing is ringing"));
        };
        // Answering opens the same streams a call places, so the same checks apply: an answered call
        // without a microphone would be worse than a refused one.
        if let Some(missing) =
            missing_audio_device_off_thread(microphone.clone(), speaker.clone()).await
        {
            return Err(anyhow!("no {missing} is available; a call needs one"));
        }
        // The same rule as placing a call: a video offer that cannot start video is not accepted as
        // something else behind the user's back. Nothing has been sent yet, so the call stays
        // ringing and the user can still decline it.
        let mut pipe = None;
        if self.video {
            match VideoPipeline::start(camera.clone(), &self.call_id).await {
                Ok((pipeline, frames)) => pipe = Some((pipeline, frames)),
                Err(error) => log::warn!("[CALL] video pipeline could not start: {error}"),
            }
        }
        require_video(self.video, pipe.is_some())?;
        let far_end = crate::call_audio::FarEnd::new();
        let (mic, mic_rx) = AudioInput::spawn(microphone.clone(), &far_end);
        let (output, output_tx) = AudioOutput::spawn(speaker.clone(), &far_end);

        let voip = client.voip();
        // From here the library owns the answer: if it fails, its own guard ends the call with the
        // peer, so there is no offer left for [`Call::abandon`] to refuse.
        self.incoming = None;
        let builder = voip.accept(&incoming).audio(mic_rx, output_tx);
        let accepted = match &pipe {
            Some((pipeline, _)) => {
                builder
                    .video(pipeline.source(), pipeline.sink())
                    .start()
                    .await
            }
            None => builder.start().await,
        };
        let handle = match accepted {
            Ok(handle) => Arc::new(handle),
            Err(error) => return Err(anyhow!("WhatsApp refused the answer: {error}")),
        };
        log::info!("[CALL] accepted call_id={}", handle.call_id());
        self.call_id = handle.call_id().to_owned();
        self.handle = Some(handle);
        self.mic = Some(mic);
        self.speaker = Some(output);
        self.microphone = microphone;
        self.speaker_device = speaker;
        self.camera = camera;
        self.video &= pipe.is_some();
        self.camera_wanted = pipe
            .as_ref()
            .is_some_and(|(pipeline, _)| pipeline.camera_running());
        // The user picked the phone up. The media plane is still coming up, so this is `Accepted`
        // rather than `Connecting`, which is what the peer's own `<accept>` means on an outgoing
        // call: neither phase claims the call is live yet.
        self.phase = CallPhase::Accepted;
        log::info!("[CALL] accepted locally call_id={}", self.call_id);
        self.became_active();
        let frames = match pipe {
            Some((pipeline, frames)) => {
                self.video_pipe = Some(pipeline);
                Some(frames)
            }
            None => None,
        };
        Ok(frames)
    }

    /// Ends a call this side could not answer: the offer is refused when it is still ours to
    /// refuse, so the caller is not left ringing, and the call reads as one that never came up.
    pub async fn abandon(&mut self, client: &Arc<Client>) -> CallUpdate {
        if let Some(incoming) = self.incoming.take()
            && let Err(error) = client.voip().reject(&incoming).await
        {
            log::warn!("[CALL] the offer could not be refused: {error}");
        }
        self.cleanup();
        self.phase = CallPhase::Failed;
        self.outcome = Some(CallOutcome::Failed);
        self.update()
    }

    /// Declines a ringing call with the real `<reject>`.
    pub async fn decline(&mut self, client: &Arc<Client>) -> Result<()> {
        let Some(incoming) = self.incoming.clone() else {
            return Err(anyhow!("nothing is ringing"));
        };
        log::info!("[CALL] rejecting call_id={}", self.call_id);
        client
            .voip()
            .reject(&incoming)
            .await
            .map_err(|error| anyhow!("WhatsApp refused the rejection: {error}"))?;
        self.incoming = None;
        self.phase = CallPhase::Ended;
        self.outcome = Some(CallOutcome::Declined);
        Ok(())
    }

    /// Ends the call: the real `<terminate>` (or `<reject>` while ringing), then every local
    /// resource released. Safe to call once the call is already over.
    pub async fn hangup(&mut self, client: Option<&Arc<Client>>) -> CallUpdate {
        if let Some(handle) = self.handle.clone() {
            log::info!("[CALL] ending call_id={}", handle.call_id());
            let outcome = handle.terminate().await;
            log::info!(
                "[CALL] terminate outcome {outcome:?} peer_notified={}",
                outcome.peer_notified()
            );
        } else if let Some((client, incoming)) = client.zip(self.incoming.clone()) {
            log::info!("[CALL] declining ringing call_id={}", self.call_id);
            let _ = client.voip().reject(&incoming).await;
        }
        // Only a call that was really up counts as answered. `is_connected` also covers the window
        // between the peer's answer and the media plane coming up, and a hangup in that window would
        // be recorded as an answered call of zero seconds even though the length only starts once
        // the call is Active.
        let connected = self.started.is_some();
        let was_live = self.phase.is_live();
        self.cleanup();
        if was_live {
            self.phase = CallPhase::Ended;
            // We ended it, so this is not the peer's answer to record. What it was before the
            // hangup is: a call that was up was answered, one still ringing never was.
            self.outcome = Some(if connected {
                CallOutcome::Answered
            } else if self.direction == CallDirection::Incoming {
                CallOutcome::Missed
            } else {
                CallOutcome::NoAnswer
            });
        }
        self.update()
    }

    /// Releases every local resource. Safe to call twice.
    pub fn cleanup(&mut self) {
        if let Some(task) = self.join_task.take() {
            // A call link still being joined is let go: the library's own guard ends what it
            // had begun with the call service.
            task.abort();
        }
        if let Some(mut pipe) = self.video_pipe.take() {
            pipe.shutdown();
        }
        if let Some(mut mic) = self.mic.take() {
            mic.stop();
        }
        if let Some(mut speaker) = self.speaker.take() {
            speaker.stop();
        }
        self.handle = None;
        self.incoming = None;
        log::info!("[CALL] cleanup complete call_id={}", self.call_id);
    }

    /// Applies one signaling stanza, when it belongs to this call.
    pub fn signaling(&mut self, action: &CallAction) -> Option<CallUpdate> {
        if action.call_id() != self.call_id {
            return None;
        }
        // A call that is already over keeps the answer it recorded. A peer that rejects and then
        // sends the terminate that follows it would otherwise have the rejection read as a call
        // nobody answered, and the log would name the wrong reason for a call that really did end.
        if matches!(self.phase, CallPhase::Ended | CallPhase::Failed) {
            log::debug!(
                "[CALL] a stanza arrived for a call that is already over call_id={} phase={:?}",
                self.call_id,
                self.phase
            );
            return None;
        }
        if let Some(roster) = self.group.as_mut() {
            match action {
                // A newer roster: someone joined, left, or was asked in.
                CallAction::GroupUpdate { update } => {
                    if !roster.apply(update) {
                        return None;
                    }
                    return self.someone_answered().or_else(|| Some(self.update()));
                }
                // One invited member's answer speaks for that member alone; the call goes on for
                // everyone else, and the roster says who is on it. The library leaves the call
                // registered for the same reason.
                CallAction::Reject { .. }
                | CallAction::PreAccept { .. }
                | CallAction::Accept { .. } => {
                    return None;
                }
                _ => {}
            }
        }
        match action {
            CallAction::PreAccept { .. } => {
                if self.phase == CallPhase::Dialing {
                    log::info!("[CALL] remote ringing call_id={}", self.call_id);
                    self.phase = CallPhase::Ringing;
                    return Some(self.update());
                }
                None
            }
            CallAction::Accept { .. } => {
                if matches!(self.phase, CallPhase::Dialing | CallPhase::Ringing) {
                    log::info!("[CALL] remote accepted call_id={}", self.call_id);
                    self.phase = CallPhase::Connecting;
                    // A relay that was allocated while the peer's phone was ringing is a live media
                    // path waiting for them: the answer is what makes the call active, not the
                    // allocate, so both halves are checked here and in `media`.
                    return self.became_active().or_else(|| Some(self.update()));
                }
                None
            }
            CallAction::Reject { reason, .. } => {
                log::info!(
                    "[CALL] remote rejected call_id={} reason={reason:?}",
                    self.call_id
                );
                self.phase = CallPhase::Failed;
                self.outcome = Some(rejection_outcome(reason.as_deref()));
                self.cleanup();
                Some(self.update())
            }
            CallAction::Terminate { reason, .. } => {
                log::info!(
                    "[CALL] peer terminated call_id={} reason={reason:?}",
                    self.call_id
                );
                // Only a call that really reached Active was answered. `is_connected` also covers
                // the window between the peer's answer and the media plane coming up, and a
                // terminate there would be recorded as an answered call of zero seconds.
                let connected = self.started.is_some();
                self.phase = if connected {
                    CallPhase::Ended
                } else {
                    CallPhase::Failed
                };
                self.outcome = Some(termination_outcome(reason.as_deref(), connected));
                self.cleanup();
                Some(self.update())
            }
            _ => None,
        }
    }

    /// Applies one media-plane event.
    pub fn media(&mut self, event: &CallEvent) -> Option<CallUpdate> {
        match event {
            // The committed roster of a group call: who joined, who left.
            CallEvent::GroupUpdated(update) => {
                let changed = self.group.as_mut()?.apply(update);
                if changed {
                    log::info!(
                        "[CALL] group roster call_id={} transaction={} joined={} listed={}",
                        self.call_id,
                        update.transaction_id,
                        update
                            .participants
                            .iter()
                            .filter(|person| person.is_connected())
                            .count(),
                        update.participants.len()
                    );
                }
                if !changed {
                    return None;
                }
                self.someone_answered().or_else(|| Some(self.update()))
            }
            CallEvent::RelayAllocated => {
                self.media_ready = true;
                self.became_active()
            }
            CallEvent::RelayAllocateFailed(code) => {
                self.fail(format!("The relay refused the call ({code})"))
            }
            CallEvent::RelayAllocateTimedOut => self.fail("The relay did not answer".to_owned()),
            CallEvent::RelayReconnectTimedOut => {
                self.fail("The relay connection dropped".to_owned())
            }
            CallEvent::MediaSetupFailed(reason) => self.fail(reason.clone()),
            CallEvent::Closed(reason) => {
                log::info!(
                    "[CALL] media closed call_id={} reason={reason:?}",
                    self.call_id
                );
                self.ended_by(CallOutcome::ConnectionLost)
            }
            // The audio-health alarms, which are the only place a call that is connected but
            // carrying no audio says so. Each fires on its own schedule and repeats while the
            // condition holds, so only a change is published: the counters in the log are the
            // diagnosis, the screen only has room for the reason.
            CallEvent::AudioSilent {
                dominant_reason,
                rtp_received,
                frames_produced,
                silent_for_ms,
            } => {
                log::warn!(
                    "[CALL] the peer's audio is not becoming sound: reason={dominant_reason:?} silent_ms={silent_for_ms} rtp_received={rtp_received} frames_produced={frames_produced}"
                );
                let reason = silence_reason(*dominant_reason);
                if self.peer_audio == Some(PeerAudio::Silent(reason)) {
                    return None;
                }
                self.peer_audio = Some(PeerAudio::Silent(reason));
                Some(self.update())
            }
            CallEvent::AudioReceptionStalled { silent_for_ms } => {
                log::warn!("[CALL] no audio is arriving from the peer: silent_ms={silent_for_ms}");
                if self.peer_audio == Some(PeerAudio::Stalled) {
                    return None;
                }
                self.peer_audio = Some(PeerAudio::Stalled);
                Some(self.update())
            }
            CallEvent::AudioCodecSwitched {
                from,
                to,
                source,
                packets_observed,
            } => {
                log::info!(
                    "[CALL] audio codec switched {from:?} -> {to:?} source={source:?} packets={packets_observed}"
                );
                // The decode path moved, so whatever silence was reported is over.
                if self.peer_audio.take().is_some() {
                    return Some(self.update());
                }
                None
            }
            CallEvent::AudioFormatMismatch {
                expected_rate,
                received_rates,
            } => {
                log::warn!(
                    "[CALL] the peer offered audio rates {received_rates:?} against ours of {expected_rate}"
                );
                None
            }
            CallEvent::AudioCodecSourceIsFixed {
                sending,
                peer_expects,
                source,
            } => {
                log::warn!(
                    "[CALL] this call sends {sending:?} while the peer expects {peer_expects:?} source={source:?}"
                );
                None
            }
            // The peer's own video state, which is the only thing that says their picture stopped.
            // Without this the flag only ever went true, so a peer who turned their camera off was
            // still reported as sending video and the decoder's last frame stayed on screen as if
            // it were live.
            // Outbound video cannot go on the wire until a keyframe does.
            CallEvent::VideoKeyframeNeeded => {
                if let Some(pipe) = &self.video_pipe {
                    pipe.force_keyframe();
                }
                None
            }
            CallEvent::PeerVideoStateChanged {
                source,
                state,
                upgrade_token,
                ..
            } => {
                if let Some(roster) = self.group.as_mut() {
                    // One member's camera: their tile goes back to their picture when it stops.
                    // A group's call never turns into a video call, so a request is not asked.
                    if state.is_inactive_for_call_mode() && roster.video_stopped(source) {
                        return Some(self.update());
                    }
                    return None;
                }
                if state.is_upgrade_request() && !self.video {
                    // The peer turned their camera on in a voice call: the screen asks, and the
                    // camera button answers. A request resolved by signaling has no token.
                    let Some(token) = upgrade_token else {
                        return None;
                    };
                    log::info!("[CALL] the peer asked for video call_id={}", self.call_id);
                    self.peer_video_request = Some(*token);
                    return Some(self.update());
                }
                if state.is_inactive_for_call_mode() {
                    let withdrawn = self.peer_video_request.take().is_some();
                    if self.remote_video_stopped() || withdrawn {
                        return Some(self.update());
                    }
                }
                None
            }
            // The peer's reports on what it received, which is what the camera's rate follows.
            CallEvent::RtcpReceived {
                reports_audio,
                reports_video,
                report_blocks,
                feedback,
                ..
            } => {
                let pipe = self.video_pipe.as_mut()?;
                for item in feedback {
                    pipe.feedback.note(item.packet_type, item.fmt, &item.fci);
                    // A PLI or a FIR: a picture the peer could not rebuild.
                    if item.packet_type == 206 && matches!(item.fmt, 1 | 4) {
                        pipe.rate.on_picture_lost(Instant::now());
                    }
                }
                // The peer's native video report covers our video stream alone.
                if *reports_video
                    && !*reports_audio
                    && let Some(block) = report_blocks.first()
                    && !block.profile_extension.is_empty()
                {
                    pipe.report_extension.clone_from(&block.profile_extension);
                }
                if *reports_video
                    && !*reports_audio
                    && let Some(block) = report_blocks.first()
                {
                    pipe.peer_jitter = Some(block.jitter);
                }
                // Every block answering one of our sender reports measures the same path, audio's
                // as well as video's; the shortest is the one least held up on the way.
                let now = crate::video_rate::ntp_middle(std::time::SystemTime::now());
                let round_trip = report_blocks
                    .iter()
                    .filter_map(|block| {
                        crate::video_rate::round_trip(
                            block.last_sender_report,
                            block.delay_since_last_sender_report,
                            now,
                        )
                    })
                    .min();
                if let Some(round_trip) = round_trip
                    && let Some(change) = pipe.rate.on_round_trip(round_trip, Instant::now())
                {
                    log_rate_change(&self.call_id, change);
                }
                let blocks: Vec<(u32, u8)> = report_blocks
                    .iter()
                    .map(|block| (block.ssrc, block.fraction_lost))
                    .collect();
                let estimates: Vec<u32> = feedback
                    .iter()
                    .filter(|item| item.packet_type == RTCP_PAYLOAD_SPECIFIC && item.fmt == 15)
                    .filter_map(|item| crate::video_rate::remb_bitrate(&item.fci))
                    .collect();
                if let Some(change) =
                    pipe.on_rtcp(*reports_audio, *reports_video, &blocks, &estimates)
                {
                    log_rate_change(&self.call_id, change);
                }
                // After the estimates, so that a path falling short is capped at what it carries
                // even when the same packet's REMB says more.
                if *reports_video
                    && !*reports_audio
                    && let Some(block) = report_blocks.first()
                    && let Some(receiving) =
                        crate::video_rate::peer_receiving(&block.profile_extension)
                    && let Some(change) = pipe.on_peer_receiving(receiving)
                {
                    log_rate_change(&self.call_id, change);
                }
                None
            }
            // The relay connection could not take all the video queued for it.
            CallEvent::OutboundMediaDropped {
                video_access_units, ..
            } if *video_access_units > 0 => {
                if let Some(change) = self.video_pipe.as_mut()?.on_send_drops() {
                    log_rate_change(&self.call_id, change);
                }
                None
            }
            _ => None,
        }
    }

    /// Frames the camera thread could not hand to the engine since the last check count as the
    /// path carrying less than is sent, like the relay connection's own drops. Called from the call
    /// heartbeat.
    ///
    /// Only once the call is up: while the peer's phone rings nothing drains the queue yet, and
    /// what overflows then says nothing about the path.
    pub fn check_video_queue(&mut self) {
        let active = self.phase == CallPhase::Active;
        let Some(pipe) = self.video_pipe.as_mut() else {
            return;
        };
        if pipe.rate_share.take_queue_drops() > 0
            && active
            && let Some(change) = pipe.on_capture_drops()
        {
            log_rate_change(&self.call_id, change);
        }
    }

    /// Takes the share of our video's wire the engine's FEC used since the last heartbeat out of
    /// the encoder's target, so video and FEC together stay at the rate the rate control chose.
    /// Called from the call heartbeat.
    pub fn budget_video_fec(&mut self) {
        let Some(handle) = self.handle.as_ref() else {
            return;
        };
        let fec_bytes = handle.media_stats().video_fec_bytes_sent;
        if let Some(pipe) = self.video_pipe.as_mut() {
            pipe.on_fec_sent(fec_bytes);
        }
    }

    /// Reads the engine's media counters into the log, so a call that is up but not carrying audio
    /// can be diagnosed from a bug report rather than guessed at. Called from the call heartbeat.
    pub fn log_media_stats(&self) {
        self.log_media_stats_at("heartbeat");
    }

    /// The same counters, tagged with what the call was doing when they were read.
    ///
    /// The tag is the whole point of the extra call sites: a heartbeat lands at some arbitrary
    /// point in a call, while these are read around the transitions that are suspected of changing
    /// it, so two lines a second apart are a before and an after rather than two samples of a
    /// steady state.
    pub fn log_media_stats_at(&self, marker: &str) {
        let Some(handle) = self.handle.as_ref() else {
            self.log_audio_path(marker);
            return;
        };
        // Every counter the engine keeps, by its own name. A list written out here missed one
        // (forwarding_envelope_rejected) and would miss each counter a newer engine adds, such as
        // its counts of the peer's video packets; the whole snapshot carries them all, and it
        // holds counts and nothing else.
        let stats = handle.media_stats();
        log::info!(
            "[CALL] media stats call_id={} at={} {stats:?}",
            self.call_id,
            marker,
        );
        if let Some(pipe) = &self.video_pipe {
            let rate = pipe.rate();
            let feedback = pipe.feedback;
            let (round_trip, floor) = pipe.rate.round_trips();
            let counts = pipe.diag.counters.take();
            let millis = |time: Option<std::time::Duration>| {
                time.map_or_else(|| "none".to_owned(), |time| time.as_millis().to_string())
            };
            log::info!(
                "[CALL] video send call_id={} at={} bitrate_kbps={} picture={} max_fps={} \
                 peer_nacked_packets={} peer_plis={} peer_firs={} peer_estimates={} \
                 peer_estimate_kbps={} rtt_ms={} rtt_floor_ms={} peer_jitter_ms={} \
                 peer_report_extension={} encoded_frames={} keyframes={} largest_keyframe_kb={} \
                 peer_frames_decoded={} peer_frames_undecodable={} encoder_kbps={} \
                 fec_packets_total={} fec_kb_total={}",
                self.call_id,
                marker,
                rate.bitrate / 1000,
                rate.tier().name,
                rate.tier().fps,
                feedback.nacked_packets,
                feedback.picture_loss,
                feedback.full_intra,
                feedback.estimates,
                feedback
                    .last_estimate
                    .map_or_else(|| "none".to_owned(), |bps| (bps / 1000).to_string()),
                millis(round_trip),
                millis(floor),
                pipe.peer_jitter
                    .map_or_else(|| "none".to_owned(), |ticks| (ticks / 90).to_string()),
                crate::settings::hex(&pipe.report_extension),
                counts.sent_frames,
                counts.sent_keyframes,
                counts.largest_keyframe / 1024,
                counts.decoded,
                counts.undecodable,
                pipe.rate_share.encoder_bitrate(rate) / 1000,
                stats.video_fec_sent,
                stats.video_fec_bytes_sent / 1024,
            );
        }
        self.log_audio_path(marker);
    }

    /// The call's own audio path, counted where the engine cannot see it.
    ///
    /// The engine's counters above say what it decoded and handed over; these say what this side
    /// did with it. Together they are what tells a call that is carrying nothing (nothing received,
    /// nothing decoded) from one whose audio dies after the engine handed it over, and from one
    /// whose sink was reopened: a reader or writer open that keeps climbing during a call is a
    /// stream being restarted rather than rebound.
    pub fn log_audio_path(&self, marker: &str) {
        log::info!(
            "[CALL] audio path call_id={} at={} mic_opens={} speaker_opens={} speaker_stalls={} speaker_restarts={} live={}",
            self.call_id,
            marker,
            self.mic
                .as_ref()
                .map_or(0, |mic| mic.opens.load(Ordering::Relaxed)),
            self.speaker
                .as_ref()
                .map_or(0, |speaker| speaker.opens.load(Ordering::Relaxed)),
            self.speaker
                .as_ref()
                .map_or(0, |speaker| speaker.stalls.load(Ordering::Relaxed)),
            self.speaker
                .as_ref()
                .map_or(0, |speaker| speaker.restarts.load(Ordering::Relaxed)),
            self.phase.is_live(),
        );
    }

    /// The media task is gone; whatever the phase was, the call is over.
    pub fn media_ended(&mut self) -> Option<CallUpdate> {
        if matches!(self.phase, CallPhase::Ended | CallPhase::Failed) {
            return None;
        }
        log::info!("[CALL] ended call_id={}", self.call_id);
        // A call is answered once it was Active and only then; the media going away while the call
        // was still connecting is a call that never came up.
        let outcome = if self.started.is_some() {
            CallOutcome::Answered
        } else {
            CallOutcome::NoAnswer
        };
        self.ended_by(outcome)
    }

    /// The call was resolved on another of the account's devices (`elsewhere` says how), or the
    /// caller gave up (`None`).
    ///
    /// A call we were ringing for that was answered or declined elsewhere is not the same thing as
    /// an unanswered one, so the cases get their own words while all release the media.
    pub fn resolved_elsewhere(&mut self, elsewhere: Option<CallOutcome>) -> Option<CallUpdate> {
        if !self.phase.is_live() {
            return None;
        }
        let ringing = self.phase == CallPhase::Incoming;
        // Only a call that reached Active really connected here. A resolve that arrives during
        // media negotiation is not an answered-elsewhere call with a zero duration: this device
        // never had the call up.
        let live = self.started.is_some();
        log::info!(
            "[CALL] resolved elsewhere call_id={} phase={:?}",
            self.call_id,
            self.phase
        );
        self.phase = if live {
            CallPhase::Ended
        } else {
            CallPhase::Failed
        };
        self.outcome = Some(if live {
            CallOutcome::AnsweredElsewhere
        } else if let Some(outcome) = elsewhere {
            outcome
        } else if ringing {
            CallOutcome::Missed
        } else {
            CallOutcome::NoAnswer
        });
        self.cleanup();
        Some(self.update())
    }

    /// Moves to Active once both halves are in place: the peer answered and the media plane is up.
    ///
    /// Either half alone is not a call. The relay is allocated as soon as our offer is acked, so
    /// `RelayAllocated` on its own would show a connected call while the other phone is still
    /// ringing — which is exactly the state this phase machine exists to avoid.
    fn became_active(&mut self) -> Option<CallUpdate> {
        if !matches!(self.phase, CallPhase::Connecting | CallPhase::Accepted) || !self.media_ready {
            return None;
        }
        self.phase = CallPhase::Active;
        // Frames the camera could not hand over while the peer's phone rang say nothing about the
        // path, and the first heartbeat would otherwise read them as congestion and cut the rate.
        if let Some(pipe) = &self.video_pipe {
            pipe.rate_share.take_queue_drops();
        }
        // The timer starts here, when there is really a call, never at the button press.
        self.started = Some(Instant::now());
        log::info!(
            "[CALL] active call_id={} video={}",
            self.call_id,
            self.video
        );
        Some(self.update())
    }

    /// Ends the call because its media went away, telling a call that was up from one that never
    /// came up: the phase says which, and the outcome names the cause either way.
    fn ended_by(&mut self, outcome: CallOutcome) -> Option<CallUpdate> {
        // A media event that arrives after the call is already over must not reopen it or rewrite
        // the outcome it ended with. The relay's `Closed` can land after a hang-up has ended the
        // call, and recording that as `ConnectionLost` would replace the real reason with a false
        // one and publish a second end for a call the screen has already dropped.
        if matches!(self.phase, CallPhase::Ended | CallPhase::Failed) {
            return None;
        }
        self.phase = if self.started.is_some() {
            CallPhase::Ended
        } else {
            CallPhase::Failed
        };
        self.outcome = Some(outcome);
        self.cleanup();
        Some(self.update())
    }

    fn fail(&mut self, reason: String) -> Option<CallUpdate> {
        if matches!(self.phase, CallPhase::Ended | CallPhase::Failed) {
            return None;
        }
        log::warn!("[CALL] failed call_id={} reason={reason}", self.call_id);
        self.phase = CallPhase::Failed;
        self.outcome = Some(CallOutcome::Failed);
        self.cleanup();
        Some(self.update())
    }

    /// Mutes or unmutes through the engine, which is what stops outgoing audio.
    ///
    /// The engine zeroes the frames it already has while this is set, so the microphone stream
    /// stays fed and the peer never re-negotiates the transport.
    pub async fn set_muted(&mut self, muted: bool) -> Option<CallUpdate> {
        let handle = self.handle.clone()?;
        match handle.set_muted(muted).await {
            Ok(()) => {}
            Err(error) if muted => log::warn!(
                "[CALL] the peer was not told about the mute; the microphone is muted locally: {error}"
            ),
            Err(error) => log::warn!("[CALL] unmute failed, the microphone stays muted: {error}"),
        }
        let now = handle.is_muted();
        if now != muted {
            log::warn!("[CALL] mute state is {now} after asking for {muted}");
        }
        if now {
            log::info!("[CALL] microphone muted");
        } else {
            log::info!("[CALL] microphone unmuted");
        }
        Some(self.update())
    }

    /// Says a media stream the call owns has just been opened or closed.
    ///
    /// A camera is a second stream on this machine's audio graph, and the graph relinks every
    /// stream already on it while the new one settles. The sink is told, so a pause that belongs to
    /// the graph is not read as a device that died and is not recovered from by throwing the peer's
    /// audio away.
    fn media_stream_changed(&self) {
        if let Some(speaker) = &self.speaker {
            speaker.relink();
        }
    }

    /// The engine's counters and this side's audio path, for one transition of a call.
    ///
    /// Called at every point a media stream comes or goes, so a report has the numbers from before
    /// and after the change rather than one heartbeat somewhere in the middle. A call that goes
    /// quiet when the camera comes on is diagnosed from these lines: `rtp_received` and
    /// `audio_frames_decoded` climbing while `audio_sink_dropped` climbs says the peer is still
    /// being heard and this side is the one dropping it, and a flat `rtp_received` says the
    /// opposite.
    pub fn note_transition(&self, marker: &str) {
        self.log_media_stats_at(marker);
    }

    /// Turns the camera on or off through the engine's video direction.
    pub async fn set_camera(&mut self, on: bool) -> Result<CallUpdate> {
        let Some(handle) = self.handle.clone() else {
            return Err(anyhow!("the call is not up"));
        };
        self.note_transition(if on {
            "camera on requested"
        } else {
            "camera off requested"
        });
        if on {
            let Some(pipe) = self.video_pipe.as_mut() else {
                return Err(anyhow!("this call has no video"));
            };
            // The local capture comes up before the engine is asked to carry it. If the engine
            // refuses, the capture that was just started is rolled back: leaving it running would
            // be a camera holding the device and frames going to nobody, while the screen still
            // reads the call as video-off.
            pipe.resume_camera(self.camera.clone()).await?;
            let (source, sink) = (pipe.source(), pipe.sink());
            // The camera stream exists now, so the graph has relinked around it.
            if let Some(speaker) = &self.speaker {
                speaker.relink();
            }
            if let Err(error) = handle.resume_video(source, sink).await {
                if let Some(pipe) = self.video_pipe.as_mut() {
                    pipe.pause_camera();
                }
                self.camera_wanted = false;
                self.media_stream_changed();
                self.note_transition("camera on rolled back");
                return Err(anyhow!("the camera could not be enabled: {error}"));
            }
            self.camera_wanted = true;
        } else {
            // The local capture is what must not outlive the press, so it is stopped whatever the
            // engine says. A failure to tell the peer is still surfaced, but it cannot be allowed
            // to leave the camera capturing and sending: the request was to turn it off.
            let notified = handle.stop_video().await;
            if let Some(pipe) = self.video_pipe.as_mut() {
                pipe.pause_camera();
            }
            self.camera_wanted = false;
            self.media_stream_changed();
            if let Err(error) = notified {
                self.note_transition("camera off, peer not told");
                return Err(anyhow!(
                    "the camera was stopped, but the peer was not told: {error}"
                ));
            }
        }
        log::info!("[CALL] camera enabled={on}");
        self.note_transition(if on {
            "camera on settled"
        } else {
            "camera off settled"
        });
        Ok(self.update())
    }

    /// Asks the peer for video on a call that started as voice, or turns the camera back on when
    /// this is already a video call.
    ///
    /// The upgrade is the real one: `CallHandle::start_video` attaches the endpoints, enables the
    /// media plane and sends `<video state=11>`, and the peer's answer arrives as video state
    /// signaling. Returns the frame channel the UI should start draining.
    ///
    /// The camera arrives already resolved, the same way a call's startup resolves it: upgrading a
    /// voice call on a fresh install opens the machine's own camera instead of failing on a `None`
    /// that only ever meant "default device".
    pub async fn upgrade_to_video(
        &mut self,
        camera: Option<String>,
    ) -> Result<Option<async_channel::Receiver<VideoTick>>> {
        let Some(handle) = self.handle.clone() else {
            return Err(anyhow!("the call is not up"));
        };
        self.note_transition("video requested");
        if self.video_pipe.is_none() {
            let request = self.peer_video_request.take();
            let (pipe, frames) = VideoPipeline::start(camera.clone(), &self.call_id).await?;
            self.media_stream_changed();
            self.camera_wanted = pipe.camera_running();
            self.video_pipe = Some(pipe);
            self.camera = camera.clone();
            self.video = true;
            let (source, sink) = {
                let pipe = self.video_pipe.as_ref().expect("just installed");
                (pipe.source(), pipe.sink())
            };
            // A peer who asked for video is answered; otherwise the peer is asked.
            let started = match request {
                Some(token) => handle.accept_video(token, source, sink).await,
                None => handle.start_video(source, sink).await,
            };
            if let Err(error) = started {
                self.abandon_video();
                return Err(anyhow!("the peer could not be asked for video: {error}"));
            }
            log::info!("[CALL] video upgrade requested call_id={}", self.call_id);
            self.note_transition("video requested ok");
            return Ok(Some(frames));
        }
        self.set_camera(true).await?;
        Ok(None)
    }

    /// Gives back everything an upgrade that the peer refused had built.
    ///
    /// The peer was never asked, so there is no video call to be in: leaving the pipeline installed
    /// would be a call the screen draws as sending a picture that was never announced, with a camera
    /// holding the device and the frames going to nobody. The camera is stopped, the pipeline is
    /// dropped, and the selection is cleared so the next press starts from the machine rather than
    /// from a device this attempt had already taken.
    fn abandon_video(&mut self) {
        if let Some(mut pipe) = self.video_pipe.take() {
            pipe.shutdown();
        }
        self.video = false;
        self.camera = None;
        self.camera_wanted = false;
        self.media_stream_changed();
        log::warn!("[CALL] video upgrade refused call_id={}", self.call_id);
    }

    /// Rebinds the microphone. The engine's channel is untouched.
    pub fn set_microphone(&mut self, device: Option<String>) -> CallUpdate {
        self.microphone = device.clone();
        self.lost_devices.clear();
        if let Some(mic) = &self.mic {
            mic.bind(device);
        }
        log::info!("[CALL] microphone device {:?}", self.microphone);
        self.update()
    }

    /// Rebinds the speaker. The engine's channel is untouched.
    pub fn set_speaker(&mut self, device: Option<String>) -> CallUpdate {
        self.speaker_device = device.clone();
        self.lost_devices.clear();
        if let Some(speaker) = &self.speaker {
            speaker.bind(device);
        }
        log::info!("[CALL] speaker device {:?}", self.speaker_device);
        self.update()
    }

    /// Switches the camera. A running camera is restarted on the new node.
    ///
    /// The device arrives already resolved: `None` is the picker's "First camera", and a camera
    /// has no system default, so the caller reads it as the first node the machine reports — exactly
    /// as [`resolve_devices`] reads it when a call starts. Resolving it here instead would enumerate
    /// devices on whatever thread is driving the call, which is the worker's async loop.
    pub async fn set_camera_device(&mut self, device: Option<String>) -> Result<CallUpdate> {
        self.camera = device.clone();
        self.lost_devices.clear();
        // Said before the switch is attempted, because the stream is going either way: the old
        // camera is already released by the time a new one is opened, so the graph relinks whether
        // or not the new node turns out to be usable.
        if self
            .video_pipe
            .as_ref()
            .is_some_and(VideoPipeline::camera_running)
        {
            self.media_stream_changed();
        }
        if let Some(pipe) = self.video_pipe.as_mut()
            && pipe.camera_running()
        {
            pipe.resume_camera(device).await?;
        }
        log::info!("[CALL] camera device {:?}", self.camera);
        Ok(self.update())
    }

    /// Notes a group member's picture: the id of the tile it belongs on, and whether it is the
    /// first since their video began (which the call screen is told, so the tile turns to it).
    /// `None` on a one-to-one call, or for a sender the roster does not name.
    pub fn group_picture(&mut self, sender: &Jid) -> Option<(String, bool)> {
        let (id, first) = self.group.as_mut()?.picture_from(sender)?;
        if first {
            log::info!(
                "[CALL] a group member's video began call_id={}",
                self.call_id
            );
        }
        Some((id, first))
    }

    /// Whether the peer's picture is the one to show right now.
    ///
    /// The flag itself is private so it can only be set by a frame arriving and cleared by the
    /// peer's own signaling; this is how the frame path asks before handing a picture to the UI.
    pub fn peer_video_visible(&self) -> bool {
        self.remote_video
    }

    /// Notes that the peer's picture arrived, so the UI can stop saying it is waiting.
    pub fn saw_remote_video(&mut self) -> bool {
        if self.remote_video {
            return false;
        }
        self.remote_video = true;
        log::info!("[CALL] remote video received call_id={}", self.call_id);
        true
    }

    /// Notes that the peer's picture stopped, so the UI stops showing the last frame it had.
    ///
    /// `remote_video` only ever became true, and nothing turned it back: a peer who turned their
    /// camera off left the call reported as sending video and the decoder's last frame on screen as
    /// if it were live. The state that says otherwise is the peer's own `<video state=N>` signaling.
    /// Returns whether this was a change, so only a change is published.
    pub fn remote_video_stopped(&mut self) -> bool {
        if !self.remote_video {
            return false;
        }
        self.remote_video = false;
        log::info!("[CALL] remote video stopped call_id={}", self.call_id);
        true
    }
}

/// How a `<reject>`'s reason reads as an outcome. The peer refusing for a reason we do not know is
/// still a refusal.
fn rejection_outcome(reason: Option<&str>) -> CallOutcome {
    match reason {
        Some("busy") => CallOutcome::Busy,
        Some("enc") => CallOutcome::Failed,
        _ => CallOutcome::Declined,
    }
}

/// How a `<terminate>`'s reason reads as an outcome, given whether the call had been up.
///
/// The reasons that talk about the call's own progress are read against whether it had been up,
/// because a reason is what the peer says and the timer is what happened here: a call that was up
/// has an answer to record whatever arrives. Reported from a real call, a
/// `<terminate reason="timeout">` four seconds after the camera came on turned a nineteen-second
/// conversation into a log entry reading "No answer", with its duration sitting right beside it.
/// Those two facts cannot both be true, and the duration is the one this side measured. The
/// `*_elsewhere` reasons are facts about the account's other devices and stand either way.
fn termination_outcome(reason: Option<&str>, connected: bool) -> CallOutcome {
    match reason {
        // Another of this account's devices took the call over, whichever state this one was in.
        Some("accepted_elsewhere") => CallOutcome::AnsweredElsewhere,
        Some("rejected_elsewhere") => CallOutcome::DeclinedElsewhere,
        // The peer's phone giving up on a call that was ringing, for the one that never answered.
        Some("timeout") => {
            if connected {
                CallOutcome::ConnectionLost
            } else {
                CallOutcome::NoAnswer
            }
        }
        // A group call that ended under a one-to-one call's member: the call was answered only if
        // there was a call here to answer.
        Some("group_call_ended") => {
            if connected {
                CallOutcome::Answered
            } else {
                CallOutcome::NoAnswer
            }
        }
        _ if connected => CallOutcome::Answered,
        _ => CallOutcome::NoAnswer,
    }
}

/// The engine's silence reason under this app's own name, so the model never leaks the library's
/// enum into the interface.
fn silence_reason(reason: whatsapp_rust::voip_control::MediaSilenceReason) -> SilenceReason {
    use whatsapp_rust::voip_control::MediaSilenceReason as Engine;
    match reason {
        Engine::NoDecoderForNegotiatedCodec => SilenceReason::NoDecoder,
        Engine::AuthenticationFailing => SilenceReason::AuthenticationFailing,
        Engine::UnexpectedPayloadType => SilenceReason::UnexpectedPayloadType,
        Engine::CodecRejectingFrames => SilenceReason::CodecRejectingFrames,
        Engine::CodecFlapping => SilenceReason::CodecFlapping,
        _ => SilenceReason::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A machine with one of each, named the way a picker would show them.
    fn machine() -> DeviceList {
        DeviceList {
            microphones: vec![AudioDevice {
                id: "alsa_input.usb-Generic_USB_Headset-00.analog-mono".to_owned(),
                label: "USB Headset Microphone".to_owned(),
            }],
            speakers: vec![AudioDevice {
                id: "alsa_output.usb-Generic_USB_Headset-00.analog-stereo".to_owned(),
                label: "USB Headset Stereo".to_owned(),
            }],
            cameras: vec![CameraDevice {
                id: "0".to_owned(),
                label: "Laptop Camera".to_owned(),
            }],
        }
    }

    /// The pickers list what rodio reports, each device named and labelled.
    #[test]
    fn the_audio_pickers_name_every_device() {
        let listed = super::devices();
        for device in listed.microphones.iter().chain(&listed.speakers) {
            assert!(
                !device.id.is_empty(),
                "a device is named by what rodio reports"
            );
            assert!(
                !device.label.is_empty(),
                "a device is labelled for the picker"
            );
        }
        for device in &listed.cameras {
            assert!(!device.id.is_empty(), "a camera is named by its node");
            assert!(
                !device.label.is_empty(),
                "a camera is labelled for the picker"
            );
        }
    }

    /// Changing the camera, the microphone or the speaker leaves the call's audio alone.
    ///
    /// This is the regression the video direction once caused: turning the camera on took the
    /// remote audio with it. The devices below belong to the test rather than the machine, so this
    /// holds on a build host with no camera and no sound at all.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn changing_media_devices_leaves_the_calls_audio_running() {
        let samples: Vec<f32> = (0..960).map(|sample| sample as f32 / 1_920.0).collect();
        let (_devices, speaker) = crate::call_audio::fake::install(samples, 2, 48_000, (48_000, 2));
        let mut call = dialing();
        let (input, frames) = AudioInput::spawn(None, &crate::call_audio::FarEnd::new());
        let (output, _tx) = AudioOutput::spawn(None, &crate::call_audio::FarEnd::new());
        call.mic = Some(input);
        call.speaker = Some(output);

        // The microphone is delivering before anything else is touched.
        assert!(
            frames.recv().await.is_ok(),
            "the microphone is delivering before the camera is touched"
        );

        call.set_camera_device(Some("9".to_owned()))
            .await
            .expect("a camera can be selected");
        call.set_camera_device(None)
            .await
            .expect("a camera can be cleared");
        call.set_speaker(Some("Second speaker".to_owned()));
        call.set_microphone(Some("Second microphone".to_owned()));

        // The engine's microphone channel is the same one it was, and it is still carrying frames.
        tokio::time::timeout(std::time::Duration::from_secs(5), frames.recv())
            .await
            .expect("the microphone channel survived the changes")
            .expect("the frames are still coming");
        // Only the rebind opened a device: the camera change never touches the audio at all.
        let started = std::time::Instant::now();
        while speaker.opened_count() < 2 && started.elapsed() < std::time::Duration::from_secs(5) {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(
            speaker.opened(),
            vec![None, Some("Second speaker".to_owned())],
            "the camera changes opened no audio device"
        );
        call.cleanup();
    }

    /// A video direction with a camera that reads as running, without a device behind it.
    ///
    /// Enough to drive the paths that only ask whether a camera is capturing, which is what a
    /// machine with no camera cannot exercise otherwise.
    fn pipeline_with_a_running_camera() -> VideoPipeline {
        let (ticks, _ticks_rx) = async_channel::bounded::<VideoTick>(2);
        let (remote_tx, _remote_rx) = async_channel::bounded::<VideoFrame>(8);
        let (_frames, timed) = async_channel::bounded::<TimedVideoFrame>(4);
        // No thread behind it: its end of the channel is already dropped.
        let (_done, finished) = std::sync::mpsc::channel();
        let rate_share = Arc::new(RateShare::default());
        VideoPipeline {
            ticks,
            remote_tx,
            camera: Some(CameraCapture {
                timed,
                running: Arc::new(AtomicBool::new(true)),
                keyframe: Arc::new(AtomicBool::new(false)),
                finished,
            }),
            device: None,
            rate: RateControl::new(Arc::clone(&rate_share)),
            rate_share,
            feedback: Default::default(),
            report_extension: Vec::new(),
            peer_jitter: None,
            diag: Arc::new(crate::call_diag::CallVideo::new("test")),
        }
    }

    /// Opening or closing a camera tells the speaker the audio graph is relinking.
    ///
    /// A camera is a second stream on the same graph the call's audio runs on, and the graph
    /// relinks every stream on it while the new one settles. Without this the sink is judged stalled
    /// through that pause and recovers by throwing the peer's audio away, which is the reported
    /// "camera on and the other side went quiet" with everything else still working.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_camera_change_tells_the_speaker_the_graph_is_relinking() {
        let samples: Vec<f32> = (0..960).map(|sample| sample as f32 / 1_920.0).collect();
        let (_devices, _speaker) =
            crate::call_audio::fake::install(samples, 2, 48_000, (48_000, 2));
        let mut call = dialing();
        let (output, _frames) = AudioOutput::spawn(None, &crate::call_audio::FarEnd::new());
        call.speaker = Some(output);
        call.video = true;
        call.video_pipe = Some(pipeline_with_a_running_camera());
        assert!(
            !call.speaker.as_ref().is_some_and(AudioOutput::relink_told),
            "nothing has relinked yet"
        );
        // Whether this machine has a tenth camera is not what is under test: the stream is
        // torn down either way, so the sink is told either way.
        let _ = call.set_camera_device(Some("9".to_owned())).await;
        assert!(
            call.speaker.as_ref().is_some_and(AudioOutput::relink_told),
            "switching the camera tells the sink the graph is relinking"
        );
        call.cleanup();
    }

    /// An upgrade the peer refused leaves no half-installed video behind it.
    ///
    /// The call would otherwise draw itself as video with a camera holding the device, the frames
    /// going to nobody, and the peer never having been asked, so the next press of the camera would
    /// have to undo a state the user never asked for.
    #[test]
    fn a_refused_upgrade_gives_back_everything_it_built() {
        let mut call = dialing();
        call.video = true;
        call.camera = Some("0".to_owned());
        call.camera_wanted = true;
        call.abandon_video();
        assert!(!call.is_video(), "the call is a voice call again");
        assert!(call.camera.is_none(), "and it holds no camera");
        assert!(!call.camera_wanted, "so none is wanted");
        assert!(call.video_pipe.is_none(), "and the pipeline is gone");
    }

    /// The engine's mute gate, as the engine's own media backend applies it to the frames a call
    /// hands it.
    ///
    /// The zeroing is the engine's rather than this module's: its media backend reads the call's
    /// microphone channel and replaces every frame that is exactly the engine's 60 ms one with zeros
    /// while the call is muted, passing anything else through untouched. The engine then turns an
    /// all-zero frame into a one-byte MLOW DTX comfort-noise packet instead of a gap, so the media
    /// stream stays fed and the peer does not re-negotiate the transport.
    ///
    /// Spelled out here because that type is private to the engine, and what the test below drives
    /// into it is the real microphone path: a frame that is not the engine's own size is the one
    /// thing that would leak the microphone while the peer is told this side is muted, and it fails
    /// here.
    fn engine_mute_gate(frame: &mut [i16], muted: bool) {
        if muted && frame.len() == crate::call_audio::FRAME_SAMPLES {
            frame.fill(0);
        }
    }

    /// A mute really silences what the engine is fed, without silencing the microphone or stopping
    /// the stream, and an unmute brings the audio back on the channel the call already had.
    ///
    /// The device is a test one, so this holds on a build host with no sound card and without a
    /// peer: what is under test is the audio path a call runs, not a second implementation of it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_mute_silences_the_engine_feed_and_an_unmute_brings_it_back() {
        // A ramp, so a frame that is not silent carries a signal nobody could mistake for silence.
        let samples: Vec<f32> = (0..960).map(|sample| sample as f32 / 1_920.0).collect();
        let (_devices, _speaker) =
            crate::call_audio::fake::install(samples, 2, 48_000, (48_000, 2));
        let (input, frames) = AudioInput::spawn(None, &crate::call_audio::FarEnd::new());
        let mut heard: Vec<Vec<i16>> = Vec::new();
        let mut quiet: Vec<Vec<i16>> = Vec::new();
        let mut microphone_while_muted: Vec<i16> = Vec::new();
        for step in 0..3 {
            let muted = step == 1;
            for _ in 0..4 {
                let mut frame =
                    tokio::time::timeout(std::time::Duration::from_secs(5), frames.recv())
                        .await
                        .expect("the engine's channel stays open")
                        .expect("the microphone keeps delivering");
                assert_eq!(
                    frame.len(),
                    crate::call_audio::FRAME_SAMPLES,
                    "a frame the engine cannot size is one its mute gate would pass through"
                );
                if muted {
                    microphone_while_muted.extend_from_slice(&frame);
                }
                engine_mute_gate(&mut frame, muted);
                if muted {
                    quiet.push(frame);
                } else {
                    heard.push(frame);
                }
            }
        }
        let opens = input.opens.load(Ordering::Relaxed);
        drop(input);

        // Unmuted, the engine is fed what the microphone delivered.
        assert!(
            heard.iter().flatten().any(|sample| *sample != 0),
            "the engine hears the microphone while the call is not muted"
        );
        // Muted, everything it is fed is silent ...
        assert_eq!(quiet.len(), 4, "frames kept arriving while muted");
        assert!(
            quiet.iter().flatten().all(|sample| *sample == 0),
            "every frame the engine is fed while muted is silent"
        );
        // ... while the microphone behind the gate is still delivering, which is what makes the
        // silence the mute rather than a device that went away.
        assert!(
            microphone_while_muted.iter().any(|sample| *sample != 0),
            "the microphone is still delivering while muted"
        );
        // The reader was never restarted by any of it: one open, and the frames above are still
        // coming from it.
        assert_eq!(opens, 1, "the mute and the unmute restarted no reader");
        // And an unmute needs no restart either, which is the frame above.
        assert!(
            heard.len() == 8,
            "the microphone came back on the same channel after the unmute"
        );
    }

    /// Waits for something a pump does on its own task, so a test does not assert before the
    /// rebind it asked for has had a chance to happen. Returns as soon as the answer is yes, and
    /// leaves the last answer for the caller to assert on.
    async fn until(mut ready: impl FnMut() -> bool) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if ready() {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        ready()
    }

    /// A call that is up stays up through a mute: the engine's channel keeps carrying whole frames,
    /// a device change made while muted rebinds the streams rather than restarting the call, and a
    /// camera that cannot be turned on leaves the audio exactly as it was.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_muted_call_keeps_its_audio_and_its_phase() {
        let samples: Vec<f32> = (0..960).map(|sample| sample as f32 / 1_920.0).collect();
        let (_devices, speaker) = crate::call_audio::fake::install(samples, 2, 48_000, (48_000, 2));
        let mut call = dialing();
        let (input, frames) = AudioInput::spawn(None, &crate::call_audio::FarEnd::new());
        let (output, _tx) = AudioOutput::spawn(None, &crate::call_audio::FarEnd::new());
        call.mic = Some(input);
        call.speaker = Some(output);
        call.signaling(&accept()).expect("the peer answered");
        call.media(&CallEvent::RelayAllocated);
        assert_eq!(call.phase(), CallPhase::Active, "the call is up");

        // Muted, and everything the screen can do meanwhile is done to it.
        call.set_microphone(Some("Second microphone".to_owned()));
        call.set_speaker(Some("Second speaker".to_owned()));
        call.set_camera_device(Some("9".to_owned()))
            .await
            .expect("a camera can be selected");
        assert!(
            call.set_camera(true).await.is_err(),
            "a call with no video refuses the camera rather than ending the call"
        );

        let mut quiet: Vec<Vec<i16>> = Vec::new();
        let mut heard: Vec<Vec<i16>> = Vec::new();
        for step in 0..2 {
            let muted = step == 0;
            for _ in 0..3 {
                let mut frame =
                    tokio::time::timeout(std::time::Duration::from_secs(5), frames.recv())
                        .await
                        .expect("the engine's channel stays open through the mute")
                        .expect("the microphone keeps delivering");
                assert_eq!(frame.len(), crate::call_audio::FRAME_SAMPLES);
                engine_mute_gate(&mut frame, muted);
                if muted {
                    quiet.push(frame);
                } else {
                    heard.push(frame);
                }
            }
        }
        assert_eq!(quiet.len(), 3);
        assert!(
            quiet.iter().flatten().all(|sample| *sample == 0),
            "every frame the engine is fed while muted is silent"
        );
        assert!(
            heard.iter().flatten().any(|sample| *sample != 0),
            "unmuting brings the microphone back"
        );
        // Neither the mute, nor the device changes, nor the refused camera moved the call.
        assert_eq!(call.phase(), CallPhase::Active, "the call is still up");
        // A rebind is the pump's own work, on its own task, so the device it opened is waited for
        // rather than assumed to be there the moment the reader asked for it.
        assert!(
            until(|| speaker
                .opened()
                .iter()
                .any(|device| device.as_deref() == Some("Second speaker")))
            .await,
            "the speaker device change was a rebind"
        );
        // The device change opened the microphone again behind the very channel above, which is
        // what a rebind is: the read moved, the engine's port did not.
        assert!(
            until(|| call
                .mic
                .as_ref()
                .expect("the call still has its microphone")
                .opens
                .load(Ordering::Relaxed)
                == 2)
            .await,
            "one open for the call and one for the device change"
        );
        call.cleanup();
    }

    /// Opens this machine's camera and drives the pipeline the way a call does: start, pause,
    /// resume, and end. A machine with no camera has nothing to drive and says so.
    /// `cargo test --lib -- --ignored --nocapture calls::tests::hardware`
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "opens this machine's real camera"]
    async fn hardware_pipeline_starts_pauses_and_stops_the_camera() {
        let Some((device, label)) = crate::camera::cameras().into_iter().next() else {
            eprintln!("no camera on this machine: nothing to drive");
            return;
        };
        eprintln!("driving {device} ({label})");
        let (mut pipe, ticks) = VideoPipeline::start(Some(device.clone()), "test")
            .await
            .expect("the camera opens");
        assert!(pipe.camera_running(), "a started camera reads as on");
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), ticks.recv())
            .await
            .expect("the camera delivers within five seconds")
            .expect("the preview channel is open");
        assert!(matches!(frame, VideoTick::Local(_)), "our own preview");
        pipe.pause_camera();
        assert!(!pipe.camera_running(), "a pause reads as off at once");
        pipe.resume_camera(Some(device))
            .await
            .expect("the camera starts again");
        assert!(pipe.camera_running(), "a resumed camera reads as on again");
        pipe.shutdown();
        assert!(
            !pipe.camera_running(),
            "a call that has ended leaves no camera running"
        );
    }

    /// A video call goes on without a camera that cannot be opened: the other side's picture
    /// still arrives, and our own camera reads as off.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_video_call_goes_on_without_a_camera_it_cannot_open() {
        let (pipe, _ticks) = VideoPipeline::start(Some("no such camera".to_owned()), "test")
            .await
            .expect("the decoder side starts");
        assert!(!pipe.camera_running(), "the camera reads as off");
    }

    fn snapshot(phase: CallPhase, camera_on: bool) -> CallUpdate {
        CallUpdate {
            generation: 7,
            chat: "15551234567@s.whatsapp.net".to_owned(),
            direction: CallDirection::Outgoing,
            video: true,
            phase,
            started: None,
            muted: false,
            camera_on,
            remote_video: false,
            video_requested: false,
            outcome: None,
            peer_audio: None,
            lost_devices: Vec::new(),
            microphone: None,
            speaker: None,
            camera: None,
            group: None,
        }
    }

    #[test]
    fn a_call_with_no_camera_named_opens_with_the_machines_own() {
        // No camera picked is what the picker calls "First camera". For a camera
        // that has to mean the first one the machine reports, or a first video call could never
        // start: there is no system default camera to fall back on.
        let resolved = resolve_devices(&machine(), None, None, None);
        assert_eq!(resolved.camera.as_deref(), Some("0"));
        assert!(
            resolved.lost_devices.is_empty(),
            "nothing moved, so the screen has nothing to say"
        );
        // A machine that reports no camera at all still resolves to none, which is what makes the
        // video pipeline refuse the call rather than offer the peer an empty stream.
        let none = resolve_devices(&DeviceList::default(), None, None, None);
        assert_eq!(none.camera, None);
    }

    /// Turning video on mid-call reads the default the same way a call's startup does.
    ///
    /// The two paths that change a camera while a call is up — upgrading a voice call to video and
    /// picking a device in the call screen — used to pass the stored `None` straight to
    /// `VideoPipeline::start`, which rejects it. A fresh install could then not turn its camera on
    /// at all, even with a working camera attached, while starting a video call worked fine. Both
    /// now go through `default_camera`, so the reading is the same one `resolve_devices` applies.
    #[test]
    fn a_camera_turned_on_mid_call_opens_the_machines_own() {
        let machine = machine();
        assert_eq!(
            default_camera(&machine, None).as_deref(),
            Some("0"),
            "the picker's default means the first camera the machine reports"
        );
        // A camera that is still there is kept.
        assert_eq!(
            default_camera(&machine, Some("0".to_owned())).as_deref(),
            Some("0")
        );
        // One the machine no longer has falls back rather than failing the switch.
        assert_eq!(
            default_camera(&machine, Some("7".to_owned())).as_deref(),
            Some("0")
        );
        // No camera named and none reported stays none, so the caller refuses rather than invents.
        assert_eq!(default_camera(&DeviceList::default(), None), None);
        // An empty list means discovery could not run, not that nothing is plugged in: the choice
        // stands rather than being silently swapped for nothing.
        assert_eq!(
            default_camera(&DeviceList::default(), Some("3".to_owned())).as_deref(),
            Some("3")
        );
    }

    /// A peer who turns their camera off stops being reported as sending video.
    ///
    /// `remote_video` only ever became true, and nothing turned it back, so a peer who stopped
    /// their camera left the call reported as carrying video and the decoder's last frame on screen
    /// as if it were live. The peer's own `<video state=N>` signaling is what says otherwise.
    #[test]
    fn a_peer_that_stops_its_camera_stops_being_reported_as_sending_video() {
        use whatsapp_rust::voip::VideoState;
        let peer = |state| CallEvent::PeerVideoStateChanged {
            source: creator(),
            call_creator: creator(),
            state,
            orientation: None,
            upgrade_token: None,
        };
        let mut call = dialing();
        call.video = true;
        assert!(!call.remote_video);
        assert!(!call.remote_video_stopped(), "nothing to stop");
        assert!(call.saw_remote_video());
        assert!(call.remote_video);
        // The peer's own state saying video is inactive is what clears the flag.
        let update = call.media(&peer(VideoState::Stopped));
        assert!(update.is_some(), "the screen is told the picture stopped");
        assert!(
            !call.remote_video,
            "a stopped peer camera is not a live picture"
        );
        // A repeated inactive state publishes nothing further.
        assert!(
            call.media(&peer(VideoState::Disabled)).is_none(),
            "only a change is published"
        );
        // A state that is still active leaves it alone.
        call.saw_remote_video();
        assert!(
            call.media(&peer(VideoState::Enabled)).is_none(),
            "an active state is not a stop"
        );
        assert!(call.remote_video, "the picture is still there");
    }

    /// A peer who turns their camera on in a voice call asks; the screen says so, and a
    /// withdrawn request is forgotten.
    #[test]
    fn a_peer_asking_for_video_is_shown_until_it_withdraws() {
        use whatsapp_rust::voip::{VideoState, VideoUpgradeToken};
        let token = VideoUpgradeToken::builder().generation(1).epoch(1).build();
        let mut call = dialing();
        let asked = call
            .media(&CallEvent::PeerVideoStateChanged {
                source: creator(),
                call_creator: creator(),
                state: VideoState::UpgradeRequestV2,
                orientation: None,
                upgrade_token: Some(token),
            })
            .expect("the request reaches the screen");
        assert!(asked.video_requested);
        assert!(!asked.video, "asking is not yet a video call");
        let withdrawn = call
            .media(&CallEvent::PeerVideoStateChanged {
                source: creator(),
                call_creator: creator(),
                state: VideoState::UpgradeCancel,
                orientation: None,
                upgrade_token: None,
            })
            .expect("the withdrawal reaches the screen");
        assert!(!withdrawn.video_requested);
    }

    #[test]
    fn the_devices_a_person_picked_are_the_ones_a_call_opens_with() {
        let machine = machine();
        let resolved = resolve_devices(
            &machine,
            Some(machine.microphones[0].id.clone()),
            Some(machine.speakers[0].id.clone()),
            Some("0".to_owned()),
        );
        assert_eq!(resolved.microphone, Some(machine.microphones[0].id.clone()));
        assert_eq!(resolved.speaker, Some(machine.speakers[0].id.clone()));
        assert_eq!(resolved.camera.as_deref(), Some("0"));
        assert!(
            resolved.lost_devices.is_empty(),
            "nothing moved, so the screen has nothing to say"
        );
    }

    #[test]
    fn a_headset_that_is_gone_falls_back_to_the_default_and_is_reported() {
        let resolved = resolve_devices(
            &machine(),
            Some("bluez_input.AC_12_34_56".to_owned()),
            Some("bluez_output.AC_12_34_56".to_owned()),
            Some("7".to_owned()),
        );
        assert_eq!(
            resolved.microphone, None,
            "the microphone moves to the default"
        );
        assert_eq!(resolved.speaker, None, "and so does the speaker");
        assert_eq!(
            resolved.camera.as_deref(),
            Some("0"),
            "a camera the machine does not have falls back to the one it does"
        );
        assert_eq!(
            resolved.lost_devices,
            vec![
                LostDevice {
                    kind: DeviceKind::Microphone,
                    name: "bluez_input.AC_12_34_56".to_owned(),
                },
                LostDevice {
                    kind: DeviceKind::Speaker,
                    name: "bluez_output.AC_12_34_56".to_owned(),
                },
                LostDevice {
                    kind: DeviceKind::Camera,
                    name: "7".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn an_empty_list_is_a_broken_tool_rather_than_an_empty_machine() {
        let resolved = resolve_devices(
            &DeviceList::default(),
            Some("bluez_input.AC".to_owned()),
            Some("bluez_output.AC".to_owned()),
            Some("0".to_owned()),
        );
        assert_eq!(resolved.microphone.as_deref(), Some("bluez_input.AC"));
        assert_eq!(resolved.speaker.as_deref(), Some("bluez_output.AC"));
        assert_eq!(resolved.camera.as_deref(), Some("0"));
        assert!(
            resolved.lost_devices.is_empty(),
            "a device list that could not be read says nothing about the devices"
        );
    }

    #[test]
    fn a_device_is_named_the_way_the_picker_named_it() {
        let machine = machine();
        assert_eq!(
            device_name(&machine, DeviceKind::Camera, "0"),
            "Laptop Camera"
        );
        assert_eq!(
            device_name(
                &machine,
                DeviceKind::Microphone,
                "alsa_input.usb-Generic_USB_Headset-00.analog-mono"
            ),
            "USB Headset Microphone"
        );
        // Nothing in the list to name it by, which is what a device that just went away looks
        // like: the node itself is the honest answer.
        assert_eq!(
            device_name(&machine, DeviceKind::Microphone, "mic-gone"),
            "mic-gone"
        );
    }

    #[test]
    fn answering_is_not_the_same_as_dialing() {
        // An outgoing call the peer answered, and an incoming one we picked up, are both live and
        // both past the ringing stage, and neither has a running timer until the media comes up.
        for phase in [
            CallPhase::Connecting,
            CallPhase::Accepted,
            CallPhase::Active,
        ] {
            assert!(phase.is_live(), "{phase:?}");
            assert!(phase.is_connected(), "{phase:?}");
            assert!(phase.is_answered(), "{phase:?}");
        }
        for phase in [
            CallPhase::Placing,
            CallPhase::Dialing,
            CallPhase::Ringing,
            CallPhase::Incoming,
        ] {
            assert!(phase.is_live(), "{phase:?}");
            assert!(!phase.is_connected(), "{phase:?}");
            assert!(!phase.is_answered(), "{phase:?}");
        }
        for phase in [CallPhase::Ended, CallPhase::Failed] {
            assert!(!phase.is_live(), "{phase:?}");
            assert!(!phase.is_connected(), "{phase:?}");
        }
    }

    #[test]
    fn a_rejection_is_read_for_what_the_peer_meant() {
        assert_eq!(rejection_outcome(Some("busy")), CallOutcome::Busy);
        assert_eq!(rejection_outcome(Some("enc")), CallOutcome::Failed);
        assert_eq!(rejection_outcome(None), CallOutcome::Declined);
        // A reason we do not know is still a refusal, not a failure to connect.
        assert_eq!(rejection_outcome(Some("unknown")), CallOutcome::Declined);
    }

    #[test]
    fn a_termination_is_read_against_whether_the_call_was_up() {
        assert_eq!(
            termination_outcome(Some("accepted_elsewhere"), false),
            CallOutcome::AnsweredElsewhere
        );
        assert_eq!(
            termination_outcome(Some("rejected_elsewhere"), true),
            CallOutcome::DeclinedElsewhere
        );
        assert_eq!(
            termination_outcome(Some("timeout"), false),
            CallOutcome::NoAnswer
        );
        // A timeout on a call that was up is the line going away, not a call nobody answered: its
        // own duration is proof it was answered, and a log entry cannot say both. Reported from a
        // real nineteen-second call that ended four seconds after the camera came on.
        assert_eq!(
            termination_outcome(Some("timeout"), true),
            CallOutcome::ConnectionLost
        );
        // A plain terminate means the peer ended a call that was up, or gave up before it was.
        assert_eq!(termination_outcome(None, true), CallOutcome::Answered);
        assert_eq!(termination_outcome(None, false), CallOutcome::NoAnswer);
    }

    /// The NAL unit types in one Annex-B access unit.
    fn nal_types(unit: &[u8]) -> Vec<u8> {
        unit.windows(4)
            .enumerate()
            .filter(|(_, window)| window[..3] == [0, 0, 1])
            .filter_map(|(at, _)| unit.get(at + 3).map(|header| header & 0x1F))
            .collect()
    }

    /// What the camera hands the engine is what WhatsApp decodes: Constrained Baseline access
    /// units with their parameter sets on every keyframe, a keyframe on request, and pictures the
    /// decoder the call screen uses reads back at the size they were sent.
    #[test]
    fn the_camera_encoder_makes_access_units_a_decoder_reads() {
        use openh264::formats::YUVSource as _;
        let (width, height) = (320, 240);
        let luma: Vec<u8> = (0..width * height).map(|at| (at % width) as u8).collect();
        let chroma = vec![128u8; width * height / 4];
        let picture = Yuv420 {
            y: &luma,
            u: &chroma,
            v: &chroma,
            size: (width, height),
        };
        let mut encoder = video_encoder(20.0, 700_000).expect("the encoder starts");
        let mut decoder = openh264::decoder::Decoder::new().expect("the decoder starts");
        let mut units = Vec::new();
        for frame in 0..4 {
            if frame == 3 {
                encoder.force_intra_frame();
            }
            units.push(encoder.encode(&picture).expect("a frame encodes").to_vec());
        }
        let first = nal_types(&units[0]);
        assert!(
            first.contains(&7) && first.contains(&8) && first.contains(&5),
            "the first unit is a keyframe with its parameter sets: {first:?}"
        );
        let sps = units[0]
            .windows(4)
            .position(|window| window[..3] == [0, 0, 1] && window[3] & 0x1F == 7)
            .expect("a sequence parameter set");
        assert_eq!(units[0][sps + 4], 66, "Baseline profile");
        assert!(!nal_types(&units[1]).contains(&5), "then delta frames");
        assert!(
            nal_types(&units[3]).contains(&5),
            "and a keyframe when asked"
        );
        let decoded = decoder
            .decode(&units[0])
            .expect("the decoder reads it")
            .expect("a picture");
        assert_eq!(decoded.dimensions(), (width, height));
    }

    /// A grainy frame: a gradient under a layer of noise, which costs an encoder what a real
    /// camera's sensor noise does, so a rate change shows up in the size of what it makes.
    fn noise_frame(seed: &mut u32, len: usize) -> Vec<u8> {
        (0..len)
            .map(|at| {
                *seed ^= *seed << 13;
                *seed ^= *seed >> 17;
                *seed ^= *seed << 5;
                ((at % 160) as u8).wrapping_add((*seed % 48) as u8)
            })
            .collect()
    }

    /// The rate control moves the encoder's bitrate in the middle of a stream, and that costs the
    /// peer no keyframe: the encoder keeps sending delta frames, only smaller ones.
    #[test]
    fn the_camera_encoder_changes_its_rate_without_a_keyframe() {
        let (width, height) = (320, 240);
        let plan = SendPlan::new(
            (width, height),
            20.0,
            crate::video_rate::Setting {
                bitrate: 1_000_000,
                tier: 1,
            },
        );
        let mut encoder = CameraEncoder::new(&plan).expect("the encoder starts");
        let mut seed = 0x2545_f491;
        let mut encode = |encoder: &mut CameraEncoder| {
            let luma = noise_frame(&mut seed, width * height);
            let chroma = noise_frame(&mut seed, width * height / 4);
            let planes = Yuv420 {
                y: &luma,
                u: &chroma,
                v: &chroma,
                size: (width, height),
            };
            encoder.encode(&planes, false).expect("a frame encodes")
        };
        let fast: Vec<Vec<u8>> = (0..20).map(|_| encode(&mut encoder)).collect();
        assert_eq!(encoder.applied, Some(1_000_000));
        encoder.set_bitrate(250_000);
        let slow: Vec<Vec<u8>> = (0..20).map(|_| encode(&mut encoder)).collect();
        assert_eq!(encoder.applied, Some(250_000), "openh264 took the new rate");
        // The encoder's own scene detection makes keyframes of grain this heavy now and then, at
        // either rate; what matters is that the change itself does not.
        assert!(
            slow[..3].iter().all(|unit| !nal_types(unit).contains(&5)),
            "no keyframe for a rate change"
        );
        // Delta frames only, settled at each rate.
        let average = |units: &[Vec<u8>]| {
            let deltas: Vec<usize> = units
                .iter()
                .filter(|unit| !nal_types(unit).contains(&5))
                .map(Vec::len)
                .collect();
            deltas.iter().sum::<usize>() / deltas.len().max(1)
        };
        let (fast, slow) = (average(&fast[5..]), average(&slow[5..]));
        assert!(
            slow * 2 < fast,
            "a quarter of the rate makes frames well under half the size: {slow} vs {fast}"
        );
    }

    /// The Cam Link that delivered 1080p50 is thinned to every third frame at every step, so what
    /// goes out is 16.7 evenly spaced frames a second rather than the 40 and 60 ms mix that
    /// thinning by elapsed time made of it, at the size of the step the rate control chose.
    #[test]
    fn a_1080p50_capture_card_is_sent_in_even_steps_of_its_own_cadence() {
        use crate::video_rate::{Setting, TIERS};
        let full = crate::camera::fit_even((1920, 1080), (CAPTURE_LONG_SIDE, CAPTURE_SHORT_SIDE));
        assert_eq!(full, (1280, 720));
        let plans: Vec<SendPlan> = (0..TIERS.len())
            .map(|tier| {
                SendPlan::new(
                    full,
                    50.0,
                    Setting {
                        bitrate: 700_000,
                        tier,
                    },
                )
            })
            .collect();
        assert_eq!(
            plans.iter().map(|plan| plan.size).collect::<Vec<_>>(),
            [(852, 480), (960, 540), (1280, 720)]
        );
        assert!(plans.iter().all(|plan| plan.every == 2), "{plans:?}");
        assert!((plans[2].sent_fps() - 25.0).abs() < 1e-9);
        assert_eq!(plans[2].frame_len(), 1280 * 720 * 3 / 2);
        // A portrait camera keeps its shape at every step.
        let upright = SendPlan::new(
            (720, 1280),
            30.0,
            Setting {
                bitrate: 300_000,
                tier: 0,
            },
        );
        assert_eq!(upright.size, (480, 852));
        assert_eq!(
            upright.every, 1,
            "every frame of a 30 fps camera, the nearest to 25"
        );
    }

    #[test]
    fn a_peer_whose_first_keyframe_went_missing_is_asked_again_each_second() {
        let start = Instant::now();
        let mut waiting = FirstKeyframe::default();
        assert!(waiting.ask(start), "at once");
        assert!(!waiting.ask(start + std::time::Duration::from_millis(400)));
        assert!(waiting.ask(start + std::time::Duration::from_millis(1000)));
        assert!(!waiting.ask(start + std::time::Duration::from_millis(1500)));
    }

    /// The peer's loss reports, the relay connection's drops and the camera thread's own drops all
    /// move the camera's rate, and none of them is something to redraw the call screen for.
    #[test]
    fn the_peers_reports_move_the_cameras_rate() {
        use whatsapp_rust::voip_control::MediaRtcpReportBlock;
        let block = |ssrc: u32, fraction_lost: u8| {
            MediaRtcpReportBlock::builder()
                .ssrc(ssrc)
                .fraction_lost(fraction_lost)
                .cumulative_lost(0)
                .extended_highest_sequence(0)
                .jitter(0)
                .last_sender_report(0)
                .delay_since_last_sender_report(0)
                .profile_extension(Vec::new())
                .build()
        };
        let report = |lost: u8| CallEvent::RtcpReceived {
            packet_types: vec![201],
            sender_ssrc: 1,
            referenced_ssrcs: vec![9],
            reports_audio: false,
            reports_video: true,
            report_blocks: vec![block(9, lost)],
            feedback: Vec::new(),
        };
        let mut call = dialing();
        call.video = true;
        call.video_pipe = Some(pipeline_with_a_running_camera());
        let rate = |call: &Call| call.video_pipe.as_ref().expect("video").rate().bitrate;
        let start = rate(&call);
        assert_eq!(start, crate::video_rate::START_BITRATE);
        assert_eq!(call.media(&report(128)), None, "nothing for the screen");
        let after_loss = rate(&call);
        assert!(after_loss < start, "half the video lost lowers the rate");
        assert_eq!(
            call.media(&CallEvent::OutboundMediaDropped {
                video_access_units: 2,
                packets: 40,
            }),
            None
        );
        let after_drops = rate(&call);
        assert!(after_drops < after_loss, "the send queue's drops lower it");
        // The camera thread's own drops count once the call is up, and not while the peer's phone
        // rings, when nothing drains the queue yet.
        let note_drop = |call: &Call| {
            call.video_pipe
                .as_ref()
                .expect("video")
                .rate_share
                .note_queue_drop();
        };
        note_drop(&call);
        call.check_video_queue();
        assert_eq!(rate(&call), after_drops, "ringing: not a sign of the path");
        call.phase = CallPhase::Active;
        note_drop(&call);
        call.check_video_queue();
        assert!(
            rate(&call) < after_drops,
            "up: the path carries less than is sent"
        );

        // A voice call has no camera rate to move, and a report is not an error for it.
        let mut voice = dialing();
        assert_eq!(voice.media(&report(128)), None);
        voice.check_video_queue();
    }

    /// Frames the camera could not hand over while the phone rang are not counted against the
    /// path at the first heartbeat of the call.
    #[test]
    fn drops_from_before_the_call_was_up_do_not_cut_the_rate() {
        let mut call = dialing();
        call.video = true;
        call.phase = CallPhase::Accepted;
        call.media_ready = true;
        call.video_pipe = Some(pipeline_with_a_running_camera());
        fn pipe(call: &Call) -> &VideoPipeline {
            call.video_pipe.as_ref().expect("video")
        }
        pipe(&call).rate_share.note_queue_drop();
        assert!(call.became_active().is_some());
        call.check_video_queue();
        assert_eq!(pipe(&call).rate().bitrate, crate::video_rate::START_BITRATE);
    }

    /// The peer's NACKs, keyframe requests and estimates are tallied for the heartbeat's log line,
    /// with the extension of its latest report on our video.
    #[test]
    fn the_peers_feedback_on_our_video_is_tallied() {
        use whatsapp_rust::voip_control::{MediaRtcpFeedback, MediaRtcpReportBlock};
        let feedback = |packet_type: u8, fmt: u8, fci: &[u8]| {
            MediaRtcpFeedback::builder()
                .packet_type(packet_type)
                .fmt(fmt)
                .sender_ssrc(1)
                .media_ssrc(9)
                .fci(fci.to_vec())
                .build()
        };
        let block = MediaRtcpReportBlock::builder()
            .ssrc(9)
            .fraction_lost(0)
            .cumulative_lost(0)
            .extended_highest_sequence(0)
            .jitter(0)
            .last_sender_report(0)
            .delay_since_last_sender_report(0)
            .profile_extension(vec![0xab; 24])
            .build();
        let mut call = dialing();
        call.video = true;
        call.video_pipe = Some(pipeline_with_a_running_camera());
        call.media(&CallEvent::RtcpReceived {
            packet_types: vec![201, 205, 206, 206],
            sender_ssrc: 1,
            referenced_ssrcs: vec![9],
            reports_audio: false,
            reports_video: true,
            report_blocks: vec![block],
            feedback: vec![
                // One packet and the two its bitmask names.
                feedback(205, 1, &[0, 10, 0b101, 0]),
                feedback(206, 1, &[]),
                feedback(
                    206,
                    15,
                    &[b'R', b'E', b'M', b'B', 1, 4 << 2, 0x30, 0xD4, 0, 0, 0, 9],
                ),
            ],
        });
        let pipe = call.video_pipe.as_ref().expect("video");
        assert_eq!(
            pipe.feedback,
            crate::video_rate::PeerFeedback {
                nacked_packets: 3,
                picture_loss: 1,
                full_intra: 0,
                estimates: 1,
                last_estimate: Some(200_000),
            }
        );
        assert_eq!(pipe.report_extension, vec![0xab; 24]);
    }

    /// A report on our video alone whose extension says the peer receives far less than we send
    /// caps the rate; the same extension on a report that covers audio as well does not.
    #[test]
    fn what_the_peer_receives_of_our_video_caps_the_rate() {
        use whatsapp_rust::voip_control::MediaRtcpReportBlock;
        let mut extension = vec![0; 24];
        extension[..4].copy_from_slice(&2_000_000u32.to_be_bytes());
        extension[20..].copy_from_slice(&400_000u32.to_be_bytes());
        let report = |reports_audio: bool| CallEvent::RtcpReceived {
            packet_types: vec![201],
            sender_ssrc: 1,
            referenced_ssrcs: vec![9],
            reports_audio,
            reports_video: true,
            report_blocks: vec![
                MediaRtcpReportBlock::builder()
                    .ssrc(9)
                    .fraction_lost(0)
                    .cumulative_lost(0)
                    .extended_highest_sequence(0)
                    .jitter(0)
                    .last_sender_report(0)
                    .delay_since_last_sender_report(0)
                    .profile_extension(extension.clone())
                    .build(),
            ],
            feedback: Vec::new(),
        };
        let mut call = dialing();
        call.video = true;
        let mut pipe = pipeline_with_a_running_camera();
        // Two seconds ago nothing had been sent; since then 700 kbit/s has.
        let earlier = Instant::now() - std::time::Duration::from_secs(2);
        let nothing_yet = crate::video_rate::PeerReceiving {
            bytes: 0,
            bitrate: 0,
        };
        assert_eq!(pipe.rate.on_peer_receiving(nothing_yet, earlier), None);
        pipe.rate_share.note_sent(175_000);
        call.video_pipe = Some(pipe);
        let rate = |call: &Call| call.video_pipe.as_ref().expect("video").rate().bitrate;
        assert_eq!(call.media(&report(true)), None);
        assert!(
            rate(&call) >= crate::video_rate::START_BITRATE,
            "not about our video alone: read only for its clean loss"
        );
        assert_eq!(call.media(&report(false)), None, "nothing for the screen");
        assert_eq!(rate(&call), 360_000);
    }

    #[test]
    fn the_corner_preview_keeps_the_capture_shape() {
        assert_eq!(preview_size((1280, 720)), (320, 180));
        assert_eq!(preview_size((640, 480)), (240, 180));
        assert_eq!(preview_size((720, 1280)), (180, 320));
    }

    #[test]
    fn the_periodic_check_only_republishes_a_real_change() {
        // The worker compares each fresh snapshot with the one the UI was handed, so a camera that
        // stopped on its own reaches the screen and a call that is merely ticking does not.
        assert_eq!(
            snapshot(CallPhase::Active, false),
            snapshot(CallPhase::Active, false)
        );
        assert_ne!(
            snapshot(CallPhase::Active, true),
            snapshot(CallPhase::Active, false)
        );
        assert_ne!(
            snapshot(CallPhase::Accepted, false),
            snapshot(CallPhase::Active, false)
        );
    }

    // The state machine, driven by the stanzas and media events themselves rather than by an engine.
    //
    // Nothing here needs a peer, a relay, or a socket: the pieces under test are the rules that
    // decide what a call *is* once a stanza arrives, and those rules are the same whether the bytes
    // came off a real relay or were handed over directly. Every field is set, so a new one breaks
    // these tests rather than silently going untested.

    const CALL_ID: &str = "3EB0CALLID";
    const PEER: &str = "15551234567@s.whatsapp.net";

    /// An outgoing call as it looks right after the offer went out, with no engine attached.
    fn dialing() -> Call {
        Call {
            generation: 1,
            call_id: CALL_ID.to_owned(),
            chat: PEER.to_owned(),
            direction: CallDirection::Outgoing,
            video: false,
            phase: CallPhase::Dialing,
            started: None,
            outcome: None,
            peer_audio: None,
            incoming: None,
            handle: None,
            media_ready: false,
            mic: None,
            speaker: None,
            video_pipe: None,
            remote_video: false,
            microphone: None,
            speaker_device: None,
            camera: None,
            lost_devices: Vec::new(),
            camera_wanted: false,
            peer_video_request: None,
            began: crate::util::now(),
            group: None,
            join_task: None,
        }
    }

    fn creator() -> Jid {
        PEER.parse().expect("peer jid")
    }

    fn preaccept() -> CallAction {
        CallAction::PreAccept {
            call_id: CALL_ID.to_owned(),
            call_creator: creator(),
            audio: Vec::new(),
        }
    }

    fn accept() -> CallAction {
        CallAction::Accept {
            call_id: CALL_ID.to_owned(),
            call_creator: creator(),
            audio: Vec::new(),
        }
    }

    fn terminate(reason: Option<&str>) -> CallAction {
        CallAction::Terminate {
            call_id: CALL_ID.to_owned(),
            call_creator: creator(),
            reason: reason.map(str::to_owned),
            duration: None,
            audio_duration: None,
        }
    }

    use test_offers::{GROUP, group_offer, member, roster};

    #[test]
    fn a_group_offer_rings_with_its_caller_and_roster() {
        let incoming = group_offer(vec![
            member("200000000000001", true),
            member("200000000000002", false),
        ]);
        assert!(offer_is_group(&incoming));
        assert_eq!(
            offer_group(&incoming).map(|jid| jid.to_string()),
            Some(GROUP.to_owned())
        );
        let call = Call::ringing(GROUP.to_owned(), Box::new(incoming), false);
        assert!(call.is_group());
        let group = call.update().group.expect("a group call");
        assert_eq!(
            group.caller.as_deref(),
            Some("200000000000001@lid"),
            "the device that rang, without its device number"
        );
        assert_eq!(
            group.participants,
            vec![
                Participant {
                    id: "200000000000001@lid".to_owned(),
                    joined: true,
                    video: false,
                },
                Participant {
                    id: "200000000000002@lid".to_owned(),
                    joined: false,
                    video: false,
                },
            ]
        );
    }

    /// A member's first picture turns their tile to it; their own video state stopping, or their
    /// leaving, turns it back, and a picture from someone the roster does not name has no tile.
    #[test]
    fn a_members_picture_comes_and_goes_with_their_camera() {
        use whatsapp_rust::voip::VideoState;
        let incoming = group_offer(vec![
            member("200000000000001", true),
            member("200000000000002", true),
        ]);
        let mut call = Call::ringing(GROUP.to_owned(), Box::new(incoming), true);
        call.phase = CallPhase::Active;
        let sender: Jid = "200000000000002:4@lid".parse().unwrap();
        let sending = |call: &Call| -> Vec<bool> {
            call.update()
                .group
                .unwrap()
                .participants
                .iter()
                .map(|person| person.video)
                .collect()
        };

        assert_eq!(
            call.group_picture(&sender),
            Some(("200000000000002@lid".to_owned(), true)),
            "the first picture, from any of the member's devices"
        );
        assert_eq!(
            call.group_picture(&sender),
            Some(("200000000000002@lid".to_owned(), false))
        );
        assert_eq!(sending(&call), [false, true]);
        assert_eq!(
            call.group_picture(&"200000000000077@lid".parse().unwrap()),
            None,
            "a stranger has no tile"
        );

        let stopped = CallEvent::PeerVideoStateChanged {
            source: sender.clone(),
            call_creator: test_offers::CREATOR.parse().unwrap(),
            state: VideoState::Stopped,
            orientation: None,
            upgrade_token: None,
        };
        assert!(call.media(&stopped).is_some(), "their tile goes back");
        assert_eq!(sending(&call), [false, false]);
        assert!(call.media(&stopped).is_none(), "only once");

        call.group_picture(&sender);
        let left = roster(2, vec![member("200000000000001", true)]);
        call.media(&CallEvent::GroupUpdated(Box::new(left)));
        let back = roster(
            3,
            vec![
                member("200000000000001", true),
                member("200000000000002", true),
            ],
        );
        call.media(&CallEvent::GroupUpdated(Box::new(back)));
        assert_eq!(
            sending(&call),
            [false, false],
            "someone who left and came back has not sent a picture yet"
        );
    }

    /// A call link the call service will not let us into fails as a call that never came up,
    /// and a join that answers after a hang-up changes nothing.
    #[test]
    fn a_call_link_that_cannot_be_joined_says_so() {
        let mut call = dialing();
        call.call_id = String::new();
        call.chat = String::new();
        call.phase = CallPhase::Connecting;
        call.group = Some(GroupRoster {
            link: true,
            ..GroupRoster::default()
        });
        assert!(call.update().group.is_some_and(|group| group.link));
        let update = call
            .link_joined(Err("call-link ended before admission".to_owned()))
            .expect("the screen is told");
        assert_eq!(update.phase, CallPhase::Failed);
        assert_eq!(update.outcome, Some(CallOutcome::Failed));
        assert!(call.link_joined(Err("again".to_owned())).is_none());
    }

    /// A group call we started rings its members until one of them joins: our own entry on the
    /// roster is not an answer, and the call is up only once someone else is on it and the media
    /// is too.
    #[tokio::test]
    async fn a_group_call_we_started_is_answered_by_a_member_joining() {
        let mut call = dialing();
        call.chat = GROUP.to_owned();
        call.group = Some(GroupRoster {
            own: vec!["200000000000099@lid".parse().unwrap()],
            ..GroupRoster::default()
        });
        let ringing = roster(
            1,
            vec![
                member("200000000000099", true),
                member("200000000000001", false),
                member("200000000000002", false),
            ],
        );
        let update = call
            .media(&CallEvent::GroupUpdated(Box::new(ringing)))
            .expect("the roster is drawn");
        assert_eq!(update.phase, CallPhase::Dialing, "only we are on it");
        assert!(call.media(&CallEvent::RelayAllocated).is_none());
        assert_eq!(
            call.phase(),
            CallPhase::Dialing,
            "a relay alone is no answer"
        );

        let joined = roster(
            2,
            vec![
                member("200000000000099", true),
                member("200000000000001", true),
                member("200000000000002", false),
            ],
        );
        let update = call
            .signaling(&CallAction::GroupUpdate {
                update: Box::new(joined),
            })
            .expect("a member joined");
        assert_eq!(update.phase, CallPhase::Active);
        assert!(update.started.is_some());

        let ended = call.hangup(None).await;
        assert_eq!(ended.outcome, Some(CallOutcome::Answered));
    }

    /// A group call nobody joined and we hung up was not answered.
    #[tokio::test]
    async fn a_group_call_we_started_that_nobody_joined_was_not_answered() {
        let mut call = dialing();
        call.chat = GROUP.to_owned();
        call.group = Some(GroupRoster {
            own: vec!["200000000000099@lid".parse().unwrap()],
            ..GroupRoster::default()
        });
        call.media(&CallEvent::GroupUpdated(Box::new(roster(
            1,
            vec![
                member("200000000000099", true),
                member("200000000000001", false),
            ],
        ))));
        let ended = call.hangup(None).await;
        assert_eq!(ended.phase, CallPhase::Ended);
        assert_eq!(ended.outcome, Some(CallOutcome::NoAnswer));
    }

    /// The people on a group call come from every roster the service sent: someone who joined and
    /// left before the end is still one of them, someone it only asked is not, and one member
    /// named by two ids is one person. A one-to-one call has no roster to name anyone from.
    #[tokio::test]
    async fn a_group_calls_people_are_everyone_who_was_on_it() {
        let mut call = dialing();
        assert_eq!(call.group_people(), None, "a one-to-one call has no roster");
        call.chat = GROUP.to_owned();
        call.group = Some(GroupRoster {
            own: vec!["200000000000099@lid".parse().unwrap()],
            ..GroupRoster::default()
        });
        assert_eq!(call.group_people(), Some(Vec::new()), "nobody is on it yet");

        call.media(&CallEvent::GroupUpdated(Box::new(roster(
            1,
            vec![
                member("200000000000099", true),
                member("200000000000001", true),
                member("200000000000002", false),
            ],
        ))));
        assert_eq!(
            call.group_people().unwrap(),
            ["200000000000099@lid", "200000000000001@lid"],
            "the member on the call and our own entry, not the one only asked"
        );

        call.media(&CallEvent::GroupUpdated(Box::new(roster(
            2,
            vec![
                member("200000000000099", true),
                member("200000000000001", true),
                member("200000000000002", true),
            ],
        ))));
        assert_eq!(
            call.group_people().unwrap().len(),
            3,
            "the second member joined too"
        );

        // The same member, now named by their phone number as well: one person, not two.
        let mut with_number = member("200000000000001", true);
        with_number.pn = Some("15550001111@s.whatsapp.net".parse().unwrap());
        call.media(&CallEvent::GroupUpdated(Box::new(roster(
            3,
            vec![
                member("200000000000099", true),
                with_number,
                member("200000000000002", true),
            ],
        ))));
        assert_eq!(
            call.group_people().unwrap().len(),
            3,
            "a second id for one person is one person"
        );

        // One of them leaves before the call ends: who was on the call does not shrink.
        call.media(&CallEvent::GroupUpdated(Box::new(roster(
            4,
            vec![
                member("200000000000099", true),
                member("200000000000001", true),
            ],
        ))));
        assert_eq!(
            call.group_people().unwrap().len(),
            3,
            "a member who left was still on the call"
        );
    }

    /// The call service refusing a group call we started fails it as a call that never came up,
    /// and an answer arriving after a hang-up changes nothing.
    #[test]
    fn a_refused_group_call_fails() {
        let mut call = dialing();
        call.call_id = String::new();
        call.chat = GROUP.to_owned();
        call.group = Some(GroupRoster::default());
        let refusal = GroupRefusal {
            not_enabled: true,
            detail: "call service response failed: missing group-call integer attribute".to_owned(),
        };
        let update = call
            .group_started(Err(refusal.clone()))
            .expect("the screen is told");
        assert_eq!(update.phase, CallPhase::Failed);
        assert_eq!(update.outcome, Some(CallOutcome::Failed));
        assert!(
            update.group.is_some(),
            "drawn as the group's call to the end"
        );
        assert!(call.group_started(Err(refusal)).is_none());
    }

    /// A refused group offer reads as WhatsApp not having turned group calls on for the account
    /// when the library names the 427, or the account's gate is off; anything else stays an
    /// ordinary failure.
    #[test]
    fn a_427_refusal_reads_as_group_calls_not_enabled() {
        use whatsapp_rust::CallError;
        let unreadable = || CallError::Response("missing group-call integer attribute".to_owned());
        assert!(GroupRefusal::of(&unreadable(), Some(false)).not_enabled);
        assert!(!GroupRefusal::of(&unreadable(), Some(true)).not_enabled);
        assert!(!GroupRefusal::of(&unreadable(), None).not_enabled);
        let named = CallError::Response("offer rejected with error 427".to_owned());
        assert!(GroupRefusal::of(&named, Some(true)).not_enabled);
        assert!(GroupRefusal::of(&named, None).not_enabled);
        let other = CallError::Setup("group call requires 2..=31 remote users".to_owned());
        assert!(!GroupRefusal::of(&other, Some(false)).not_enabled);
        assert!(!GroupRefusal::of(&CallError::ResponseTimeout, Some(false)).not_enabled);
        assert_eq!(
            GroupRefusal::of(&unreadable(), Some(false)).detail,
            "call service response failed: missing group-call integer attribute"
        );
    }

    #[test]
    fn a_one_to_one_offer_is_no_group_call() {
        let incoming = IncomingCall::builder()
            .from(creator())
            .stanza_id("STANZA".to_owned())
            .offline(false)
            .action(CallAction::Offer {
                call_id: CALL_ID.to_owned(),
                call_creator: creator(),
                caller_pn: None,
                caller_country_code: None,
                device_class: None,
                joinable: false,
                is_video: false,
                audio: Vec::new(),
                group_jid: None,
            })
            .build();
        assert!(!offer_is_group(&incoming));
        let call = Call::ringing(PEER.to_owned(), Box::new(incoming), false);
        assert!(call.update().group.is_none());
    }

    #[test]
    fn an_invitation_from_the_call_service_names_the_member_who_added_us() {
        let mut incoming = group_offer(vec![member("200000000000003", true)]);
        incoming.from = format!("{CALL_ID}@call").parse().unwrap();
        incoming.participant = Some("200000000000003:7@lid".parse().unwrap());
        let call = Call::ringing(GROUP.to_owned(), Box::new(incoming), false);
        assert_eq!(
            call.update()
                .group
                .and_then(|group| group.caller)
                .as_deref(),
            Some("200000000000003@lid")
        );
    }

    /// Joining and leaving are new roster snapshots; a late one never undoes a newer one, and one
    /// member saying no leaves the call to everyone else.
    #[test]
    fn a_group_calls_roster_follows_the_newest_snapshot() {
        let incoming = group_offer(vec![member("200000000000001", true)]);
        let mut call = Call::ringing(GROUP.to_owned(), Box::new(incoming), false);
        call.phase = CallPhase::Active;
        call.started = Some(Instant::now());

        let joined = roster(
            3,
            vec![
                member("200000000000001", true),
                member("200000000000002", true),
            ],
        );
        let update = call
            .media(&CallEvent::GroupUpdated(Box::new(joined.clone())))
            .expect("someone joined");
        assert_eq!(update.group.unwrap().joined().count(), 2);
        assert!(
            call.media(&CallEvent::GroupUpdated(Box::new(joined)))
                .is_none(),
            "the same roster again changes nothing"
        );

        let stale = roster(2, vec![member("200000000000001", true)]);
        assert!(
            call.signaling(&CallAction::GroupUpdate {
                update: Box::new(stale)
            })
            .is_none(),
            "an older snapshot is not applied"
        );
        assert_eq!(call.update().group.unwrap().participants.len(), 2);

        let left = roster(4, vec![member("200000000000002", true)]);
        let update = call
            .signaling(&CallAction::GroupUpdate {
                update: Box::new(left),
            })
            .expect("someone left");
        assert_eq!(update.group.unwrap().participants.len(), 1);

        assert!(
            call.signaling(&CallAction::Reject {
                call_id: CALL_ID.to_owned(),
                call_creator: creator(),
                reason: None,
            })
            .is_none(),
            "one member's refusal is not the end of the call"
        );
        assert_eq!(call.phase(), CallPhase::Active);

        let ended = call
            .signaling(&terminate(Some("group_call_ended")))
            .expect("the call service ends it");
        assert_eq!(ended.phase, CallPhase::Ended);
        assert_eq!(ended.outcome, Some(CallOutcome::Answered));
    }

    #[test]
    fn ringing_is_told_from_dialing_and_dialing_shows_no_duration() {
        let mut call = dialing();
        assert!(call.phase().is_live());
        assert!(!call.phase().is_connected(), "nobody has answered yet");
        let update = call.signaling(&preaccept()).expect("the peer is ringing");
        assert_eq!(update.phase, CallPhase::Ringing);
        assert_eq!(update.started, None, "the timer has not begun");
        // A second preaccept says nothing new.
        assert!(call.signaling(&preaccept()).is_none());
    }

    #[test]
    fn a_relay_alone_never_makes_a_call_active() {
        // The allocation lands as soon as our offer is acked, while the peer's phone is still
        // ringing. Reading it as an answer would show "Connected" to a call nobody picked up.
        let mut call = dialing();
        call.signaling(&preaccept()).expect("ringing");
        assert!(
            call.media(&CallEvent::RelayAllocated).is_none(),
            "nothing is published for a call that has not been answered"
        );
        assert_eq!(call.phase(), CallPhase::Ringing);
        assert!(call.started.is_none(), "the timer still has not begun");
    }

    #[test]
    fn the_answer_and_the_media_plane_together_make_a_call_active() {
        // In this order: the peer answers before the relay is up, which is the usual one.
        let mut call = dialing();
        let connecting = call.signaling(&accept()).expect("the peer answered");
        assert_eq!(connecting.phase, CallPhase::Connecting);
        assert_eq!(
            connecting.started, None,
            "there is no media path yet, so there is no call to time"
        );
        let active = call
            .media(&CallEvent::RelayAllocated)
            .expect("the media plane is live");
        assert_eq!(active.phase, CallPhase::Active);
        assert!(active.started.is_some(), "the duration starts now");
        assert!(call.phase().is_connected());
    }

    #[test]
    fn the_answer_arriving_second_still_makes_a_call_active() {
        // In this order: the relay came up first, then the peer answered.
        let mut call = dialing();
        call.media(&CallEvent::RelayAllocated);
        let update = call.signaling(&accept()).expect("the peer answered");
        assert_eq!(update.phase, CallPhase::Active);
        assert!(update.started.is_some());
    }

    #[test]
    fn a_stanza_for_another_call_is_left_alone() {
        let mut call = dialing();
        let other = CallAction::Accept {
            call_id: "someone-elses-call".to_owned(),
            call_creator: creator(),
            audio: Vec::new(),
        };
        assert!(call.signaling(&other).is_none());
        assert_eq!(call.phase(), CallPhase::Dialing);
    }

    #[test]
    fn a_rejection_says_which_kind_it_was_and_releases_the_call() {
        for (reason, expected) in [
            (Some("busy"), CallOutcome::Busy),
            (Some("enc"), CallOutcome::Failed),
            (None, CallOutcome::Declined),
        ] {
            let mut call = dialing();
            call.media(&CallEvent::RelayAllocated);
            let update = call
                .signaling(&CallAction::Reject {
                    call_id: CALL_ID.to_owned(),
                    call_creator: creator(),
                    reason: reason.map(str::to_owned),
                })
                .expect("the peer rejected the call");
            assert_eq!(update.phase, CallPhase::Failed);
            assert_eq!(update.outcome, Some(expected));
            assert!(call.handle.is_none(), "the media is released");
            assert!(
                call.started.is_none(),
                "a call that was never answered has no length"
            );
        }
    }

    #[test]
    fn a_peer_who_never_answered_reads_as_no_answer_and_a_lost_line_as_connection_lost() {
        let mut call = dialing();
        call.signaling(&preaccept()).expect("ringing");
        let update = call
            .signaling(&terminate(Some("timeout")))
            .expect("the peer gave up");
        assert_eq!(update.phase, CallPhase::Failed);
        assert_eq!(update.outcome, Some(CallOutcome::NoAnswer));

        // A call that was up and lost its media is a different thing from one that never came up.
        let mut call = dialing();
        call.media(&CallEvent::RelayAllocated);
        call.signaling(&accept()).expect("the peer answered");
        let update = call
            .media(&CallEvent::Closed(
                whatsapp_rust::voip_control::MediaCloseReason::RelayDisconnected,
            ))
            .expect("the media is gone");
        assert_eq!(update.phase, CallPhase::Ended);
        assert_eq!(update.outcome, Some(CallOutcome::ConnectionLost));
    }

    #[test]
    fn a_call_that_had_been_up_ends_as_answered_with_its_timer_running() {
        let mut call = dialing();
        call.media(&CallEvent::RelayAllocated);
        call.signaling(&accept()).expect("the peer answered");
        // The length counts from the moment the call became active, not from the button press, and
        // the monotonic start is what carries it.
        let started = call.started.expect("an active call has started");
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        let update = call.signaling(&terminate(None)).expect("the peer hung up");
        assert_eq!(update.phase, CallPhase::Ended);
        assert_eq!(update.outcome, Some(CallOutcome::Answered));
        assert_eq!(
            update.started,
            Some(started),
            "an answered call keeps its start"
        );
    }

    #[test]
    fn the_call_this_one_resolved_elsewhere_reads_as_the_right_kind_of_ending() {
        // An outgoing call answered on another of the account's devices: it was not ours to have,
        // and it was not a call nobody answered either.
        let mut call = dialing();
        let update = call
            .resolved_elsewhere(None)
            .expect("the other device took the call");
        assert_eq!(update.phase, CallPhase::Failed);
        assert_eq!(update.outcome, Some(CallOutcome::NoAnswer));

        // A ringing incoming call resolved elsewhere: this device never picked it up.
        let ringing = || {
            let mut incoming = dialing();
            incoming.direction = CallDirection::Incoming;
            incoming.phase = CallPhase::Incoming;
            incoming
        };
        let update = ringing()
            .resolved_elsewhere(None)
            .expect("resolved elsewhere");
        assert_eq!(update.phase, CallPhase::Failed);
        assert_eq!(update.outcome, Some(CallOutcome::Missed));

        // The phone took it, or turned it down, while it rang here: said as such.
        let update = ringing()
            .resolved_elsewhere(Some(CallOutcome::AnsweredElsewhere))
            .expect("answered on the phone");
        assert_eq!(update.outcome, Some(CallOutcome::AnsweredElsewhere));
        let update = ringing()
            .resolved_elsewhere(Some(CallOutcome::DeclinedElsewhere))
            .expect("declined on the phone");
        assert_eq!(update.outcome, Some(CallOutcome::DeclinedElsewhere));

        // A call that was up and then resolved elsewhere was answered, just not here.
        let mut live = dialing();
        live.media(&CallEvent::RelayAllocated);
        live.signaling(&accept()).expect("the peer answered");
        let update = live.resolved_elsewhere(None).expect("resolved elsewhere");
        assert_eq!(update.phase, CallPhase::Ended);
        // The other device answered, so this is neither an answered call here nor one nobody
        // answered: it is its own kind of ending, and the outcome is what says so rather than the
        // local timer, which did start.
        assert_eq!(update.outcome, Some(CallOutcome::AnsweredElsewhere));
        assert!(
            live.started.is_some(),
            "the call had been live here as well"
        );
    }

    #[test]
    fn a_call_resolved_elsewhere_before_it_is_active_is_not_answered_elsewhere() {
        // The peer answered and the media plane is still negotiating, so this device never reached
        // Active: a resolve now is not a call another device took with a zero local duration.
        let mut call = dialing();
        call.signaling(&accept()).expect("the peer answered");
        assert_eq!(call.phase(), CallPhase::Connecting);
        let update = call.resolved_elsewhere(None).expect("resolved elsewhere");
        assert_eq!(update.phase, CallPhase::Failed);
        assert_eq!(update.outcome, Some(CallOutcome::NoAnswer));
        assert!(call.started.is_none(), "never answered here");
    }

    #[tokio::test]
    async fn a_call_that_is_over_keeps_the_ending_it_recorded() {
        let mut call = dialing();
        call.media(&CallEvent::RelayAllocated);
        let rejected = call
            .signaling(&CallAction::Reject {
                call_id: CALL_ID.to_owned(),
                call_creator: creator(),
                reason: None,
            })
            .expect("the peer rejected the call");
        assert_eq!(rejected.outcome, Some(CallOutcome::Declined));

        // The terminate a rejecting peer sends next must not relabel the call as answered
        // nowhere: the log keeps the reason the call really ended.
        assert!(call.signaling(&terminate(Some("timeout"))).is_none());
        assert_eq!(call.outcome, Some(CallOutcome::Declined));
        let update = call.hangup(None).await;
        assert_eq!(update.phase, CallPhase::Failed);
        assert_eq!(update.outcome, Some(CallOutcome::Declined));

        // The media plane failing under a call that is already over says nothing new either.
        assert!(call.media(&CallEvent::RelayAllocateFailed(1)).is_none());
        assert!(call.media_ended().is_none());
    }

    #[test]
    fn a_video_call_is_refused_rather_than_quietly_downgraded() {
        // A voice call needs no camera, so no pipeline is not a failure.
        assert!(require_video(false, false).is_ok());
        assert!(require_video(false, true).is_ok());
        assert!(require_video(true, true).is_ok());
        // Video was asked for and did not start: the call is refused, not turned into voice.
        assert!(require_video(true, false).is_err());
    }

    #[test]
    fn a_peer_terminate_before_the_call_is_active_is_not_an_answered_call() {
        // The peer answered, so we are past ringing, but the media plane never came up: the call
        // was never Active, so a terminate here is not a call of zero seconds that was answered.
        let mut call = dialing();
        call.signaling(&accept()).expect("the peer answered");
        assert_eq!(call.phase(), CallPhase::Connecting);
        assert!(call.started.is_none(), "no media path, so no duration");
        let update = call.signaling(&terminate(None)).expect("the peer hung up");
        assert_eq!(update.phase, CallPhase::Failed);
        assert_eq!(update.outcome, Some(CallOutcome::NoAnswer));
    }

    #[test]
    fn media_that_disappears_before_the_call_is_active_is_not_an_answered_call() {
        let mut call = dialing();
        call.signaling(&accept()).expect("the peer answered");
        assert_eq!(call.phase(), CallPhase::Connecting);
        let update = call
            .media(&CallEvent::Closed(
                whatsapp_rust::voip_control::MediaCloseReason::RelayDisconnected,
            ))
            .expect("the media is gone");
        // The cause is still a lost line, but the call never came up: it is Failed, not Ended, and
        // it is not booked as an answered call of zero seconds.
        assert_eq!(update.phase, CallPhase::Failed);
        assert_eq!(update.outcome, Some(CallOutcome::ConnectionLost));
        assert!(call.started.is_none(), "the call never came up");
    }
}

/// The device discovery, on the machine it is running on.
///
/// Ignored by default: it reads this computer's real audio devices, so its result depends on the
/// hardware. Run it with `cargo test --lib -- --ignored --nocapture calls::hardware_tests` to see
/// what a call would be offered, which is how a report about a device that is missing from the
/// pickers gets answered.
#[cfg(test)]
mod hardware_tests {
    #[test]
    #[ignore = "reads this machine's real devices"]
    fn the_machine_this_runs_on_lists_its_devices() {
        let list = super::devices();
        eprintln!("microphones: {:#?}", list.microphones);
        eprintln!("speakers: {:#?}", list.speakers);
        eprintln!("cameras: {:#?}", list.cameras);
        for device in list.microphones.iter().chain(&list.speakers) {
            assert!(
                !device.id.is_empty(),
                "a device is named by what the platform reports"
            );
            assert!(
                !device.id.ends_with(".monitor"),
                "a monitor of a sink is not an input anybody can speak into"
            );
        }
        for camera in &list.cameras {
            assert!(
                !camera.id.is_empty(),
                "a camera has an id: {}",
                camera.label
            );
        }
    }
}

/// Group call offers and rosters as the library hands them over, for the tests here and the
/// worker's.
#[cfg(test)]
pub(crate) mod test_offers {
    use super::*;

    pub const CALL_ID: &str = "3EB0CALLID";
    pub const CREATOR: &str = "200000000000001@lid";

    pub const GROUP: &str = "120363000000000001@g.us";

    /// One member on a group call's roster, by privacy id, joined or only asked.
    pub fn member(lid: &str, joined: bool) -> GroupCallParticipant {
        let mut person =
            GroupCallParticipant::new(format!("{lid}@lid").parse().unwrap(), Vec::new());
        person.state = joined.then(|| "connected".to_owned());
        person
    }

    pub fn roster(transaction: u32, participants: Vec<GroupCallParticipant>) -> GroupCallUpdate {
        GroupCallUpdate::builder()
            .call_id(CALL_ID.to_owned())
            .call_creator(CREATOR.parse::<Jid>().unwrap())
            .group_jid(GROUP.parse().unwrap())
            .transaction_id(transaction)
            .media("audio".to_owned())
            .connected_limit(32)
            .joinable(true)
            .av_upgradable(false)
            .rekey_requested(false)
            .participants(participants)
            .build()
    }

    /// A group's offer as it rings here: sent by the member who started the call, naming the
    /// group and carrying the roster.
    pub fn group_offer(participants: Vec<GroupCallParticipant>) -> IncomingCall {
        IncomingCall::builder()
            .from("200000000000001:3@lid".parse().unwrap())
            .stanza_id("STANZA".to_owned())
            .offline(false)
            .action(CallAction::Offer {
                call_id: CALL_ID.to_owned(),
                call_creator: CREATOR.parse::<Jid>().unwrap(),
                caller_pn: None,
                caller_country_code: None,
                device_class: None,
                joinable: true,
                is_video: false,
                audio: Vec::new(),
                group_jid: Some(GROUP.parse().unwrap()),
            })
            .group(Box::new(roster(1, participants)))
            .build()
    }
}
