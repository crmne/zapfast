//! Read a page's title, description and picture from its HTML head, and
//! refuse hosts that point at the reader's own machine or network.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::Duration;

use crate::model::{ComposedPreview, LinkPreview};

const USER_AGENT: &str = concat!("ZapFast/", env!("CARGO_PKG_VERSION"));
/// Page and picture together.
const TOTAL_TIME: Duration = Duration::from_secs(5);
const MAX_PAGE: usize = 512 * 1024;
const MAX_IMAGE: usize = 2 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
const THUMB_SIDE: u32 = 300;
const MIN_SIDE: u32 = 16;
const JPEG_QUALITY: u8 = 70;
const TITLE_CHARS: usize = 300;
const DESCRIPTION_CHARS: usize = 600;

/// What a page says about itself, for the preview under an outgoing link.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct PageMeta {
    pub title: Option<String>,
    pub description: Option<String>,
    /// Already resolved against the page's address; always http or https.
    pub image: Option<String>,
}

/// Read the metadata from the part of `html` before `</head>` or `<body`.
/// Open Graph wins over Twitter cards, which win over the plain tags.
pub(super) fn parse_page(html: &str, base: &reqwest::Url) -> PageMeta {
    // ASCII lowercasing keeps byte offsets, so searches on `lower` index `html`.
    let lower = html.to_ascii_lowercase();
    let end = ["</head>", "<body"]
        .iter()
        .filter_map(|marker| lower.find(marker))
        .min()
        .unwrap_or(html.len());

    let mut metas: Vec<(String, String)> = Vec::new();
    let mut title_tag = None;
    let mut at = 0;
    while let Some(found) = lower[at..end].find('<') {
        let start = at + found;
        at = start + 1;
        let rest = &lower[start..end];
        if tag_is(rest, "<meta") {
            let (attrs, used) = attributes(&html[start + 5..end]);
            at = start + 5 + used;
            let mut key = None;
            let mut content = None;
            for (name, value) in attrs {
                match name.as_str() {
                    "property" | "name" if key.is_none() => key = Some(value),
                    "content" if content.is_none() => content = Some(value),
                    _ => {}
                }
            }
            if let (Some(key), Some(content)) = (key, content) {
                metas.push((key.trim().to_ascii_lowercase(), content));
            }
        } else if title_tag.is_none() && tag_is(rest, "<title") {
            let Some(open) = rest.find('>') else { break };
            let body = start + open + 1;
            // An unclosed title runs to the end of the head.
            let close = lower[body..end].find("</title").map_or(end, |n| body + n);
            title_tag = Some(decode(&html[body..close]));
            at = close;
        }
    }

    let first = |keys: &[&str], limit: usize| {
        keys.iter().find_map(|wanted| {
            metas
                .iter()
                .filter(|(key, _)| key == wanted)
                .find_map(|(_, value)| clean(value, limit))
        })
    };
    let title = first(&["og:title", "twitter:title"], TITLE_CHARS)
        .or_else(|| title_tag.and_then(|text| clean(&text, TITLE_CHARS)));
    let description = first(
        &["og:description", "twitter:description", "description"],
        DESCRIPTION_CHARS,
    );
    let image = ["og:image", "twitter:image"].iter().find_map(|wanted| {
        metas
            .iter()
            .filter(|(key, _)| key == wanted)
            .find_map(|(_, value)| resolve(value, base))
    });
    PageMeta {
        title,
        description,
        image,
    }
}

/// Whether `rest` starts with the tag `name` followed by a name boundary.
fn tag_is(rest: &str, name: &str) -> bool {
    rest.strip_prefix(name)
        .and_then(|after| after.chars().next())
        .is_some_and(|c| c.is_ascii_whitespace() || c == '>' || c == '/')
}

/// The attributes of a tag whose name has been read, with lowercase names and
/// decoded values, and how many bytes of `input` the tag took.
fn attributes(input: &str) -> (Vec<(String, String)>, usize) {
    let bytes = input.as_bytes();
    let skip_space = |mut i: usize| {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'>' => return (out, i + 1),
            c if c.is_ascii_whitespace() || c == b'/' => i += 1,
            _ => {
                let start = i;
                // A name always takes its first byte, even a stray '='.
                i += 1;
                while i < bytes.len()
                    && !bytes[i].is_ascii_whitespace()
                    && !matches!(bytes[i], b'=' | b'>' | b'/')
                {
                    i += 1;
                }
                let name = input[start..i].to_ascii_lowercase();
                i = skip_space(i);
                if bytes.get(i) != Some(&b'=') {
                    continue;
                }
                i = skip_space(i + 1);
                let value = match bytes.get(i) {
                    Some(&quote @ (b'"' | b'\'')) => {
                        let from = i + 1;
                        let to = input[from..]
                            .find(quote as char)
                            .map_or(bytes.len(), |n| from + n);
                        i = (to + 1).min(bytes.len());
                        &input[from..to]
                    }
                    _ => {
                        let from = i;
                        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>'
                        {
                            i += 1;
                        }
                        let value = &input[from..i];
                        // A self-closing slash ends the tag, not the value.
                        if bytes.get(i) == Some(&b'>') {
                            value.strip_suffix('/').unwrap_or(value)
                        } else {
                            value
                        }
                    }
                };
                out.push((name, decode(value)));
            }
        }
    }
    (out, bytes.len())
}

/// Collapse whitespace and cap at `limit` characters; `None` if nothing is left.
fn clean(text: &str, limit: usize) -> Option<String> {
    let words = text.split_whitespace().collect::<Vec<_>>().join(" ");
    // `take` cuts on a character boundary.
    let capped: String = words.chars().take(limit).collect();
    let capped = capped.trim_end().to_owned();
    (!capped.is_empty()).then_some(capped)
}

/// Replace the named entities pages use and numeric references.
fn decode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let decoded = rest[1..]
            .find(';')
            .filter(|&n| n <= 10)
            .and_then(|n| entity(&rest[1..1 + n]).map(|c| (c, n + 2)));
        match decoded {
            Some((c, used)) => {
                // `&#0;` and the like would put control characters in a title.
                if !c.is_control() {
                    out.push(c);
                }
                rest = &rest[used..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        "ndash" => Some('\u{2013}'),
        "mdash" => Some('\u{2014}'),
        "hellip" => Some('\u{2026}'),
        "lsquo" => Some('\u{2018}'),
        "rsquo" => Some('\u{2019}'),
        "ldquo" => Some('\u{201c}'),
        "rdquo" => Some('\u{201d}'),
        "laquo" => Some('\u{ab}'),
        "raquo" => Some('\u{bb}'),
        "middot" => Some('\u{b7}'),
        "bull" => Some('\u{2022}'),
        "copy" => Some('\u{a9}'),
        "reg" => Some('\u{ae}'),
        "trade" => Some('\u{2122}'),
        "euro" => Some('\u{20ac}'),
        "pound" => Some('\u{a3}'),
        "yen" => Some('\u{a5}'),
        "deg" => Some('\u{b0}'),
        "times" => Some('\u{d7}'),
        "eacute" => Some('\u{e9}'),
        "egrave" => Some('\u{e8}'),
        "aacute" => Some('\u{e1}'),
        "agrave" => Some('\u{e0}'),
        "uuml" => Some('\u{fc}'),
        "ouml" => Some('\u{f6}'),
        "auml" => Some('\u{e4}'),
        "Uuml" => Some('\u{dc}'),
        "Ouml" => Some('\u{d6}'),
        "Auml" => Some('\u{c4}'),
        "szlig" => Some('\u{df}'),
        "ccedil" => Some('\u{e7}'),
        "ntilde" => Some('\u{f1}'),
        _ => {
            let digits = name.strip_prefix('#')?;
            let code = match digits.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => digits.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// The picture's address against the page's, if it is a web address.
fn resolve(value: &str, base: &reqwest::Url) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let url = base.join(value).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.into())
}

/// Whether `ip` is the machine itself or a private, link-local or shared
/// address, which a preview fetch must never reach.
pub(super) fn is_local_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => local_v4(ip),
        IpAddr::V6(ip) => local_v6(ip),
    }
}

fn local_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_multicast()
        // 240.0.0.0/4 reserved, which includes the broadcast address.
        || a >= 240
        // 100.64.0.0/10, carrier-grade NAT.
        || (a == 100 && b & 0xc0 == 64)
        // 192.0.0.0/24, protocol assignments.
        || (a == 192 && b == 0 && c == 0)
        // 198.18.0.0/15, benchmarking.
        || (a == 198 && b & 0xfe == 18)
}

fn local_v6(ip: Ipv6Addr) -> bool {
    let seg = ip.segments();
    let first = seg[0];
    let embedded = |high: u16, low: u16| {
        let [a, b] = high.to_be_bytes();
        let [c, d] = low.to_be_bytes();
        local_v4(Ipv4Addr::new(a, b, c, d))
    };
    ip.is_loopback()
        || ip.is_unspecified()
        // fc00::/7 unique-local, fe80::/10 link-local and fec0::/10 site-local.
        || first & 0xfe00 == 0xfc00
        || first & 0xffc0 == 0xfe80
        || first & 0xffc0 == 0xfec0
        // ff00::/8 multicast.
        || first & 0xff00 == 0xff00
        || ip.to_ipv4_mapped().is_some_and(local_v4)
        // ::a.b.c.d, the deprecated IPv4-compatible form.
        || seg[..6] == [0; 6]
        // 64:ff9b::/96 (NAT64) and 2002::/16 (6to4) reach their embedded address.
        || (seg[..6] == [0x64, 0xff9b, 0, 0, 0, 0] && embedded(seg[6], seg[7]))
        || (first == 0x2002 && embedded(seg[1], seg[2]))
}

/// `localhost`, `*.localhost` and `*.local` (mDNS), in any case, with or
/// without the root dot.
pub(super) fn is_local_name(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local")
}

/// Whether a link may be fetched going by its text alone: a web address whose
/// host is not a local name or a local address. Names that resolve to local
/// addresses are the fetcher's to catch.
pub(super) fn literal_host_allowed(url: &reqwest::Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    // IPv6 literals come bracketed.
    match host.trim_matches(['[', ']']).parse::<IpAddr>() {
        Ok(ip) => !is_local_ip(ip),
        Err(_) => !is_local_name(host),
    }
}

/// Shrink a picture to at most 300 px on its long side and encode it as JPEG.
/// Pictures too small to be worth showing, and anything undecodable, give `None`.
pub(super) fn thumbnail(bytes: &[u8]) -> Option<Vec<u8>> {
    let image = image::load_from_memory(bytes).ok()?;
    if image.width() < MIN_SIDE || image.height() < MIN_SIDE {
        return None;
    }
    // `thumbnail` would also enlarge a small picture.
    let image = if image.width() > THUMB_SIDE || image.height() > THUMB_SIDE {
        image.thumbnail(THUMB_SIDE, THUMB_SIDE)
    } else {
        image
    };
    // Transparent areas would turn black as plain RGB: lay them on white.
    let mut rgb = image::RgbImage::new(image.width(), image.height());
    for (x, y, pixel) in image.to_rgba8().enumerate_pixels() {
        let [r, g, b, a] = pixel.0;
        let over = |c: u8| ((u16::from(c) * u16::from(a) + 255 * (255 - u16::from(a))) / 255) as u8;
        rgb.put_pixel(x, y, image::Rgb([over(r), over(g), over(b)]));
    }
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
        .encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .ok()?;
    Some(out)
}

/// Resolves names itself so that a name pointing at the reader's own network
/// is refused, and the connection goes to the addresses that were checked.
struct GuardedResolver;

impl reqwest::dns::Resolve for GuardedResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let found: Vec<std::net::SocketAddr> =
                tokio::net::lookup_host((name.as_str(), 0)).await?.collect();
            if found.is_empty() || found.iter().any(|addr| is_local_ip(addr.ip())) {
                return Err("the address is not public".into());
            }
            Ok(Box::new(found.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// The body up to `limit` bytes, and whether more followed.
async fn read_body(mut response: reqwest::Response, limit: usize) -> Option<(Vec<u8>, bool)> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        body.extend_from_slice(&chunk);
        if body.len() > limit {
            body.truncate(limit);
            return Some((body, true));
        }
    }
    Some((body, false))
}

/// Makes SOCKS proxies resolve the target name themselves, so it is never
/// looked up on this computer. Credentials and other schemes are unchanged.
fn proxy_url_for_preview(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("socks5://") {
        format!("socks5h://{rest}")
    } else if let Some(rest) = url.strip_prefix("socks4://") {
        format!("socks4a://{rest}")
    } else {
        url.to_owned()
    }
}

fn client(url: &reqwest::Url) -> Option<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() > MAX_REDIRECTS || !literal_host_allowed(attempt.url()) {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .no_proxy();
    let proxied = if let Some(proxy) = crate::proxy::from_settings() {
        builder = builder.proxy(reqwest::Proxy::all(proxy_url_for_preview(proxy.url())).ok()?);
        true
    } else if let Some(proxy) = url.host_str().and_then(crate::proxy::from_env) {
        builder = builder.proxy(reqwest::Proxy::all(proxy_url_for_preview(proxy.url())).ok()?);
        true
    } else {
        false
    };
    if !proxied {
        builder = builder.dns_resolver(Arc::new(GuardedResolver));
    }
    builder.build().ok()
}

/// Read the page at `url` and, if it describes itself, its picture. Only the
/// linked site is contacted. Gives `None` on any failure or when the page says
/// nothing; nothing about the link is logged.
pub(super) async fn fetch(url: String) -> Option<ComposedPreview> {
    tokio::time::timeout(TOTAL_TIME, fetch_inner(url))
        .await
        .ok()
        .flatten()
}

async fn fetch_inner(url: String) -> Option<ComposedPreview> {
    // The preview names the address that was asked for, not where it led.
    let requested = crate::safety::preview_url(&url)?;
    let parsed = reqwest::Url::parse(&requested).ok()?;
    if !literal_host_allowed(&parsed) {
        return None;
    }
    let client = client(&parsed)?;

    let response = client.get(parsed).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let is_html = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("text/html")
        });
    if !is_html {
        return None;
    }
    let base = response.url().clone();
    let (bytes, _) = read_body(response, MAX_PAGE).await?;
    let meta = parse_page(&String::from_utf8_lossy(&bytes), &base);
    if meta.title.is_none() && meta.description.is_none() && meta.image.is_none() {
        return None;
    }

    let thumbnail = match meta.image.as_deref() {
        Some(image) => fetch_image(&client, image).await,
        None => None,
    };
    Some(ComposedPreview {
        preview: LinkPreview {
            url: requested,
            title: meta.title,
            description: meta.description,
        },
        thumbnail,
    })
}

async fn fetch_image(client: &reqwest::Client, url: &str) -> Option<Vec<u8>> {
    let url = reqwest::Url::parse(url).ok()?;
    // A literal address skips the resolver, so check it here.
    if !literal_host_allowed(&url) {
        return None;
    }
    let response = client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let (bytes, truncated) = read_body(response, MAX_IMAGE).await?;
    if truncated {
        return None;
    }
    tokio::task::spawn_blocking(move || thumbnail(&bytes))
        .await
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    #[test]
    fn common_named_entities_decode() {
        assert_eq!(
            decode("Caf&eacute; &ndash; M&Uuml;ller&hellip; &copy; 2026 &Auml; &auml;"),
            "Caf\u{e9} \u{2013} M\u{dc}ller\u{2026} \u{a9} 2026 \u{c4} \u{e4}"
        );
        assert_eq!(decode("&nosuch; &AMP;"), "&nosuch; &AMP;");
    }

    #[test]
    fn socks_proxies_resolve_names_remotely() {
        assert_eq!(
            proxy_url_for_preview("socks5://u:p@127.0.0.1:1080"),
            "socks5h://u:p@127.0.0.1:1080"
        );
        assert_eq!(
            proxy_url_for_preview("socks4://127.0.0.1:1080"),
            "socks4a://127.0.0.1:1080"
        );
        for kept in ["socks5h://u:p@h:1", "socks4a://h:1", "http://u:p@h:8080"] {
            assert_eq!(proxy_url_for_preview(kept), kept);
        }
    }

    fn base() -> reqwest::Url {
        reqwest::Url::parse("https://ex.com/a/b").unwrap()
    }

    #[test]
    fn parses_open_graph_first() {
        let html = r#"<html><head>
            <title>Tag title</title>
            <meta name="twitter:title" content="Twitter title">
            <meta name="description" content="Plain description">
            <meta name="twitter:description" content="Twitter description">
            <meta name="twitter:image" content="/twitter.png">
            <meta property="og:title" content="OG title">
            <meta property="og:description" content="OG description">
            <meta property="og:image" content="/og.png">
            </head><body></body></html>"#;
        let meta = parse_page(html, &base());
        assert_eq!(meta.title.as_deref(), Some("OG title"));
        assert_eq!(meta.description.as_deref(), Some("OG description"));
        assert_eq!(meta.image.as_deref(), Some("https://ex.com/og.png"));

        let html = r#"<head><title>Tag title</title>
            <meta name="twitter:title" content="Twitter title">
            <meta name="description" content="Plain description">
            <meta name="twitter:description" content="Twitter description">
            <meta name="twitter:image" content="/twitter.png"></head>"#;
        let meta = parse_page(html, &base());
        assert_eq!(meta.title.as_deref(), Some("Twitter title"));
        assert_eq!(meta.description.as_deref(), Some("Twitter description"));
        assert_eq!(meta.image.as_deref(), Some("https://ex.com/twitter.png"));

        let html = r#"<head><meta name="description" content="Plain description"></head>"#;
        let meta = parse_page(html, &base());
        assert_eq!(meta.description.as_deref(), Some("Plain description"));
    }

    #[test]
    fn falls_back_to_title_tag() {
        let meta = parse_page("<TITLE> Hello &amp; welcome </TITLE>", &base());
        assert_eq!(meta.title.as_deref(), Some("Hello & welcome"));
    }

    #[test]
    fn tolerates_attribute_order_and_quotes() {
        let meta = parse_page(
            r#"<META content='A &#39;b&#39;' property="og:title">"#,
            &base(),
        );
        assert_eq!(meta.title.as_deref(), Some("A 'b'"));
        let meta = parse_page("<meta property=og:title content=Plain/>", &base());
        assert_eq!(meta.title.as_deref(), Some("Plain"));
        let meta = parse_page(
            r#"<meta property="og:title" content="x &#x41; &lt;">"#,
            &base(),
        );
        assert_eq!(meta.title.as_deref(), Some("x A <"));
        let meta = parse_page(
            r#"<meta property="og:title" content="a&#0;b&#7;c&#x1b;d">"#,
            &base(),
        );
        assert_eq!(meta.title.as_deref(), Some("abcd"));
    }

    #[test]
    fn stops_at_the_body() {
        let html = r#"<head></head><body><meta property="og:title" content="late">"#;
        assert_eq!(parse_page(html, &base()).title, None);
    }

    #[test]
    fn collapses_whitespace_and_caps_length() {
        let html = format!(
            "<title>a\n\t  b</title><meta name=description content=\"{}\">",
            "é".repeat(700)
        );
        let meta = parse_page(&html, &base());
        assert_eq!(meta.title.as_deref(), Some("a b"));
        assert_eq!(meta.description.unwrap().chars().count(), 600);
        let html = format!("<title>{}</title>", "x".repeat(400));
        assert_eq!(
            parse_page(&html, &base()).title.unwrap().chars().count(),
            300
        );
    }

    #[test]
    fn resolves_relative_image() {
        let meta = parse_page(r#"<meta property="og:image" content="/img.png">"#, &base());
        assert_eq!(meta.image.as_deref(), Some("https://ex.com/img.png"));
        let meta = parse_page(
            r#"<meta property="og:image" content="javascript:x">"#,
            &base(),
        );
        assert_eq!(meta.image, None);
    }

    #[test]
    fn empty_values_are_none() {
        let meta = parse_page(r#"<meta property="og:title" content="">"#, &base());
        assert_eq!(meta.title, None);
        assert_eq!(meta.description, None);
        assert_eq!(meta.image, None);
    }

    #[test]
    fn survives_junk() {
        let mut html = "<".repeat(1 << 20);
        assert_eq!(parse_page(&html, &base()), PageMeta::default());
        let mut state = 0x2545_f491_4f6c_dd1du64;
        html.clear();
        while html.len() < 1 << 20 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            html.push(char::from_u32((state % 0x3000) as u32).unwrap_or('<'));
            if state.is_multiple_of(7) {
                html.push_str("<meta ");
            }
        }
        let meta = parse_page(&html, &base());
        assert_eq!(meta, PageMeta::default());
    }

    #[test]
    fn local_ips() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.74",
            "169.254.1.1",
            "0.0.0.0",
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:192.168.0.1",
            "100.64.0.1",
            "255.255.255.255",
            "224.0.0.1",
            "239.255.255.250",
            "240.0.0.1",
            "192.0.0.8",
            "198.18.0.1",
            "198.19.255.255",
            "ff02::1",
            "fec0::1",
            "::192.168.0.1",
            "::8.8.8.8",
            "64:ff9b::c0a8:101",
            "64:ff9b::7f00:1",
            "2002:c0a8:101::1",
            "2002:7f00:1::",
        ] {
            assert!(is_local_ip(ip.parse::<IpAddr>().unwrap()), "{ip}");
        }
        for ip in [
            "1.1.1.1",
            "2606:4700::1111",
            "100.128.0.1",
            "198.20.0.1",
            "192.0.1.1",
            "64:ff9b::808:808",
            "2002:808:808::1",
        ] {
            assert!(!is_local_ip(ip.parse::<IpAddr>().unwrap()), "{ip}");
        }
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbImage::from_fn(width, height, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        });
        let mut out = std::io::Cursor::new(Vec::new());
        image.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn decoded(jpeg: &[u8]) -> (image::ImageFormat, u32, u32) {
        let format = image::guess_format(jpeg).unwrap();
        let image = image::load_from_memory(jpeg).unwrap();
        (format, image.width(), image.height())
    }

    #[test]
    fn thumbnail_shrinks_to_300() {
        let out = thumbnail(&png(1200, 600)).unwrap();
        assert_eq!(decoded(&out), (image::ImageFormat::Jpeg, 300, 150));
    }

    #[test]
    fn thumbnail_keeps_small() {
        let out = thumbnail(&png(200, 100)).unwrap();
        assert_eq!(decoded(&out), (image::ImageFormat::Jpeg, 200, 100));
    }

    #[test]
    fn thumbnail_lays_transparency_on_white() {
        let clear = image::RgbaImage::from_pixel(100, 100, image::Rgba([0, 0, 0, 0]));
        let mut png = std::io::Cursor::new(Vec::new());
        clear.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let out = thumbnail(png.get_ref()).unwrap();
        let pixel = image::load_from_memory(&out).unwrap().to_rgb8()[(50, 50)];
        assert!(pixel.0.iter().all(|&c| c > 240), "{pixel:?}");
    }

    #[test]
    fn thumbnail_rejects_garbage_and_pixels() {
        assert_eq!(thumbnail(b"not an image"), None);
        assert_eq!(thumbnail(&png(1, 1)), None);
        assert_eq!(thumbnail(&png(15, 400)), None);
    }

    #[tokio::test]
    async fn fetch_refuses_local_without_network() {
        for url in ["http://127.0.0.1:9/", "http://printer.local/"] {
            let result =
                tokio::time::timeout(std::time::Duration::from_millis(100), fetch(url.into()))
                    .await
                    .expect("refused before any connection");
            assert!(result.is_none(), "{url}");
        }
    }

    #[tokio::test]
    async fn resolver_refuses_localhost() {
        use reqwest::dns::Resolve;
        let result = GuardedResolver.resolve("localhost".parse().unwrap()).await;
        assert!(result.is_err());
    }

    #[test]
    fn local_names() {
        for name in ["localhost", "LOCALHOST.", "a.localhost", "printer.local"] {
            assert!(is_local_name(name), "{name}");
        }
        for name in ["example.com", "localhost.example.com"] {
            assert!(!is_local_name(name), "{name}");
        }
    }

    #[test]
    fn literal_hosts() {
        let allowed = |url: &str| literal_host_allowed(&reqwest::Url::parse(url).unwrap());
        assert!(!allowed("http://192.168.1.1/"));
        assert!(!allowed("https://[::1]/"));
        assert!(!allowed("https://localhost/"));
        assert!(!allowed("ftp://ex.com/"));
        assert!(allowed("https://ex.com/"));
    }
}
