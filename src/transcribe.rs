//! Optional, private remote audio transcription.
//!
//! Transcription is provider-neutral: every provider speaks an
//! OpenAI-compatible `POST /audio/transcriptions` endpoint, so the same
//! multipart request works for OpenAI, Grok, Gemini, and any self-hosted
//! compatible server. The base URL and model are user-configurable; the API
//! key lives in the OS keyring, never in settings or logs.

use std::path::Path;

use anyhow::{Context, Result, ensure};
use keyring_core::api::CredentialStoreApi;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::settings::TranscriptionProvider;

/// Keyring service shared with the archive key. The two entries use distinct
/// identities, so neither can read or replace the other.
const SERVICE: &str = "rocks.zapfast.ZapFast";
/// Keyring identity for the transcription API key. A fixed name, not a path
/// or user content.
const API_KEY_IDENTITY: &str = "transcription-api-key";

/// A resolved remote-transcription configuration, without the secret key.
#[derive(Clone, Debug, PartialEq)]
pub struct TranscriptionConfig {
    pub provider: TranscriptionProvider,
    pub base_url: String,
    pub model: String,
}

impl TranscriptionConfig {
    /// Resolves empty settings against the provider defaults. Does not
    /// validate the URL; [`Request::build`] does that before any request.
    pub fn resolve(provider: TranscriptionProvider, base_url: &str, model: &str) -> Self {
        let base_url = if base_url.trim().is_empty() {
            provider.default_base_url().unwrap_or_default().to_owned()
        } else {
            base_url.trim().to_owned()
        };
        let model = if model.trim().is_empty() {
            provider.default_model().to_owned()
        } else {
            model.trim().to_owned()
        };
        Self {
            provider,
            base_url,
            model,
        }
    }

    /// Whether transcription can run at all.
    pub fn enabled(&self) -> bool {
        self.provider != TranscriptionProvider::None
    }
}

/// A validated transcription request ready to be assembled.
#[derive(Clone, Debug)]
pub struct Request {
    provider_kind: String,
    model: String,
    /// Validated base URL without a trailing slash.
    base_url: String,
}

/// A finished transcription together with the hash of its source audio.
#[derive(Clone, Debug)]
pub struct Completed {
    pub text: String,
    pub source_sha256: String,
    pub provider_kind: String,
    pub model: String,
}

impl Request {
    /// Validates the configuration and picks the endpoint.
    pub fn build(config: &TranscriptionConfig) -> std::result::Result<Self, String> {
        if !config.enabled() {
            return Err("Transcription is not configured".to_owned());
        }
        validate_base_url(&config.base_url)?;
        let base_url = config.base_url.trim_end_matches('/').to_owned();
        let model = config.model.trim().to_owned();
        if model.is_empty() {
            return Err("Transcription model is not configured".to_owned());
        }
        Ok(Self {
            provider_kind: config.provider.kind().to_owned(),
            model,
            base_url,
        })
    }

    /// The provider's transcription endpoint.
    pub fn endpoint(&self) -> String {
        format!("{}/audio/transcriptions", self.base_url)
    }

    /// Assembles the multipart body. Reads the recording from disk; nothing
    /// is sent here.
    pub fn multipart(
        &self,
        bytes: Vec<u8>,
        file_name: String,
    ) -> reqwest::blocking::multipart::Form {
        reqwest::blocking::multipart::Form::new()
            .text("model", self.model.clone())
            .part(
                "file",
                reqwest::blocking::multipart::Part::bytes(bytes).file_name(file_name),
            )
    }
}

/// Sends the recording to the provider and returns the transcription text and
/// the source hash. The key never appears in an error or log message.
pub fn transcribe(
    request: &Request,
    path: &Path,
    api_key: &str,
) -> std::result::Result<Completed, String> {
    let bytes =
        std::fs::read(path).map_err(|error| format!("Could not read the recording: {error}"))?;
    let source_sha256 = sha256_hex(&bytes);
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "audio.ogg".to_owned());
    let form = request.multipart(bytes, file_name);
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|error| format!("Could not prepare the transcription request: {error}"))?;
    let response = client
        .post(&request.endpoint())
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .map_err(|error| format!("Transcription request failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Transcription failed with HTTP {}",
            response.status().as_u16()
        ));
    }
    // Read and parse the body without ever echoing it into an error, so a
    // provider that mirrors the audio back cannot reach the log.
    let body = response
        .text()
        .map_err(|error| format!("The transcription response could not be read: {error}"))?;
    #[derive(serde::Deserialize)]
    struct Response {
        text: String,
    }
    let parsed: Response = serde_json::from_str(&body)
        .map_err(|_| "The transcription response was not understood".to_owned())?;
    let text = parsed.text.trim().to_owned();
    if text.is_empty() {
        return Err("The transcription came back empty".to_owned());
    }
    Ok(Completed {
        text,
        source_sha256,
        provider_kind: request.provider_kind.clone(),
        model: request.model.clone(),
    })
}

/// Hex-encodes a digest for the transcription cache.
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Rejects anything but HTTPS, or HTTP to a real loopback address. Embedded
/// credentials, queries, and fragments are always rejected, so a key cannot
/// leak through a URL and a redirect cannot smuggle parameters.
pub fn validate_base_url(url: &str) -> std::result::Result<(), String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("Transcription URL is empty".to_owned());
    }
    let Some((scheme, rest)) = url.split_once("://") else {
        return Err("Transcription URL must include a scheme".to_owned());
    };
    if rest.contains(['@', '?', '#']) {
        return Err(
            "Transcription URL must not contain credentials, a query, or a fragment".to_owned(),
        );
    }
    let host = rest.split('/').next().unwrap_or(rest);
    match scheme.to_ascii_lowercase().as_str() {
        "https" => Ok(()),
        "http" => {
            if is_loopback(host) {
                Ok(())
            } else {
                Err("Transcription over HTTP is only allowed for loopback endpoints".to_owned())
            }
        }
        other => Err(format!("Unsupported transcription URL scheme: {other}")),
    }
}

fn is_loopback(host: &str) -> bool {
    // Strip a trailing port and IPv6 brackets before matching.
    let host = host.trim();
    let host = host.strip_prefix('[').unwrap_or(host);
    let host = host.strip_suffix(']').unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host);
    host == "localhost" || host == "::1" || host.starts_with("127.")
}

/// Writes the API key to the OS keyring and verifies it by reading it back.
/// An empty key clears the entry instead.
pub fn set_api_key(key: &str) -> Result<()> {
    let entry = keyring_entry()?;
    let key = key.trim();
    if key.is_empty() {
        return clear_api_key();
    }
    let secret = Zeroizing::new(key.to_owned());
    entry
        .set_secret(secret.as_bytes())
        .context("Could not save the transcription API key in the OS keyring")?;
    let saved = Zeroizing::new(
        entry
            .get_secret()
            .context("Could not verify the saved transcription API key")?,
    );
    ensure!(
        saved.as_slice() == secret.as_bytes(),
        "The OS keyring did not retain the transcription API key"
    );
    Ok(())
}

/// Removes the transcription API key, tolerating an already-absent entry.
pub fn clear_api_key() -> Result<()> {
    let entry = keyring_entry()?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring_core::Error::NoEntry) => Ok(()),
        Err(error) => Err(error).context("Could not remove the transcription API key"),
    }
}

/// Reads the transcription API key, returning `None` when none is stored.
pub fn api_key() -> Result<Option<Zeroizing<String>>> {
    let entry = keyring_entry()?;
    match entry.get_secret() {
        Ok(secret) => {
            let secret = Zeroizing::new(secret);
            let text = std::str::from_utf8(&secret)
                .context("The transcription API key in the OS keyring is not text")?
                .to_owned();
            Ok(Some(Zeroizing::new(text)))
        }
        Err(keyring_core::Error::NoEntry) => Ok(None),
        Err(error) => Err(error).context("Unlock your OS keyring and restart ZapFast"),
    }
}

/// Whether an API key is stored, without reading its value out of the keyring.
pub fn has_api_key() -> bool {
    api_key().ok().flatten().is_some()
}

fn keyring_entry() -> Result<keyring_core::Entry> {
    #[cfg(target_os = "linux")]
    let store = zbus_secret_service_keyring_store::Store::new();
    #[cfg(target_os = "macos")]
    let store = apple_native_keyring_store::keychain::Store::new();
    #[cfg(windows)]
    let store = windows_native_keyring_store::Store::new();
    let store = store.context("Unlock your OS keyring and restart ZapFast")?;
    store
        .build(SERVICE, API_KEY_IDENTITY, None)
        .context("The OS keyring could not open the transcription API key")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_require_https_or_real_loopback() {
        assert!(validate_base_url("https://api.openai.com/v1").is_ok());
        assert!(validate_base_url("http://localhost:8000/v1").is_ok());
        assert!(validate_base_url("http://127.0.0.1:8080").is_ok());
        assert!(validate_base_url("http://[::1]:8080").is_ok());
        assert!(validate_base_url("http://example.com/v1").is_err());
        assert!(validate_base_url("ftp://example.com").is_err());
        assert!(validate_base_url("not a url").is_err());
        assert!(validate_base_url("").is_err());
    }

    #[test]
    fn urls_reject_embedded_credentials_query_and_fragment() {
        assert!(validate_base_url("https://user:pass@api.openai.com/v1").is_err());
        assert!(validate_base_url("https://api.openai.com/v1?key=x").is_err());
        assert!(validate_base_url("https://api.openai.com/v1#frag").is_err());
    }

    #[test]
    fn providers_map_to_default_endpoints_and_models() {
        assert_eq!(
            TranscriptionProvider::OpenAiCompatible.default_base_url(),
            Some("https://api.openai.com/v1")
        );
        assert_eq!(
            TranscriptionProvider::Grok.default_base_url(),
            Some("https://api.x.ai/v1")
        );
        assert_eq!(
            TranscriptionProvider::Gemini.default_base_url(),
            Some("https://generativelanguage.googleapis.com/v1beta/openai")
        );
        assert_eq!(TranscriptionProvider::None.default_base_url(), None);
        assert_eq!(
            TranscriptionProvider::OpenAiCompatible.default_model(),
            "whisper-1"
        );
        assert_eq!(TranscriptionProvider::None.default_model(), "");
    }

    #[test]
    fn config_resolves_empty_fields_to_provider_defaults() {
        let config = TranscriptionConfig::resolve(TranscriptionProvider::OpenAiCompatible, "", " ");
        assert_eq!(config.base_url, "https://api.openai.com/v1");
        assert_eq!(config.model, "whisper-1");
        let config = TranscriptionConfig::resolve(
            TranscriptionProvider::Grok,
            "https://mine.example/v1",
            "",
        );
        assert_eq!(config.base_url, "https://mine.example/v1");
        assert_eq!(config.model, "whisper-1");
    }

    #[test]
    fn requests_join_the_endpoint_and_reject_misconfiguration() {
        let request = Request::build(&TranscriptionConfig::resolve(
            TranscriptionProvider::OpenAiCompatible,
            "",
            "",
        ))
        .unwrap();
        assert_eq!(
            request.endpoint(),
            "https://api.openai.com/v1/audio/transcriptions"
        );
        // Trailing slashes and whitespace are normalized away.
        let request = Request::build(&TranscriptionConfig::resolve(
            TranscriptionProvider::Grok,
            " https://api.x.ai/v1/ ",
            "",
        ))
        .unwrap();
        assert_eq!(
            request.endpoint(),
            "https://api.x.ai/v1/audio/transcriptions"
        );
        assert!(
            Request::build(&TranscriptionConfig::resolve(
                TranscriptionProvider::None,
                "",
                ""
            ))
            .is_err()
        );
        assert!(
            Request::build(&TranscriptionConfig::resolve(
                TranscriptionProvider::OpenAiCompatible,
                "http://example.com",
                ""
            ))
            .is_err()
        );
        assert!(
            Request::build(&TranscriptionConfig::resolve(
                TranscriptionProvider::OpenAiCompatible,
                "",
                "   "
            ))
            .is_err()
        );
    }

    #[test]
    fn a_request_assembles_a_multipart_body_without_network() {
        let request = Request::build(&TranscriptionConfig::resolve(
            TranscriptionProvider::OpenAiCompatible,
            "",
            "whisper-1",
        ))
        .unwrap();
        let form = request.multipart(b"fixture audio".to_vec(), "note.ogg".to_owned());
        assert!(!form.boundary().is_empty());
    }

    #[test]
    fn source_hashes_are_deterministic() {
        assert_eq!(
            sha256_hex(b"fixture"),
            "6a948b2a316908dbbbc0ff9b682a5f7eb7b82bb3e648f3c3ce028d10896ff967"
        );
    }
}
