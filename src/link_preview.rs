//! Link previews ZapFast fetches itself.
//!
//! WhatsApp usually attaches preview metadata to a message, and the protocol
//! carries it in `extendedTextMessage`. It does not always: a sender can
//! disable link previews, and a link typed on a device that failed to reach
//! the page arrives bare. This module fills that gap the way the official
//! clients do, by reading the page's own OpenGraph and Twitter card tags.
//!
//! The parser is a plain function over `&str`, with no UI and no network, so
//! it is covered by fixtures in the tests below. Fetching lives in
//! [`fetch`], which the worker calls off the interface thread.

use std::sync::OnceLock;
use std::time::Duration;

/// Longest page body read for its metadata. A preview is in the first
/// kilobytes of `<head>`; a cap keeps a hostile or endless response from
/// filling memory.
const MAX_HTML_BYTES: usize = 512 * 1024;
/// Longest preview image read. Bigger images are skipped rather than
/// downscaled: the card shows them at most a few hundred points wide.
const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;
/// A page that takes longer than this shows no preview at all.
const TIMEOUT: Duration = Duration::from_secs(10);
/// The card draws a thumbnail, so a redirect chain longer than this is a
/// sign the URL is not a page.
const MAX_REDIRECTS: usize = 5;
/// Preview text longer than this is cut: the card is two lines tall.
const MAX_TITLE: usize = 200;
const MAX_DESCRIPTION: usize = 400;

/// What a page says about itself, as ZapFast shows it in a bubble.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preview {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    /// The page's own picture address, before it is fetched.
    pub image_url: Option<String>,
    /// The decoded picture bytes, when the page offered one ZapFast could
    /// read within its size cap.
    pub image: Option<Vec<u8>>,
}

impl Preview {
    /// Whether the page said anything worth a card.
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.description.is_none() && self.image.is_none()
    }

    /// The preview as the card draws it, with the picture as bytes.
    #[cfg(test)]
    fn image_address(&self) -> Option<&str> {
        self.image_url.as_deref()
    }
}

/// Reads the page at `url` and returns what it says about itself.
///
/// `base` is the URL the body was finally served from, so a relative
/// `og:image` resolves against the page that answered rather than the address
/// the user pasted. Anything that cannot be fetched, or that says nothing,
/// is `None` rather than an error the interface has to render: a missing
/// preview is not a failure the reader needs to see.
pub async fn fetch(url: &str) -> Option<Preview> {
    let base = safety_url(url)?;
    let body = get_text(&base).await?;
    let mut preview = parse(&body, &base);
    if let Some(source) = preview.image_url.take() {
        preview.image = get_image(&source).await;
    }
    (!preview.is_empty()).then_some(preview)
}

/// The address ZapFast is willing to fetch: an ordinary web address, with the
/// same validation the preview card already applies before opening a link.
fn safety_url(url: &str) -> Option<String> {
    crate::safety::preview_url(url)
}

/// Reads at most [`MAX_HTML_BYTES`] of a page as text.
async fn get_text(url: &str) -> Option<String> {
    let response = client()?.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    if let Some(length) = response.content_length()
        && length > MAX_HTML_BYTES as u64
    {
        return None;
    }
    let body = read_capped(response, MAX_HTML_BYTES).await?;
    Some(String::from_utf8_lossy(&body).into_owned())
}

/// Reads the picture at `url`, if it is one ZapFast can draw.
async fn get_image(url: &str) -> Option<Vec<u8>> {
    crate::safety::preview_url(url)?;
    let response = client()?.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    if let Some(length) = response.content_length()
        && length > MAX_IMAGE_BYTES as u64
    {
        return None;
    }
    let bytes = read_capped(response, MAX_IMAGE_BYTES).await?;
    // A page that names a picture it cannot serve still answers here, with an
    // error page. Only real image bytes are worth a card.
    image::guess_format(&bytes).is_ok().then_some(bytes)
}

/// Streams a response body, stopping at `limit` so an endless answer cannot
/// grow without bound.
async fn read_capped(mut response: reqwest::Response, limit: usize) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if body.len() + chunk.len() > limit {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    Some(body)
}

/// The shared client, with a UA that names the reader and a redirect cap.
fn client() -> Option<&'static reqwest::Client> {
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            let mut builder = reqwest::Client::builder()
                .user_agent(concat!("ZapFast/", env!("CARGO_PKG_VERSION")))
                .timeout(TIMEOUT)
                .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS));
            // `None` leaves reqwest reading the environment's own proxy.
            if let Some(proxy) = crate::proxy::reqwest_proxy() {
                builder = builder.proxy(proxy);
            }
            builder.build().ok()
        })
        .as_ref()
}

/// Reads preview metadata out of a page body.
///
/// `base` resolves relative addresses, as [`fetch`] documents.
pub fn parse(html: &str, base: &str) -> Preview {
    let title = meta(html, "property", "og:title")
        .or_else(|| meta(html, "name", "twitter:title"))
        .or_else(|| document_title(html))
        .map(|title| clip(&title, MAX_TITLE));
    let description = meta(html, "property", "og:description")
        .or_else(|| meta(html, "name", "twitter:description"))
        .or_else(|| meta(html, "name", "description"))
        .map(|text| clip(&text, MAX_DESCRIPTION));
    let image_url = meta(html, "property", "og:image")
        .or_else(|| meta(html, "name", "twitter:image"))
        .or_else(|| meta(html, "name", "twitter:image:src"))
        .and_then(|source| absolute(base, &source))
        .filter(|url| crate::safety::preview_url(url).is_some());
    Preview {
        url: base.to_owned(),
        title: title.filter(|value| !value.is_empty()),
        description: description.filter(|value| !value.is_empty()),
        image_url: image_url.filter(|url| !url.is_empty()),
        image: None,
    }
}

/// The `content` of the first `<meta>` tag with this attribute and value.
///
/// Attribute order varies between sites, so the tag is read as a set of
/// attributes rather than by slicing at fixed offsets. `property` and `name`
/// are both used for OpenGraph and Twitter cards, and a page may spell the
/// same tag either way.
fn meta(html: &str, attribute: &str, value: &str) -> Option<String> {
    let wanted = value.to_ascii_lowercase();
    let mut at = 0;
    while let Some(open) = html[at..].find('<') {
        let tag_start = at + open;
        let after = tag_name(&html[tag_start + 1..]);
        let Some((name, rest)) = after else {
            at = tag_start + 1;
            continue;
        };
        if !name.eq_ignore_ascii_case("meta") {
            at = tag_start + 1;
            continue;
        }
        let end = rest.find('>')?;
        let attributes = attributes(&rest[..end]);
        if attributes
            .iter()
            .any(|(key, content)| key == attribute && content.eq_ignore_ascii_case(&wanted))
            && let Some((_, content)) = attributes.iter().find(|(key, _)| key == "content")
        {
            let decoded = decode_entities(content);
            if !decoded.trim().is_empty() {
                return Some(decoded);
            }
        }
        at = tag_start + 1 + end + 1;
    }
    None
}

/// The name of the tag at the start of `text`, and what follows it.
fn tag_name(text: &str) -> Option<(&str, &str)> {
    let name: usize = text
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .unwrap_or(text.len());
    (name > 0).then(|| (&text[..name], &text[name..]))
}

/// The text of the document's `<title>`, the fallback OpenGraph leaves out.
fn document_title(html: &str) -> Option<String> {
    let rest = &html[html.to_ascii_lowercase().find("<title>")? + "<title>".len()..];
    let end = rest.to_ascii_lowercase().find("</title>")?;
    Some(decode_entities(&rest[..end]))
}

/// The `name="value"` pairs of one tag, unquoted and lowercased.
fn attributes(tag: &str) -> Vec<(String, String)> {
    let bytes = tag.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !(bytes[i] as char).is_ascii_alphanumeric() && bytes[i] != b'-' && bytes[i] != b':' {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len()
            && ((bytes[i] as char).is_ascii_alphanumeric()
                || matches!(bytes[i], b'-' | b'_' | b':' | b'.'))
        {
            i += 1;
        }
        let name = tag[start..i].to_ascii_lowercase();
        while i < bytes.len() && (bytes[i] as char).is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && (bytes[i] as char).is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let quote = bytes[i];
        // A double quote is 34 and an apostrophe is 39; comparing the byte
        // keeps both quote characters out of this file's literals.
        let value = if quote == 34 || quote == 39 {
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            let value = &tag[start..i.min(tag.len())];
            i += 1;
            value
        } else {
            let start = i;
            while i < bytes.len() && !(bytes[i] as char).is_ascii_whitespace() {
                i += 1;
            }
            &tag[start..i]
        };
        out.push((name, decode_entities(value)));
    }
    out
}

/// The character references a title or description actually uses.
fn decode_entities(value: &str) -> String {
    // The five named entities below are written as escapes rather than
    // literals: a double quote and an apostrophe inside a character literal
    // are easy to mangle in transit, and `\u{22}` / `\u{27}` cannot be.
    const AMP: char = '&';
    const QUOTE: char = '\u{22}';
    const APOSTROPHE: char = '\u{27}';
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(at) = rest.find(AMP) {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';').filter(|end| *end <= 12) else {
            out.push(AMP);
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some(AMP),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some(QUOTE),
            "apos" | "#39" => Some(APOSTROPHE),
            "nbsp" => Some(' '),
            _ => entity
                .strip_prefix('#')
                .and_then(|digits| match digits.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => digits.parse().ok(),
                })
                .and_then(char::from_u32),
        };
        match decoded {
            Some(character) => {
                out.push(character);
                rest = &rest[end + 1..];
            }
            None => {
                out.push(AMP);
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Resolves a possibly relative address against the page that served it.
fn absolute(base: &str, source: &str) -> Option<String> {
    let source = source.trim();
    if source.is_empty() {
        return None;
    }
    let resolved = match reqwest::Url::parse(source) {
        Ok(url) => url,
        Err(_) => reqwest::Url::parse(base).ok()?.join(source).ok()?,
    };
    Some(resolved.to_string())
}

/// Trims a value to `max` characters, on a character boundary.
fn clip(value: &str, max: usize) -> String {
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if value.chars().count() <= max {
        return value;
    }
    value.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(head: &str) -> String {
        format!("<!doctype html><html><head>{head}</head><body>ignored</body></html>")
    }

    #[test]
    fn reads_opengraph_tags_in_any_attribute_order() {
        let html = page(
            r#"<meta content="A title" property="og:title">
               <meta property="og:description" content="A description">
               <meta property="og:image" content="/card.png">"#,
        );
        let preview = parse(&html, "https://example.com/a/b");
        assert_eq!(preview.title.as_deref(), Some("A title"));
        assert_eq!(preview.description.as_deref(), Some("A description"));
        // Relative to the page, not to the host root.
        assert_eq!(
            preview.image_address(),
            Some("https://example.com/card.png")
        );
    }

    #[test]
    fn single_quoted_and_uppercase_tags_are_read() {
        let html = page("<META PROPERTY='OG:TITLE' CONTENT='Shouty'>");
        assert_eq!(
            parse(&html, "https://example.com").title.as_deref(),
            Some("Shouty")
        );
    }

    #[test]
    fn twitter_cards_stand_in_for_opengraph() {
        let html = page(
            r#"<meta name="twitter:title" content="T">
               <meta name="twitter:description" content="D">
               <meta name="twitter:image" content="https://cdn.example/i.png">"#,
        );
        let preview = parse(&html, "https://example.com");
        assert_eq!(preview.title.as_deref(), Some("T"));
        assert_eq!(preview.description.as_deref(), Some("D"));
        assert_eq!(preview.image_address(), Some("https://cdn.example/i.png"));
    }

    #[test]
    fn the_document_title_is_the_last_fallback() {
        let html = page("<title>Just a title</title>");
        assert_eq!(
            parse(&html, "https://example.com").title.as_deref(),
            Some("Just a title")
        );
    }

    #[test]
    fn character_references_are_decoded() {
        let html = page(
            r#"<meta property="og:title" content="Tom &amp; Jerry &#8212; &quot;cartoon&quot;">
               <meta property="og:description" content="caf&eacute; &#233;">"#,
        );
        let preview = parse(&html, "https://example.com");
        assert_eq!(preview.title.as_deref(), Some("Tom & Jerry — \"cartoon\""));
        // `&eacute;` is a named entity this parser does not know, so it is
        // left as written rather than guessed at.
        assert_eq!(preview.description.as_deref(), Some("caf&eacute; é"));
    }

    #[test]
    fn an_ampersand_without_an_entity_survives() {
        assert_eq!(decode_entities("Bell & Howell"), "Bell & Howell");
        assert_eq!(decode_entities("a&notanentity;b"), "a&notanentity;b");
    }

    #[test]
    fn a_page_that_says_nothing_yields_nothing() {
        let preview = parse(&page(""), "https://example.com");
        assert!(preview.is_empty());
        assert_eq!(preview.url, "https://example.com");
    }

    #[test]
    fn blank_tags_fall_through_to_the_next_source() {
        let html = page(
            r#"<meta property="og:title" content="   ">
               <meta name="twitter:title" content="Real">
               <meta property="og:description" content="">
               <meta name="description" content="Also real">"#,
        );
        let preview = parse(&html, "https://example.com");
        assert_eq!(preview.title.as_deref(), Some("Real"));
        assert_eq!(preview.description.as_deref(), Some("Also real"));
    }

    #[test]
    fn a_desktop_scheme_image_is_refused() {
        let html = page(r#"<meta property="og:image" content="file:///etc/passwd">"#);
        assert_eq!(parse(&html, "https://example.com").image_address(), None);
    }

    #[test]
    fn long_metadata_is_clipped_to_the_card() {
        let long = "x".repeat(MAX_TITLE + 50);
        let html = page(&format!(r#"<meta property="og:title" content="{long}">"#));
        let title = parse(&html, "https://example.com").title.unwrap();
        assert_eq!(title.chars().count(), MAX_TITLE);
    }

    #[test]
    fn whitespace_in_metadata_is_collapsed() {
        let html = page("<meta property=\"og:title\" content=\"a\n\n  b\">");
        assert_eq!(
            parse(&html, "https://example.com").title.as_deref(),
            Some("a b")
        );
    }

    #[test]
    fn relative_addresses_resolve_against_the_page() {
        assert_eq!(
            absolute("https://example.com/a/b", "c.png").as_deref(),
            Some("https://example.com/a/c.png")
        );
        assert_eq!(
            absolute("https://example.com/a/b", "/c.png").as_deref(),
            Some("https://example.com/c.png")
        );
        assert_eq!(
            absolute("https://example.com/a/b", "https://other.example/c.png").as_deref(),
            Some("https://other.example/c.png")
        );
        assert_eq!(absolute("https://example.com", "   "), None);
    }

    /// Serves canned bodies for one request per path, on a loopback port.
    fn serve(replies: Vec<(&'static str, &'static str)>) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds");
        let base = format!("http://{}", listener.local_addr().expect("has an address"));
        std::thread::spawn(move || {
            for (path, body) in replies {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buffer = [0u8; 1024];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                assert!(
                    request.starts_with(&format!("GET {path} ")),
                    "unexpected request: {request}"
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        base
    }

    #[tokio::test]
    async fn a_page_is_read_for_its_own_metadata() {
        let base = serve(vec![(
            "/page",
            r#"<html><head><meta property="og:title" content="A page">
               <meta property="og:description" content="What it says">
               <meta property="og:image" content="/card.png"></head></html>"#,
        )]);
        let preview = fetch(&format!("{base}/page"))
            .await
            .expect("the page is read");
        assert_eq!(preview.url, format!("{base}/page"));
        assert_eq!(preview.title.as_deref(), Some("A page"));
        assert_eq!(preview.description.as_deref(), Some("What it says"));
        // The picture is fetched too, and this server has nothing at that
        // path, so no image is claimed.
        assert_eq!(preview.image, None);
    }

    #[tokio::test]
    async fn a_page_that_says_nothing_produces_no_preview() {
        let base = serve(vec![("/page", "<html><head></head><body>hi</body></html>")]);
        assert_eq!(fetch(&format!("{base}/page")).await, None);
    }

    #[tokio::test]
    async fn a_page_that_never_answers_produces_no_preview() {
        // A port nothing listens on: the address is well formed, the page is
        // not there, and the reader must not wait on it.
        assert_eq!(fetch("http://127.0.0.1:1/page").await, None);
    }

    #[tokio::test]
    async fn a_desktop_address_is_never_fetched() {
        assert_eq!(fetch("file:///etc/passwd").await, None);
    }

    #[test]
    fn only_web_addresses_are_fetched() {
        assert!(safety_url("https://example.com/page").is_some());
        assert!(safety_url("example.com/page").is_some());
        assert_eq!(safety_url("file:///etc/passwd"), None);
        assert_eq!(safety_url("javascript:alert(1)"), None);
        assert_eq!(safety_url("mailto:a@example.com"), None);
        assert_eq!(safety_url("not a url at all"), None);
    }
}
