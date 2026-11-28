//! Bounded, off-thread rendering of downloaded PDF attachments.

use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::thread;

use egui::ColorImage;
use hayro::hayro_interpret::font::{Glyph, GlyphRun};
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, DrawMode, DrawProps, Image, ImageDrawProps, InterpreterCache,
    InterpreterSettings, SoftMask, TransformExt, interpret_page,
};
use hayro::hayro_syntax::Pdf;
use hayro::kurbo::{Point, Rect as KurboRect};
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings, render};

pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_PAGES: usize = 2_000;
pub const MAX_OBJECTS: usize = 100_000;
pub const MAX_RENDER_SIDE: f32 = 4096.0;
pub const MAX_RENDER_PIXELS: f32 = 8_000_000.0;

const MAX_RENDER_SCALE: f32 = 4.0;
const MAX_PAGE_POINTS: f32 = 100_000.0;
const MAX_GLYPHS_PER_PAGE: usize = 4_000;
const MAX_GLYPHS_TOTAL: usize = 20_000;
const TEXT_BATCH: usize = 8;
const UNITS_PER_EM: f64 = 1000.0;
const GLYPH_TOP_UNITS: f64 = 820.0;
const GLYPH_BOTTOM_UNITS: f64 = -180.0;

/// One positioned Unicode run from a PDF, in render-space document points.
#[derive(Clone, Debug, PartialEq)]
pub struct TextGlyph {
    pub page: usize,
    pub order: usize,
    pub text: String,
    pub origin: (f32, f32),
    /// Unit vector along the text baseline in render-space document points.
    pub baseline: (f32, f32),
    pub rect: egui::Rect,
}

#[derive(Debug)]
pub enum Event {
    Ready(Vec<(f32, f32)>),
    Text {
        first_page: usize,
        pages: Vec<Vec<TextGlyph>>,
        complete: bool,
        partial: bool,
    },
    Rendered {
        request: u64,
        page: usize,
        image: ColorImage,
    },
    Failed {
        request: Option<u64>,
        message: &'static str,
    },
}

#[derive(Debug)]
pub struct Request {
    pub id: u64,
    pub page: usize,
    pub scale: f32,
}

pub struct Worker {
    requests: SyncSender<Request>,
    events: Receiver<Event>,
    cancelled: Arc<AtomicBool>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        // The thread is detached. Tell it to stop before its next expensive
        // phase so a closed preview does not keep a document alive.
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl Worker {
    pub fn spawn(path: &Path, ctx: egui::Context) -> Result<Self, std::io::Error> {
        let (request_tx, request_rx) = sync_channel(1);
        let (event_tx, event_rx) = sync_channel(1);
        let path = path.to_owned();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancelled);
        thread::Builder::new()
            .name("zapfast-pdf-preview".into())
            .spawn(move || run(path, request_rx, event_tx, ctx, worker_cancel))?;
        Ok(Self {
            requests: request_tx,
            events: event_rx,
            cancelled,
        })
    }

    pub fn request(&self, request: Request) -> bool {
        self.requests.try_send(request).is_ok()
    }

    pub fn next_event(&self) -> Result<Event, TryRecvError> {
        self.events.try_recv()
    }
}

fn stopped(cancelled: &AtomicBool) -> bool {
    cancelled.load(Ordering::Relaxed)
}

fn run(
    path: std::path::PathBuf,
    requests: Receiver<Request>,
    events: SyncSender<Event>,
    ctx: egui::Context,
    cancelled: Arc<AtomicBool>,
) {
    if stopped(&cancelled) {
        return;
    }
    let loaded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| load(&path, &cancelled)));
    let pdf = match loaded {
        Ok(Ok(pdf)) => pdf,
        Ok(Err(message)) => {
            let _ = events.send(Event::Failed {
                request: None,
                message,
            });
            ctx.request_repaint();
            return;
        }
        Err(_) => {
            let _ = events.send(Event::Failed {
                request: None,
                message: "This PDF could not be read.",
            });
            ctx.request_repaint();
            return;
        }
    };
    if stopped(&cancelled) {
        return;
    }

    let pages = pdf.pages();
    let dimensions: Vec<_> = pages.iter().map(|page| page.render_dimensions()).collect();
    if dimensions.is_empty() {
        let _ = events.send(Event::Failed {
            request: None,
            message: "This PDF has no pages.",
        });
        ctx.request_repaint();
        return;
    }
    if dimensions.len() > MAX_PAGES {
        let _ = events.send(Event::Failed {
            request: None,
            message: "This PDF has too many pages to preview.",
        });
        ctx.request_repaint();
        return;
    }
    if dimensions
        .iter()
        .any(|&(width, height)| !valid_dimensions(width, height))
    {
        let _ = events.send(Event::Failed {
            request: None,
            message: "This PDF has an invalid page size.",
        });
        ctx.request_repaint();
        return;
    }
    if events.send(Event::Ready(dimensions)).is_err() {
        return;
    }
    ctx.request_repaint();

    let interpreter = InterpreterSettings::default();
    let settings = RenderSettings::default();
    // Index in short runs. A visible page can render between them, so a long
    // appendix never holds the cover on "Rendering page". Search gets each
    // run as it arrives, with `complete` telling the viewer if it may finish.
    let mut first = 0usize;
    let mut remaining = MAX_GLYPHS_TOTAL;
    let mut partial = false;
    while first < pages.len() {
        if stopped(&cancelled) {
            return;
        }
        while let Ok(request) = requests.try_recv() {
            if !serve_request(
                request,
                pages,
                &interpreter,
                &settings,
                &events,
                &ctx,
                &cancelled,
            ) {
                return;
            }
        }
        let end = (first + TEXT_BATCH).min(pages.len());
        let mut batch = Vec::with_capacity(end - first);
        for page in first..end {
            if stopped(&cancelled) {
                return;
            }
            if remaining == 0 {
                partial = true;
                batch.push(Vec::new());
                continue;
            }
            let extracted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                extract_page(&pages[page], page, remaining)
            }))
            .unwrap_or_default();
            remaining = remaining.saturating_sub(extracted.len());
            batch.push(extracted);
        }
        let complete = end == pages.len();
        if events
            .send(Event::Text {
                first_page: first,
                pages: batch,
                complete,
                partial,
            })
            .is_err()
        {
            return;
        }
        ctx.request_repaint();
        first = end;
    }
    while let Ok(request) = requests.recv() {
        if !serve_request(
            request,
            pages,
            &interpreter,
            &settings,
            &events,
            &ctx,
            &cancelled,
        ) {
            return;
        }
    }
}

fn serve_request<'a>(
    request: Request,
    pages: &[hayro::hayro_syntax::page::Page<'a>],
    interpreter: &InterpreterSettings,
    settings: &RenderSettings,
    events: &SyncSender<Event>,
    ctx: &egui::Context,
    cancelled: &AtomicBool,
) -> bool {
    if stopped(cancelled) {
        return false;
    }
    if request.page >= pages.len() {
        return true;
    }
    let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        render_page(
            &pages[request.page],
            // Hayro cannot clear its cache. Don't retain resources from every page read.
            &RenderCache::new(),
            interpreter,
            settings,
            request.scale,
        )
    }));
    let outcome = match rendered {
        Ok(Some(image)) => events.send(Event::Rendered {
            request: request.id,
            page: request.page,
            image,
        }),
        Ok(None) | Err(_) => events.send(Event::Failed {
            request: Some(request.id),
            message: "This PDF page could not be rendered.",
        }),
    };
    if outcome.is_err() {
        return false;
    }
    ctx.request_repaint();
    true
}

fn load(path: &Path, cancelled: &AtomicBool) -> Result<Pdf, &'static str> {
    if stopped(cancelled) {
        return Err("This PDF is no longer available.");
    }
    let mut file = std::fs::File::open(path).map_err(|_| "This PDF is no longer available.")?;
    if file
        .metadata()
        .map_err(|_| "This PDF could not be read.")?
        .len()
        > MAX_FILE_BYTES
    {
        return Err("This PDF is too large to preview.");
    }
    let mut bytes = Vec::new();
    Read::take(&mut file, MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "This PDF could not be read.")?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("This PDF is too large to preview.");
    }
    if stopped(cancelled) {
        return Err("This PDF is no longer available.");
    }
    let pdf = Pdf::new(bytes).map_err(|_| "This file is not a readable PDF.")?;
    if pdf.len() > MAX_OBJECTS {
        return Err("This PDF is too complex to preview.");
    }
    Ok(pdf)
}

fn valid_dimensions(width: f32, height: f32) -> bool {
    width.is_finite()
        && height.is_finite()
        && width > 0.0
        && height > 0.0
        && width <= MAX_PAGE_POINTS
        && height <= MAX_PAGE_POINTS
}

pub fn render_scale(width: f32, height: f32, requested: f32) -> Option<f32> {
    render_scale_with_side(width, height, requested, MAX_RENDER_SIDE as usize)
}

/// The same bounds as `render_scale`, with the viewport's texture limit added.
/// egui debug-asserts that an uploaded texture fits this side on every axis.
pub fn render_scale_with_side(
    width: f32,
    height: f32,
    requested: f32,
    max_texture_side: usize,
) -> Option<f32> {
    if !valid_dimensions(width, height) || !requested.is_finite() || requested <= 0.0 {
        return None;
    }
    if max_texture_side == 0 {
        return None;
    }
    let area = f64::from(width) * f64::from(height);
    let area_scale = (f64::from(MAX_RENDER_PIXELS) / area).sqrt() as f32;
    let side = max_texture_side as f32;
    let scale = requested
        .min(MAX_RENDER_SCALE)
        .min(MAX_RENDER_SIDE / width.max(height))
        .min(area_scale)
        .min(side / width.max(height));
    (scale.is_finite() && scale > 0.0).then_some(scale)
}

fn render_page<'a>(
    page: &'a hayro::hayro_syntax::page::Page<'a>,
    cache: &RenderCache<'a>,
    interpreter: &InterpreterSettings,
    settings: &RenderSettings,
    requested_scale: f32,
) -> Option<ColorImage> {
    let (width, height) = page.render_dimensions();
    let scale = render_scale(width, height, requested_scale)?;
    let pixmap = render(
        page,
        cache,
        interpreter,
        settings,
        &PixmapSettings {
            x_scale: scale,
            y_scale: scale,
            bg_color: WHITE,
        },
    );
    let (width, height) = (usize::from(pixmap.width()), usize::from(pixmap.height()));
    if width == 0
        || height == 0
        || width.max(height) as f32 > MAX_RENDER_SIDE
        || (width * height) as f32 > MAX_RENDER_PIXELS
    {
        return None;
    }
    Some(ColorImage::from_rgba_premultiplied(
        [width, height],
        pixmap.data_as_u8_slice(),
    ))
}

/// Extracts Unicode runs in the same document coordinates the page is drawn
/// in. Text that cannot be mapped is skipped: a search must not claim a match
/// on a glyph it cannot name.
#[cfg(any(test, feature = "demo"))]
fn extract_text(pages: &[hayro::hayro_syntax::page::Page<'_>]) -> Vec<Vec<TextGlyph>> {
    let mut remaining = MAX_GLYPHS_TOTAL;
    pages
        .iter()
        .enumerate()
        .map(|(page, page_ref)| {
            let extracted = extract_page(page_ref, page, remaining);
            remaining = remaining.saturating_sub(extracted.len());
            extracted
        })
        .collect()
}

fn extract_page(
    page_ref: &hayro::hayro_syntax::page::Page<'_>,
    page: usize,
    limit: usize,
) -> Vec<TextGlyph> {
    if limit == 0 {
        return Vec::new();
    }
    let (width, height) = page_ref.render_dimensions();
    if !valid_dimensions(width, height) {
        return Vec::new();
    }
    let cache = InterpreterCache::new();
    let mut context = Context::new(
        page_ref.initial_transform(true).to_kurbo(),
        KurboRect::new(0.0, 0.0, f64::from(width), f64::from(height)),
        &cache,
        page_ref.xref(),
        InterpreterSettings::default(),
    );
    let mut extractor = TextExtractor {
        page,
        order: 0,
        glyphs: Vec::new(),
        limit: limit.min(MAX_GLYPHS_PER_PAGE),
    };
    interpret_page(page_ref, &mut context, &mut extractor);
    extractor.glyphs
}

struct TextExtractor {
    page: usize,
    order: usize,
    glyphs: Vec<TextGlyph>,
    limit: usize,
}

impl hayro::hayro_interpret::Device<'_> for TextExtractor {
    fn draw_path(&mut self, _: &hayro::kurbo::BezPath, _: DrawProps<'_>, _: &DrawMode) {}

    fn push_clip_path(&mut self, _: &ClipPath) {}

    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}

    fn draw_glyph_run(&mut self, glyph_run: &GlyphRun<'_, '_>, props: DrawProps<'_>, _: &DrawMode) {
        if self.glyphs.len() >= self.limit {
            return;
        }
        let glyphs = glyph_run.glyphs();
        for (index, glyph) in glyphs.iter().enumerate() {
            let text = match glyph.as_unicode() {
                Some(BfString::Char(c)) => c.to_string(),
                Some(BfString::String(text)) => text,
                None => continue,
            };
            // Hayro scales glyph transforms by 1 / UNITS_PER_EM (1000), so
            // the corners below are in the font's own coordinate space.
            let transform = props.transform * glyph.transform();
            let origin = transform * Point::ZERO;
            let [a, b, c, d, _, _] = transform.as_coeffs();
            let x_scale = a.hypot(b);
            let baseline = ((a / x_scale) as f32, (b / x_scale) as f32);
            let em_x = (x_scale * UNITS_PER_EM) as f32;
            let em_y = (c.hypot(d) * UNITS_PER_EM) as f32;
            let height = em_y.max(em_x * 0.25);
            if !height.is_finite() || height <= 0.0 || x_scale <= 0.0 {
                continue;
            }
            let font_advance = match &**glyph {
                Glyph::Outline(g) => g.advance_width(),
                Glyph::Type3(g) => g.advance_width(),
            }
            .map(f64::from)
            .filter(|units| units.is_finite() && *units > 0.0 && *units < UNITS_PER_EM * 4.0);
            let next_advance = glyphs
                .get(index + 1)
                .map(|next| {
                    ((props.transform * next.transform() * Point::ZERO) - origin).hypot() / x_scale
                })
                .filter(|units| units.is_finite() && *units > 0.0 && *units < UNITS_PER_EM * 1.8);
            let advance_units = font_advance
                .or(next_advance)
                .unwrap_or(UNITS_PER_EM * 0.55 * text.chars().count().max(1) as f64);
            let corners = [
                Point::new(0.0, GLYPH_BOTTOM_UNITS),
                Point::new(0.0, GLYPH_TOP_UNITS),
                Point::new(advance_units, GLYPH_TOP_UNITS),
                Point::new(advance_units, GLYPH_BOTTOM_UNITS),
            ];
            let mut min_x = f64::INFINITY;
            let mut min_y = f64::INFINITY;
            let mut max_x = f64::NEG_INFINITY;
            let mut max_y = f64::NEG_INFINITY;
            for corner in corners {
                let point = transform * corner;
                min_x = min_x.min(point.x);
                min_y = min_y.min(point.y);
                max_x = max_x.max(point.x);
                max_y = max_y.max(point.y);
            }
            if ![min_x, min_y, max_x, max_y]
                .iter()
                .all(|value| value.is_finite())
            {
                continue;
            }
            // Transforming every corner keeps the box over the painted letter
            // when the page turns the text 90 degrees.
            let rect = egui::Rect::from_min_max(
                egui::pos2(min_x as f32, min_y as f32),
                egui::pos2(max_x as f32, max_y as f32),
            );
            if let Some(prev) = self.glyphs.last()
                && prev.page == self.page
                && (origin.y as f32 - prev.origin.1).abs() < height * 0.4
            {
                let gap = origin.x as f32 - prev.rect.max.x;
                let needs_space = gap > height * 0.22
                    && gap < height * 4.0
                    && !prev.text.chars().last().is_some_and(char::is_whitespace)
                    && !text.chars().next().is_some_and(char::is_whitespace);
                if needs_space && self.glyphs.len() < self.limit {
                    self.glyphs.push(TextGlyph {
                        page: self.page,
                        order: self.order,
                        text: " ".into(),
                        origin: (prev.rect.max.x, prev.origin.1),
                        baseline,
                        rect: egui::Rect::from_min_max(
                            egui::pos2(prev.rect.max.x.min(origin.x as f32), rect.min.y),
                            egui::pos2(prev.rect.max.x.max(origin.x as f32), rect.max.y),
                        ),
                    });
                    self.order += 1;
                }
            }
            if self.glyphs.len() >= self.limit {
                return;
            }
            self.glyphs.push(TextGlyph {
                page: self.page,
                order: self.order,
                text,
                origin: (origin.x as f32, origin.y as f32),
                baseline,
                rect,
            });
            self.order += 1;
            if self.glyphs.len() >= self.limit {
                return;
            }
        }
    }

    fn pop_clip(&mut self) {}

    fn pop_transparency_group(&mut self) {}

    fn draw_image(&mut self, _: Image<'_, '_>, _: ImageDrawProps<'_>) {}
}

#[cfg(any(test, feature = "demo"))]
pub fn demo_pdf() -> Vec<u8> {
    let first = b"0.16 0.25 0.45 rg 20 270 160 12 re f\nBT /F1 12 Tf 0.95 0.97 1 rg 24 273 Td (Find me in the report) Tj ET\n0.72 0.75 0.80 rg 20 250 120 4 re f\n0.82 0.84 0.87 rg 20 235 156 3 re f\nBT /F1 10 Tf 0.16 0.25 0.45 rg 20 213 Td (Aurora notes) Tj ET\n0.82 0.84 0.87 rg 20 224 148 3 re f\n0.82 0.84 0.87 rg 20 213 155 3 re f\n0.82 0.84 0.87 rg 20 202 103 3 re f\n0.16 0.56 0.62 rg 20 105 74 68 re f\n0.88 0.25 0.28 rg 106 105 74 68 re f\n0.82 0.84 0.87 rg 20 86 160 3 re f\n0.82 0.84 0.87 rg 20 75 140 3 re f\n";
    let second = b"0.16 0.25 0.45 rg 20 270 160 12 re f\nBT /F1 12 Tf 0.95 0.97 1 rg 24 273 Td (Search the second page) Tj ET\n0.82 0.84 0.87 rg 20 248 156 4 re f\n0.82 0.84 0.87 rg 20 235 144 3 re f\n0.25 0.35 0.8 rg 20 80 160 112 re f\n0.18 0.58 0.55 rg 34 94 25 48 re f\n0.88 0.48 0.24 rg 72 94 25 78 re f\n0.5 0.38 0.72 rg 110 94 25 60 re f\n0.19 0.62 0.48 rg 148 94 20 92 re f\nBT /F1 10 Tf 0.95 0.97 1 rg 34 104 Td (Caf\\351 resume) Tj ET\n";
    let bodies = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Contents 4 0 R /Resources << /Font << /F1 7 0 R >> >> >>".to_vec(),
        stream(first),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Contents 6 0 R /Resources << /Font << /F1 7 0 R >> >> >>".to_vec(),
        stream(second),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
    ];
    serialize_pdf(&bodies)
}

#[cfg(any(test, feature = "demo"))]
pub fn demo_text() -> Vec<Vec<TextGlyph>> {
    let pdf = Pdf::new(demo_pdf()).expect("the synthetic PDF parses");
    let pages = pdf.pages();
    extract_text(pages)
}

#[cfg(any(test, feature = "demo"))]
fn serialize_pdf(bodies: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0usize];
    for (index, body) in bodies.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(body);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n", offsets.len()).as_bytes());
    bytes.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    bytes
}

#[cfg(any(test, feature = "demo"))]
fn stream(contents: &[u8]) -> Vec<u8> {
    let mut bytes = format!("<< /Length {} >>\nstream\n", contents.len()).into_bytes();
    bytes.extend_from_slice(contents);
    bytes.extend_from_slice(b"endstream");
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_bounds_reject_invalid_geometry_and_cap_large_pages() {
        assert_eq!(render_scale(f32::NAN, 100.0, 1.0), None);
        assert_eq!(render_scale(100.0, f32::INFINITY, 1.0), None);
        assert_eq!(render_scale(100.0, 100.0, f32::NAN), None);
        let scale = render_scale(20_000.0, 20_000.0, 2.0).unwrap();
        assert!(scale * 20_000.0 <= MAX_RENDER_SIDE + 1.0);
        assert!(scale * scale * 20_000.0 * 20_000.0 <= MAX_RENDER_PIXELS + 1_000.0);
        assert_eq!(render_scale(100.0, 200.0, 1.0), Some(1.0));
    }

    #[test]
    fn synthetic_multipage_pdf_parses_and_renders_locally() {
        let pdf = Pdf::new(demo_pdf()).unwrap();
        assert_eq!(pdf.pages().len(), 2);
        let page = &pdf.pages()[0];
        let image = render_page(
            page,
            &RenderCache::new(),
            &InterpreterSettings::default(),
            &RenderSettings::default(),
            1.0,
        )
        .unwrap();
        assert_eq!(image.size, [200, 300]);
        assert!(
            image
                .pixels
                .iter()
                .any(|pixel| *pixel != egui::Color32::WHITE)
        );
    }

    #[test]
    fn synthetic_pdf_text_is_extracted_in_document_coordinates() {
        let pdf = Pdf::new(demo_pdf()).unwrap();
        let pages = pdf.pages();
        let text = extract_text(pages);
        assert_eq!(text.len(), 2);
        let joined: String = text
            .iter()
            .flatten()
            .map(|glyph| glyph.text.as_str())
            .collect();
        let lower = joined.to_lowercase();
        assert!(lower.contains("find me in the report"), "{joined}");
        assert!(lower.contains("search the second page"), "{joined}");
        assert!(lower.contains("café"), "{joined}");
        let find = text[0]
            .iter()
            .find(|glyph| glyph.text == "F")
            .expect("the first glyph of Find me");
        assert_eq!(find.page, 0);
        assert_eq!(find.order, 0);
        assert!(find.rect.min.x >= 0.0 && find.rect.max.x <= 200.0);
        assert!(find.rect.min.y >= 0.0 && find.rect.max.y <= 300.0);
        assert!(
            find.rect.width() >= 4.0 && find.rect.width() <= 20.0,
            "unexpected glyph width: {:?}",
            find.rect
        );
        assert!(
            find.rect.height() >= 8.0 && find.rect.height() <= 20.0,
            "unexpected glyph height: {:?}",
            find.rect
        );
    }

    #[test]
    fn tj_word_gaps_emit_space_glyphs() {
        let content = b"BT /F1 12 Tf 20 260 Td [(Hello) -400 (world)] TJ ET\n";
        let bodies = [
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_vec(),
            stream(content),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
        ];
        let pdf = Pdf::new(serialize_pdf(&bodies)).unwrap();
        let text = extract_text(pdf.pages());
        let joined: String = text[0].iter().map(|g| g.text.as_str()).collect();
        assert_eq!(joined, "Hello world");
    }

    #[test]
    fn texture_side_cap_keeps_rasters_uploadable() {
        assert_eq!(render_scale_with_side(100.0, 200.0, 1.0, 0), None);
        assert_eq!(render_scale_with_side(100.0, 200.0, 1.0, 4096), Some(1.0));
        let scale = render_scale_with_side(3_000.0, 1_000.0, 1.0, 2_048).unwrap();
        assert!(scale * 3_000.0 <= 2_048.0 + 0.01);
        assert!(scale * 1_000.0 <= 2_048.0 + 0.01);
    }

    #[test]
    fn rotated_glyph_boxes_follow_the_transform() {
        let fixture = |matrix: &[u8]| {
            let mut content = b"BT /F1 12 Tf ".to_vec();
            content.extend_from_slice(matrix);
            content.extend_from_slice(b" (A) Tj ET\n");
            let bodies = [
                b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
                b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
                b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_vec(),
                stream(&content),
                b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
            ];
            let pdf = Pdf::new(serialize_pdf(&bodies)).unwrap();
            extract_page(&pdf.pages()[0], 0, MAX_GLYPHS_PER_PAGE)
                .pop()
                .expect("the glyph is extracted")
        };
        let upright = fixture(b"1 0 0 1 100 100 Tm");
        let rotated = fixture(b"0 1 -1 0 100 100 Tm");
        assert!(upright.rect.height() > upright.rect.width(), "{upright:?}");
        assert_eq!(upright.baseline, (1.0, 0.0));
        assert!(
            rotated.rect.width() > rotated.rect.height() * 1.2,
            "rotated box should turn with the glyph: {rotated:?}"
        );
        assert_eq!(rotated.baseline, (0.0, 1.0));
    }

    #[test]
    fn dropping_worker_signals_cancellation() {
        let file = tempfile::Builder::new().suffix(".pdf").tempfile().unwrap();
        std::fs::write(file.path(), demo_pdf()).unwrap();
        let worker = Worker::spawn(file.path(), egui::Context::default()).unwrap();
        let cancelled = Arc::clone(&worker.cancelled);
        assert!(!stopped(&cancelled));
        drop(worker);
        assert!(stopped(&cancelled));
    }

    #[test]
    fn invalid_pdf_is_rejected_without_exposing_its_contents() {
        let path = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(path.path(), b"private fixture text, not a PDF").unwrap();
        assert!(matches!(
            load(path.path(), &AtomicBool::new(false)),
            Err("This file is not a readable PDF.")
        ));
    }
}
