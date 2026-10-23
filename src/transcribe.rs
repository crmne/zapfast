//! What a voice message says, transcribed on this computer.
//!
//! The speech model is the system's own, reached through a small Swift helper
//! (`zapfast-asr`, built beside the app and shipped in its bundle): the
//! `SpeechAnalyzer` API exists only in Swift, so `objc2` cannot reach it. The
//! audio never leaves the machine, nothing is downloaded from us, and the only
//! thing written to disk is a temporary WAV that is removed as soon as the
//! helper answers.
//!
//! On a Mac older than macOS 26, or when the helper is missing, the feature is
//! simply absent: [`available`] is false and the interface offers nothing.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Stored with every transcript, so a future engine can be told apart.
pub const ENGINE: &str = "speech-analyzer";
/// How long the helper may take before it is stopped. The first run on a Mac
/// may install the locale's model from Apple, which is a download; the rest
/// take a fraction of the clip's duration.
const HELPER_TIMEOUT: Duration = Duration::from_secs(300);

/// Whether this build and this Mac can transcribe.
pub fn available() -> bool {
    supported() && helper().is_some()
}

fn supported() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        #[cfg(target_os = "macos")]
        {
            use objc2_foundation::{NSOperatingSystemVersion, NSProcessInfo};
            NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(NSOperatingSystemVersion {
                majorVersion: 26,
                minorVersion: 0,
                patchVersion: 0,
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    })
}

/// The helper beside the running binary, as the bundle and the build leave it.
fn helper() -> Option<PathBuf> {
    static HELPER: OnceLock<Option<PathBuf>> = OnceLock::new();
    HELPER
        .get_or_init(|| {
            let exe = std::env::current_exe().ok()?;
            let dir = exe.parent()?;
            // `deps/` in a test build holds the binary; the helper sits one
            // level up.
            let mut candidates = vec![dir.join("zapfast-asr")];
            if let Some(up) = dir.parent() {
                candidates.push(up.join("zapfast-asr"));
            }
            candidates.into_iter().find(|path| path.is_file())
        })
        .clone()
}

/// Transcribes one WhatsApp OGG/Opus clip into text, in `locale`.
///
/// `scratch` receives the temporary WAV and must be private to the account.
pub fn transcribe(ogg: &Path, locale: &str, scratch: &Path) -> Result<String, String> {
    let helper = helper().ok_or("the speech helper is not installed")?;
    let bytes = std::fs::read(ogg).map_err(|error| format!("could not read the audio: {error}"))?;
    let samples = crate::voice::decode(&bytes)
        .map_err(|error| format!("could not decode the audio: {error}"))?;
    if samples.is_empty() {
        return Err("the audio is empty".to_owned());
    }
    std::fs::create_dir_all(scratch)
        .map_err(|error| format!("could not prepare the scratch folder: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(scratch, std::fs::Permissions::from_mode(0o700));
    }
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let wav = scratch.join(format!(
        "{}-{}.wav",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    if let Err(error) = write_wav(&wav, &samples) {
        // A partial file must not outlive the failure that made it.
        let _ = std::fs::remove_file(&wav);
        return Err(error);
    }
    let answer = run_helper(&helper, locale, &wav);
    let _ = std::fs::remove_file(&wav);
    answer
}

fn run_helper(helper: &Path, locale: &str, wav: &Path) -> Result<String, String> {
    let mut child = Command::new(helper)
        .arg("--locale")
        .arg(locale)
        .arg(wav)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not start the speech helper: {error}"))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < HELPER_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("the speech helper took too long".to_owned());
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("could not wait for the speech helper: {error}"));
            }
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("could not read the speech helper's answer: {error}"))?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        let error = error.trim();
        return Err(if error.is_empty() {
            "the speech helper failed".to_owned()
        } else {
            error.to_owned()
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Writes mono 48 kHz PCM as a WAV the system's speech model reads.
fn write_wav(path: &Path, samples: &[f32]) -> Result<(), String> {
    let mut file = std::fs::File::create(path)
        .map_err(|error| format!("could not write the audio: {error}"))?;
    let data = (samples.len() * 2) as u32;
    let rate = crate::voice::RATE;
    let mut header = Vec::with_capacity(44);
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&(36 + data).to_le_bytes());
    header.extend_from_slice(b"WAVEfmt ");
    header.extend_from_slice(&16u32.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&rate.to_le_bytes());
    header.extend_from_slice(&(rate * 2).to_le_bytes());
    header.extend_from_slice(&2u16.to_le_bytes());
    header.extend_from_slice(&16u16.to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&data.to_le_bytes());
    file.write_all(&header)
        .map_err(|error| format!("could not write the audio: {error}"))?;
    let mut body = Vec::with_capacity(data as usize);
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
        body.extend_from_slice(&value.to_le_bytes());
    }
    file.write_all(&body)
        .map_err(|error| format!("could not write the audio: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wav_header_names_pcm_mono_at_the_voice_rate() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("clip.wav");
        write_wav(&path, &[0.0, 0.5, -0.5]).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 42);
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(u16::from_le_bytes(bytes[20..22].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(bytes[22..24].try_into().unwrap()), 1);
        assert_eq!(
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            crate::voice::RATE
        );
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 6);
        assert_eq!(bytes.len(), 44 + 6);
    }

    #[test]
    fn a_clip_without_a_helper_is_reported_rather_than_guessed() {
        if available() {
            return;
        }
        let error = transcribe(Path::new("/nonexistent.ogg"), "pt-BR", Path::new(".")).unwrap_err();
        assert!(!error.is_empty());
    }
}
