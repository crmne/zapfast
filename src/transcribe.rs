//! Local speech recognition shared by every platform and every account.
//!
//! Only model weights are downloaded from pinned Hugging Face revisions.
//! Audio stays in memory, never goes into an HTTP request or a temporary WAV,
//! and the encrypted account archive owns the resulting text.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Duration;

use parakeet_rs::{ExecutionConfig, ParakeetTDT, Transcriber, ort};

mod catalog;
mod download;
pub use catalog::Model;

/// Public, non-sensitive model progress. It contains no account or audio data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Queued,
    Checking,
    Downloading { received: u64, total: u64 },
    Loading,
    Ready,
    Paused,
    Failed(String),
}

impl State {
    pub fn busy(&self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Checking | Self::Downloading { .. } | Self::Loading
        )
    }
}

#[derive(Debug)]
pub enum Error {
    Cancelled,
    Model(String),
    Audio,
}

pub type Progress = Box<dyn Fn(State) + Send>;

/// The recognizer runs on all three desktop platforms, without a system
/// speech package, a GPU driver, Python or a helper executable.
pub fn available() -> bool {
    true
}

/// The common cache contains public weights only, never account data.
pub fn models_directory() -> PathBuf {
    crate::paths::AppDirs::discover()
        .cache
        .join("speech-models")
}

/// Downloads and prepares a model without reading an account or a recording.
pub fn prepare(model: Model, cancel: Arc<AtomicBool>, progress: Progress) -> Result<(), Error> {
    submit(model, None, models_directory(), cancel, progress).map(|_| ())
}

/// Transcribes a downloaded voice message. The caller keeps its account,
/// session and model generation, and rejects late results itself.
pub fn transcribe(
    model: Model,
    ogg: &Path,
    cancel: Arc<AtomicBool>,
    progress: Progress,
) -> Result<String, Error> {
    submit(
        model,
        Some(ogg.to_owned()),
        models_directory(),
        cancel,
        progress,
    )
}

struct Job {
    model: Model,
    audio: Option<PathBuf>,
    root: PathBuf,
    cancel: Arc<AtomicBool>,
    progress: Progress,
    reply: mpsc::SyncSender<Result<String, Error>>,
}

fn submit(
    model: Model,
    audio: Option<PathBuf>,
    root: PathBuf,
    cancel: Arc<AtomicBool>,
    progress: Progress,
) -> Result<String, Error> {
    static WORKER: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
    let sender = WORKER.get_or_init(|| {
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("zapfast-speech".to_owned())
            .spawn(move || worker(receiver))
            .expect("could not start the speech thread");
        sender
    });
    let (reply, answer) = mpsc::sync_channel(1);
    progress(State::Queued);
    sender
        .send(Job {
            model,
            audio,
            root,
            cancel,
            progress,
            reply,
        })
        .map_err(|_| Error::Model("The speech worker stopped. Restart ZapFast.".to_owned()))?;
    answer
        .recv()
        .map_err(|_| Error::Model("The speech worker stopped. Restart ZapFast.".to_owned()))?
}

struct Loaded {
    model: Model,
    root: PathBuf,
    engine: ParakeetTDT,
}

/// One model across the whole process, not one multi-gigabyte copy per
/// account. An idle worker releases its sessions and all model memory.
fn worker(receiver: mpsc::Receiver<Job>) {
    let mut loaded = None;
    loop {
        match receiver.recv_timeout(Duration::from_secs(60)) {
            Ok(job) => {
                let result = run(&job, &mut loaded);
                match &result {
                    Err(Error::Cancelled) => (job.progress)(State::Paused),
                    Err(Error::Model(message)) => (job.progress)(State::Failed(message.clone())),
                    _ => {}
                }
                let _ = job.reply.send(result);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => loaded = None,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), Error> {
    if cancel.load(Ordering::Relaxed) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

fn run(job: &Job, loaded: &mut Option<Loaded>) -> Result<String, Error> {
    check_cancel(&job.cancel)?;
    if loaded
        .as_ref()
        .is_none_or(|current| current.model != job.model || current.root != job.root)
    {
        // Return the old model's memory before allocating its replacement.
        *loaded = None;
        let directory = download::ensure(job.model, &job.root, &job.cancel, job.progress.as_ref())?;
        check_cancel(&job.cancel)?;
        (job.progress)(State::Loading);
        let environment = ort::init()
            .with_name("ZapFast speech")
            .with_telemetry(false)
            .build()
            .map_err(|_| Error::Model("Could not start ONNX Runtime.".to_owned()))?;
        let config = ExecutionConfig::new()
            .with_environment(environment)
            .with_intra_threads(4)
            .with_inter_threads(1);
        let engine = ParakeetTDT::from_pretrained(directory, Some(config))
            .map_err(|_| Error::Model("Could not load the speech model. Try again.".to_owned()))?;
        check_cancel(&job.cancel)?;
        *loaded = Some(Loaded {
            model: job.model,
            root: job.root.clone(),
            engine,
        });
    }
    (job.progress)(State::Ready);
    let Some(path) = &job.audio else {
        return Ok(String::new());
    };
    let bytes = std::fs::read(path).map_err(|_| Error::Audio)?;
    let samples = crate::voice::decode_at_rate(&bytes, 16_000).map_err(|_| Error::Audio)?;
    if samples.is_empty() {
        return Err(Error::Audio);
    }
    let mut text = String::new();
    let engine = &mut loaded.as_mut().expect("the speech model was loaded").engine;
    for part in chunks(&samples, 16_000) {
        check_cancel(&job.cancel)?;
        let result = engine
            .transcribe_samples(part.to_vec(), 16_000, 1, None)
            .map_err(|_| Error::Audio)?;
        let piece = result.text.trim();
        if !text.is_empty() && !piece.is_empty() {
            text.push(' ');
        }
        text.push_str(piece);
    }
    check_cancel(&job.cancel)?;
    Ok(text)
}

/// Bound memory and cancellation latency on long voice notes. Prefer a quiet
/// 200 ms interval between 12 and 25 seconds instead of cutting through a word.
fn chunks(samples: &[f32], rate: u32) -> Vec<&[f32]> {
    let rate = rate as usize;
    let mut parts = Vec::new();
    let mut start = 0;
    while samples.len() - start > 25 * rate {
        let quiet = rate / 5;
        let step = rate / 50;
        let low = start + 12 * rate;
        let high = start + 25 * rate;
        let end = (low..high - quiet)
            .step_by(step)
            .rfind(|&at| {
                samples[at..at + quiet]
                    .iter()
                    .map(|value| value * value)
                    .sum::<f32>()
                    / (quiet as f32)
                    < 0.000_04
            })
            .map_or(high, |at| at + quiet / 2);
        parts.push(&samples[start..end]);
        start = end;
    }
    if start < samples.len() {
        parts.push(&samples[start..]);
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunking_preserves_every_sample_and_bounds_the_work() {
        let samples = vec![0.5; crate::voice::RATE as usize * 68];
        let pieces = chunks(&samples, crate::voice::RATE);
        assert_eq!(
            pieces.iter().map(|part| part.len()).sum::<usize>(),
            samples.len()
        );
        assert!(
            pieces
                .iter()
                .all(|part| part.len() <= 25 * crate::voice::RATE as usize)
        );
        assert_eq!(pieces.concat(), samples);
    }

    #[test]
    fn cancellation_stops_before_loading_or_reading_audio() {
        let root = tempfile::tempdir().unwrap();
        let job = Job {
            model: Model::default(),
            root: root.path().to_owned(),
            audio: Some(PathBuf::from("must-not-be-read.ogg")),
            cancel: Arc::new(AtomicBool::new(true)),
            progress: Box::new(|_| {}),
            reply: mpsc::sync_channel(1).0,
        };
        assert!(matches!(run(&job, &mut None), Err(Error::Cancelled)));
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    /// Optional, explicitly supplied fixture, never the user's archive. Use a
    /// synthetic OGG and pre-downloaded models to validate a packaged engine.
    #[test]
    #[ignore = "loads a real model; requires ZAPFAST_ASR_FIXTURE and ZAPFAST_ASR_MODELS"]
    fn native_model_transcribes_an_explicit_fixture() {
        let audio = std::env::var_os("ZAPFAST_ASR_FIXTURE").expect("synthetic OGG fixture");
        let root = std::env::var_os("ZAPFAST_ASR_MODELS").expect("pinned model cache root");
        let selected = std::env::var("ZAPFAST_ASR_MODEL")
            .unwrap_or_else(|_| Model::default().key().to_owned());
        let model = Model::ALL
            .into_iter()
            .find(|model| model.key() == selected)
            .expect("known model");
        let text = submit(
            model,
            Some(PathBuf::from(audio)),
            PathBuf::from(root),
            Arc::new(AtomicBool::new(false)),
            Box::new(|_| {}),
        )
        .expect("native speech recognition");
        assert!(!text.trim().is_empty());
        if let Ok(expected) = std::env::var("ZAPFAST_ASR_EXPECTED") {
            assert!(text.to_lowercase().contains(&expected.to_lowercase()));
        }
        // Explicit offline probe only, never a production log.
        println!("{text}");
    }
}
