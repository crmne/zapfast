//! Local speech recognition shared by every platform and every account.
//!
//! Only model weights are downloaded from pinned Hugging Face revisions.
//! Audio stays in memory, never goes into an HTTP request or a temporary WAV,
//! and the encrypted account archive owns the resulting text.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::Duration;

use parakeet_rs::{ExecutionConfig, ParakeetTDT, Transcriber};

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
    /// Whether preparation is queued or actively checking, downloading or loading.
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

/// Whether this copy of ZapFast carries the speech runtime. The recognizer
/// needs no system speech package, GPU driver, Python or helper executable.
pub fn available() -> bool {
    runtime_library().is_some()
}

/// Turns off ONNX Runtime's own usage reports before anything loads it: from
/// 1.29 on, Microsoft's macOS and Linux builds send them unless this is set,
/// and `ORT_DYLIB_PATH` can name such a build.
///
/// # Safety
///
/// Call first in `main`, while the process has a single thread.
pub unsafe fn disable_runtime_telemetry() {
    // SAFETY: the caller guarantees no other thread reads the environment.
    unsafe { std::env::set_var("ORT_DISABLE_TELEMETRY", "1") };
}

/// Where packages put the runtime, relative to the executable's folder.
#[cfg(windows)]
const RUNTIME_BESIDE_EXECUTABLE: &[&str] = &["onnxruntime.dll"];
#[cfg(target_os = "macos")]
const RUNTIME_BESIDE_EXECUTABLE: &[&str] =
    &["../Frameworks/libonnxruntime.dylib", "libonnxruntime.dylib"];
#[cfg(not(any(windows, target_os = "macos")))]
const RUNTIME_BESIDE_EXECUTABLE: &[&str] = &[
    "libonnxruntime.so",
    "../lib/zapfast/libonnxruntime.so",
    "../lib/libonnxruntime.so",
];

/// The runtime named by `ORT_DYLIB_PATH`, else the one a package was built
/// against (`ZAPFAST_ONNXRUNTIME`), else the one beside the executable.
fn runtime_library() -> Option<PathBuf> {
    static FOUND: OnceLock<Option<PathBuf>> = OnceLock::new();
    FOUND
        .get_or_init(|| {
            let executable = std::env::current_exe().ok();
            runtime_candidates(
                std::env::var_os("ORT_DYLIB_PATH").map(PathBuf::from),
                option_env!("ZAPFAST_ONNXRUNTIME").map(PathBuf::from),
                executable.as_deref().and_then(Path::parent),
            )
            .into_iter()
            .find(|path| path.is_file())
        })
        .clone()
}

fn runtime_candidates(
    named: Option<PathBuf>,
    built: Option<PathBuf>,
    folder: Option<&Path>,
) -> Vec<PathBuf> {
    let beside = folder.into_iter().flat_map(|folder| {
        RUNTIME_BESIDE_EXECUTABLE
            .iter()
            .map(move |relative| folder.join(relative))
    });
    named.into_iter().chain(built).chain(beside).collect()
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

/// Serializes account requests onto the process-wide worker and waits off the UI
/// thread. Channel failure is reported without exposing paths or audio data.
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

const RUNTIME_MISSING: &str = "This copy of ZapFast does not include the speech runtime. \
     Download ZapFast again from https://zapfast.rocks/download/ to add it.";

/// Microsoft's Windows build writes usage events to Event Tracing for Windows,
/// which Windows' diagnostic data can collect and send to Microsoft. The
/// switch is process-wide, and a Windows trace session that enables the
/// provider turns it back on, so it is turned off again before each model
/// load and each transcription. The macOS and Linux builds of 1.23 have no
/// such events.
fn silence_runtime_events() {
    // SAFETY: the runtime is loaded, and DisableTelemetryEvents ignores its
    // environment argument (onnxruntime_c_api.cc).
    let status = unsafe { (ort::api().DisableTelemetryEvents)(std::ptr::null()) };
    if !status.0.is_null() {
        // SAFETY: a status the runtime returned, released once.
        unsafe { (ort::api().ReleaseStatus)(status.0) };
    }
}

/// Why the runtime would not load, as far as this computer can tell.
fn runtime_failure() -> String {
    #[cfg(windows)]
    if let Some(library) = missing_visual_cpp_library() {
        let installer = if cfg!(target_arch = "aarch64") {
            "https://aka.ms/vs/17/release/vc_redist.arm64.exe"
        } else {
            "https://aka.ms/vs/17/release/vc_redist.x64.exe"
        };
        return format!(
            "The speech runtime needs the Microsoft Visual C++ Redistributable, \
             and {library} is missing. Install it from {installer}."
        );
    }
    "Could not start the speech runtime on this computer.".to_owned()
}

/// The first Visual C++ runtime library Microsoft's onnxruntime.dll imports
/// that Windows cannot find. The rest of what it imports ships with Windows.
#[cfg(windows)]
fn missing_visual_cpp_library() -> Option<&'static str> {
    use windows_sys::Win32::Foundation::FreeLibrary;
    use windows_sys::Win32::System::LibraryLoader::LoadLibraryW;
    let imported: &[&'static str] = if cfg!(target_arch = "aarch64") {
        &["VCRUNTIME140.dll", "MSVCP140.dll", "MSVCP140_1.dll"]
    } else {
        &[
            "VCRUNTIME140.dll",
            "VCRUNTIME140_1.dll",
            "MSVCP140.dll",
            "MSVCP140_1.dll",
        ]
    };
    imported.iter().copied().find(|name| {
        let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
        // SAFETY: a NUL-terminated name; a module found is released at once.
        let module = unsafe { LoadLibraryW(wide.as_ptr()) };
        if module.is_null() {
            return true;
        }
        // SAFETY: the module this call loaded.
        unsafe { FreeLibrary(module) };
        false
    })
}

/// ONNX Runtime allows only one environment to be built per process, even
/// after its last session was dropped. Keep the small runtime context alive
/// independently of the multi-gigabyte model sessions evicted on idle or a
/// model switch. Failed initialization is not cached, so it remains retryable.
fn environment() -> Result<ort::environment::Environment, Error> {
    static ENVIRONMENT: Mutex<Option<ort::environment::Environment>> = Mutex::new(None);
    let mut current = ENVIRONMENT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(environment) = &*current {
        return Ok(environment.clone());
    }
    let library = runtime_library().ok_or_else(|| Error::Model(RUNTIME_MISSING.to_owned()))?;
    let builder = ort::init_from(&library).map_err(|error| {
        log::warn!("could not load ONNX Runtime: {error}");
        Error::Model(runtime_failure())
    })?;
    // Before the environment exists, so its start event is not written either.
    silence_runtime_events();
    let environment = builder
        .with_name("ZapFast speech")
        .with_telemetry(false)
        .build()
        .map_err(|_| Error::Model("Could not start ONNX Runtime.".to_owned()))?;
    *current = Some(environment.clone());
    Ok(environment)
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

/// Rejects cancelled work before another download, model load or audio chunk.
fn check_cancel(cancel: &AtomicBool) -> Result<(), Error> {
    if cancel.load(Ordering::Relaxed) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

/// Reuses only the selected model/cache pair, then decodes the downloaded Opus
/// entirely in memory. Cancellation is checked before publishing any text.
fn run(job: &Job, loaded: &mut Option<Loaded>) -> Result<String, Error> {
    check_cancel(&job.cancel)?;
    if loaded
        .as_ref()
        .is_none_or(|current| current.model != job.model || current.root != job.root)
    {
        // Return the old model's memory before allocating its replacement.
        *loaded = None;
        // A computer where the runtime cannot start needs no model download.
        let environment = environment()?;
        let directory = download::ensure(job.model, &job.root, &job.cancel, job.progress.as_ref())?;
        check_cancel(&job.cancel)?;
        (job.progress)(State::Loading);
        let config = ExecutionConfig::new()
            .with_environment(environment)
            .with_intra_threads(4)
            .with_inter_threads(1);
        silence_runtime_events();
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
    // The model is loaded, so the runtime is too.
    silence_runtime_events();
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
    fn the_runtime_is_looked_for_where_it_is_named_then_where_packages_put_it() {
        let folder = Path::new("app");
        let found = runtime_candidates(Some("named".into()), Some("built".into()), Some(folder));
        assert_eq!(found[..2], [PathBuf::from("named"), PathBuf::from("built")]);
        assert_eq!(found.len(), 2 + RUNTIME_BESIDE_EXECUTABLE.len());
        assert!(found[2..].iter().all(|path| path.starts_with(folder)));
        assert!(runtime_candidates(None, None, None).is_empty());
    }

    #[test]
    #[ignore = "loads ONNX Runtime; requires ORT_DYLIB_PATH"]
    /// Pins the process-wide context across shared users and idle eviction.
    fn the_runtime_environment_outlives_its_model_sessions() {
        use ort::AsPointer;

        let first = environment().expect("first runtime context");
        let pointer = first.ptr();
        let also_live = environment().expect("another model can share the runtime");
        assert_eq!(pointer, also_live.ptr());
        drop(first);
        drop(also_live);
        let after_idle = environment().expect("runtime remains available after idle eviction");
        assert_eq!(pointer, after_idle.ptr());
    }

    #[test]
    /// Ensures chunk boundaries neither lose samples nor exceed the work budget.
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
    /// A cancelled job must touch neither the fixture path nor the model cache.
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
    #[ignore = "loads a real model; requires ORT_DYLIB_PATH, ZAPFAST_ASR_FIXTURE and ZAPFAST_ASR_MODELS"]
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

    /// Simulates idle eviction without a real-time minute-long sleep. The
    /// model is dropped between jobs, just as the speech thread does on idle.
    #[test]
    #[ignore = "loads a real model; requires ORT_DYLIB_PATH, ZAPFAST_ASR_FIXTURE and ZAPFAST_ASR_MODELS"]
    fn native_model_reloads_after_idle_without_restarting_the_process() {
        let audio = std::env::var_os("ZAPFAST_ASR_FIXTURE").expect("synthetic OGG fixture");
        let root = std::env::var_os("ZAPFAST_ASR_MODELS").expect("pinned model cache root");
        let job = Job {
            model: Model::default(),
            root: PathBuf::from(root),
            audio: Some(PathBuf::from(audio)),
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Box::new(|_| {}),
            reply: mpsc::sync_channel(1).0,
        };
        let mut loaded = None;
        let first = run(&job, &mut loaded).expect("first recognition");
        assert!(!first.is_empty());
        drop(loaded.take());
        let second = run(&job, &mut loaded).expect("recognition after idle eviction");
        assert_eq!(first, second);
        // Switching a model uses this same eviction/load path too.
        drop(loaded.take());
        assert_eq!(first, run(&job, &mut loaded).expect("another model reload"));
        if let Ok(selected) = std::env::var("ZAPFAST_ASR_SWITCH_MODEL") {
            let alternate = Job {
                model: Model::ALL
                    .into_iter()
                    .find(|model| model.key() == selected)
                    .expect("alternate model"),
                root: job.root.clone(),
                audio: job.audio.clone(),
                cancel: Arc::clone(&job.cancel),
                progress: Box::new(|_| {}),
                reply: mpsc::sync_channel(1).0,
            };
            assert_ne!(alternate.model, job.model);
            assert!(
                !run(&alternate, &mut loaded)
                    .expect("alternate model")
                    .is_empty()
            );
            assert_eq!(
                first,
                run(&job, &mut loaded).expect("switch back to the original model")
            );
        }
    }
}
