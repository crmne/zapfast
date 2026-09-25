//! Emoji drawn by DirectWrite and Direct2D with the system's Segoe UI Emoji.
//!
//! Segoe UI Emoji keeps its colours in COLR layers, which Direct2D draws when
//! it is asked to use colour fonts.

use std::cell::Cell;
use std::ffi::c_void;

use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory,
    ID2D1Factory,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_GLYPH_RUN, DWRITE_GLYPH_RUN_DESCRIPTION, DWRITE_LINE_METRICS,
    DWRITE_MATRIX, DWRITE_MEASURING_MODE, DWRITE_STRIKETHROUGH, DWRITE_UNDERLINE,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory, IDWriteFactory, IDWriteFontFace2,
    IDWriteInlineObject, IDWritePixelSnapping_Impl, IDWriteTextRenderer, IDWriteTextRenderer_Impl,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory,
    WICBitmapCacheOnLoad, WICBitmapLockRead, WICRect,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::core::{BOOL, ComObject, IUnknown, Interface, Ref, Result, implement, w};
use windows_numerics::Vector2;

/// Size to draw at, in pixels: about twice the cached texture width, so the
/// picture is scaled down rather than up.
const SIZE: f32 = 144.0;

/// Canvas side, two ems, and where the pen starts, in pixels from the top
/// left. Every sequence is drawn at the same place, and no ink, overhangs
/// included, reaches the edge.
const CANVAS: u32 = 288;
const PEN_X: f32 = 72.0;
const BASELINE: f32 = 202.0;

struct Factories {
    d2d: ID2D1Factory,
    write: IDWriteFactory,
    wic: IWICImagingFactory,
}

thread_local! {
    /// COM objects stay on the thread that made them.
    static FACTORIES: Option<Factories> = factories().ok();
}

/// Draws `cluster` as one colour picture on a fixed canvas and returns its
/// width, height, and premultiplied RGBA rows. Returns `None` when
/// DirectWrite does not join the cluster into one glyph that advances or
/// draws it with a font without colour glyphs. Zero-advance glyphs are layers
/// under it. Also `None` for subdivision flags: Segoe UI Emoji has none and
/// draws their tag characters as nothing over a plain black flag.
pub(super) fn render(cluster: &str) -> Option<(u32, u32, Vec<u8>)> {
    if cluster
        .chars()
        .any(|character| ('\u{E0020}'..='\u{E007F}').contains(&character))
    {
        return None;
    }
    FACTORIES.with(|factories| draw(factories.as_ref()?, cluster).ok()?)
}

fn factories() -> Result<Factories> {
    // SAFETY: the factory constructors take no pointers but the null options.
    unsafe {
        // WIC is a COM class. A thread that already has an apartment keeps
        // it; a new one is single-threaded, like the one winit's OLE drag and
        // drop sets up.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        Ok(Factories {
            d2d: D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?,
            write: DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?,
            wic: CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?,
        })
    }
}

fn draw(factories: &Factories, cluster: &str) -> Result<Option<(u32, u32, Vec<u8>)>> {
    let text: Vec<u16> = cluster.encode_utf16().collect();
    // SAFETY: every pointer passed points to a live local, and the locked
    // bitmap memory is read only while `lock` is held, within its size.
    unsafe {
        let format = factories.write.CreateTextFormat(
            w!("Segoe UI Emoji"),
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            SIZE,
            w!("en-us"),
        )?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        let layout = factories
            .write
            .CreateTextLayout(&text, &format, SIZE * 8.0, SIZE * 8.0)?;
        let counter = ComObject::new(Counter::default());
        let renderer: IDWriteTextRenderer = counter.to_interface();
        layout.Draw(None, &renderer, 0.0, 0.0)?;
        if counter.glyphs.get() != 1 || counter.monochrome.get() {
            return Ok(None);
        }
        let mut lines = [DWRITE_LINE_METRICS::default()];
        let mut count = 0;
        layout.GetLineMetrics(Some(&mut lines), &mut count)?;
        let top = BASELINE - lines[0].baseline;

        let bitmap = factories.wic.CreateBitmap(
            CANVAS,
            CANVAS,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapCacheOnLoad,
        )?;
        let properties = D2D1_RENDER_TARGET_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            ..Default::default()
        };
        let target = factories
            .d2d
            .CreateWicBitmapRenderTarget(&bitmap, &properties)?;
        let black = D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        };
        let brush = target.CreateSolidColorBrush(&black, None)?;
        target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        target.BeginDraw();
        // No colour clears to transparent black.
        target.Clear(None);
        target.DrawTextLayout(
            Vector2 { X: PEN_X, Y: top },
            &layout,
            &brush,
            D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
        );
        target.EndDraw(None, None)?;

        let area = WICRect {
            X: 0,
            Y: 0,
            Width: CANVAS as i32,
            Height: CANVAS as i32,
        };
        let lock = bitmap.Lock(&area, WICBitmapLockRead.0 as u32)?;
        let stride = lock.GetStride()? as usize;
        let mut size = 0;
        let mut data = std::ptr::null_mut();
        lock.GetDataPointer(&mut size, &mut data)?;
        if data.is_null() {
            return Ok(None);
        }
        let source = std::slice::from_raw_parts(data, size as usize);
        let row = CANVAS as usize * 4;
        let mut pixels = Vec::with_capacity(row * CANVAS as usize);
        for line in source.chunks(stride).take(CANVAS as usize) {
            let Some(line) = line.get(..row) else {
                return Ok(None);
            };
            pixels.extend_from_slice(line);
        }
        drop(lock);
        if pixels.len() != row * CANVAS as usize {
            return Ok(None);
        }
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        Ok(Some((CANVAS, CANVAS, pixels)))
    }
}

/// Counts the glyphs a layout draws that advance, and notes a font without
/// colour glyphs, which DirectWrite substitutes for a character Segoe UI
/// Emoji lacks.
#[implement(IDWriteTextRenderer)]
#[derive(Default)]
struct Counter {
    glyphs: Cell<u32>,
    monochrome: Cell<bool>,
}

impl IDWriteTextRenderer_Impl for Counter_Impl {
    fn DrawGlyphRun(
        &self,
        _context: *const c_void,
        _x: f32,
        _y: f32,
        _mode: DWRITE_MEASURING_MODE,
        run: *const DWRITE_GLYPH_RUN,
        _description: *const DWRITE_GLYPH_RUN_DESCRIPTION,
        _effect: Ref<IUnknown>,
    ) -> Result<()> {
        // SAFETY: DirectWrite passes a valid glyph run for this call.
        let Some(run) = (unsafe { run.as_ref() }) else {
            return Ok(());
        };
        let advancing = if run.glyphAdvances.is_null() {
            run.glyphCount
        } else {
            // SAFETY: DirectWrite passes one advance for each glyph of the run.
            let advances =
                unsafe { std::slice::from_raw_parts(run.glyphAdvances, run.glyphCount as usize) };
            advances.iter().filter(|advance| **advance != 0.0).count() as u32
        };
        self.glyphs.set(self.glyphs.get() + advancing);
        let colour = (*run.fontFace)
            .as_ref()
            .and_then(|face| face.cast::<IDWriteFontFace2>().ok())
            // SAFETY: the face is a live font face for this call.
            .is_some_and(|face| unsafe { face.IsColorFont() }.as_bool());
        if !colour {
            self.monochrome.set(true);
        }
        Ok(())
    }

    fn DrawUnderline(
        &self,
        _context: *const c_void,
        _x: f32,
        _y: f32,
        _underline: *const DWRITE_UNDERLINE,
        _effect: Ref<IUnknown>,
    ) -> Result<()> {
        Ok(())
    }

    fn DrawStrikethrough(
        &self,
        _context: *const c_void,
        _x: f32,
        _y: f32,
        _strikethrough: *const DWRITE_STRIKETHROUGH,
        _effect: Ref<IUnknown>,
    ) -> Result<()> {
        Ok(())
    }

    fn DrawInlineObject(
        &self,
        _context: *const c_void,
        _x: f32,
        _y: f32,
        _object: Ref<IDWriteInlineObject>,
        _sideways: BOOL,
        _right_to_left: BOOL,
        _effect: Ref<IUnknown>,
    ) -> Result<()> {
        Ok(())
    }
}

impl IDWritePixelSnapping_Impl for Counter_Impl {
    fn IsPixelSnappingDisabled(&self, _context: *const c_void) -> Result<BOOL> {
        Ok(true.into())
    }

    fn GetCurrentTransform(
        &self,
        _context: *const c_void,
        transform: *mut DWRITE_MATRIX,
    ) -> Result<()> {
        // SAFETY: DirectWrite passes a valid matrix to fill in.
        if let Some(transform) = unsafe { transform.as_mut() } {
            *transform = DWRITE_MATRIX {
                m11: 1.0,
                m22: 1.0,
                ..Default::default()
            };
        }
        Ok(())
    }

    fn GetPixelsPerDip(&self, _context: *const c_void) -> Result<f32> {
        Ok(1.0)
    }
}
