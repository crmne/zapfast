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
        assert_eq!(themes.len(), 5);
        for theme in themes {
            let palette = theme.palette;
            assert_eq!(palette.chat, palette.window);
            assert_eq!(palette.bubble_in, palette.surface);
            assert_ne!(palette.bubble_out, palette.bubble_in);
            assert_eq!(palette.link, palette.accent);
            assert_eq!(palette.dark, theme.filename != "Catppuccin Latte.json");
        }
    }
}
