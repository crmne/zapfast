//! Keep protocol diagnostics useful without persisting untrusted payloads.

use std::borrow::Cow;

/// Protocol errors can embed JIDs, nodes, credentials and message contents.
/// Record only recognized failure categories, including in verbose logs.
///
/// Nothing from `message` is copied into the summary except an HTTP status,
/// which is rebuilt from its three digits: every other word is chosen here.
pub fn protocol_summary(message: &str) -> Cow<'static, str> {
    if message.contains("signature-mismatch") {
        "pairing signature verification failed".into()
    } else if message.contains("rate-overlimit") {
        "WhatsApp rate limit reached".into()
    } else if let Some(summary) = connect_summary(&message.to_ascii_lowercase()) {
        summary
    } else if let Some(summary) = app_state_summary(message) {
        summary
    } else {
        "protocol diagnostic (private details omitted)".into()
    }
}

/// Preserve only an allowlisted collection name and a fixed recovery stage.
/// The upstream lines may also carry account data or key-derived MACs.
fn app_state_summary(message: &str) -> Option<Cow<'static, str>> {
    for (wire, kind) in [
        ("regular_low", "RegularLow"),
        ("regular_high", "RegularHigh"),
        ("regular", "Regular"),
        ("critical_block", "CriticalBlock"),
        ("critical_unblock_low", "CriticalUnblockLow"),
    ] {
        let snapshot = message.starts_with(&format!("Snapshot {wire} v"));
        let stage = if snapshot && message.contains(" MAC mismatch over ") {
            "snapshot MAC mismatch"
        } else if snapshot && message.contains(" carries no MAC; refusing it rather than accepting unverified records") {
            "snapshot MAC missing"
        } else if snapshot && message.contains(" carries no key id; refusing it rather than accepting unverified records") {
            "snapshot key id missing"
        } else if message.starts_with(&format!("Asking the primary device to send back {kind}:")) {
            "recovery request attempted"
        } else if message.starts_with(&format!("Could not ask the primary to rebuild {kind}:")) {
            "recovery request failed"
        } else if message.starts_with(&format!("Recovered the {wire} collection from the primary device:")) {
            "recovery reply applied by library"
        } else if message.starts_with(&format!("Failed to apply the snapshot recovery for {wire}:")) {
            "recovery apply failed"
        } else if message.starts_with(&format!("Discarding the primary's {kind} at v"))
            && message.contains(": this side already holds v")
        {
            "recovery reply stale"
        } else if wire == "critical_block"
            && message.starts_with("Not asking the primary to rebuild CriticalBlock: the block list is not recovered this way")
        {
            "peer recovery unsupported"
        } else if message.starts_with(&format!("Not asking the primary to rebuild {kind}: the account has peer snapshot recovery turned off")) {
            "peer recovery disabled"
        } else {
            continue;
        };
        return Some(format!("app-state {wire} {stage}").into());
    }
    None
}

/// Names why a connection attempt failed, from the phrases whatsapp-rust and
/// [`crate::transport`] put in the error chain (`Failed to connect: {err:#}`).
///
/// A fresh install must first fetch the WhatsApp Web version, so a device that
/// cannot reach web.whatsapp.com stays at "Connecting" (crmne/zapfast#271).
fn connect_summary(lower: &str) -> Option<Cow<'static, str>> {
    if lower.contains("version fetch timed out") {
        return Some("timed out fetching the WhatsApp Web version".into());
    }
    if lower.contains("failed to resolve app version")
        || lower.contains("failed to fetch latest whatsapp version")
    {
        let cause: Cow<'static, str> = if let Some(status) = status_after(lower, "returned status ")
        {
            format!("HTTP status {status}").into()
        } else if lower.contains("could not find '")
            || lower.contains("could not decode the response body")
        {
            "unparsable response".into()
        } else {
            network_cause(lower).unwrap_or("request failed").into()
        };
        return Some(format!("could not fetch the WhatsApp Web version ({cause})").into());
    }
    for (stage, summary) in [
        ("transport connect", "timed out connecting to WhatsApp"),
        ("socket wait", "timed out waiting for the WhatsApp socket"),
        (
            "connection wait",
            "timed out waiting for WhatsApp to finish connecting",
        ),
    ] {
        if lower.contains(&format!("{stage} timed out after")) {
            return Some(summary.into());
        }
    }
    if lower.contains("timed out waiting for handshake response") {
        return Some("timed out waiting for the WhatsApp handshake".into());
    }
    if !(lower.contains("failed to open transport") || lower.contains("transport error:")) {
        return None;
    }
    Some(
        if lower.contains("tls to ") || lower.contains("tls through ") {
            if lower.contains("certificate") {
                "TLS connection to WhatsApp failed (certificate rejected)".into()
            } else {
                "TLS connection to WhatsApp failed".into()
            }
        } else if lower.contains("websocket connect ") {
            match status_after(lower, "got status code ") {
                Some(status) => {
                    format!("WebSocket connect to WhatsApp failed (HTTP status {status})").into()
                }
                None => "WebSocket connect to WhatsApp failed".into(),
            }
        } else if lower.contains("could not resolve ") || lower.contains("resolved to no address") {
            "could not resolve WhatsApp's address".into()
        } else if lower.contains("could not reach ") {
            "could not reach WhatsApp's servers".into()
        } else if lower.contains("proxy") {
            "could not connect through the proxy".into()
        } else {
            "could not open a connection to WhatsApp".into()
        },
    )
}

/// Why an HTTP request never got an answer, from ureq's error text.
fn network_cause(lower: &str) -> Option<&'static str> {
    if lower.contains("rustls:") || lower.contains("native-tls:") || lower.contains("certificate") {
        Some("TLS failure")
    } else if lower.contains("timeout:") || lower.contains("timed out") {
        Some("timed out")
    } else if lower.contains("host not found") {
        Some("host not found")
    } else if lower.contains("connect proxy failed") {
        Some("proxy failed")
    } else if lower.contains("connection failed") || lower.contains("io:") {
        Some("connection failed")
    } else {
        None
    }
}

/// The HTTP status right after `marker`: exactly three digits, then no more.
fn status_after(lower: &str, marker: &str) -> Option<u16> {
    let rest = &lower[lower.find(marker)? + marker.len()..];
    let digits = rest.get(..3)?;
    if !digits.bytes().all(|byte| byte.is_ascii_digit())
        || rest[3..].starts_with(|c: char| c.is_ascii_alphanumeric())
    {
        return None;
    }
    let status: u16 = digits.parse().ok()?;
    (100..600).contains(&status).then_some(status)
}

pub fn is_protocol_target(target: &str) -> bool {
    let module = target.split("::").next().unwrap_or_default();
    module == "whatsapp_rust"
        || module.starts_with("whatsapp_rust_")
        || module == "wacore"
        || module.starts_with("wacore_")
        || module == "waproto"
}

#[cfg(test)]
mod tests {
    use super::*;

    const SW: &str = "https://web.whatsapp.com/sw.js";

    /// `error!("Failed to connect: {connect_err:#}. Will retry...")` around a
    /// `ConnectError::Version` whose chain the fetch built.
    fn version_failure(cause: &str) -> String {
        format!(
            "Failed to connect: failed to resolve app version: \
             Failed to fetch latest WhatsApp version: {cause}. Will retry..."
        )
    }

    fn transport_failure(cause: &str) -> String {
        format!("Failed to connect: failed to open transport: {cause}. Will retry...")
    }

    #[test]
    fn protocol_errors_never_repeat_their_private_details() {
        for detail in [
            "fixture message",
            "123456789@s.whatsapp.net",
            "123456789012345@lid",
            "qr: fixture-secret",
        ] {
            for prefix in [
                "",
                "signature-mismatch ",
                "rate-overlimit ",
                "Failed to connect: failed to resolve app version: ",
                "Failed to connect: failed to resolve app version: returned status ",
                "Failed to connect: failed to open transport: TLS to ",
                "Failed to connect: failed to open transport: WebSocket connect ",
                "Failed to connect: failed to open transport: got status code ",
                "Failed to connect: transport connect timed out after 20s ",
            ] {
                let summary = protocol_summary(&format!("{prefix}{detail}"));
                assert!(!summary.contains(detail), "{summary}");
                assert!(!summary.contains("123"), "{summary}");
            }
        }
        assert!(is_protocol_target("whatsapp_rust::pair"));
        assert!(is_protocol_target("wacore::appstate"));
        assert!(is_protocol_target("wacore_noise::handshake"));
        assert!(is_protocol_target("whatsapp_rust_sqlite_storage::store"));
        assert!(!is_protocol_target("zapfast::backend::worker"));
    }

    #[test]
    fn a_failed_version_fetch_names_its_cause() {
        let cases = [
            (
                format!("HTTP request to {SW} returned status 403: HTTP status 403"),
                "could not fetch the WhatsApp Web version (HTTP status 403)",
            ),
            (
                format!("could not find 'client_revision' in the response from {SW}"),
                "could not fetch the WhatsApp Web version (unparsable response)",
            ),
            (
                format!("could not decode the response body from {SW}: invalid utf-8"),
                "could not fetch the WhatsApp Web version (unparsable response)",
            ),
            (
                format!(
                    "HTTP request to {SW} failed: rustls: invalid peer certificate: UnknownIssuer"
                ),
                "could not fetch the WhatsApp Web version (TLS failure)",
            ),
            (
                format!("HTTP request to {SW} failed: timeout: global"),
                "could not fetch the WhatsApp Web version (timed out)",
            ),
            (
                format!("HTTP request to {SW} failed: host not found"),
                "could not fetch the WhatsApp Web version (host not found)",
            ),
            (
                format!("HTTP request to {SW} failed: io: Connection refused (os error 111)"),
                "could not fetch the WhatsApp Web version (connection failed)",
            ),
            (
                format!("HTTP request to {SW} failed: something new"),
                "could not fetch the WhatsApp Web version (request failed)",
            ),
        ];
        for (cause, expected) in cases {
            assert_eq!(protocol_summary(&version_failure(&cause)), expected);
        }
        assert_eq!(
            protocol_summary("Failed to connect: version fetch timed out after 20s. Will retry..."),
            "timed out fetching the WhatsApp Web version"
        );
    }

    #[test]
    fn a_status_is_exactly_three_digits() {
        for cause in [
            "returned status 4031234567@s.whatsapp.net",
            "returned status 12@lid",
            "returned status 999",
        ] {
            assert_eq!(
                protocol_summary(&version_failure(cause)),
                "could not fetch the WhatsApp Web version (request failed)"
            );
        }
    }

    #[test]
    fn a_failed_transport_names_its_stage() {
        let cases = [
            (
                "TLS to 157.240.0.1:443 failed: invalid peer certificate: UnknownIssuer",
                "TLS connection to WhatsApp failed (certificate rejected)",
            ),
            (
                "TLS through the proxy failed: connection reset by peer",
                "TLS connection to WhatsApp failed",
            ),
            (
                "WebSocket connect to 157.240.0.1:443 failed: \
                 expected HTTP 101 Switching Protocols, got status code 403",
                "WebSocket connect to WhatsApp failed (HTTP status 403)",
            ),
            (
                "WebSocket connect to [2a03:2880::1]:443 failed: connection reset by peer",
                "WebSocket connect to WhatsApp failed",
            ),
            (
                "Could not resolve web.whatsapp.com: failed to lookup address information",
                "could not resolve WhatsApp's address",
            ),
            (
                "Could not reach web.whatsapp.com: Connection refused (os error 111)",
                "could not reach WhatsApp's servers",
            ),
        ];
        for (cause, expected) in cases {
            assert_eq!(protocol_summary(&transport_failure(cause)), expected);
        }
        assert_eq!(
            protocol_summary(
                "Failed to connect: transport connect timed out after 20s. Will retry..."
            ),
            "timed out connecting to WhatsApp"
        );
        assert_eq!(
            protocol_summary(
                "Transient connect failure, will retry: Timed out waiting for handshake response"
            ),
            "timed out waiting for the WhatsApp handshake"
        );
    }

    #[test]
    fn snapshot_recovery_logs_name_collection_and_stage_without_private_values() {
        let cases = [
            (
                "Snapshot regular_low v253 MAC mismatch over 31 records",
                "app-state regular_low snapshot MAC mismatch",
            ),
            (
                "Asking the primary device to send back RegularLow: its snapshot did not validate here",
                "app-state regular_low recovery request attempted",
            ),
            (
                "Could not ask the primary to rebuild RegularLow: request failed for 12345@s.whatsapp.net",
                "app-state regular_low recovery request failed",
            ),
            (
                "Recovered the regular_low collection from the primary device: 31 record(s)",
                "app-state regular_low recovery reply applied by library",
            ),
            (
                "Failed to apply the snapshot recovery for regular_low: secret 12345@s.whatsapp.net",
                "app-state regular_low recovery apply failed",
            ),
            (
                "Not asking the primary to rebuild RegularLow: the account has peer snapshot recovery turned off",
                "app-state regular_low peer recovery disabled",
            ),
            (
                "Snapshot regular_high v254 carries no MAC; refusing it rather than accepting unverified records",
                "app-state regular_high snapshot MAC missing",
            ),
            (
                "Snapshot regular v255 carries no key id; refusing it rather than accepting unverified records",
                "app-state regular snapshot key id missing",
            ),
            (
                "Discarding the primary's RegularLow at v255: this side already holds v257",
                "app-state regular_low recovery reply stale",
            ),
            (
                "Not asking the primary to rebuild CriticalBlock: the block list is not recovered this way",
                "app-state critical_block peer recovery unsupported",
            ),
        ];
        for (message, expected) in cases {
            assert_eq!(protocol_summary(message), expected);
        }
        assert_eq!(
            protocol_summary("Snapshot 12345@s.whatsapp.net v253 MAC mismatch over 31 records"),
            "protocol diagnostic (private details omitted)",
        );
    }

    #[test]
    fn unrelated_protocol_lines_stay_opaque() {
        for message in [
            "Failed to decrypt message from 123456789@s.whatsapp.net: timed out",
            "Received <message from=\"123456789@s.whatsapp.net\"> with a certificate",
            "HTTP request to https://mmg.whatsapp.net/v/t62 failed: timeout: global",
        ] {
            assert_eq!(
                protocol_summary(message),
                "protocol diagnostic (private details omitted)"
            );
        }
    }
}
