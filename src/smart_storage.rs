//! Bounded lossless message packing and validated, atomic video optimization.
//! Searchable message content stays in SQLite; only the duplicate protocol
//! payload is packed. User-saved downloads are never video candidates.
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant, UNIX_EPOCH};

pub const AGE: i64 = 90 * 24 * 60 * 60;
const MAGIC: &[u8] = b"\0ZAPFAST-GZIP1\0";
const MAX_RAW: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default)]
pub struct Report {
    pub messages: u64,
    pub videos: u64,
    pub saved_bytes: u64,
    pub failed: u64,
    pub video_unavailable: bool,
}

pub fn packed(raw: &[u8]) -> bool {
    raw.starts_with(MAGIC)
}
pub fn pack(raw: &[u8]) -> io::Result<Option<Vec<u8>>> {
    if packed(raw) || raw.len() < 256 || raw.len() > MAX_RAW {
        return Ok(None);
    }
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(raw)?;
    let mut result = MAGIC.to_vec();
    result.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    result.extend(encoder.finish()?);
    if result.len() * 100 >= raw.len() * 85 {
        return Ok(None);
    }
    if unpack(result.clone())? != raw {
        return Err(io::Error::other("Message compression verification failed"));
    }
    Ok(Some(result))
}
pub fn unpack(raw: Vec<u8>) -> io::Result<Vec<u8>> {
    if !packed(&raw) {
        return Ok(raw);
    }
    let corrupt = || io::Error::other("Compressed message data is damaged");
    let n = u32::from_le_bytes(
        raw.get(MAGIC.len()..MAGIC.len() + 4)
            .ok_or_else(corrupt)?
            .try_into()
            .map_err(|_| corrupt())?,
    ) as usize;
    if n > MAX_RAW {
        return Err(corrupt());
    }
    let mut result = Vec::with_capacity(n);
    GzDecoder::new(&raw[MAGIC.len() + 4..])
        .take(n as u64 + 1)
        .read_to_end(&mut result)?;
    if result.len() != n {
        return Err(corrupt());
    }
    Ok(result)
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Stamp {
    pub bytes: u64,
    pub seconds: u64,
    pub nanos: u32,
}
impl Stamp {
    pub fn of(path: &Path) -> io::Result<Self> {
        let info = std::fs::symlink_metadata(path)?;
        if !info.file_type().is_file() {
            return Err(io::Error::other("Not a regular cached file"));
        }
        let modified = info
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?;
        Ok(Self {
            bytes: info.len(),
            seconds: modified.as_secs(),
            nanos: modified.subsec_nanos(),
        })
    }
}
pub struct PreparedVideo {
    pub file: tempfile::NamedTempFile,
    pub path: PathBuf,
    pub stamp: Stamp,
}
impl std::fmt::Debug for PreparedVideo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PreparedVideo { .. }")
    }
}
#[derive(Debug)]
pub enum VideoResult {
    Ready(PreparedVideo),
    Skipped,
    Unavailable,
    Failed,
}

fn executable(name: &str) -> Option<PathBuf> {
    let found = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|dir| {
            dir.join(if cfg!(windows) {
                format!("{name}.exe")
            } else {
                name.to_owned()
            })
        })
        .find(|path| path.is_file());
    found.or_else(|| {
        ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"]
            .into_iter()
            .map(|dir| Path::new(dir).join(name))
            .find(|path| path.is_file())
    })
}
fn run(command: Command, cancel: &AtomicBool) -> io::Result<bool> {
    run_with_output(command, cancel, Stdio::null(), Duration::from_secs(30 * 60))
}
fn run_with_output(
    mut command: Command,
    cancel: &AtomicBool,
    stdout: Stdio,
    timeout: Duration,
) -> io::Result<bool> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(false);
    }
    command
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000 | 0x00004000);
    }
    let mut child = command.spawn()?;
    let start = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) || start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(false);
        }
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.success()),
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
fn ffmpeg_command(exe: &Path) -> Command {
    #[cfg(unix)]
    {
        let mut cmd = Command::new("nice");
        cmd.args(["-n", "15"]).arg(exe);
        cmd
    }
    #[cfg(not(unix))]
    {
        Command::new(exe)
    }
}
#[derive(Debug)]
struct VideoInfo {
    duration: f64,
    audio: usize,
}
fn probe(exe: &Path, path: &Path, cancel: &AtomicBool) -> Option<VideoInfo> {
    let mut output = tempfile::tempfile().ok()?;
    let mut command = ffmpeg_command(exe);
    command
        .args([
            "-v",
            "error",
            "-protocol_whitelist",
            "file,pipe",
            "-show_entries",
            "format=duration:stream=codec_type,color_transfer",
            "-of",
            "json",
        ])
        .arg(path);
    if !run_with_output(
        command,
        cancel,
        Stdio::from(output.try_clone().ok()?),
        Duration::from_secs(60),
    )
    .ok()?
    {
        return None;
    }
    use std::io::{Seek, SeekFrom};
    output.seek(SeekFrom::Start(0)).ok()?;
    let mut bytes = Vec::new();
    output.take(64 * 1024).read_to_end(&mut bytes).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let streams = json["streams"].as_array()?;
    // Plain yuv420p conversion would lose HDR tone mapping. Keep these originals.
    if streams.iter().any(|stream| {
        matches!(
            stream["color_transfer"].as_str(),
            Some("smpte2084" | "arib-std-b67")
        )
    }) {
        return None;
    }
    if streams
        .iter()
        .filter(|s| s["codec_type"] == "video")
        .count()
        != 1
        || streams
            .iter()
            .any(|s| !matches!(s["codec_type"].as_str(), Some("audio" | "video")))
    {
        return None;
    }
    let duration = json["format"]["duration"].as_str()?.parse::<f64>().ok()?;
    (duration.is_finite() && duration > 0.0).then_some(VideoInfo {
        duration,
        audio: streams
            .iter()
            .filter(|s| s["codec_type"] == "audio")
            .count(),
    })
}
fn cached(path: &Path, cache: &Path) -> bool {
    match (path.canonicalize(), cache.canonicalize()) {
        (Ok(path), Ok(cache)) => path.starts_with(cache),
        _ => false,
    }
}
pub fn prepare_video(
    path: PathBuf,
    cache: &Path,
    stamp: Stamp,
    cancel: Arc<AtomicBool>,
) -> VideoResult {
    if !cached(&path, cache)
        || Stamp::of(&path).ok().as_ref() != Some(&stamp)
        || stamp.bytes < 1024 * 1024
    {
        return VideoResult::Skipped;
    }
    let (Some(ffmpeg), Some(ffprobe)) = (executable("ffmpeg"), executable("ffprobe")) else {
        return VideoResult::Unavailable;
    };
    let Some(original) = probe(&ffprobe, &path, &cancel) else {
        return VideoResult::Skipped;
    };
    let Ok(file) = tempfile::Builder::new()
        .prefix(".zapfast-optimize-")
        .suffix(".mp4")
        .tempfile_in(cache)
    else {
        return VideoResult::Failed;
    };
    let mut cmd = ffmpeg_command(&ffmpeg);
    cmd.args(["-nostdin","-v","error","-y","-protocol_whitelist","file,pipe","-i"]).arg(&path).args(["-map","0:v:0","-map","0:a?","-map_metadata","0","-map_chapters","0","-vf","scale=w='min(1280,iw)':h='min(1280,ih)':force_original_aspect_ratio=decrease:force_divisible_by=2","-c:v","libx264","-preset","medium","-crf","25","-pix_fmt","yuv420p","-threads","2","-c:a","aac","-b:a","96k","-movflags","+faststart"]).arg(file.path());
    if !run(cmd, &cancel).unwrap_or(false) {
        return VideoResult::Failed;
    }
    let Ok(size) = file.as_file().metadata().map(|m| m.len()) else {
        return VideoResult::Failed;
    };
    if size >= stamp.bytes * 85 / 100 {
        return VideoResult::Skipped;
    }
    let Some(output) = probe(&ffprobe, file.path(), &cancel) else {
        return VideoResult::Failed;
    };
    if output.audio != original.audio
        || (output.duration - original.duration).abs() > 0.5f64.max(original.duration * 0.005)
    {
        return VideoResult::Failed;
    }
    let mut verify = ffmpeg_command(&ffmpeg);
    verify
        .args([
            "-nostdin",
            "-v",
            "error",
            "-protocol_whitelist",
            "file,pipe",
            "-i",
        ])
        .arg(file.path())
        .args(["-f", "null", "-"]);
    if !run(verify, &cancel).unwrap_or(false) || cancel.load(Ordering::Relaxed) {
        return VideoResult::Failed;
    }
    VideoResult::Ready(PreparedVideo { file, path, stamp })
}
impl PreparedVideo {
    pub fn publish(self, cache: &Path) -> io::Result<u64> {
        if !cached(&self.path, cache) || Stamp::of(&self.path)? != self.stamp {
            return Ok(0);
        }
        self.file.as_file().sync_all()?;
        let after = self.file.as_file().metadata()?.len();
        self.file.persist(&self.path).map_err(|error| error.error)?;
        Ok(self.stamp.bytes.saturating_sub(after))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_synthetic_video_is_smaller_playable_and_not_published_after_cancellation() {
        let (Some(ffmpeg), Some(ffprobe)) = (executable("ffmpeg"), executable("ffprobe")) else {
            return; // External tools are optional; exercised when installed.
        };
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("synthetic.mp4");
        let cancel = Arc::new(AtomicBool::new(false));
        let mut generate = ffmpeg_command(&ffmpeg);
        generate
            .args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=1280x720:rate=24",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-t",
                "3",
                "-c:v",
                "libx264",
                "-crf",
                "0",
                "-threads",
                "2",
                "-c:a",
                "aac",
            ])
            .arg(&path);
        assert!(run(generate, &cancel).unwrap());
        let stamp = Stamp::of(&path).unwrap();
        assert!(stamp.bytes >= 1024 * 1024);
        let VideoResult::Ready(video) =
            prepare_video(path.clone(), root.path(), stamp.clone(), cancel.clone())
        else {
            panic!("the synthetic lossless video should have worthwhile savings");
        };
        let saved = video.publish(root.path()).unwrap();
        assert!(saved > stamp.bytes * 15 / 100);
        let info = probe(&ffprobe, &path, &cancel).unwrap();
        assert_eq!(info.audio, 1);
        assert!((info.duration - 3.0).abs() < 0.1);
        let unchanged = std::fs::read(&path).unwrap();
        cancel.store(true, Ordering::Relaxed);
        assert!(!matches!(
            prepare_video(path.clone(), root.path(), Stamp::of(&path).unwrap(), cancel),
            VideoResult::Ready(_)
        ));
        assert_eq!(std::fs::read(path).unwrap(), unchanged);
    }
    #[test]
    fn message_packing_is_lossless_bounded_and_never_grows_small_data() {
        let raw = b"Searchable older message with repeated protocol fields. ".repeat(200);
        let compressed = pack(&raw).unwrap().unwrap();
        assert!(compressed.len() < raw.len() / 2);
        assert_eq!(unpack(compressed.clone()).unwrap(), raw);
        assert!(pack(&compressed).unwrap().is_none());
        assert!(pack(b"small").unwrap().is_none());
        let mut corrupt = compressed;
        corrupt.truncate(corrupt.len() - 5);
        assert!(unpack(corrupt).is_err());
    }
    #[test]
    fn publication_refuses_a_changed_original_or_a_saved_download() {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("cache");
        std::fs::create_dir(&cache).unwrap();
        let path = cache.join("old.mp4");
        std::fs::write(&path, b"original").unwrap();
        let stamp = Stamp::of(&path).unwrap();
        let mut file = tempfile::NamedTempFile::new_in(&cache).unwrap();
        file.write_all(b"small").unwrap();
        std::fs::write(&path, b"newer original").unwrap();
        assert_eq!(
            PreparedVideo {
                file,
                path: path.clone(),
                stamp
            }
            .publish(&cache)
            .unwrap(),
            0
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"newer original");
        let saved = root.path().join("saved.mp4");
        std::fs::write(&saved, b"saved").unwrap();
        assert!(!cached(&saved, &cache));
    }
}
