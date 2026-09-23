//! Pitch-preserving speed-up for voice playback.
//!
//! Playing samples faster by feeding them to the sink at a higher rate
//! sharpens the voice, which the phone client does not do: it compresses the
//! time and keeps the speaker's pitch. `speed_up` does that compression once,
//! offline, with waveform-similarity overlap-add (WSOLA): the clip is cut
//! into overlapping frames, the frames most similar to the natural
//! continuation of the output are picked, and they are overlap-added back at
//! a smaller hop. The sink then plays the result at normal rate.

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
}
