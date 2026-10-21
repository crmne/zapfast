//! The call's processing chain, in 10 ms chunks at 16 kHz: a high-pass filter, WebRTC's echo
//! canceller (AEC3) driven directly with a configuration tuned for speakers in a room, then
//! noise suppression and level control.
//!
//! sonora's own pipeline runs AEC3 with its stock configuration and takes no other. That one
//! was made for laptops and phones: a short, mostly direct echo path. Desk speakers in an
//! untreated room give a mostly reverberant echo, which the stock configuration left about 11 dB
//! below the microphone (see `examples/aec_replay.rs` and [`ChainSettings::call`]), so the chain
//! drives AEC3's block processor itself, framing exactly as sonora's wrapper does, and hands the
//! result to sonora's pipeline for the noise suppressor and AGC2 only.

use sonora::config::{AdaptiveDigital, GainController2, NoiseSuppression};
use sonora::{AudioProcessing, Config, StreamConfig};
use sonora_aec3::block::Block;
use sonora_aec3::block_framer::BlockFramer;
use sonora_aec3::block_processor::BlockProcessor;
use sonora_aec3::config::{EchoCanceller3Config, TransparentModeType};
use sonora_aec3::frame_blocker::FrameBlocker;

use super::RATE;

/// One chunk: 10 ms at [`RATE`].
const CHUNK: usize = RATE as usize / 100;
/// AEC3 works on 80-sample halves of a chunk, and in the 16-bit range.
const HALF: usize = CHUNK / 2;
const SCALE: f32 = 32_768.0;
/// The largest gain the level control may add: 20 dB, the ten times `voice::normalize` caps a
/// voice message's at, so a quiet microphone is raised as far as a recorded one and no further.
const MAX_GAIN_DB: f32 = 20.0;

/// What the chain runs.
#[derive(Clone, Debug)]
pub struct ChainSettings {
    pub aec3: EchoCanceller3Config,
    pub noise_suppression: bool,
    pub gain_control: bool,
}

impl ChainSettings {
    /// What a call runs: AEC3 tuned for speakers in a room (see `tuned` for what and why, with
    /// the measurements), noise suppression and level control.
    pub fn call() -> Self {
        Self {
            aec3: tuned(),
            noise_suppression: true,
            gain_control: true,
        }
    }

    /// AEC3's stock configuration, as sonora's pipeline runs it.
    pub fn stock() -> Self {
        Self {
            aec3: EchoCanceller3Config::default(),
            noise_suppression: true,
            gain_control: true,
        }
    }
}

/// The call's AEC3 configuration: the stock one with a longer main filter and a reverberation
/// model that starts long and then follows the room.
///
/// Through desk speakers in an untreated room most of the echo is the room's tail rather than
/// the speaker's direct sound: on the recording these settings were measured on, the reference
/// and the microphone were barely related over 16 ms (coherence near 0) and only well related
/// over a second (0.9 below 600 Hz), so a linear filter of any length could take out at most
/// about 9 dB, and the rest is the suppressor's. It estimates the residual echo with a
/// reverberation model that decays by `default_len` every 4 ms block: the stock 0.83 assumes the
/// tail is gone within some 50 ms. A negative value starts the model at its magnitude, 0.985
/// (a tail of about a second, as this room's), and lets AEC3 estimate the room's own decay from
/// its filter, so a room with a short tail is not suppressed as if it had a long one; a fixed
/// 0.985 took a voice talking over the peer in a synthetic reverberant room down 43 dB. The
/// estimate needs a filter that reaches into the tail: the main (refined) filter is 60 blocks,
/// 240 ms, against the stock 13; the coarse one stays short so it still converges quickly.
///
/// Measured with `examples/aec_replay.rs --agc off --near -45` (how far the output is below the
/// microphone in far-end-only seconds; how much of a voice mixed into the microphone comes out,
/// and while the peer talks):
///
/// | Room | Stock | These |
/// |---|---|---|
/// | Recorded desk speakers, 0 to 25 s | 11.2 dB mean, 10.6 median; voice -1.3, -0.8 | 26.2 mean, 28.7 median; voice -2.6, -0.7 |
/// | Synthetic, short echo | 37.6, voice -1.5, -1.7 | 37.6, voice -1.7, -2.0 |
/// | Synthetic, 0.6 s reverberation | 30.4, voice -2.4, -5.1 | 35.4, voice -1.4, -2.0 |
///
/// The recording's two worst seconds are its first two, while the canceller first converges.
/// Longer filters, more delay filters, a scaled suppressor, a larger default echo path gain,
/// bounded ERL, the HMM transparent mode and clock-drift handling each moved the recording's
/// average by under 4 dB without the reverberation model, and none helped with it.
fn tuned() -> EchoCanceller3Config {
    let mut config = EchoCanceller3Config::default();
    config.ep_strength.default_len = -0.985;
    config.filter.refined.length_blocks = 60;
    config
}

/// Sets one AEC3 setting by name, for trying settings offline (`examples/aec_replay.rs`).
pub fn apply_setting(
    config: &mut EchoCanceller3Config,
    key: &str,
    value: &str,
) -> Result<(), String> {
    fn number<T: std::str::FromStr>(value: &str) -> Result<T, String> {
        value.parse().map_err(|_| format!("not a number: {value}"))
    }
    fn flag(value: &str) -> Result<bool, String> {
        match value {
            "on" | "true" | "1" => Ok(true),
            "off" | "false" | "0" => Ok(false),
            other => Err(format!("expected on or off, got {other}")),
        }
    }
    let suppressor = &mut config.suppressor;
    match key {
        "filter.length_blocks" => {
            let blocks = number(value)?;
            config.filter.refined.length_blocks = blocks;
            config.filter.refined_initial.length_blocks = blocks;
            config.filter.coarse.length_blocks = blocks;
            config.filter.coarse_initial.length_blocks = blocks;
        }
        "filter.refined.length_blocks" => config.filter.refined.length_blocks = number(value)?,
        "filter.coarse.length_blocks" => config.filter.coarse.length_blocks = number(value)?,
        "filter.conservative_initial_phase" => {
            config.filter.conservative_initial_phase = flag(value)?;
        }
        "filter.initial_state_seconds" => config.filter.initial_state_seconds = number(value)?,
        "filter.high_pass_filter_echo_reference" => {
            config.filter.high_pass_filter_echo_reference = flag(value)?;
        }
        "delay.num_filters" => config.delay.num_filters = number(value)?,
        "delay.down_sampling_factor" => config.delay.down_sampling_factor = number(value)?,
        "delay.delay_headroom_samples" => config.delay.delay_headroom_samples = number(value)?,
        "delay.fixed_capture_delay_samples" => {
            config.delay.fixed_capture_delay_samples = number(value)?;
        }
        "erle.min" => config.erle.min = number(value)?,
        "erle.max_l" => config.erle.max_l = number(value)?,
        "erle.max_h" => config.erle.max_h = number(value)?,
        "erle.onset_detection" => config.erle.onset_detection = flag(value)?,
        "erle.num_sections" => config.erle.num_sections = number(value)?,
        "ep_strength.default_gain" => config.ep_strength.default_gain = number(value)?,
        "ep_strength.default_len" => config.ep_strength.default_len = number(value)?,
        "ep_strength.nearend_len" => config.ep_strength.nearend_len = number(value)?,
        "ep_strength.bounded_erl" => config.ep_strength.bounded_erl = flag(value)?,
        "ep_strength.echo_can_saturate" => config.ep_strength.echo_can_saturate = flag(value)?,
        "echo_removal_control.has_clock_drift" => {
            config.echo_removal_control.has_clock_drift = flag(value)?;
        }
        "echo_removal_control.linear_and_stable_echo_path" => {
            config.echo_removal_control.linear_and_stable_echo_path = flag(value)?;
        }
        "echo_removal_control.transparent_mode" => {
            config.echo_removal_control.transparent_mode = match value {
                "legacy" => TransparentModeType::Legacy,
                "hmm" => TransparentModeType::Hmm,
                other => return Err(format!("expected legacy or hmm, got {other}")),
            };
        }
        "echo_model.model_reverb_in_nonlinear_mode" => {
            config.echo_model.model_reverb_in_nonlinear_mode = flag(value)?;
        }
        "suppressor.scale" => {
            let scale: f32 = number(value)?;
            for tuning in [
                &mut suppressor.normal_tuning,
                &mut suppressor.nearend_tuning,
            ] {
                for mask in [&mut tuning.mask_lf, &mut tuning.mask_hf] {
                    mask.enr_transparent *= scale;
                    mask.enr_suppress *= scale;
                }
            }
        }
        "suppressor.normal.enr_transparent" => {
            let v = number(value)?;
            suppressor.normal_tuning.mask_lf.enr_transparent = v;
            suppressor.normal_tuning.mask_hf.enr_transparent = v;
        }
        "suppressor.normal.enr_suppress" => {
            let v = number(value)?;
            suppressor.normal_tuning.mask_lf.enr_suppress = v;
            suppressor.normal_tuning.mask_hf.enr_suppress = v;
        }
        "suppressor.normal.max_dec_factor_lf" => {
            suppressor.normal_tuning.max_dec_factor_lf = number(value)?;
        }
        "suppressor.normal.max_inc_factor" => {
            suppressor.normal_tuning.max_inc_factor = number(value)?;
        }
        "suppressor.nearend_average_blocks" => {
            suppressor.nearend_average_blocks = number(value)?;
        }
        "suppressor.dominant_nearend.enr_threshold" => {
            suppressor.dominant_nearend_detection.enr_threshold = number(value)?;
        }
        "suppressor.dominant_nearend.enr_exit_threshold" => {
            suppressor.dominant_nearend_detection.enr_exit_threshold = number(value)?;
        }
        "suppressor.dominant_nearend.snr_threshold" => {
            suppressor.dominant_nearend_detection.snr_threshold = number(value)?;
        }
        "suppressor.dominant_nearend.hold_duration" => {
            suppressor.dominant_nearend_detection.hold_duration = number(value)?;
        }
        "suppressor.dominant_nearend.trigger_threshold" => {
            suppressor.dominant_nearend_detection.trigger_threshold = number(value)?;
        }
        "suppressor.use_subband_nearend_detection" => {
            suppressor.use_subband_nearend_detection = flag(value)?;
        }
        "suppressor.conservative_hf_suppression" => {
            suppressor.conservative_hf_suppression = flag(value)?;
        }
        "suppressor.floor_first_increase" => suppressor.floor_first_increase = number(value)?,
        other => return Err(format!("unknown AEC3 setting {other}")),
    }
    Ok(())
}

/// The chain's state: one call's microphone stream.
pub struct Chain {
    high_pass: sonora::high_pass_filter::HighPassFilter,
    processor: BlockProcessor,
    render_blocker: FrameBlocker,
    capture_blocker: FrameBlocker,
    output_framer: BlockFramer,
    block: Block,
    /// The capture chunk in AEC3's range, as the high-pass filter takes it.
    capture: Vec<Vec<f32>>,
    /// One processed half, as the framer hands it back.
    half: Vec<Vec<Vec<f32>>>,
    /// Noise suppression and level control, when either is on.
    post: Option<AudioProcessing>,
    scratch: Vec<f32>,
}

impl Chain {
    pub fn new(settings: &ChainSettings) -> Self {
        let mut aec3 = settings.aec3.clone();
        if !aec3.validate() {
            log::debug!("[CALL] AEC3 clamped some of its settings to their limits");
        }
        let post = (settings.noise_suppression || settings.gain_control).then(|| {
            let stream = StreamConfig::new(RATE, 1);
            AudioProcessing::builder()
                .config(Config {
                    // Moderate is WebRTC's own default, about 12 dB off steady noise: a fan or
                    // a hum, not a voice.
                    noise_suppression: settings.noise_suppression.then(NoiseSuppression::default),
                    // The digital gain only. The input volume controller would move the
                    // system's microphone volume, which is the person's to set. It starts from
                    // no gain at all and rises only as far as a voice message's normalization.
                    gain_controller2: settings.gain_control.then(|| GainController2 {
                        input_volume_controller: false,
                        adaptive_digital: Some(AdaptiveDigital {
                            max_gain_db: MAX_GAIN_DB,
                            initial_gain_db: 0.0,
                            ..AdaptiveDigital::default()
                        }),
                        ..GainController2::default()
                    }),
                    ..Config::default()
                })
                .capture_config(stream)
                .render_config(stream)
                .build()
        });
        Self {
            high_pass: sonora::high_pass_filter::HighPassFilter::new(RATE as i32, 1),
            processor: BlockProcessor::new(&aec3, RATE as usize, 1, 1),
            render_blocker: FrameBlocker::new(1, 1),
            capture_blocker: FrameBlocker::new(1, 1),
            output_framer: BlockFramer::new(1, 1),
            block: Block::new(1, 1),
            capture: vec![vec![0.0; CHUNK]],
            half: vec![vec![vec![0.0; HALF]]],
            post,
            scratch: vec![0.0; CHUNK],
        }
    }

    /// One chunk of what the speaker played.
    pub fn render(&mut self, chunk: &[f32]) {
        let mut scaled = [0.0_f32; CHUNK];
        for (to, from) in scaled.iter_mut().zip(chunk) {
            *to = from * SCALE;
        }
        for half in scaled.as_chunks::<HALF>().0 {
            self.render_blocker
                .insert_sub_frame_and_extract_block(&[vec![&half[..]]], &mut self.block);
            self.processor.buffer_render(&self.block);
        }
        if self.render_blocker.is_block_available() {
            self.render_blocker.extract_block(&mut self.block);
            self.processor.buffer_render(&self.block);
        }
    }

    /// Processes one chunk of the microphone in place; `hint_ms` is where the canceller starts
    /// looking for the echo, which it finds itself.
    pub fn capture(&mut self, chunk: &mut [f32], hint_ms: Option<i32>) {
        if let Some(hint) = hint_ms {
            self.processor.set_audio_buffer_delay(hint.clamp(0, 500));
        }
        let saturated = chunk.iter().any(|s| s.abs() * SCALE >= 32_700.0);
        for (to, from) in self.capture[0].iter_mut().zip(chunk.iter()) {
            *to = from * SCALE;
        }
        self.high_pass.process_channels(&mut self.capture);
        for index in 0..2 {
            let half = &self.capture[0][index * HALF..(index + 1) * HALF];
            self.capture_blocker
                .insert_sub_frame_and_extract_block(&[vec![half]], &mut self.block);
            self.processor
                .process_capture(false, saturated, None, &mut self.block);
            self.output_framer
                .insert_block_and_extract_sub_frame(&self.block, &mut self.half);
            for (to, from) in chunk[index * HALF..(index + 1) * HALF]
                .iter_mut()
                .zip(&self.half[0][0])
            {
                *to = from / SCALE;
            }
        }
        if self.capture_blocker.is_block_available() {
            self.capture_blocker.extract_block(&mut self.block);
            self.processor
                .process_capture(false, saturated, None, &mut self.block);
            self.output_framer.insert_block(&self.block);
        }
        if let Some(post) = &mut self.post
            && post
                .process_capture_f32(&[&chunk[..]], &mut [&mut self.scratch[..]])
                .is_ok()
        {
            chunk.copy_from_slice(&self.scratch);
        }
    }

    /// Echo reduction, echo return loss and delay, as the canceller reports them.
    pub fn metrics(&self) -> (f64, f64, i32) {
        let metrics = self.processor.get_metrics();
        (
            metrics.echo_return_loss_enhancement,
            metrics.echo_return_loss,
            metrics.delay_ms,
        )
    }
}
