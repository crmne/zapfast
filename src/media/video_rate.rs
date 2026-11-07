//! How much a call's camera sends, adapted to the path the call runs over.
//!
//! The encoder used to run at one fixed 1.5 Mbit/s, whatever the path between the two sides could
//! carry. A path that carries less turns the surplus into loss and into access units the relay
//! connection's send queue throws away, and every lost unit costs the peer's decoder its reference
//! chain until the next keyframe arrives: the stutter a fixed rate produces on a real network.
//!
//! This starts below what a 720p picture needs, lowers the rate when the peer reports loss or the
//! send queue drops video, raises it slowly while the peer reports a clean path, and moves the
//! picture between three steps of size and frame rate as the rate crosses them. A step down is
//! taken at once; a step up only once the rate has held for a while, because a new picture size
//! starts with a keyframe and a keyframe is the most expensive thing to send.
//!
//! The rate chosen is what goes on the wire, and in a 1:1 call the library sends XOR forward error
//! correction beside the video: about one repair packet per eight video packets, one per four under
//! loss. So the encoder is given the rate less FEC's share, measured from what was sent.
//!
//! [`RateControl`] is plain state driven by the peer's reports, so it can be tested without a call;
//! [`RateShare`] is what it publishes for the capture thread, which reads it between frames.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// One step of picture size and frame rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tier {
    /// The picture's bounds, either way round: a portrait camera fits them turned upright.
    pub long_side: usize,
    pub short_side: usize,
    pub fps: u32,
    /// The most this step is sent at. A rate past it buys more from a larger picture than from a
    /// sharper small one, so it is where the next step up begins.
    pub max_bitrate: u32,
    /// How the log names it.
    pub name: &'static str,
}

/// The steps, smallest first. Every step keeps 25 frames a second: a call reads as live through
/// smooth motion more than through a larger picture, so a slower path gets a smaller picture, never
/// a choppier one. 720p only comes once the path clearly carries it.
pub const TIERS: [Tier; 3] = [
    Tier {
        long_side: 854,
        short_side: 480,
        fps: 25,
        max_bitrate: 600_000,
        name: "480p",
    },
    Tier {
        long_side: 960,
        short_side: 540,
        fps: 25,
        max_bitrate: 1_300_000,
        name: "540p",
    },
    Tier {
        long_side: 1280,
        short_side: 720,
        fps: 25,
        max_bitrate: 1_800_000,
        name: "720p",
    },
];

/// Below this a picture is not worth sending at any size here.
pub const MIN_BITRATE: u32 = 200_000;
/// Where a call starts: enough for a clean 540p picture, well under what the path is assumed to
/// carry until the peer's reports say otherwise.
pub const START_BITRATE: u32 = 700_000;

/// The rate at which the picture steps up from tier `n` to `n + 1`: the top of tier `n`.
const STEP_UP: [u32; 2] = [TIERS[0].max_bitrate, TIERS[1].max_bitrate];
/// The rate below which the picture steps down from tier `n + 1` to `n`. Lower than the step up,
/// so a rate hovering at a boundary does not flip the picture size back and forth.
const STEP_DOWN: [u32; 2] = [450_000, 1_100_000];
/// How long a rate has to hold before the picture grows, the first time.
const STEP_UP_HOLD: Duration = Duration::from_secs(4);
/// How soon after a step up a sign of congestion means the larger picture is what the path could
/// not carry. A larger picture starts with its largest frame, a keyframe at the new size, so a
/// path at its limit says so within a few reports.
const STEP_UP_PROBATION: Duration = Duration::from_secs(10);
/// The longest the hold before a step up grows to after steps up that failed.
const MAX_STEP_UP_HOLD: Duration = Duration::from_secs(64);
/// After a sign of congestion, how long before the rate may rise again.
const CONGESTION_HOLD: Duration = Duration::from_secs(3);
/// The rise for a clean report, and how often it may be applied.
const RISE: f64 = 1.08;
const RISE_EVERY: Duration = Duration::from_secs(1);
/// Where a rate that met loss is remembered as the path's limit: a little under it.
const LOSS_CEILING: f64 = 0.9;
/// The rise past that limit: a slow probe, since the path lost video there not long ago. A real
/// call climbed back to its old rate within ten seconds of each loss, and lost again there.
const PROBE_RISE: f64 = 1.02;
/// Reported loss above this is congestion; below `LOSS_CLEAN` the path is clean. Between the two
/// the rate is not cut, since a few percent is what a healthy wireless link loses anyway, but it
/// holds off rising as after congestion: a peer that never asks for a packet again freezes on any
/// loss its repair packets cannot rebuild, and in a live call the rate kept climbing through a
/// stretch of such losses, each one a frozen picture.
const LOSS_CONGESTED: f64 = 0.10;
const LOSS_CLEAN: f64 = 0.02;
/// What one round of send-queue drops costs the rate.
const DROP_CUT: f64 = 0.7;
/// How long a receiver estimate keeps capping the rate after the last one arrived.
const REMB_TTL: Duration = Duration::from_secs(10);
/// The bytes the peer must have received before what it says it receives is read: its rate starts
/// from nothing when the video does, and read earlier it held the call at the floor for seconds.
const RECEIVING_AFTER_BYTES: u32 = 500_000;
/// The share of what we sent that the peer must receive for the path to count as carrying it.
const RECEIVING_SHORT: f64 = 0.75;
/// How far back what we sent is measured, to set beside what the peer says it receives; at least
/// [`SENT_SPAN_MIN`] of it before anything is compared.
const SENT_SPAN: Duration = Duration::from_secs(3);
const SENT_SPAN_MIN: Duration = Duration::from_secs(1);
/// The share of what the peer receives that a path falling short is sent: a little under, so the
/// queue it built drains.
const RECEIVING_CAP: f64 = 0.9;
/// Our own audio streams the reports can name; a call has one, a renumbered one two.
const AUDIO_SSRC_MEMORY: usize = 4;
/// Round trip above the call's shortest that means a queue is filling on the path. A path at its
/// limit queues before it loses anything, and that queue is delay the peer sees: on a deep buffer,
/// seconds of it with no loss reported at all.
const QUEUE_DELAY: Duration = Duration::from_millis(250);
/// Round trip above the shortest at which the rate stops rising, short of cutting it.
const RISE_DELAY: Duration = Duration::from_millis(100);
/// What one queue-delay reading costs the rate, and how often it may: a queue takes a while to
/// drain once the rate is under what the path carries, so the next reports still show it.
const DELAY_CUT: f64 = 0.85;
const DELAY_CUT_EVERY: Duration = Duration::from_secs(1);
/// The least share of the rate the encoder is given. The library sends at most one FEC packet per
/// two video packets, a third of what goes out, so a measurement past that is noise.
const MIN_VIDEO_SHARE: f64 = 2.0 / 3.0;
/// The encoded video a measurement of FEC's share waits for: a few frames even at the lowest rate,
/// so one frame's last repair packet does not swing it. Until then the last share holds.
const FEC_MEASURE_BYTES: u64 = 32 * 1024;
/// Seconds between the NTP era (1900) and the Unix one.
const NTP_UNIX_OFFSET: u64 = 2_208_988_800;

/// What the capture thread reads between frames: the rate and the step, as the controller last
/// set them, and the share of that rate the encoder gets once FEC has had its own. And what it
/// reports back: frames it could not hand to the engine, and the bytes of those it could.
#[derive(Debug)]
pub struct RateShare {
    bitrate: AtomicU32,
    tier: AtomicUsize,
    /// The encoder's share of the rate, in millionths.
    video_share: AtomicU32,
    queue_drops: AtomicU32,
    sent_bytes: AtomicU64,
}

impl Default for RateShare {
    fn default() -> Self {
        let start = Setting::start();
        Self {
            bitrate: AtomicU32::new(start.bitrate),
            tier: AtomicUsize::new(start.tier),
            video_share: AtomicU32::new(millionths(configured_video_share(
                whatsapp_rust::voip::VideoFecConfig::default(),
            ))),
            queue_drops: AtomicU32::new(0),
            sent_bytes: AtomicU64::new(0),
        }
    }
}

/// The encoder's share of the rate a FEC configuration leaves before anything was measured: one
/// repair packet per `clean_group` video packets, about as large as they are.
pub fn configured_video_share(config: whatsapp_rust::voip::VideoFecConfig) -> f64 {
    if !config.enabled {
        return 1.0;
    }
    let group = f64::from(config.clean_group.clamp(2, 8));
    group / (group + 1.0)
}

fn millionths(share: f64) -> u32 {
    (share.clamp(0.0, 1.0) * 1e6).round() as u32
}

impl RateShare {
    pub fn load(&self) -> Setting {
        Setting {
            bitrate: self.bitrate.load(Ordering::Relaxed),
            tier: self.tier.load(Ordering::Relaxed).min(TIERS.len() - 1),
        }
    }

    fn store(&self, setting: Setting) {
        self.bitrate.store(setting.bitrate, Ordering::Relaxed);
        self.tier.store(setting.tier, Ordering::Relaxed);
    }

    /// The capture thread found the queue to the engine full and dropped a frame.
    pub fn note_queue_drop(&self) {
        self.queue_drops.fetch_add(1, Ordering::Relaxed);
    }

    /// The frames dropped that way since the last time this was asked.
    pub fn take_queue_drops(&self) -> u32 {
        self.queue_drops.swap(0, Ordering::Relaxed)
    }

    /// The capture thread handed an access unit of `bytes` to the engine.
    pub fn note_sent(&self, bytes: usize) {
        let bytes = u64::try_from(bytes).unwrap_or(u64::MAX);
        self.sent_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    /// The encoded video handed to the engine so far in the call.
    pub fn sent_bytes(&self) -> u64 {
        self.sent_bytes.load(Ordering::Relaxed)
    }

    /// The share of the rate the encoder gets, the rest left to FEC.
    pub fn video_share(&self) -> f64 {
        f64::from(self.video_share.load(Ordering::Relaxed)) / 1e6
    }

    /// The bitrate the encoder runs at for `setting`: its rate less what FEC takes on the wire.
    pub fn encoder_bitrate(&self, setting: Setting) -> u32 {
        scale(setting.bitrate, self.video_share())
    }
}

/// Measures FEC's share of the video on the wire, from the call's running totals.
#[derive(Debug, Default)]
struct FecBudget {
    /// The totals at the last measurement.
    video: u64,
    fec: u64,
}

impl FecBudget {
    /// The encoder's share of the wire since the last measurement, from the encoded video and the
    /// FEC bytes sent so far in the call. `None` until enough video went out to tell, the interval
    /// then running on into the next reading.
    fn observe(&mut self, video_total: u64, fec_total: u64) -> Option<f64> {
        let video = video_total.saturating_sub(self.video);
        let fec = fec_total.saturating_sub(self.fec);
        if video < FEC_MEASURE_BYTES {
            return None;
        }
        self.video = video_total;
        self.fec = fec_total;
        let share = video as f64 / (video as f64 + fec as f64);
        Some(share.clamp(MIN_VIDEO_SHARE, 1.0))
    }
}

/// A rate and the step of picture it is sent at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Setting {
    pub bitrate: u32,
    /// Index into [`TIERS`].
    pub tier: usize,
}

impl Setting {
    fn start() -> Self {
        Self {
            bitrate: START_BITRATE,
            tier: tier_for(START_BITRATE),
        }
    }

    pub fn tier(self) -> Tier {
        TIERS[self.tier]
    }
}

/// The step a rate settles in when nothing else is known: the largest one it reaches.
fn tier_for(bitrate: u32) -> usize {
    STEP_UP.iter().filter(|step| bitrate >= **step).count()
}

/// Why the rate moved, for the log.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reason {
    /// The peer reported this fraction of our video lost.
    Loss(f64),
    /// The peer reported a clean path.
    Clean,
    /// The relay connection's send queue dropped video it could not take.
    SendDrops,
    /// The camera's frames backed up on the way to the engine and some were let go.
    CaptureDrops,
    /// The peer's own estimate of what it can receive.
    Estimate(u32),
    /// The peer receives only this much of our video, in bits per second: a queue on the path.
    Receiving(u32),
    /// The round trip stood this far above the call's shortest: a queue on the path.
    Delay(Duration),
}

/// A move of the rate or the step, from one setting to the next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Change {
    pub from: Setting,
    pub to: Setting,
    pub reason: Reason,
}

/// The adaptation, driven by the peer's RTCP and the send queue.
#[derive(Debug)]
pub struct RateControl {
    setting: Setting,
    /// When the rate first stood high enough for the next step up, while it still does.
    above_since: Option<Instant>,
    /// How long the rate has to stand there: doubled each time a step up meets congestion within
    /// its probation, so a path that cannot carry the larger picture is not offered it every few
    /// seconds, each try costing a keyframe and a burst of loss.
    step_up_hold: Duration,
    /// When the picture last grew, while that step is still on probation.
    stepped_up_at: Option<Instant>,
    congested_at: Option<Instant>,
    raised_at: Option<Instant>,
    /// A little under the rate the peer last reported loss at: clean reports climb quickly up to
    /// it and slowly past it.
    loss_ceiling: Option<u32>,
    /// The peer's latest estimate of what it can receive, and when it said so.
    estimate: Option<(u32, Instant)>,
    /// The shortest round trip of the call, the path with nothing queued, and the latest one.
    round_trip_floor: Option<Duration>,
    round_trip: Option<Duration>,
    delay_cut_at: Option<Instant>,
    /// The video handed to the engine so far, at each of the peer's recent reports of what it
    /// receives: what we actually sent over the last few seconds, which a calm picture keeps well
    /// under the rate.
    sent: std::collections::VecDeque<(Instant, u64)>,
    /// Our own audio SSRCs, learnt from reports that cover audio alone, so that a report covering
    /// both directions of media can be read for its video block alone.
    audio_ssrcs: Vec<u32>,
    fec: FecBudget,
    share: Arc<RateShare>,
}

impl RateControl {
    pub fn new(share: Arc<RateShare>) -> Self {
        let setting = Setting::start();
        share.store(setting);
        Self {
            setting,
            above_since: None,
            step_up_hold: STEP_UP_HOLD,
            stepped_up_at: None,
            congested_at: None,
            raised_at: None,
            loss_ceiling: None,
            estimate: None,
            round_trip_floor: None,
            round_trip: None,
            delay_cut_at: None,
            sent: std::collections::VecDeque::new(),
            audio_ssrcs: Vec::new(),
            fec: FecBudget::default(),
            share,
        }
    }

    pub fn setting(&self) -> Setting {
        self.setting
    }

    /// The FEC bytes the engine has sent so far in the call, read against the video the capture
    /// thread handed it, so the encoder is given the rate less FEC's measured share and the two
    /// together stay at the rate chosen. Returns the encoder's new share once measured.
    pub fn on_fec_sent(&mut self, fec_total: u64) -> Option<f64> {
        let share = self.fec.observe(self.share.sent_bytes(), fec_total)?;
        self.share
            .video_share
            .store(millionths(share), Ordering::Relaxed);
        Some(share)
    }

    /// One RTCP packet from the peer. `blocks` are its report blocks as `(ssrc, fraction lost)`,
    /// `reports_audio` and `reports_video` whether it names our audio and our video stream, and
    /// `estimates` any receiver bandwidth estimates (REMB) it carried.
    pub fn on_rtcp(
        &mut self,
        reports_audio: bool,
        reports_video: bool,
        blocks: &[(u32, u8)],
        estimates: &[u32],
        now: Instant,
    ) -> Option<Change> {
        let mut change = None;
        for &estimate in estimates {
            change = self.on_estimate(estimate, now).or(change);
        }
        if reports_audio && !reports_video {
            for &(ssrc, _) in blocks {
                if !self.audio_ssrcs.contains(&ssrc) {
                    if self.audio_ssrcs.len() == AUDIO_SSRC_MEMORY {
                        self.audio_ssrcs.remove(0);
                    }
                    self.audio_ssrcs.push(ssrc);
                }
            }
            return change;
        }
        if !reports_video || blocks.is_empty() {
            return change;
        }
        // The blocks that can be about our video: all of them, less the ones known to be audio.
        // Read conservatively when that leaves nothing to tell them apart by: the worst of them.
        let video = blocks
            .iter()
            .filter(|(ssrc, _)| !self.audio_ssrcs.contains(ssrc))
            .map(|&(_, lost)| lost)
            .max();
        let lost = video.or_else(|| blocks.iter().map(|&(_, lost)| lost).max())?;
        self.on_loss(lost, now).or(change)
    }

    /// The fraction of our video the peer lost since its last report, in RTCP's 1/256 units.
    pub fn on_loss(&mut self, fraction_lost: u8, now: Instant) -> Option<Change> {
        let loss = f64::from(fraction_lost) / 256.0;
        let before = self.setting;
        let reason = if loss > LOSS_CONGESTED {
            self.congested(now);
            self.loss_ceiling = Some(scale(self.setting.bitrate, LOSS_CEILING).max(MIN_BITRATE));
            self.setting.bitrate = scale(self.setting.bitrate, 1.0 - loss / 2.0);
            Reason::Loss(loss)
        } else if loss < LOSS_CLEAN && self.may_rise(now) {
            self.raised_at = Some(now);
            self.setting.bitrate = self.risen(self.setting.bitrate);
            Reason::Clean
        } else {
            if loss >= LOSS_CLEAN {
                self.congested_at = Some(now);
            }
            return self.restep(now, before, Reason::Loss(loss));
        };
        self.restep(now, before, reason)
    }

    /// The peer asked for a keyframe: a picture it could not rebuild, so the rate holds off rising
    /// for a while, as after congestion. Not cut, and no ceiling remembered: an iPhone asks once
    /// near the start of a clean call too.
    pub fn on_picture_lost(&mut self, now: Instant) {
        self.congested_at = Some(now);
    }

    /// The latest round trip and the call's shortest, for the log.
    pub fn round_trips(&self) -> (Option<Duration>, Option<Duration>) {
        (self.round_trip, self.round_trip_floor)
    }

    /// A round trip measured from the peer's report on one of our sender reports.
    ///
    /// The queue a path builds when it is sent more than it carries shows first as delay, and only
    /// once the queue overflows as loss; a deep buffer may hold seconds before it does. So a round
    /// trip well above the call's shortest cuts the rate like loss would, and one somewhat above it
    /// keeps the rate from rising.
    pub fn on_round_trip(&mut self, round_trip: Duration, now: Instant) -> Option<Change> {
        let floor = self
            .round_trip_floor
            .map_or(round_trip, |floor| floor.min(round_trip));
        self.round_trip_floor = Some(floor);
        self.round_trip = Some(round_trip);
        let queued = round_trip.saturating_sub(floor);
        if queued < RISE_DELAY {
            return None;
        }
        let before = self.setting;
        if queued < QUEUE_DELAY
            || self
                .delay_cut_at
                .is_some_and(|at| now.duration_since(at) < DELAY_CUT_EVERY)
        {
            // Holding is enough here: the rate stays put until the queue drains.
            self.congested_at = Some(now);
            return None;
        }
        self.delay_cut_at = Some(now);
        self.congested(now);
        self.setting.bitrate = scale(self.setting.bitrate, DELAY_CUT);
        self.restep(now, before, Reason::Delay(queued))
    }

    /// The send queue dropped video it could not take: the path is carrying less than is sent.
    pub fn on_send_drops(&mut self, now: Instant) -> Option<Change> {
        self.on_drops(now, Reason::SendDrops)
    }

    /// The camera's frames could not all be handed to the engine, which is behind for the same
    /// reason.
    pub fn on_capture_drops(&mut self, now: Instant) -> Option<Change> {
        self.on_drops(now, Reason::CaptureDrops)
    }

    fn on_drops(&mut self, now: Instant, reason: Reason) -> Option<Change> {
        let before = self.setting;
        self.congested(now);
        self.setting.bitrate = scale(self.setting.bitrate, DROP_CUT);
        self.restep(now, before, reason)
    }

    /// The peer's own estimate of what it can receive caps the rate while it is fresh.
    pub fn on_estimate(&mut self, bitrate: u32, now: Instant) -> Option<Change> {
        let before = self.setting;
        self.estimate = Some((bitrate, now));
        self.restep(now, before, Reason::Estimate(bitrate))
    }

    /// What the peer says it receives of our video caps the rate, like its estimate, once it falls
    /// well short of what we sent: the path is queueing what it cannot carry, and the picture the
    /// peer sees freezes and then rushes to catch up. Ignored until the peer has had half a
    /// megabyte, since its rate starts from nothing when the video does. Set beside what the
    /// encoder actually made, never the rate: a calm picture comes out well under the rate, and
    /// read against the rate each cap made the next one, down to the floor.
    pub fn on_peer_receiving(&mut self, receiving: PeerReceiving, now: Instant) -> Option<Change> {
        let sent = self.sent_rate(now)?;
        if receiving.bytes <= RECEIVING_AFTER_BYTES
            || receiving.bitrate == 0
            || f64::from(receiving.bitrate) >= sent * RECEIVING_SHORT
        {
            return None;
        }
        let before = self.setting;
        self.estimate = Some((scale(receiving.bitrate, RECEIVING_CAP), now));
        self.restep(now, before, Reason::Receiving(receiving.bitrate))
    }

    /// The encoded video's bitrate over the last few seconds, once a second of it is known.
    fn sent_rate(&mut self, now: Instant) -> Option<f64> {
        self.sent.push_back((now, self.share.sent_bytes()));
        while self
            .sent
            .get(1)
            .is_some_and(|&(at, _)| now.duration_since(at) >= SENT_SPAN)
        {
            self.sent.pop_front();
        }
        let &(since, before) = self.sent.front()?;
        let span = now.duration_since(since);
        (span >= SENT_SPAN_MIN)
            .then(|| (self.share.sent_bytes() - before) as f64 * 8.0 / span.as_secs_f64())
    }

    /// A sign of congestion: the rate holds off rising, and a step up still on probation failed.
    fn congested(&mut self, now: Instant) {
        self.congested_at = Some(now);
        if let Some(at) = self.stepped_up_at.take()
            && now.duration_since(at) < STEP_UP_PROBATION
        {
            self.step_up_hold = (self.step_up_hold * 2).min(MAX_STEP_UP_HOLD);
        }
    }

    /// The rate after one clean report: up by [`RISE`] to the loss ceiling, by [`PROBE_RISE`] past
    /// it.
    fn risen(&self, bitrate: u32) -> u32 {
        match self.loss_ceiling {
            Some(ceiling) if bitrate >= ceiling => scale(bitrate, PROBE_RISE),
            Some(ceiling) => scale(bitrate, RISE).min(ceiling.max(scale(bitrate, PROBE_RISE))),
            None => scale(bitrate, RISE),
        }
    }

    fn may_rise(&self, now: Instant) -> bool {
        self.congested_at
            .is_none_or(|at| now.duration_since(at) >= CONGESTION_HOLD)
            && self
                .raised_at
                .is_none_or(|at| now.duration_since(at) >= RISE_EVERY)
    }

    /// Keeps the rate between the floor and what it may be right now: the step's ceiling, and the
    /// peer's estimate while that is fresh.
    fn clamp(&mut self, now: Instant) {
        let mut ceiling = self.setting.tier().max_bitrate;
        if let Some((estimate, at)) = self.estimate
            && now.duration_since(at) < REMB_TTL
        {
            ceiling = ceiling.min(estimate);
        }
        self.setting.bitrate = self
            .setting
            .bitrate
            .clamp(MIN_BITRATE, ceiling.max(MIN_BITRATE));
    }

    /// Clamps the rate, settles the step for it, and publishes what changed.
    ///
    /// Clamped before the step is chosen, so a cap from the peer's estimate moves the picture as
    /// well as the rate; and again after, against the ceiling of the step it settled in.
    fn restep(&mut self, now: Instant, before: Setting, reason: Reason) -> Option<Change> {
        self.clamp(now);
        while self.setting.tier > 0 && self.setting.bitrate < STEP_DOWN[self.setting.tier - 1] {
            self.setting.tier -= 1;
            self.above_since = None;
        }
        // The rate cannot pass the ceiling of its step, which is exactly where the next one starts,
        // so reaching the ceiling is what "high enough" means.
        if self.setting.tier + 1 < TIERS.len() && self.setting.bitrate >= STEP_UP[self.setting.tier]
        {
            let since = *self.above_since.get_or_insert(now);
            if now.duration_since(since) >= self.step_up_hold {
                self.setting.tier += 1;
                self.above_since = None;
                self.stepped_up_at = Some(now);
            }
        } else {
            self.above_since = None;
        }
        self.clamp(now);
        if self.setting == before {
            return None;
        }
        self.share.store(self.setting);
        Some(Change {
            from: before,
            to: self.setting,
            reason,
        })
    }
}

fn scale(bitrate: u32, factor: f64) -> u32 {
    (f64::from(bitrate) * factor)
        .round()
        .clamp(0.0, f64::from(u32::MAX)) as u32
}

/// The middle 32 bits of the NTP timestamp for `now`, the form RTCP's LSR field takes.
pub fn ntp_middle(now: std::time::SystemTime) -> u32 {
    let since = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = since.as_secs().wrapping_add(NTP_UNIX_OFFSET);
    let fraction = (u64::from(since.subsec_nanos()) << 32) / 1_000_000_000;
    (((seconds & 0xffff) << 16) | (fraction >> 16)) as u32
}

/// The round trip a report block measures (RFC 3550, 6.4.1): from when our sender report left,
/// which its LSR names, to `now`, less the time the peer held it, which its DLSR names. All three
/// in NTP's middle 32 bits, units of 1/65536 s. `None` before the peer has had a sender report, or
/// when the clocks say the report came back before it left.
pub fn round_trip(last_sender_report: u32, delay_since: u32, now: u32) -> Option<Duration> {
    if last_sender_report == 0 {
        return None;
    }
    let units = now
        .wrapping_sub(last_sender_report)
        .wrapping_sub(delay_since);
    if units >= 0x8000_0000 {
        return None;
    }
    Some(Duration::from_micros(u64::from(units) * 1_000_000 / 65_536))
}

/// A receiver estimate (RTCP PSFB "REMB", `draft-alvestrand-rmcat-remb`) from one feedback item's
/// FCI, in bits per second.
pub fn remb_bitrate(fci: &[u8]) -> Option<u32> {
    if fci.get(..4)? != b"REMB" {
        return None;
    }
    let exponent = u32::from(*fci.get(5)? >> 2);
    let mantissa =
        (u64::from(fci[5] & 0x03) << 16) | (u64::from(*fci.get(6)?) << 8) | u64::from(*fci.get(7)?);
    Some(u32::try_from(mantissa << exponent.min(40)).unwrap_or(u32::MAX))
}

/// What the peer says it has received of our video, from the 24-byte WhatsApp extension it appends
/// to its report block on our video stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeerReceiving {
    /// All the bytes received since the video started (word 0).
    pub bytes: u32,
    /// The rate it receives at now, in bits per second (word 5).
    pub bitrate: u32,
}

/// Reads [`PeerReceiving`] from a report block's profile extension: big-endian 32-bit words, of
/// which the first and the sixth are understood. `None` for one shorter than 24 bytes.
pub fn peer_receiving(extension: &[u8]) -> Option<PeerReceiving> {
    let word = |at: usize| {
        let bytes = extension.get(at * 4..at * 4 + 4)?;
        Some(u32::from_be_bytes(bytes.try_into().ok()?))
    };
    Some(PeerReceiving {
        bytes: word(0)?,
        bitrate: word(5)?,
    })
}

/// What the peer's feedback has asked of our video over the call, for the log.
///
/// Counted rather than acted on here: the engine answers keyframe requests on its own and the
/// rate control reads the estimates. What these say is whether the peer recovers lost video by
/// asking for packets again (NACK) or for a whole new picture (PLI, FIR), and whether anyone sends
/// an estimate at all, which a freeze or a stuck rate cannot tell apart from the outside.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PeerFeedback {
    /// Packets named in generic NACKs, the bitmask's included.
    pub nacked_packets: u32,
    pub picture_loss: u32,
    pub full_intra: u32,
    pub estimates: u32,
    /// The latest estimate, in bits per second.
    pub last_estimate: Option<u32>,
}

impl PeerFeedback {
    /// One feedback message (RFC 4585): its RTCP packet type, FMT and FCI.
    pub fn note(&mut self, packet_type: u8, fmt: u8, fci: &[u8]) {
        const TRANSPORT_FEEDBACK: u8 = 205;
        const PAYLOAD_FEEDBACK: u8 = 206;
        match (packet_type, fmt) {
            (TRANSPORT_FEEDBACK, 1) => {
                for row in fci.as_chunks::<4>().0 {
                    let following = u16::from_be_bytes([row[2], row[3]]);
                    self.nacked_packets = self
                        .nacked_packets
                        .saturating_add(1 + following.count_ones());
                }
            }
            (PAYLOAD_FEEDBACK, 1) => self.picture_loss = self.picture_loss.saturating_add(1),
            (PAYLOAD_FEEDBACK, 4) => self.full_intra = self.full_intra.saturating_add(1),
            (PAYLOAD_FEEDBACK, 15) => {
                if let Some(bitrate) = remb_bitrate(fci) {
                    self.estimates = self.estimates.saturating_add(1);
                    self.last_estimate = Some(bitrate);
                }
            }
            _ => {}
        }
    }
}

/// How many camera frames to let go for each one sent, so the frames that are sent are evenly
/// spaced: every `n`th frame of a steady camera is a steady stream.
///
/// Thinning by elapsed time instead sends a 50 fps camera at a mix of 40 and 60 ms intervals for a
/// 20 fps target, and that uneven spacing is what judders. The factor that lands nearest the
/// target wins, with ties going to the lower rate, so a 50 fps camera sends 16.7 frames a second
/// and a 60 fps one exactly 20.
pub fn decimation(camera_fps: u32, target_fps: u32) -> u32 {
    let camera = camera_fps.max(1);
    let target = f64::from(target_fps.max(1));
    (1..=camera)
        .min_by(|a, b| {
            let distance = |n: &u32| (f64::from(camera) / f64::from(*n) - target).abs();
            distance(a)
                .partial_cmp(&distance(b))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.cmp(a))
        })
        .unwrap_or(1)
}

/// The camera's real frame rate, measured over windows of thirty frames.
///
/// A driver reports the rate it was set to, and a camera does not always keep to it: many slow
/// down in a dim room, and a capture card runs at whatever its HDMI source does. Thinning by a
/// rate the camera is not keeping sends far fewer frames than intended, so the rate is measured,
/// and a move of more than a sixth from what the thinning was planned for is reported once.
///
/// Only a move two windows in a row agree on. Frames are timed as they reach this thread, and
/// while it is busy (a keyframe to encode, a new encoder to build) the driver's buffers fill and
/// then hand over several frames at once: one window of a 50 fps capture card read 92 fps and
/// another 75, and each reading rebuilt the encoder, which costs the peer a keyframe, only for the
/// next to put it back. A camera that really changed its rate is still at the new one a window
/// later; a backlog is not.
#[derive(Debug)]
pub struct Cadence {
    recent: std::collections::VecDeque<Instant>,
    basis: f64,
    /// The last window's rate, while it stood apart from the basis.
    pending: Option<f64>,
}

/// Frames over which the rate is measured: thirty intervals.
const CADENCE_WINDOW: usize = 31;
/// How far the measured rate may drift from the planned one before the plan changes.
const CADENCE_DRIFT: f64 = 1.0 / 6.0;

impl Cadence {
    pub fn new(planned_fps: f64) -> Self {
        Self {
            recent: std::collections::VecDeque::with_capacity(CADENCE_WINDOW),
            basis: planned_fps.max(1.0),
            pending: None,
        }
    }

    /// One frame from the camera, taken at `at`. Returns the measured rate when it has moved far
    /// enough from the one planned for to plan again, which then becomes the new basis.
    pub fn observe(&mut self, at: Instant) -> Option<f64> {
        self.recent.push_back(at);
        if self.recent.len() < CADENCE_WINDOW {
            return None;
        }
        let span = self
            .recent
            .back()?
            .saturating_duration_since(*self.recent.front()?)
            .as_secs_f64();
        // The window's last frame opens the next one, so no interval goes unmeasured.
        self.recent.drain(..CADENCE_WINDOW - 1);
        if span <= 0.0 {
            return None;
        }
        let measured = (CADENCE_WINDOW - 1) as f64 / span;
        if drifted(measured, self.basis) {
            let confirmed = self
                .pending
                .replace(measured)
                .filter(|previous| !drifted(measured, *previous))?;
            self.pending = None;
            self.basis = (confirmed + measured) / 2.0;
            Some(self.basis)
        } else {
            self.pending = None;
            None
        }
    }
}

/// Whether `rate` is further than [`CADENCE_DRIFT`] from `from`.
fn drifted(rate: f64, from: f64) -> bool {
    (rate - from).abs() / from > CADENCE_DRIFT
}

/// Turns capture instants into evenly spaced RTP timestamps.
///
/// A frame's time comes from when the camera handed it over, not from when its encode finished, so
/// a keyframe that takes longer to encode is not stamped late. It is then snapped to the nominal
/// spacing of the frames being sent: driver jitter of a millisecond or two would otherwise reach
/// the peer as uneven playout. A frame the camera dropped still leaves its gap in the timeline.
#[derive(Debug)]
pub struct FrameClock {
    started: Instant,
    /// Microseconds between two sent frames.
    period_us: f64,
    last: Option<u64>,
}

impl FrameClock {
    pub fn new(started: Instant, sent_fps: f64) -> Self {
        Self {
            started,
            period_us: 1_000_000.0 / sent_fps.max(1.0),
            last: None,
        }
    }

    /// A new spacing from here on, keeping the timeline continuous.
    pub fn respace(&mut self, sent_fps: f64) {
        let last_us = self.last.map(|slot| slot as f64 * self.period_us);
        self.period_us = 1_000_000.0 / sent_fps.max(1.0);
        self.last = last_us.map(|us| (us / self.period_us).floor() as u64);
    }

    /// The 90 kHz timestamp for a frame captured at `captured`. Strictly increasing.
    pub fn stamp(&mut self, captured: Instant) -> u32 {
        let elapsed = captured.saturating_duration_since(self.started).as_micros() as f64;
        let mut slot = (elapsed / self.period_us).round() as u64;
        if let Some(last) = self.last
            && slot <= last
        {
            slot = last + 1;
        }
        self.last = Some(slot);
        let micros = slot as f64 * self.period_us;
        ((micros * 90.0 / 1000.0).round() as u64 & u64::from(u32::MAX)) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn control() -> (RateControl, Arc<RateShare>) {
        let share = Arc::new(RateShare::default());
        (RateControl::new(Arc::clone(&share)), share)
    }

    #[test]
    fn a_call_starts_at_a_540p_picture_well_under_what_720p_needs() {
        let (control, share) = control();
        assert_eq!(control.setting().bitrate, START_BITRATE);
        assert_eq!(control.setting().tier().name, "540p");
        assert_eq!(
            share.load(),
            control.setting(),
            "the capture thread sees the same"
        );
    }

    #[test]
    fn loss_lowers_the_rate_and_a_lot_of_it_drops_the_picture_to_480p() {
        let (mut control, share) = control();
        let now = Instant::now();
        // A quarter of the video lost: the rate comes down by an eighth.
        let change = control.on_loss(64, now).expect("a change");
        assert!(matches!(change.reason, Reason::Loss(_)));
        assert_eq!(change.to.bitrate, 612_500);
        // Far more of it, and the picture steps down at once.
        control.on_loss(200, now + Duration::from_millis(500));
        let setting = control.setting();
        assert!(setting.bitrate < 450_000, "{setting:?}");
        assert_eq!(setting.tier().name, "480p");
        assert_eq!(setting.tier().fps, 25);
        assert_eq!(share.load(), setting);
        // And never below the floor.
        for step in 0..30 {
            control.on_loss(255, now + Duration::from_secs(1 + step));
        }
        assert_eq!(control.setting().bitrate, MIN_BITRATE);
    }

    #[test]
    fn a_few_percent_holds_the_rate_and_a_clean_path_raises_it_slowly() {
        let (mut control, _) = control();
        let now = Instant::now();
        assert_eq!(control.on_loss(12, now), None, "5% is not cut");
        assert_eq!(
            control.on_loss(0, now + Duration::from_secs(1)),
            None,
            "nor followed by a rise for a while"
        );
        let later = now + CONGESTION_HOLD;
        let change = control
            .on_loss(0, later)
            .expect("a clean report raises the rate");
        assert_eq!(change.reason, Reason::Clean);
        assert_eq!(change.to.bitrate, 756_000);
        assert_eq!(
            control.on_loss(0, later + Duration::from_millis(300)),
            None,
            "at most once a second"
        );
        // Under 2% is a clean path, and rises go on.
        assert!(control.on_loss(4, later + Duration::from_secs(1)).is_some());
    }

    /// A keyframe request is a picture the peer could not rebuild: the rate stops climbing for a
    /// while, but is not cut.
    #[test]
    fn a_lost_picture_holds_the_rate_from_rising() {
        let (mut control, _) = control();
        let now = Instant::now();
        control.on_picture_lost(now);
        assert_eq!(control.on_loss(0, now + Duration::from_secs(1)), None);
        assert_eq!(control.setting().bitrate, START_BITRATE);
        assert!(control.on_loss(0, now + CONGESTION_HOLD).is_some());
    }

    #[test]
    fn after_loss_the_rate_climbs_quickly_to_under_where_it_was_lost_and_slowly_past_it() {
        let (mut control, _) = control();
        let mut now = Instant::now();
        for _ in 0..40 {
            now += Duration::from_secs(1);
            control.on_loss(0, now);
        }
        let top = control.setting().bitrate;
        assert_eq!(top, TIERS[2].max_bitrate, "a clean call reaches the top");
        // The peer loses a quarter of it, as a real call did at 1.5 Mbit/s.
        now += Duration::from_secs(1);
        control.on_loss(64, now);
        let ceiling = scale(top, LOSS_CEILING);
        now += CONGESTION_HOLD;
        let mut seconds = 0;
        while control.setting().bitrate < ceiling {
            now += Duration::from_secs(1);
            seconds += 1;
            control.on_loss(0, now);
        }
        assert!(seconds <= 5, "back near the limit in {seconds} s");
        assert_eq!(
            control.setting().bitrate,
            ceiling,
            "and not past it in one step"
        );
        now += Duration::from_secs(1);
        let change = control.on_loss(0, now).expect("still rises");
        assert_eq!(
            change.to.bitrate,
            scale(ceiling, PROBE_RISE),
            "slowly past it"
        );
    }

    #[test]
    fn a_round_trip_is_read_from_lsr_and_dlsr() {
        // RFC 3550's example: LSR 0xb705:2000, DLSR 5.25 s, arriving at 0xb710:8000.
        let round_trip = round_trip(0xb705_2000, 0x0005_4000, 0xb710_8000);
        assert_eq!(round_trip, Some(Duration::from_micros(6_125_000)));
        assert_eq!(super::round_trip(0, 0, 0xb710_8000), None, "no report yet");
        assert_eq!(
            super::round_trip(0xb710_8000, 0x0001_0000, 0xb710_8000),
            None,
            "back before it left"
        );
    }

    #[test]
    fn the_ntp_middle_bits_count_from_1900() {
        assert_eq!(ntp_middle(std::time::UNIX_EPOCH), 0x7e80_0000);
        let half = std::time::UNIX_EPOCH + Duration::from_millis(1_500);
        assert_eq!(ntp_middle(half), 0x7e81_8000);
    }

    #[test]
    fn a_queue_on_the_path_cuts_the_rate_before_any_loss() {
        let (mut control, _) = control();
        let now = Instant::now();
        let floor = Duration::from_millis(80);
        assert_eq!(control.on_round_trip(floor, now), None);
        assert_eq!(
            control.on_round_trip(floor + Duration::from_millis(60), now),
            None,
            "a little delay is the path's own"
        );
        let change = control
            .on_round_trip(floor + Duration::from_millis(400), now)
            .expect("a queue cuts the rate");
        assert_eq!(change.reason, Reason::Delay(Duration::from_millis(400)));
        assert_eq!(change.to.bitrate, 595_000);
        let later = now + Duration::from_millis(500);
        assert_eq!(
            control.on_round_trip(floor + Duration::from_millis(400), later),
            None,
            "at most once a second, while the queue drains"
        );
        assert!(
            control
                .on_round_trip(
                    floor + Duration::from_millis(400),
                    now + Duration::from_secs(1)
                )
                .is_some()
        );
        assert_eq!(control.round_trips().1, Some(floor));
    }

    #[test]
    fn some_queue_delay_keeps_the_rate_from_rising() {
        let (mut control, _) = control();
        let now = Instant::now();
        control.on_round_trip(Duration::from_millis(80), now);
        assert_eq!(control.on_round_trip(Duration::from_millis(230), now), None);
        assert_eq!(control.on_loss(0, now + Duration::from_secs(2)), None);
        assert!(control.on_loss(0, now + Duration::from_secs(3)).is_some());
    }

    #[test]
    fn congestion_holds_off_the_next_rise() {
        let (mut control, _) = control();
        let now = Instant::now();
        control.on_send_drops(now);
        assert_eq!(control.setting().bitrate, 490_000);
        assert_eq!(control.on_loss(0, now + Duration::from_secs(2)), None);
        assert!(control.on_loss(0, now + Duration::from_secs(3)).is_some());
    }

    #[test]
    fn the_picture_grows_only_once_the_rate_has_held() {
        let (mut control, share) = control();
        let start = Instant::now();
        let mut now = start;
        // Clean reports every second climb the rate to the top of 540p...
        while control.setting().bitrate < TIERS[1].max_bitrate {
            now += Duration::from_secs(1);
            control.on_loss(0, now);
        }
        assert_eq!(control.setting().tier().name, "540p", "not yet");
        // ...and only four seconds there later does the picture grow to 720p.
        for _ in 0..4 {
            now += Duration::from_secs(1);
            control.on_loss(0, now);
        }
        assert_eq!(control.setting().tier().name, "720p");
        assert_eq!(share.load().tier, 2);
        // Then the rate keeps climbing to 720p's ceiling and no further.
        for _ in 0..20 {
            now += Duration::from_secs(1);
            control.on_loss(0, now);
        }
        assert_eq!(control.setting().bitrate, TIERS[2].max_bitrate);
    }

    #[test]
    fn the_peers_estimate_caps_the_rate_while_it_is_fresh() {
        let (mut control, _) = control();
        let now = Instant::now();
        let change = control.on_estimate(400_000, now).expect("a cap");
        assert_eq!(change.to.bitrate, 400_000);
        assert_eq!(change.to.tier().name, "480p");
        assert_eq!(
            control.on_loss(0, now + Duration::from_secs(1)),
            None,
            "capped"
        );
        // An estimate that has gone quiet stops holding the rate down.
        assert!(control.on_loss(0, now + Duration::from_secs(11)).is_some());
    }

    /// A WhatsApp report extension with `bytes` in word 0 and `bitrate` in word 5.
    fn extension(bytes: u32, bitrate: u32) -> Vec<u8> {
        let mut extension = vec![0xee; 24];
        extension[..4].copy_from_slice(&bytes.to_be_bytes());
        extension[20..].copy_from_slice(&bitrate.to_be_bytes());
        extension
    }

    #[test]
    fn the_report_extension_says_what_the_peer_has_received() {
        assert_eq!(
            peer_receiving(&extension(1_234_567, 640_000)),
            Some(PeerReceiving {
                bytes: 1_234_567,
                bitrate: 640_000,
            })
        );
        assert_eq!(
            peer_receiving(&extension(1_234_567, 640_000)[..23]),
            None,
            "too short to hold word 5"
        );
        assert_eq!(peer_receiving(&[]), None);
    }

    #[test]
    fn the_peer_receiving_far_less_than_we_send_caps_the_rate() {
        let (mut control, share) = control();
        let start = Instant::now();
        let receiving = |bytes, bitrate| PeerReceiving { bytes, bitrate };
        // Nothing is compared before a second of what we sent is known.
        assert_eq!(
            control.on_peer_receiving(receiving(600_000, 100_000), start),
            None
        );
        // A second at 600 kbit/s.
        share.note_sent(75_000);
        let now = start + Duration::from_secs(1);
        // Before half a megabyte its rate is still climbing from nothing.
        assert_eq!(
            control.on_peer_receiving(receiving(RECEIVING_AFTER_BYTES, 100_000), now),
            None
        );
        // Three quarters of what we sent is a path carrying it.
        assert_eq!(
            control.on_peer_receiving(receiving(600_000, 450_000), now),
            None
        );
        assert_eq!(control.setting().bitrate, START_BITRATE);
        // Less is a queue: send a little under what gets through.
        let change = control
            .on_peer_receiving(receiving(600_000, 400_000), now)
            .expect("a cap");
        assert_eq!(change.reason, Reason::Receiving(400_000));
        assert_eq!(change.to.bitrate, 360_000);
        assert_eq!(change.to.tier().name, "480p");
        assert_eq!(
            control.on_loss(0, now + Duration::from_secs(1)),
            None,
            "capped like an estimate"
        );
    }

    /// A calm picture leaves the encoder well under the rate. The peer receiving all of that is a
    /// path carrying it, not a queue, however far it is under the rate.
    #[test]
    fn a_calm_picture_under_the_rate_is_not_read_as_a_queue() {
        let (mut control, share) = control();
        let start = Instant::now();
        let receiving = |bitrate| PeerReceiving {
            bytes: 2_000_000,
            bitrate,
        };
        assert_eq!(control.on_peer_receiving(receiving(300_000), start), None);
        for second in 1..=10 {
            // 300 kbit/s made at a 700 kbit/s rate, and all of it received.
            share.note_sent(37_500);
            let now = start + Duration::from_secs(second);
            assert_eq!(control.on_peer_receiving(receiving(300_000), now), None);
        }
        assert_eq!(control.setting().bitrate, START_BITRATE);
    }

    #[test]
    fn before_anything_is_measured_the_encoder_leaves_room_for_the_configured_fec() {
        use whatsapp_rust::voip::VideoFecConfig;
        let (control, share) = control();
        // The library's default: one repair packet per eight video packets.
        let expected = scale(START_BITRATE, 8.0 / 9.0);
        assert_eq!(share.encoder_bitrate(control.setting()), expected);
        assert_eq!(configured_video_share(VideoFecConfig::disabled()), 1.0);
    }

    #[test]
    fn the_encoder_gets_the_rate_less_the_fec_measured_on_the_wire() {
        let (mut control, share) = control();
        let setting = control.setting();
        // Too little video yet to tell: the configured share holds, and the interval runs on.
        share.note_sent(20_000);
        assert_eq!(control.on_fec_sent(4_000), None);
        // 100 kB of video and 25 kB of FEC over the two readings: the encoder gets four fifths.
        share.note_sent(80_000);
        assert_eq!(control.on_fec_sent(25_000), Some(0.8));
        assert_eq!(share.encoder_bitrate(setting), scale(setting.bitrate, 0.8));
        // A call that sends no FEC (a group call) gives the encoder the whole rate.
        share.note_sent(100_000);
        assert_eq!(control.on_fec_sent(25_000), Some(1.0));
        assert_eq!(share.encoder_bitrate(setting), setting.bitrate);
    }

    #[test]
    fn fec_never_takes_more_than_a_third_of_the_rate() {
        let (mut control, share) = control();
        share.note_sent(40_000);
        assert_eq!(control.on_fec_sent(80_000), Some(MIN_VIDEO_SHARE));
        let setting = control.setting();
        assert_eq!(
            share.encoder_bitrate(setting),
            scale(setting.bitrate, MIN_VIDEO_SHARE)
        );
    }

    #[test]
    fn a_report_is_read_for_our_video_block() {
        let (mut control, _) = control();
        let now = Instant::now();
        // A report about our audio alone teaches which SSRC is audio.
        assert_eq!(control.on_rtcp(true, false, &[(7, 200)], &[], now), None);
        // A report about both is read for the other block: clean video, lossy audio, so it rises.
        let change = control
            .on_rtcp(true, true, &[(7, 200), (9, 0)], &[], now)
            .expect("the video block is clean");
        assert_eq!(change.reason, Reason::Clean);
        // A report about video alone is read whole.
        let change = control
            .on_rtcp(false, true, &[(9, 128)], &[], now + Duration::from_secs(1))
            .expect("half the video lost");
        assert!(matches!(change.reason, Reason::Loss(_)));
        // One that names neither of our streams is not ours to act on.
        assert_eq!(
            control.on_rtcp(false, false, &[(5, 255)], &[], now + Duration::from_secs(2)),
            None
        );
    }

    #[test]
    fn a_remb_is_read_from_its_mantissa_and_exponent() {
        // 2^4 * 0x30D4 (12500) = 200000 bits a second.
        let fci = [b'R', b'E', b'M', b'B', 1, (4 << 2), 0x30, 0xD4, 0, 0, 0, 9];
        assert_eq!(remb_bitrate(&fci), Some(200_000));
        assert_eq!(remb_bitrate(b"NOPE\x01\x00\x00\x01"), None);
        assert_eq!(remb_bitrate(b"REMB"), None);
    }

    #[test]
    fn the_frames_sent_are_every_nth_the_camera_gives() {
        assert_eq!(decimation(50, 20), 3, "50 fps sends a steady 16.7");
        assert_eq!(decimation(60, 20), 3);
        assert_eq!(decimation(30, 20), 2, "15 is nearer 20 than 30 is");
        assert_eq!(decimation(40, 16), 3, "13.3 is nearer 16 than 20 is");
        assert_eq!(decimation(30, 15), 2);
        assert_eq!(decimation(25, 20), 1);
        assert_eq!(decimation(20, 20), 1);
        assert_eq!(
            decimation(15, 20),
            1,
            "a slow camera is sent at what it gives"
        );
        assert_eq!(decimation(0, 20), 1);
    }

    #[test]
    fn capture_jitter_does_not_reach_the_timestamps() {
        let start = Instant::now();
        // 50 fps camera, every third frame sent: 60 ms apart, captured with a few ms of jitter.
        let mut clock = FrameClock::new(start, 50.0 / 3.0);
        let captured = [0u64, 62, 118, 181, 239];
        let stamps: Vec<u32> = captured
            .iter()
            .map(|ms| clock.stamp(start + Duration::from_millis(*ms)))
            .collect();
        assert_eq!(stamps, [0, 5400, 10800, 16200, 21600]);
        // A frame the camera dropped leaves its gap rather than being closed up.
        assert_eq!(clock.stamp(start + Duration::from_millis(361)), 32_400);
        // Two frames that round to the same slot still move forward.
        let again = clock.stamp(start + Duration::from_millis(362));
        assert!(again > 32_400);
    }

    #[test]
    fn a_camera_keeping_its_rate_is_left_alone_and_one_that_slows_down_is_replanned() {
        let start = Instant::now();
        let mut steady = Cadence::new(50.0);
        for frame in 0..100u64 {
            assert_eq!(
                steady.observe(start + Duration::from_millis(frame * 20)),
                None
            );
        }
        // Said to run at 30, really running at 15: noticed once two windows of frames agree.
        let mut dim = Cadence::new(30.0);
        let noticed = (0..62u64).find_map(|frame| {
            dim.observe(start + Duration::from_micros(frame * 66_667))
                .map(|measured| (frame, measured))
        });
        let (frame, measured) = noticed.expect("the slowdown is noticed");
        assert_eq!(frame, 60, "at the end of the second window");
        assert!((measured - 15.0).abs() < 0.1, "{measured}");
        // And not reported again while it stays there.
        assert!((62..150u64).all(|frame| {
            dim.observe(start + Duration::from_micros(frame * 66_667))
                .is_none()
        }));
    }

    /// A capture card at 50 fps whose frames reach the thread in bunches while it encodes a
    /// keyframe: one window reads far faster than the camera runs, the next is back to normal.
    #[test]
    fn a_backlog_handed_over_at_once_does_not_replan_the_camera() {
        let start = Instant::now();
        let mut cadence = Cadence::new(50.0);
        let mut at = start;
        let mut frame = |gap_ms: u64| {
            at += Duration::from_millis(gap_ms);
            cadence.observe(at)
        };
        // A steady window, then one with fourteen frames delivered back to back (92 fps read).
        for _ in 0..30 {
            assert_eq!(frame(20), None);
        }
        for n in 0..30 {
            assert_eq!(frame(if n < 14 { 1 } else { 20 }), None, "frame {n}");
        }
        // And steady again: nothing to replan, ever.
        for _ in 0..90 {
            assert_eq!(frame(20), None);
        }
    }

    /// The step up that triggered the peer's 75% loss report: a larger picture offered again and
    /// again would cost a keyframe and a burst of loss every few seconds.
    #[test]
    fn a_step_up_that_meets_congestion_waits_longer_before_the_next() {
        let (mut control, _) = control();
        let mut now = Instant::now();
        let climb_to_720p = |control: &mut RateControl, now: &mut Instant| -> Duration {
            let began = *now;
            while control.setting().tier().name != "720p" {
                *now += Duration::from_secs(1);
                control.on_loss(0, *now);
                assert!(*now - began < Duration::from_secs(300), "never stepped up");
            }
            *now - began
        };
        climb_to_720p(&mut control, &mut now);
        // Within its probation the larger picture is lost: down it goes.
        now += Duration::from_secs(3);
        control.on_loss(192, now);
        control.on_loss(192, now + Duration::from_secs(1));
        now += Duration::from_secs(1);
        assert_ne!(control.setting().tier().name, "720p");
        // Climbing back takes the doubled hold at the top of 540p: eight seconds, not four.
        let held_at_top = |control: &mut RateControl, now: &mut Instant| {
            while control.setting().bitrate < TIERS[1].max_bitrate || control.setting().tier != 1 {
                *now += Duration::from_secs(1);
                control.on_loss(0, *now);
            }
            climb_to_720p(control, now)
        };
        let second = held_at_top(&mut control, &mut now);
        assert!(second >= Duration::from_secs(8), "{second:?}");
        // A step up that holds through its probation leaves the hold where it is.
        now += STEP_UP_PROBATION;
        control.on_loss(0, now);
        control.on_loss(192, now + Duration::from_secs(1));
        now += Duration::from_secs(2);
        let third = held_at_top(&mut control, &mut now);
        assert!(third < Duration::from_secs(16), "{third:?}");
    }

    #[test]
    fn the_peers_feedback_is_counted_by_kind() {
        let mut feedback = PeerFeedback::default();
        // Two NACK rows: 1 + 2 packets, and 1 + 16.
        feedback.note(205, 1, &[0, 1, 0, 0b11, 0, 40, 0xff, 0xff]);
        feedback.note(206, 1, &[]);
        feedback.note(206, 4, &[0, 0, 0, 9, 1, 0, 0, 0]);
        feedback.note(206, 15, b"REMB\x01\x10\x30\xd4\x00\x00\x00\x09");
        feedback.note(206, 15, b"NOPE");
        feedback.note(201, 0, &[]);
        assert_eq!(feedback.nacked_packets, 3 + 17);
        assert_eq!(feedback.picture_loss, 1);
        assert_eq!(feedback.full_intra, 1);
        assert_eq!(feedback.estimates, 1);
        assert_eq!(feedback.last_estimate, Some(200_000));
    }

    #[test]
    fn frames_the_capture_thread_dropped_are_handed_over_once() {
        let share = RateShare::default();
        share.note_queue_drop();
        share.note_queue_drop();
        assert_eq!(share.take_queue_drops(), 2);
        assert_eq!(share.take_queue_drops(), 0);
    }

    #[test]
    fn a_new_frame_rate_keeps_the_timeline_continuous() {
        let start = Instant::now();
        let mut clock = FrameClock::new(start, 20.0);
        assert_eq!(clock.stamp(start + Duration::from_millis(1000)), 90_000);
        clock.respace(15.0);
        let next = clock.stamp(start + Duration::from_millis(1066));
        assert_eq!(next, 96_000);
    }
}
