//! The worker's side of a 1:1 call: commands, signaling routing, and media events.
//!
//! One call at a time. [`Worker::call`] holds the only one, and every entry point here refuses to
//! start or accept a second while it exists — a double click, a second chat's call button, and a
//! second inbound offer all land on the same refusal.
//!
//! The state the UI renders is never computed here: it comes from the backend. The window draws a
//! call as placing the moment its button is pressed; from there `<preaccept>` says the peer's
//! phone rings, `<accept>` moves an outgoing call out of dialing, the media plane's `RelayAllocated` is what makes a call Active and
//! starts the timer, and a `<reject>`/`<terminate>`/relay loss ends it with the reason the peer
//! gave. Nothing is inferred from the button that was pressed.

use super::*;
use crate::calls::{self, Call, CallUpdate, VideoTick};
use whatsapp_rust::types::call::IncomingCall;
use whatsapp_rust::voip::{CallEvent, KeyframeUrgency};

/// The call this worker owns: its state, and the channels the tasks around it feed.
///
/// Dropped the moment the call reaches a terminal phase, which is what releases the media tasks,
/// the microphone and speaker streams and the camera thread.
pub(super) struct CallRuntime {
    pub(super) call: Call,
    /// Media-plane events, and the signal that the media task finished.
    pub(super) events: async_channel::Receiver<CallRuntimeEvent>,
    /// Video frames for the UI.
    pub(super) frames: Option<async_channel::Receiver<VideoTick>>,
    /// Whether the media-plane watcher was started for this call's handle.
    watching: bool,
    /// The snapshot the UI was last handed, so the periodic check publishes only real changes.
    last: CallUpdate,
}

/// What a call's background tasks report back to the worker loop.
pub(super) enum CallRuntimeEvent {
    /// One event off the call's media plane.
    Media(CallEvent),
    /// The media task is gone: relay disconnect, send failure, or hangup.
    Ended,
    /// A call link's join finished: the call is in, or why it is not.
    LinkJoined(Result<whatsapp_rust::voip::CallHandle, String>),
    /// The call service answered a group call we started: it is out, or why it is not.
    GroupStarted(Result<whatsapp_rust::voip::CallHandle, calls::GroupRefusal>),
}

impl CallRuntime {
    /// A runtime whose media plane is not watched yet: a call that is still ringing has no handle
    /// to watch, and gets one the moment it is placed or answered.
    pub(super) fn new(call: Call, frames: Option<async_channel::Receiver<VideoTick>>) -> Self {
        let (_tx, events) = async_channel::bounded(64);
        let last = call.update();
        Self {
            call,
            events,
            frames,
            watching: false,
            last,
        }
    }

    /// A call link's runtime, whose events are its join's answer until it has a handle to watch.
    fn joining(
        call: Call,
        frames: Option<async_channel::Receiver<VideoTick>>,
        events: async_channel::Receiver<CallRuntimeEvent>,
    ) -> Self {
        let last = call.update();
        Self {
            call,
            events,
            frames,
            watching: false,
            last,
        }
    }

    /// Starts forwarding this call's media events and its media-finished signal, once.
    pub(super) fn watch(&mut self) {
        if self.watching {
            return;
        }
        let Some(handle) = self.call.handle() else {
            return;
        };
        self.watching = true;
        let (tx, events) = async_channel::bounded(64);
        self.events = events;
        // The handle's event queue has one reader, and this is it.
        let Some(mut media) = handle.take_events() else {
            log::warn!("[CALL] the call's events were already taken");
            return;
        };
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    event = media.recv() => match event {
                        Ok(event) => {
                            if tx.send(CallRuntimeEvent::Media(event)).await.is_err() {
                                return;
                            }
                        }
                        Err(_) => {
                            let _ = tx.send(CallRuntimeEvent::Ended).await;
                            return;
                        }
                    },
                    _ = handle.wait_ended() => {
                        let _ = tx.send(CallRuntimeEvent::Ended).await;
                        return;
                    }
                }
            }
        });
    }
}

impl Worker {
    // -----------------------------------------------------------------------
    // Command entry points
    // -----------------------------------------------------------------------

    /// Hands the call screen the microphones, speakers and cameras it can offer, and keeps them:
    /// they are how a device that goes away is named by the description the user saw.
    pub(super) async fn emit_call_devices(&mut self) -> calls::DeviceList {
        let devices = discover_devices().await;
        self.publish_call_devices(devices.clone());
        devices
    }

    /// Stores and publishes a device list, so the pickers and the fallback naming share one
    /// snapshot of the machine.
    pub(super) fn publish_call_devices(&mut self, devices: calls::DeviceList) {
        self.call_devices = devices.clone();
        self.emit(Event::CallDevices(Box::new(devices)));
    }

    /// Re-sends the current call after privacy recovery, since call updates are withheld while the
    /// lock state is unknown.
    ///
    /// Nothing is rediscovered: the last published list is what the picker already had, and the
    /// runtime's own snapshot is what the surface should draw. The chat can finally be judged
    /// private or not, so a call that arrived mid-recovery rings behind the right lock state rather
    /// than being dropped for the whole recovery.
    pub(super) fn replay_call(&mut self) {
        let Some(runtime) = self.call.as_ref() else {
            return;
        };
        let devices = self.call_devices.clone();
        let update = runtime.last.clone();
        self.publish_call_devices(devices);
        self.emit_call(update);
    }

    /// Publishes one call state, and lets the call go once it reaches a terminal phase.
    ///
    /// The UI keeps rendering the snapshot it was handed; what is released here is the media, so a
    /// finished call holds no child process and no task.
    pub(super) fn emit_call(&mut self, update: CallUpdate) {
        let update = self.canonical_call(update);
        let finished = !update.phase.is_live();
        // Remembered so the periodic check can tell a real change from a snapshot it already sent.
        if let Some(runtime) = self.call.as_mut()
            && runtime.call.generation() == update.generation
        {
            runtime.last = update.clone();
        }
        self.emit(Event::Call(Box::new(update.clone())));
        // Only the call this update is about: a call refused before it was placed must not end the
        // one that made it busy.
        if finished
            && self
                .call
                .as_ref()
                .is_some_and(|runtime| runtime.call.generation() == update.generation)
            && let Some(runtime) = self.call.take()
        {
            self.file_finished_call(&runtime.call, &update);
        }
    }

    /// Files a group call's people under the chat ids the rest of the app uses, as every other
    /// `Jid` that reaches the window is: a privacy id becomes the phone number once it is known. Our
    /// own entry on the roster is left out, since the call screen draws the others.
    fn canonical_call(&self, mut update: CallUpdate) -> CallUpdate {
        if let Some(group) = update.group.as_mut() {
            group.caller = group.caller.as_deref().map(|id| self.canonical_str(id));
            for person in &mut group.participants {
                person.id = self.canonical_str(&person.id);
            }
            let me = self.me();
            group.participants.retain(|person| person.id != me);
        }
        update
    }

    /// Whether a call is already up, which every entry point refuses to double.
    pub(super) fn call_busy(&mut self) -> bool {
        if self.call.is_some() {
            log::warn!("[CALL] refusing a new call: one is already up");
            return true;
        }
        false
    }

    /// Places the call the window is already showing as placing under `generation`.
    ///
    /// Every way this can stop short of an offer answers the window with that generation failed,
    /// so the screen it drew at once says so and goes instead of waiting on a call that will never
    /// ring. A call hung up while it was still placing (`cancel`) is let go without a word: the
    /// window already took it down, and the peer was never rung.
    pub(super) async fn start_call(
        &mut self,
        chat: ChatId,
        video: bool,
        generation: u64,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        if crate::model::ChatKind::from_id(&chat) == crate::model::ChatKind::Group {
            if !calls::group_calls_enabled() {
                // The interface offers no button for it; the worker is the boundary all the same.
                log::warn!("[CALL] refusing to start a group call: group calls are off");
                let mut placing = CallUpdate::placing(generation, chat.to_string(), video);
                placing.group = Some(calls::GroupCall::default());
                self.emit_call(placing.not_placed());
                return;
            }
            self.start_group_call(chat, video, generation, cancel).await;
            return;
        }
        let placing = CallUpdate::placing(generation, chat.to_string(), video);
        if self.call_busy() {
            self.emit_call(placing.not_placed());
            return;
        }
        if !callable_chat(&chat) {
            log::warn!("[CALL] refusing a call to a chat that is neither a person nor a group");
            self.emit(Event::Error("Calls are one to one only".to_owned()));
            self.emit_call(placing.not_placed());
            return;
        }
        let Some(client) = self.client.clone() else {
            log::warn!("[CALL] cannot start a call: WhatsApp is not connected");
            self.emit(Event::Error(
                "ZapFast is not connected to WhatsApp".to_owned(),
            ));
            self.emit_call(placing.not_placed());
            return;
        };
        // A call opens on the system's default devices; the call screen's pickers move it to
        // others while it runs.
        let devices = self.emit_call_devices().await;
        let camera = video
            .then(|| calls::default_camera(&devices, None))
            .flatten();
        let placed = Call::place(
            &client,
            generation,
            chat.to_string(),
            video,
            None,
            None,
            camera,
            &cancel,
        )
        .await;
        match placed {
            Ok((call, frames)) => {
                let mut runtime = CallRuntime::new(call, frames);
                runtime.watch();
                let update = runtime.call.update();
                self.call = Some(runtime);
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    // Hung up while the offer was on its way: it is ended with the peer at once,
                    // and filed like any call nobody answered.
                    log::info!("[CALL] the call was hung up while it was being placed");
                    self.hangup_call().await;
                } else {
                    self.emit_call(update);
                }
            }
            Err(error) if error.is::<calls::Cancelled>() => {
                log::info!("[CALL] {error}");
                self.emit_call(placing.not_placed());
            }
            Err(error) => {
                log::error!("[CALL] could not start the call: {error}");
                self.emit(Event::Error(format!(
                    "The call could not be started: {error}"
                )));
                self.emit_call(placing.not_placed());
            }
        }
    }

    /// Starts a call in the group `chat`, which the window is already showing as placing under
    /// `generation`, the way [`Worker::start_call`] places a 1:1 call.
    ///
    /// The offer runs on a task of its own (the library waits for the call service's answer and
    /// the relay before it hands the call back), and its answer comes back as
    /// [`CallRuntimeEvent::GroupStarted`]. Until then the call reads as calling.
    async fn start_group_call(
        &mut self,
        chat: ChatId,
        video: bool,
        generation: u64,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        let mut placing = CallUpdate::placing(generation, chat.to_string(), video);
        placing.group = Some(calls::GroupCall::default());
        if self.call_busy() {
            self.emit_call(placing.not_placed());
            return;
        }
        let Some(client) = self.client.clone() else {
            log::warn!("[CALL] cannot start a call: WhatsApp is not connected");
            self.emit(Event::Error(
                "ZapFast is not connected to WhatsApp".to_owned(),
            ));
            self.emit_call(placing.not_placed());
            return;
        };
        let devices = self.emit_call_devices().await;
        let camera = video
            .then(|| calls::default_camera(&devices, None))
            .flatten();
        let (done, answers) = async_channel::bounded(1);
        let placed = Call::place_group(
            &client,
            generation,
            chat.to_string(),
            video,
            camera,
            &cancel,
            done,
        )
        .await;
        match placed {
            Ok((call, frames)) => {
                let (tx, events) = async_channel::bounded(1);
                // The call service's answer, as the runtime's first event.
                tokio::spawn(async move {
                    if let Ok(answer) = answers.recv().await {
                        let _ = tx.send(CallRuntimeEvent::GroupStarted(answer)).await;
                    }
                });
                let runtime = CallRuntime::joining(call, frames, events);
                let update = runtime.call.update();
                self.call = Some(runtime);
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    log::info!("[CALL] the call was hung up while it was being placed");
                    self.hangup_call().await;
                } else {
                    self.emit_call(update);
                }
            }
            Err(error) if error.is::<calls::Cancelled>() => {
                log::info!("[CALL] {error}");
                self.emit_call(placing.not_placed());
            }
            Err(error) => {
                log::error!("[CALL] could not start the group call: {error}");
                self.emit(Event::Error(format!(
                    "The call could not be started: {error}"
                )));
                self.emit_call(placing.not_placed());
            }
        }
    }

    /// Joins a call link the window is already showing as placing under `generation`.
    ///
    /// The join itself runs on a task of its own (the call service may hold us in its waiting
    /// room), and its answer comes back as [`CallRuntimeEvent::LinkJoined`]. A call link belongs
    /// to no chat, so it is drawn under its roster and filed nowhere.
    pub(super) async fn join_call_link(
        &mut self,
        token: String,
        video: bool,
        generation: u64,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        let mut placing = CallUpdate::placing(generation, String::new(), video);
        placing.group = Some(calls::GroupCall {
            link: true,
            ..calls::GroupCall::default()
        });
        if !calls::group_calls_enabled() {
            log::warn!("[CALL] not joining a call link: group calls are off");
            self.emit(Event::Error(calls::CALL_LINKS_OFF.to_owned()));
            self.emit_call(placing.not_placed());
            return;
        }
        if self.call_busy() {
            self.emit_call(placing.not_placed());
            return;
        }
        let Some(client) = self.client.clone() else {
            log::warn!("[CALL] cannot join a call link: WhatsApp is not connected");
            self.emit(Event::Error(
                "ZapFast is not connected to WhatsApp".to_owned(),
            ));
            self.emit_call(placing.not_placed());
            return;
        };
        let devices = self.emit_call_devices().await;
        let camera = video
            .then(|| calls::default_camera(&devices, None))
            .flatten();
        let (done, answers) = async_channel::bounded(1);
        let joined =
            Call::join_link(&client, generation, token, video, camera, &cancel, done).await;
        match joined {
            Ok((call, frames)) => {
                let (tx, events) = async_channel::bounded(1);
                // The join's answer, as the runtime's first event.
                tokio::spawn(async move {
                    if let Ok(answer) = answers.recv().await {
                        let _ = tx.send(CallRuntimeEvent::LinkJoined(answer)).await;
                    }
                });
                let runtime = CallRuntime::joining(call, frames, events);
                let update = runtime.call.update();
                self.call = Some(runtime);
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    self.hangup_call().await;
                } else {
                    self.emit_call(update);
                }
            }
            Err(error) if error.is::<calls::Cancelled>() => {
                self.emit_call(placing.not_placed());
            }
            Err(error) => {
                log::error!("[CALL] could not join the call link: {error}");
                self.emit(Event::Error(format!(
                    "The call link could not be joined: {error}"
                )));
                self.emit_call(placing.not_placed());
            }
        }
    }

    pub(super) async fn answer_call(&mut self) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let update = match self.call.as_mut() {
            Some(runtime) => {
                // Whatever the user picked while the phone was ringing is what the media binds to,
                // checked against the machine so one that vanished falls back rather than opening a
                // stream that can never deliver.
                let (microphone, speaker, camera) = runtime.call.selections();
                let resolved =
                    calls::resolve_devices(&discover_devices().await, microphone, speaker, camera);
                let answered = runtime
                    .call
                    .answer(
                        &client,
                        resolved.microphone,
                        resolved.speaker,
                        resolved.camera,
                    )
                    .await;
                match answered {
                    Ok(frames) => {
                        if frames.is_some() {
                            runtime.frames = frames;
                        }
                        runtime.call.set_lost_devices(resolved.lost_devices);
                        runtime.watch();
                        Some(runtime.call.update())
                    }
                    Err(error) => {
                        log::error!("[CALL] could not answer the call: {error}");
                        let update = runtime.call.abandon(&client).await;
                        self.emit(Event::Error(format!(
                            "The call could not be answered: {error}"
                        )));
                        Some(update)
                    }
                }
            }
            None => None,
        };
        if let Some(update) = update {
            self.emit_call(update);
        }
    }

    pub(super) async fn decline_call(&mut self) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let update = match self.call.as_mut() {
            Some(runtime) => match runtime.call.decline(&client).await {
                Ok(()) => Some(runtime.call.update()),
                Err(error) => {
                    log::error!("[CALL] could not decline the call: {error}");
                    return;
                }
            },
            None => None,
        };
        if let Some(update) = update {
            self.emit_call(update);
        }
    }

    pub(super) async fn hangup_call(&mut self) {
        let client = self.client.clone();
        let update = match self.call.as_mut() {
            Some(runtime) => Some(runtime.call.hangup(client.as_ref()).await),
            None => None,
        };
        if let Some(update) = update {
            self.emit_call(update);
        }
    }

    pub(super) async fn set_call_muted(&mut self, muted: bool) {
        let update = match self.call.as_mut() {
            Some(runtime) => runtime.call.set_muted(muted).await,
            None => None,
        };
        if let Some(update) = update {
            self.emit_call(update);
        }
    }

    pub(super) async fn set_call_camera(&mut self, on: bool) {
        let outcome = match self.call.as_mut() {
            Some(runtime) => {
                if on && !runtime.call.is_video() {
                    // A voice call: ask the peer for video, or answer the peer who asked, and start
                    // draining the frames it brings, from the first camera the machine reports.
                    let camera = calls::default_camera(&self.call_devices, None);
                    match runtime.call.upgrade_to_video(camera).await {
                        Ok(frames) => {
                            if frames.is_some() {
                                runtime.frames = frames;
                            }
                            Ok(runtime.call.update())
                        }
                        Err(error) => Err(error),
                    }
                } else {
                    runtime.call.set_camera(on).await
                }
            }
            None => return,
        };
        match outcome {
            Ok(update) => self.emit_call(update),
            Err(error) => {
                log::error!("[CALL] the camera could not be toggled: {error}");
                self.emit(Event::Error(error.to_string()));
            }
        }
    }

    pub(super) fn set_call_microphone(&mut self, device: Option<String>) {
        let update = self
            .call
            .as_mut()
            .map(|runtime| runtime.call.set_microphone(device));
        if let Some(update) = update {
            self.emit_call(update);
        }
    }

    pub(super) fn set_call_speaker(&mut self, device: Option<String>) {
        let update = self
            .call
            .as_mut()
            .map(|runtime| runtime.call.set_speaker(device));
        if let Some(update) = update {
            self.emit_call(update);
        }
    }

    pub(super) async fn set_call_camera_device(&mut self, device: Option<String>) {
        // `None` is the picker's "First camera": a camera has no system default, so it is read as
        // the first camera the machine reports, the same reading a call's startup applies.
        let device = calls::default_camera(&self.call_devices, device);
        let outcome = match self.call.as_mut() {
            Some(runtime) => Some(runtime.call.set_camera_device(device).await),
            None => None,
        };
        match outcome {
            Some(Ok(update)) => self.emit_call(update),
            Some(Err(error)) => {
                log::error!("[CALL] the camera could not be switched: {error}");
                self.emit(Event::Error(error.to_string()));
            }
            None => {}
        }
    }

    // -----------------------------------------------------------------------
    // Backend events
    // -----------------------------------------------------------------------

    /// One event from the current call's media plane.
    pub(super) fn call_runtime(&mut self, event: CallRuntimeEvent) {
        let mut refused = None;
        let mut group_refused = None;
        let update = match self.call.as_mut() {
            Some(runtime) => match event {
                CallRuntimeEvent::LinkJoined(joined) => {
                    refused = joined.as_ref().err().cloned();
                    let update = runtime.call.link_joined(joined);
                    runtime.watch();
                    update
                }
                CallRuntimeEvent::GroupStarted(started) => {
                    let live = runtime.call.phase().is_live();
                    group_refused = started.as_ref().err().filter(|_| live).cloned();
                    let update = runtime.call.group_started(started);
                    runtime.watch();
                    update
                }
                CallRuntimeEvent::Ended => runtime.call.media_ended(),
                CallRuntimeEvent::Media(media) => {
                    if matches!(media, CallEvent::RelayAllocated)
                        && runtime.call.is_video()
                        && let Some(handle) = runtime.call.handle()
                    {
                        // A decoder cannot start on a delta frame, so ask for an IDR at once.
                        handle.request_peer_keyframe(KeyframeUrgency::Immediate);
                    }
                    runtime.call.media(&media)
                }
            },
            None => None,
        };
        if let Some(error) = refused {
            self.emit(Event::Error(format!(
                "The call link could not be joined: {error}"
            )));
        }
        if let Some(refusal) = group_refused {
            self.emit(Event::Error(group_refusal_text(&refusal)));
        }
        if let Some(update) = update {
            self.emit_call(update);
        }
    }

    /// One video frame from the current call, straight to the UI.
    pub(super) fn call_frame(&mut self, tick: VideoTick) {
        match tick {
            VideoTick::Lost => {
                if let Some(handle) = self.call.as_ref().and_then(|runtime| runtime.call.handle()) {
                    handle.request_peer_keyframe(KeyframeUrgency::Coalesced);
                }
            }
            VideoTick::Local(image) => {
                self.emit(Event::CallVideo {
                    local: Some(image),
                    remote: None,
                    from: None,
                });
            }
            VideoTick::RemoteFrom { sender, image } => {
                // A group member's picture, for their own tile, under the id their tile has.
                let Some((id, first)) = self
                    .call
                    .as_mut()
                    .and_then(|runtime| runtime.call.group_picture(&sender))
                else {
                    return;
                };
                self.emit(Event::CallVideo {
                    local: None,
                    remote: Some(image),
                    from: Some(self.canonical_str(&id)),
                });
                // Their tile turns from their photo to their picture.
                if first && let Some(runtime) = self.call.as_ref() {
                    let update = self.canonical_call(runtime.call.update());
                    self.emit(Event::Call(Box::new(update)));
                }
            }
            VideoTick::Remote(image) => {
                let first = self
                    .call
                    .as_mut()
                    .is_some_and(|runtime| runtime.call.saw_remote_video());
                // The flag is what the UI gates the picture on, so a frame that arrives after the
                // peer said their video stopped is not drawn: it is a picture of a stream that has
                // ended, and showing it would contradict the state the peer just sent.
                let still_sending = self
                    .call
                    .as_ref()
                    .is_none_or(|runtime| runtime.call.peer_video_visible());
                if still_sending {
                    self.emit(Event::CallVideo {
                        local: None,
                        remote: Some(image),
                        from: None,
                    });
                }
                if first && let Some(runtime) = self.call.as_ref() {
                    let update = self.canonical_call(runtime.call.update());
                    self.emit(Event::Call(Box::new(update)));
                }
            }
        }
    }

    /// Routes one inbound `<call>` stanza: either it belongs to the call we own, or it is a new
    /// offer that should ring.
    ///
    /// This is where an outgoing call learns it was answered, rejected, or ended, so the phase the
    /// UI shows is the peer's, not the dialer's.
    pub(super) async fn call_signaling(&mut self, incoming: &IncomingCall) {
        let action = &incoming.action;
        let mine = self
            .call
            .as_ref()
            .is_some_and(|runtime| runtime.call.call_id() == action.call_id());
        if mine {
            let update = self
                .call
                .as_mut()
                .and_then(|runtime| runtime.call.signaling(action));
            if let Some(update) = update {
                self.emit_call(update);
            }
            return;
        }
        if let Some(index) = self.phone_only_call(action.call_id()) {
            if let Some(update) = self.phone_only_calls[index].signaling(action) {
                self.phone_only_changed(index, update);
            }
            return;
        }
        if !calls::is_offer(action) {
            return;
        }
        // A group call this device cannot join yet rings on the phone alone. Nothing goes back to
        // WhatsApp, not even while another call is up here, since a refusal could stop the phone
        // ringing; the window only says who is calling and to answer there.
        let group = calls::offer_is_group(incoming);
        if group && !calls::group_calls_enabled() {
            self.ring_on_phone_only(incoming);
            return;
        }
        if self.call_busy() {
            // Refuse it to the caller rather than dropping it. An offer that is only dropped leaves
            // the other side ringing until its own protocol timeout, believing this device might
            // still answer; a real `<reject>` ends it there and then. The call that is already up
            // is untouched: this is the second offer's own call id being refused, not ours.
            match self.client.as_ref() {
                Some(client) => match client.voip().reject(incoming).await {
                    Ok(()) => log::info!(
                        "[CALL] refused a second offer while one is up call_id={}",
                        action.call_id()
                    ),
                    Err(error) => {
                        log::warn!("[CALL] a second offer could not be refused: {error}")
                    }
                },
                None => log::warn!("[CALL] no client to refuse a second offer with"),
            }
            return;
        }
        let chat = self.offer_chat(incoming, group);
        // A broadcast or newsletter offer never reaches the call surface, and neither does a
        // group's offer that names something other than a group. The worker is the boundary: such
        // an offer is refused here rather than rung, so no call is created and no history entry is
        // written.
        //
        // Refuse it to the caller instead of only dropping it: a dropped offer leaves the other
        // side ringing until its own protocol timeout, believing this device might still answer,
        // while a real `<reject>` ends it there and then.
        if !rings_in(&chat, group) {
            match self.client.as_ref() {
                Some(client) => match client.voip().reject(incoming).await {
                    Ok(()) => log::warn!(
                        "[CALL] refused an incoming offer from a chat that cannot be called group={group}"
                    ),
                    Err(error) => {
                        log::warn!("[CALL] a non-direct offer could not be refused: {error}")
                    }
                },
                None => log::warn!("[CALL] no client to refuse a non-direct offer with"),
            }
            return;
        }
        let video = calls::offer_is_video(action);
        let call = Call::ringing(chat, Box::new(incoming.clone()), video);
        let update = call.update();
        self.call = Some(CallRuntime::new(call, None));
        self.emit_call_devices().await;
        self.emit_call(update);
    }

    /// The chat an offer rings in. A group call rings under its group, as WhatsApp files it; a
    /// call among several people with no group behind it rings under the person who started it.
    /// Either way the roster the offer carried says who else is on it.
    fn offer_chat(&self, incoming: &IncomingCall, group: bool) -> ChatId {
        match calls::offer_group(incoming) {
            Some(jid) => self.canonical(&jid),
            None if group => self.canonical(&incoming.action.call_creator().to_non_ad()),
            None => self.canonical(&incoming.from),
        }
    }

    /// Keeps a group call's offer that only the phone can answer while group calls are off, and
    /// tells the window, which says who is calling and to answer on the phone. The call is not
    /// this worker's call: it takes no devices, never rings here, and leaves the worker free for
    /// another. It is filed in its group as it ends, as a call that rang here would be.
    fn ring_on_phone_only(&mut self, incoming: &IncomingCall) {
        let chat = self.offer_chat(incoming, true);
        // An offer naming something other than a group is no group's call to tell anyone about.
        if !rings_in(&chat, true) {
            log::warn!("[CALL] ignored a group offer from a chat that cannot be called");
            return;
        }
        let video = calls::offer_is_video(&incoming.action);
        let call = Call::ringing(chat, Box::new(incoming.clone()), video);
        log::info!(
            "[CALL] a group call rings on the phone only, group calls are off call_id={}",
            call.call_id()
        );
        let update = self.canonical_call(call.update());
        if self.phone_only_calls.len() >= MAX_PHONE_ONLY_CALLS {
            self.phone_only_calls.remove(0);
        }
        self.phone_only_calls.push(call);
        self.emit(Event::PhoneOnlyCall(Box::new(update)));
    }

    /// Where the call `call_id` is among the group calls ringing on the phone only.
    fn phone_only_call(&self, call_id: &str) -> Option<usize> {
        self.phone_only_calls
            .iter()
            .position(|call| call.call_id() == call_id)
    }

    /// A group call ringing on the phone only moved on: once it is over it is filed in its group
    /// and the window takes back its word about it.
    fn phone_only_changed(&mut self, index: usize, update: CallUpdate) {
        if update.phase.is_live() {
            return;
        }
        let call = self.phone_only_calls.remove(index);
        let update = self.canonical_call(update);
        self.file_finished_call(&call, &update);
        self.emit(Event::PhoneOnlyCall(Box::new(update)));
    }

    /// A call we were ringing for was resolved on another of the account's devices (`elsewhere`
    /// says how), or the caller gave up before anyone answered (`None`).
    ///
    /// Returns whether the call was this worker's own, which files its entry as it ends.
    pub(super) fn call_resolved(
        &mut self,
        call_id: &str,
        elsewhere: Option<calls::CallOutcome>,
    ) -> bool {
        let mut mine = false;
        let update = match self.call.as_mut() {
            Some(runtime) if runtime.call.call_id() == call_id => {
                mine = true;
                runtime.call.resolved_elsewhere(elsewhere)
            }
            _ => None,
        };
        if let Some(update) = update {
            self.emit_call(update);
        }
        if let Some(index) = self.phone_only_call(call_id) {
            mine = true;
            if let Some(update) = self.phone_only_calls[index].resolved_elsewhere(elsewhere) {
                self.phone_only_changed(index, update);
            }
        }
        mine
    }

    /// Releases the call on the way out, so quitting leaves no child process or task behind.
    pub(super) async fn shutdown_call(&mut self) {
        if let Some(mut runtime) = self.call.take() {
            let update = runtime.call.hangup(self.client.as_ref()).await;
            let update = self.canonical_call(update);
            // The window lets the call go as it lets any other finished one.
            self.emit(Event::Call(Box::new(update.clone())));
            self.file_finished_call(&runtime.call, &update);
        }
    }

    /// Keeps a live call's picture of the machine honest.
    ///
    /// A device the user picked can disappear mid-call — a Bluetooth headset switching off, a camera
    /// unplugged, a sound card taken over by another app — and nothing here ends the call over it.
    /// The stream behind it is rebound to the system default, a camera that stopped is stopped for
    /// the peer as well, and the snapshot carries the reason, so the picker stops claiming a device
    /// that is not there.
    ///
    /// The fresh list is also published when it changed, so a device that came or went shows up in
    /// the pickers without anyone pressing anything.
    pub(super) async fn reconcile_call(&mut self) {
        if self
            .call
            .as_ref()
            .is_none_or(|runtime| !runtime.call.phase().is_live())
        {
            return;
        }
        // Once per heartbeat, the engine's own counters for this call. A call that is up but carrying
        // no audio is otherwise indistinguishable in a bug report from one where nobody spoke, and
        // these numbers are what tell the two apart.
        if let Some(runtime) = self.call.as_mut() {
            runtime.call.check_video_queue();
            runtime.call.budget_video_fec();
            runtime.call.log_media_stats();
        }
        // Discovery opens every audio device and asks every camera to name itself, so it runs on a
        // blocking thread rather than on the worker's async loop: a slow device must not hold up
        // message handling or the call's own events.
        let devices = discover_devices().await;
        // Read before the fresh list replaces it: a device that just went away is still named in
        // here by the description the user saw.
        let known = self.call_devices.clone();
        if devices != known {
            self.call_devices = devices.clone();
            self.emit(Event::CallDevices(Box::new(devices.clone())));
        }
        let mut lost = Vec::new();
        if let Some(runtime) = self.call.as_mut() {
            lost.extend(runtime.call.take_stream_fallbacks());
            lost.extend(runtime.call.verify_devices(&devices).await);
            if runtime.call.camera_stalled()
                && let Err(error) = runtime.call.set_camera(false).await
            {
                log::warn!("[CALL] the peer was not told the camera stopped: {error}");
            }
        }
        if !lost.is_empty() {
            // Worded by the call screen, which knows the reader's language; the log says it plainly.
            let lost: Vec<calls::LostDevice> = lost
                .into_iter()
                .map(|(kind, id)| {
                    let name = calls::device_name(&known, kind, &id);
                    log::warn!("[CALL] {kind:?} \"{name}\" is not available any more");
                    calls::LostDevice { kind, name }
                })
                .collect();
            if let Some(runtime) = self.call.as_mut() {
                runtime.call.set_lost_devices(lost);
            }
        }
        // Compared against the snapshot the UI was handed, so a camera that stopped on its own or a
        // mute another device set reaches the screen even though no user pressed anything.
        let current = self
            .call
            .as_ref()
            .map(|runtime| self.canonical_call(runtime.call.update()));
        let changed = self
            .call
            .as_mut()
            .zip(current)
            .and_then(|(runtime, current)| {
                let changed = current != runtime.last;
                runtime.last = current.clone();
                changed.then_some(current)
            });
        if let Some(update) = changed {
            self.emit_call(update);
        }
    }
}

/// How many group calls ringing on the phone only are kept at once. A call the phone never says
/// the end of is let go once this many newer ones have come.
const MAX_PHONE_ONLY_CALLS: usize = 4;

/// A call to file among a chat's messages.
pub(super) struct CallEntry {
    pub chat: ChatId,
    /// Who placed a call we did not: the chat itself for a one-to-one call, the member who rang
    /// for a group's. `None` reads as the chat.
    pub caller: Option<String>,
    pub call_id: String,
    pub from_me: bool,
    pub timestamp: i64,
    pub video: bool,
    pub status: calls::CallOutcome,
    pub duration: u64,
    pub group: bool,
    /// How many people were on a group call, this account included; `None` when the entry's
    /// record does not name who took part.
    pub participants: Option<u32>,
}

impl Worker {
    /// Files a call among its chat's messages, as WhatsApp shows calls, unless
    /// the chat already has an entry for it (the phone's own call-log
    /// message). The entry is local: an id of ZapFast's own, never sent, never
    /// unread.
    pub(super) fn file_call_entry(&mut self, entry: CallEntry) {
        if entry.chat.is_empty() || entry.call_id.is_empty() || entry.timestamp <= 0 {
            return;
        }
        let id = format!("{}{}", crate::model::CALL_ENTRY_PREFIX, entry.call_id);
        match self
            .archive
            .call_entry_near(&entry.chat, entry.timestamp, entry.from_me, &id)
        {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => {
                log::warn!("[CALL] the chat's call entries could not be read: {error}");
                return;
            }
        }
        let link = entry.chat == crate::model::CALL_LINK_CHAT;
        if !link {
            self.ensure_chat(&entry.chat, None);
        }
        let sender = if entry.from_me {
            self.me()
        } else {
            entry.caller.clone().unwrap_or_else(|| entry.chat.clone())
        };
        let message = Message {
            id,
            chat: entry.chat.clone(),
            sender,
            sender_name: None,
            from_me: entry.from_me,
            timestamp: entry.timestamp,
            history_order: None,
            content: Content::Call {
                video: entry.video,
                status: entry.status,
                duration: entry.duration,
                group: entry.group,
                participants: entry.participants,
            },
            status: Delivery::None,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: Vec::new(),
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        };
        if let Err(error) = self.archive.insert_message(&message, None) {
            log::warn!("[CALL] a call entry could not be filed: {error}");
            return;
        }
        if link {
            // No chat shows it: the Calls page is where it goes.
            self.emit_call_log();
            return;
        }
        self.emit(Event::Messages {
            chat: entry.chat.clone(),
            messages: vec![message],
            older: false,
            complete: false,
        });
        self.emit_chat(&entry.chat);
    }

    /// Removes ZapFast's own entries for a call once the phone's own message
    /// for it is filed, so the chat shows the call once.
    pub(super) fn retire_local_call_twins(&mut self, message: &Message) {
        if !matches!(message.content, Content::Call { .. })
            || message.id.starts_with(crate::model::CALL_ENTRY_PREFIX)
        {
            return;
        }
        let twins =
            match self
                .archive
                .local_call_twins(&message.chat, message.timestamp, message.from_me)
            {
                Ok(twins) => twins,
                Err(error) => {
                    log::warn!("[CALL] the chat's call entries could not be read: {error}");
                    return;
                }
            };
        for id in twins {
            match self.archive.delete_message(&message.chat, &id) {
                Ok(_) => self.emit(Event::MessageDeleted {
                    chat: message.chat.clone(),
                    id,
                }),
                Err(error) => log::warn!("[CALL] a duplicate call entry stays: {error}"),
            }
        }
    }

    /// Files a call this worker took part in once it is over.
    fn file_finished_call(&mut self, call: &Call, update: &CallUpdate) {
        let Some(status) = update.outcome else {
            return;
        };
        // A call joined through a link belongs to no chat, and is filed under
        // the id kept for such calls.
        let link = update.group.as_ref().is_some_and(|group| group.link);
        // Everyone the call service's roster reported as on the call, minus our own entry, plus
        // this account when its own outcome says it was there: the service lists our entry on a
        // call we started, and a call that only rang here must not put us on it.
        let participants = call.group_people().and_then(|people| {
            let me = self.me();
            // The archive's canonical id is what tells one person from another: a participant
            // named once by privacy id and once by phone number is counted once.
            let mut others: Vec<String> = people
                .iter()
                .map(|id| self.canonical_str(id.as_str()))
                .filter(|id| *id != me)
                .collect();
            others.sort_unstable();
            others.dedup();
            // A call cut off before it was up never put this account on it, whatever the
            // service calls the outcome.
            let was_up = update.started.is_some();
            let on_call = account_was_on_call(status)
                && (status != calls::CallOutcome::ConnectionLost || was_up);
            let count = others.len() as u32 + u32::from(on_call);
            (count > 0).then_some(count)
        });
        self.file_call_entry(CallEntry {
            chat: if link {
                crate::model::CALL_LINK_CHAT.to_owned()
            } else {
                update.chat.clone()
            },
            caller: update.group.as_ref().and_then(|group| group.caller.clone()),
            call_id: call.call_id().to_owned(),
            from_me: update.direction == crate::model::CallDirection::Outgoing,
            timestamp: call.began(),
            video: call.carried_video(),
            status,
            duration: update
                .started
                .map_or(0, |started| started.elapsed().as_secs()),
            group: call.is_group(),
            participants,
        });
    }

    /// Files a call the phone reports as missed while this device was away.
    pub(super) fn missed_call_entry(&mut self, call: &whatsapp_rust::types::call::MissedCall) {
        // A group call's notice can come from the call service itself (`@call`), which is no chat
        // to file it in; the phone's own call log files that one in its group.
        if matches!(call.from.server, whatsapp_rust::prelude::Server::Call) {
            return;
        }
        let chat = self.canonical(&call.from);
        self.file_call_entry(CallEntry {
            chat,
            caller: None,
            call_id: call.call_id.clone(),
            from_me: false,
            timestamp: call.timestamp.timestamp(),
            video: false,
            status: calls::CallOutcome::Missed,
            duration: 0,
            group: false,
            // The notice names nobody on the call.
            participants: None,
        });
    }

    /// Files a call from the phone's call log: synced as it happens, or sent
    /// with history when this device was linked.
    pub(super) fn call_log_entry(
        &mut self,
        call_id: &str,
        creator: Option<&Jid>,
        from_me: Option<bool>,
        record: &wa::CallLogRecord,
    ) {
        use wa::call_log_record::CallResult;
        let Some(from_me) = from_me else {
            return;
        };
        let Some(status) = record_status(record, from_me) else {
            return;
        };
        let group = record
            .group_jid
            .as_deref()
            .and_then(|jid| jid.parse::<Jid>().ok());
        let chat = match (&group, from_me) {
            (Some(group), _) => self.canonical(group),
            (None, false) => match creator {
                Some(creator) => self.canonical(&creator.to_non_ad()),
                None => return,
            },
            // Our own call: the other side is its participant.
            (None, true) => match record
                .participants
                .iter()
                .filter_map(|participant| participant.user_jid.as_deref())
                .filter_map(|jid| jid.parse::<Jid>().ok())
                .next()
            {
                Some(peer) => self.canonical(&peer.to_non_ad()),
                None => return,
            },
        };
        // The phone's record names who placed a group call it did not.
        let caller = (group.is_some() && !from_me)
            .then(|| creator.map(|creator| self.canonical(&creator.to_non_ad())))
            .flatten();
        let participants = group.is_some().then(|| {
            counted_people(
                record
                    .participants
                    .iter()
                    .filter(|participant| participant.call_result == Some(CallResult::CONNECTED))
                    .filter_map(|participant| participant.user_jid.as_deref()),
                account_was_on_call(status),
            )
        });
        self.file_call_entry(CallEntry {
            chat,
            caller,
            call_id: call_id.to_owned(),
            from_me,
            timestamp: record.start_time.unwrap_or(0),
            video: record.is_video.unwrap_or(false),
            status,
            duration: record.duration.unwrap_or(0).max(0) as u64,
            group: group.is_some(),
            participants: participants.flatten(),
        });
    }

    /// Hands the Calls page this account's calls.
    pub(super) fn emit_call_log(&mut self) {
        match self.archive.call_log(crate::archive::CALL_LOG_LIMIT) {
            Ok(entries) => self.emit(Event::CallLog(entries)),
            Err(error) => log::warn!("[CALL] the call log could not be read: {error}"),
        }
    }
}

/// What a call-log record says became of a call; `None` for a call that has
/// not happened yet or is still going on.
fn record_status(record: &wa::CallLogRecord, from_me: bool) -> Option<calls::CallOutcome> {
    use calls::CallOutcome as Status;
    use wa::call_log_record::CallResult;
    Some(match record.call_result? {
        CallResult::CONNECTED => Status::Answered,
        CallResult::REJECTED => Status::Declined,
        CallResult::ACCEPTEDELSEWHERE => Status::AnsweredElsewhere,
        CallResult::MISSED => Status::Missed,
        CallResult::CANCELLED | CallResult::ABANDONED | CallResult::UNAVAILABLE => {
            if from_me {
                Status::NoAnswer
            } else {
                Status::Missed
            }
        }
        CallResult::INVALID | CallResult::FAILED => Status::Failed,
        CallResult::UPCOMING | CallResult::ONGOING => return None,
        _ => return None,
    })
}

/// A call-log message in a chat, as WhatsApp files a call there.
pub(super) fn call_log_content(log: &wa::message::CallLogMessage) -> Content {
    use calls::CallOutcome as Status;
    use wa::message::call_log_message::CallOutcome;
    let status = match log.call_outcome {
        Some(CallOutcome::CONNECTED) | None => Status::Answered,
        Some(CallOutcome::REJECTED) => Status::Declined,
        Some(CallOutcome::ACCEPTED_ELSEWHERE) => Status::AnsweredElsewhere,
        Some(CallOutcome::FAILED) => Status::Failed,
        Some(CallOutcome::ONGOING) => Status::Answered,
        // Missed, silenced while Do Not Disturb was on or for an unknown
        // caller: either way nobody answered.
        _ => Status::Missed,
    };
    let group = log.participants.len() > 1;
    let participants = if group {
        counted_people(
            log.participants
                .iter()
                .filter(|participant| participant.call_outcome == Some(CallOutcome::CONNECTED))
                .filter_map(|participant| participant.jid.as_deref()),
            account_was_on_call(status),
        )
    } else {
        None
    };
    Content::Call {
        video: log.is_video.unwrap_or(false),
        status,
        duration: log.duration_secs.unwrap_or(0).max(0) as u64,
        group,
        participants,
    }
}

/// Whether this account was on the call: answered here or on another of its devices, or cut off
/// after the call was up. One that only rang, one refused, and one that never connected did not
/// put this account on it, and so must not be counted among the people who were.
fn account_was_on_call(status: calls::CallOutcome) -> bool {
    use calls::CallOutcome as Status;
    matches!(
        status,
        Status::Answered | Status::AnsweredElsewhere | Status::ConnectionLost
    )
}

/// How many people a call's own record says were on it: the people it lists as having connected,
/// each counted once, plus this account when the record says it was on the call.
///
/// A record can list a person's device more than once, so the ids are compared without their
/// device suffix. Only the count comes back: who was on the call is never kept.
fn counted_people<'a>(ids: impl Iterator<Item = &'a str>, ours: bool) -> Option<u32> {
    let mut people: Vec<String> = Vec::new();
    for id in ids {
        let Ok(jid) = id.parse::<Jid>() else {
            continue;
        };
        let person = jid.to_non_ad_string();
        if !people.contains(&person) {
            people.push(person);
        }
    }
    let count = people.len() as u32 + u32::from(ours);
    (count > 0).then_some(count)
}

/// A missed call the phone filed as a bare notice in a chat's history.
pub(super) fn missed_call_stub(stub: wa::web_message_info::StubType) -> Option<Content> {
    use wa::web_message_info::StubType;
    let (video, group) = match stub {
        StubType::CALL_MISSED_VOICE => (false, false),
        StubType::CALL_MISSED_VIDEO => (true, false),
        StubType::CALL_MISSED_GROUP_VOICE => (false, true),
        StubType::CALL_MISSED_GROUP_VIDEO => (true, true),
        _ => return None,
    };
    Some(Content::Call {
        video,
        status: calls::CallOutcome::Missed,
        duration: 0,
        group,
        // A notice that the call was missed carries no roster.
        participants: None,
    })
}

/// Device discovery off the worker thread: enumerating opens every audio device to name it, so it
/// runs on a blocking thread rather than on the async loop, where a slow device would hold up
/// message handling and the call's own events.
async fn discover_devices() -> calls::DeviceList {
    tokio::task::spawn_blocking(calls::devices)
        .await
        .unwrap_or_default()
}

/// What the window says when the call service refused a group call we started. The plain words
/// are a catalog key the window translates; any other refusal keeps the generic failure.
fn group_refusal_text(refusal: &calls::GroupRefusal) -> String {
    if refusal.not_enabled {
        calls::GROUP_CALLS_NOT_ENABLED.to_owned()
    } else {
        format!("The call could not be started: {}", refusal.detail)
    }
}

/// Whether a chat can take a one-to-one call.
///
/// The interface only draws the call buttons on a person's chat or a group's (which goes to
/// [`Worker::start_group_call`]), but the worker is the boundary rather than the interface: a
/// channel or a broadcast list reaching the 1:1 builder would be refused by the protocol at best
/// and misbehave at worst, so the JID is checked here as well.
fn callable_chat(chat: &str) -> bool {
    matches!(
        crate::model::ChatKind::from_id(chat),
        crate::model::ChatKind::Direct
    )
}

/// Whether an offer may ring in `chat`: a one-to-one offer in a direct chat, a group call's in its
/// group, or, for a call among several people with no group behind it, in its creator's chat.
fn rings_in(chat: &str, group: bool) -> bool {
    match crate::model::ChatKind::from_id(chat) {
        crate::model::ChatKind::Direct => true,
        crate::model::ChatKind::Group => group,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A group call reaches the window under the app's own ids, without our own roster entry, and
    /// is filed in its group, under the member who rang, once it is over.
    #[test]
    fn a_group_call_is_drawn_and_filed_under_its_group() {
        use crate::calls::test_offers::{GROUP, group_offer, member};
        let (mut worker, events, _, _) = super::super::receipt_tests::worker();
        worker.archive.ensure_chat(GROUP, "Book club").unwrap();
        // The caller's privacy id is known to be this phone number; ours is our own.
        worker
            .lid_to_pn
            .insert("200000000000001".to_owned(), "15550002222".to_owned());
        worker.me_lid = Some("200000000000009@lid".to_owned());
        let incoming = group_offer(vec![
            member("200000000000001", true),
            member("200000000000002", true),
            member("200000000000009", true),
        ]);
        let call = Call::ringing(GROUP.to_owned(), Box::new(incoming), false);
        let update = call.update();
        worker.call = Some(CallRuntime::new(call, None));
        worker.emit_call(update);
        let drawn = events
            .try_iter()
            .find_map(|event| match event {
                Event::Call(update) => Some(update),
                _ => None,
            })
            .expect("the call rings");
        assert_eq!(drawn.chat, GROUP);
        let group = drawn.group.expect("a group call");
        assert_eq!(group.caller.as_deref(), Some("15550002222@s.whatsapp.net"));
        let ids: Vec<&str> = group.participants.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            ids,
            ["15550002222@s.whatsapp.net", "200000000000002@lid"],
            "named by phone number where known, and without us"
        );

        // A member's picture reaches the window under their chat id, and turns their tile.
        let image = std::sync::Arc::new(egui::ColorImage::filled([2, 2], egui::Color32::BLACK));
        worker.call_frame(VideoTick::RemoteFrom {
            sender: "200000000000001:5@lid".parse().unwrap(),
            image,
        });
        let events: Vec<Event> = events.try_iter().collect();
        assert!(events.iter().any(|event| matches!(
            event,
            Event::CallVideo { from: Some(id), remote: Some(_), .. }
                if id == "15550002222@s.whatsapp.net"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            Event::Call(update) if update.group.as_ref().is_some_and(|group| group.participants[0].video)
        )));

        // The caller gives up before we answer: a missed group call, in the group.
        assert!(worker.call_resolved(crate::calls::test_offers::CALL_ID, None));
        let filed = worker.archive.call_log(10).unwrap();
        assert_eq!(filed.len(), 1);
        let entry = &filed[0];
        assert_eq!(entry.chat, GROUP);
        assert_eq!(entry.sender, "15550002222@s.whatsapp.net");
        assert!(!entry.from_me);
        assert!(matches!(
            entry.content,
            Content::Call {
                group: true,
                status: calls::CallOutcome::Missed,
                ..
            }
        ));
    }

    /// With group calls off, a group's offer never becomes this worker's call: the window is told
    /// who is calling, nothing is sent back, the worker stays free for another call, and the call
    /// is filed in its group as it ends, as one that rang here and was answered on the phone.
    #[tokio::test]
    async fn with_group_calls_off_a_group_call_rings_on_the_phone_only() {
        use crate::calls::test_offers::{CALL_ID, GROUP, group_offer, member};
        let _off = calls::override_group_calls(false);
        let (mut worker, events, _, _) = super::super::receipt_tests::worker();
        worker.archive.ensure_chat(GROUP, "Book club").unwrap();
        worker
            .lid_to_pn
            .insert("200000000000001".to_owned(), "15550002222".to_owned());
        let incoming = group_offer(vec![
            member("200000000000001", true),
            member("200000000000002", false),
        ]);
        worker.call_signaling(&incoming).await;
        assert!(worker.call.is_none(), "the worker stays free");
        let first: Vec<Event> = events.try_iter().collect();
        assert!(
            !first.iter().any(|event| matches!(event, Event::Call(_))),
            "no call screen"
        );
        let told = first
            .iter()
            .find_map(|event| match event {
                Event::PhoneOnlyCall(update) => Some(update),
                _ => None,
            })
            .expect("the window is told");
        assert_eq!(told.chat, GROUP);
        assert_eq!(told.phase, calls::CallPhase::Incoming);
        assert_eq!(
            told.group
                .as_ref()
                .and_then(|group| group.caller.as_deref()),
            Some("15550002222@s.whatsapp.net")
        );
        // The same offer again is the same call.
        worker.call_signaling(&incoming).await;
        assert_eq!(worker.phone_only_calls.len(), 1);

        // Answered on the phone: filed in the group, and the window takes its word back.
        assert!(worker.call_resolved(CALL_ID, Some(calls::CallOutcome::AnsweredElsewhere)));
        assert!(worker.phone_only_calls.is_empty());
        let filed = worker.archive.call_log(10).unwrap();
        assert_eq!(filed.len(), 1);
        assert_eq!(filed[0].chat, GROUP);
        assert_eq!(filed[0].sender, "15550002222@s.whatsapp.net");
        assert!(matches!(
            filed[0].content,
            Content::Call {
                group: true,
                status: calls::CallOutcome::AnsweredElsewhere,
                ..
            }
        ));
        assert!(events.try_iter().any(|event| matches!(
            event,
            Event::PhoneOnlyCall(update) if !update.phase.is_live()
        )));
    }

    /// With group calls off, a group call or a call link cannot be started here even if asked:
    /// the window hears it was not placed, and a link says why.
    #[tokio::test]
    async fn with_group_calls_off_nothing_starts_a_group_call() {
        let _off = calls::override_group_calls(false);
        let (mut worker, events, _, _) = super::super::receipt_tests::worker();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        worker
            .start_call(
                "120363000000000009@g.us".to_owned(),
                false,
                7,
                cancel.clone(),
            )
            .await;
        worker
            .join_call_link("AbCdEf1234567890".to_owned(), true, 8, cancel)
            .await;
        assert!(worker.call.is_none());
        let events: Vec<Event> = events.try_iter().collect();
        for generation in [7, 8] {
            assert!(
                events.iter().any(|event| matches!(
                    event,
                    Event::Call(update) if update.generation == generation && !update.phase.is_live()
                )),
                "{generation} was not placed"
            );
        }
        assert!(events.iter().any(
            |event| matches!(event, Event::Error(message) if message == calls::CALL_LINKS_OFF)
        ));
    }

    /// A call joined through a link is filed under the id kept for such calls, which no chat
    /// has, and reaches the Calls page without making a chat of it.
    #[test]
    fn a_call_link_call_is_filed_for_the_calls_page_only() {
        let (mut worker, events, _, _) = super::super::receipt_tests::worker();
        worker.file_call_entry(CallEntry {
            chat: crate::model::CALL_LINK_CHAT.to_owned(),
            caller: None,
            call_id: "LINKCALL".to_owned(),
            from_me: true,
            timestamp: 1_000,
            video: true,
            status: calls::CallOutcome::Answered,
            duration: 75,
            group: true,
            participants: None,
        });
        let filed = worker.archive.call_log(10).unwrap();
        assert_eq!(filed.len(), 1);
        assert_eq!(filed[0].chat, crate::model::CALL_LINK_CHAT);
        assert!(
            worker
                .archive
                .chat(crate::model::CALL_LINK_CHAT)
                .unwrap()
                .is_none(),
            "no chat is made for it"
        );
        let events: Vec<Event> = events.try_iter().collect();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::CallLog(_)))
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Event::Messages { .. } | Event::ChatUpdated(_)))
        );
    }

    #[test]
    fn a_group_call_rings_in_its_group_and_nowhere_else() {
        assert!(rings_in("15551234567@s.whatsapp.net", false));
        assert!(
            rings_in("15551234567@s.whatsapp.net", true),
            "a call with no group"
        );
        assert!(rings_in("12345-67890@g.us", true));
        assert!(
            !rings_in("12345-67890@g.us", false),
            "a one-to-one offer never rings as a group's"
        );
        assert!(!rings_in("1234567890@broadcast", true));
        assert!(!rings_in("1234567890@newsletter", true));
    }

    #[test]
    fn only_a_one_to_one_chat_can_be_called() {
        assert!(callable_chat("15551234567@s.whatsapp.net"));
        assert!(callable_chat("123456789012345@lid"));
        assert!(!callable_chat("12345-67890@g.us"));
        assert!(!callable_chat("1234567890@broadcast"));
        assert!(!callable_chat("1234567890@newsletter"));
    }

    #[test]
    fn the_phones_call_log_says_how_each_call_ended() {
        use calls::CallOutcome as Status;
        use wa::call_log_record::CallResult;
        let record = |result| {
            let mut record = wa::CallLogRecord::default();
            record.call_result = result;
            record
        };
        assert_eq!(
            record_status(&record(Some(CallResult::CONNECTED)), true),
            Some(Status::Answered)
        );
        assert_eq!(
            record_status(&record(Some(CallResult::REJECTED)), false),
            Some(Status::Declined)
        );
        assert_eq!(
            record_status(&record(Some(CallResult::ACCEPTEDELSEWHERE)), false),
            Some(Status::AnsweredElsewhere)
        );
        // A call nobody picked up: unanswered when we placed it, missed when
        // it rang here.
        assert_eq!(
            record_status(&record(Some(CallResult::CANCELLED)), true),
            Some(Status::NoAnswer)
        );
        assert_eq!(
            record_status(&record(Some(CallResult::CANCELLED)), false),
            Some(Status::Missed)
        );
        assert_eq!(
            record_status(&record(Some(CallResult::FAILED)), true),
            Some(Status::Failed)
        );
        // Calls still to come or still going on are not filed yet.
        assert_eq!(
            record_status(&record(Some(CallResult::ONGOING)), true),
            None
        );
        assert_eq!(record_status(&record(None), true), None);
    }

    #[test]
    fn a_call_log_message_reads_as_a_call() {
        use wa::message::call_log_message::CallOutcome;
        let mut log = wa::message::CallLogMessage::default();
        log.is_video = Some(true);
        log.call_outcome = Some(CallOutcome::CONNECTED);
        log.duration_secs = Some(754);
        assert!(matches!(
            call_log_content(&log),
            Content::Call {
                video: true,
                status: calls::CallOutcome::Answered,
                duration: 754,
                group: false,
                participants: None,
            }
        ));
        let mut missed = wa::message::CallLogMessage::default();
        missed.call_outcome = Some(CallOutcome::MISSED);
        assert!(matches!(
            call_log_content(&missed),
            Content::Call {
                video: false,
                status: calls::CallOutcome::Missed,
                duration: 0,
                ..
            }
        ));
    }

    /// How many people a call's own record says were on it: the ids it lists as connected, each
    /// person once, plus this account when the record says it was on the call. A record that names
    /// nobody leaves the entry without a count rather than with zero people.
    #[test]
    fn a_records_people_are_counted_once_each() {
        let ids = [
            "4917000000001@lid",
            "4917000000001:4@lid",
            "not a jid",
            "4917000000002@s.whatsapp.net",
        ];
        assert_eq!(
            counted_people(ids.into_iter(), false),
            Some(2),
            "the same person under two device ids is one person, and a malformed id is nobody"
        );
        assert_eq!(
            counted_people(ids.into_iter(), true),
            Some(3),
            "the account is one of the people on the call"
        );
        assert_eq!(counted_people(["not a jid"].into_iter(), false), None);
        assert_eq!(counted_people(std::iter::empty(), false), None);
        assert_eq!(counted_people(std::iter::empty(), true), Some(1));
    }

    /// Whether the entry's own outcome puts this account on the call: a call that only rang, one
    /// refused, and one that never connected did not.
    #[test]
    fn only_an_answered_call_puts_this_account_on_it() {
        use calls::CallOutcome as Status;
        for status in [
            Status::Answered,
            Status::AnsweredElsewhere,
            Status::ConnectionLost,
        ] {
            assert!(account_was_on_call(status), "{status:?}");
        }
        for status in [
            Status::Missed,
            Status::NoAnswer,
            Status::Declined,
            Status::DeclinedElsewhere,
            Status::Busy,
            Status::Failed,
        ] {
            assert!(!account_was_on_call(status), "{status:?}");
        }
    }

    /// A group's call-log message counts the people it lists as having connected, and only them:
    /// a member who refused, a member named twice, and an id that is not one are not people who
    /// were on the call.
    #[test]
    fn a_group_call_log_message_counts_who_connected() {
        use wa::message::call_log_message::{CallOutcome, CallParticipant};
        let participant = |jid: &str, outcome: CallOutcome| {
            let mut person = CallParticipant::default();
            person.jid = Some(jid.to_owned());
            person.call_outcome = Some(outcome);
            person
        };
        let mut log = wa::message::CallLogMessage::default();
        log.is_video = Some(true);
        log.call_outcome = Some(CallOutcome::CONNECTED);
        log.duration_secs = Some(754);
        log.participants = vec![
            participant("4917000000001@s.whatsapp.net", CallOutcome::CONNECTED),
            participant("4917000000001:4@s.whatsapp.net", CallOutcome::CONNECTED),
            participant("4917000000002@s.whatsapp.net", CallOutcome::REJECTED),
        ];
        assert!(
            matches!(
                call_log_content(&log),
                Content::Call {
                    group: true,
                    status: calls::CallOutcome::Answered,
                    participants: Some(2),
                    ..
                }
            ),
            "we were on it and so was one other person: {:?}",
            call_log_content(&log)
        );

        // Nobody connected: a missed group call shows no count rather than zero people.
        let mut missed = wa::message::CallLogMessage::default();
        missed.call_outcome = Some(CallOutcome::MISSED);
        missed.participants = vec![
            participant("4917000000001@s.whatsapp.net", CallOutcome::REJECTED),
            participant("4917000000002@s.whatsapp.net", CallOutcome::REJECTED),
        ];
        assert!(matches!(
            call_log_content(&missed),
            Content::Call {
                group: true,
                status: calls::CallOutcome::Missed,
                participants: None,
                ..
            }
        ));

        // A one-to-one call has nobody to count.
        let mut alone = wa::message::CallLogMessage::default();
        alone.call_outcome = Some(CallOutcome::CONNECTED);
        alone.participants = vec![participant(
            "4917000000001@s.whatsapp.net",
            CallOutcome::CONNECTED,
        )];
        assert!(matches!(
            call_log_content(&alone),
            Content::Call {
                group: false,
                participants: None,
                ..
            }
        ));
    }

    /// The phone's own record of a group call files how many people were on it, and only that:
    /// no name, number, or id of anyone on the call is kept.
    #[tokio::test]
    async fn the_phones_group_record_files_how_many_were_on_the_call() {
        use wa::call_log_record::{CallResult, ParticipantInfo};
        let (mut worker, _events, _inbox, _wa) = super::super::receipt_tests::worker();
        let group = "120363000000000009@g.us";
        let person = |jid: &str, result: CallResult| {
            let mut info = ParticipantInfo::default();
            info.user_jid = Some(jid.to_owned());
            info.call_result = Some(result);
            info
        };
        let mut record = wa::CallLogRecord::default();
        record.call_result = Some(CallResult::CONNECTED);
        record.group_jid = Some(group.to_owned());
        record.duration = Some(754);
        record.start_time = Some(1_700_000_000);
        record.participants = vec![
            person("4917000000001@s.whatsapp.net", CallResult::CONNECTED),
            person("4917000000001:9@s.whatsapp.net", CallResult::CONNECTED),
            person("4917000000002@s.whatsapp.net", CallResult::REJECTED),
        ];
        worker.call_log_entry("CALLGROUP1", None, Some(true), &record);
        let filed = worker.archive.call_log(10).unwrap();
        assert_eq!(filed.len(), 1);
        assert_eq!(filed[0].chat, group);
        assert!(
            matches!(
                filed[0].content,
                Content::Call {
                    group: true,
                    status: calls::CallOutcome::Answered,
                    participants: Some(2),
                    ..
                }
            ),
            "this account and one other person: {:?}",
            filed[0].content
        );
        let stored = serde_json::to_string(&filed[0].content).unwrap();
        assert!(stored.contains("\"participants\":2"), "{stored}");
        assert!(
            !stored.contains("4917000000001") && !stored.contains("4917000000002"),
            "only the number is kept: {stored}"
        );
    }

    /// A group call this device handled is filed with how many people were on it. One answered
    /// here counts this account with the members the roster named; one that only rang, and one
    /// cut off before it came up, count the members and not us.
    #[tokio::test]
    async fn a_group_call_entry_says_how_many_people_were_on_it() {
        use crate::calls::test_offers::{CALL_ID, GROUP, group_offer, member};
        use calls::CallOutcome as Status;
        let _on = calls::override_group_calls(true);
        for (outcome, expected) in [
            (Some(Status::Answered), 2),
            (None, 1),
            (Some(Status::ConnectionLost), 1),
        ] {
            let (mut worker, _events, _, _) = super::super::receipt_tests::worker();
            worker.archive.ensure_chat(GROUP, "Book club").unwrap();
            worker.me_lid = Some("200000000000009@lid".to_owned());
            let incoming = group_offer(vec![
                member("200000000000001", true),
                member("200000000000002", false),
                member("200000000000009", true),
            ]);
            worker.call_signaling(&incoming).await;
            assert!(worker.call.is_some(), "the group's call rings here");
            assert!(worker.call_resolved(CALL_ID, outcome));
            let filed = worker.archive.call_log(10).unwrap();
            assert_eq!(filed.len(), 1);
            assert!(
                matches!(
                    filed[0].content,
                    Content::Call {
                        group: true,
                        participants: Some(count),
                        ..
                    } if count == expected
                ),
                "{outcome:?}: the member on the roster, our own entry left out, and this account only when it answered: {:?}",
                filed[0].content
            );
        }
    }

    #[test]
    fn only_missed_call_notices_become_calls() {
        use wa::web_message_info::StubType;
        assert!(matches!(
            missed_call_stub(StubType::CALL_MISSED_GROUP_VIDEO),
            Some(Content::Call {
                video: true,
                group: true,
                status: calls::CallOutcome::Missed,
                ..
            })
        ));
        assert!(matches!(
            missed_call_stub(StubType::CALL_MISSED_VOICE),
            Some(Content::Call {
                video: false,
                group: false,
                ..
            })
        ));
        assert!(missed_call_stub(StubType::GROUP_CREATE).is_none());
    }
}
