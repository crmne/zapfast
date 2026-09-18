//! Clones a friend's voice from their sent voice notes and synthesizes text
//! messages in that voice, fully offline via the `vox` crate's Chatterbox
//! Turbo backend (zero-shot cloning from a reference WAV).

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use vox::{ChatterboxBackend, TtsBackend, TtsRequest};

use crate::archive::Archive;
use crate::paths::AppDirs;
use crate::voice;

/// Minimum seconds of voice notes required before a friend's voice unlocks.
pub const UNLOCK_THRESHOLD_SECONDS: u32 = 15;
/// Reference clip length trimmed from accumulated voice notes. Comfortably
/// above Chatterbox's recommended 5-20s of clean reference speech.
const REFERENCE_CLIP_SECONDS: usize = 15;

/// Process-wide Chatterbox engine, built lazily on first use so idle chats
/// never pay the ~700MB model download/load cost.
static BACKEND: OnceLock<Result<Arc<ChatterboxBackend>, String>> = OnceLock::new();

async fn backend() -> Result<Arc<ChatterboxBackend>, String> {
    tokio::task::spawn_blocking(|| {
        BACKEND
            .get_or_init(|| {
                // The reference wav is supplied per-request (see `synthesize`
                // below), so this constructor's default is never read.
                ChatterboxBackend::new("unused-default-reference.wav")
                    .map(Arc::new)
                    .map_err(|error| error.to_string())
            })
            .clone()
    })
    .await
    .map_err(|error| format!("voice engine init panicked: {error}"))?
}

/// Builds a reference clip from the friend's downloaded voice notes and
/// writes it under the app's voice-clone cache, for use as the Chatterbox
/// cloning reference.
pub fn build_reference_clip(
    archive: &Archive,
    dirs: &AppDirs,
    chat: &str,
) -> Result<PathBuf, String> {
    let paths = archive
        .friend_voice_note_paths(chat)
        .map_err(|error| error.to_string())?;
    let target_samples = REFERENCE_CLIP_SECONDS * voice::RATE as usize;
    let mut samples = Vec::new();
    for path in paths {
        if samples.len() >= target_samples {
            break;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(decoded) = voice::decode(&bytes) else {
            continue;
        };
        samples.extend(decoded);
    }
    if samples.is_empty() {
        return Err("no downloaded voice notes to clone from".into());
    }
    samples.truncate(target_samples);

    let dir = dirs.voice_ref_dir();
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let out_path = dirs.voice_ref_file(chat);
    write_wav(&out_path, &samples, voice::RATE, 1)?;
    Ok(out_path)
}

/// Synthesizes `text` in the friend's cloned voice, writing (and returning)
/// `cache_path`. Returns the cached path immediately if already synthesized.
pub async fn synthesize(
    text: &str,
    reference_wav: &Path,
    cache_path: &Path,
) -> Result<PathBuf, String> {
    if cache_path.exists() {
        return Ok(cache_path.to_path_buf());
    }
    let engine = backend().await?;
    let request = TtsRequest {
        text: text.to_owned(),
        voice: Some(reference_wav.to_string_lossy().into_owned()),
        seed: None,
    };
    let output = engine
        .synthesize(&request)
        .await
        .map_err(|error| error.to_string())?;
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    write_wav(
        cache_path,
        &output.audio.samples,
        output.audio.sample_rate,
        output.audio.channels,
    )?;
    Ok(cache_path.to_path_buf())
}

fn write_wav(path: &Path, samples: &[f32], rate: u32, channels: u16) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels,
        sample_rate: rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(|error| error.to_string())?;
    for sample in samples {
        writer
            .write_sample(*sample)
            .map_err(|error| error.to_string())?;
    }
    writer.finalize().map_err(|error| error.to_string())
}
