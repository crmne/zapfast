//! Emoji and pack metadata inside WebP sticker files.
//!
//! WhatsApp stickers carry a small EXIF chunk whose single TIFF entry (tag
//! 0x5741) holds JSON such as `{"sticker-pack-id": "…", "emojis": ["😂"]}`.
//! WhatsApp reads those emojis to suggest stickers, and ZapFast reads them to
//! search stickers by emoji. Writing the same chunk keeps the association when
//! a sticker made or imported here is sent.

use serde::{Deserialize, Serialize};

/// The TIFF tag WhatsApp stores its sticker JSON under.
const STICKER_TAG: u16 = 0x5741;

/// The JSON WhatsApp keeps in a sticker's EXIF chunk.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct StickerInfo {
    #[serde(
        rename = "sticker-pack-id",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub pack_id: String,
    #[serde(
        rename = "sticker-pack-name",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub pack_name: String,
    #[serde(
        rename = "sticker-pack-publisher",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub publisher: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emojis: Vec<String>,
}

/// One RIFF chunk: its four-byte id and payload.
struct Chunk<'a> {
    id: [u8; 4],
    data: &'a [u8],
}

/// Splits a WebP file into its chunks, or `None` when it is not one.
fn chunks(bytes: &[u8]) -> Option<Vec<Chunk<'_>>> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return None;
    }
    let mut chunks = Vec::new();
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let id: [u8; 4] = bytes[pos..pos + 4].try_into().ok()?;
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().ok()?) as usize;
        let start = pos + 8;
        let end = start.checked_add(size)?;
        if end > bytes.len() {
            return None;
        }
        chunks.push(Chunk {
            id,
            data: &bytes[start..end],
        });
        // Chunks are padded to an even size.
        pos = end + (size & 1);
    }
    Some(chunks)
}

/// Reads WhatsApp's sticker JSON from a WebP file, if it has any.
pub fn read(bytes: &[u8]) -> Option<StickerInfo> {
    let exif = chunks(bytes)?
        .into_iter()
        .find(|chunk| &chunk.id == b"EXIF")?
        .data;
    tiff_entry(exif, STICKER_TAG)
        .and_then(json_object)
        // Some tools write the JSON without a TIFF wrapper, or point at it
        // wrongly; take the object from the whole chunk then.
        .or_else(|| json_object(exif))
}

/// The JSON object inside `bytes`, from its first brace to its last.
fn json_object(bytes: &[u8]) -> Option<StickerInfo> {
    let start = bytes.iter().position(|byte| *byte == b'{')?;
    let end = bytes.iter().rposition(|byte| *byte == b'}')?;
    serde_json::from_slice(bytes.get(start..=end)?).ok()
}

/// The emojis a sticker file is associated with.
pub fn emojis(bytes: &[u8]) -> Vec<String> {
    read(bytes)
        .map(|info| info.emojis)
        .unwrap_or_default()
        .into_iter()
        .map(|emoji| emoji.trim().to_owned())
        .filter(|emoji| !emoji.is_empty())
        .collect()
}

/// The value of one entry in a little- or big-endian TIFF block.
fn tiff_entry(tiff: &[u8], tag: u16) -> Option<&[u8]> {
    let little = match tiff.get(0..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let bytes: [u8; 2] = tiff.get(at..at + 2)?.try_into().ok()?;
        Some(if little {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let bytes: [u8; 4] = tiff.get(at..at + 4)?.try_into().ok()?;
        Some(if little {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        })
    };
    let ifd = u32_at(4)? as usize;
    let count = u16_at(ifd)? as usize;
    for index in 0..count {
        let entry = ifd + 2 + index * 12;
        if u16_at(entry)? == tag {
            let length = u32_at(entry + 4)? as usize;
            if length <= 4 {
                return tiff.get(entry + 8..entry + 8 + length);
            }
            let offset = u32_at(entry + 8)? as usize;
            return tiff.get(offset..offset.checked_add(length)?);
        }
    }
    None
}

/// A little-endian TIFF block with the sticker JSON as its only entry, laid
/// out the way WhatsApp's own stickers are.
fn tiff_block(json: &[u8]) -> Vec<u8> {
    let mut tiff = Vec::with_capacity(22 + json.len());
    tiff.extend_from_slice(b"II*\0");
    tiff.extend_from_slice(&8u32.to_le_bytes());
    tiff.extend_from_slice(&1u16.to_le_bytes());
    tiff.extend_from_slice(&STICKER_TAG.to_le_bytes());
    // Type 7 is UNDEFINED: raw bytes.
    tiff.extend_from_slice(&7u16.to_le_bytes());
    tiff.extend_from_slice(&(json.len() as u32).to_le_bytes());
    // WhatsApp's own stickers put the JSON right after the entry, at 22,
    // without the next-directory pointer.
    tiff.extend_from_slice(&22u32.to_le_bytes());
    tiff.extend_from_slice(json);
    tiff
}

/// The canvas size and alpha flag of a simple (VP8 or VP8L) WebP bitstream.
fn simple_canvas(chunk: &Chunk<'_>) -> Option<(u32, u32, bool)> {
    let data = chunk.data;
    match &chunk.id {
        b"VP8 " => {
            // A key frame: 3-byte tag, start code, then 14-bit sizes.
            if data.get(3..6)? != [0x9d, 0x01, 0x2a] {
                return None;
            }
            let width = u16::from_le_bytes(data.get(6..8)?.try_into().ok()?) & 0x3fff;
            let height = u16::from_le_bytes(data.get(8..10)?.try_into().ok()?) & 0x3fff;
            Some((u32::from(width), u32::from(height), false))
        }
        b"VP8L" => {
            if *data.first()? != 0x2f {
                return None;
            }
            let bits = u32::from_le_bytes(data.get(1..5)?.try_into().ok()?);
            let width = (bits & 0x3fff) + 1;
            let height = ((bits >> 14) & 0x3fff) + 1;
            let alpha = (bits >> 28) & 1 == 1;
            Some((width, height, alpha))
        }
        _ => None,
    }
}

fn push_chunk(out: &mut Vec<u8>, id: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() & 1 == 1 {
        out.push(0);
    }
}

/// Returns the WebP with `info` as its sticker metadata, replacing any EXIF
/// chunk it had. A simple WebP becomes an extended one, since only that
/// layout carries metadata. `None` when the bytes are not a WebP it can read.
pub fn write(bytes: &[u8], info: &StickerInfo) -> Option<Vec<u8>> {
    let chunks = chunks(bytes)?;
    let json = serde_json::to_vec(info).ok()?;
    let exif = tiff_block(&json);
    let mut body = Vec::with_capacity(bytes.len() + exif.len() + 32);
    if let Some(header) = chunks.iter().find(|chunk| &chunk.id == b"VP8X") {
        let mut header = header.data.to_vec();
        // Bit 3 of the flags says an EXIF chunk follows.
        *header.first_mut()? |= 0x08;
        push_chunk(&mut body, b"VP8X", &header);
        for chunk in chunks
            .iter()
            .filter(|chunk| !matches!(&chunk.id, b"VP8X" | b"EXIF"))
        {
            push_chunk(&mut body, &chunk.id, chunk.data);
        }
    } else {
        let image = chunks.first()?;
        let (width, height, alpha) = simple_canvas(image)?;
        let mut header = [0u8; 10];
        header[0] = 0x08 | if alpha { 0x10 } else { 0 };
        header[4..7].copy_from_slice(&(width - 1).to_le_bytes()[..3]);
        header[7..10].copy_from_slice(&(height - 1).to_le_bytes()[..3]);
        push_chunk(&mut body, b"VP8X", &header);
        push_chunk(&mut body, &image.id, image.data);
    }
    push_chunk(&mut body, b"EXIF", &exif);
    let mut out = Vec::with_capacity(body.len() + 12);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(&body);
    Some(out)
}

/// Keeps emojis only, one per entry, dropping text and repeats.
pub fn clean_emojis(text: &str) -> Vec<String> {
    use unicode_segmentation::UnicodeSegmentation;
    let mut found: Vec<String> = Vec::new();
    for grapheme in text.graphemes(true) {
        if emojis::get(grapheme).is_some() && !found.iter().any(|seen| seen == grapheme) {
            found.push(grapheme.to_owned());
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lossless(width: u32, height: u32, alpha: u8) -> Vec<u8> {
        let picture = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, alpha]));
        let mut out = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut out)
            .encode(&picture, width, height, image::ExtendedColorType::Rgba8)
            .expect("encodes");
        out
    }

    fn info() -> StickerInfo {
        StickerInfo {
            pack_id: "zapfast.test".into(),
            pack_name: "Ducks".into(),
            publisher: String::new(),
            emojis: vec!["🦆".into(), "😂".into()],
        }
    }

    #[test]
    fn metadata_written_to_a_simple_webp_reads_back_and_still_decodes() {
        let plain = lossless(7, 5, 128);
        assert!(read(&plain).is_none());
        let tagged = write(&plain, &info()).expect("writes");
        assert_eq!(read(&tagged), Some(info()));
        assert_eq!(emojis(&tagged), vec!["🦆", "😂"]);
        let decoded = image::load_from_memory(&tagged).expect("still a picture");
        assert_eq!((decoded.width(), decoded.height()), (7, 5));
        assert_eq!(
            decoded.to_rgba8().get_pixel(0, 0).0[3],
            128,
            "alpha survives"
        );
    }

    #[test]
    fn writing_again_replaces_the_old_metadata() {
        let tagged = write(&lossless(4, 4, 255), &info()).expect("writes");
        let mut other = info();
        other.emojis = vec!["🐸".into()];
        let again = write(&tagged, &other).expect("writes");
        assert_eq!(emojis(&again), vec!["🐸"]);
        let exif_chunks = chunks(&again)
            .expect("webp")
            .iter()
            .filter(|chunk| &chunk.id == b"EXIF")
            .count();
        assert_eq!(exif_chunks, 1);
        image::load_from_memory(&again).expect("still a picture");
    }

    #[test]
    fn whatsapps_own_layout_is_read() {
        // The EXIF WhatsApp's sticker makers write: a TIFF entry at offset 22.
        let json = r#"{"sticker-pack-id":"x","emojis":["❤"]}"#.as_bytes();
        let mut body = Vec::new();
        push_chunk(&mut body, b"EXIF", &tiff_block(json));
        let mut file = b"RIFF\0\0\0\0WEBP".to_vec();
        file.extend(body);
        assert_eq!(emojis(&file), vec!["❤"]);
        assert!(read(b"not a webp").is_none());
    }

    #[test]
    fn typed_emojis_are_kept_once_and_text_is_dropped() {
        assert_eq!(clean_emojis("😂 lol 😂🐸 ❤️"), vec!["😂", "🐸", "❤️"]);
    }
}
