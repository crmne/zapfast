//! Sticker-pack import from signal.art links and .wastickers archives.
//!
//! Imported packs become named directories of WebP files.

use std::io::Read;
use std::path::{Path, PathBuf};

/// Parses a pack id and key from a signal.art URL or parameter string.
pub fn parse_signal_url(url: &str) -> Result<(String, [u8; 32]), String> {
    let tail = url.rsplit('#').next().unwrap_or(url);
    let mut id = None;
    let mut key = None;
    for part in tail.split(['&', '?', ';']) {
        if let Some(value) = part.strip_prefix("pack_id=") {
            id = Some(value.trim().to_lowercase());
        }
        if let Some(value) = part.strip_prefix("pack_key=") {
            key = Some(value.trim().to_lowercase());
        }
    }
    let id = id
        .filter(|id| id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or("Missing pack_id. Copy the full signal.art link")?;
    let key = key
        .and_then(|key| hex_bytes(&key))
        .ok_or("Missing or invalid pack_key. Copy the full signal.art link")?;
    Ok((id, key))
}

fn hex_bytes(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}

/// Whether pasted text contains a complete signal.art pack link.
pub fn looks_like_signal_url(text: &str) -> bool {
    parse_signal_url(text).is_ok()
}

/// Decrypts one Signal sticker file using HKDF, HMAC-SHA256, and AES-256-CBC.
pub fn decrypt_blob(payload: &[u8], pack_key: &[u8; 32]) -> Result<Vec<u8>, String> {
    use aes::cipher::{BlockModeDecrypt, KeyInit, KeyIvInit, block_padding::Pkcs7};
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    if payload.len() < 16 + 16 + 32 {
        return Err("The sticker data is incomplete".to_owned());
    }
    let mut keys = [0u8; 64];
    hkdf::Hkdf::<Sha256>::new(Some(&[0u8; 32]), pack_key)
        .expand(b"Sticker Pack", &mut keys)
        .map_err(|_| "Could not derive the sticker key".to_owned())?;
    let (aes_key, mac_key) = keys.split_at(32);
    let (iv, rest) = payload.split_at(16);
    let (ciphertext, mac) = rest.split_at(rest.len() - 32);
    let mut hmac = <Hmac<Sha256> as KeyInit>::new_from_slice(mac_key)
        .map_err(|_| "Could not derive the sticker key".to_owned())?;
    hmac.update(iv);
    hmac.update(ciphertext);
    hmac.verify_slice(mac)
        .map_err(|_| "The key does not match this pack".to_owned())?;
    let iv: [u8; 16] = iv.try_into().expect("split at 16");
    let aes_key: [u8; 32] = aes_key.try_into().expect("split at 32");
    cbc::Decryptor::<aes::Aes256>::new(&aes_key.into(), &iv.into())
        .decrypt_padded_vec::<Pkcs7>(ciphertext)
        .map_err(|_| "Could not decrypt the sticker pack".to_owned())
}

/// One sticker in a Signal manifest: its id and the emoji it expresses.
#[derive(Clone, Debug, PartialEq)]
pub struct ManifestSticker {
    pub id: u64,
    pub emoji: Option<String>,
}

/// Reads the pack title and stickers from the small manifest protobuf.
pub fn parse_manifest(bytes: &[u8]) -> Result<(String, Vec<ManifestSticker>), String> {
    let mut title = String::new();
    let mut ids = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        let (field, wire) = read_tag(bytes, &mut pos)?;
        match (field, wire) {
            (1, 2) => {
                title = String::from_utf8_lossy(read_chunk(bytes, &mut pos)?).into_owned();
            }
            (4, 2) => {
                let sticker = read_chunk(bytes, &mut pos)?;
                let mut inner = 0;
                let mut id = None;
                let mut emoji = None;
                while inner < sticker.len() {
                    let (field, wire) = read_tag(sticker, &mut inner)?;
                    match (field, wire) {
                        (1, 0) => id = Some(read_varint(sticker, &mut inner)?),
                        (2, 2) => {
                            let text = read_chunk(sticker, &mut inner)?;
                            emoji = Some(String::from_utf8_lossy(text).trim().to_owned())
                                .filter(|emoji| !emoji.is_empty());
                        }
                        _ => skip_field(sticker, &mut inner, wire)?,
                    }
                }
                if let Some(id) = id {
                    ids.push(ManifestSticker { id, emoji });
                }
            }
            _ => skip_field(bytes, &mut pos, wire)?,
        }
    }
    Ok((title, ids))
}

fn read_varint(bytes: &[u8], pos: &mut usize) -> Result<u64, String> {
    let mut value = 0u64;
    for shift in 0..10 {
        let byte = *bytes
            .get(*pos)
            .ok_or("The sticker manifest is incomplete".to_owned())?;
        *pos += 1;
        value |= u64::from(byte & 0x7f) << (shift * 7);
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err("The sticker manifest contains an invalid number".to_owned())
}

fn read_tag(bytes: &[u8], pos: &mut usize) -> Result<(u64, u64), String> {
    let tag = read_varint(bytes, pos)?;
    Ok((tag >> 3, tag & 7))
}

fn read_chunk<'a>(bytes: &'a [u8], pos: &mut usize) -> Result<&'a [u8], String> {
    let length = read_varint(bytes, pos)? as usize;
    let end = pos
        .checked_add(length)
        .filter(|end| *end <= bytes.len())
        .ok_or("The sticker manifest is incomplete".to_owned())?;
    let chunk = &bytes[*pos..end];
    *pos = end;
    Ok(chunk)
}

fn skip_field(bytes: &[u8], pos: &mut usize, wire: u64) -> Result<(), String> {
    match wire {
        0 => {
            read_varint(bytes, pos)?;
        }
        1 => *pos += 8,
        2 => {
            read_chunk(bytes, pos)?;
        }
        5 => *pos += 4,
        _ => return Err("The sticker manifest contains an unknown field".to_owned()),
    }
    if *pos > bytes.len() {
        return Err("The sticker manifest is incomplete".to_owned());
    }
    Ok(())
}

/// Signal's pinned certificate authority for the sticker CDN, valid to 2032.
const SIGNAL_CA: &str = "-----BEGIN CERTIFICATE-----\n\
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\n\
9qfgAJizyy3kqM4lOwBH\n\
-----END CERTIFICATE-----";

/// HTTP client that trusts only Signal's sticker CDN authority.
fn signal_agent() -> Result<ureq::Agent, String> {
    use ureq::tls::{Certificate, RootCerts, TlsConfig};
    let ca = Certificate::from_pem(SIGNAL_CA.as_bytes())
        .map_err(|error| format!("Could not read the Signal certificate: {error}"))?;
    let tls = TlsConfig::builder()
        .root_certs(RootCerts::new_with_certs(&[ca]))
        .build();
    Ok(ureq::Agent::config_builder()
        .tls_config(tls)
        .proxy(crate::proxy::ureq_proxy())
        .build()
        .into())
}

fn fetch(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, String> {
    agent
        .get(url)
        .call()
        .map_err(|error| format!("signal.art request failed: {error}"))?
        .body_mut()
        .read_to_vec()
        .map_err(|error| format!("Could not read the signal.art response: {error}"))
}

/// Downloads a signal.art pack into a directory named after its title.
pub fn import_signal_pack(url: &str, packs: &Path) -> Result<String, String> {
    let (id, key) = parse_signal_url(url)?;
    let agent = signal_agent()?;
    let manifest = decrypt_blob(
        &fetch(
            &agent,
            &format!("https://cdn.signal.org/stickers/{id}/manifest.proto"),
        )?,
        &key,
    )?;
    let (title, stickers) = parse_manifest(&manifest)?;
    if stickers.is_empty() {
        return Err("This pack contains no stickers".to_owned());
    }
    let ids: Vec<u64> = stickers.iter().map(|sticker| sticker.id).collect();
    // Download several of the pack's files concurrently.
    let mut blobs: Vec<Option<Vec<u8>>> = vec![None; ids.len()];
    let lane = ids.len().div_ceil(6).max(1);
    std::thread::scope(|scope| {
        for (chunk, out) in ids.chunks(lane).zip(blobs.chunks_mut(lane)) {
            let id = &id;
            let key = &key;
            let agent = &agent;
            scope.spawn(move || {
                for (sticker, slot) in chunk.iter().zip(out) {
                    if let Ok(payload) = fetch(
                        agent,
                        &format!("https://cdn.signal.org/stickers/{id}/full/{sticker}"),
                    ) && let Ok(bytes) = decrypt_blob(&payload, key)
                    {
                        *slot = Some(bytes);
                    }
                }
            });
        }
    });
    // Keep each sticker's emoji in the file, where search and WhatsApp read it.
    let files: Vec<Vec<u8>> = blobs
        .into_iter()
        .zip(&stickers)
        .filter_map(|(blob, sticker)| {
            let webp = webp_bytes(blob?)?;
            let info = crate::sticker_meta::StickerInfo {
                pack_id: format!("signal.{id}"),
                pack_name: title.clone(),
                emojis: sticker.emoji.iter().cloned().collect(),
                ..Default::default()
            };
            Some(crate::sticker_meta::write(&webp, &info).unwrap_or(webp))
        })
        .collect();
    write_pack(packs, &title, files)
}

/// Imports a .wastickers or zip archive. Uses `title.txt` or the filename as title.
pub fn import_archive(path: &Path, packs: &Path) -> Result<String, String> {
    let file =
        std::fs::File::open(path).map_err(|error| format!("Could not open the file: {error}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| format!("This file is not a sticker archive: {error}"))?;
    let mut title: Option<String> = None;
    let mut pictures: Vec<(String, Vec<u8>)> = Vec::new();
    for index in 0..archive.len() {
        let Ok(mut entry) = archive.by_index(index) else {
            continue;
        };
        let name = entry.name().to_lowercase();
        let mut bytes = Vec::new();
        if entry.read_to_end(&mut bytes).is_err() {
            continue;
        }
        if name.ends_with("title.txt") {
            let text = String::from_utf8_lossy(&bytes).trim().to_owned();
            if !text.is_empty() {
                title = Some(text);
            }
            continue;
        }
        let picture = [".webp", ".png", ".jpg", ".jpeg"]
            .iter()
            .any(|extension| name.ends_with(extension));
        // Exclude the pack thumbnail.
        if picture && !name.contains("tray") {
            pictures.push((name, bytes));
        }
    }
    pictures.sort_by(|a, b| a.0.cmp(&b.0));
    let title = title
        .or_else(|| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    let files: Vec<Vec<u8>> = pictures
        .into_iter()
        .filter_map(|(_, bytes)| webp_bytes(bytes))
        .collect();
    write_pack(packs, &title, files)
}

/// Unpacks a WhatsApp sticker pack zip into `dir`, in the pack's order and
/// without its tray icon, writing each sticker's emojis into its metadata.
/// `stickers` names each file with its emojis, as the pack message lists them.
pub fn extract_whatsapp_pack(
    zip: &[u8],
    stickers: &[(String, Vec<String>)],
    tray: Option<&str>,
    name: &str,
    dir: &Path,
) -> Result<Vec<PathBuf>, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip))
        .map_err(|error| format!("This is not a sticker pack: {error}"))?;
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for index in 0..archive.len() {
        let Ok(mut entry) = archive.by_index(index) else {
            continue;
        };
        let file = entry.name().to_owned();
        if Some(file.as_str()) == tray || !file.to_lowercase().ends_with(".webp") {
            continue;
        }
        let mut bytes = Vec::new();
        if entry.read_to_end(&mut bytes).is_ok() {
            files.push((file, bytes));
        }
    }
    // The message's order first; files it does not list follow.
    let rank = |file: &str| {
        stickers
            .iter()
            .position(|(listed, _)| listed == file)
            .unwrap_or(usize::MAX)
    };
    files.sort_by_key(|(file, _)| rank(file));
    if files.is_empty() {
        return Err("No stickers could be read from this pack".to_owned());
    }
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let mut written = Vec::new();
    for (index, (file, bytes)) in files.into_iter().enumerate() {
        let emojis = stickers
            .iter()
            .find(|(listed, _)| *listed == file)
            .map(|(_, emojis)| emojis.clone())
            .unwrap_or_default();
        let bytes = if emojis.is_empty() {
            bytes
        } else {
            let info = crate::sticker_meta::StickerInfo {
                pack_name: name.to_owned(),
                emojis,
                ..crate::sticker_meta::read(&bytes).unwrap_or_default()
            };
            crate::sticker_meta::write(&bytes, &info).unwrap_or(bytes)
        };
        let path = dir.join(format!("{index:03}.webp"));
        std::fs::write(&path, bytes).map_err(|error| error.to_string())?;
        written.push(path);
    }
    Ok(written)
}

/// Copies a pack's stickers into a new pack folder named after it, and
/// returns the folder's name.
pub fn copy_pack(from: &Path, packs: &Path, name: &str) -> Result<String, String> {
    let mut stickers: Vec<PathBuf> = std::fs::read_dir(from)
        .map_err(|error| error.to_string())?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "webp")
        })
        .collect();
    stickers.sort();
    let files = stickers
        .iter()
        .map(std::fs::read)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    write_pack(packs, name, files)
}

/// A pack's tray icon (a 96-pixel WebP) and thumbnail (a 252-pixel JPEG on
/// white), both drawn from its first sticker.
pub fn pack_art(first: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let picture = image::load_from_memory(first).ok()?.to_rgba8();
    let tray = image::imageops::resize(&picture, 96, 96, image::imageops::FilterType::Lanczos3);
    let mut cover = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut cover)
        .encode(&tray, 96, 96, image::ExtendedColorType::Rgba8)
        .ok()?;
    let side = 252;
    let small =
        image::imageops::resize(&picture, side, side, image::imageops::FilterType::Lanczos3);
    let mut flat = image::RgbImage::from_pixel(side, side, image::Rgb([255, 255, 255]));
    for (x, y, pixel) in small.enumerate_pixels() {
        let alpha = f32::from(pixel[3]) / 255.0;
        let under = flat.get_pixel(x, y).0;
        let mixed = std::array::from_fn(|channel| {
            (f32::from(pixel[channel]) * alpha + f32::from(under[channel]) * (1.0 - alpha)).round()
                as u8
        });
        flat.put_pixel(x, y, image::Rgb(mixed));
    }
    let mut thumbnail = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut thumbnail, 85)
        .encode_image(&flat)
        .ok()?;
    Some((cover, thumbnail))
}

/// Writes pack images to a new directory named after the title.
fn write_pack(packs: &Path, title: &str, files: Vec<Vec<u8>>) -> Result<String, String> {
    if files.is_empty() {
        return Err("No stickers could be read from this pack".to_owned());
    }
    let dir = unique_pack_dir(packs, title)?;
    for (index, bytes) in files.iter().enumerate() {
        std::fs::write(dir.join(format!("{index:03}.webp")), bytes)
            .map_err(|error| format!("Could not write the sticker pack: {error}"))?;
    }
    Ok(dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| title.to_owned()))
}

/// Converts an image to WebP. Preserves WebP, converts animated APNG/GIF to
/// animated WebP, and scales other images to 512 pixels per side.
fn webp_bytes(bytes: Vec<u8>) -> Option<Vec<u8>> {
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some(bytes);
    }
    if let Some(frames) = animation_frames(&bytes) {
        return encode_animated(frames);
    }
    let picture = image::load_from_memory(&bytes).ok()?;
    let picture = if picture.width().max(picture.height()) > 512 {
        picture.resize(512, 512, image::imageops::FilterType::Lanczos3)
    } else {
        picture
    };
    let mut out = Vec::new();
    let picture = picture.to_rgba8();
    image::codecs::webp::WebPEncoder::new_lossless(&mut out)
        .encode(
            &picture,
            picture.width(),
            picture.height(),
            image::ExtendedColorType::Rgba8,
        )
        .ok()?;
    Some(out)
}

/// Returns frames and millisecond delays for animated APNG or GIF data.
fn animation_frames(bytes: &[u8]) -> Option<Vec<(image::RgbaImage, u32)>> {
    use image::AnimationDecoder;
    let cursor = std::io::Cursor::new(bytes);
    let frames = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        let decoder = image::codecs::png::PngDecoder::new(cursor).ok()?;
        if !decoder.is_apng().ok()? {
            return None;
        }
        decoder.apng().ok()?.into_frames()
    } else if bytes.starts_with(b"GIF8") {
        image::codecs::gif::GifDecoder::new(cursor)
            .ok()?
            .into_frames()
    } else {
        return None;
    };
    let frames: Vec<(image::RgbaImage, u32)> = frames
        .take(200)
        .filter_map(|frame| frame.ok())
        .map(|frame| {
            let (numerator, denominator) = frame.delay().numer_denom_ms();
            (frame.into_buffer(), numerator / denominator.max(1))
        })
        .collect();
    (frames.len() > 1).then_some(frames)
}

/// Encodes frames as a lossy animated WebP at sticker size.
fn encode_animated(frames: Vec<(image::RgbaImage, u32)>) -> Option<Vec<u8>> {
    use webp_animation::prelude::*;
    let (source_width, source_height) = frames.first().map(|(frame, _)| frame.dimensions())?;
    let scale = (512.0 / f64::from(source_width.max(source_height))).min(1.0);
    let width = (f64::from(source_width) * scale).round().max(1.0) as u32;
    let height = (f64::from(source_height) * scale).round().max(1.0) as u32;
    let mut encoder = Encoder::new_with_options(
        (width, height),
        EncoderOptions {
            encoding_config: Some(EncodingConfig {
                quality: 80.0,
                encoding_type: EncodingType::Lossy(LossyEncodingConfig::default()),
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .ok()?;
    let mut clock = 0i32;
    for (frame, delay) in frames {
        let frame = if frame.dimensions() == (width, height) {
            frame
        } else {
            image::imageops::resize(&frame, width, height, image::imageops::FilterType::Lanczos3)
        };
        encoder.add_frame(frame.as_raw(), clock).ok()?;
        clock += delay.clamp(10, 10_000) as i32;
    }
    Some(encoder.finalize(clock).ok()?.to_vec())
}

/// Creates a unique pack directory from a sanitized title.
pub(super) fn unique_pack_dir(root: &Path, title: &str) -> Result<PathBuf, String> {
    let clean: String = title
        .trim()
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':'))
        .take(60)
        .collect();
    let base = if clean.trim().is_empty() {
        "Stickers".to_owned()
    } else {
        clean.trim().to_owned()
    };
    for attempt in 0..100 {
        let name = if attempt == 0 {
            base.clone()
        } else {
            format!("{base} {}", attempt + 1)
        };
        let dir = root.join(&name);
        if !dir.exists() {
            std::fs::create_dir_all(&dir)
                .map_err(|error| format!("Could not create the pack folder: {error}"))?;
            return Ok(dir);
        }
    }
    Err("Too many sticker packs have this name".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signal_link_gives_up_its_id_and_key() {
        let url = format!(
            "https://signal.art/addstickers/#pack_id={}&pack_key={}",
            "0123456789abcdef0123456789abcdef",
            "aa".repeat(32)
        );
        let (id, key) = parse_signal_url(&url).expect("parses");
        assert_eq!(id, "0123456789abcdef0123456789abcdef");
        assert_eq!(key, [0xaa; 32]);
        assert!(looks_like_signal_url(&url));
        assert!(!looks_like_signal_url("https://signal.art/addstickers/"));
        assert!(!looks_like_signal_url("pack_id=zz&pack_key=short"));
    }

    #[test]
    fn a_pack_blob_decrypts_back_to_its_content() {
        use aes::cipher::{BlockModeEncrypt, KeyInit, KeyIvInit, block_padding::Pkcs7};
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let pack_key = [7u8; 32];
        let mut keys = [0u8; 64];
        hkdf::Hkdf::<Sha256>::new(Some(&[0u8; 32]), &pack_key)
            .expand(b"Sticker Pack", &mut keys)
            .expect("expands");
        let (aes_key, mac_key) = keys.split_at(32);
        let aes_key: [u8; 32] = aes_key.try_into().expect("32");
        let iv = [9u8; 16];
        let plain = b"RIFF....WEBP pretend sticker bytes";
        let ciphertext = cbc::Encryptor::<aes::Aes256>::new(&aes_key.into(), &iv.into())
            .encrypt_padded_vec::<Pkcs7>(plain);
        let mut hmac = <Hmac<Sha256> as KeyInit>::new_from_slice(mac_key).expect("keys");
        hmac.update(&iv);
        hmac.update(&ciphertext);
        let mac = hmac.finalize().into_bytes();
        let payload: Vec<u8> = iv
            .iter()
            .chain(ciphertext.iter())
            .chain(mac.iter())
            .copied()
            .collect();
        assert_eq!(decrypt_blob(&payload, &pack_key).expect("opens"), plain);
        let mut wrong = payload.clone();
        *wrong.last_mut().expect("bytes") ^= 1;
        assert!(
            decrypt_blob(&wrong, &pack_key).is_err(),
            "a bad MAC refuses"
        );
    }

    #[test]
    fn the_manifest_yields_title_and_sticker_ids() {
        // StickerPack { title: "Ducks", author: "A", stickers: [{id:0},{id:1,emoji:"x"}] }
        let mut bytes = vec![0x0a, 5];
        bytes.extend(b"Ducks");
        bytes.extend([0x12, 1]);
        bytes.extend(b"A");
        bytes.extend([0x22, 2, 0x08, 0]);
        bytes.extend([0x22, 5, 0x08, 1, 0x12, 1, b'x']);
        let (title, stickers) = parse_manifest(&bytes).expect("parses");
        assert_eq!(title, "Ducks");
        assert_eq!(
            stickers,
            vec![
                ManifestSticker { id: 0, emoji: None },
                ManifestSticker {
                    id: 1,
                    emoji: Some("x".to_owned())
                },
            ]
        );
    }

    #[test]
    fn a_wastickers_archive_becomes_a_named_pack() {
        let root = std::env::temp_dir().join(format!("zapfast-packs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let archive_path = root.join("in").join("Frogs.wastickers");
        std::fs::create_dir_all(archive_path.parent().expect("dir")).expect("dirs");
        let webp = webp_bytes(tiny_png()).expect("encodes");
        let file = std::fs::File::create(&archive_path).expect("creates");
        let mut writer = zip::ZipWriter::new(file);
        let plain: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
        writer.start_file("title.txt", plain).expect("entry");
        std::io::Write::write_all(&mut writer, b"Happy Frogs\n").expect("writes");
        writer.start_file("tray.png", plain).expect("entry");
        std::io::Write::write_all(&mut writer, &tiny_png()).expect("writes");
        writer.start_file("02.webp", plain).expect("entry");
        std::io::Write::write_all(&mut writer, &webp).expect("writes");
        writer.start_file("01.png", plain).expect("entry");
        std::io::Write::write_all(&mut writer, &tiny_png()).expect("writes");
        writer.finish().expect("finishes");
        let packs = root.join("packs");
        let title = import_archive(&archive_path, &packs).expect("imports");
        assert_eq!(title, "Happy Frogs");
        let mut files: Vec<String> = std::fs::read_dir(packs.join("Happy Frogs"))
            .expect("lists")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        files.sort();
        assert_eq!(
            files,
            vec!["000.webp", "001.webp"],
            "tray and title stay out"
        );
        // Reimporting creates a separate numbered directory.
        let again = import_archive(&archive_path, &packs).expect("imports");
        assert_eq!(again, "Happy Frogs 2");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_apng_keeps_its_motion_as_animated_webp() {
        let mut apng = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut apng, 4, 4);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_animated(2, 0).expect("animated");
            encoder.set_frame_delay(1, 10).expect("delay");
            let mut writer = encoder.write_header().expect("header");
            writer.write_image_data(&[10u8; 4 * 4 * 4]).expect("frame");
            writer.write_image_data(&[200u8; 4 * 4 * 4]).expect("frame");
            writer.finish().expect("finishes");
        }
        let webp = webp_bytes(apng).expect("converts");
        let decoder =
            image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(&webp)).expect("is a webp");
        assert!(decoder.has_animation(), "the motion survives");
    }

    #[test]
    fn an_animated_gif_keeps_its_motion_too() {
        let mut gif = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut gif);
            let frames = [10u8, 200].map(|shade| {
                image::Frame::from_parts(
                    image::RgbaImage::from_pixel(4, 4, image::Rgba([shade, 0, 0, 255])),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(100, 1),
                )
            });
            encoder.encode_frames(frames).expect("encodes");
        }
        let webp = webp_bytes(gif).expect("converts");
        let decoder =
            image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(&webp)).expect("is a webp");
        assert!(decoder.has_animation(), "the motion survives");
    }

    #[test]
    fn a_whatsapp_pack_unpacks_in_order_with_its_emojis() {
        use whatsapp_rust::sticker_pack::{StickerInput, create_sticker_pack_zip};
        let red = webp_bytes(tiny_png()).expect("encodes");
        let blue = {
            let mut bytes = Vec::new();
            let picture = image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 255, 255]));
            image::codecs::webp::WebPEncoder::new_lossless(&mut bytes)
                .encode(&picture, 4, 4, image::ExtendedColorType::Rgba8)
                .expect("encodes");
            bytes
        };
        let (cover, thumbnail) = pack_art(&red).expect("draws");
        assert!(image::load_from_memory(&cover).is_ok());
        assert_eq!(
            image::guess_format(&thumbnail).expect("known"),
            image::ImageFormat::Jpeg
        );
        let inputs = [
            StickerInput::new(&blue).with_emojis(vec!["🐸".into()]),
            StickerInput::new(&red),
        ];
        let zip = create_sticker_pack_zip("pack1", &inputs, &cover).expect("zips");
        let listed: Vec<(String, Vec<String>)> = zip
            .stickers
            .iter()
            .map(|sticker| {
                (
                    sticker.file_name.clone().expect("named"),
                    sticker.emojis.clone(),
                )
            })
            .collect();
        let root = tempfile::tempdir().expect("temp");
        let dir = root.path().join("shared").join("pack1");
        let files = extract_whatsapp_pack(
            &zip.zip_bytes,
            &listed,
            Some(&zip.tray_icon_file_name),
            "Frogs",
            &dir,
        )
        .expect("unpacks");
        assert_eq!(files.len(), 2, "the tray icon stays out");
        let first = std::fs::read(&files[0]).expect("reads");
        assert_eq!(crate::sticker_meta::emojis(&first), vec!["🐸"]);
        assert_eq!(
            image::load_from_memory(&first)
                .expect("decodes")
                .to_rgba8()
                .get_pixel(0, 0)
                .0,
            [0, 0, 255, 255],
            "the pack's order is kept"
        );
        let packs = root.path().join("packs");
        assert_eq!(copy_pack(&dir, &packs, "Frogs").expect("copies"), "Frogs");
        assert_eq!(
            std::fs::read_dir(packs.join("Frogs"))
                .expect("lists")
                .count(),
            2
        );
    }

    /// Imports Signal's example pack from the live CDN. `ZAPFAST_TEST_PACK`
    /// can provide another signal.art link.
    #[test]
    #[ignore = "network"]
    fn fetches_a_real_pack_from_signal_on_this_machine() {
        let root = std::env::temp_dir().join(format!("zapfast-signal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let url = std::env::var("ZAPFAST_TEST_PACK").unwrap_or_else(|_| {
            "https://signal.art/addstickers/#pack_id=9acc9e8aba563d26a4994e69263e3b25&pack_key=5a6dff3948c28efb9b7aaf93ecc375c69fc316e78077ed26867a14d10a0f6a12"
                .to_owned()
        });
        let title = import_signal_pack(&url, &root).expect("imports");
        let mut files = 0;
        let mut moving = 0;
        for entry in std::fs::read_dir(root.join(&title))
            .expect("lists")
            .flatten()
        {
            files += 1;
            let head = std::fs::read(entry.path()).expect("reads");
            if head[..head.len().min(64)]
                .windows(4)
                .any(|window| window == b"ANIM")
            {
                moving += 1;
            }
        }
        eprintln!("pack {title:?}: {files} stickers, {moving} of them animated");
        assert!(files > 0, "the pack holds stickers");
        let _ = std::fs::remove_dir_all(root);
    }

    /// Verifies converted frames apply animation disposal correctly.
    #[test]
    fn a_moving_square_converts_without_a_trace() {
        let side = 64u32;
        let mut apng = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut apng, side, side);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_animated(2, 0).expect("animated");
            encoder.set_frame_delay(1, 10).expect("delay");
            let mut writer = encoder.write_header().expect("header");
            let mut frame1 = vec![0u8; (side * side * 4) as usize];
            let mut frame2 = frame1.clone();
            for y in 0..16u32 {
                for x in 0..16u32 {
                    let a = ((y * side + x) * 4) as usize;
                    frame1[a..a + 4].copy_from_slice(&[255, 0, 0, 255]);
                    let b = (((y + 40) * side + x + 40) * 4) as usize;
                    frame2[b..b + 4].copy_from_slice(&[0, 255, 0, 255]);
                }
            }
            writer.write_image_data(&frame1).expect("frame");
            writer.write_image_data(&frame2).expect("frame");
            writer.finish().expect("finishes");
        }
        let webp = webp_bytes(apng).expect("converts");
        let frames: Vec<_> = webp_animation::Decoder::new(&webp)
            .expect("decodes")
            .into_iter()
            .collect();
        assert_eq!(frames.len(), 2);
        let at = |frame: &webp_animation::Frame, x: u32, y: u32| {
            let p = ((y * side + x) * 4) as usize;
            frame.data()[p + 3]
        };
        assert_eq!(at(&frames[1], 8, 8), 0, "the old square is gone");
        assert!(at(&frames[1], 48, 48) > 200, "the new square shows");
        assert_eq!(at(&frames[0], 48, 48), 0, "frame one starts clean");
    }

    fn tiny_png() -> Vec<u8> {
        use image::ImageEncoder;
        let mut bytes = Vec::new();
        let picture = image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 255, 0, 255]));
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&picture, 4, 4, image::ExtendedColorType::Rgba8)
            .expect("encodes");
        bytes
    }
}
