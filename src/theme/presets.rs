//! The same local palettes used in Spotifast, embedded for every installation.

use super::custom::{CustomTheme, parse_palette};

const FILES: &[(&str, &str)] = &[
    (
        "Catppuccin Latte.json",
        include_str!("../../assets/themes/Catppuccin Latte.json"),
    ),
    (
        "Catppuccin.json",
        include_str!("../../assets/themes/Catppuccin.json"),
    ),
    ("Nord.json", include_str!("../../assets/themes/Nord.json")),
    (
        "Ristretto.json",
        include_str!("../../assets/themes/Ristretto.json"),
    ),
    (
        "Tokyo Night.json",
        include_str!("../../assets/themes/Tokyo Night.json"),
    ),
    (
        "Rose Pine.json",
        include_str!("../../assets/themes/Rose Pine.json"),
    ),
    (
        "Rose Pine Moon.json",
        include_str!("../../assets/themes/Rose Pine Moon.json"),
    ),
    (
        "Rose Pine Dawn.json",
        include_str!("../../assets/themes/Rose Pine Dawn.json"),
    ),
];

pub(crate) fn themes() -> impl Iterator<Item = CustomTheme> {
    FILES.iter().map(|(filename, text)| CustomTheme {
        filename: (*filename).into(),
        palette: parse_palette(text).expect("bundled palette must be valid"),
    })
}

pub(super) fn contains(filename: &str) -> bool {
    FILES.iter().any(|(name, _)| *name == filename)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spotifast_palettes_also_colour_the_conversation() {
        let themes: Vec<_> = themes().collect();
        assert_eq!(themes.len(), 8);
        for theme in themes {
            let palette = theme.palette;
            assert_eq!(palette.chat, palette.window);
            assert_eq!(palette.bubble_in, palette.surface);
            assert_ne!(palette.bubble_out, palette.bubble_in);
            assert_eq!(palette.link, palette.accent);
            assert_eq!(
                palette.dark,
                !matches!(
                    theme.filename.as_str(),
                    "Catppuccin Latte.json" | "Rose Pine Dawn.json"
                )
            );
        }
    }

    #[test]
    fn rose_pine_dawn_hovered_primary_buttons_keep_readable_content() {
        fn luminance(color: egui::Color32) -> f64 {
            let linear = [color.r(), color.g(), color.b()].map(|channel| {
                let value = f64::from(channel) / 255.0;
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            });
            0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2]
        }

        let palette = themes()
            .find(|theme| theme.filename == "Rose Pine Dawn.json")
            .unwrap()
            .palette;
        let foreground = luminance(palette.on_accent);
        let background = luminance(palette.accent_hover);
        let contrast = (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05);
        assert!(contrast >= 4.5, "hover contrast is only {contrast:.2}:1");
    }
}
