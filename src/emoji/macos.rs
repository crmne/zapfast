//! Emoji drawn by CoreText with the system's Apple Color Emoji.
//!
//! Apple Color Emoji joins flags, skin tones, and ZWJ sequences through an
//! AAT `morx` table, which only CoreText applies. Some sequences, such as
//! couples, become a zero-advance layer glyph under one normal glyph.

use std::ptr::{NonNull, null, null_mut};

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

/// Draws `cluster` as one colour picture and returns its width, height, and
/// unpremultiplied RGBA rows. Returns `None` when CoreText does not join the
/// cluster into one picture, draws it with a font without colour glyphs, or
/// draws nothing.
pub(super) fn render(cluster: &str) -> Option<(u32, u32, Vec<u8>)> {
    // SAFETY: every pointer passed to CoreText and CoreGraphics is null where
    // allowed or points to live memory; `pixels` outlives the bitmap context
    // that draws into it, and has room for `side` rows of `side * 4` bytes.
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
        // Emoji glyphs are square, so a square as wide as the advance holds
        // the picture, centred vertically on what CoreText draws.
        let advance = line.typographic_bounds(null_mut(), null_mut(), null_mut());
        let side = advance.ceil() as usize;
        if side == 0 {
            return None;
        }
        let mut pixels = vec![0u8; side * side * 4];
        let space = CGColorSpace::with_name(Some(kCGColorSpaceSRGB))?;
        let context = CGBitmapContextCreate(
            pixels.as_mut_ptr().cast(),
            side,
            side,
            8,
            side * 4,
            Some(&space),
            CGImageAlphaInfo::PremultipliedLast.0 | CGImageByteOrderInfo::Order32Big.0,
        )?;
        let bounds = line.image_bounds(Some(&context));
        let baseline = side as CGFloat / 2.0 - (bounds.origin.y + bounds.size.height / 2.0);
        CGContext::set_text_position(Some(&context), 0.0, baseline);
        line.draw(&context);
        drop(context);
        if pixels.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0) {
            return None;
        }
        unpremultiply(&mut pixels);
        Some((side as u32, side as u32, pixels))
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

/// Divides each colour channel by alpha.
fn unpremultiply(pixels: &mut [u8]) {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        if alpha == 0 {
            continue;
        }
        for channel in &mut pixel[..3] {
            *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
        }
    }
}
