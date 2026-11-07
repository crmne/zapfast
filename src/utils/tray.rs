pub fn image_uri(path: &std::path::Path) -> String {
    image_uri_for_platform(&path.to_string_lossy(), cfg!(windows))
}

fn image_uri_for_platform(path: &str, windows: bool) -> String {
    format!("file://{}{path}", if windows { "/" } else { "" })
}

const MARK: &[u8] = include_bytes!("../../packaging/icons/zapfast.svg");
const SMALL_MARK: &[u8] = include_bytes!("../../packaging/icons/zapfast-small.svg");
const SMALL_BELOW: usize = 40;

pub fn app_icon_rgba(size: usize) -> Vec<u8> {
    let mark = if size < SMALL_BELOW { SMALL_MARK } else { MARK };
    match render_mark(mark, size) {
        Some(rgba) => rgba,
        None => plain_disc(size),
    }
}

fn render_mark(mark: &[u8], size: usize) -> Option<Vec<u8>> {
    let side = size.max(1) as u32;
    let tree = resvg::usvg::Tree::from_data(mark, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(side, side)?;
    let scale = side as f32 / tree.size().width();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Some(
        pixmap
            .pixels()
            .iter()
            .flat_map(|pixel| {
                let color = pixel.demultiply();
                [color.red(), color.green(), color.blue(), color.alpha()]
            })
            .collect(),
    )
}

fn plain_disc(size: usize) -> Vec<u8> {
    let mut rgba = vec![0u8; size * size * 4];
    let center = size as f32 / 2.0;
    let radius = center - 2.0;
    for y in 0..size {
        for x in 0..size {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let distance = ((px - center).powi(2) + (py - center).powi(2)).sqrt();
            let coverage = (radius - distance + 0.5).clamp(0.0, 1.0);
            let index = (y * size + x) * 4;
            rgba[index] = 0;
            rgba[index + 1] = 168;
            rgba[index + 2] = 132;
            rgba[index + 3] = (coverage * 255.0) as u8;
        }
    }
    rgba
}

pub fn tray_template_rgba(size: usize) -> Vec<u8> {
    let mut rgba = render_mark(SMALL_MARK, size).unwrap_or_else(|| plain_disc(size));
    const INK: f32 = 14.0;
    const DISC: f32 = 168.0;
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let disc = ((f32::from(pixel[1]) - INK) / (DISC - INK)).clamp(0.0, 1.0);
        pixel[3] = (f32::from(pixel[3]) * disc).round() as u8;
        pixel[0] = 0;
        pixel[1] = 0;
        pixel[2] = 0;
    }
    rgba
}

#[cfg(windows)]
pub fn windows_taskbar_dark() -> Option<bool> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let value = wide("SystemUsesLightTheme");
    let mut light = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            std::ptr::from_mut(&mut light).cast(),
            &mut size,
        )
    };
    (status == ERROR_SUCCESS).then_some(light == 0)
}

const TRAY_BADGE_MAX: u32 = 9;

pub type TrayIcon = fn(usize) -> Vec<u8>;

pub fn tray_icons(count: u32) -> (TrayIcon, TrayIcon) {
    fn pair<const COUNT: u32>() -> (TrayIcon, TrayIcon) {
        (
            |size| badged(app_icon_rgba(size), size, COUNT, false),
            |size| badged(tray_template_rgba(size), size, COUNT, true),
        )
    }
    match tray_badge_level(count) {
        0 => (app_icon_rgba, tray_template_rgba),
        1 => pair::<1>(),
        2 => pair::<2>(),
        3 => pair::<3>(),
        4 => pair::<4>(),
        5 => pair::<5>(),
        6 => pair::<6>(),
        7 => pair::<7>(),
        8 => pair::<8>(),
        9 => pair::<9>(),
        _ => pair::<{ TRAY_BADGE_MAX + 1 }>(),
    }
}

pub fn tray_badge_level(count: u32) -> u32 {
    count.min(TRAY_BADGE_MAX + 1)
}

const BADGE_DIGITS: [[u8; 5]; 9] = [
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b011, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b010, 0b010, 0b010],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];
const BADGE_PLUS: [u8; 5] = [0b000, 0b010, 0b111, 0b010, 0b000];

fn badged(mut rgba: Vec<u8>, size: usize, level: u32, template: bool) -> Vec<u8> {
    const RED: [u8; 3] = [0xe5, 0x39, 0x35];
    let radius = size as f32 * 0.3;
    let center = (size as f32 - radius, radius);
    let ring = size as f32 / 20.0;
    let more = level > TRAY_BADGE_MAX;
    let digit = BADGE_DIGITS[(level.clamp(1, TRAY_BADGE_MAX) - 1) as usize];
    let columns = if more { 7 } else { 3 };
    let cell = (size / 16).max(1);
    let left = (center.0 - (columns * cell) as f32 / 2.0).round() as isize;
    let top = (center.1 - (5 * cell) as f32 / 2.0).round() as isize;
    let inked = |x: usize, y: usize| {
        let (x, y) = (x as isize - left, y as isize - top);
        if x < 0 || y < 0 {
            return false;
        }
        let (column, row) = (x as usize / cell, y as usize / cell);
        if row >= 5 || column >= columns {
            return false;
        }
        match column {
            0..=2 => digit[row] >> (2 - column) & 1 == 1,
            3 => false,
            _ => BADGE_PLUS[row] >> (6 - column) & 1 == 1,
        }
    };
    for (index, pixel) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = (index % size, index / size);
        let distance =
            ((x as f32 + 0.5 - center.0).powi(2) + (y as f32 + 0.5 - center.1).powi(2)).sqrt();
        if distance >= radius + ring + 0.5 {
            continue;
        }
        let disc = (radius - distance + 0.5).clamp(0.0, 1.0);
        let colour = if template { [0, 0, 0] } else { RED };
        pixel[..3].copy_from_slice(&colour);
        pixel[3] = (disc * 255.0) as u8;
        if inked(x, y) {
            if template {
                pixel[3] = 0;
            } else {
                pixel[..4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
    }
    rgba
}
