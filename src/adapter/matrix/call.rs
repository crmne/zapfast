//! One-to-one voice calls over WebRTC.
//!
//! The homeserver's TURN service supplies the ICE servers, and the whole SDP
//! (candidates included) travels in the version 0 `m.call.invite` and
//! `m.call.answer` events, so no trickle signalling is needed. Audio is Opus
//! at 48 kHz, encoded and decoded with the same crate the voice notes use.

use std::num::NonZero;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use bytes::Bytes;
use rodio::Source;
use tokio::sync::mpsc;
use webrtc::api::APIBuilder;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MIME_TYPE_OPUS, MediaEngine};
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::media::Sample;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::TrackLocal;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

use crate::backend::CallState;

/// Opus runs at this rate over WebRTC too.
const SAMPLE_RATE: u32 = 48_000;
/// Samples in one 20 ms frame.
const FRAME: usize = 960;
/// The encoder bitrate, matching voice notes.
const BITRATE: i32 = 32_000;

/// Maps the homeserver's TURN URIs to one ICE server entry.
pub fn ice_servers(uris: &[String], username: &str, credential: &str) -> Vec<RTCIceServer> {
    if uris.is_empty() {
        return Vec::new();
    }
    vec![RTCIceServer {
        urls: uris.to_vec(),
        username: username.to_owned(),
        credential: credential.to_owned(),
    }]
}

/// A random call id. Matrix only asks that both sides use the same string.
pub fn call_id() -> String {
    let value = rand::random::<u64>() % 1_000_000_000_000_000_000;
    value.to_string()
}

/// Whether an invite names this account, or the whole room.
pub fn invites_me(invitee: Option<&str>, me: &str) -> bool {
    invitee.is_none_or(|invitee| invitee == me)
}

/// The live audio side of a call: the peer connection and the state changes
/// it reports.
pub struct Call {
    connection: Arc<RTCPeerConnection>,
    states: tokio::sync::Mutex<mpsc::UnboundedReceiver<CallState>>,
    stop: Arc<AtomicBool>,
}

impl Call {
    /// Places a call: builds the offer, waits for the candidates to gather,
    /// and returns the local description to send in the invite.
    pub async fn place(servers: Vec<RTCIceServer>) -> Result<(Self, String)> {
        let (connection, track, states, stop, audio) = setup(servers).await?;
        let local: Arc<dyn TrackLocal + Send + Sync> = track.clone();
        connection.add_track(local).await?;
        let offer = connection.create_offer(None).await?;
        connection.set_local_description(offer).await?;
        let sdp = gather(&connection).await?;
        start_microphone(audio, stop.clone());
        Ok((
            Self {
                connection,
                states: tokio::sync::Mutex::new(states),
                stop,
            },
            sdp,
        ))
    }

    /// Answers a call: takes the remote offer and returns the local answer.
    pub async fn answer(servers: Vec<RTCIceServer>, offer: String) -> Result<(Self, String)> {
        let (connection, track, states, stop, audio) = setup(servers).await?;
        connection
            .set_remote_description(RTCSessionDescription::offer(offer)?)
            .await?;
        let local: Arc<dyn TrackLocal + Send + Sync> = track.clone();
        connection.add_track(local).await?;
        let answer = connection.create_answer(None).await?;
        connection.set_local_description(answer).await?;
        let sdp = gather(&connection).await?;
        start_microphone(audio, stop.clone());
        Ok((
            Self {
                connection,
                states: tokio::sync::Mutex::new(states),
                stop,
            },
            sdp,
        ))
    }

    /// Applies the answer the other side sent for our offer.
    pub async fn accept_answer(&self, answer: String) -> Result<()> {
        self.connection
            .set_remote_description(RTCSessionDescription::answer(answer)?)
            .await?;
        Ok(())
    }

    /// The next state the peer connection reported, if any.
    pub async fn next_state(&self) -> Option<CallState> {
        self.states.lock().await.recv().await
    }

    /// Stops the audio and closes the connection.
    pub async fn close(&self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.connection.close().await;
    }
}

type Parts = (
    Arc<RTCPeerConnection>,
    Arc<TrackLocalStaticSample>,
    mpsc::UnboundedReceiver<CallState>,
    Arc<AtomicBool>,
    mpsc::UnboundedSender<(Bytes, Duration)>,
);

/// Builds the peer connection and the outgoing Opus track. Incoming audio is
/// decoded in the track handler into a small player thread.
async fn setup(servers: Vec<RTCIceServer>) -> Result<Parts> {
    let mut media = MediaEngine::default();
    media.register_default_codecs()?;
    let mut registry = Registry::new();
    registry = register_default_interceptors(registry, &mut media)?;
    let api = APIBuilder::new()
        .with_media_engine(media)
        .with_interceptor_registry(registry)
        .build();
    let connection = Arc::new(
        api.new_peer_connection(RTCConfiguration {
            ice_servers: servers,
            ..Default::default()
        })
        .await?,
    );
    let track = Arc::new(TrackLocalStaticSample::new(
        RTCRtpCodecCapability {
            mime_type: MIME_TYPE_OPUS.to_owned(),
            clock_rate: SAMPLE_RATE,
            channels: 1,
            ..Default::default()
        },
        "audio".to_owned(),
        "zapfast".to_owned(),
    ));
    let (state_tx, state_rx) = mpsc::unbounded_channel();
    connection.on_peer_connection_state_change(Box::new(move |state| {
        let state_tx = state_tx.clone();
        Box::pin(async move {
            let ended = match state {
                RTCPeerConnectionState::Connected => Some(CallState::Connected {
                    since: jiff::Timestamp::now().as_second(),
                }),
                RTCPeerConnectionState::Failed => Some(CallState::Ended {
                    reason: "The call could not connect".to_owned(),
                }),
                RTCPeerConnectionState::Closed => Some(CallState::Ended {
                    reason: "The call ended".to_owned(),
                }),
                _ => None,
            };
            if let Some(ended) = ended {
                let _ = state_tx.send(ended);
            }
        })
    }));
    let stop = Arc::new(AtomicBool::new(false));
    let (audio_tx, mut audio_rx) = mpsc::unbounded_channel::<(Bytes, Duration)>();
    let writer = track.clone();
    tokio::spawn(async move {
        while let Some((data, duration)) = audio_rx.recv().await {
            let _ = writer
                .write_sample(&Sample {
                    data,
                    duration,
                    ..Default::default()
                })
                .await;
        }
    });
    let stopped = stop.clone();
    connection.on_track(Box::new(move |remote, _, _| {
        let stopped = stopped.clone();
        Box::pin(async move {
            let Ok(mut decoder) = opus::Decoder::new(SAMPLE_RATE, opus::Channels::Mono) else {
                return;
            };
            let mut scratch = vec![0f32; 5760];
            let (samples_tx, samples_rx) = std::sync::mpsc::channel::<Vec<f32>>();
            let player = std::thread::Builder::new()
                .name("zapfast-call-out".to_owned())
                .spawn(move || {
                    let Ok(device) = rodio::DeviceSinkBuilder::open_default_sink() else {
                        return;
                    };
                    let player = rodio::Player::connect_new(device.mixer());
                    while let Ok(samples) = samples_rx.recv() {
                        player.append(rodio::buffer::SamplesBuffer::new(
                            NonZero::new(1).expect("mono"),
                            NonZero::new(SAMPLE_RATE).expect("48 kHz"),
                            samples,
                        ));
                    }
                });
            if player.is_err() {
                return;
            }
            while !stopped.load(Ordering::Relaxed) {
                let Ok((packet, _)) = remote.read_rtp().await else {
                    break;
                };
                let Ok(frames) = decoder.decode_float(&packet.payload, &mut scratch, false) else {
                    continue;
                };
                if samples_tx.send(scratch[..frames].to_vec()).is_err() {
                    break;
                }
            }
        })
    }));
    Ok((connection, track, state_rx, stop, audio_tx))
}

/// Waits until the ICE candidates have gathered, then returns the SDP with
/// them embedded.
async fn gather(connection: &Arc<RTCPeerConnection>) -> Result<String> {
    let mut complete = connection.gathering_complete_promise().await;
    let _ = complete.recv().await;
    connection
        .local_description()
        .await
        .map(|description| description.sdp)
        .context("The call has no local description")
}

/// Records the microphone and writes 20 ms Opus frames into the track.
fn start_microphone(audio: mpsc::UnboundedSender<(Bytes, Duration)>, stop: Arc<AtomicBool>) {
    let spawned = std::thread::Builder::new()
        .name("zapfast-call-mic".to_owned())
        .spawn(move || {
            let opened = rodio::microphone::MicrophoneBuilder::new()
                .default_device()
                .map_err(|error| format!("No microphone available: {error}"))
                .and_then(|builder| {
                    builder
                        .default_config()
                        .map_err(|error| format!("The microphone has no supported format: {error}"))
                })
                .and_then(|config| {
                    config
                        .open_stream()
                        .map_err(|error| format!("Could not open the microphone: {error}"))
                });
            let Ok(mut microphone) = opened else {
                log::warn!("The call microphone is not available");
                return;
            };
            let channels = microphone.channels().get() as usize;
            let Ok(mut encoder) =
                opus::Encoder::new(SAMPLE_RATE, opus::Channels::Mono, opus::Application::Voip)
            else {
                return;
            };
            let _ = encoder.set_bitrate(opus::Bitrate::Bits(BITRATE));
            let mut frame: Vec<f32> = Vec::with_capacity(FRAME);
            let mut packet = vec![0u8; 4000];
            while !stop.load(Ordering::Relaxed) {
                let Some(sample) = microphone.next() else {
                    break;
                };
                frame.push(sample);
                if frame.len() < FRAME * channels {
                    continue;
                }
                let mono = if channels == 1 {
                    std::mem::take(&mut frame)
                } else {
                    let down: Vec<f32> = frame
                        .chunks(channels)
                        .map(|chunk| chunk.iter().sum::<f32>() / channels as f32)
                        .collect();
                    frame.clear();
                    down
                };
                if let Ok(len) = encoder.encode_float(&mono, &mut packet)
                    && audio
                        .send((
                            Bytes::copy_from_slice(&packet[..len]),
                            Duration::from_millis(20),
                        ))
                        .is_err()
                {
                    break;
                }
            }
        });
    if let Err(error) = spawned {
        log::warn!("The call microphone could not start: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_uris_become_one_ice_server() {
        let servers = ice_servers(
            &[
                "turn:example.org:3478".to_owned(),
                "turns:example.org:5349".to_owned(),
            ],
            "user",
            "secret",
        );
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].urls.len(), 2);
        assert_eq!(servers[0].username, "user");
        assert!(ice_servers(&[], "user", "secret").is_empty());
    }

    #[test]
    fn call_ids_are_digit_strings() {
        let id = call_id();
        assert!(!id.is_empty());
        assert!(id.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn only_our_invites_are_answered() {
        assert!(invites_me(None, "@ada:example.org"));
        assert!(invites_me(Some("@ada:example.org"), "@ada:example.org"));
        assert!(!invites_me(Some("@bob:example.org"), "@ada:example.org"));
    }
}
