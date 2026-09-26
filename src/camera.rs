//! Camera capture: the node read directly, with `ffmpeg` only as a fallback.
//!
//! On Linux the camera is read through V4L2 itself. The node is opened, its packed format is chosen,
//! and frames are read straight into the encoder's buffer, so nothing external is needed to make a
//! video call work. `ffmpeg` is started only when a node cannot be read that way, because AGENTS.md
//! keeps it as a fallback rather than a requirement.
//!
//! Discovery is native too: `/dev/video*` is walked and each node is asked what it is through
//! `VIDIOC_QUERYCAP` and `VIDIOC_ENUM_FMT`, rather than shelling out to `v4l2-ctl`. No external
//! tool is needed to list a camera or to open one.
//!
//! A platform without a capture here reports no camera at all, and a call offers none. That is
//! [`crate::calls::CallCapabilities::camera`], built from [`available`].
//!
//! Every frame is YUV 4:2:0, tightly packed, in the size the encoder was built for: the preview and
//! the encoder both read a known layout, and a portrait camera stays portrait because the size
//! comes from what the driver grants.

use std::sync::{Arc, Mutex};

/// What one read attempt did.
pub enum Read {
    /// A whole frame landed in the buffer.
    Frame,
    /// Nothing arrived within the poll window: the caller checks whether it should stop, then reads
    /// again. This is what keeps a stop honoured while a camera is holding the read.
    Idle,
    /// The camera is finished, either because it was stopped or because the device went away.
    Ended,
}

/// How long a read waits for a frame before handing control back to the caller.
///
/// Short enough that a stop is noticed promptly and long enough that a healthy 15 fps camera is
/// never mistaken for a stalled one.
#[cfg(target_os = "linux")]
const POLL: libc::c_int = 200;

/// Whether this platform can open a camera at all.
pub fn available() -> bool {
    cfg!(target_os = "linux")
}

/// The cameras the platform found, as `(id, label)`.
///
/// The id is what a call opens and the label is what the driver calls it, so a person recognises
/// the entry and a saved setting can reopen it.
pub fn cameras() -> Vec<(String, String)> {
    #[cfg(target_os = "linux")]
    {
        v4l2::cameras()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Vec::new()
    }
}

/// The size a node delivers right now, for a caller that wants the camera's own shape before
/// opening it.
pub fn current_size(device: &str) -> Option<(u32, u32)> {
    #[cfg(target_os = "linux")]
    {
        v4l2::current_size(device)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = device;
        None
    }
}

/// Whether a node can deliver frames, which is what makes it a camera rather than a metadata node.
pub fn is_capture(device: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        v4l2::is_capture(device)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = device;
        false
    }
}

/// Kills and reaps a capture child, whichever side still holds it.
///
/// Taking it out of the slot first is what lets the capture thread and its caller both ask to end
/// the camera: neither kills a pid twice, and the one that finds an empty slot reaps nothing and
/// moves on.
#[cfg(target_os = "linux")]
fn kill_child(slot: &Mutex<Option<std::process::Child>>) {
    let child = slot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
    if let Some(mut child) = child {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// The camera node being read.
#[cfg(target_os = "linux")]
type Node = v4l2::Node;

/// An open camera, reading YUV 4:2:0 frames at [`Source::size`].
///
/// On Linux a camera is either read through V4L2 or filled in by `ffmpeg`. A platform with no
/// capture backend has no values at all, so the call runtime needs no platform check to say so: a
/// source cannot be opened there, and [`available`] is what keeps a call from asking.
#[cfg(target_os = "linux")]
pub enum Source {
    /// The node read through V4L2, which is the main path.
    Node(Node),
    /// An `ffmpeg` child writing raw YUV 4:2:0 to its stdout, used only when the node cannot be
    /// read directly. The child is shared because killing it, from here or from a stop, is what
    /// closes the stdout a blocked read is waiting on.
    Ffmpeg {
        child: Arc<Mutex<Option<std::process::Child>>>,
        stdout: std::io::BufReader<std::process::ChildStdout>,
        budget: (usize, usize),
    },
}

/// A platform with no capture backend: there is no source to read frames from, so this type has no
/// values and every method below is unreachable by construction.
#[cfg(not(target_os = "linux"))]
pub enum Source {}

#[cfg(target_os = "linux")]
impl Source {
    /// Opens `device`, trying the node first.
    ///
    /// `ffmpeg` is started only when the node will not give a format this module can convert, so a
    /// camera that works natively never spawns a process. A node the driver does not call a camera
    /// is an error rather than a capture process that could only fail frame by frame. `child` is the
    /// slot a stop outside the capture loop kills, which is what ends a read blocked on the
    /// fallback's stdout.
    pub fn open(
        device: &str,
        budget: (usize, usize),
        child: Arc<Mutex<Option<std::process::Child>>>,
    ) -> Result<Self, String> {
        match Node::open(device, budget) {
            Ok(node) => Ok(Source::Node(node)),
            Err(error) => {
                if !is_capture(device) {
                    return Err(format!("{device} is not a camera: {error}"));
                }
                log::info!("[CALL] reading the camera directly is not possible: {error}");
                Source::fallback(device, budget, child)
            }
        }
    }

    /// The `ffmpeg` path, which scales and pads into the encoder's frame itself.
    fn fallback(
        device: &str,
        budget: (usize, usize),
        slot: Arc<Mutex<Option<std::process::Child>>>,
    ) -> Result<Self, String> {
        let (width, height) = budget;
        let filter = format!(
            "scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2,format=yuv420p"
        );
        let mut process = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "v4l2",
                "-i",
                device,
                "-vf",
                &filter,
                "-r",
                "15",
                "-pix_fmt",
                "yuv420p",
                "-f",
                "rawvideo",
                "-",
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|error| {
                format!("the camera could not be read directly and ffmpeg is unavailable: {error}")
            })?;
        let Some(stdout) = process.stdout.take() else {
            let _ = process.kill();
            let _ = process.wait();
            return Err("ffmpeg did not provide the camera stream".to_owned());
        };
        // Hand the process to the shared slot before the first read, so a stop that arrives while a
        // read is waiting for a frame still ends it. A stop that already ran is honoured too: its
        // kill found an empty slot and this fill is what the opened-source check below catches.
        *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(process);
        Ok(Source::Ffmpeg {
            child: slot,
            stdout: std::io::BufReader::new(stdout),
            budget,
        })
    }

    /// The frame size this source delivers, which is the size the encoder must be built for.
    pub fn size(&self) -> (usize, usize) {
        match self {
            Source::Node(node) => node.size(),
            Source::Ffmpeg { budget, .. } => *budget,
        }
    }

    /// Reads one whole frame into `out`, which must be `width * height * 3 / 2` bytes.
    pub fn read(&mut self, out: &mut [u8]) -> Read {
        match self {
            Source::Node(node) => node.read(out),
            Source::Ffmpeg { stdout, .. } => {
                match std::io::Read::read_exact(stdout, out) {
                    Ok(()) => Read::Frame,
                    // A closed pipe is either the stop that killed the child or a camera that went
                    // away; either way there is nothing more to read.
                    Err(_) => Read::Ended,
                }
            }
        }
    }

    /// Ends the capture, unblocking a read that is waiting for a frame.
    ///
    /// A node ends its stream, which is what tells the driver its buffers are no longer read; its
    /// read returns every poll window anyway, which is where the caller notices that the
    /// camera is no longer wanted, and that is what keeps a stop honoured without the capture ever
    /// being killed out from under the device. A fallback child is killed, which is what closes the
    /// stdout a blocked read is waiting on.
    pub fn stop(&mut self) {
        match self {
            Source::Node(node) => node.stop(),
            Source::Ffmpeg { child, .. } => kill_child(child),
        }
    }

    /// Reaps the child, if there is one, once the capture loop has ended.
    pub fn reap(&mut self) {
        self.stop();
    }
}

#[cfg(not(target_os = "linux"))]
impl Source {
    /// A platform with no capture backend has no camera to open, and says so rather than handing
    /// back a source that would never deliver a frame.
    pub fn open(
        device: &str,
        _budget: (usize, usize),
        _child: Arc<Mutex<Option<std::process::Child>>>,
    ) -> Result<Self, String> {
        Err(format!(
            "{device} cannot be opened: this platform has no camera backend"
        ))
    }

    /// Unreachable: there is no source, so there is no frame size.
    pub fn size(&self) -> (usize, usize) {
        match *self {}
    }

    /// Unreachable: there is no source, so nothing is read.
    pub fn read(&mut self, _out: &mut [u8]) -> Read {
        match *self {}
    }

    /// Unreachable: there is no source, so there is no capture to end.
    pub fn stop(&mut self) {
        match *self {}
    }

    /// Unreachable: there is no source, so there is no child to reap.
    pub fn reap(&mut self) {
        match *self {}
    }
}

#[cfg(target_os = "linux")]
fn video_nodes() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir("/dev") else {
        return Vec::new();
    };
    let mut found: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let number = name.strip_prefix("video")?;
            number.parse::<u32>().ok()?;
            Some(format!("/dev/{name}"))
        })
        .collect();
    // A person recognises the order the kernel hands out and a saved setting must reopen the same
    // node, so the list is stable rather than the directory order.
    found.sort_by_key(|path| {
        path.strip_prefix("/dev/video")
            .and_then(|number| number.parse::<u32>().ok())
            .unwrap_or(u32::MAX)
    });
    found
}

#[cfg(target_os = "linux")]
mod v4l2 {
    //! V4L2 through `ioctl`, without a crate or an external tool.
    //!
    //! The kernel structs are filled in byte buffers and read at the offsets `videodev2.h` lays out,
    //! rather than through `repr(C)` mirrors: the header's unions are larger than any one member, and
    //! a mirror a byte short makes the kernel reject the call and write past the buffer. The offsets
    //! are named once, here, and used everywhere else.

    use super::{POLL, Read};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    /// The `_IOC` encoding of one request: direction, the size of the argument, the `'V'` type and
    /// the request number. Computed rather than written out, so a request can never disagree with
    /// the struct it is paired with.
    const fn request(direction: u32, number: u32, size: usize) -> libc::c_ulong {
        ((direction << 30) | ((size as u32) << 16) | ((b'V' as u32) << 8) | number) as libc::c_ulong
    }

    /// A request that reads its argument back out.
    const fn ior(number: u32, size: usize) -> libc::c_ulong {
        request(2, number, size)
    }

    /// A request that writes its argument in and reads it back.
    const fn iowr(number: u32, size: usize) -> libc::c_ulong {
        request(3, number, size)
    }

    /// A request that writes its argument in.
    const fn iow(number: u32, size: usize) -> libc::c_ulong {
        request(1, number, size)
    }

    /// `VIDIOC_QUERYCAP`, on a `v4l2_capability`.
    const QUERYCAP: libc::c_ulong = ior(0, CAP_LEN);
    /// `VIDIOC_ENUM_FMT`, on a `v4l2_fmtdesc`.
    const ENUM_FMT: libc::c_ulong = iowr(2, DESC_LEN);
    /// `VIDIOC_G_FMT` and `VIDIOC_S_FMT`, on a `v4l2_format`.
    const G_FMT: libc::c_ulong = iowr(4, FORMAT_LEN);
    const S_FMT: libc::c_ulong = iowr(5, FORMAT_LEN);
    /// `VIDIOC_REQBUFS`, on a `v4l2_requestbuffers`.
    const REQBUFS: libc::c_ulong = iowr(8, REQUESTBUFFERS_LEN);
    /// `VIDIOC_QUERYBUF`, `VIDIOC_QBUF` and `VIDIOC_DQBUF`, on a `v4l2_buffer`.
    const QUERYBUF: libc::c_ulong = iowr(9, BUFFER_LEN);
    const QBUF: libc::c_ulong = iowr(15, BUFFER_LEN);
    const DQBUF: libc::c_ulong = iowr(17, BUFFER_LEN);
    /// `VIDIOC_STREAMON` and `VIDIOC_STREAMOFF`, on a buffer type.
    const STREAMON: libc::c_ulong = iow(18, 4);
    const STREAMOFF: libc::c_ulong = iow(19, 4);
    /// `VIDIOC_G_PARM` and `VIDIOC_S_PARM`, on a `v4l2_streamparm`.
    const G_PARM: libc::c_ulong = iowr(21, PARM_LEN);
    const S_PARM: libc::c_ulong = iowr(22, PARM_LEN);

    const BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
    /// `V4L2_MEMORY_MMAP`, the streaming method every camera driver offers.
    const MEMORY_MMAP: u32 = 1;
    /// How many buffers are queued: enough that one is being filled while another is drained.
    const BUFFERS: u32 = 4;
    /// `V4L2_CAP_VIDEO_CAPTURE`, and the flag that says `device_caps` describes this node rather
    /// than the driver behind it.
    const CAP_VIDEO_CAPTURE: u32 = 0x0000_0001;
    /// `V4L2_CAP_DEVICE_CAPS`, the flag saying `device_caps` is the field to read.
    const FLAG_DEVICE_CAPS: u32 = 0x8000_0000;

    /// `V4L2_PIX_FMT_YUYV`, the packed 4:2:2 nearly every camera offers, and the one this module
    /// converts. A node that offers only something else is read through the `ffmpeg` fallback
    /// instead, which can scale and convert whatever the driver gives.
    pub(super) const FMT_YUYV: u32 = 0x5659_5559;

    /// One `v4l2_capability`, and where its fields sit.
    const CAP_LEN: usize = 104;
    const CAP_CARD: usize = 16;
    const CAP_CAPABILITIES: usize = 84;
    /// Where `device_caps` sits inside a `v4l2_capability`.
    const CAP_DEVICE_CAPS: usize = 88;
    /// One `v4l2_fmtdesc`, and where its fields sit.
    const DESC_LEN: usize = 64;
    const DESC_PIXEL_FORMAT: usize = 44;
    /// One `v4l2_requestbuffers`.
    const REQUESTBUFFERS_LEN: usize = 20;
    /// One `v4l2_streamparm`, whose union holds no pointer and so needs no padding.
    const PARM_LEN: usize = 204;

    /// One `v4l2_format`, as far as the pixel format arm reaches.
    ///
    /// A `repr(C)` mirror rather than byte offsets: the union starts past the alignment its first
    /// four-byte member allows, which on a 64-bit kernel means four bytes of padding the compiler
    /// has to reproduce. Getting that wrong makes the driver read a width out of a height and
    /// negotiate a size nobody asked for, which is exactly what a hand-written offset did here.
    /// The trailing bytes are the rest of the union, which this module never reads.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub(super) struct Format {
        kind: u32,
        /// The padding the union's eight-byte alignment requires.
        _padding: u32,
        width: u32,
        height: u32,
        pixel: u32,
        field: u32,
        stride: u32,
        sizeimage: u32,
        colorspace: u32,
        priv_: u32,
        flags: u32,
        ycbcr_enc: u32,
        quantization: u32,
        xfer_func: u32,
        /// The rest of the union, unread. Whole words rather than bytes, both because they are
        /// what the union is aligned to and because `Default` stops at thirty-two byte elements.
        _rest: [u64; 19],
    }

    /// The size of one `Format`, which is what the ioctl number is built from.
    const FORMAT_LEN: usize = std::mem::size_of::<Format>();

    /// Where the pixel format arm starts, past the padding the union's alignment needs.
    #[cfg(test)]
    pub(super) const PIXEL_ARM: usize = std::mem::offset_of!(Format, width);

    impl Format {
        /// A format request for the capture buffer type.
        fn for_capture() -> Self {
            Self {
                kind: BUF_TYPE_VIDEO_CAPTURE,
                ..Self::default()
            }
        }
    }

    /// Sends one format request, which needs the struct itself rather than a byte buffer.
    fn format_ioctl(
        fd: libc::c_int,
        request: libc::c_ulong,
        format: &mut Format,
    ) -> Result<(), String> {
        let result = unsafe { libc::ioctl(fd, request, format as *mut Format) };
        if result < 0 {
            Err(std::io::Error::last_os_error().to_string())
        } else {
            Ok(())
        }
    }

    /// Opens a node, owning the descriptor from the moment it exists.
    fn open(device: &str) -> std::io::Result<OwnedFd> {
        use std::os::unix::ffi::OsStrExt as _;
        let path = std::ffi::CString::new(std::ffi::OsStr::new(device).as_bytes())
            .map_err(|_| std::io::Error::other("the camera path has a NUL byte"))?;
        // Non-blocking, because every read is preceded by a poll with a timeout: that timeout is
        // what lets a stop be honoured while the camera holds no frame.
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_NONBLOCK) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    fn ioctl(fd: libc::c_int, request: libc::c_ulong, buffer: &mut [u8]) -> Result<(), String> {
        let result =
            unsafe { libc::ioctl(fd, request, buffer.as_mut_ptr().cast::<libc::c_void>()) };
        if result < 0 {
            Err(std::io::Error::last_os_error().to_string())
        } else {
            Ok(())
        }
    }

    fn u32_at(buffer: &[u8], at: usize) -> u32 {
        u32::from_ne_bytes(buffer[at..at + 4].try_into().expect("four bytes"))
    }

    fn put_u32(buffer: &mut [u8], at: usize, value: u32) {
        buffer[at..at + 4].copy_from_slice(&value.to_ne_bytes());
    }

    /// What the driver calls the node, and what it can do.
    fn capability(fd: libc::c_int) -> Result<(String, u32), String> {
        let mut buffer = vec![0_u8; CAP_LEN];
        ioctl(fd, QUERYCAP, &mut buffer)?;
        let card = String::from_utf8_lossy(&buffer[CAP_CARD..CAP_CARD + 32])
            .trim_end_matches('\0')
            .trim()
            .to_owned();
        let driver = u32_at(&buffer, CAP_CAPABILITIES);
        // A node that reports its own capabilities is the one being asked about.
        let capabilities = if driver & FLAG_DEVICE_CAPS != 0 {
            u32_at(&buffer, CAP_DEVICE_CAPS)
        } else {
            driver
        };
        Ok((card, capabilities))
    }

    /// The pixel formats a node enumerates.
    fn pixel_formats(fd: libc::c_int) -> Vec<u32> {
        let mut found = Vec::new();
        for index in 0..32_u32 {
            let mut buffer = vec![0_u8; DESC_LEN];
            put_u32(&mut buffer, 0, index);
            put_u32(&mut buffer, 4, BUF_TYPE_VIDEO_CAPTURE);
            if ioctl(fd, ENUM_FMT, &mut buffer).is_err() {
                break;
            }
            found.push(u32_at(&buffer, DESC_PIXEL_FORMAT));
        }
        found
    }

    /// Asks for one size and format, and answers with what the driver granted, including the row
    /// stride the granted format carries.
    fn set_format(
        fd: libc::c_int,
        pixel: u32,
        width: usize,
        height: usize,
    ) -> Result<(usize, usize, usize), String> {
        let mut format = Format::for_capture();
        format.width = width as u32;
        format.height = height as u32;
        format.pixel = pixel;
        format_ioctl(fd, S_FMT, &mut format)?;
        Ok((
            format.width as usize,
            format.height as usize,
            format.stride as usize,
        ))
    }

    /// The size and row stride the node is set to.
    fn get_format(fd: libc::c_int) -> Result<(usize, usize, usize), String> {
        let mut format = Format::for_capture();
        format_ioctl(fd, G_FMT, &mut format)?;
        Ok((
            format.width as usize,
            format.height as usize,
            format.stride as usize,
        ))
    }

    /// Asks for 15 frames a second. A driver that will not say is not a failure: the read loop does
    /// not depend on the rate.
    fn set_rate(fd: libc::c_int) {
        let mut buffer = vec![0_u8; 204];
        put_u32(&mut buffer, 0, BUF_TYPE_VIDEO_CAPTURE);
        if ioctl(fd, G_PARM, &mut buffer).is_err() {
            return;
        }
        // v4l2_streamparm's capture arm is capability, capturemode, then timeperframe, which is a
        // numerator and a denominator.
        buffer[12..16].copy_from_slice(&1_u32.to_ne_bytes());
        buffer[16..20].copy_from_slice(&15_u32.to_ne_bytes());
        let _ = ioctl(fd, S_PARM, &mut buffer);
    }

    /// Whether a node's formats can be read into the encoder's buffer as they are.
    ///
    /// Pure so the choice between the two capture paths is testable without a camera: a node that
    /// offers the packed 4:2:2 this module converts is read through V4L2, and one that offers only
    /// something else is read through the fallback rather than into a buffer whose layout would be
    /// wrong.
    pub fn readable_directly(formats: &[u32]) -> bool {
        formats.contains(&FMT_YUYV)
    }

    /// Whether a node is a camera: it captures and it enumerates at least one pixel format. The
    /// metadata node beside a camera reports capture but enumerates nothing, which is what this
    /// rules out. The formats do not have to include the one this module converts: a camera that
    /// offers only a compressed format is still a camera, and is read through the fallback.
    pub fn is_capture(device: &str) -> bool {
        let Ok(fd) = open(device) else {
            return false;
        };
        let raw = fd.as_raw_fd();
        let Ok((_, capabilities)) = capability(raw) else {
            return false;
        };
        capabilities & CAP_VIDEO_CAPTURE != 0 && !pixel_formats(raw).is_empty()
    }

    pub fn current_size(device: &str) -> Option<(u32, u32)> {
        let fd = open(device).ok()?;
        let (width, height, _) = get_format(fd.as_raw_fd()).ok()?;
        (width > 0 && height > 0).then_some((width as u32, height as u32))
    }

    pub fn cameras() -> Vec<(String, String)> {
        let mut found: Vec<(String, String)> = Vec::new();
        for device in super::video_nodes() {
            if !is_capture(&device) {
                continue;
            }
            let label = open(&device)
                .ok()
                .and_then(|fd| capability(fd.as_raw_fd()).ok())
                .map(|(card, _)| card)
                .filter(|card| !card.is_empty())
                .unwrap_or_else(|| device.clone());
            found.push((device, label));
        }
        found
    }

    /// One `v4l2_buffer`, laid out as the header does.
    ///
    /// A `repr(C)` mirror rather than byte offsets, because the struct carries a union whose
    /// alignment the compiler has to reproduce: a union of a pointer and a word is eight-byte
    /// aligned and padded like one, and it is modelled here as a single `u64`, which the compiler
    /// places exactly where the kernel does. The ioctl number is built from this type's own size,
    /// so the request sent to the kernel can never disagree with the struct it points at.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub(super) struct Buffer {
        index: u32,
        kind: u32,
        bytesused: u32,
        flags: u32,
        field: u32,
        timestamp: [u64; 2],
        timecode: [u8; 12],
        sequence: u32,
        memory: u32,
        offset: u64,
        length: u32,
        reserved2: u32,
        request_fd: u32,
    }

    /// The size of one `Buffer`, which is what the ioctl number is built from.
    const BUFFER_LEN: usize = std::mem::size_of::<Buffer>();

    impl Buffer {
        /// A buffer with its index, buffer type and memory model filled in, ready for one request.
        fn at(index: u32) -> Self {
            Self {
                index,
                kind: BUF_TYPE_VIDEO_CAPTURE,
                memory: MEMORY_MMAP,
                ..Self::default()
            }
        }
    }

    /// One buffer the driver fills, mapped into this process.
    struct Mapping {
        /// The buffer exactly as the driver described it in `QUERYBUF`.
        ///
        /// Kept and handed back on every queue request, because that is what the driver checks: a
        /// queue that carries a different length than the one the driver handed out is refused, so
        /// a buffer built from scratch and queued is rejected even though every field looks right.
        buffer: Buffer,
        pointer: *mut u8,
    }

    impl Mapping {
        /// Maps the buffer the driver described, taking its offset and length from the struct.
        fn map(fd: libc::c_int, buffer: Buffer) -> Result<Self, String> {
            let length = buffer.length as usize;
            let pointer = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    length,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    fd,
                    buffer.offset as i64,
                )
            };
            if pointer == libc::MAP_FAILED {
                return Err(std::io::Error::last_os_error().to_string());
            }
            Ok(Self {
                buffer,
                pointer: pointer.cast::<u8>(),
            })
        }

        /// The request that hands this buffer back to the driver, cleared of what a dequeue left in
        /// it: only a queue's own fields are sent, with the length and offset the driver gave.
        fn prepare_for_queue(&mut self) -> &mut Buffer {
            self.buffer.bytesused = 0;
            self.buffer.flags = 0;
            &mut self.buffer
        }

        /// The bytes this buffer spans, which is what the driver said it would fill.
        fn length(&self) -> usize {
            self.buffer.length as usize
        }

        fn pointer(&self) -> *const u8 {
            self.pointer
        }
    }

    impl Drop for Mapping {
        fn drop(&mut self) {
            unsafe { libc::munmap(self.pointer.cast::<libc::c_void>(), self.length()) };
        }
    }

    /// Sends one buffer request, which needs the struct itself rather than a byte buffer.
    fn buffer_request(
        fd: libc::c_int,
        request: libc::c_ulong,
        buffer: &mut Buffer,
    ) -> Result<(), std::io::Error> {
        let result = unsafe { libc::ioctl(fd, request, buffer as *mut Buffer) };
        if result < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// Asks for `count` buffers in the mmap memory model.
    fn request_buffers(fd: libc::c_int, count: u32) -> Result<(), String> {
        let mut buffer = [0_u8; REQUESTBUFFERS_LEN];
        put_u32(&mut buffer, 0, count);
        put_u32(&mut buffer, 4, BUF_TYPE_VIDEO_CAPTURE);
        put_u32(&mut buffer, 8, MEMORY_MMAP);
        ioctl(fd, REQBUFS, &mut buffer)
    }

    /// Asks the driver for its buffers and maps them.
    ///
    /// A driver may grant fewer than were asked for and stops answering `QUERYBUF` after the last
    /// one, so a later request that fails ends the walk rather than the whole open.
    fn map_buffers(fd: libc::c_int) -> Result<Vec<Mapping>, String> {
        request_buffers(fd, BUFFERS)
            .map_err(|error| format!("no buffers were granted: {error}"))?;
        let mut mapped = Vec::new();
        let mut index = 0;
        loop {
            let mut buffer = Buffer::at(index);
            match buffer_request(fd, QUERYBUF, &mut buffer) {
                Ok(()) => {}
                Err(_) if index > 0 => break,
                Err(error) => return Err(error.to_string()),
            }
            if buffer.length == 0 {
                return Err("the camera granted an empty buffer".to_owned());
            }
            mapped.push(
                Mapping::map(fd, buffer)
                    .map_err(|error| format!("a buffer could not be mapped: {error}"))?,
            );
            index += 1;
        }
        Ok(mapped)
    }

    /// An open camera, streamed through the driver's own buffer queue.
    ///
    /// Streamed with `mmap` rather than read: a plain read needs the driver to advertise
    /// `V4L2_CAP_READWRITE`, which the camera drivers this app has met do not, so a direct path
    /// built on it would be unavailable on the very machine it was written for. A stop is honoured
    /// by polling the descriptor with a short timeout before each dequeue, instead of waiting for a
    /// frame that may never come, which is the bug a plain blocking read had.
    pub struct Node {
        fd: OwnedFd,
        size: (usize, usize),
        stride: usize,
        buffers: Vec<Mapping>,
        streaming: bool,
    }

    impl Node {
        pub fn open(device: &str, budget: (usize, usize)) -> Result<Self, String> {
            let fd = open(device).map_err(|error| error.to_string())?;
            let raw = fd.as_raw_fd();
            let (_, capabilities) = capability(raw)?;
            if capabilities & CAP_VIDEO_CAPTURE == 0 {
                return Err("the node does not capture video".to_owned());
            }
            if !readable_directly(&pixel_formats(raw)) {
                return Err("the node offers no format this can convert".to_owned());
            }
            // Ask for the budget and take what the driver grants: encoding at the granted size
            // needs no scaler, so a camera that gives a smaller frame is used at that size.
            let (width, height, stride) = set_format(raw, FMT_YUYV, budget.0, budget.1)?;
            if width == 0 || height == 0 {
                return Err("the camera reported an empty frame".to_owned());
            }
            set_rate(raw);
            // YUYV packs two pixels into four bytes, so a row is at least `width * 2`.
            let stride = stride.max(width * 2);
            let buffers = map_buffers(raw)?;
            let mut node = Self {
                fd,
                size: (width, height),
                stride,
                buffers,
                streaming: false,
            };
            // Every buffer is handed to the driver and the stream is started, so frames begin to
            // arrive without the caller doing anything else.
            for index in 0..node.buffers.len() as u32 {
                node.queue(index)
                    .map_err(|error| format!("a buffer could not be queued: {error}"))?;
            }
            node.stream(true)
                .map_err(|error| format!("the stream could not be started: {error}"))?;
            Ok(node)
        }

        pub fn size(&self) -> (usize, usize) {
            self.size
        }

        /// Reads the next frame into `out`, which must be `width * height * 3 / 2` bytes.
        pub fn read(&mut self, out: &mut [u8]) -> Read {
            let (width, height) = self.size;
            debug_assert_eq!(
                out.len(),
                width * height * 3 / 2,
                "the encoder's frame size"
            );
            let mut descriptor = libc::pollfd {
                fd: self.fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut descriptor, 1, POLL) };
            if ready == 0 {
                return Read::Idle;
            }
            if ready < 0 || descriptor.revents & (libc::POLLIN | libc::POLLPRI) == 0 {
                // An unplugged camera reports an error on the descriptor, which is a camera that has
                // gone rather than one to keep polling.
                return Read::Ended;
            }
            let (index, used) = match self.dequeue() {
                Ok(Some(filled)) => filled,
                // The poll said a frame was ready and the dequeue disagreed: try the next poll
                // rather than treating a race as a camera that has gone.
                Ok(None) => return Read::Idle,
                Err(error) => {
                    log::warn!("[CALL] the camera stopped delivering frames: {error}");
                    return Read::Ended;
                }
            };
            let Some(mapping) = self.buffers.get(index as usize) else {
                log::warn!("[CALL] the camera handed back a buffer it never mapped");
                return Read::Ended;
            };
            let (pointer, length) = (mapping.pointer(), mapping.length());
            // Safety: the pointer is a mapping the kernel filled, whose length is the mapping's
            // own, so the slice stays inside memory this process owns. The frame is converted
            // before the buffer is handed back, so nothing else writes to it meanwhile.
            let source =
                unsafe { std::slice::from_raw_parts(pointer, (used as usize).min(length)) };
            to_yuv420(source, self.stride, width, height, out);
            if let Err(error) = self.queue(index) {
                log::warn!("[CALL] the camera frame could not be handed back: {error}");
                return Read::Ended;
            }
            Read::Frame
        }

        /// Hands one buffer back to the driver to be filled again, as the driver described it.
        fn queue(&mut self, index: u32) -> Result<(), String> {
            let fd = self.fd.as_raw_fd();
            match self.buffers.get_mut(index as usize) {
                Some(mapping) => buffer_request(fd, QBUF, mapping.prepare_for_queue())
                    .map_err(|error| error.to_string()),
                None => Err(format!("buffer {index} was never mapped")),
            }
        }

        /// Takes one filled buffer back, with its index and how many bytes it holds.
        fn dequeue(&mut self) -> Result<Option<(u32, u32)>, String> {
            let mut buffer = Buffer::at(0);
            match buffer_request(self.fd.as_raw_fd(), DQBUF, &mut buffer) {
                Ok(()) => Ok(Some((buffer.index, buffer.bytesused))),
                // The descriptor is non-blocking, so a frame that is not quite ready is not a
                // failure: the next poll has it.
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
                Err(error) => Err(error.to_string()),
            }
        }

        /// Starts or stops the stream. Stopping is what tells the driver its buffers are no longer
        /// being read, so the device is not left streaming after the call has ended.
        fn stream(&mut self, on: bool) -> Result<(), String> {
            let mut kind = BUF_TYPE_VIDEO_CAPTURE.to_ne_bytes();
            let request = if on { STREAMON } else { STREAMOFF };
            ioctl(self.fd.as_raw_fd(), request, &mut kind)?;
            self.streaming = on;
            Ok(())
        }

        /// Ends the stream, if it was started. The buffers are unmapped when this node drops.
        pub fn stop(&mut self) {
            if self.streaming {
                let _ = self.stream(false);
            }
        }
    }

    impl Drop for Node {
        fn drop(&mut self) {
            self.stop();
        }
    }

    /// Converts one packed YUYV frame into the tightly packed YUV 4:2:0 the encoder takes.
    ///
    /// A YUYV row is `Y0 U Y1 V` pairs, two pixels to four bytes, so luma is copied row by row from
    /// the driver's stride and each output chroma sample is the average of the two pairs covering
    /// its 2x2 block, which keeps colour aligned with the luma it belongs to. Rows the driver did
    /// not fill are left as they are rather than read past the end of the buffer.
    pub(super) fn to_yuv420(
        source: &[u8],
        stride: usize,
        width: usize,
        height: usize,
        out: &mut [u8],
    ) {
        let (y_len, chroma_len) = (width * height, width * height / 4);
        let (luma, chroma) = out.split_at_mut(y_len);
        let (u_plane, v_plane) = chroma.split_at_mut(chroma_len);
        let rows = height.min(source.len() / stride.max(1));
        for row in 0..rows {
            let from = row * stride;
            let to = row * width;
            for column in 0..width {
                luma[to + column] = source[from + column * 2];
            }
        }
        let pairs = width / 2;
        for row in 0..rows / 2 {
            let top = row * 2 * stride;
            let bottom = top + stride;
            let to = row * pairs;
            for pair in 0..pairs {
                let at = pair * 4;
                let u_top = u16::from(source[top + at + 1]);
                let u_bottom = u16::from(source[bottom + at + 1]);
                let v_top = u16::from(source[top + at + 3]);
                let v_bottom = u16::from(source[bottom + at + 3]);
                u_plane[to + pair] = ((u_top + u_bottom) / 2) as u8;
                v_plane[to + pair] = ((v_top + v_bottom) / 2) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_three_bytes_for_every_two_pixels() {
        // The contract the encoder relies on, at each size a call can ask for.
        for (width, height) in [(640, 360), (360, 640), (320, 180)] {
            assert_eq!(
                width * height * 3 / 2,
                width * height + width * height / 4 * 2
            );
        }
    }

    /// The choice between the two capture paths, decided from the driver's formats alone.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_packed_format_is_read_directly_and_anything_else_goes_to_the_fallback() {
        use super::v4l2::{FMT_YUYV, readable_directly};

        // MJPG, the compressed format a camera with no packed mode offers.
        const FMT_MJPG: u32 = 0x4750_4A4D;
        assert!(readable_directly(&[FMT_MJPG, FMT_YUYV]));
        // Nothing this module converts, so the fallback reads it instead of a buffer whose layout
        // would be wrong.
        assert!(!readable_directly(&[FMT_MJPG]));
        assert!(!readable_directly(&[]));
    }

    /// The packed frame becomes the three planes the encoder reads, at the right offsets.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_packed_frame_becomes_the_planes_the_encoder_takes() {
        // Two rows of two YUYV pairs, `Y0 U Y1 V` twice per row.
        let source = [
            10, 100, 20, 110, 30, 120, 40, 130, // first row
            50, 200, 60, 210, 70, 220, 80, 230, // second row
        ];
        let mut out = vec![0_u8; 4 * 2 + 4 * 2 / 4 * 2];
        super::v4l2::to_yuv420(&source, 8, 4, 2, &mut out);
        // Luma is every other byte, row by row.
        assert_eq!(&out[..8], &[10, 20, 30, 40, 50, 60, 70, 80]);
        // Each chroma sample averages the pair above and below it, so colour stays with its luma.
        assert_eq!(&out[8..10], &[150, 170]);
        assert_eq!(&out[10..12], &[160, 180]);
    }

    /// A frame shorter than the driver promised is converted row by row as far as it goes rather
    /// than read past the end of the mapping.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_short_frame_is_converted_as_far_as_it_goes() {
        // One complete row of a two-row frame: that row arrives and the other stays as it was.
        let source = [10, 100, 20, 110, 30, 120, 40, 130];
        let mut out = vec![0_u8; 4 * 2 + 4 * 2 / 4 * 2];
        super::v4l2::to_yuv420(&source, 8, 4, 2, &mut out);
        assert_eq!(&out[..8], &[10, 20, 30, 40, 0, 0, 0, 0]);
        // A fragment with no complete row in it converts nothing at all.
        let mut nothing = vec![9_u8; 4 * 2 + 4 * 2 / 4 * 2];
        super::v4l2::to_yuv420(&[10, 100, 20, 110], 8, 4, 2, &mut nothing);
        assert!(nothing.iter().all(|byte| *byte == 9));
    }

    /// The ioctl numbers are built from these sizes, so a layout that drifts would ask the kernel
    /// for a struct it does not have.
    #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
    #[test]
    fn the_kernel_layouts_are_reproduced() {
        // `v4l2_buffer`, with its eight-byte union, and `v4l2_format`, whose union is pushed four
        // bytes along by the alignment its pointer members need.
        assert_eq!(std::mem::size_of::<super::v4l2::Buffer>(), 88);
        assert_eq!(std::mem::size_of::<super::v4l2::Format>(), 208);
        // Where the pixel format arm starts, which the driver reads a width from.
        assert_eq!(
            super::v4l2::PIXEL_ARM,
            8,
            "the union starts past four bytes of padding"
        );
    }

    /// A node that is not a camera is refused instead of a capture process being started for it.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_node_that_is_not_a_camera_is_refused_rather_than_read() {
        let child = Arc::new(Mutex::new(None));
        let error = Source::open("/dev/null", (640, 360), child)
            .err()
            .expect("a device that is not a camera does not open");
        assert!(error.contains("not a camera"), "{error}");
    }

    #[test]
    fn a_platform_without_a_backend_reports_no_camera() {
        if available() {
            return;
        }
        assert!(cameras().is_empty());
        assert!(!is_capture("/dev/video0"));
        assert!(current_size("/dev/video0").is_none());
        assert!(Source::open("/dev/video0", (640, 360), Arc::new(Mutex::new(None))).is_err());
    }

    /// Opens this machine's real camera and counts frames.
    /// `cargo test --lib -- --ignored --nocapture camera::tests::hardware`
    #[test]
    #[ignore = "opens this machine's real camera"]
    fn hardware_reads_frames_from_the_camera() {
        let listed = cameras();
        eprintln!("cameras: {listed:#?}");
        assert!(
            !listed.is_empty(),
            "no camera was found to open on this machine"
        );
        let (device, label) = &listed[0];
        let child = Arc::new(Mutex::new(None));
        let mut source = Source::open(device, (640, 360), child).expect("the camera opens");
        // The node is the main path and `ffmpeg` only fills in, so a camera this readable must not
        // have started a capture process at all.
        assert!(
            matches!(source, Source::Node(_)),
            "the camera is read through V4L2 rather than the fallback"
        );
        let (width, height) = source.size();
        eprintln!("{label}: {width}x{height}");
        let mut frame = vec![0_u8; width * height * 3 / 2];
        let started = std::time::Instant::now();
        let mut frames = 0;
        while started.elapsed() < std::time::Duration::from_secs(2) {
            match source.read(&mut frame) {
                Read::Frame => {
                    frames += 1;
                    // A picture arrives, not a wall of one value.
                    let first = frame[0];
                    assert!(
                        frame[..width * height].iter().any(|byte| *byte != first),
                        "the frame carries a picture rather than one flat value"
                    );
                }
                Read::Idle => std::thread::sleep(std::time::Duration::from_millis(5)),
                Read::Ended => break,
            }
        }
        source.stop();
        source.reap();
        eprintln!("{frames} frames in 2 s at {width}x{height}");
        assert!(frames > 3, "the camera delivered {frames} frames");
    }
}
