//! What a video call's pictures did, in numbers, and, on request, the pictures themselves.
//!
//! The counters go in the call's log: how many frames went out and how many of them were
//! keyframes (the largest thing a call sends), and how many of the peer's frames decoded. They are
//! counts and sizes only.
//!
//! The dump is for working on a call's video, never on by default: with `ZAPFAST_CALL_VIDEO_DUMP`
//! set to a folder, each call writes the access units it sent and the ones it received there, as
//! raw H.264 that ffmpeg and other tools read (`ffmpeg -i received.h264 out.mp4`). That is the
//! picture of both people on the call, so only set it for a call whose other side agreed.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

/// The environment variable that names the folder a call's video is dumped to.
pub const DUMP_VARIABLE: &str = "ZAPFAST_CALL_VIDEO_DUMP";

/// The NAL units of an Annex-B access unit, each without its start code.
pub fn nal_units(access_unit: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut rest = access_unit;
    std::iter::from_fn(move || {
        let start = start_code_end(rest)?;
        rest = &rest[start..];
        let end = next_start_code(rest).unwrap_or(rest.len());
        let (unit, tail) = rest.split_at(end);
        rest = tail;
        // A four-byte start code leaves a zero at the end of the unit before it.
        let unit = unit.strip_suffix(&[0]).unwrap_or(unit);
        Some(unit)
    })
}

/// Where the first start code in `data` ends.
fn start_code_end(data: &[u8]) -> Option<usize> {
    data.windows(3)
        .position(|window| window == [0, 0, 1])
        .map(|at| at + 3)
}

/// Where the next start code in `data` begins.
fn next_start_code(data: &[u8]) -> Option<usize> {
    data.windows(3).position(|window| window == [0, 0, 1])
}

/// Whether an access unit holds an IDR slice: a picture a decoder can start from.
pub fn has_idr(access_unit: &[u8]) -> bool {
    nal_units(access_unit).any(|unit| unit.first().is_some_and(|header| header & 0x1f == 5))
}

/// What a sequence parameter set says of the stream: its profile, its constraint flags and its
/// level (H.264 7.3.2.1.1), the three bytes after the NAL header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamProfile {
    pub profile: u8,
    pub constraints: u8,
    pub level: u8,
}

impl StreamProfile {
    /// The profile's common name, as far as these bytes tell it.
    pub fn name(self) -> &'static str {
        match self.profile {
            66 if self.constraints & 0x40 != 0 => "Constrained Baseline",
            66 => "Baseline",
            77 => "Main",
            88 => "Extended",
            100 => "High",
            110 => "High 10",
            122 => "High 4:2:2",
            244 => "High 4:4:4",
            _ => "other",
        }
    }
}

/// The profile of the first sequence parameter set in an access unit, if it carries one.
pub fn stream_profile(access_unit: &[u8]) -> Option<StreamProfile> {
    nal_units(access_unit)
        .find(|unit| unit.first().is_some_and(|header| header & 0x1f == 7))
        .and_then(|sps| match sps {
            [_, profile, constraints, level, ..] => Some(StreamProfile {
                profile: *profile,
                constraints: *constraints,
                level: *level,
            }),
            _ => None,
        })
}

/// Counts of one call's pictures, both ways, read and reset by the call's heartbeat.
#[derive(Debug, Default)]
pub struct VideoCounters {
    sent_frames: AtomicU32,
    sent_keyframes: AtomicU32,
    largest_keyframe: AtomicU32,
    decoded: AtomicU32,
    undecodable: AtomicU32,
}

/// What [`VideoCounters::take`] read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VideoCounts {
    pub sent_frames: u32,
    pub sent_keyframes: u32,
    /// The largest keyframe sent, in bytes.
    pub largest_keyframe: u32,
    /// The peer's frames that decoded, and the ones the decoder refused.
    pub decoded: u32,
    pub undecodable: u32,
}

impl VideoCounters {
    /// One access unit went to the engine.
    pub fn sent(&self, access_unit: &[u8]) {
        self.sent_frames.fetch_add(1, Ordering::Relaxed);
        if has_idr(access_unit) {
            self.sent_keyframes.fetch_add(1, Ordering::Relaxed);
            let bytes = u32::try_from(access_unit.len()).unwrap_or(u32::MAX);
            self.largest_keyframe.fetch_max(bytes, Ordering::Relaxed);
        }
    }

    pub fn decoded(&self) {
        self.decoded.fetch_add(1, Ordering::Relaxed);
    }

    pub fn undecodable(&self) {
        self.undecodable.fetch_add(1, Ordering::Relaxed);
    }

    /// The counts since the last time this was asked.
    pub fn take(&self) -> VideoCounts {
        VideoCounts {
            sent_frames: self.sent_frames.swap(0, Ordering::Relaxed),
            sent_keyframes: self.sent_keyframes.swap(0, Ordering::Relaxed),
            largest_keyframe: self.largest_keyframe.swap(0, Ordering::Relaxed),
            decoded: self.decoded.swap(0, Ordering::Relaxed),
            undecodable: self.undecodable.swap(0, Ordering::Relaxed),
        }
    }
}

/// Everything one call keeps about its pictures: the counts, and the dumps of both directions.
pub struct CallVideo {
    pub counters: VideoCounters,
    pub sent: Dump,
    pub received: Dump,
}

impl CallVideo {
    pub fn new(call_id: &str) -> Self {
        Self {
            counters: VideoCounters::default(),
            sent: Dump::open(call_id, "sent"),
            received: Dump::open(call_id, "received"),
        }
    }
}

/// One direction of a call's video, written as raw H.264, when [`DUMP_VARIABLE`] asks for it.
pub struct Dump {
    file: Mutex<Option<std::io::BufWriter<std::fs::File>>>,
}

impl Dump {
    /// A dump of one direction (`sent` or `received`) of the call `call_id`, when the variable
    /// names a folder; a dump that writes nothing otherwise.
    pub fn open(call_id: &str, direction: &str) -> Self {
        let file = std::env::var_os(DUMP_VARIABLE)
            .filter(|folder| !folder.is_empty())
            .and_then(|folder| Self::create(Path::new(&folder), call_id, direction));
        Self {
            file: Mutex::new(file),
        }
    }

    fn create(
        folder: &Path,
        call_id: &str,
        direction: &str,
    ) -> Option<std::io::BufWriter<std::fs::File>> {
        let started = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let path = dump_path(folder, started, call_id, direction);
        let created = std::fs::create_dir_all(folder).and_then(|()| std::fs::File::create(&path));
        match created {
            Ok(file) => {
                log::info!("[CALL] writing the {direction} video to {}", path.display());
                Some(std::io::BufWriter::new(file))
            }
            Err(error) => {
                log::warn!("[CALL] the {direction} video cannot be dumped: {error}");
                None
            }
        }
    }

    /// Appends one access unit. A failed write stops the dump rather than the call.
    pub fn write(&self, access_unit: &[u8]) {
        let mut file = self.file.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(writer) = file.as_mut()
            && let Err(error) = writer.write_all(access_unit)
        {
            log::warn!("[CALL] the video dump stopped: {error}");
            *file = None;
        }
    }
}

/// The file one direction of a call's video is dumped to: named by when the video started and by
/// the call (an outgoing call has no id yet when its camera starts), so two calls never write to
/// one file, and by the direction.
fn dump_path(folder: &Path, started: u64, call_id: &str, direction: &str) -> PathBuf {
    let call: String = call_id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(32)
        .collect();
    let name = if call.is_empty() {
        format!("{started}-{direction}.h264")
    } else {
        format!("{started}-{call}-{direction}.h264")
    };
    folder.join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An access unit as the encoder writes one: an AUD, an SPS, a PPS and an IDR slice, with both
    /// lengths of start code.
    fn keyframe() -> Vec<u8> {
        let mut unit = vec![0, 0, 0, 1, 0x09, 0xf0];
        unit.extend([0, 0, 0, 1, 0x67, 66, 0xc0, 31, 0xaa]);
        unit.extend([0, 0, 1, 0x68, 0xce]);
        unit.extend([0, 0, 1, 0x65, 0x88, 0x84]);
        unit
    }

    #[test]
    fn an_access_unit_splits_into_its_nal_units() {
        let unit = keyframe();
        let headers: Vec<u8> = nal_units(&unit).map(|nal| nal[0] & 0x1f).collect();
        assert_eq!(headers, [9, 7, 8, 5]);
        assert_eq!(
            nal_units(&unit).nth(1),
            Some(&[0x67, 66, 0xc0, 31, 0xaa][..])
        );
    }

    #[test]
    fn a_keyframe_is_told_from_a_delta_frame() {
        assert!(has_idr(&keyframe()));
        assert!(!has_idr(&[0, 0, 0, 1, 0x09, 0xf0, 0, 0, 1, 0x41, 0x9a]));
    }

    #[test]
    fn the_stream_profile_is_read_from_its_parameter_set() {
        let profile = stream_profile(&keyframe()).expect("an SPS");
        assert_eq!(
            profile,
            StreamProfile {
                profile: 66,
                constraints: 0xc0,
                level: 31
            }
        );
        assert_eq!(profile.name(), "Constrained Baseline");
        assert_eq!(stream_profile(&[0, 0, 1, 0x41, 0x9a]), None);
    }

    #[test]
    fn counters_count_keyframes_and_reset_when_read() {
        let counters = VideoCounters::default();
        counters.sent(&keyframe());
        counters.sent(&[0, 0, 1, 0x41, 0x9a]);
        counters.decoded();
        counters.undecodable();
        let counts = counters.take();
        assert_eq!(counts.sent_frames, 2);
        assert_eq!(counts.sent_keyframes, 1);
        assert_eq!(counts.largest_keyframe, keyframe().len() as u32);
        assert_eq!((counts.decoded, counts.undecodable), (1, 1));
        assert_eq!(counters.take(), VideoCounts::default());
    }

    #[test]
    fn a_dump_file_is_named_by_the_call_and_the_direction() {
        let path = dump_path(
            Path::new("/dumps"),
            1_760_000_000,
            "00CA6C61/../x",
            "received",
        );
        assert_eq!(path, Path::new("/dumps/1760000000-00CA6C61x-received.h264"));
        let path = dump_path(Path::new("/dumps"), 1_760_000_000, "", "sent");
        assert_eq!(path, Path::new("/dumps/1760000000-sent.h264"));
    }

    #[test]
    fn a_dump_writes_what_it_is_given() {
        let folder = tempfile::tempdir().expect("a folder");
        let file = Dump::create(folder.path(), "call", "sent").expect("created");
        let dump = Dump {
            file: Mutex::new(Some(file)),
        };
        dump.write(&keyframe());
        drop(dump);
        let written = std::fs::read_dir(folder.path())
            .expect("listed")
            .flatten()
            .map(|entry| std::fs::read(entry.path()).expect("read"))
            .next()
            .expect("written");
        assert_eq!(written, keyframe());
    }
}
