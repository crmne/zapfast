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
