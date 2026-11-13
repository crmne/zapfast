//! A short-lived helper owns the authenticator and its uncancellable native calls.
//! Secrets travel only over bounded anonymous pipes, never arguments or files.

use std::io::{Read, Write};
use std::process::{ExitCode, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use webauthn_authenticator_rs::WebauthnAuthenticator;
use webauthn_authenticator_rs::prelude::{PublicKeyCredential, RequestChallengeResponse, Url};
use whatsapp_rust::passkey::{Assertion, AssertionRequest, PasskeyError, UserVerification};

const MAX_FRAME: usize = 64 * 1024;
const MAX_TIMEOUT_MS: u64 = 60_000;
const ORIGIN: &str = "https://web.whatsapp.com";
pub(crate) const MISSING_USER_HANDLE_ERROR: &str = "passkey-missing-user-handle";

pub(super) enum Progress {
    Qr(String),
    Waiting,
    Phase(Phase),
}

/// Closed diagnostic values keep authenticator data out of application logs.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(super) enum Phase {
    Discovery,
    ConnectingTunnel,
    Handshaking,
    WaitingForPhone,
    Processing,
    RequestingAssertion,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", content = "value")]
enum Reply {
    Qr(String),
    Waiting,
    Phase(Phase),
    Assertion {
        json: Vec<u8>,
        credential_id: Vec<u8>,
    },
    Failed(Failure),
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
enum Failure {
    InvalidOptions,
    UnsupportedOptions,
    MissingUserHandle,
    NoCredential,
    Cancelled,
    BluetoothUnavailable,
    PermissionDenied,
    Bluetooth,
    Websocket,
    Closed,
    Cbor,
    Security,
    UserVerificationRequired,
    Internal,
    Ctap(u8),
    Backend,
}

impl Failure {
    fn error(self) -> PasskeyError {
        match self {
            Self::InvalidOptions => PasskeyError::InvalidOptions("invalid passkey options".into()),
            Self::UnsupportedOptions => PasskeyError::InvalidOptions(
                "these passkey options are not supported by this authenticator".into(),
            ),
            Self::MissingUserHandle => PasskeyError::Backend(MISSING_USER_HANDLE_ERROR.into()),
            Self::NoCredential => PasskeyError::NoCredential,
            Self::Cancelled => PasskeyError::Cancelled,
            Self::BluetoothUnavailable => PasskeyError::Backend(
                "Bluetooth is unavailable. Enable Bluetooth and try again.".into(),
            ),
            Self::PermissionDenied => PasskeyError::Backend(
                "Bluetooth permission was denied. Allow Bluetooth access and try again.".into(),
            ),
            Self::Backend => backend_error(),
            other => PasskeyError::Backend(format!("passkey authenticator failed: {other:?}")),
        }
    }
}

fn backend_error() -> PasskeyError {
    PasskeyError::Backend("passkey authentication could not be completed".into())
}

/// Dropping this future kills the helper, including native dialogs and discovery.
pub(super) async fn authenticate(
    request: AssertionRequest,
    progress: impl Fn(Progress) + Send + Sync + 'static,
) -> Result<Assertion, PasskeyError> {
    let options = options(&request).map_err(|failure| {
        log::warn!("passkey request rejected: {failure:?}");
        failure.error()
    })?;
    let timeout = Duration::from_millis(u64::from(options.public_key.timeout.unwrap_or(60_000)));
    if request.raw_options_json.len() > MAX_FRAME {
        return Err(Failure::InvalidOptions.error());
    }
    let executable = std::env::current_exe().map_err(|_| backend_error())?;
    let mut child = tokio::process::Command::new(executable)
        .arg("--passkey-helper")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| backend_error())?;
    let result = tokio::time::timeout(timeout, async {
        let mut input = child.stdin.take().ok_or_else(backend_error)?;
        input
            .write_u32(request.raw_options_json.len() as u32)
            .await
            .map_err(|_| backend_error())?;
        input
            .write_all(request.raw_options_json.as_bytes())
            .await
            .map_err(|_| backend_error())?;
        input.shutdown().await.map_err(|_| backend_error())?;
        drop(input);
        let mut output = child.stdout.take().ok_or_else(backend_error)?;
        loop {
            let size = output.read_u32().await.map_err(|_| backend_error())? as usize;
            if size == 0 || size > MAX_FRAME {
                return Err(backend_error());
            }
            let mut frame = vec![0; size];
            output
                .read_exact(&mut frame)
                .await
                .map_err(|_| backend_error())?;
            let reply: Reply = serde_json::from_slice(&frame).map_err(|_| backend_error())?;
            match reply {
                Reply::Qr(qr) => progress(Progress::Qr(qr)),
                Reply::Waiting => progress(Progress::Waiting),
                Reply::Phase(phase) => progress(Progress::Phase(phase)),
                Reply::Failed(error) => {
                    log::warn!("passkey helper failed: {error:?}");
                    return Err(error.error());
                }
                Reply::Assertion {
                    json,
                    credential_id,
                } => {
                    if json.is_empty() || credential_id.is_empty() {
                        return Err(backend_error());
                    }
                    return Ok(Assertion::new(json, credential_id));
                }
            }
        }
    })
    .await;
    // Also terminate on malformed output and after receiving the result. Never
    // wait for an upstream blocking operation or runtime shutdown to finish.
    let _ = child.start_kill();
    match result {
        Ok(result) => result,
        Err(_) => Err(PasskeyError::Cancelled),
    }
}

/// Called immediately after parsing the hidden flag, before logging or app setup.
/// This is an internal child-process entry point, not a messaging interface.
#[doc(hidden)]
pub fn run_helper() -> ExitCode {
    // No logger is installed in this process. Suppress upstream tracing too:
    // its debug/trace events may contain decrypted CTAP frames and QR material.
    if tracing::subscriber::set_global_default(tracing::subscriber::NoSubscriber::default())
        .is_err()
    {
        return ExitCode::FAILURE;
    }
    std::panic::set_hook(Box::new(|_| {}));
    let result = helper_request().and_then(|request| {
        let options = options(&request)?;
        perform(options)
    });
    let reply = match result {
        Ok(assertion) => Reply::Assertion {
            json: assertion.assertion_json,
            credential_id: assertion.credential_id,
        },
        Err(error) => Reply::Failed(error),
    };
    if send(&reply).is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn helper_request() -> Result<AssertionRequest, Failure> {
    let mut input = std::io::stdin().lock();
    let mut length = [0; 4];
    input
        .read_exact(&mut length)
        .map_err(|_| Failure::InvalidOptions)?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > MAX_FRAME {
        return Err(Failure::InvalidOptions);
    }
    let mut json = vec![0; length];
    input
        .read_exact(&mut json)
        .map_err(|_| Failure::InvalidOptions)?;
    let json = std::str::from_utf8(&json).map_err(|_| Failure::InvalidOptions)?;
    whatsapp_rust::passkey::parse_request_options(json).map_err(|_| Failure::InvalidOptions)
}

fn send(reply: &Reply) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(reply)?;
    if bytes.len() > MAX_FRAME {
        return Err(std::io::Error::other("passkey frame too large"));
    }
    let mut output = std::io::stdout().lock();
    output.write_all(&(bytes.len() as u32).to_be_bytes())?;
    output.write_all(&bytes)?;
    output.flush()
}

fn options(request: &AssertionRequest) -> Result<RequestChallengeResponse, Failure> {
    if request.raw_options_json.len() > MAX_FRAME {
        return Err(Failure::InvalidOptions);
    }
    let mut value: serde_json::Value =
        serde_json::from_str(&request.raw_options_json).map_err(|_| Failure::InvalidOptions)?;
    let object = value.as_object_mut().ok_or(Failure::InvalidOptions)?;
    if let Some(extensions) = object.remove("extensions")
        && !extensions.is_object()
    {
        return Err(Failure::InvalidOptions);
    }
    // Unsupported WebAuthn extensions are optional; never invent their outputs.
    let rp = request.rp_id.as_deref().unwrap_or("web.whatsapp.com");
    if !matches!(rp, "whatsapp.com" | "web.whatsapp.com") || request.challenge.is_empty() {
        return Err(Failure::InvalidOptions);
    }
    if object.get("rpId").map(|value| value.as_str()) != request.rp_id.as_deref().map(Some) {
        return Err(Failure::InvalidOptions);
    }
    if let Some(credentials) = object.get_mut("allowCredentials") {
        for credential in credentials.as_array_mut().ok_or(Failure::InvalidOptions)? {
            let credential = credential.as_object_mut().ok_or(Failure::InvalidOptions)?;
            if credential.get("type").and_then(|value| value.as_str()) != Some("public-key") {
                return Err(Failure::UnsupportedOptions);
            }
            // This flow already chose hybrid. Transport hints do not change allowed IDs.
            credential.remove("transports");
        }
    }
    let timeout = match object.get("timeout") {
        Some(value) => value
            .as_u64()
            .filter(|value| *value > 0)
            .ok_or(Failure::InvalidOptions)?,
        None => 60_000,
    }
    .min(MAX_TIMEOUT_MS);
    object.insert("timeout".into(), timeout.into());
    object.insert("rpId".into(), rp.into());
    object
        .entry("allowCredentials")
        .or_insert_with(|| serde_json::json!([]));
    object
        .entry("userVerification")
        .or_insert_with(|| "preferred".into());
    let options: RequestChallengeResponse =
        serde_json::from_value(serde_json::json!({"publicKey": value}))
            .map_err(|_| Failure::InvalidOptions)?;
    // Do not trust a caller's parsed fields if they diverge from its original JSON.
    let verification = match request.user_verification {
        UserVerification::Required => "required",
        UserVerification::Preferred => "preferred",
        UserVerification::Discouraged => "discouraged",
    };
    let raw_verification = serde_json::to_value(options.public_key.user_verification)
        .map_err(|_| Failure::InvalidOptions)?;
    if options.public_key.challenge != request.challenge
        || options
            .public_key
            .allow_credentials
            .iter()
            .map(|credential| &credential.id)
            .ne(request.allow_credentials.iter())
        || raw_verification.as_str() != Some(verification)
    {
        return Err(Failure::InvalidOptions);
    }
    Ok(options)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn perform(options: RequestChallengeResponse) -> Result<Assertion, Failure> {
    let user_handle_required = options.public_key.allow_credentials.is_empty();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .map_err(|_| Failure::Backend)?;
    let ui = HybridUi;
    // The caller thread runs the synchronous library method while the runtime's
    // worker drives Bluetooth/WebSocket I/O. The parent kills this entire process
    // on cancel/timeout, including detached discovery tasks and blocking calls.
    runtime.block_on(async {
        let _ = send(&Reply::Phase(Phase::Discovery));
        let mut authenticator = webauthn_authenticator_rs::cable::connect_cable_authenticator(
            webauthn_authenticator_rs::types::CableRequestType::GetAssertion,
            &ui,
        )
        .await
        .map_err(authenticator_error)?;
        let _ = send(&Reply::Phase(Phase::RequestingAssertion));
        let credential = authenticator
            .do_authentication(origin()?, options)
            .map_err(authenticator_error)?;
        assertion(credential, user_handle_required)
    })
}

#[cfg(target_os = "windows")]
fn perform(options: RequestChallengeResponse) -> Result<Assertion, Failure> {
    use webauthn_authenticator_rs::win10::Win10;
    let user_handle_required = options.public_key.allow_credentials.is_empty();
    if !Win10::supports_cable() {
        return Err(Failure::UnsupportedOptions);
    }
    let _ = send(&Reply::Waiting);
    let credential = Win10::default()
        .do_authentication(origin()?, options)
        .map_err(authenticator_error)?;
    assertion(credential, user_handle_required)
}

fn origin() -> Result<Url, Failure> {
    Url::parse(ORIGIN).map_err(|_| Failure::Backend)
}

fn assertion(
    credential: PublicKeyCredential,
    user_handle_required: bool,
) -> Result<Assertion, Failure> {
    if credential.type_ != "public-key"
        || credential.raw_id.is_empty()
        || credential.response.authenticator_data.is_empty()
        || credential.response.client_data_json.is_empty()
        || credential.response.signature.is_empty()
    {
        return Err(Failure::Backend);
    }
    if user_handle_required
        && credential
            .response
            .user_handle
            .as_ref()
            .is_none_or(|handle| handle.is_empty())
    {
        return Err(Failure::MissingUserHandle);
    }
    let json = whatsapp_rust::passkey::build_webauthn_assertion_json(
        &credential.raw_id,
        &credential.response.client_data_json,
        &credential.response.authenticator_data,
        &credential.response.signature,
        credential.response.user_handle.as_deref(),
    );
    Ok(Assertion::new(json, credential.raw_id))
}

fn authenticator_error(error: webauthn_authenticator_rs::error::WebauthnCError) -> Failure {
    use webauthn_authenticator_rs::error::{CtapError, WebauthnCError};
    match error {
        WebauthnCError::Cancelled => Failure::Cancelled,
        WebauthnCError::Ctap(CtapError::Ctap2NoCredentials) => Failure::NoCredential,
        WebauthnCError::Ctap(
            CtapError::Ctap2KeepAliveCancel
            | CtapError::Ctap2OperationDenied
            | CtapError::Ctap2UserActionTimeout
            | CtapError::Ctap2ActionTimeout,
        ) => Failure::Cancelled,
        WebauthnCError::NoBluetoothAdapter => Failure::BluetoothUnavailable,
        WebauthnCError::PermissionDenied => Failure::PermissionDenied,
        WebauthnCError::BluetoothError(_) => Failure::Bluetooth,
        WebauthnCError::WebsocketError(_) => Failure::Websocket,
        WebauthnCError::Closed => Failure::Closed,
        WebauthnCError::Cbor => Failure::Cbor,
        WebauthnCError::Security => Failure::Security,
        WebauthnCError::UserVerificationRequired => Failure::UserVerificationRequired,
        WebauthnCError::Internal | WebauthnCError::UnexpectedState => Failure::Internal,
        WebauthnCError::Ctap(code) => Failure::Ctap(code.into()),
        _ => Failure::Backend,
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Debug)]
struct HybridUi;

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl webauthn_authenticator_rs::ui::UiCallback for HybridUi {
    fn request_pin(&self) -> Option<String> {
        None
    }
    fn request_touch(&self) {
        let _ = send(&Reply::Waiting);
    }
    fn processing(&self) {
        let _ = send(&Reply::Waiting);
    }
    fn fingerprint_enrollment_feedback(
        &self,
        _remaining_samples: u32,
        _feedback: Option<webauthn_authenticator_rs::types::EnrollSampleStatus>,
    ) {
    }
    fn cable_qr_code(
        &self,
        _request_type: webauthn_authenticator_rs::types::CableRequestType,
        url: String,
    ) {
        let _ = send(&Reply::Qr(url));
    }
    fn dismiss_qr_code(&self) {
        let _ = send(&Reply::Waiting);
    }
    fn cable_status_update(&self, state: webauthn_authenticator_rs::types::CableState) {
        use webauthn_authenticator_rs::types::CableState;
        let phase = match state {
            CableState::ConnectingToTunnelServer => Phase::ConnectingTunnel,
            CableState::Handshaking => Phase::Handshaking,
            CableState::Processing => Phase::Processing,
            _ => Phase::WaitingForPhone,
        };
        let _ = send(&Reply::Phase(phase));
        let _ = send(&Reply::Waiting);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credential(user_handle: serde_json::Value) -> PublicKeyCredential {
        serde_json::from_value(serde_json::json!({
            "id": "AQID",
            "rawId": "AQID",
            "type": "public-key",
            "response": {
                "authenticatorData": "AQID",
                "clientDataJSON": "e30",
                "signature": "BAUG",
                "userHandle": user_handle
            },
            "clientExtensionResults": {}
        }))
        .unwrap()
    }

    #[test]
    fn discoverable_assertion_reports_a_missing_user_handle() {
        for handle in [serde_json::Value::Null, serde_json::json!("")] {
            let failure = assertion(credential(handle), true)
                .err()
                .map(Failure::error);
            assert!(matches!(
                failure,
                Some(PasskeyError::Backend(message)) if message == MISSING_USER_HANDLE_ERROR
            ));
        }
    }

    #[test]
    fn allowed_credential_assertion_does_not_invent_a_user_handle() {
        let assertion = assertion(credential(serde_json::Value::Null), false).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&assertion.assertion_json).unwrap();
        assert!(json["response"]["userHandle"].is_null());
    }

    #[test]
    fn optional_webauthn_metadata_does_not_block_phone_authentication() {
        let request = whatsapp_rust::passkey::parse_request_options(
            r#"{
            "challenge":"AQID","rpId":"whatsapp.com","userVerification":"required",
            "hints":["hybrid"],"extensions":{"uvm":true,"futureOptionalExtension":{}},
            "allowCredentials":[{
                "type":"public-key","id":"BAUG","transports":["hybrid","future-transport"],
                "futureCredentialHint":true
            }]
        }"#,
        )
        .unwrap();
        let options = options(&request).expect("optional metadata must not reject the request");
        assert_eq!(options.public_key.challenge, [1, 2, 3]);
        assert_eq!(options.public_key.rp_id, "whatsapp.com");
        assert_eq!(options.public_key.allow_credentials[0].id, [4, 5, 6]);
        assert_eq!(
            serde_json::to_value(options.public_key.user_verification).unwrap(),
            "required"
        );
    }

    #[test]
    fn optional_metadata_does_not_allow_an_unrelated_relying_party() {
        let request = whatsapp_rust::passkey::parse_request_options(
            r#"{
            "challenge":"AQID","rpId":"whatsapp.com.attacker.example",
            "userVerification":"required","hints":["hybrid"]
        }"#,
        )
        .unwrap();
        assert!(matches!(options(&request), Err(Failure::InvalidOptions)));
    }
}
