//! 1:1 WhatsApp calling: signaling, media, audio devices, camera, and state.
//!
//! Audio goes through rodio, the same layer the rest of the app plays and records with, so a call
//! opens the platform's own audio API on every platform it ships for rather than shelling out to
//! `pw-record` and `pw-play`. The engine's 16 kHz mono frames meet whatever rate and channel count a
//! device wants inside [`crate::call_audio`], and each direction sits behind a *stable* channel, so
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

use crate::model::{CallDirection, CallMedia, CallRecord, CallStatus};
use whatsapp_rust::prelude::{Client, Jid};
use whatsapp_rust::types::call::{CallAction, IncomingCall};
use whatsapp_rust::voip::{
    CallEvent, CallHandle, TimedVideoFrame, VideoFrame, VideoSink, VideoSource,
};

/// The pixel budget one camera session encodes within: a landscape frame at most 640 by 360, a
/// portrait one at most 360 by 640.
///
/// Following the camera's own aspect is what keeps a portrait camera portrait on the peer's screen
/// and in the corner preview. Stretching it into one fixed landscape frame would either pad it into
/// a letterbox or squash it, and neither is what the camera saw.
const CAPTURE_LONG_SIDE: usize = 640;
const CAPTURE_SHORT_SIDE: usize = 360;
/// The largest the corner preview may be, in the same orientation as the capture.
const PREVIEW_LONG_SIDE: usize = 320;
const PREVIEW_SHORT_SIDE: usize = 180;
const VIDEO_FPS: u32 = 15;
/// RTP video clock (90 kHz) divided by the capture cadence.
const VIDEO_TS_STRIDE: u32 = 90_000 / VIDEO_FPS;
const VIDEO_BITRATE: u32 = 1_200_000;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Where a 1:1 call is in its life.
///
/// Every value except the terminal ones comes from the backend: the offer that was sent or
/// received, the peer's `<accept>`, and the media plane reporting itself live. Nothing is inferred
/// from the button that started the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallPhase {
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
            Self::Dialing
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

/// How a call turned out, as data rather than as a finished sentence.
///
/// The words belong to the call screen, which is the only place that knows the reader's language,
/// and the call history stores the same value under a stable status. Nothing here is guessed from
/// the button that was pressed: each one comes from the peer's signaling or from the media plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallOutcome {
    /// The two sides were connected, so the length in the record is real.
    Answered,
    /// An incoming call nobody here picked up.
    Missed,
    /// The peer rejected it, or we did.
    Declined,
    /// The peer's phone was already on a call.
    Busy,
    /// It never came up, and not for any of the reasons above.
    Failed,
    /// Outgoing, and nobody answered before the call gave up.
    NoAnswer,
    /// The media plane went away under a call that was up.
    ConnectionLost,
    /// Another of this account's devices took it.
    AnsweredElsewhere,
    /// Another of this account's devices rejected it.
    DeclinedElsewhere,
}

impl CallOutcome {
    /// The status the call log stores for this outcome. One call, one status: the log keeps the
    /// peer's answer rather than the sequence of stanzas that led to it.
    pub fn status(self) -> CallStatus {
        match self {
            Self::Answered => CallStatus::Answered,
            // Answered here? No: another device took it, so this device has no length to record.
            Self::AnsweredElsewhere => CallStatus::AnsweredElsewhere,
            Self::Missed => CallStatus::Missed,
            Self::Declined | Self::DeclinedElsewhere => CallStatus::Declined,
            Self::Busy => CallStatus::Busy,
            Self::Failed => CallStatus::Failed,
            Self::NoAnswer => CallStatus::NoAnswer,
            Self::ConnectionLost => CallStatus::ConnectionLost,
        }
    }
}

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
}

/// A microphone or speaker PipeWire knows about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioDevice {
    /// The PipeWire `node.name`, used verbatim as `--target`.
    pub id: String,
    /// The `node.description`, which is what a person recognises.
    pub label: String,
}

/// A camera V4L2 knows about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CameraDevice {
    /// The capture node, such as `/dev/video0`.
    pub id: String,
    /// The name the driver reports.
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

/// The devices a call should open with, as the settings hold them.
#[derive(Clone, Debug, Default)]
pub struct CallDevices {
    pub microphone: Option<String>,
    pub speaker: Option<String>,
    pub camera: Option<String>,
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
    let camera = checked(DeviceKind::Camera, camera, &cameras, &mut lost);

    ResolvedDevices {
        microphone,
        speaker,
        camera,
        lost_devices: lost,
    }
}

/// Whether the call surface may offer 1:1 screen sharing.
///
/// It may not, at the revision this app pins, so the button says so instead of pretending.
/// `CallHandle::start_screen_share` reaches `Voip::set_screen_share_for_generation`, which requires
/// `CallRegistry::group_creator_matches_if_current` — an *active group* call (`entry.is_group_call`)
/// whose creator matches. A direct 1:1 call has no group state, so the call is refused with
/// `CallError::Media("call creator does not match the active group call")`. The protocol crate
/// carries `<screen_share>` for group calls only, so a Wayland portal capture would produce frames
/// with no 1:1 signaling to carry them.
pub const fn screen_share_supported() -> bool {
    false
}

/// Scales `size` to the largest even size with the same aspect that fits `bounds`, never upscaling.
///
/// Even on both sides because `yuv420p` has no half rows or columns, and never upscaled because a
/// small camera gains nothing but bytes and latency from being blown up.
fn fit_even(size: (u32, u32), bounds: (usize, usize)) -> (usize, usize) {
    if size.0 == 0 || size.1 == 0 {
        return bounds;
    }
    let scale = (bounds.0 as f64 / f64::from(size.0))
        .min(bounds.1 as f64 / f64::from(size.1))
        .min(1.0);
    let even = |value: f64| (((value.round() as usize) / 2) * 2).max(2);
    (
        even(f64::from(size.0) * scale),
        even(f64::from(size.1) * scale),
    )
}

/// The frame size one camera session encodes at, given the format the camera reports.
///
/// A portrait camera is encoded portrait over the same pixel budget (360 by 640 rather than 640 by
/// 360), so the peer receives a picture whose shape is the shape the camera saw: nothing cropped,
/// nothing stretched, and no rotation metadata needed, because the frames really are upright. A
/// camera that will not say what it has keeps the landscape default, which is what every camera
/// this app has met so far reports.
pub fn capture_size(native: Option<(u32, u32)>) -> (usize, usize) {
    let bounds = match native {
        Some((width, height)) if height > width => (CAPTURE_SHORT_SIDE, CAPTURE_LONG_SIDE),
        _ => (CAPTURE_LONG_SIDE, CAPTURE_SHORT_SIDE),
    };
    match native {
        Some(size) => fit_even(size, bounds),
        None => bounds,
    }
}

/// The corner preview's size for a capture of `capture`, in the capture's own orientation.
fn preview_size(capture: (usize, usize)) -> (usize, usize) {
    let bounds = if capture.1 > capture.0 {
        (PREVIEW_SHORT_SIDE, PREVIEW_LONG_SIDE)
    } else {
        (PREVIEW_LONG_SIDE, PREVIEW_SHORT_SIDE)
    };
    fit_even((capture.0 as u32, capture.1 as u32), bounds)
}

/// The capture format a camera reports, as `(width, height)`, straight from the driver.
///
/// A node that will not answer, which is every metadata node, leaves the default, so the encoder
/// runs at the budget's own shape.
fn native_format(device: &str) -> Option<(u32, u32)> {
    crate::camera::current_size(device)
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
// Platform capabilities
// ---------------------------------------------------------------------------

/// What calling can actually do on the platform this build runs on.
///
/// Voice goes through rodio, which opens the platform's own audio API wherever the app builds, so
/// voice is offered everywhere. Video is V4L2 camera capture with an `ffmpeg` fallback, which is
/// Linux only as it stands, and offering the camera button elsewhere would only lead to a call that
/// fails on its first frame. The interface asks this, and a platform without a backend for a
/// feature does not offer it: the chat header shows no camera button and the camera picker is never
/// reached. `cfg!` rather than `#[cfg]` keeps every arm type-checked on every platform, so a
/// platform that gains a backend cannot drift out of sync with the interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallCapabilities {
    /// One-to-one voice calls: a microphone and a speaker.
    pub voice: bool,
    /// Camera video inside a call.
    pub video: bool,
    /// Choosing between cameras, which needs a camera list at all.
    pub camera: bool,
    /// Sharing a screen in a one-to-one call. The pinned protocol library carries screen sharing
    /// for group calls only, so this is false everywhere for now.
    pub screen_share: bool,
}

/// What the media backend behind [`CallPhase`] can do on this platform.
pub fn capabilities() -> CallCapabilities {
    CallCapabilities {
        // rodio opens PipeWire, CoreAudio or WASAPI, so a call can carry voice wherever the app
        // builds. Whether this machine has a microphone at all is asked when a call is placed,
        // because that is a fact about the machine rather than about the platform.
        voice: true,
        video: crate::camera::available(),
        camera: crate::camera::available(),
        screen_share: false,
    }
}

// ---------------------------------------------------------------------------
// Device discovery
// ---------------------------------------------------------------------------

/// Lists microphones, speakers and cameras for the call screen.
///
/// The microphones and the speakers come from rodio, the same layer a call opens them through, so
/// a picker offers what a call can actually take and the names it reports are what the saved
/// setting holds. On a platform whose call backend is not built, the lists are empty rather than a
/// list nothing can use, and the pickers are never reached.
pub fn devices() -> DeviceList {
    let capable = capabilities();
    DeviceList {
        microphones: if capable.voice {
            audio_devices(crate::call_audio::microphones())
        } else {
            Vec::new()
        },
        speakers: if capable.voice {
            audio_devices(crate::call_audio::speakers())
        } else {
            Vec::new()
        },
        cameras: if capable.camera {
            cameras()
        } else {
            Vec::new()
        },
    }
}

/// Wraps the names rodio reports as the call screen's device entries.
fn audio_devices(found: Vec<(String, String)>) -> Vec<AudioDevice> {
    found
        .into_iter()
        .map(|(id, label)| AudioDevice { id, label })
        .collect()
}

/// Lists the cameras the platform reports.
///
/// The nodes come from the camera module, which asks each one what it is instead of parsing the
/// output of an external tool, so a machine without `v4l2-ctl` still offers its camera. A camera
/// exposes several nodes and only one of them captures: the metadata node reports capture but
/// enumerates no usable format, and the module drops it.
fn cameras() -> Vec<CameraDevice> {
    crate::camera::cameras()
        .into_iter()
        .map(|(id, label)| CameraDevice { id, label })
        .collect()
}

/// Whether a node can deliver frames, which is what makes it a capture node.
fn captures(node: &str) -> bool {
    crate::camera::is_capture(node)
}

// ---------------------------------------------------------------------------
// Audio
// ---------------------------------------------------------------------------

/// The engine's audio clock, and the microphone and the speaker the engine talks to.
///
/// Both live in [`crate::call_audio`], which opens them through rodio the way the rest of the app
/// already plays and records: the platform's own audio API on every platform it ships for, with no
/// `pw-record` or `pw-play` needed for a call to exist. A device change replaces only the reader or
/// the writer behind the engine's channel, so nothing about the call's codec or stream state is
/// rebuilt for it.
pub use crate::call_audio::{AudioInput, AudioOutput, RATE};

/// What a call still cannot open on this machine, if anything.
fn missing_audio_device() -> Option<&'static str> {
    crate::call_audio::unavailable()
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
    /// The peer's picture.
    Remote(Arc<ColorImage>),
}

/// The camera side of the call: the camera module reads YUV 4:2:0 at a fixed size, so the encoder
/// and the preview both have a known layout, and the frames the engine wants are complete H.264
/// Annex-B access units led by an access-unit delimiter.
///
/// A device change starts a fresh thread rather than telling this one to switch: a running encoder
/// carries state the new device's stream cannot inherit.
struct CameraCapture {
    timed: async_channel::Receiver<TimedVideoFrame>,
    /// Cleared when the camera is no longer wanted, and read by the capture loop between reads, so
    /// a stopped camera ends without waiting for a frame that may never come.
    running: Arc<AtomicBool>,
    /// The capture child, shared with the camera module that owns it. On the fallback path the
    /// read is a blocking `read_exact` on the child's stdout, so ending it without a frame means
    /// killing the process, which closes that pipe and returns the thread. Without this, a camera
    /// or `ffmpeg` that stalled while holding the device would keep the read, and the process,
    /// alive past [`VideoPipeline::shutdown`], and the next call would race it for the node.
    child: Arc<std::sync::Mutex<Option<std::process::Child>>>,
}

impl CameraCapture {
    fn start(device: Option<String>, ticks: async_channel::Sender<VideoTick>) -> Result<Self> {
        // Refuse a camera that cannot be opened before any signaling, rather than spawning a
        // thread that exits and sends a video call with no local picture: `capture` treats a
        // missing device as "no camera" and returns, which would otherwise pass silently.
        let Some(device) = device else {
            return Err(anyhow!("no camera is available"));
        };
        // A node that enumerates no pixel format is not a capture device (a metadata node beside a
        // camera reports capture but lists nothing); starting on it would leave the peer with an
        // empty video stream.
        if !captures(&device) {
            return Err(anyhow!("{device} is not a usable camera"));
        }
        let (frames, timed) = async_channel::bounded::<TimedVideoFrame>(4);
        let running = Arc::new(AtomicBool::new(true));
        let child = Arc::new(std::sync::Mutex::new(None));
        let alive = CaptureAlive(Arc::clone(&running));
        let slot = Arc::clone(&child);
        let stopping = Arc::clone(&running);
        std::thread::Builder::new()
            .name("zapfast-camera".to_owned())
            .spawn(move || {
                let _alive = alive;
                capture(Some(device), frames, ticks, slot, stopping);
            })
            .context("camera thread could not be started")?;
        Ok(Self {
            timed,
            running,
            child,
        })
    }

    /// Ends the camera. No frame has to arrive for the thread to stop: the flag is cleared, which
    /// is what the capture loop reads between reads, and the child, if there is one, is killed so a
    /// read waiting on its stdout returns.
    fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
        kill_camera_child(&self.child);
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

/// Kills and reaps the camera child, whichever side still holds it.
///
/// Taking it out of the slot first is what lets the capture thread and its caller both ask to end
/// the camera: neither kills a pid twice, and the one that finds an empty slot can still reap
/// nothing and move on.
fn kill_camera_child(slot: &std::sync::Mutex<Option<std::process::Child>>) {
    let child = slot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
    if let Some(mut child) = child {
        let _ = child.kill();
        let _ = child.wait();
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
}

impl VideoPipeline {
    /// Starts the camera and the peer's decoder. The returned receiver carries both directions'
    /// frames to the UI.
    pub fn start(device: Option<String>) -> Result<(Self, async_channel::Receiver<VideoTick>)> {
        let (ticks, tick_rx) = async_channel::bounded::<VideoTick>(2);
        let (remote_tx, remote_rx) = async_channel::bounded::<VideoFrame>(8);
        decode_remote(remote_rx, ticks.clone());
        let camera = CameraCapture::start(device.clone(), ticks.clone())?;
        Ok((
            Self {
                ticks,
                remote_tx,
                camera: Some(camera),
                device,
            },
            tick_rx,
        ))
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
        if let Some(camera) = self.camera.take() {
            camera.stop();
        }
    }

    /// Starts capturing from `device` again, reusing the frame channel the UI already drains.
    pub fn resume_camera(&mut self, device: Option<String>) -> Result<()> {
        if let Some(camera) = self.camera.take() {
            camera.stop();
        }
        self.device = device.clone();
        self.camera = Some(CameraCapture::start(device, self.ticks.clone())?);
        Ok(())
    }

    /// Whether the camera is capturing right now. A capture thread that ended — an unplugged
    /// camera, a dead `ffmpeg` — reads as off, so the frame channel and the controls agree.
    pub fn camera_running(&self) -> bool {
        self.camera.as_ref().is_some_and(CameraCapture::running)
    }

    /// Ends the video direction: the capture thread stops and the child is reaped.
    pub fn shutdown(&mut self) {
        if let Some(camera) = self.camera.take() {
            camera.stop();
        }
    }
}

/// Captures from the camera, encodes each frame to an Annex-B access unit, and feeds the preview.
///
/// Runs on its own thread: the `openh264` encoder is not shared between threads and the camera read
/// is blocking. Everything it hands out goes through a bounded channel with `try_send`, so a
/// stalled consumer drops a frame instead of stalling the camera.
fn capture(
    device: Option<String>,
    timed: async_channel::Sender<TimedVideoFrame>,
    ticks: async_channel::Sender<VideoTick>,
    child: Arc<std::sync::Mutex<Option<std::process::Child>>>,
    stopping: Arc<AtomicBool>,
) {
    use openh264::encoder::{
        BitRate, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, Profile,
    };

    let Some(device) = device else {
        log::warn!("[CALL] no camera is available");
        return;
    };
    // The camera's own shape decides the frame the encoder runs at: the driver is offered the
    // capture budget and the encoder is built for whatever it grants, so a portrait camera is
    // encoded portrait and nothing is scaled or cropped.
    let budget = capture_size(native_format(&device));
    // The node is read first and `ffmpeg` only fills in, so a camera that is readable directly
    // never starts a process.
    let mut source = match crate::camera::Source::open(&device, budget, child) {
        Ok(source) => source,
        Err(error) => {
            log::error!("[CALL] camera capture could not start: {error}");
            return;
        }
    };
    let size = source.size();
    let config = EncoderConfig::new()
        .profile(Profile::Baseline)
        .bitrate(BitRate::from_bps(VIDEO_BITRATE))
        .max_frame_rate(FrameRate::from_hz(VIDEO_FPS as f32))
        .intra_frame_period(IntraFramePeriod::from_num_frames(VIDEO_FPS * 2))
        .skip_frames(false);
    let mut encoder = match Encoder::with_api_config(openh264::OpenH264API::from_source(), config) {
        Ok(encoder) => encoder,
        Err(error) => {
            log::error!("[CALL] camera encoder could not start: {error}");
            source.stop();
            source.reap();
            ticks.close();
            return;
        }
    };

    capture_frames(
        &mut source,
        size,
        &stopping,
        &timed,
        &ticks,
        &mut encoder,
        Instant::now(),
    );
    // Whatever ended the loop, the source is done with and its child, if it started one, is reaped
    // rather than left for the next call to race.
    source.reap();
}

/// Reads frames from one camera until the call stops it, encoding and previewing each one.
///
/// Separate from [`capture`] because the encoder is built once per camera session and the source
/// owns the device: a device change starts a new thread with a new encoder rather than handing a
/// mid-stream encoder to another device's frames. `size` is what the source delivers, which is the
/// size the driver granted, so nothing here scales or crops.
fn capture_frames(
    source: &mut crate::camera::Source,
    size: (usize, usize),
    stopping: &AtomicBool,
    timed: &async_channel::Sender<TimedVideoFrame>,
    ticks: &async_channel::Sender<VideoTick>,
    encoder: &mut openh264::encoder::Encoder,
    started: Instant,
) {
    let (width, height) = size;
    let (luma_len, chroma_len) = (width * height, width * height / 4);
    let mut bytes = vec![0u8; luma_len + chroma_len * 2];
    log::info!("[CALL] camera enabled size={width}x{height}");
    loop {
        // Checked before each read, not only between them: a read returns every poll window even
        // when the camera has no frame for it, so a stop is honoured while the device is held.
        if !stopping.load(Ordering::Relaxed) {
            source.stop();
            log::info!("[CALL] camera disabled");
            return;
        }
        match source.read(&mut bytes) {
            crate::camera::Read::Frame => {}
            // Nothing this poll: go round and re-check whether the camera is still wanted.
            crate::camera::Read::Idle => continue,
            crate::camera::Read::Ended => {
                log::warn!("[CALL] the camera stopped delivering frames");
                return;
            }
        }

        let luma = &bytes[..luma_len];
        let chroma_u = &bytes[luma_len..luma_len + chroma_len];
        let chroma_v = &bytes[luma_len + chroma_len..];
        let planes = Yuv420 {
            y: luma,
            u: chroma_u,
            v: chroma_v,
            size,
        };
        match encoder.encode(&planes) {
            Ok(stream) => {
                // One access unit per encoded frame, led by an access-unit delimiter so the
                // peer's depacketizer sees the boundaries an encoder would have produced.
                let mut unit = vec![0x00, 0x00, 0x00, 0x01, 0x09, 0xF0];
                stream.write_vec(&mut unit);
                let elapsed = started.elapsed().as_micros() as u64;
                let timestamp = (elapsed * 90 / 1000) as u32;
                // A full queue means the wire is behind: dropping is the documented answer for
                // video, and the next frame carries the picture forward anyway.
                let _ = timed.try_send(
                    TimedVideoFrame::builder()
                        .data(unit)
                        .timestamp(timestamp)
                        .build(),
                );
            }
            Err(error) => log::warn!("[CALL] camera frame could not be encoded: {error}"),
        }

        let image = crate::video::rgb_image(
            luma,
            chroma_u,
            chroma_v,
            (width, width / 2, width / 2),
            (width, height),
            preview_size(size),
            // Our own frames arrive upright from the camera: there is no rotation to undo, and
            // none is announced either.
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
/// keyframe. Runs on its own thread for the same reason the encoder does.
fn decode_remote(
    frames: async_channel::Receiver<VideoFrame>,
    ticks: async_channel::Sender<VideoTick>,
) {
    let spawned = std::thread::Builder::new()
        .name("zapfast-remote-video".to_owned())
        .spawn(move || {
            use openh264::formats::YUVSource as _;

            let mut decoder = openh264::decoder::Decoder::new().ok();
            let mut started = false;
            let mut last_orientation = 0u8;
            while let Ok(frame) = frames.recv_blocking() {
                if !started {
                    if !frame.keyframe {
                        continue;
                    }
                    started = true;
                }
                let Some(decoder) = decoder.as_mut() else {
                    return;
                };
                // The peer's camera rotation, which the frames carry so a phone held upright arrives
                // upright. It rides the same bits on a keyframe and on a delta frame, and a peer
                // that turns its phone changes it mid-call, so it is read per frame rather than once.
                let orientation = frame.orientation & 0x03;
                if orientation != last_orientation {
                    last_orientation = orientation;
                    log::info!(
                        "[CALL] remote camera orientation is {} degrees",
                        orientation as u32 * 90
                    );
                }
                let turns = crate::video::orientation_turns(orientation);
                match decoder.decode(&frame.data) {
                    Ok(Some(decoded)) => {
                        let (width, height) = decoded.dimensions();
                        if width == 0 || height == 0 {
                            continue;
                        }
                        let image = crate::video::rgb_image(
                            decoded.y(),
                            decoded.u(),
                            decoded.v(),
                            decoded.strides(),
                            (width, height),
                            (width, height),
                            turns,
                        );
                        let _ = ticks.try_send(VideoTick::Remote(Arc::new(image)));
                    }
                    Ok(None) => {}
                    Err(error) => {
                        log::debug!("[CALL] remote video frame was not decodable: {error}")
                    }
                }
            }
        });
    if let Err(error) = spawned {
        log::error!("[CALL] remote video decoder could not start: {error}");
    }
}

// ---------------------------------------------------------------------------
// The call
// ---------------------------------------------------------------------------

/// Counter behind [`CallUpdate::generation`].
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn next_generation() -> u64 {
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
    /// When this call reached this computer, on the wall clock, which is what the log stores. The
    /// monotonic [`Call::started`] cannot be written down: it means nothing once the process exits.
    began: std::time::SystemTime,
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
}

impl Call {
    /// Places an outgoing 1:1 call. `video` offers video from the first frame, which is what makes
    /// the peer's phone ring as a video call.
    pub async fn place(
        client: &Arc<Client>,
        chat: String,
        video: bool,
        microphone: Option<String>,
        speaker: Option<String>,
        camera: Option<String>,
    ) -> Result<(Self, Option<async_channel::Receiver<VideoTick>>)> {
        let peer: Jid = chat
            .parse()
            .map_err(|error| anyhow!("not a WhatsApp JID: {error}"))?;
        if !capabilities().voice {
            return Err(anyhow!("calling is not available on this platform yet"));
        }
        if let Some(missing) = missing_audio_device() {
            return Err(anyhow!("no {missing} is available; a call needs one"));
        }
        // Video first: a camera or encoder that will not start refuses the call before any stream
        // exists and before the peer is rung, because a video call must not silently become voice.
        let mut pipe = None;
        if video {
            match VideoPipeline::start(camera.clone()) {
                Ok((pipeline, frames)) => pipe = Some((pipeline, frames)),
                Err(error) => log::warn!("[CALL] video pipeline could not start: {error}"),
            }
        }
        require_video(video, pipe.is_some())?;
        let (mic, mic_rx) = AudioInput::spawn(microphone.clone());
        let (output, output_tx) = AudioOutput::spawn(speaker.clone());

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
                generation: next_generation(),
                call_id: handle.call_id().to_owned(),
                chat,
                began: std::time::SystemTime::now(),
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
                video_pipe,
                remote_video: false,
                microphone,
                speaker_device: speaker,
                camera,
                lost_devices: Vec::new(),
            },
            frames,
        ))
    }

    /// Records an incoming offer so the UI can ask the user. No media exists until they answer.
    ///
    /// The remembered devices are pre-selected on the prompt, so answering opens the same
    /// microphone, speaker and camera the last call used rather than the system defaults.
    pub fn ringing(
        chat: String,
        incoming: Box<IncomingCall>,
        video: bool,
        devices: CallDevices,
    ) -> Self {
        let call_id = incoming.action.call_id().to_owned();
        log::info!("[CALL] incoming offer call_id={call_id} video={video}");
        Self {
            generation: next_generation(),
            call_id,
            chat,
            began: std::time::SystemTime::now(),
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
            microphone: devices.microphone,
            speaker_device: devices.speaker,
            camera: devices.camera,
            lost_devices: Vec::new(),
            camera_wanted: false,
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
            outcome: self.outcome,
            peer_audio: self.peer_audio,
            lost_devices: self.lost_devices.clone(),
            microphone: self.microphone.clone(),
            speaker: self.speaker_device.clone(),
            camera: self.camera.clone(),
        }
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
        // with no media backend would be worse than a refused one, because the other side's call is
        // already up.
        if !capabilities().voice {
            return Err(anyhow!("calling is not available on this platform yet"));
        }
        if let Some(missing) = missing_audio_device() {
            return Err(anyhow!("no {missing} is available; a call needs one"));
        }
        // The same rule as placing a call: a video offer that cannot start video is not accepted as
        // something else behind the user's back. Nothing has been sent yet, so the call stays
        // ringing and the user can still decline it.
        let mut pipe = None;
        if self.video {
            match VideoPipeline::start(camera.clone()) {
                Ok((pipeline, frames)) => pipe = Some((pipeline, frames)),
                Err(error) => log::warn!("[CALL] video pipeline could not start: {error}"),
            }
        }
        require_video(self.video, pipe.is_some())?;
        let (mic, mic_rx) = AudioInput::spawn(microphone.clone());
        let (output, output_tx) = AudioOutput::spawn(speaker.clone());

        let voip = client.voip();
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
        self.incoming = None;
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
            _ => None,
        }
    }

    /// Reads the engine's media counters into the log, so a call that is up but not carrying audio
    /// can be diagnosed from a bug report rather than guessed at. Called from the call heartbeat.
    pub fn log_media_stats(&self) {
        let Some(handle) = self.handle.as_ref() else {
            return;
        };
        let stats = handle.media_stats();
        log::info!(
            "[CALL] media stats call_id={} rtp_received={} unexpected_pt={} srtp_failed={} decoded={} delivered={} concealed={} without_decoder={} sink_dropped={} trimmed={} codec_switches={} video_sink_dropped={}",
            self.call_id,
            stats.rtp_received,
            stats.rtp_payload_type_unexpected,
            stats.srtp_unprotect_failed,
            stats.audio_frames_decoded,
            stats.audio_frames_delivered,
            stats.audio_frames_concealed,
            stats.audio_frames_without_decoder,
            stats.audio_sink_dropped,
            stats.playout_trimmed_samples,
            stats.codec_switches,
            stats.video_sink_dropped,
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

    /// The call was resolved on another of the account's devices, or the caller gave up.
    ///
    /// A call we were ringing for that was answered or declined elsewhere is not the same thing as
    /// an unanswered one, so the two cases get their own words while both release the media.
    pub fn resolved_elsewhere(&mut self) -> Option<CallUpdate> {
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

    /// Turns the camera on or off through the engine's video direction.
    pub async fn set_camera(&mut self, on: bool) -> Result<CallUpdate> {
        let Some(handle) = self.handle.clone() else {
            return Err(anyhow!("the call is not up"));
        };
        if on {
            let Some(pipe) = self.video_pipe.as_mut() else {
                return Err(anyhow!("this call has no video"));
            };
            pipe.resume_camera(self.camera.clone())?;
            let (source, sink) = (pipe.source(), pipe.sink());
            handle
                .resume_video(source, sink)
                .await
                .map_err(|error| anyhow!("the camera could not be enabled: {error}"))?;
            self.camera_wanted = true;
        } else {
            handle
                .stop_video()
                .await
                .map_err(|error| anyhow!("the camera could not be stopped: {error}"))?;
            if let Some(pipe) = self.video_pipe.as_mut() {
                pipe.pause_camera();
            }
            self.camera_wanted = false;
        }
        log::info!("[CALL] camera enabled={on}");
        Ok(self.update())
    }

    /// Asks the peer for video on a call that started as voice, or turns the camera back on when
    /// this is already a video call.
    ///
    /// The upgrade is the real one: `CallHandle::start_video` attaches the endpoints, enables the
    /// media plane and sends `<video state=11>`, and the peer's answer arrives as video state
    /// signaling. Returns the frame channel the UI should start draining.
    pub async fn upgrade_to_video(
        &mut self,
        camera: Option<String>,
    ) -> Result<Option<async_channel::Receiver<VideoTick>>> {
        let Some(handle) = self.handle.clone() else {
            return Err(anyhow!("the call is not up"));
        };
        if self.video_pipe.is_none() {
            let (pipe, frames) = VideoPipeline::start(camera.clone())?;
            self.camera_wanted = pipe.camera_running();
            self.video_pipe = Some(pipe);
            self.camera = camera;
            self.video = true;
            let (source, sink) = {
                let pipe = self.video_pipe.as_ref().expect("just installed");
                (pipe.source(), pipe.sink())
            };
            handle
                .start_video(source, sink)
                .await
                .map_err(|error| anyhow!("the peer could not be asked for video: {error}"))?;
            log::info!("[CALL] video upgrade requested call_id={}", self.call_id);
            return Ok(Some(frames));
        }
        self.set_camera(true).await?;
        Ok(None)
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
    pub fn set_camera_device(&mut self, device: Option<String>) -> Result<CallUpdate> {
        self.camera = device.clone();
        self.lost_devices.clear();
        if let Some(pipe) = self.video_pipe.as_mut()
            && pipe.camera_running()
        {
            pipe.resume_camera(device)?;
        }
        log::info!("[CALL] camera device {:?}", self.camera);
        Ok(self.update())
    }

    /// The archived record of a call that has finished, or `None` while it is still going.
    ///
    /// Every field comes from the call itself: which side placed it, what it carried, what the peer
    /// and the media plane said became of it, and how long the two sides were really connected. The
    /// length counts from the moment the call became active, so a call that never connected records
    /// zero rather than the time spent ringing.
    pub fn record(&self) -> Option<CallRecord> {
        let outcome = self.outcome?;
        Some(CallRecord {
            id: self.call_id.clone(),
            chat: self.chat.clone(),
            started_at: unix_seconds(self.began),
            ended_at: unix_seconds(std::time::SystemTime::now()),
            direction: self.direction,
            media: if self.video {
                CallMedia::Video
            } else {
                CallMedia::Voice
            },
            status: outcome.status(),
            duration: self
                .started
                .map(|started| started.elapsed().as_secs())
                .unwrap_or(0),
        })
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
}

/// Whole seconds since the Unix epoch. A clock set before 1970 reads as zero rather than as a
/// negative timestamp, which no query and no layout expects.
fn unix_seconds(at: std::time::SystemTime) -> i64 {
    at.duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0)
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
fn termination_outcome(reason: Option<&str>, connected: bool) -> CallOutcome {
    match reason {
        Some("accepted_elsewhere") => CallOutcome::AnsweredElsewhere,
        Some("rejected_elsewhere") => CallOutcome::DeclinedElsewhere,
        Some("timeout") => CallOutcome::NoAnswer,
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
                id: "/dev/video0".to_owned(),
                label: "Laptop Camera".to_owned(),
            }],
        }
    }

    /// The pickers and the backend agree: a platform that can carry voice lists the devices rodio
    /// reports, and a platform that cannot lists none rather than a list nothing can open.
    #[test]
    fn the_audio_pickers_follow_the_voice_capability() {
        let listed = super::devices();
        if super::capabilities().voice {
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
        } else {
            assert!(listed.microphones.is_empty());
            assert!(listed.speakers.is_empty());
        }
        assert_eq!(
            listed.cameras.is_empty(),
            !super::capabilities().camera,
            "the camera list follows the camera capability"
        );
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
            outcome: None,
            peer_audio: None,
            lost_devices: Vec::new(),
            microphone: None,
            speaker: None,
            camera: None,
        }
    }

    #[test]
    fn the_devices_a_person_picked_are_the_ones_a_call_opens_with() {
        let machine = machine();
        let resolved = resolve_devices(
            &machine,
            Some(machine.microphones[0].id.clone()),
            Some(machine.speakers[0].id.clone()),
            Some("/dev/video0".to_owned()),
        );
        assert_eq!(resolved.microphone, Some(machine.microphones[0].id.clone()));
        assert_eq!(resolved.speaker, Some(machine.speakers[0].id.clone()));
        assert_eq!(resolved.camera.as_deref(), Some("/dev/video0"));
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
            Some("/dev/video7".to_owned()),
        );
        assert_eq!(
            resolved.microphone, None,
            "the microphone moves to the default"
        );
        assert_eq!(resolved.speaker, None, "and so does the speaker");
        assert_eq!(
            resolved.camera, None,
            "a camera the machine does not have is not opened"
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
                    name: "/dev/video7".to_owned(),
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
            Some("/dev/video0".to_owned()),
        );
        assert_eq!(resolved.microphone.as_deref(), Some("bluez_input.AC"));
        assert_eq!(resolved.speaker.as_deref(), Some("bluez_output.AC"));
        assert_eq!(resolved.camera.as_deref(), Some("/dev/video0"));
        assert!(
            resolved.lost_devices.is_empty(),
            "a device list that could not be read says nothing about the devices"
        );
    }

    #[test]
    fn a_device_is_named_the_way_the_picker_named_it() {
        let machine = machine();
        assert_eq!(
            device_name(&machine, DeviceKind::Camera, "/dev/video0"),
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
        for phase in [CallPhase::Dialing, CallPhase::Ringing, CallPhase::Incoming] {
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
        // A plain terminate means the peer ended a call that was up, or gave up before it was.
        assert_eq!(termination_outcome(None, true), CallOutcome::Answered);
        assert_eq!(termination_outcome(None, false), CallOutcome::NoAnswer);
    }

    #[test]
    fn a_portrait_camera_is_encoded_portrait() {
        // The shape the camera reports is the shape the peer receives, within the same pixel
        // budget: 640 by 360 across, 360 by 640 upright.
        assert_eq!(capture_size(Some((1280, 720))), (640, 360));
        assert_eq!(capture_size(Some((1920, 1080))), (640, 360));
        assert_eq!(capture_size(Some((1920, 960))), (640, 320));
        assert_eq!(capture_size(Some((640, 480))), (480, 360));
        assert_eq!(capture_size(Some((480, 640))), (360, 480));
        assert_eq!(capture_size(Some((1080, 1920))), (360, 640));
        // A small camera is not blown up: that costs bytes and latency for nothing.
        assert_eq!(capture_size(Some((320, 240))), (320, 240));
        // Nothing known about the camera keeps the landscape default rather than guessing.
        assert_eq!(capture_size(None), (640, 360));
        assert_eq!(capture_size(Some((0, 0))), (640, 360));
    }

    #[test]
    fn the_corner_preview_keeps_the_capture_shape() {
        assert_eq!(preview_size((640, 360)), (320, 180));
        assert_eq!(preview_size((480, 360)), (240, 180));
        assert_eq!(preview_size((360, 640)), (180, 320));
    }

    #[test]
    fn a_camera_that_cannot_be_opened_is_refused() {
        // The camera module decides how to read a device and says when it cannot; a call built on a
        // device that is not a camera would otherwise start with no picture and no reason shown.
        let child = Arc::new(std::sync::Mutex::new(None));
        assert!(crate::camera::Source::open("/dev/null", (640, 360), child).is_err());
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
            began: std::time::SystemTime::now(),
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
            assert_eq!(
                call.record()
                    .expect("a finished call has a record")
                    .duration,
                0,
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
        assert_eq!(
            call.record().expect("a record").status,
            CallStatus::ConnectionLost
        );
    }

    #[test]
    fn a_call_that_had_been_up_records_who_hung_up_and_how_long_it_lasted() {
        let mut call = dialing();
        call.media(&CallEvent::RelayAllocated);
        call.signaling(&accept()).expect("the peer answered");
        // Backdate the start: the record's length comes from the monotonic clock, and no test
        // should have to sleep for a minute to prove that a minute is measured.
        call.started = Some(Instant::now() - std::time::Duration::from_secs(221));
        let update = call.signaling(&terminate(None)).expect("the peer hung up");
        assert_eq!(update.phase, CallPhase::Ended);
        assert_eq!(update.outcome, Some(CallOutcome::Answered));
        let record = call.record().expect("a record");
        assert_eq!(record.status, CallStatus::Answered);
        assert_eq!(record.direction, CallDirection::Outgoing);
        assert_eq!(record.media, CallMedia::Voice);
        assert_eq!(record.duration, 221);
        assert!(record.ended_at >= record.started_at);
    }

    #[test]
    fn the_call_this_one_resolved_elsewhere_reads_as_the_right_kind_of_ending() {
        // An outgoing call answered on another of the account's devices: it was not ours to have,
        // and it was not a call nobody answered either.
        let mut call = dialing();
        let update = call
            .resolved_elsewhere()
            .expect("the other device took the call");
        assert_eq!(update.phase, CallPhase::Failed);
        assert_eq!(update.outcome, Some(CallOutcome::NoAnswer));

        // A ringing incoming call resolved elsewhere: this device never picked it up.
        let mut incoming = dialing();
        incoming.direction = CallDirection::Incoming;
        incoming.phase = CallPhase::Incoming;
        let update = incoming.resolved_elsewhere().expect("resolved elsewhere");
        assert_eq!(update.phase, CallPhase::Failed);
        assert_eq!(update.outcome, Some(CallOutcome::Missed));

        // A call that was up and then resolved elsewhere was answered, just not here.
        let mut live = dialing();
        live.media(&CallEvent::RelayAllocated);
        live.signaling(&accept()).expect("the peer answered");
        let update = live.resolved_elsewhere().expect("resolved elsewhere");
        assert_eq!(update.phase, CallPhase::Ended);
        assert_eq!(update.outcome, Some(CallOutcome::AnsweredElsewhere));
        // The other device answered, so this device's record is not an answered call with no
        // length: it carries its own status and an explicit history label.
        let record = live.record().expect("a record");
        assert_eq!(record.status, CallStatus::AnsweredElsewhere);
        assert!(
            !record.status.connected(),
            "this device never carried the call"
        );
    }

    #[test]
    fn a_call_resolved_elsewhere_before_it_is_active_is_not_answered_elsewhere() {
        // The peer answered and the media plane is still negotiating, so this device never reached
        // Active: a resolve now is not a call another device took with a zero local duration.
        let mut call = dialing();
        call.signaling(&accept()).expect("the peer answered");
        assert_eq!(call.phase(), CallPhase::Connecting);
        let update = call.resolved_elsewhere().expect("resolved elsewhere");
        assert_eq!(update.phase, CallPhase::Failed);
        assert_eq!(update.outcome, Some(CallOutcome::NoAnswer));
        assert!(
            !call.record().expect("a record").status.connected(),
            "never answered here"
        );
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
        assert_eq!(
            call.record().expect("a record").status,
            CallStatus::Declined
        );

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
        assert_eq!(
            call.record().expect("a record").status,
            CallStatus::NoAnswer
        );
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
        assert_eq!(
            call.record().expect("a record").status,
            CallStatus::ConnectionLost
        );
        assert!(!call.record().expect("a record").status.connected());
    }

    #[cfg(unix)]
    #[test]
    fn killing_the_camera_child_ends_a_blocked_read() {
        use std::io::Read as _;
        use std::process::Stdio;
        // A long-lived child stands in for a camera or `ffmpeg` that stalled while holding the
        // device: the capture thread would be parked in a blocking read on its stdout.
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .stdout(Stdio::piped())
            .spawn()
            .expect("a child to stand in for a stalled camera");
        let mut stdout = child.stdout.take().expect("the child has a stdout");
        let slot = std::sync::Mutex::new(Some(child));
        let reader = std::thread::spawn(move || {
            let mut buffer = [0u8; 8];
            // Blocks until the child dies, then reads its end of file.
            let _ = stdout.read(&mut buffer);
        });
        kill_camera_child(&slot);
        assert!(
            slot.lock().expect("the slot").is_none(),
            "the child is taken out of the slot when it is killed"
        );
        reader
            .join()
            .expect("the blocked read returned once the child died");
    }
}

/// The device discovery, on the machine it is running on.
///
/// Ignored by default: it reads this computer's real PipeWire graph and V4L2 nodes, so its result
/// depends on the hardware. Run it with `cargo test --lib -- --ignored --nocapture
/// calls::hardware_tests` on a machine with PipeWire to see what a call would be offered, which is
/// how a report about a device that is missing from the pickers gets answered.
#[cfg(test)]
mod platform_tests {
    use super::*;

    /// The compile-time platform is what the interface is told, so a build for macOS or Windows
    /// cannot offer a camera whose backend would fail to open one.
    #[test]
    fn calling_is_offered_only_where_the_media_backend_runs() {
        let capable = capabilities();
        assert!(
            capable.voice,
            "rodio carries voice on every platform the app builds for"
        );
        let linux = cfg!(target_os = "linux");
        assert_eq!(capable.video, linux, "video needs V4L2 and ffmpeg");
        assert_eq!(capable.camera, linux, "a camera list is V4L2's");
        assert!(
            !capable.screen_share,
            "one-to-one screen sharing is not in the pinned protocol library"
        );
    }
}

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
            assert!(!device.id.is_empty(), "a device is named by its node name");
            assert!(
                !device.id.ends_with(".monitor"),
                "a monitor of a sink is not an input anybody can speak into"
            );
        }
        for camera in &list.cameras {
            assert!(
                camera.id.starts_with("/dev/video"),
                "a camera is a V4L2 node: {}",
                camera.id
            );
        }
    }
}
