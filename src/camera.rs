//! Camera capture for video calls, on every platform: Video4Linux through
//! linuxvideo on Linux, AVFoundation on macOS and Media Foundation on Windows
//! through nokhwa.
//!
//! A camera hands over frames in its own format (YU12/I420, YUYV, NV12, MJPEG, RGB or
//! grey) and at its own size. [`Source`] asks for the mode that best fits the
//! call and turns each frame into tightly packed YUV 4:2:0 at a size within
//! the call's budget, which is what the H.264 encoder and the corner preview
//! both read. A camera is opened, read and dropped on one thread: the
//! platforms' capture objects are not all `Send`.

/// A pixel layout this module converts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pixel {
    I420,
    Nv12,
    Yuyv,
    Mjpeg,
    Rgb,
    Bgr,
    Gray,
}

/// One way a camera can deliver: a size, a rate and a layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub pixel: Pixel,
}

/// What one read did.
pub enum Read {
    /// A whole frame landed in the buffer, taken from the camera at this instant: when the driver
    /// handed it over, before any conversion, so the time a frame carries does not depend on how
    /// long its conversion took.
    Frame(std::time::Instant),
    /// The camera delivered something that is not a picture; read again.
    Idle,
    /// The camera is gone: unplugged, taken by another program, or refused.
    Ended,
}

/// The cameras this machine has, as `(id, name)`, the id being what
/// [`Source::open`] takes back.
pub fn cameras() -> Vec<(String, String)> {
    let mut cameras = backend::list();
    for (_, name) in &mut cameras {
        if name.trim().is_empty() {
            *name = "Camera".to_owned();
        } else {
            *name = name.trim().to_owned();
        }
    }
    // A camera listed twice under one name is one camera with a second node.
    let mut seen = std::collections::HashSet::new();
    cameras.retain(|(_, name)| seen.insert(name.clone()));
    cameras
}

/// One open camera, delivering YUV 4:2:0 frames of [`Source::size`].
pub struct Source {
    camera: backend::Camera,
    /// The size frames are delivered at, both sides even.
    size: (usize, usize),
}

impl Source {
    /// Opens the camera `id` (the first one when `None`) in the mode that
    /// best fits `budget` at `fps`.
    pub fn open(id: Option<&str>, budget: (usize, usize), fps: u32) -> Result<Self, String> {
        let id = match id {
            Some(id) => id.to_owned(),
            None => {
                cameras()
                    .into_iter()
                    .next()
                    .ok_or_else(|| "no camera is connected".to_owned())?
                    .0
            }
        };
        let camera = backend::Camera::open(&id, budget, fps)?;
        let mode = camera.mode();
        let size = fit_even((mode.width as usize, mode.height as usize), budget);
        log::info!(
            "[CALL] camera opened {}x{} {:?} at {} fps, delivering {}x{}",
            mode.width,
            mode.height,
            mode.pixel,
            mode.fps,
            size.0,
            size.1
        );
        Ok(Self { camera, size })
    }

    /// The largest size frames are delivered at, both sides even: the camera's own shape within
    /// the budget it was opened with.
    pub fn size(&self) -> (usize, usize) {
        self.size
    }

    /// The camera's own frame rate, as the driver granted it. A camera that would not say is
    /// taken to deliver 30 frames a second.
    pub fn fps(&self) -> u32 {
        match self.camera.mode().fps {
            0 => 30,
            fps => fps,
        }
    }

    /// Waits for the next frame and writes it into `out` as YUV 4:2:0 of `size`: the Y plane,
    /// then U, then V. `size` is [`Source::size`] or a smaller size of the same shape, which is how
    /// a call sends a smaller picture when its path carries less.
    pub fn read(&mut self, out: &mut [u8], size: (usize, usize)) -> Read {
        let mut taken = None;
        let converted = self.camera.read(|pixel, data, width, height, stride| {
            taken = Some(std::time::Instant::now());
            let picture = match pixel {
                Pixel::I420 => planar_to_i420(data, width, height, stride),
                _ => to_i420(pixel, data, width, height),
            };
            let Some(picture) = picture else {
                return false;
            };
            picture.scale_into(size, out);
            true
        });
        match converted {
            Ok(true) => Read::Frame(taken.unwrap_or_else(std::time::Instant::now)),
            Ok(false) => Read::Idle,
            Err(error) => {
                log::warn!("[CALL] the camera stopped delivering: {error}");
                Read::Ended
            }
        }
    }

    /// Waits for the next frame and lets it go without converting it: the frames a call does not
    /// send cost a dequeue, not a conversion and a scale of the camera's full picture.
    pub fn skip(&mut self) -> Read {
        let mut taken = None;
        match self
            .camera
            .read(|_, _, _, _, _| taken = Some(std::time::Instant::now()))
        {
            Ok(()) => Read::Frame(taken.unwrap_or_else(std::time::Instant::now)),
            Err(error) => {
                log::warn!("[CALL] the camera stopped delivering: {error}");
                Read::Ended
            }
        }
    }

    /// Stops the stream; the device is released when the source is dropped.
    pub fn stop(&mut self) {
        self.camera.stop();
    }
}

/// The mode a call asks for: the largest size that fits the budget either
/// way round at a usable rate, raw YUV before MJPEG at the same size since it
/// costs no decoding. A camera with only bigger pictures gives its smallest.
pub fn choose_mode(modes: &[Mode], budget: (usize, usize), fps: u32) -> Option<Mode> {
    let (long, short) = (budget.0.max(budget.1), budget.0.min(budget.1));
    let fits = |mode: &Mode| {
        let (width, height) = (mode.width as usize, mode.height as usize);
        width.max(height) <= long && width.min(height) <= short
    };
    let preference = |pixel: Pixel| match pixel {
        Pixel::I420 | Pixel::Nv12 => 3,
        Pixel::Yuyv => 2,
        Pixel::Mjpeg => 1,
        _ => 0,
    };
    let usable: Vec<&Mode> = modes
        .iter()
        .filter(|mode| mode.fps >= fps.min(15) && mode.width >= 2 && mode.height >= 2)
        .collect();
    let fitting: Vec<&Mode> = usable.iter().copied().filter(|mode| fits(mode)).collect();
    let pool = if fitting.is_empty() { usable } else { fitting };
    pool.into_iter()
        .max_by_key(|mode| {
            let pixels = u64::from(mode.width) * u64::from(mode.height);
            let size = if fits(mode) {
                pixels
            } else {
                u64::MAX - pixels
            };
            (size, mode.fps.min(fps), preference(mode.pixel))
        })
        .copied()
}

/// Scales `size` to the largest even size with the same shape that fits
/// `bounds` either way round, never upscaling.
pub fn fit_even(size: (usize, usize), bounds: (usize, usize)) -> (usize, usize) {
    let even = |value: f64| (((value.round() as usize) / 2) * 2).max(2);
    if size.0 == 0 || size.1 == 0 {
        return (even(bounds.0 as f64), even(bounds.1 as f64));
    }
    let (long, short) = (bounds.0.max(bounds.1), bounds.0.min(bounds.1));
    let (fit_w, fit_h) = if size.0 >= size.1 {
        (long, short)
    } else {
        (short, long)
    };
    let scale = (fit_w as f64 / size.0 as f64)
        .min(fit_h as f64 / size.1 as f64)
        .min(1.0);
    (even(size.0 as f64 * scale), even(size.1 as f64 * scale))
}

/// A YUV 4:2:0 picture with both sides even.
#[derive(Debug, PartialEq)]
pub struct I420 {
    pub width: usize,
    pub height: usize,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
}

impl I420 {
    /// Writes the picture at `size` into `out` (Y, U, V), nearest sample.
    pub fn scale_into(&self, size: (usize, usize), out: &mut [u8]) {
        let (width, height) = size;
        let (chroma_w, chroma_h) = (width / 2, height / 2);
        let (y_out, rest) = out.split_at_mut(width * height);
        let (u_out, v_out) = rest.split_at_mut(chroma_w * chroma_h);
        sample(&self.y, (self.width, self.height), y_out, (width, height));
        let source_chroma = (self.width / 2, self.height / 2);
        sample(&self.u, source_chroma, u_out, (chroma_w, chroma_h));
        sample(&self.v, source_chroma, v_out, (chroma_w, chroma_h));
    }
}

/// Copies one plane at another size, taking the nearest sample.
///
/// The source column of every output column is worked out once per plane rather than once per
/// pixel: a 1080p camera scaled for a call is a division per pixel otherwise, on the thread that
/// has to keep up with the camera.
fn sample(from: &[u8], from_size: (usize, usize), to: &mut [u8], to_size: (usize, usize)) {
    if from_size == to_size {
        let length = to.len().min(from.len());
        to[..length].copy_from_slice(&from[..length]);
        return;
    }
    if to_size.0 == 0 || to_size.1 == 0 {
        return;
    }
    let columns: Vec<usize> = (0..to_size.0)
        .map(|column| column * from_size.0 / to_size.0)
        .collect();
    for (row, line) in to.chunks_exact_mut(to_size.0).take(to_size.1).enumerate() {
        let start = row * from_size.1 / to_size.1 * from_size.0;
        match from.get(start..start + from_size.0) {
            Some(source) => {
                for (out, &column) in line.iter_mut().zip(&columns) {
                    *out = source.get(column).copied().unwrap_or(128);
                }
            }
            None => line.fill(128),
        }
    }
}

/// Converts one camera frame to YUV 4:2:0, dropping an odd last row or
/// column. YU12 requires even dimensions. `None` when the data does not hold a whole frame.
pub fn to_i420(pixel: Pixel, data: &[u8], width: usize, height: usize) -> Option<I420> {
    match pixel {
        Pixel::I420 => planar_to_i420(data, width, height, width),
        Pixel::Yuyv => yuyv_to_i420(data, width, height),
        Pixel::Nv12 => nv12_to_i420(data, width, height),
        Pixel::Mjpeg => {
            let picture =
                image::load_from_memory_with_format(data, image::ImageFormat::Jpeg).ok()?;
            let picture = picture.to_rgb8();
            let (width, height) = (picture.width() as usize, picture.height() as usize);
            rgb_to_i420(picture.as_raw(), width, height, false)
        }
        Pixel::Rgb => rgb_to_i420(data, width, height, false),
        Pixel::Bgr => rgb_to_i420(data, width, height, true),
        Pixel::Gray => {
            let (even_w, even_h) = (width & !1, height & !1);
            if even_w == 0 || even_h == 0 || data.len() < width * height {
                return None;
            }
            let mut y = Vec::with_capacity(even_w * even_h);
            for row in 0..even_h {
                y.extend_from_slice(&data[row * width..row * width + even_w]);
            }
            let chroma = vec![128; even_w * even_h / 4];
            Some(I420 {
                width: even_w,
                height: even_h,
                y,
                u: chroma.clone(),
                v: chroma,
            })
        }
    }
}

/// V4L2 YU12 stores Y, U, V in one buffer; chroma rows have half the luma stride.
fn planar_to_i420(data: &[u8], width: usize, height: usize, stride: usize) -> Option<I420> {
    if width < 2
        || height < 2
        || !width.is_multiple_of(2)
        || !height.is_multiple_of(2)
        || stride < width
        || !stride.is_multiple_of(2)
    {
        return None;
    }
    let y_len = stride.checked_mul(height)?;
    let chroma_stride = stride / 2;
    let chroma_len = chroma_stride.checked_mul(height / 2)?;
    let total = y_len.checked_add(chroma_len.checked_mul(2)?)?;
    let data = data.get(..total)?;
    let plane = |offset: usize, row_stride: usize, columns: usize, rows: usize| {
        let mut out = Vec::with_capacity(columns * rows);
        for row in 0..rows {
            let start = offset + row * row_stride;
            out.extend_from_slice(&data[start..start + columns]);
        }
        out
    };
    Some(I420 {
        width,
        height,
        y: plane(0, stride, width, height),
        u: plane(y_len, chroma_stride, width / 2, height / 2),
        v: plane(y_len + chroma_len, chroma_stride, width / 2, height / 2),
    })
}

fn yuyv_to_i420(data: &[u8], width: usize, height: usize) -> Option<I420> {
    let (even_w, even_h) = (width & !1, height & !1);
    if even_w == 0 || even_h == 0 || data.len() < width * height * 2 {
        return None;
    }
    let stride = width * 2;
    let mut y = Vec::with_capacity(even_w * even_h);
    let mut u = Vec::with_capacity(even_w * even_h / 4);
    let mut v = Vec::with_capacity(even_w * even_h / 4);
    for row in 0..even_h {
        let line = &data[row * stride..row * stride + even_w * 2];
        y.extend(line.iter().step_by(2));
        if row % 2 == 0 {
            let next = &data[(row + 1) * stride..(row + 1) * stride + even_w * 2];
            for (pair, below) in line.as_chunks::<4>().0.iter().zip(next.as_chunks::<4>().0) {
                u.push(((u16::from(pair[1]) + u16::from(below[1])) / 2) as u8);
                v.push(((u16::from(pair[3]) + u16::from(below[3])) / 2) as u8);
            }
        }
    }
    Some(I420 {
        width: even_w,
        height: even_h,
        y,
        u,
        v,
    })
}

fn nv12_to_i420(data: &[u8], width: usize, height: usize) -> Option<I420> {
    let (even_w, even_h) = (width & !1, height & !1);
    let chroma_rows = height.div_ceil(2);
    let chroma_stride = width.div_ceil(2) * 2;
    if even_w == 0 || even_h == 0 || data.len() < width * height + chroma_stride * chroma_rows {
        return None;
    }
    let mut y = Vec::with_capacity(even_w * even_h);
    for row in 0..even_h {
        y.extend_from_slice(&data[row * width..row * width + even_w]);
    }
    let chroma = &data[width * height..];
    let mut u = Vec::with_capacity(even_w * even_h / 4);
    let mut v = Vec::with_capacity(even_w * even_h / 4);
    for row in 0..even_h / 2 {
        let line = &chroma[row * chroma_stride..row * chroma_stride + even_w];
        for pair in line.as_chunks::<2>().0 {
            u.push(pair[0]);
            v.push(pair[1]);
        }
    }
    Some(I420 {
        width: even_w,
        height: even_h,
        y,
        u,
        v,
    })
}

/// BT.601 studio range, which is what H.264 decoders assume by default.
fn rgb_to_i420(data: &[u8], width: usize, height: usize, bgr: bool) -> Option<I420> {
    let (even_w, even_h) = (width & !1, height & !1);
    if even_w == 0 || even_h == 0 || data.len() < width * height * 3 {
        return None;
    }
    let pixel = |x: usize, y: usize| {
        let at = (y * width + x) * 3;
        let (r, g, b) = (
            i32::from(data[at]),
            i32::from(data[at + 1]),
            i32::from(data[at + 2]),
        );
        if bgr { (b, g, r) } else { (r, g, b) }
    };
    let luma = |(r, g, b): (i32, i32, i32)| ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
    let mut y = Vec::with_capacity(even_w * even_h);
    for row in 0..even_h {
        for column in 0..even_w {
            y.push(luma(pixel(column, row)).clamp(0, 255) as u8);
        }
    }
    let mut u = Vec::with_capacity(even_w * even_h / 4);
    let mut v = Vec::with_capacity(even_w * even_h / 4);
    for row in (0..even_h).step_by(2) {
        for column in (0..even_w).step_by(2) {
            let mut sum = (0, 0, 0);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (r, g, b) = pixel(column + dx, row + dy);
                sum = (sum.0 + r, sum.1 + g, sum.2 + b);
            }
            let (r, g, b) = (sum.0 / 4, sum.1 / 4, sum.2 / 4);
            u.push((((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128).clamp(0, 255) as u8);
            v.push((((112 * r - 94 * g - 18 * b + 128) >> 8) + 128).clamp(0, 255) as u8);
        }
    }
    Some(I420 {
        width: even_w,
        height: even_h,
        y,
        u,
        v,
    })
}

/// Video4Linux, read in Rust through linuxvideo.
#[cfg(target_os = "linux")]
mod backend {
    use super::{Mode, Pixel};
    use linuxvideo::format::{FrameIntervals, FrameSizes, PixFormat, PixelFormat};
    use linuxvideo::{BufType, CapabilityFlags, Device, Fract};

    fn pixel(format: PixelFormat) -> Option<Pixel> {
        match &format.as_fourcc() {
            b"YU12" => Some(Pixel::I420),
            b"NV12" => Some(Pixel::Nv12),
            b"YUYV" => Some(Pixel::Yuyv),
            b"MJPG" | b"JPEG" => Some(Pixel::Mjpeg),
            b"RGB3" => Some(Pixel::Rgb),
            b"BGR3" => Some(Pixel::Bgr),
            b"GREY" => Some(Pixel::Gray),
            _ => None,
        }
    }

    fn fourcc(pixel: Pixel) -> PixelFormat {
        PixelFormat::from_fourcc(*match pixel {
            Pixel::I420 => b"YU12",
            Pixel::Nv12 => b"NV12",
            Pixel::Yuyv => b"YUYV",
            Pixel::Mjpeg => b"MJPG",
            Pixel::Rgb => b"RGB3",
            Pixel::Bgr => b"BGR3",
            Pixel::Gray => b"GREY",
        })
    }

    /// Frames a second from a frame interval.
    fn rate(interval: &Fract) -> u32 {
        if interval.numerator() == 0 {
            0
        } else {
            interval.denominator() / interval.numerator()
        }
    }

    /// The video capture nodes, named by their card. A camera's metadata node
    /// does not capture video and is left out.
    pub fn list() -> Vec<(String, String)> {
        let Ok(devices) = linuxvideo::list() else {
            return Vec::new();
        };
        let mut found: Vec<(String, String)> = devices
            .flatten()
            .filter_map(|device| {
                let capabilities = device.capabilities().ok()?;
                if !capabilities
                    .device_capabilities()
                    .contains(CapabilityFlags::VIDEO_CAPTURE)
                {
                    return None;
                }
                let path = device.path().ok()?;
                Some((
                    path.to_string_lossy().into_owned(),
                    capabilities.card().to_owned(),
                ))
            })
            .collect();
        found.sort();
        found
    }

    /// Every mode the device lists in a layout this module reads.
    fn modes(device: &Device, budget: (usize, usize)) -> Vec<Mode> {
        let mut modes = Vec::new();
        for description in device.formats(BufType::VIDEO_CAPTURE).flatten() {
            let format = description.pixel_format();
            let Some(pixel) = pixel(format) else {
                continue;
            };
            let sizes: Vec<(u32, u32)> = match device.frame_sizes(format) {
                Ok(FrameSizes::Discrete(sizes)) => sizes
                    .iter()
                    .map(|size| (size.width(), size.height()))
                    .collect(),
                // A camera that takes any size within a range is asked for
                // the budget, inside its range.
                Ok(range) => vec![(
                    (budget.0 as u32).clamp(range.min_width(), range.max_width()),
                    (budget.1 as u32).clamp(range.min_height(), range.max_height()),
                )],
                Err(_) => continue,
            };
            for (width, height) in sizes {
                let fps = match device.frame_intervals(format, width, height) {
                    Ok(FrameIntervals::Discrete(intervals)) => intervals
                        .iter()
                        .map(|interval| rate(interval.fract()))
                        .max()
                        .unwrap_or(0),
                    Ok(FrameIntervals::Stepwise(range) | FrameIntervals::Continuous(range)) => {
                        rate(range.min())
                    }
                    // A driver that will not say is taken at its word later.
                    Err(_) => 30,
                };
                modes.push(Mode {
                    width,
                    height,
                    fps,
                    pixel,
                });
            }
        }
        modes
    }

    pub struct Camera {
        stream: linuxvideo::stream::ReadStream,
        stride: usize,
        mode: Mode,
    }

    impl Camera {
        pub fn open(id: &str, budget: (usize, usize), fps: u32) -> Result<Self, String> {
            let device = Device::open(id)
                .map_err(|error| format!("the camera could not be opened: {error}"))?;
            let chosen = super::choose_mode(&modes(&device, budget), budget, fps)
                .ok_or_else(|| "the camera offers no picture format ZapFast reads".to_owned())?;
            let capture = device
                .video_capture(PixFormat::new(
                    chosen.width,
                    chosen.height,
                    fourcc(chosen.pixel),
                ))
                .map_err(|error| format!("the camera refused the format: {error}"))?;
            // The driver may have adjusted what was asked for.
            let granted = capture.format();
            let stride = granted.bytes_per_line() as usize;
            let mode = Mode {
                width: granted.width(),
                height: granted.height(),
                fps: chosen.fps,
                pixel: pixel(granted.pixel_format())
                    .ok_or_else(|| "the camera chose a format ZapFast does not read".to_owned())?,
            };
            let fps = match capture.set_frame_interval(Fract::new(1, fps.max(1))) {
                Ok(interval) => rate(&interval),
                Err(_) => mode.fps,
            };
            let stream = capture
                .into_stream()
                .map_err(|error| format!("the camera could not start: {error}"))?;
            Ok(Self {
                stream,
                stride,
                mode: Mode { fps, ..mode },
            })
        }

        pub fn mode(&self) -> Mode {
            self.mode
        }

        /// Waits for a frame and hands it to `convert`.
        pub fn read<R>(
            &mut self,
            convert: impl FnOnce(Pixel, &[u8], usize, usize, usize) -> R,
        ) -> Result<R, String> {
            let mode = self.mode;
            let mut convert = Some(convert);
            loop {
                let result = self
                    .stream
                    .dequeue(|view| {
                        if view.is_error() {
                            return Ok(None);
                        }
                        let convert = convert.take().expect("converted once");
                        Ok(Some(convert(
                            mode.pixel,
                            &view,
                            mode.width as usize,
                            mode.height as usize,
                            self.stride,
                        )))
                    })
                    .map_err(|error| error.to_string())?;
                if let Some(result) = result {
                    return Ok(result);
                }
            }
        }

        /// Linux stops the stream when the source is dropped.
        pub fn stop(&mut self) {}
    }
    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn yu12_is_a_readable_capture_format() {
            let format = PixelFormat::from_fourcc(*b"YU12");
            assert_eq!(pixel(format), Some(Pixel::I420));
            assert_eq!(fourcc(Pixel::I420), format);
        }
    }
}

/// AVFoundation on macOS and Media Foundation on Windows, through nokhwa.
#[cfg(not(target_os = "linux"))]
mod backend {
    use super::{Mode, Pixel};
    use nokhwa::utils::{
        ApiBackend, CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType,
        Resolution,
    };

    /// The layouts this module reads, as nokhwa names them.
    static READABLE: [FrameFormat; 6] = [
        FrameFormat::NV12,
        FrameFormat::YUYV,
        FrameFormat::MJPEG,
        FrameFormat::RAWRGB,
        FrameFormat::RAWBGR,
        FrameFormat::GRAY,
    ];

    /// How long a person is given to answer macOS's camera permission prompt.
    #[cfg(target_os = "macos")]
    const PERMISSION_WAIT: std::time::Duration = std::time::Duration::from_secs(60);

    fn pixel(format: FrameFormat) -> Pixel {
        match format {
            FrameFormat::NV12 => Pixel::Nv12,
            FrameFormat::YUYV => Pixel::Yuyv,
            FrameFormat::MJPEG => Pixel::Mjpeg,
            FrameFormat::RAWRGB => Pixel::Rgb,
            FrameFormat::RAWBGR => Pixel::Bgr,
            FrameFormat::GRAY => Pixel::Gray,
        }
    }

    fn frame_format(pixel: Pixel) -> &'static FrameFormat {
        let at = match pixel {
            Pixel::I420 => unreachable!("YU12 is only enumerated by the Linux backend"),
            Pixel::Nv12 => 0,
            Pixel::Yuyv => 1,
            Pixel::Mjpeg => 2,
            Pixel::Rgb => 3,
            Pixel::Bgr => 4,
            Pixel::Gray => 5,
        };
        &READABLE[at]
    }

    pub fn list() -> Vec<(String, String)> {
        match nokhwa::query(ApiBackend::Auto) {
            Ok(found) => found
                .into_iter()
                .map(|info| (info.index().to_string(), info.human_name()))
                .collect(),
            Err(error) => {
                log::warn!("[CALL] the cameras could not be listed: {error}");
                Vec::new()
            }
        }
    }

    /// Asks macOS for the camera, once, and waits for the answer.
    #[cfg(target_os = "macos")]
    fn authorize() -> Result<(), String> {
        if nokhwa::nokhwa_check() {
            return Ok(());
        }
        let (answer, answered) = std::sync::mpsc::channel();
        let answer = std::sync::Mutex::new(answer);
        nokhwa::nokhwa_initialize(move |granted| {
            if let Ok(answer) = answer.lock() {
                let _ = answer.send(granted);
            }
        });
        match answered.recv_timeout(PERMISSION_WAIT) {
            Ok(true) => Ok(()),
            Ok(false) => Err("macOS did not allow ZapFast to use the camera".to_owned()),
            Err(_) => Err("macOS did not answer whether the camera may be used".to_owned()),
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn authorize() -> Result<(), String> {
        Ok(())
    }

    pub struct Camera {
        camera: nokhwa::Camera,
        mode: Mode,
    }

    impl Camera {
        pub fn open(id: &str, budget: (usize, usize), fps: u32) -> Result<Self, String> {
            authorize()?;
            // The id is what `list` wrote: a number for an index, else the
            // platform's own identifier.
            let index = match id.parse::<u32>() {
                Ok(number) => CameraIndex::Index(number),
                Err(_) => CameraIndex::String(id.to_owned()),
            };
            let any = RequestedFormat::with_formats(RequestedFormatType::None, &READABLE);
            let mut camera = nokhwa::Camera::new(index, any)
                .map_err(|error| format!("the camera could not be opened: {error}"))?;
            let modes: Vec<Mode> = camera
                .compatible_camera_formats()
                .map_err(|error| format!("the camera's formats could not be read: {error}"))?
                .into_iter()
                .map(|format| Mode {
                    width: format.resolution().width(),
                    height: format.resolution().height(),
                    fps: format.frame_rate(),
                    pixel: pixel(format.format()),
                })
                .collect();
            if let Some(chosen) = super::choose_mode(&modes, budget, fps) {
                let format = CameraFormat::new(
                    Resolution::new(chosen.width, chosen.height),
                    *frame_format(chosen.pixel),
                    chosen.fps,
                );
                let exact = RequestedFormat::with_formats(
                    RequestedFormatType::Exact(format),
                    std::slice::from_ref(frame_format(chosen.pixel)),
                );
                if let Err(error) = camera.set_camera_requset(exact) {
                    log::warn!("[CALL] the camera kept its own format: {error}");
                }
            }
            camera
                .open_stream()
                .map_err(|error| format!("the camera could not start: {error}"))?;
            let format = camera.camera_format();
            let mode = Mode {
                width: format.resolution().width(),
                height: format.resolution().height(),
                fps: format.frame_rate(),
                pixel: pixel(format.format()),
            };
            Ok(Self { camera, mode })
        }

        pub fn mode(&self) -> Mode {
            self.mode
        }

        /// Waits for a frame and hands it to `convert`.
        pub fn read<R>(
            &mut self,
            convert: impl FnOnce(Pixel, &[u8], usize, usize, usize) -> R,
        ) -> Result<R, String> {
            let frame = self.camera.frame().map_err(|error| error.to_string())?;
            let resolution = frame.resolution();
            Ok(convert(
                pixel(frame.source_frame_format()),
                frame.buffer(),
                resolution.width() as usize,
                resolution.height() as usize,
                resolution.width() as usize,
            ))
        }

        pub fn stop(&mut self) {
            if let Err(error) = self.camera.stop_stream() {
                log::debug!("[CALL] the camera stream did not stop cleanly: {error}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yu12_keeps_luma_and_separate_chroma_planes() {
        let data = [1, 2, 3, 4, 5, 6, 7, 8, 40, 50, 180, 200];
        let picture = to_i420(Pixel::I420, &data, 4, 2).unwrap();
        assert_eq!(picture.y, data[..8]);
        assert_eq!(picture.u, [40, 50]);
        assert_eq!(picture.v, [180, 200]);
        let mut out = [0; 12];
        picture.scale_into((4, 2), &mut out);
        assert_eq!(out, data);
    }

    #[test]
    fn yu12_skips_padding_in_each_plane() {
        let data = [1, 2, 99, 99, 3, 4, 99, 99, 50, 99, 200, 99];
        let picture = planar_to_i420(&data, 2, 2, 4).unwrap();
        assert_eq!(picture.y, [1, 2, 3, 4]);
        assert_eq!(picture.u, [50]);
        assert_eq!(picture.v, [200]);
    }

    #[test]
    fn yu12_preserves_multiple_chroma_rows_with_padding() {
        let mut data = Vec::new();
        for row in 0..4u8 {
            data.extend([row; 4]);
            data.extend([99; 4]);
        }
        for row in [[40, 41], [42, 43], [180, 181], [182, 183]] {
            data.extend(row);
            data.extend([99; 2]);
        }
        let picture = planar_to_i420(&data, 4, 4, 8).unwrap();
        assert_eq!(picture.y, [0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3]);
        assert_eq!(picture.u, [40, 41, 42, 43]);
        assert_eq!(picture.v, [180, 181, 182, 183]);
    }

    #[test]
    fn yu12_refuses_short_frames_and_invalid_layouts() {
        for length in 0..12 {
            assert!(planar_to_i420(&[0; 12][..length], 2, 2, 4).is_none());
        }
        for (width, height, stride) in [
            (0, 2, 2),
            (2, 0, 2),
            (3, 2, 4),
            (2, 3, 2),
            (4, 2, 2),
            (2, 2, 3),
            (2, 2, usize::MAX - 1),
        ] {
            assert!(planar_to_i420(&[0; 32], width, height, stride).is_none());
        }
    }

    #[test]
    fn an_iriun_only_mode_is_usable_without_a_format_fallback() {
        let mode = Mode {
            width: 1280,
            height: 720,
            fps: 30,
            pixel: Pixel::I420,
        };
        assert_eq!(choose_mode(&[mode], (1280, 720), 25), Some(mode));
    }

    #[test]
    #[ignore = "opens the real camera; run manually with Iriun connected"]
    fn hardware_yu12_capture_encodes_and_decodes() {
        use openh264::formats::YUVSource as _;
        let mut source = Source::open(None, (1280, 720), 25).expect("camera opens");
        assert_eq!(
            source.camera.mode().pixel,
            Pixel::I420,
            "select a YU12 camera"
        );
        let (width, height) = source.size();
        let mut frame = vec![0; width * height * 3 / 2];
        let mut encoder = openh264::encoder::Encoder::new().unwrap();
        let mut decoder = openh264::decoder::Decoder::new().unwrap();
        for _ in 0..3 {
            assert!(matches!(
                source.read(&mut frame, (width, height)),
                Read::Frame(_)
            ));
            let (y, chroma) = frame.split_at(width * height);
            let (u, v) = chroma.split_at(width * height / 4);
            let picture = openh264::formats::YUVSlices::new(
                (y, u, v),
                (width, height),
                (width, width / 2, width / 2),
            );
            let unit = encoder.encode(&picture).expect("frame encodes").to_vec();
            assert!(!unit.is_empty());
            let decoded = decoder
                .decode(&unit)
                .expect("frame decodes")
                .expect("whole picture");
            assert_eq!(decoded.dimensions(), (width, height));
        }
        source.stop();
        eprintln!(
            "Captured, encoded and decoded 3 YU12 frames at {width}x{height}; no images saved"
        );
    }

    #[test]
    fn yuyv_keeps_every_luma_sample_and_averages_chroma_down() {
        // Two by two: Y0 U Y1 V on each row.
        let data = [10, 100, 20, 200, 30, 110, 40, 210];
        let picture = to_i420(Pixel::Yuyv, &data, 2, 2).unwrap();
        assert_eq!(picture.y, [10, 20, 30, 40]);
        assert_eq!(picture.u, [105]);
        assert_eq!(picture.v, [205]);
    }

    #[test]
    fn nv12_splits_its_interleaved_chroma() {
        let data = [1, 2, 3, 4, 50, 60];
        let picture = to_i420(Pixel::Nv12, &data, 2, 2).unwrap();
        assert_eq!(picture.y, [1, 2, 3, 4]);
        assert_eq!(picture.u, [50]);
        assert_eq!(picture.v, [60]);
    }

    #[test]
    fn white_and_black_land_at_the_ends_of_studio_range() {
        let white = to_i420(Pixel::Rgb, &[255; 12], 2, 2).unwrap();
        assert!(white.y.iter().all(|&y| y == 235), "{:?}", white.y);
        assert_eq!(white.u, [128]);
        let black = to_i420(Pixel::Bgr, &[0; 12], 2, 2).unwrap();
        assert!(black.y.iter().all(|&y| y == 16));
    }

    #[test]
    fn a_jpeg_frame_is_decoded() {
        let mut jpeg = Vec::new();
        image::RgbImage::from_pixel(4, 2, image::Rgb([200, 200, 200]))
            .write_to(
                &mut std::io::Cursor::new(&mut jpeg),
                image::ImageFormat::Jpeg,
            )
            .unwrap();
        let picture = to_i420(Pixel::Mjpeg, &jpeg, 4, 2).unwrap();
        assert_eq!((picture.width, picture.height), (4, 2));
        assert_eq!(picture.u.len(), 2);
    }

    #[test]
    fn an_odd_or_short_frame_is_cropped_or_refused() {
        let picture = to_i420(Pixel::Gray, &[7; 15], 5, 3).unwrap();
        assert_eq!((picture.width, picture.height), (4, 2));
        assert!(to_i420(Pixel::Yuyv, &[0; 4], 2, 2).is_none());
    }

    #[test]
    fn a_frame_is_scaled_into_the_delivered_size() {
        let picture = I420 {
            width: 4,
            height: 4,
            y: (0..16).collect(),
            u: vec![1, 2, 3, 4],
            v: vec![5, 6, 7, 8],
        };
        let mut out = vec![0; 2 * 2 + 2];
        picture.scale_into((2, 2), &mut out);
        assert_eq!(out, [0, 2, 8, 10, 1, 5]);
    }

    #[test]
    fn the_size_fits_the_budget_either_way_round_without_upscaling() {
        assert_eq!(fit_even((1920, 1080), (1280, 720)), (1280, 720));
        assert_eq!(fit_even((1080, 1920), (1280, 720)), (720, 1280));
        assert_eq!(fit_even((640, 480), (1280, 720)), (640, 480));
        assert_eq!(fit_even((641, 481), (1280, 720)), (640, 480));
    }

    #[test]
    fn a_call_asks_for_the_largest_mode_that_fits_at_a_usable_rate() {
        let mode = |width, height, pixel, fps| Mode {
            width,
            height,
            fps,
            pixel,
        };
        let modes = [
            mode(1920, 1080, Pixel::Mjpeg, 30),
            mode(1280, 720, Pixel::Mjpeg, 30),
            mode(1280, 720, Pixel::Yuyv, 10),
            mode(640, 480, Pixel::Yuyv, 30),
        ];
        let chosen = choose_mode(&modes, (1280, 720), 20).unwrap();
        assert_eq!((chosen.width, chosen.height), (1280, 720));
        assert_eq!(chosen.pixel, Pixel::Mjpeg, "YUYV at 10 fps is too slow");
        // Raw YUV wins a tie.
        let tie = [
            mode(640, 480, Pixel::Mjpeg, 30),
            mode(640, 480, Pixel::Yuyv, 30),
        ];
        assert_eq!(
            choose_mode(&tie, (1280, 720), 20).unwrap().pixel,
            Pixel::Yuyv
        );
        // A camera with only bigger pictures gives its smallest.
        let big = [
            mode(3840, 2160, Pixel::Mjpeg, 30),
            mode(1920, 1080, Pixel::Mjpeg, 30),
        ];
        assert_eq!(choose_mode(&big, (1280, 720), 20).unwrap().width, 1920);
        // A portrait camera fits the budget turned upright.
        let upright = [mode(720, 1280, Pixel::Nv12, 30)];
        assert_eq!(choose_mode(&upright, (1280, 720), 20).unwrap().height, 1280);
    }
}
