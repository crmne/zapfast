//! Replays a call's audio dump (`ZAPFAST_CALL_AUDIO_DUMP`) through the echo canceller offline,
//! with settings to try, so tuning it does not need a live call.
//!
//! ```sh
//! cargo run --release --example aec_replay -- <folder>/<time>-<stream>-reference.wav [options]
//! ```
//!
//! It reads the dump's reference, microphone and order, feeds the canceller exactly as the call
//! did, and prints, every second, the three signals' levels, how far the output is below the
//! microphone, and what the canceller says of itself. It writes the processed output next to the
//! dump (`…-replay.wav`, or `--out`). Before that it measures the true echo delay by
//! cross-correlating the reference with the microphone, from 200 ms early to 1 s late, over the
//! whole recording and in 20 s windows. It ends with one `summary` line, for comparing settings.
//!
//! Options:
//!
//! - `--engine call|pipeline`: `call` (the default) is the call's own chain
//!   (`zapfast::call_audio::chain`); `pipeline` is sonora's pipeline with AEC3's stock settings,
//!   as the first calls ran it;
//! - `--base call|stock`: the AEC3 settings `--set` starts from (`call` by default);
//! - `--set <key>=<value>`, repeatable: one AEC3 setting, by the names
//!   `chain::apply_setting` takes (`filter.length_blocks=40`, `ep_strength.default_len=0.95`,
//!   `suppressor.scale=0.5`, `ep_strength.bounded_erl=on`, …);
//! - `--hint recorded|off|<ms>`: the delay hint, as the call gave it (the default), none, or fixed;
//! - `--ns on|off`, `--agc on|off`: noise suppression and level control (on by default, as in
//!   calls);
//! - `--capture-delay-ms <ms>`: delays the microphone before the canceller;
//! - `--score <from>-<to>`: the seconds of far-end-only speech to score (all by default): the
//!   summary gives how far the output is below the microphone in each of them where the reference
//!   is above -32 dBFS;
//! - `--near <dBFS>`: mixes a synthetic voice at that level into the scored seconds, two seconds
//!   on and two off, replays twice, and says how much of it came through (the output with it,
//!   less the output without, against the voice itself);
//! - `--live-near <from>-<to>`: seconds where someone really talked at the microphone: the summary
//!   says how the output compares with the microphone there;
//! - `--quiet`: only the summary;
//! - `--out <file.wav>`: where the processed output goes.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use zapfast::call_audio::RATE;
use zapfast::call_audio::chain::{Chain, ChainSettings, apply_setting};
use zapfast::call_audio::dump::{Wav, read_wav, siblings};
use zapfast::call_audio::echo::{CHUNK, stock_pipeline_config};

const SECOND: usize = RATE as usize;

#[derive(Debug, Clone, Copy)]
enum Hint {
    Recorded,
    Off,
    Fixed(i32),
}

#[derive(Debug)]
struct Options {
    reference: PathBuf,
    pipeline: bool,
    stock: bool,
    sets: Vec<(String, String)>,
    hint: Hint,
    ns: bool,
    agc: bool,
    capture_delay_ms: usize,
    score: Option<(usize, usize)>,
    near: Option<f64>,
    live_near: Option<(usize, usize)>,
    quiet: bool,
    out: Option<PathBuf>,
}

fn range(value: &str) -> Result<(usize, usize), String> {
    let (from, to) = value
        .split_once('-')
        .ok_or_else(|| format!("expected <from>-<to>, got {value}"))?;
    let parse = |v: &str| v.parse().map_err(|_| format!("not a number: {v}"));
    Ok((parse(from)?, parse(to)?))
}

fn parse() -> Result<Options, String> {
    let mut args = std::env::args().skip(1);
    let mut options = Options {
        reference: PathBuf::new(),
        pipeline: false,
        stock: false,
        sets: Vec::new(),
        hint: Hint::Recorded,
        ns: true,
        agc: true,
        capture_delay_ms: 0,
        score: None,
        near: None,
        live_near: None,
        quiet: false,
        out: None,
    };
    let on = |value: &str| match value {
        "on" => Ok(true),
        "off" => Ok(false),
        other => Err(format!("expected on or off, got {other}")),
    };
    fn number<T: std::str::FromStr>(value: &str) -> Result<T, String> {
        value.parse().map_err(|_| format!("not a number: {value}"))
    }
    while let Some(arg) = args.next() {
        if !arg.starts_with("--") {
            options.reference = PathBuf::from(arg);
            continue;
        }
        if arg == "--quiet" {
            options.quiet = true;
            continue;
        }
        let value = args.next().ok_or_else(|| format!("{arg} needs a value"))?;
        match arg.as_str() {
            "--engine" => {
                options.pipeline = match value.as_str() {
                    "call" => false,
                    "pipeline" => true,
                    other => return Err(format!("unknown engine {other}")),
                }
            }
            "--base" => {
                options.stock = match value.as_str() {
                    "call" => false,
                    "stock" => true,
                    other => return Err(format!("unknown base {other}")),
                }
            }
            "--set" => {
                let (key, value) = value
                    .split_once('=')
                    .ok_or_else(|| format!("expected <key>=<value>, got {value}"))?;
                options.sets.push((key.to_owned(), value.to_owned()));
            }
            "--hint" => {
                options.hint = match value.as_str() {
                    "recorded" => Hint::Recorded,
                    "off" => Hint::Off,
                    ms => Hint::Fixed(number(ms)?),
                }
            }
            "--ns" => options.ns = on(&value)?,
            "--agc" => options.agc = on(&value)?,
            "--capture-delay-ms" => options.capture_delay_ms = number(&value)?,
            "--score" => options.score = Some(range(&value)?),
            "--near" => options.near = Some(number(&value)?),
            "--live-near" => options.live_near = Some(range(&value)?),
            "--out" => options.out = Some(PathBuf::from(value)),
            other => return Err(format!("unknown option {other}")),
        }
    }
    if options.reference.as_os_str().is_empty() {
        return Err("name the dump's …-reference.wav".to_owned());
    }
    Ok(options)
}

/// One line of the dump's order: reference chunks fed before a microphone chunk, and the hint.
fn read_order(path: &Path) -> Result<Vec<(usize, i32)>, String> {
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    text.lines()
        .skip(1)
        .map(|line| {
            let (fed, hint) = line
                .split_once(',')
                .ok_or_else(|| format!("a bad order line: {line}"))?;
            Ok((
                fed.trim()
                    .parse()
                    .map_err(|_| format!("a bad count: {line}"))?,
                hint.trim()
                    .parse()
                    .map_err(|_| format!("a bad hint: {line}"))?,
            ))
        })
        .collect()
}

fn db(power: f64) -> f64 {
    10.0 * power.max(1e-20).log10()
}

fn mean_square(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    samples
        .iter()
        .map(|s| f64::from(*s) * f64::from(*s))
        .sum::<f64>()
        / samples.len() as f64
}

/// The canceller under test, behind one interface.
trait Canceller {
    fn render(&mut self, chunk: &[f32]);
    fn capture(&mut self, chunk: &mut [f32], hint: Option<i32>);
    /// What the canceller says of itself.
    fn state(&self) -> String;
}

impl Canceller for Chain {
    fn render(&mut self, chunk: &[f32]) {
        Chain::render(self, chunk);
    }

    fn capture(&mut self, chunk: &mut [f32], hint: Option<i32>) {
        Chain::capture(self, chunk, hint);
    }

    fn state(&self) -> String {
        let (erle, erl, delay) = self.metrics();
        format!("{erle:>6.1} dB {erl:>8.1} dB {delay:>4} ms")
    }
}

struct Pipeline {
    processing: sonora::AudioProcessing,
    scratch: Vec<f32>,
}

impl Pipeline {
    fn new(options: &Options) -> Self {
        let mut config = stock_pipeline_config();
        if !options.ns {
            config.noise_suppression = None;
        }
        if !options.agc {
            config.gain_controller2 = None;
        }
        let stream = sonora::StreamConfig::new(RATE, 1);
        Self {
            processing: sonora::AudioProcessing::builder()
                .config(config)
                .capture_config(stream)
                .render_config(stream)
                .build(),
            scratch: vec![0.0; CHUNK],
        }
    }
}

impl Canceller for Pipeline {
    fn render(&mut self, chunk: &[f32]) {
        let _ = self
            .processing
            .process_render_f32(&[chunk], &mut [&mut self.scratch[..]]);
    }

    fn capture(&mut self, chunk: &mut [f32], hint: Option<i32>) {
        if let Some(hint) = hint {
            let _ = self.processing.set_stream_delay_ms(hint);
        }
        if self
            .processing
            .process_capture_f32(&[&chunk[..]], &mut [&mut self.scratch[..]])
            .is_ok()
        {
            chunk.copy_from_slice(&self.scratch);
        }
    }

    fn state(&self) -> String {
        let stats = self.processing.statistics();
        let shown = |v: Option<f64>| v.map_or("-".to_owned(), |v| format!("{v:.1} dB"));
        format!(
            "{:>9} {:>11} {:>7}",
            shown(stats.echo_return_loss_enhancement),
            shown(stats.echo_return_loss),
            stats.delay_ms.map_or("-".to_owned(), |d| format!("{d} ms"))
        )
    }
}

fn canceller(options: &Options) -> Result<Box<dyn Canceller>, String> {
    if options.pipeline {
        return Ok(Box::new(Pipeline::new(options)));
    }
    let mut settings = if options.stock {
        ChainSettings::stock()
    } else {
        ChainSettings::call()
    };
    for (key, value) in &options.sets {
        apply_setting(&mut settings.aec3, key, value)?;
    }
    settings.noise_suppression = options.ns;
    settings.gain_control = options.agc;
    Ok(Box::new(Chain::new(&settings)))
}

/// Feeds the canceller as the call did and returns its output; prints a line a second unless
/// `quiet`.
fn replay(
    options: &Options,
    reference: &[f32],
    microphone: &[f32],
    order: &[(usize, i32)],
    quiet: bool,
) -> Result<Vec<f32>, String> {
    let mut canceller = canceller(options)?;
    let capture_delay = options.capture_delay_ms * SECOND / 1_000;
    let mut delayed: VecDeque<f32> = VecDeque::from(vec![0.0; capture_delay]);
    let chunks = order.len().min(microphone.len() / CHUNK);
    let mut fed = 0_usize;
    let mut output = Vec::with_capacity(chunks * CHUNK);
    let mut second_ref = Vec::new();
    if !quiet {
        println!(
            "  time   reference  microphone      output   reduced | canceller: reduced  return loss  delay"
        );
    }
    for (index, (count, hint)) in order.iter().take(chunks).enumerate() {
        for _ in 0..*count {
            let Some(chunk) = reference.get(fed * CHUNK..(fed + 1) * CHUNK) else {
                break;
            };
            canceller.render(chunk);
            second_ref.extend_from_slice(chunk);
            fed += 1;
        }
        delayed.extend(
            microphone[index * CHUNK..(index + 1) * CHUNK]
                .iter()
                .copied(),
        );
        let mut chunk: Vec<f32> = delayed.drain(..CHUNK).collect();
        let hint = match options.hint {
            Hint::Recorded => Some(*hint),
            Hint::Off => None,
            Hint::Fixed(ms) => Some(ms),
        };
        canceller.capture(&mut chunk, hint);
        output.extend_from_slice(&chunk);
        if (index + 1) % 100 == 0 {
            if !quiet {
                let second = (index + 1) / 100 - 1;
                let span = second * SECOND..(second + 1) * SECOND;
                let (mic, out) = (
                    mean_square(&microphone[span.clone()]),
                    mean_square(&output[span]),
                );
                println!(
                    "{:>5} s {:>7.1} dBFS {:>7.1} dBFS {:>7.1} dBFS {:>6.1} dB | {}",
                    second + 1,
                    db(mean_square(&second_ref)),
                    db(mic),
                    db(out),
                    db(mic) - db(out),
                    canceller.state(),
                );
            }
            second_ref.clear();
        }
    }
    Ok(output)
}

/// Something shaped like speech: noise coloured by a wandering resonance with a pitched buzz under
/// it, in syllables with pauses between words, scaled to `level` dBFS while it sounds.
fn voice(samples: usize, level: f64) -> Vec<f32> {
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    let mut noise = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        ((state >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
    };
    let (mut y1, mut y2, mut phase) = (0.0_f32, 0.0_f32, 0.0_f32);
    let tau = std::f32::consts::TAU;
    let mut out: Vec<f32> = (0..samples)
        .map(|index| {
            let t = index as f32 / RATE as f32;
            let formant = 900.0 + 600.0 * (tau * 0.6 * t).sin();
            let theta = tau * formant / RATE as f32;
            let y = noise() * 0.3 + 2.0 * 0.97 * theta.cos() * y1 - 0.97 * 0.97 * y2;
            y2 = y1;
            y1 = y;
            phase = (phase + 190.0 * (1.0 + 0.1 * (tau * 1.1 * t).sin()) / RATE as f32).fract();
            let buzz: f32 = (1..=14)
                .map(|h| (tau * h as f32 * phase).sin() / h as f32)
                .sum();
            let syllable = (std::f32::consts::PI * 4.0 * t).sin().abs();
            (y * 0.02 + buzz * 0.08) * syllable
        })
        .collect();
    let gain = (10f64.powf(level / 10.0) / mean_square(&out).max(1e-20)).sqrt() as f32;
    for sample in &mut out {
        *sample *= gain;
    }
    out
}

fn aligned_reference(
    reference: &[f32],
    order: &[(usize, i32)],
    length: usize,
) -> (Vec<f32>, String) {
    let mut gaps = Vec::new();
    let mut fed = 0_usize;
    for (index, (count, _)) in order.iter().enumerate() {
        for _ in 0..*count {
            gaps.push(index as i64 - fed as i64);
            fed += 1;
        }
    }
    if gaps.is_empty() {
        return (vec![0.0; length], "no reference was fed".to_owned());
    }
    let mut sorted = gaps.clone();
    sorted.sort_unstable();
    let median = sorted[sorted.len() / 2];
    let summary = format!(
        "the reference was fed {} to {} chunks (10 ms) after its place in its own stream, {} at the median; it drifts {} chunks from the first minute to the last",
        sorted[0],
        sorted[sorted.len() - 1],
        median,
        {
            let minute = 6_000.min(gaps.len());
            let first = gaps[..minute].iter().sum::<i64>() as f64 / minute as f64;
            let last = gaps[gaps.len() - minute..].iter().sum::<i64>() as f64 / minute as f64;
            format!("{:+.1}", last - first)
        }
    );
    let mut aligned = vec![0.0; length];
    for (index, sample) in reference.iter().enumerate() {
        let at = index as i64 + median * CHUNK as i64;
        if let Ok(at) = usize::try_from(at)
            && at < length
        {
            aligned[at] = *sample;
        }
    }
    (aligned, summary)
}

/// The lag, in samples, at which `microphone` best matches `reference`, from `early` samples early
/// to `late` late, with its normalized correlation. Searched at 2 kHz, then refined at 16 kHz.
fn echo_lag(reference: &[f32], microphone: &[f32], early: i64, late: i64) -> Option<(i64, f64)> {
    let decimate = |signal: &[f32]| -> Vec<f32> {
        signal
            .chunks(8)
            .map(|chunk| chunk.iter().sum::<f32>() / chunk.len() as f32)
            .collect()
    };
    let correlate = |r: &[f32], m: &[f32], lag: i64| -> f64 {
        let mut sum = 0.0_f64;
        for (index, sample) in r.iter().enumerate() {
            let at = index as i64 + lag;
            if at >= 0 && (at as usize) < m.len() {
                sum += f64::from(*sample) * f64::from(m[at as usize]);
            }
        }
        sum
    };
    let norm = (mean_square(reference)
        * reference.len() as f64
        * mean_square(microphone)
        * microphone.len() as f64)
        .sqrt();
    if norm <= 0.0 {
        return None;
    }
    let (r, m) = (decimate(reference), decimate(microphone));
    let coarse_norm = (mean_square(&r) * r.len() as f64 * mean_square(&m) * m.len() as f64).sqrt();
    if coarse_norm <= 0.0 {
        return None;
    }
    let (coarse, _) = (-early / 8..=late / 8)
        .map(|lag| (lag, correlate(&r, &m, lag).abs()))
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    (coarse * 8 - 16..=coarse * 8 + 16)
        .map(|lag| (lag, correlate(reference, microphone, lag) / norm))
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
}

fn run() -> Result<(), String> {
    let options = parse()?;
    let [reference_path, microphone_path, _output, order_path] =
        siblings(&options.reference).ok_or_else(|| "name the dump's …-reference.wav".to_owned())?;
    let read = |path: &Path| -> Result<Vec<f32>, String> {
        let (rate, samples) =
            read_wav(path).map_err(|error| format!("{}: {error}", path.display()))?;
        if rate != RATE {
            return Err(format!("{} is at {rate} Hz, not {RATE}", path.display()));
        }
        Ok(samples)
    };
    let reference = read(&reference_path)?;
    let microphone = read(&microphone_path)?;
    let order = read_order(&order_path)?;
    let chunks = order.len().min(microphone.len() / CHUNK);
    let length = chunks * CHUNK;
    let (aligned, drift) = aligned_reference(&reference, &order, length);
    if !options.quiet {
        println!(
            "{} s of microphone, {} s of reference, {chunks} chunks; {options:?}",
            microphone.len() / SECOND,
            reference.len() / SECOND
        );
        println!("{drift}");
        let early = i64::from(RATE) / 5;
        let late = i64::from(RATE);
        match echo_lag(&aligned, &microphone[..length], early, late) {
            Some((lag, correlation)) => println!(
                "true echo delay over the whole recording: {:.1} ms (correlation {correlation:+.3})",
                lag as f64 * 1_000.0 / f64::from(RATE)
            ),
            None => println!("true echo delay: the reference or the microphone is silent"),
        }
        let window = 20 * SECOND;
        for start in (0..length.saturating_sub(window / 2)).step_by(window) {
            let end = (start + window).min(length);
            if let Some((lag, correlation)) =
                echo_lag(&aligned[start..end], &microphone[start..end], early, late)
            {
                println!(
                    "  {:>4} s to {:>4} s: {:.1} ms (correlation {correlation:+.3})",
                    start / SECOND,
                    end / SECOND,
                    lag as f64 * 1_000.0 / f64::from(RATE)
                );
            }
        }
    }

    let output = replay(
        &options,
        &reference,
        &microphone[..length],
        &order,
        options.quiet,
    )?;
    let out_path = options.out.clone().unwrap_or_else(|| {
        let name = reference_path.to_string_lossy();
        PathBuf::from(name.replace("reference.wav", "replay.wav"))
    });
    let mut out =
        Wav::create(&out_path).map_err(|error| format!("{}: {error}", out_path.display()))?;
    out.write(&output);
    out.patch();

    // The score: far-end-only seconds where the peer is heard.
    let seconds = length / SECOND;
    let (from, to) = options.score.unwrap_or((0, seconds));
    let scored: Vec<usize> = (from..to.min(seconds))
        .filter(|second| {
            db(mean_square(
                &aligned[second * SECOND..(second + 1) * SECOND],
            )) > -32.0
        })
        .collect();
    let reductions: Vec<f64> = scored
        .iter()
        .map(|second| {
            let span = second * SECOND..(second + 1) * SECOND;
            db(mean_square(&microphone[span.clone()])) - db(mean_square(&output[span]))
        })
        .collect();
    let mut summary = String::from("summary:");
    if reductions.is_empty() {
        summary.push_str(" no far-end seconds to score;");
    } else {
        let mut sorted = reductions.clone();
        sorted.sort_by(f64::total_cmp);
        let energy = |signal: &[f32]| -> f64 {
            scored
                .iter()
                .map(|second| mean_square(&signal[second * SECOND..(second + 1) * SECOND]))
                .sum()
        };
        summary.push_str(&format!(
            " far-end only ({} s in {from}-{to}): mean {:.1} dB, median {:.1}, worst {:.1}, overall {:.1}, under 25 dB {};",
            scored.len(),
            reductions.iter().sum::<f64>() / reductions.len() as f64,
            sorted[sorted.len() / 2],
            sorted[0],
            db(energy(&microphone)) - db(energy(&output)),
            reductions.iter().filter(|r| **r < 25.0).count(),
        ));
    }
    if let Some(level) = options.near {
        // Two seconds on and two off, over the scored seconds.
        let near_all = voice(length, level);
        let mut near = vec![0.0_f32; length];
        let mut on = Vec::new();
        for second in from..to.min(seconds) {
            if (second - from) % 4 < 2 {
                let span = second * SECOND..(second + 1) * SECOND;
                near[span.clone()].copy_from_slice(&near_all[span]);
                on.push(second);
            }
        }
        let mixed: Vec<f32> = microphone[..length]
            .iter()
            .zip(&near)
            .map(|(m, n)| m + n)
            .collect();
        let with = replay(&options, &reference, &mixed, &order, true)?;
        let passed: Vec<f32> = with.iter().zip(&output).map(|(w, o)| w - o).collect();
        let span_energy = |signal: &[f32]| -> f64 {
            on.iter()
                .map(|second| mean_square(&signal[second * SECOND..(second + 1) * SECOND]))
                .sum()
        };
        let during_echo: Vec<usize> = on
            .iter()
            .copied()
            .filter(|second| scored.contains(second))
            .collect();
        let echo_energy = |signal: &[f32]| -> f64 {
            during_echo
                .iter()
                .map(|second| mean_square(&signal[second * SECOND..(second + 1) * SECOND]))
                .sum()
        };
        summary.push_str(&format!(
            " a voice at {level:.0} dBFS mixed in comes out at {:+.1} dB ({:+.1} dB while the peer talks);",
            db(span_energy(&passed)) - db(span_energy(&near)),
            db(echo_energy(&passed)) - db(echo_energy(&near)),
        ));
    }
    if let Some((from, to)) = options.live_near {
        let span = from * SECOND..to.min(seconds) * SECOND;
        summary.push_str(&format!(
            " live near end {from}-{to} s: output {:+.1} dB against the microphone;",
            db(mean_square(&output[span.clone()])) - db(mean_square(&microphone[span]))
        ));
    }
    println!("{summary} output in {}", out_path.display());
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("aec_replay: {error}");
            ExitCode::FAILURE
        }
    }
}
