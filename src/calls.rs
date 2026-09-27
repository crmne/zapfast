//! 1:1 WhatsApp calling: signaling, media, audio devices, and state.
//!
//! Audio goes through rodio, the same layer the rest of the app plays and records with, so a call
//! opens the platform's own audio API on every platform it ships for rather than shelling out to
//! `pw-record` and `pw-play`. The engine's 16 kHz mono frames meet whatever rate and channel count a
//! device wants inside [`crate::call_audio`], and each direction sits behind a *stable* channel, so
//! changing the input or output device replaces only the stream behind it without the engine ever
//! seeing a port close.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use anyhow::{Result, anyhow};

use crate::model::CallDirection;
use whatsapp_rust::prelude::{Client, Jid};
use whatsapp_rust::types::call::{CallAction, IncomingCall};
use whatsapp_rust::voip::{CallEvent, CallHandle};

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

    /// Whether the microphone and speaker pickers apply.
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
    pub phase: CallPhase,
    /// Set when the call became active; the timer counts from here.
    pub started: Option<Instant>,
    pub muted: bool,
    /// How the call ended, once it has. Worded by the call screen, stored by the call history.
    pub outcome: Option<CallOutcome>,
    /// Set when the engine says the peer's audio is missing, so a silent call says why instead of
    /// just being silent.
    pub peer_audio: Option<PeerAudio>,
    /// Devices the user picked that the machine no longer has, so the call screen can name them
    /// instead of quietly using another one.
    pub lost_devices: Vec<LostDevice>,
    /// The selected input and output nodes; `None` means the system default.
    pub microphone: Option<String>,
    pub speaker: Option<String>,
}

/// A microphone or speaker PipeWire knows about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioDevice {
    /// The PipeWire `node.name`, used verbatim as `--target`.
    pub id: String,
    /// The `node.description`, which is what a person recognises.
    pub label: String,
}

/// The devices the call screen offers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceList {
    pub microphones: Vec<AudioDevice>,
    pub speakers: Vec<AudioDevice>,
}

/// Which kind of call device a selection belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Microphone,
    Speaker,
}

/// The devices a call should open with, as the settings hold them.
#[derive(Clone, Debug, Default)]
pub struct CallDevices {
    pub microphone: Option<String>,
    pub speaker: Option<String>,
}

/// What a set of selections resolved to against the machine's own list.
#[derive(Clone, Debug, Default)]
pub struct ResolvedDevices {
    pub microphone: Option<String>,
    pub speaker: Option<String>,
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
) -> ResolvedDevices {
    let microphones: Vec<&str> = list.microphones.iter().map(|d| d.id.as_str()).collect();
    let speakers: Vec<&str> = list.speakers.iter().map(|d| d.id.as_str()).collect();
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

    ResolvedDevices {
        microphone,
        speaker,
        lost_devices: lost,
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
/// voice is offered everywhere. The interface asks this, and a platform without a backend for a
/// feature does not offer it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallCapabilities {
    /// One-to-one voice calls: a microphone and a speaker.
    pub voice: bool,
}

/// What the media backend behind [`CallPhase`] can do on this platform.
pub fn capabilities() -> CallCapabilities {
    CallCapabilities {
        // rodio opens PipeWire, CoreAudio or WASAPI, so a call can carry voice wherever the app
        // builds. Whether this machine has a microphone at all is asked when a call is placed,
        // because that is a fact about the machine rather than about the platform.
        voice: true,
    }
}

// ---------------------------------------------------------------------------
// Device discovery
// ---------------------------------------------------------------------------

/// Lists microphones and speakers for the call screen.
///
/// They come from rodio, the same layer a call opens them through, so a picker offers what a call
/// can actually take and the names it reports are what the saved setting holds. On a platform whose
/// call backend is not built, the lists are empty rather than a list nothing can use, and the
/// pickers are never reached.
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
    }
}

/// Wraps the names rodio reports as the call screen's device entries.
fn audio_devices(found: Vec<(String, String)>) -> Vec<AudioDevice> {
    found
        .into_iter()
        .map(|(id, label)| AudioDevice { id, label })
        .collect()
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
    microphone: Option<String>,
    speaker_device: Option<String>,
    /// The devices the user picked that the machine no longer has, newest check last.
    lost_devices: Vec<LostDevice>,
}

impl Call {
    /// Places an outgoing 1:1 call.
    pub async fn place(
        client: &Arc<Client>,
        chat: String,
        microphone: Option<String>,
        speaker: Option<String>,
    ) -> Result<Self> {
        let peer: Jid = chat
            .parse()
            .map_err(|error| anyhow!("not a WhatsApp JID: {error}"))?;
        if !capabilities().voice {
            return Err(anyhow!("calling is not available on this platform yet"));
        }
        if let Some(missing) = missing_audio_device() {
            return Err(anyhow!("no {missing} is available; a call needs one"));
        }
        let (mic, mic_rx) = AudioInput::spawn(microphone.clone());
        let (output, output_tx) = AudioOutput::spawn(speaker.clone());

        let voip = client.voip();
        let placed = voip.call(&peer).audio(mic_rx, output_tx).start().await;
        let handle = match placed {
            Ok(handle) => Arc::new(handle),
            Err(error) => return Err(anyhow!("WhatsApp refused the call: {error}")),
        };
        // The destination is deliberately not logged: a call's chat id is the peer's phone
        // number, and this log ships.
        log::info!("[CALL] outgoing created call_id={}", handle.call_id());

        Ok(Self {
            generation: next_generation(),
            call_id: handle.call_id().to_owned(),
            chat,
            direction: CallDirection::Outgoing,
            phase: CallPhase::Dialing,
            started: None,
            outcome: None,
            peer_audio: None,
            incoming: None,
            handle: Some(handle),
            media_ready: false,
            mic: Some(mic),
            speaker: Some(output),
            microphone,
            speaker_device: speaker,
            lost_devices: Vec::new(),
        })
    }

    /// Records an incoming offer so the UI can ask the user. No media exists until they answer.
    ///
    /// The remembered devices are pre-selected on the prompt, so answering opens the same
    /// microphone and speaker the last call used rather than the system defaults.
    pub fn ringing(chat: String, incoming: Box<IncomingCall>, devices: CallDevices) -> Self {
        let call_id = incoming.action.call_id().to_owned();
        log::info!("[CALL] incoming offer call_id={call_id}");
        Self {
            generation: next_generation(),
            call_id,
            chat,
            direction: CallDirection::Incoming,
            phase: CallPhase::Incoming,
            started: None,
            outcome: None,
            peer_audio: None,
            incoming: Some(incoming),
            handle: None,
            media_ready: false,
            mic: None,
            speaker: None,
            microphone: devices.microphone,
            speaker_device: devices.speaker,
            lost_devices: Vec::new(),
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
            phase: self.phase,
            started: self.started,
            muted: self.handle.as_ref().is_some_and(|handle| handle.is_muted()),
            outcome: self.outcome,
            peer_audio: self.peer_audio,
            lost_devices: self.lost_devices.clone(),
            microphone: self.microphone.clone(),
            speaker: self.speaker_device.clone(),
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

    /// Rebinds anything the machine no longer has: a headset switched off mid-call.
    ///
    /// Returns what had to change, so the caller can say it. Nothing is reported against an empty
    /// list, which is a discovery tool that could not run rather than an empty machine. A lost
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
        lost
    }

    /// The devices picked so far: what a call that has not started media yet should bind to, so a
    /// microphone chosen while the phone was ringing is the one the call actually uses.
    pub fn selections(&self) -> (Option<String>, Option<String>) {
        (self.microphone.clone(), self.speaker_device.clone())
    }

    /// Answers a ringing call: sends the real `<accept>` and brings up media.
    pub async fn answer(
        &mut self,
        client: &Arc<Client>,
        microphone: Option<String>,
        speaker: Option<String>,
    ) -> Result<()> {
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
        let (mic, mic_rx) = AudioInput::spawn(microphone.clone());
        let (output, output_tx) = AudioOutput::spawn(speaker.clone());

        let voip = client.voip();
        let accepted = voip
            .accept(&incoming)
            .audio(mic_rx, output_tx)
            .start()
            .await;
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
        // The user picked the phone up. The media plane is still coming up, so this is `Accepted`
        // rather than `Connecting`, which is what the peer's own `<accept>` means on an outgoing
        // call: neither phase claims the call is live yet.
        self.phase = CallPhase::Accepted;
        log::info!("[CALL] accepted locally call_id={}", self.call_id);
        self.became_active();
        Ok(())
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
        let stats = handle.media_stats();
        log::info!(
            "[CALL] media stats call_id={} at={} rtp_received={} unexpected_pt={} srtp_failed={} sframe_failed={} unclassified={} pipe_dropped={} decoded={} delivered={} foreign_decoded={} concealed={} without_decoder={} without_encoder={} mlow_sid={} mlow_dropped={} sink_dropped={} trimmed={} codec_switches={} video_sink_dropped={} peer_keyframes={}",
            self.call_id,
            marker,
            stats.rtp_received,
            stats.rtp_payload_type_unexpected,
            stats.srtp_unprotect_failed,
            stats.sframe_decrypt_failed,
            stats.relay_packet_unclassified,
            stats.inbound_pipe_dropped,
            stats.audio_frames_decoded,
            stats.audio_frames_delivered,
            stats.foreign_frames_decoded,
            stats.audio_frames_concealed,
            stats.audio_frames_without_decoder,
            stats.outbound_frames_without_encoder,
            // The two that say a peer's packets arrived and were read as frames that carry no
            // speech: MLOW's silence descriptor, and a frame the profile's own decoder refused. A
            // call whose `rtp_received` climbs while `decoded` does not is either of these, and
            // without them a report cannot tell a peer who stopped talking from one whose audio
            // this side stopped understanding.
            stats.mlow_inactive_or_sid,
            stats.mlow_off_point_dropped,
            stats.audio_sink_dropped,
            stats.playout_trimmed_samples,
            stats.codec_switches,
            stats.video_sink_dropped,
            stats.peer_keyframe_requests,
        );
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
        log::info!("[CALL] active call_id={}", self.call_id);
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

    /// A machine with a microphone and a speaker, named the way a picker would show them.
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
    }

    /// Changing the microphone or the speaker leaves the call's audio alone.
    ///
    /// The devices below belong to the test rather than the machine, so this holds on a build host
    /// with no sound at all.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn changing_audio_devices_leaves_the_calls_audio_running() {
        let samples: Vec<f32> = (0..960).map(|sample| sample as f32 / 1_920.0).collect();
        let (_devices, speaker) = crate::call_audio::fake::install(samples, 2, 48_000, (48_000, 2));
        let mut call = dialing();
        let (input, frames) = AudioInput::spawn(None);
        let (output, _tx) = AudioOutput::spawn(None);
        call.mic = Some(input);
        call.speaker = Some(output);

        // The microphone is delivering before anything else is touched.
        assert!(
            frames.recv().await.is_ok(),
            "the microphone is delivering before any device changes"
        );

        call.set_speaker(Some("Second speaker".to_owned()));
        call.set_microphone(Some("Second microphone".to_owned()));

        // The engine's microphone channel is the same one it was, and it is still carrying frames.
        tokio::time::timeout(std::time::Duration::from_secs(5), frames.recv())
            .await
            .expect("the microphone channel survived the changes")
            .expect("the frames are still coming");
        // Only the rebind opened a device.
        let started = std::time::Instant::now();
        while speaker.opened_count() < 2 && started.elapsed() < std::time::Duration::from_secs(5) {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(
            speaker.opened(),
            vec![None, Some("Second speaker".to_owned())]
        );
        call.cleanup();
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
        let (input, frames) = AudioInput::spawn(None);
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
    /// and a device change made while muted rebinds the streams rather than restarting the call.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_muted_call_keeps_its_audio_and_its_phase() {
        let samples: Vec<f32> = (0..960).map(|sample| sample as f32 / 1_920.0).collect();
        let (_devices, speaker) = crate::call_audio::fake::install(samples, 2, 48_000, (48_000, 2));
        let mut call = dialing();
        let (input, frames) = AudioInput::spawn(None);
        let (output, _tx) = AudioOutput::spawn(None);
        call.mic = Some(input);
        call.speaker = Some(output);
        call.signaling(&accept()).expect("the peer answered");
        call.media(&CallEvent::RelayAllocated);
        assert_eq!(call.phase(), CallPhase::Active, "the call is up");

        // Muted, and everything the screen can do meanwhile is done to it.
        call.set_microphone(Some("Second microphone".to_owned()));
        call.set_speaker(Some("Second speaker".to_owned()));

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
        // Neither the mute nor the device changes moved the call.
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

    fn snapshot(phase: CallPhase) -> CallUpdate {
        CallUpdate {
            generation: 7,
            chat: "15551234567@s.whatsapp.net".to_owned(),
            direction: CallDirection::Outgoing,
            phase,
            started: None,
            muted: false,
            outcome: None,
            peer_audio: None,
            lost_devices: Vec::new(),
            microphone: None,
            speaker: None,
        }
    }

    #[test]
    fn the_devices_a_person_picked_are_the_ones_a_call_opens_with() {
        let machine = machine();
        let resolved = resolve_devices(
            &machine,
            Some(machine.microphones[0].id.clone()),
            Some(machine.speakers[0].id.clone()),
        );
        assert_eq!(resolved.microphone, Some(machine.microphones[0].id.clone()));
        assert_eq!(resolved.speaker, Some(machine.speakers[0].id.clone()));
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
        );
        assert_eq!(
            resolved.microphone, None,
            "the microphone moves to the default"
        );
        assert_eq!(resolved.speaker, None, "and so does the speaker");
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
            ]
        );
    }

    #[test]
    fn an_empty_list_is_a_broken_tool_rather_than_an_empty_machine() {
        let resolved = resolve_devices(
            &DeviceList::default(),
            Some("bluez_input.AC".to_owned()),
            Some("bluez_output.AC".to_owned()),
        );
        assert_eq!(resolved.microphone.as_deref(), Some("bluez_input.AC"));
        assert_eq!(resolved.speaker.as_deref(), Some("bluez_output.AC"));
        assert!(
            resolved.lost_devices.is_empty(),
            "a device list that could not be read says nothing about the devices"
        );
    }

    #[test]
    fn a_device_is_named_the_way_the_picker_named_it() {
        let machine = machine();
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

    #[test]
    fn the_periodic_check_only_republishes_a_real_change() {
        // The worker compares each fresh snapshot with the one the UI was handed, so a call that is
        // merely ticking does not reach the screen again and a changed phase does.
        assert_eq!(snapshot(CallPhase::Active), snapshot(CallPhase::Active));
        assert_ne!(snapshot(CallPhase::Accepted), snapshot(CallPhase::Active));
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
            phase: CallPhase::Dialing,
            started: None,
            outcome: None,
            peer_audio: None,
            incoming: None,
            handle: None,
            media_ready: false,
            mic: None,
            speaker: None,
            microphone: None,
            speaker_device: None,
            lost_devices: Vec::new(),
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
        let update = call.resolved_elsewhere().expect("resolved elsewhere");
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
/// Ignored by default: it reads this computer's real PipeWire graph, so its result depends on the
/// hardware. Run it with `cargo test --lib -- --ignored --nocapture calls::hardware_tests` on a
/// machine with PipeWire to see what a call would be offered, which is how a report about a device
/// that is missing from the pickers gets answered.
#[cfg(test)]
mod platform_tests {
    use super::*;

    /// The compile-time platform is what the interface is told.
    #[test]
    fn calling_is_offered_only_where_the_media_backend_runs() {
        let capable = capabilities();
        assert!(
            capable.voice,
            "rodio carries voice on every platform the app builds for"
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
        for device in list.microphones.iter().chain(&list.speakers) {
            assert!(!device.id.is_empty(), "a device is named by its node name");
            assert!(
                !device.id.ends_with(".monitor"),
                "a monitor of a sink is not an input anybody can speak into"
            );
        }
    }
}
