//! Sticker search: by emoji, by words that name an emoji, and by pack name.
//!
//! Stickers carry the emojis they express in their metadata. Typing 😂 finds
//! stickers tagged 😂, typing "laugh" finds them through the emoji's name, and
//! typing part of a pack's name lists that pack.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::model::StickerPack;

/// The same emoji with or without its presentation selector compares equal.
fn plain(emoji: &str) -> String {
    emoji.chars().filter(|c| *c != '\u{fe0f}').collect()
}

/// Emojis a search term names: the emoji itself, or every emoji whose name
/// or shortcode contains the word.
fn named_emojis(word: &str) -> HashSet<String> {
    let mut found = HashSet::new();
    if let Some(emoji) = emojis::get(word) {
        found.insert(plain(emoji.as_str()));
        return found;
    }
    let word = word.to_lowercase();
    if word.chars().count() < 2 {
        return found;
    }
    for emoji in emojis::iter() {
        let named = emoji.name().to_lowercase().contains(&word)
            || emoji.shortcodes().any(|code| code.contains(&word));
        if named {
            found.insert(plain(emoji.as_str()));
        }
    }
    found
}

/// Where a search looks: the picker's lists and each sticker's emojis.
pub struct Library<'a> {
    pub recent: &'a [PathBuf],
    pub favorites: &'a [PathBuf],
    pub packs: &'a [StickerPack],
    pub emojis: &'a HashMap<PathBuf, Vec<String>>,
}

/// Stickers matching `query`, favorites and recent ones first, each once.
pub fn search(library: &Library<'_>, query: &str) -> Vec<PathBuf> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    use unicode_segmentation::UnicodeSegmentation;
    // Every word or emoji in the query must be matched by the sticker.
    let terms: Vec<HashSet<String>> = query
        .split_whitespace()
        .flat_map(|word| {
            if emojis::get(word).is_some() || word.chars().all(|c| !c.is_alphanumeric()) {
                word.graphemes(true).map(named_emojis).collect::<Vec<_>>()
            } else {
                vec![named_emojis(word)]
            }
        })
        .collect();
    let tagged = |path: &PathBuf| {
        let emojis: HashSet<String> = library
            .emojis
            .get(path)
            .into_iter()
            .flatten()
            .map(|emoji| plain(emoji))
            .collect();
        !emojis.is_empty()
            && !terms.is_empty()
            && terms
                .iter()
                .all(|term| term.iter().any(|emoji| emojis.contains(emoji)))
    };
    let needle = query.to_lowercase();
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    let everything = library
        .favorites
        .iter()
        .chain(library.recent)
        .chain(library.packs.iter().flat_map(|pack| &pack.stickers));
    for path in everything {
        if tagged(path) && seen.insert(path.clone()) {
            found.push(path.clone());
        }
    }
    for pack in library.packs {
        if pack.name.to_lowercase().contains(&needle) {
            for path in &pack.stickers {
                if seen.insert(path.clone()) {
                    found.push(path.clone());
                }
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(name: &str, stickers: &[&str]) -> StickerPack {
        StickerPack {
            name: name.into(),
            dir: PathBuf::from(format!("/packs/{name}")),
            stickers: stickers.iter().map(PathBuf::from).collect(),
            local: false,
        }
    }

    fn library_search(query: &str) -> Vec<String> {
        let emojis: HashMap<PathBuf, Vec<String>> = [
            ("/fav/laugh.webp", vec!["😂"]),
            ("/fav/love.webp", vec!["❤️", "😍"]),
            ("/packs/Ducks/1.webp", vec!["🦆", "😂"]),
            ("/recent/frog.webp", vec!["🐸"]),
        ]
        .into_iter()
        .map(|(path, emojis)| {
            (
                PathBuf::from(path),
                emojis.into_iter().map(str::to_owned).collect(),
            )
        })
        .collect();
        let favorites = [
            PathBuf::from("/fav/laugh.webp"),
            PathBuf::from("/fav/love.webp"),
        ];
        let recent = [
            PathBuf::from("/recent/frog.webp"),
            PathBuf::from("/fav/laugh.webp"),
        ];
        let packs = [
            pack("Ducks", &["/packs/Ducks/1.webp", "/packs/Ducks/2.webp"]),
            pack("Bom dia", &["/packs/Bom dia/1.webp"]),
        ];
        search(
            &Library {
                recent: &recent,
                favorites: &favorites,
                packs: &packs,
                emojis: &emojis,
            },
            query,
        )
        .into_iter()
        .map(|path| path.display().to_string())
        .collect()
    }

    #[test]
    fn an_emoji_finds_the_stickers_tagged_with_it_once() {
        assert_eq!(
            library_search("😂"),
            vec!["/fav/laugh.webp", "/packs/Ducks/1.webp"]
        );
        // The heart matches with or without its presentation selector.
        assert_eq!(library_search("❤"), vec!["/fav/love.webp"]);
    }

    #[test]
    fn a_word_finds_stickers_through_the_emoji_it_names() {
        assert_eq!(library_search("frog"), vec!["/recent/frog.webp"]);
        assert_eq!(
            library_search("DUCK"),
            vec!["/packs/Ducks/1.webp", "/packs/Ducks/2.webp"]
        );
    }

    #[test]
    fn a_pack_name_lists_the_whole_pack() {
        assert_eq!(library_search("bom"), vec!["/packs/Bom dia/1.webp"]);
        assert!(library_search("   ").is_empty());
        assert!(library_search("zzzz").is_empty());
    }
}
