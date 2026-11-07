//! Pitch-preserving speed-up for voice playback.
//!
//! Playing samples faster by feeding them to the sink at a higher rate
//! sharpens the voice, which the phone client does not do: it compresses the
//! time and keeps the speaker's pitch. `speed_up` does that compression once,
//! offline, with waveform-similarity overlap-add (WSOLA): the clip is cut
//! into overlapping frames, the frames most similar to the natural
//! continuation of the output are picked, and they are overlap-added back at
//! a smaller hop. The sink then plays the result at normal rate. A video's
//! sound is too long to hold that way: [`Stream`] does the same as it plays.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};

/// Analysis frame, about 43 ms of speech at 48 kHz.
const FRAME: usize = 2048;
/// Synthesis hop; consecutive frames overlap three quarters.
const HOP: usize = FRAME / 4;
/// How far the similarity search may stray from the nominal read position.
const SEARCH: usize = 448;
/// The search runs on this decimation of the signal; 8 keeps the shape of
/// speech while comparing far fewer samples.
const STEP: usize = 8;

/// Compresses mono samples by `factor` (above 1) without changing the pitch.
///
/// The result's length is the input's length divided by `factor`, rounded.
pub fn speed_up(samples: &[f32], factor: f32) -> Vec<f32> {
    speed_up_unless(samples, factor, &AtomicBool::new(false)).expect("never cancelled")
}

/// Like `speed_up`, but gives up with `None` as soon as `cancelled` is set,
/// so a compression nobody will play stops scanning a long clip.
pub fn speed_up_unless(samples: &[f32], factor: f32, cancelled: &AtomicBool) -> Option<Vec<f32>> {
    // Only finite speed-ups compress; anything else plays as recorded.
    if !(factor.is_finite() && factor > 1.0) {
        return Some(samples.to_vec());
    }
    let target = ((samples.len() as f64) / f64::from(factor)).round() as usize;
    if samples.len() < FRAME * 2 {
        // Too short to overlap-add: drop samples, pitch and all. Clips this
        // short are clicks and beeps, not speech.
        let last = samples.len().saturating_sub(1);
        return Some(
            (0..target)
                .map(|i| samples[(((i as f64) * f64::from(factor)) as usize).min(last)])
                .collect(),
        );
    }

    // Averaged decimation carries the waveform shape the comparison needs.
    let dec: Vec<f32> = (0..samples.len() / STEP)
        .map(|i| {
            let start = i * STEP;
            samples[start..start + STEP].iter().sum::<f32>() / STEP as f32
        })
        .collect();
    let window: Vec<f32> = (0..FRAME)
        .map(|i| {
            (std::f32::consts::PI * i as f32 / FRAME as f32)
                .sin()
                .powi(2)
        })
        .collect();
    let analysis_hop = ((HOP as f64) * f64::from(factor)).round() as usize;

    let mut out = vec![0.0f32; target + FRAME];
    let mut weight = vec![0.0f32; target + FRAME];
    let mut nominal = 0usize;
    let mut written = 0usize;
    let mut previous: Option<usize> = None;
    while written + FRAME <= out.len() {
        if cancelled.load(Ordering::Relaxed) {
            return None;
        }
        let frame_end = (nominal + FRAME + SEARCH).min(samples.len());
        let last_start = samples.len() - FRAME;
        let read = if frame_end < samples.len() {
            nominal
        } else {
            // Near the end there is nothing left to search through; read the
            // last full frame so the tail of the clip is heard.
            last_start
        };
        let chosen = previous
            .filter(|_| read + FRAME + SEARCH <= samples.len())
            .map(|prev| {
                let shift = best_alignment(&dec, read, prev);
                (read as isize + shift).clamp(0, last_start as isize) as usize
            })
            .unwrap_or(read.min(last_start));
        for i in 0..FRAME {
            out[written + i] += samples[chosen + i] * window[i];
            weight[written + i] += window[i];
        }
        previous = Some(chosen);
        written += HOP;
        if read == last_start {
            break;
        }
        nominal += analysis_hop;
    }
    for i in 0..out.len() {
        out[i] /= weight[i].max(1e-3);
    }
    out.truncate(target);
    Some(out)
}

/// Returns the offset within `±SEARCH` of `nominal` whose decimated frame
/// best matches the natural continuation of the frame chosen at `previous`:
/// the input `HOP` samples after it.
fn best_alignment(dec: &[f32], nominal: usize, previous: usize) -> isize {
    let width = FRAME / STEP;
    let template = previous / STEP + HOP / STEP;
    let centre = nominal / STEP;
    let reach = SEARCH / STEP;
    if template + width > dec.len() || centre < reach || centre + reach + width > dec.len() {
        return 0;
    }
    let mut best = 0isize;
    let mut best_score = f32::NEG_INFINITY;
    for shift in -(reach as isize)..=(reach as isize) {
        let at = (centre as isize + shift) as usize;
        let mut dot = 0.0;
        let mut left = 0.0;
        let mut right = 0.0;
        for i in 0..width {
            let a = dec[at + i];
            let b = dec[template + i];
            dot += a * b;
            left += a * a;
            right += b * b;
        }
        let score = dot / (left * right).sqrt().max(1e-9);
        if score > best_score {
            best_score = score;
            best = shift * STEP as isize;
        }
    }
    best
}

/// Compresses interleaved sound by `factor` as it is read, for playback
/// that cannot wait for, or hold, the whole track: a video's sound.
///
/// The same overlap-add as [`speed_up`], with the similarity search run on
/// the average of the channels and the chosen frame copied from every
/// channel, so stereo stays stereo and its channels stay aligned. Only the
/// frames the next search can reach are held, a few thousand per channel.
/// Output frame `n` comes from about input frame `n * factor`, so the
/// played time times `factor` is the position in the source; past the end
/// the source reads as silence, and the output stops at its length divided
/// by `factor`, rounded.
pub struct Stream<I> {
    input: I,
    channels: usize,
    analysis_hop: usize,
    window: Vec<f32>,
    /// Input frames, interleaved, the first being frame `offset`.
    held: VecDeque<f32>,
    offset: usize,
    /// Frames read from `input`, and whether it has run out.
    read: usize,
    ended: bool,
    /// The channel average decimated by `STEP`, the first being block
    /// `dec_offset`, and the sum of the block being read.
    dec: VecDeque<f32>,
    dec_offset: usize,
    block: f32,
    /// Where the next frame would be read without a search, and where the
    /// last one was.
    nominal: usize,
    previous: Option<usize>,
    /// Overlap-add of the frames placed so far, interleaved, and the
    /// window weight under each frame.
    sum: VecDeque<f32>,
    weight: VecDeque<f32>,
    /// Finished output, interleaved.
    out: VecDeque<f32>,
    emitted: usize,
    factor: f64,
}

impl<I: Iterator<Item = f32>> Stream<I> {
    /// Reads interleaved samples of `channels` channels from `input`;
    /// `factor` is clamped to at least 1.
    pub fn new(input: I, channels: usize, factor: f32) -> Self {
        let factor = if factor.is_finite() {
            f64::from(factor.max(1.0))
        } else {
            1.0
        };
        let channels = channels.max(1);
        Self {
            input,
            channels,
            analysis_hop: ((HOP as f64) * factor).round() as usize,
            window: (0..FRAME)
                .map(|i| {
                    (std::f32::consts::PI * i as f32 / FRAME as f32)
                        .sin()
                        .powi(2)
                })
                .collect(),
            held: VecDeque::new(),
            offset: 0,
            read: 0,
            ended: false,
            dec: VecDeque::new(),
            dec_offset: 0,
            block: 0.0,
            nominal: 0,
            previous: None,
            sum: std::iter::repeat_n(0.0, FRAME * channels).collect(),
            weight: std::iter::repeat_n(0.0, FRAME).collect(),
            out: VecDeque::new(),
            emitted: 0,
            factor,
        }
    }

    /// Input frames held at the moment, for tests of the memory bound.
    #[cfg(test)]
    fn held_frames(&self) -> usize {
        self.held.len() / self.channels
    }

    /// Reads input until frame `until` (exclusive) is held or it runs out.
    fn fill(&mut self, until: usize) {
        let mut frame = Vec::with_capacity(self.channels);
        while !self.ended && self.read < until {
            frame.clear();
            frame.extend(self.input.by_ref().take(self.channels));
            if frame.len() < self.channels {
                self.ended = true;
                break;
            }
            self.held.extend(&frame);
            self.block += frame.iter().sum::<f32>() / self.channels as f32;
            self.read += 1;
            if self.read.is_multiple_of(STEP) {
                self.dec.push_back(self.block / STEP as f32);
                self.block = 0.0;
            }
        }
    }

    fn sample(&self, frame: usize, channel: usize) -> f32 {
        frame
            .checked_sub(self.offset)
            .and_then(|at| self.held.get(at * self.channels + channel))
            .copied()
            .unwrap_or(0.0)
    }

    /// The offset within `±SEARCH` of the nominal position whose decimated
    /// frame best matches the natural continuation of the previous frame,
    /// as [`best_alignment`] finds it in a whole clip.
    fn alignment(&self, previous: usize) -> isize {
        let width = FRAME / STEP;
        let reach = SEARCH / STEP;
        let template = previous / STEP + HOP / STEP;
        let centre = self.nominal / STEP;
        let available = self.dec_offset + self.dec.len();
        if centre < reach
            || template < self.dec_offset
            || centre - reach < self.dec_offset
            || template + width > available
            || centre + reach + width > available
        {
            return 0;
        }
        let dec = |block: usize| self.dec[block - self.dec_offset];
        let mut best = 0isize;
        let mut best_score = f32::NEG_INFINITY;
        for shift in -(reach as isize)..=(reach as isize) {
            let at = (centre as isize + shift) as usize;
            let (mut dot, mut left, mut right) = (0.0, 0.0, 0.0);
            for i in 0..width {
                let a = dec(at + i);
                let b = dec(template + i);
                dot += a * b;
                left += a * a;
                right += b * b;
            }
            let score = dot / (left * right).sqrt().max(1e-9);
            if score > best_score {
                best_score = score;
                best = shift * STEP as isize;
            }
        }
        best
    }

    /// The number of output frames there will be, once the input is over.
    fn target(&self) -> Option<usize> {
        self.ended
            .then(|| ((self.read as f64) / self.factor).round() as usize)
    }

    /// Places one more frame and finishes `HOP` frames of output.
    fn step(&mut self) {
        self.fill(self.nominal + FRAME + SEARCH);
        let chosen = self.previous.map_or(self.nominal, |previous| {
            (self.nominal as isize + self.alignment(previous)).max(0) as usize
        });
        for i in 0..FRAME {
            let gain = self.window[i];
            for channel in 0..self.channels {
                let sample = self.sample(chosen + i, channel);
                self.sum[i * self.channels + channel] += sample * gain;
            }
            self.weight[i] += gain;
        }
        for _ in 0..HOP {
            let weight = self.weight.pop_front().unwrap_or(0.0).max(1e-3);
            for _ in 0..self.channels {
                let sample = self.sum.pop_front().unwrap_or(0.0);
                self.out.push_back(sample / weight);
                self.sum.push_back(0.0);
            }
            self.weight.push_back(0.0);
        }
        self.emitted += HOP;
        self.previous = Some(chosen);
        self.nominal += self.analysis_hop;
        // Nothing before the next search's reach, or the continuation of
        // the frame just placed, is read again.
        let keep = (self.nominal.saturating_sub(SEARCH)).min(chosen + HOP) / STEP * STEP;
        if keep > self.offset {
            let drop = (keep - self.offset).min(self.held.len() / self.channels);
            self.held.drain(..drop * self.channels);
            self.offset += drop;
        }
        let keep_blocks = keep / STEP;
        if keep_blocks > self.dec_offset {
            let drop = (keep_blocks - self.dec_offset).min(self.dec.len());
            self.dec.drain(..drop);
            self.dec_offset += drop;
        }
        if let Some(target) = self.target()
            && self.emitted > target
        {
            let extra = (self.emitted - target).min(self.out.len() / self.channels);
            self.out.truncate(self.out.len() - extra * self.channels);
            self.emitted -= extra;
        }
    }
}

impl<I: Iterator<Item = f32>> Iterator for Stream<I> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        loop {
            if let Some(sample) = self.out.pop_front() {
                return Some(sample);
            }
            if self.target().is_some_and(|target| self.emitted >= target) {
                return None;
            }
            self.step();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    fn sine(hz: f32, seconds: f32) -> Vec<f32> {
        (0..(RATE * seconds) as usize)
            .map(|i| (i as f32 * hz * std::f32::consts::TAU / RATE).sin() * 0.8)
            .collect()
    }

    /// Positive-going zero crossings per second, ignoring the crossfaded ends.
    fn frequency(samples: &[f32]) -> f32 {
        let skip = 4_000;
        let body = &samples[skip..samples.len() - skip];
        let crossings = body
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        crossings as f32 / (body.len() as f32 / RATE)
    }

    #[test]
    fn the_length_shrinks_by_the_factor() {
        for factor in [1.5, 2.0] {
            let samples = sine(440.0, 1.0);
            let faster = speed_up(&samples, factor);
            let expected = (samples.len() as f32 / factor).round() as usize;
            assert_eq!(faster.len(), expected, "at {factor}x");
        }
        // Lengths that divide unevenly still land on the rounded target.
        let samples = sine(440.0, 123_457.0 / RATE);
        let faster = speed_up(&samples, 2.0);
        assert_eq!(faster.len(), 61_729);
    }

    #[test]
    fn the_pitch_survives_double_speed() {
        let samples = sine(440.0, 1.0);
        let faster = speed_up(&samples, 2.0);
        let heard = frequency(&faster);
        assert!(
            (425.0..=455.0).contains(&heard),
            "heard {heard} Hz instead of about 440"
        );
    }

    #[test]
    fn the_pitch_survives_one_and_a_half_speed() {
        let samples = sine(440.0, 1.0);
        let faster = speed_up(&samples, 1.5);
        let heard = frequency(&faster);
        assert!(
            (425.0..=455.0).contains(&heard),
            "heard {heard} Hz instead of about 440"
        );
    }

    #[test]
    fn the_loudness_survives() {
        let samples = sine(440.0, 1.0);
        let faster = speed_up(&samples, 2.0);
        let body = &faster[4_000..faster.len() - 4_000];
        let loudest = body.iter().fold(0.0f32, |peak, &s| peak.max(s.abs()));
        assert!(
            (0.6..=1.0).contains(&loudest),
            "loudest sample {loudest} of a 0.8 tone"
        );
    }

    #[test]
    fn silence_stays_silent() {
        let faster = speed_up(&vec![0.0; RATE as usize], 2.0);
        assert_eq!(faster.len(), RATE as usize / 2);
        assert!(faster.iter().all(|&s| s.abs() < 1e-6));
    }

    #[test]
    fn short_clips_fall_back_to_dropping_samples() {
        let samples: Vec<f32> = (0..1_000).map(|i| i as f32).collect();
        let faster = speed_up(&samples, 2.0);
        assert_eq!(faster.len(), 500);
        assert_eq!(faster[10], 20.0);
        assert_eq!(faster[499], 998.0);
    }

    #[test]
    fn a_cancelled_compression_gives_up() {
        let samples = sine(440.0, 1.0);
        assert!(speed_up_unless(&samples, 2.0, &AtomicBool::new(true)).is_none());
    }

    #[test]
    fn unusable_factors_return_the_same_samples() {
        let samples = sine(440.0, 0.1);
        for factor in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -2.0] {
            assert_eq!(speed_up(&samples, factor), samples, "at {factor}");
        }
    }

    #[test]
    fn unit_speed_returns_the_same_samples() {
        let samples = sine(440.0, 0.1);
        assert_eq!(speed_up(&samples, 1.0), samples);
    }

    /// Interleaves a left and a right channel.
    fn stereo(left: &[f32], right: &[f32]) -> Vec<f32> {
        left.iter().zip(right).flat_map(|(&l, &r)| [l, r]).collect()
    }

    fn channel(interleaved: &[f32], which: usize) -> Vec<f32> {
        interleaved.iter().skip(which).step_by(2).copied().collect()
    }

    #[test]
    fn a_stream_shrinks_by_the_factor_and_keeps_both_channels() {
        for factor in [1.5, 2.0] {
            let (left, right) = (sine(440.0, 2.0), sine(660.0, 2.0));
            let faster: Vec<f32> =
                Stream::new(stereo(&left, &right).into_iter(), 2, factor).collect();
            let expected = (left.len() as f32 / factor).round() as usize;
            assert_eq!(faster.len(), expected * 2, "at {factor}x");
            // Each side keeps its own pitch: the channels were not mixed.
            let heard = (
                frequency(&channel(&faster, 0)),
                frequency(&channel(&faster, 1)),
            );
            assert!((425.0..=455.0).contains(&heard.0), "left {heard:?}");
            assert!((640.0..=680.0).contains(&heard.1), "right {heard:?}");
        }
    }

    #[test]
    fn a_silent_channel_stays_silent() {
        let tone = sine(440.0, 1.0);
        let quiet = vec![0.0; tone.len()];
        let faster: Vec<f32> = Stream::new(stereo(&quiet, &tone).into_iter(), 2, 2.0).collect();
        assert!(channel(&faster, 0).iter().all(|&s| s.abs() < 1e-6));
        let loudest = channel(&faster, 1)[4_000..20_000]
            .iter()
            .fold(0.0f32, |peak, &s| peak.max(s.abs()));
        assert!((0.6..=1.0).contains(&loudest), "loudest {loudest}");
    }

    /// The sound that plays at output time t is the source's sound at t
    /// times the factor, so the picture, whose clock runs at the factor,
    /// stays with it: a syllable-long tone every half second starts every
    /// quarter second at 2x, within a couple of hundredths of a second.
    #[test]
    fn a_stream_keeps_events_where_the_clock_puts_them() {
        for factor in [1.5f32, 2.0] {
            let tone = sine(440.0, 0.1);
            let mut beeps = vec![0.0f32; (RATE * 6.0) as usize];
            let every = (RATE / 2.0) as usize;
            for start in (every..beeps.len() - every).step_by(every) {
                beeps[start..start + tone.len()].copy_from_slice(&tone);
            }
            let faster: Vec<f32> =
                Stream::new(stereo(&beeps, &beeps).into_iter(), 2, factor).collect();
            let left = channel(&faster, 0);
            for start in (every..beeps.len() - every).step_by(every) {
                let expected = (start as f32 / factor) as usize;
                let found = (expected.saturating_sub(4_800)..expected + 4_800)
                    .find(|&i| left[i].abs() > 0.4)
                    .unwrap_or_else(|| panic!("no beep near {expected} at {factor}x"));
                let off = (found as f32 - expected as f32).abs() / RATE;
                assert!(off < 0.025, "beep {off} s off at {factor}x");
            }
        }
    }

    #[test]
    fn a_stream_holds_only_a_few_frames() {
        let samples = sine(440.0, 10.0);
        let mut stream = Stream::new(stereo(&samples, &samples).into_iter(), 2, 2.0);
        let mut most = 0;
        let mut played = 0;
        while stream.next().is_some() {
            played += 1;
            most = most.max(stream.held_frames());
        }
        assert_eq!(played, samples.len());
        assert!(most < FRAME * 3, "held {most} frames of {}", samples.len());
    }

    #[test]
    fn a_short_stream_still_ends_at_its_length() {
        let samples: Vec<f32> = (0..1_001).map(|i| (i as f32 / 1_000.0).sin()).collect();
        assert_eq!(Stream::new(samples.into_iter(), 1, 2.0).count(), 501);
        assert_eq!(Stream::new(std::iter::empty(), 2, 2.0).count(), 0);
    }
}
