//! Keep protocol diagnostics useful without persisting untrusted payloads.

/// Protocol errors can embed JIDs, nodes, credentials and message contents.
/// Record only recognized failure categories, including in verbose logs.
pub fn protocol_summary(message: &str) -> &'static str {
    if message.contains("signature-mismatch") {
        "pairing signature verification failed"
    } else if message.contains("rate-overlimit") {
        "WhatsApp rate limit reached"
    } else {
        "protocol diagnostic (private details omitted)"
    }
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

    #[test]
    fn protocol_errors_never_repeat_their_private_details() {
        for detail in [
            "fixture message",
            "123456789@s.whatsapp.net",
            "qr: fixture-secret",
        ] {
            for prefix in ["", "signature-mismatch ", "rate-overlimit "] {
                assert!(!protocol_summary(&format!("{prefix}{detail}")).contains(detail));
            }
        }
        assert!(is_protocol_target("whatsapp_rust::pair"));
        assert!(is_protocol_target("wacore::appstate"));
        assert!(is_protocol_target("wacore_noise::handshake"));
        assert!(is_protocol_target("whatsapp_rust_sqlite_storage::store"));
        assert!(!is_protocol_target("zapfast::backend::worker"));
    }
}
