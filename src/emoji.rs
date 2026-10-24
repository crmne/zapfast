//! Colour emoji in the platform's own style, with bundled Noto fallback.
//! Builds with ZAPFAST_EMOJI_FONT embed that font instead and disable system
//! emoji so messages, reactions, the picker, and text fields use its artwork.
//!
//! Layout replaces each emoji sequence with a transparent placeholder, and
//! its picture is painted over it. New pictures are drawn on a worker
//! thread, so a picker full of unseen emoji never stalls a frame; tests and
//! demo builds draw them inside the frame so every screenshot shows them.
//! These wrappers install ZapFast's choice before the first use.

use egui::text::LayoutJob;
use egui::{Pos2, Rect, TextFormat};

pub use fastframe_emoji::{PLACEHOLDER, only_emoji};

include!(concat!(env!("OUT_DIR"), "/emoji-font.rs"));

/// The default fallback, used when no custom font was supplied at build time.
const NOTO: &[u8] = include_bytes!("../assets/fonts/NotoColorEmoji.ttf");
const BUNDLED: &[u8] = match CUSTOM_EMOJI_FONT {
    Some(font) => font,
    None => NOTO,
};

fn configuration() -> fastframe_emoji::EmojiSetup {
    fastframe_emoji::EmojiSetup::default()
        .system(CUSTOM_EMOJI_FONT.is_none())
        .bundled(BUNDLED)
        .synchronous(cfg!(any(test, feature = "demo")))
}

fn setup() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        configuration().install();
    });
}

/// Whether a colour emoji font is available.
pub fn available() -> bool {
    setup();
    fastframe_emoji::available()
}

/// Loads the bundled emoji font before the first frame needs it.
pub fn warm_up() {
    setup();
    fastframe_emoji::warm_up();
}

/// The egui plugin that colours the emoji in every other text: labels,
/// buttons, menus, tooltips and text fields. It leaves the placeholders
/// `append` lays out and the glyphs `editor_job` hides to the paint calls
/// below, so message bodies keep their selectable, copyable placeholders.
pub fn plugin() -> fastframe_emoji::EmojiPlugin {
    setup();
    fastframe_emoji::EmojiPlugin::default()
}

/// Queues pictures to be drawn off the interface thread before a frame shows
/// them: the first page of a picker as it opens.
pub fn prewarm<'a>(ctx: &egui::Context, clusters: impl IntoIterator<Item = &'a str>) {
    setup();
    fastframe_emoji::prewarm(ctx, clusters);
}

/// Appends text with placeholders and records their emoji sequences.
pub fn append(
    ui: &egui::Ui,
    job: &mut LayoutJob,
    placements: &mut Vec<String>,
    text: &str,
    format: &TextFormat,
) -> usize {
    setup();
    fastframe_emoji::append(ui, job, placements, text, format)
}

/// Paints one emoji over `rect`, or its text when no font draws it.
pub fn paint_cluster(ui: &egui::Ui, cluster: &str, rect: Rect) {
    setup();
    fastframe_emoji::paint_cluster(ui, cluster, rect);
}

/// Builds an editor layout without changing character offsets. Emoji glyphs
/// are transparent and returned for painting over.
pub fn editor_job(
    text: &str,
    format: &egui::TextFormat,
) -> (egui::text::LayoutJob, Vec<(usize, usize, String)>) {
    setup();
    fastframe_emoji::editor_job(text, format)
}

/// Paints the emoji over a galley's placeholders.
pub fn paint(ui: &egui::Ui, galley: &egui::Galley, origin: Pos2, placements: &[String]) {
    setup();
    fastframe_emoji::paint(ui, galley, origin, placements);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bundled font alone, for comparison with the installed renderer.
    fn bundled() -> fastframe_emoji::Emoji {
        fastframe_emoji::EmojiSetup::default()
            .system(false)
            .bundled(BUNDLED)
            .load()
    }

    #[test]
    fn the_bundled_font_renders_colour_emoji() {
        let picture = bundled().render("\u{1F600}", 72).expect("grinning face");
        assert_eq!(picture.size[1], 72);
        assert!(picture.has_colour());
    }

    #[cfg(custom_emoji_font)]
    #[test]
    fn the_installed_renderer_uses_the_bundled_artwork() {
        setup();
        assert!(fastframe_emoji::available());
        let expected = bundled();
        for sequence in ["😀", "👍🏽", "🇩🇪", "❤️", "👨‍👩‍👧"] {
            if let Some(picture) = expected.render(sequence, 72) {
                assert_eq!(
                    fastframe_emoji::get().render(sequence, 72),
                    Some(picture),
                    "{sequence} must use the bundled artwork instead of the system font"
                );
            }
        }
    }

    #[cfg(not(custom_emoji_font))]
    #[test]
    fn emoji_render_in_colour_at_chat_and_picker_sizes() {
        let emoji = bundled();
        for sequence in ["😀", "👍🏽", "🇩🇪", "❤️"] {
            for height in [16, 24, 48, 72] {
                let picture = emoji.render(sequence, height).expect("colour emoji");
                assert_eq!(picture.size[1], height as usize);
                assert!(picture.has_colour(), "{sequence} at {height}px");
                assert!(picture.coverage() > 0.0, "{sequence} at {height}px");
            }
        }
    }

    #[cfg(not(custom_emoji_font))]
    #[test]
    fn the_bundled_font_joins_sequences() {
        let emoji = bundled();
        for sequence in ["🇩🇪", "👍🏽", "👨‍👩‍👧", "#️⃣", "🏳️‍🌈", "🫱🏼‍🫲🏿", "🏃‍➡️"]
        {
            let joined = emoji.render(sequence, 72).expect("a picture");
            let first: String = sequence.chars().take(1).collect();
            assert_ne!(
                Some(joined),
                emoji.render(&first, 72),
                "{sequence} fell back to its first part"
            );
        }
    }

    #[test]
    #[cfg(not(custom_emoji_font))]
    fn the_default_build_keeps_platform_emoji_and_noto_fallback() {
        setup();
        let expected = fastframe_emoji::EmojiSetup::default().bundled(NOTO).load();
        for sequence in ["😀", "👍🏽", "🇩🇪"] {
            assert_eq!(
                fastframe_emoji::get().render(sequence, 24),
                expected.render(sequence, 24)
            );
        }
    }

    #[test]
    fn placeholders_line_up_with_placements() {
        let mut job = LayoutJob::default();
        let mut placements = Vec::new();
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            append(
                ui,
                &mut job,
                &mut placements,
                "a 😀 b",
                &TextFormat::default(),
            );
        });
        output.textures_delta.clear();
        assert!(available(), "the bundled font is always there");
        assert_eq!(placements, vec!["😀".to_owned()]);
        assert_eq!(job.text.matches(PLACEHOLDER).count(), 1);
    }

    #[test]
    fn an_editor_job_keeps_the_text_and_places_the_emoji() {
        let format =
            egui::TextFormat::simple(egui::FontId::proportional(14.0), egui::Color32::WHITE);
        let text = "hi 😊 and 👍🏽!";
        let (job, clusters) = editor_job(text, &format);
        assert_eq!(job.text, text, "the galley must mirror the buffer");
        assert_eq!(clusters.len(), 2);
        for (start, length, cluster) in &clusters {
            assert_eq!(
                text.chars().skip(*start).take(*length).collect::<String>(),
                *cluster
            );
        }
    }

    #[test]
    fn emoji_only_messages_are_counted() {
        assert_eq!(only_emoji("😂"), Some(1));
        assert_eq!(only_emoji("😂 🙏"), Some(2));
        assert_eq!(only_emoji("ok 😂"), None);
    }
}
