//! Read a page's title, description and picture from its HTML head, and
//! refuse hosts that point at the reader's own machine or network.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

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
                out.push(c);
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
    let [a, b, ..] = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        // 100.64.0.0/10, carrier-grade NAT.
        || (a == 100 && b & 0xc0 == 64)
}

fn local_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    ip.is_loopback()
        || ip.is_unspecified()
        // fc00::/7 unique-local and fe80::/10 link-local.
        || first & 0xfe00 == 0xfc00
        || first & 0xffc0 == 0xfe80
        || ip.to_ipv4_mapped().is_some_and(local_v4)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

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
        ] {
            assert!(is_local_ip(ip.parse::<IpAddr>().unwrap()), "{ip}");
        }
        for ip in ["1.1.1.1", "2606:4700::1111", "100.128.0.1"] {
            assert!(!is_local_ip(ip.parse::<IpAddr>().unwrap()), "{ip}");
        }
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
