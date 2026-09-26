//! Emoji drawn by CoreText with the system's Apple Color Emoji.
//!
//! Apple Color Emoji joins flags, skin tones, and ZWJ sequences through an
//! AAT `morx` table, which only CoreText applies. Some sequences, such as
//! couples, become a zero-advance layer glyph under one normal glyph.

use std::ptr::{NonNull, null};

use objc2_core_foundation::{
    CFAttributedString, CFDictionary, CFRange, CFString, CFType, CGFloat, CGSize,
};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGImageAlphaInfo, CGImageByteOrderInfo,
    kCGColorSpaceSRGB,
};
use objc2_core_text::{CTFont, CTFontSymbolicTraits, CTLine, CTRun, kCTFontAttributeName};

/// Point size to draw at: about twice the cached texture width, so the
/// picture is scaled down rather than up.
const SIZE: CGFloat = 144.0;

/// Canvas side, two ems, and where the pen starts, in pixels from the top
/// left. Every sequence is drawn at the same place, and no ink reaches the
/// edge.
const CANVAS: usize = 288;
const PEN_X: CGFloat = 72.0;
const BASELINE: CGFloat = 202.0;

/// Draws `cluster` as one colour picture on a fixed canvas and returns its
/// width, height, and premultiplied RGBA rows. Returns `None` when CoreText
/// does not join the cluster into one picture or draws it with a font without
/// colour glyphs.
pub(super) fn render(cluster: &str) -> Option<(u32, u32, Vec<u8>)> {
    // SAFETY: every pointer passed to CoreText and CoreGraphics is null where
    // allowed or points to live memory; `pixels` outlives the bitmap context
    // that draws into it, and has room for `CANVAS` rows of `CANVAS * 4` bytes.
    unsafe {
        let name = CFString::from_static_str("AppleColorEmoji");
        let font = CTFont::with_name(&name, SIZE, null());
        let font_value: &CFType = &font;
        let attributes =
            CFDictionary::<CFString, CFType>::from_slices(&[kCTFontAttributeName], &[font_value]);
        let text = CFAttributedString::new(
            None,
            Some(&CFString::from_str(cluster)),
            Some(attributes.as_opaque()),
        )?;
        let line = CTLine::with_attributed_string(&text);
        if !joined(&line) {
            return None;
        }
        let mut pixels = vec![0u8; CANVAS * CANVAS * 4];
        let space = CGColorSpace::with_name(Some(kCGColorSpaceSRGB))?;
        let context = CGBitmapContextCreate(
            pixels.as_mut_ptr().cast(),
            CANVAS,
            CANVAS,
            8,
            CANVAS * 4,
            Some(&space),
            CGImageAlphaInfo::PremultipliedLast.0 | CGImageByteOrderInfo::Order32Big.0,
        )?;
        // CoreGraphics measures up from the bottom edge.
        CGContext::set_text_position(Some(&context), PEN_X, CANVAS as CGFloat - BASELINE);
        line.draw(&context);
        drop(context);
        Some((CANVAS as u32, CANVAS as u32, pixels))
    }
}

/// Whether the line is one picture: a single run in a font with colour
/// glyphs, rather than a monochrome font CoreText substituted, with exactly
/// one glyph that advances. Zero-advance glyphs are layers drawn under it.
///
/// # Safety
///
/// `line` must be a valid line built from an attributed string.
unsafe fn joined(line: &CTLine) -> bool {
    // SAFETY: a line's runs are `CTRun`s and a run's font attribute, when
    // present, is a `CTFont`; both stay alive while `runs` is held. The
    // advances buffer has room for every glyph of the run.
    unsafe {
        let runs = line.glyph_runs();
        if runs.count() != 1 {
            return false;
        }
        let run = &*runs.value_at_index(0).cast::<CTRun>();
        let attributes = run.attributes();
        let key: *const CFString = kCTFontAttributeName;
        let Some(font) = attributes.value(key.cast()).cast::<CTFont>().as_ref() else {
            return false;
        };
        if !font
            .symbolic_traits()
            .contains(CTFontSymbolicTraits::ColorGlyphsTrait)
        {
            return false;
        }
        let count = usize::try_from(run.glyph_count()).unwrap_or(0);
        let mut advances = vec![CGSize::new(0.0, 0.0); count];
        let Some(buffer) = NonNull::new(advances.as_mut_ptr()) else {
            return false;
        };
        run.advances(CFRange::new(0, 0), buffer);
        advances
            .iter()
            .filter(|advance| advance.width != 0.0)
            .count()
            == 1
    }
}
