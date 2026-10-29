//! A call's echo canceller's input and output, saved for working on it offline. Never on by
//! default.
//!
//! With `ZAPFAST_CALL_AUDIO_DUMP` set to a folder, each microphone stream of a call writes four
//! files there, named `<time>-<stream>-…`:
//!
//! - `reference.wav`: the far end as the canceller was fed it, the peer's voice as the speaker
//!   played it;
//! - `microphone.wav`: our microphone before any processing;
//! - `output.wav`: what went to the engine after processing;
//! - `order.csv`: for each 10 ms microphone chunk, how many reference chunks were fed before it
//!   and the delay hint the canceller was given, so a replay feeds the canceller exactly as the
//!   call did (`examples/aec_replay.rs`).
//!
//! The WAV files are mono, 16 kHz, 32-bit float, in the canceller's own 10 ms chunks:
//! `microphone.wav` and `output.wav` are sample for sample the same timeline. They are both
//! people's voices, so only set it for a call whose other side agreed, and delete the files when
//! done. The writes are buffered and happen on the microphone's reading thread, never on an audio
//! callback.

use std::fs::File;
use std::io::{BufWriter, Read as _, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use super::RATE;

/// The environment variable that names the folder a call's audio is dumped to.
pub const DUMP_VARIABLE: &str = "ZAPFAST_CALL_AUDIO_DUMP";
/// How many chunks are written between rewriting the WAV headers' lengths, so a file cut short by
/// a crash still opens: a second.
const PATCH_EVERY: u32 = 100;

/// One microphone stream's dump: four files, written together.
pub(crate) struct AudioDump {
    reference: Wav,
    microphone: Wav,
    output: Wav,
    order: BufWriter<File>,
    chunks: u32,
}

impl AudioDump {
    /// A dump when [`DUMP_VARIABLE`] names a folder, else none.
    pub(crate) fn from_environment() -> Option<Self> {
        let folder = std::env::var_os(DUMP_VARIABLE).filter(|folder| !folder.is_empty())?;
        match Self::create(Path::new(&folder)) {
            Ok(dump) => Some(dump),
            Err(error) => {
                log::warn!("[CALL] the call's audio cannot be dumped: {error}");
                None
            }
        }
    }

    pub(crate) fn create(folder: &Path) -> std::io::Result<Self> {
        static STREAM: AtomicU32 = AtomicU32::new(1);
        let started = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let stream = STREAM.fetch_add(1, Ordering::Relaxed);
        std::fs::create_dir_all(folder)?;
        let path = |name: &str| folder.join(format!("{started}-{stream}-{name}"));
        let mut order = BufWriter::new(File::create(path("order.csv"))?);
        writeln!(order, "reference_chunks_before,delay_hint_ms")?;
        let dump = Self {
            reference: Wav::create(&path("reference.wav"))?,
            microphone: Wav::create(&path("microphone.wav"))?,
            output: Wav::create(&path("output.wav"))?,
            order,
            chunks: 0,
        };
        log::info!(
            "[CALL] writing the call's audio to {}",
            path("{reference,microphone,output}.wav").display()
        );
        Ok(dump)
    }

    /// One reference chunk, as it was fed.
    pub(crate) fn reference(&mut self, chunk: &[f32]) {
        self.reference.write(chunk);
    }

    /// One microphone chunk before processing, how many reference chunks came before it since the
    /// last, and the delay hint given with it.
    pub(crate) fn microphone(&mut self, chunk: &[f32], fed: u32, hint_ms: i32) {
        self.microphone.write(chunk);
        let _ = writeln!(self.order, "{fed},{hint_ms}");
    }

    /// One chunk as it went to the engine.
    pub(crate) fn output(&mut self, chunk: &[f32]) {
        self.output.write(chunk);
        self.chunks += 1;
        if self.chunks.is_multiple_of(PATCH_EVERY) {
            self.patch();
        }
    }

    fn patch(&mut self) {
        self.reference.patch();
        self.microphone.patch();
        self.output.patch();
        let _ = self.order.flush();
    }
}

impl Drop for AudioDump {
    fn drop(&mut self) {
        self.patch();
    }
}

/// A mono 16 kHz 32-bit float WAV file being written. A failed write stops it rather than the
/// call.
pub struct Wav {
    file: Option<BufWriter<File>>,
    samples: u32,
}

impl Wav {
    pub fn create(path: &Path) -> std::io::Result<Self> {
        let mut file = BufWriter::new(File::create(path)?);
        file.write_all(&header(0))?;
        Ok(Self {
            file: Some(file),
            samples: 0,
        })
    }

    pub fn write(&mut self, samples: &[f32]) {
        let Some(file) = &mut self.file else {
            return;
        };
        for sample in samples {
            if file.write_all(&sample.to_le_bytes()).is_err() {
                log::warn!("[CALL] the audio dump stopped");
                self.file = None;
                return;
            }
        }
        self.samples = self
            .samples
            .saturating_add(u32::try_from(samples.len()).unwrap_or(u32::MAX));
    }

    /// Rewrites the header's lengths, so the file opens whole up to here.
    pub fn patch(&mut self) {
        let Some(file) = &mut self.file else {
            return;
        };
        let header = header(self.samples);
        let patched = file
            .seek(SeekFrom::Start(0))
            .and_then(|_| file.write_all(&header))
            .and_then(|()| file.seek(SeekFrom::End(0)))
            .and_then(|_| file.flush());
        if patched.is_err() {
            self.file = None;
        }
    }
}

impl Drop for Wav {
    fn drop(&mut self) {
        self.patch();
    }
}

/// A WAV header for `samples` mono 32-bit float samples at [`RATE`].
fn header(samples: u32) -> [u8; 44] {
    let data = samples.saturating_mul(4);
    let mut header = [0_u8; 44];
    let mut at = 0;
    let mut put = |bytes: &[u8]| {
        header[at..at + bytes.len()].copy_from_slice(bytes);
        at += bytes.len();
    };
    put(b"RIFF");
    put(&data.saturating_add(36).to_le_bytes());
    put(b"WAVE");
    put(b"fmt ");
    put(&16_u32.to_le_bytes());
    // IEEE float, one channel.
    put(&3_u16.to_le_bytes());
    put(&1_u16.to_le_bytes());
    put(&RATE.to_le_bytes());
    put(&(RATE * 4).to_le_bytes());
    put(&4_u16.to_le_bytes());
    put(&32_u16.to_le_bytes());
    put(b"data");
    put(&data.to_le_bytes());
    header
}

/// Reads a WAV file's first channel and its rate: 32-bit float or 16-bit integer samples, so a
/// recording edited elsewhere still replays.
pub fn read_wav(path: &Path) -> std::io::Result<(u32, Vec<f32>)> {
    let invalid =
        |what: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, what.to_owned());
    let mut bytes = Vec::new();
    File::open(path)?.read_to_end(&mut bytes)?;
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(invalid("not a WAV file"));
    }
    let mut at = 12;
    let mut format = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]])
            as usize;
        let body = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
        if id == b"fmt " && body.len() >= 16 {
            let word = |at: usize| u16::from_le_bytes([body[at], body[at + 1]]);
            let rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
            format = Some((word(0), word(2).max(1), rate, word(14)));
        } else if id == b"data" {
            let (kind, channels, rate, bits) = format.ok_or_else(|| invalid("data before fmt"))?;
            let width = usize::from(bits / 8);
            let frame = width * usize::from(channels);
            let samples = body
                .chunks_exact(frame)
                .map(|frame| match (kind, bits) {
                    (3, 32) => Ok(f32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]])),
                    (1, 16) => Ok(f32::from(i16::from_le_bytes([frame[0], frame[1]])) / 32_768.0),
                    _ => Err(invalid("only 32-bit float and 16-bit WAV files are read")),
                })
                .collect::<Result<Vec<f32>, _>>()?;
            return Ok((rate, samples));
        }
        at += 8 + size + (size & 1);
    }
    Err(invalid("no data in the WAV file"))
}

/// The four files of one dump, by the name of its reference file.
pub fn siblings(reference: &Path) -> Option<[PathBuf; 4]> {
    let name = reference.file_name()?.to_str()?;
    let stem = name.strip_suffix("reference.wav")?;
    let folder = reference.parent()?;
    Some(
        ["reference.wav", "microphone.wav", "output.wav", "order.csv"]
            .map(|file| folder.join(format!("{stem}{file}"))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dump_writes_wav_files_that_read_back_and_an_order_to_replay() {
        let folder = tempfile::tempdir().expect("a folder");
        let reference = {
            let mut dump = AudioDump::create(folder.path()).expect("a dump");
            let chunk: Vec<f32> = (0..160).map(|i| i as f32 / 160.0 - 0.5).collect();
            for index in 0..250 {
                dump.reference(&chunk);
                dump.microphone(&chunk, u32::from(index % 2 == 0), 52);
                dump.output(&chunk);
            }
            let mut files: Vec<PathBuf> = std::fs::read_dir(folder.path())
                .expect("listed")
                .map(|entry| entry.expect("an entry").path())
                .collect();
            files.sort();
            assert_eq!(files.len(), 4, "{files:?}");
            files
                .into_iter()
                .find(|path| path.to_string_lossy().ends_with("reference.wav"))
                .expect("a reference file")
        };
        let [reference, microphone, output, order] = siblings(&reference).expect("its siblings");
        for path in [&reference, &microphone, &output] {
            let (rate, samples) = read_wav(path).expect("it reads back");
            assert_eq!(rate, RATE);
            assert_eq!(samples.len(), 250 * 160);
            assert_eq!(samples[1], 1.0 / 160.0 - 0.5);
        }
        let order = std::fs::read_to_string(order).expect("the order");
        let lines: Vec<&str> = order.lines().collect();
        assert_eq!(lines[0], "reference_chunks_before,delay_hint_ms");
        assert_eq!(lines[1], "1,52");
        assert_eq!(lines[2], "0,52");
        assert_eq!(lines.len(), 251);
    }

    #[test]
    fn nothing_is_dumped_without_the_variable() {
        // The suite never sets it; a dump appearing here would mean one appears in every call.
        if std::env::var_os(DUMP_VARIABLE).is_none() {
            assert!(AudioDump::from_environment().is_none());
        }
    }
}
