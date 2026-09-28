//! Fully local, on-demand Whisper transcription for voice messages.
//!
//! The multilingual large-v3-turbo model is downloaded separately, verified before it
//! is installed, and then used without sending recordings off this computer.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use sha2::{Digest, Sha256};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// Human-readable model name stored with encrypted transcript rows.
pub const MODEL_NAME: &str = "Whisper large-v3-turbo (multilingual)";
/// The full-precision multilingual large-v3-turbo model published for whisper.cpp.
pub const MODEL_FILE: &str = "ggml-large-v3-turbo.bin";
pub const MODEL_DOWNLOAD_LABEL: &str = "1.5 GB";
const MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin";
const MODEL_SHA256: &str = "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69";
/// A modest margin above the published 1,624,555,275-byte model, preventing an
/// unexpected response from consuming unbounded disk space.
const MAX_MODEL_BYTES: u64 = 1_700_000_000;
const LEGACY_MODEL_FILES: &[&str] = &["ggml-base.bin"];

/// User-visible progress for one local transcription request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    DownloadingVoice,
    DownloadingModel { received: u64, total: Option<u64> },
    InstallingModel,
    LoadingModel,
    Transcribing,
}

/// A completed local transcription and the digest of its source recording.
#[derive(Clone, Debug)]
pub struct Completed {
    pub text: String,
    pub source_sha256: String,
}

/// Serializes model installation and inference. This avoids loading the model
/// twice when two messages are requested together and keeps whisper.cpp's
/// native context away from the interface thread.
#[derive(Default)]
struct Engine {
    context: Option<WhisperContext>,
}

fn engine() -> &'static Mutex<Engine> {
    static ENGINE: OnceLock<Mutex<Engine>> = OnceLock::new();
    ENGINE.get_or_init(|| Mutex::new(Engine::default()))
}

pub fn model_path(directory: &Path) -> PathBuf {
    directory.join(MODEL_FILE)
}

pub fn model_installed(directory: &Path) -> bool {
    let path = model_path(directory);
    path.is_file() && sha256_file(&path).is_ok_and(|digest| digest == MODEL_SHA256)
}

/// Downloads and verifies the model into `directory` when it is absent.
/// A partial or unverified download never becomes the installed model.
fn ensure_model(directory: &Path, progress: &mut impl FnMut(Progress)) -> Result<PathBuf, String> {
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("Could not create the Whisper model folder: {error}"))?;
    let target = model_path(directory);
    if model_installed(directory) {
        return Ok(target);
    }

    progress(Progress::DownloadingModel {
        received: 0,
        total: None,
    });

    let partial = directory.join(format!("{MODEL_FILE}.download"));
    let _ = std::fs::remove_file(&partial);
    let mut builder = reqwest::blocking::Client::builder().timeout(Duration::from_secs(30 * 60));
    if let Some(proxy) = crate::proxy::reqwest_proxy() {
        builder = builder.proxy(proxy);
    }
    let client = builder
        .build()
        .map_err(|error| format!("Could not prepare the Whisper model download: {error}"))?;
    let mut response = client
        .get(MODEL_URL)
        .send()
        .map_err(|error| format!("Could not download the Whisper model: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Could not download the Whisper model (HTTP {})",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_MODEL_BYTES)
    {
        return Err("The Whisper model download was unexpectedly large".to_owned());
    }
    let expected = response.content_length();
    let mut file = std::fs::File::create(&partial)
        .map_err(|error| format!("Could not save the Whisper model: {error}"))?;
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut reported_percent = None;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|error| format!("Could not download the Whisper model: {error}"))?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > MAX_MODEL_BYTES {
            drop(file);
            let _ = std::fs::remove_file(&partial);
            return Err("The Whisper model download was unexpectedly large".to_owned());
        }
        hash.update(&buffer[..read]);
        file.write_all(&buffer[..read])
            .map_err(|error| format!("Could not save the Whisper model: {error}"))?;
        let percent = expected
            .filter(|expected| *expected > 0)
            .map(|expected| (total.saturating_mul(100) / expected).min(100));
        if percent != reported_percent {
            reported_percent = percent;
            progress(Progress::DownloadingModel {
                received: total,
                total: expected,
            });
        }
    }
    file.flush()
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("Could not finish saving the Whisper model: {error}"))?;

    let digest = hex_digest(hash.finalize());
    if digest != MODEL_SHA256 {
        let _ = std::fs::remove_file(&partial);
        return Err("The downloaded Whisper model failed its integrity check".to_owned());
    }
    progress(Progress::InstallingModel);
    if target.exists() {
        std::fs::remove_file(&target)
            .map_err(|error| format!("Could not replace the Whisper model: {error}"))?;
    }
    std::fs::rename(&partial, &target)
        .map_err(|error| format!("Could not install the Whisper model: {error}"))?;
    for legacy in LEGACY_MODEL_FILES {
        let _ = std::fs::remove_file(directory.join(legacy));
    }
    Ok(target)
}

/// Transcribes a WhatsApp OGG/Opus voice message with multilingual Whisper.
pub fn transcribe(model_directory: &Path, recording: &Path) -> Result<Completed, String> {
    transcribe_with_progress(model_directory, recording, |_| {})
}

/// Transcribes while reporting the one-time model download separately from
/// inference. The callback runs on the blocking transcription worker.
pub fn transcribe_with_progress(
    model_directory: &Path,
    recording: &Path,
    mut progress: impl FnMut(Progress),
) -> Result<Completed, String> {
    let mut engine = engine()
        .lock()
        .map_err(|_| "The Whisper engine could not be started".to_owned())?;
    let model = ensure_model(model_directory, &mut progress)?;
    let bytes = std::fs::read(recording)
        .map_err(|error| format!("Could not read the voice message: {error}"))?;
    let source_sha256 = sha256_bytes(&bytes);
    let samples = crate::voice::decode(&bytes)
        .map_err(|error| format!("Could not decode the voice message: {error}"))?;
    if samples.is_empty() {
        return Err("The voice message contains no audio".to_owned());
    }
    // WhatsApp Opus is 48 kHz and Whisper expects 16 kHz. Averaging each
    // exact group of three is a small low-pass filter as well as resampling.
    let (frames, _) = samples.as_chunks::<3>();
    let samples: Vec<f32> = frames
        .iter()
        .map(|frame| (frame[0] + frame[1] + frame[2]) / 3.0)
        .collect();

    if engine.context.is_none() {
        progress(Progress::LoadingModel);
        engine.context = Some(
            WhisperContext::new_with_params(
                model
                    .to_str()
                    .ok_or_else(|| "The Whisper model path is not valid text".to_owned())?,
                WhisperContextParameters::default(),
            )
            .map_err(|error| format!("Could not load the Whisper model: {error}"))?,
        );
    }
    progress(Progress::Transcribing);
    let mut state = engine
        .context
        .as_ref()
        .expect("the Whisper context was installed above")
        .create_state()
        .map_err(|error| format!("Could not start Whisper: {error}"))?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(None);
    params.set_translate(false);
    params.set_n_threads(inference_threads());
    params.set_no_timestamps(true);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_suppress_blank(true);
    params.set_suppress_nst(true);
    params.set_no_context(true);
    state
        .full(params, &samples)
        .map_err(|error| format!("Whisper could not transcribe this message: {error}"))?;
    let mut text = String::new();
    for segment in state.as_iter() {
        text.push_str(
            segment
                .to_str()
                .map_err(|error| format!("Whisper returned unreadable text: {error}"))?,
        );
    }
    let text = text.trim().to_owned();
    if text.is_empty() {
        return Err("Whisper did not find any speech in this message".to_owned());
    }
    Ok(Completed {
        text,
        source_sha256,
    })
}

fn inference_threads() -> i32 {
    let logical = std::thread::available_parallelism().map_or(1, usize::from);
    logical.div_ceil(2).clamp(4.min(logical), 16) as i32
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(hex_digest(hash.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    hex_digest(Sha256::digest(bytes))
}

fn hex_digest(digest: impl AsRef<[u8]>) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_hashes_are_stable() {
        assert_eq!(
            sha256_bytes(b"fixture"),
            "f16d05ec6b29248d2c61adb1e9263f78e4f7bace1b955014a2d17872cfe4064d"
        );
    }

    #[test]
    fn incomplete_model_is_not_installed() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(model_path(directory.path()), b"not a model").unwrap();
        assert!(!model_installed(directory.path()));
    }

    #[test]
    fn inference_uses_more_than_four_threads_on_large_cpus() {
        let expected = std::thread::available_parallelism().map_or(1, usize::from);
        assert!((1..=expected.min(16) as i32).contains(&inference_threads()));
        if expected >= 10 {
            assert!(inference_threads() > 4);
        }
    }

    /// Manual end-to-end probe with a real model and WhatsApp-style OGG:
    /// `ZAPFAST_WHISPER_MODEL_DIR=... ZAPFAST_WHISPER_PROBE=note.ogg cargo test
    /// transcribe::tests::probe -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn probe() {
        let Some(model_dir) = std::env::var_os("ZAPFAST_WHISPER_MODEL_DIR") else {
            return;
        };
        let Some(recording) = std::env::var_os("ZAPFAST_WHISPER_PROBE") else {
            return;
        };
        let started = std::time::Instant::now();
        let completed =
            transcribe(Path::new(&model_dir), Path::new(&recording)).expect("transcribes");
        let first = started.elapsed();
        assert!(!completed.text.is_empty());
        let started = std::time::Instant::now();
        let repeated =
            transcribe(Path::new(&model_dir), Path::new(&recording)).expect("transcribes again");
        assert_eq!(repeated.text, completed.text);
        println!(
            "{}\nfirst: {first:.2?}; warm: {:.2?}",
            completed.text,
            started.elapsed()
        );
    }
}
