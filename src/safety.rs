//! Validation at the boundary between message content and desktop handlers.

use std::path::Path;

/// Preview metadata may supply a bare host, but never a desktop URI scheme.
pub fn preview_url(value: &str) -> Option<String> {
    if value.chars().any(char::is_control) || value.contains('\\') {
        return None;
    }
    let value = value.trim();
    let url = match reqwest::Url::parse(value) {
        Ok(url) => url,
        Err(_) => reqwest::Url::parse(&format!("https://{value}")).ok()?,
    };
    (matches!(url.scheme(), "http" | "https") && url.host_str().is_some()).then(|| url.to_string())
}

/// Check again at the action boundary, including previews already in archives.
pub fn external_url(value: &str) -> Option<String> {
    if value.chars().any(char::is_control) || value.contains('\\') {
        return None;
    }
    let url = reqwest::Url::parse(value).ok()?;
    match url.scheme() {
        "http" | "https" if url.host_str().is_some() => Some(url.to_string()),
        "mailto" if !url.path().is_empty() => Some(url.to_string()),
        _ => None,
    }
}

/// Image formats decoded by ZapFast's in-process preview.
pub fn can_preview_image(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "jpg" | "jpeg" | "png" | "gif" | "webp"
    )
}

/// The code of a WhatsApp group invite link such as
/// `https://chat.whatsapp.com/AbCd123`, which ZapFast opens itself.
pub fn group_invite_code(value: &str) -> Option<String> {
    let url = reqwest::Url::parse(value.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.host_str()?.eq_ignore_ascii_case("chat.whatsapp.com")
    {
        return None;
    }
    let mut segments = url.path_segments()?.filter(|segment| !segment.is_empty());
    let first = segments.next()?;
    let code = if first == "invite" {
        segments.next()?
    } else {
        first
    };
    (segments.next().is_none()
        && (10..=40).contains(&code.len())
        && code
            .chars()
            .all(|character| character.is_ascii_alphanumeric()))
    .then(|| code.to_owned())
}

/// Only common document/media formats go to their desktop application.
/// Unknown files, scripts, installers and application bundles can be revealed
/// in their folder instead. Sender-provided MIME types cannot grant permission.
pub fn can_open_attachment(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "jpg"
            | "jpeg"
            | "png"
            | "gif"
            | "webp"
            | "bmp"
            | "tif"
            | "tiff"
            | "heic"
            | "heif"
            | "avif"
            | "mp4"
            | "m4v"
            | "mov"
            | "webm"
            | "mkv"
            | "3gp"
            | "mp3"
            | "m4a"
            | "aac"
            | "ogg"
            | "opus"
            | "wav"
            | "flac"
            | "pdf"
            | "txt"
            | "log"
            | "csv"
            | "docx"
            | "xlsx"
            | "pptx"
            | "odt"
            | "ods"
            | "odp"
            | "rtf"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_invite_links_are_recognised_and_nothing_else() {
        let code = "AbCdEf1234567890XyZ";
        for link in [
            format!("https://chat.whatsapp.com/{code}"),
            format!("https://chat.whatsapp.com/invite/{code}"),
            format!("http://CHAT.whatsapp.com/{code}/"),
            format!("https://chat.whatsapp.com/{code}?utm=1"),
        ] {
            assert_eq!(group_invite_code(&link).as_deref(), Some(code), "{link}");
        }
        for link in [
            "https://chat.whatsapp.com/",
            "https://chat.whatsapp.com/short",
            "https://evil.example/AbCdEf1234567890XyZ",
            "https://chat.whatsapp.com.evil.example/AbCdEf1234567890XyZ",
            "https://chat.whatsapp.com/AbCdEf1234567890XyZ/extra",
            "https://chat.whatsapp.com/AbCd-Ef1234567890XyZ",
            "whatsapp://chat.whatsapp.com/AbCdEf1234567890XyZ",
        ] {
            assert_eq!(group_invite_code(link), None, "{link}");
        }
    }

    #[test]
    fn previews_and_actions_reject_desktop_handlers() {
        for value in [
            "file:///tmp/example",
            "ms-appinstaller://example",
            "javascript:alert(1)",
            "data:text/html,example",
            "shell:AppsFolder",
            "https:\\example.com",
            "https://example.com\n",
            "mailto:test@example.com",
        ] {
            assert_eq!(preview_url(value), None, "{value}");
            if !value.starts_with("mailto:") {
                assert_eq!(external_url(value), None, "{value}");
            }
        }
        assert_eq!(
            preview_url("example.com/page"),
            Some("https://example.com/page".into())
        );
        assert_eq!(
            preview_url("HTTPS://example.com"),
            Some("https://example.com/".into())
        );
        assert_eq!(
            external_url("mailto:test@example.com"),
            Some("mailto:test@example.com".into())
        );
    }

    #[test]
    fn only_supported_raster_images_open_in_the_native_preview() {
        for name in [
            "photo.JPG",
            "photo.jpeg",
            "photo.png",
            "photo.webp",
            "photo.gif",
        ] {
            assert!(can_preview_image(Path::new(name)), "{name}");
        }
        for name in [
            "photo.bmp",
            "photo.heic",
            "photo.avif",
            "photo.jpg.exe",
            "photo",
        ] {
            assert!(!can_preview_image(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn only_recognized_document_and_media_extensions_can_launch() {
        for name in ["photo.JPG", "document.pdf", "voice.ogg", "sheet.xlsx"] {
            assert!(can_open_attachment(Path::new(name)), "{name}");
        }
        for name in [
            "invoice.pdf.exe",
            "setup.msi",
            "script.ps1",
            "script.js",
            "script.bat",
            "script.cmd",
            "script.vbs",
            "screen.scr",
            "link.lnk",
            "link.url",
            "app.desktop",
            "app.AppImage",
            "app.app",
            "script.command",
            "no-extension",
            "macro.docm",
            "file.pdf ",
            "file.pdf:payload.exe",
            "file.unknown",
        ] {
            assert!(!can_open_attachment(Path::new(name)), "{name}");
        }
    }
}
