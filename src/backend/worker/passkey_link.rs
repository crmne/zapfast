//! Keeps passkey attempts tied to their connection and explicit user approval.

use std::time::{Duration, Instant};

use whatsapp_rust::passkey::{Assertion, AssertionRequest, PasskeyError, parse_request_options};

use super::{RuntimeEvent, Worker, wa_events};
use crate::backend::{LinkStatus, PasskeyStage, passkey};

#[derive(Default)]
pub(super) struct State {
    pub(super) connection: u64,
    sequence: u64,
    attempt: Option<Attempt>,
}

struct Attempt {
    id: u64,
    request: Option<AssertionRequest>,
    task: Option<tokio::task::JoinHandle<()>>,
    deadline: Option<Instant>,
}

impl State {
    pub(super) fn active(&self) -> bool {
        self.attempt.is_some()
    }

    pub(super) fn is_current(&self, id: u64) -> bool {
        self.attempt
            .as_ref()
            .is_some_and(|attempt| attempt.id == id)
    }

    pub(super) fn finish(&mut self) {
        if let Some(attempt) = self.attempt.take()
            && let Some(task) = attempt.task
        {
            task.abort();
        }
    }

    pub(super) fn reset(&mut self) {
        self.finish();
        self.connection = self.connection.wrapping_add(1);
    }
}

impl Drop for State {
    fn drop(&mut self) {
        self.finish();
    }
}

impl Worker {
    pub(super) async fn link_event(
        &mut self,
        connection: u64,
        event: std::sync::Arc<wa_events::Event>,
    ) {
        if connection == self.passkey.connection {
            self.handle_wa_event(event).await;
        }
    }

    pub(super) fn passkey_requested(&mut self, json: &str) {
        self.passkey.finish();
        self.passkey.sequence = self.passkey.sequence.wrapping_add(1);
        let id = self.passkey.sequence;
        let request = parse_request_options(json).ok();
        let stage = if request.is_some() {
            PasskeyStage::Ready
        } else {
            PasskeyStage::Failed("WhatsApp sent an invalid passkey request. Try again.".into())
        };
        self.passkey.attempt = Some(Attempt {
            id,
            request,
            task: None,
            deadline: None,
        });
        self.qr = None;
        self.pair_code = None;
        self.pairing_phone = None;
        self.set_status(LinkStatus::Passkey { attempt: id, stage });
    }

    fn passkey_stage(&mut self, id: u64, stage: PasskeyStage) {
        if self.passkey.is_current(id) {
            if matches!(stage, PasskeyStage::Failed(_))
                && let Some(attempt) = self.passkey.attempt.as_mut()
            {
                attempt.deadline = None;
                if let Some(task) = attempt.task.take() {
                    task.abort();
                }
            }
            self.set_status(LinkStatus::Passkey { attempt: id, stage });
        }
    }

    fn authenticating_passkey(&self, id: u64) -> bool {
        self.passkey.is_current(id)
            && matches!(
                self.status,
                LinkStatus::Passkey {
                    stage: PasskeyStage::Starting | PasskeyStage::Qr(_) | PasskeyStage::Waiting,
                    ..
                }
            )
    }

    pub(super) async fn start_passkey(&mut self, id: u64) {
        if !self.passkey.is_current(id) {
            return;
        }
        if matches!(
            self.status,
            LinkStatus::Passkey {
                stage: PasskeyStage::Failed(_),
                ..
            }
        ) {
            // The upstream handshake may have rotated its pending secrets. Retry
            // on a fresh connection, never by reusing a partially completed flow.
            self.stop_bot().await;
            self.start_bot().await;
            return;
        }
        if !matches!(
            self.status,
            LinkStatus::Passkey {
                stage: PasskeyStage::Ready,
                ..
            }
        ) {
            return;
        }
        let Some(request) = self
            .passkey
            .attempt
            .as_ref()
            .and_then(|attempt| attempt.request.clone())
        else {
            return;
        };
        if self.client.is_none() {
            self.passkey_failed();
            return;
        }
        self.passkey_stage(id, PasskeyStage::Starting);
        self.passkey.attempt.as_mut().unwrap().deadline =
            Some(Instant::now() + Duration::from_secs(120));
        let sender = self.wa_sender.clone();
        let task = tokio::spawn(async move {
            let progress_sender = sender.clone();
            let result = passkey::authenticate(request, move |progress| {
                let _ = progress_sender.send(RuntimeEvent::PasskeyProgress {
                    attempt: id,
                    progress,
                });
            })
            .await;
            let _ = sender.send(RuntimeEvent::PasskeyAssertion {
                attempt: id,
                result,
            });
        });
        self.passkey.attempt.as_mut().unwrap().task = Some(task);
    }

    pub(super) fn passkey_progress(&mut self, id: u64, progress: passkey::Progress) {
        if !self.authenticating_passkey(id) {
            return;
        }
        let stage = match progress {
            passkey::Progress::Qr(qr) => PasskeyStage::Qr(qr),
            passkey::Progress::Waiting => PasskeyStage::Waiting,
            passkey::Progress::Phase(phase) => {
                log::info!("passkey helper phase: {phase:?}");
                return;
            }
        };
        self.passkey_stage(id, stage);
    }

    pub(super) fn passkey_assertion(&mut self, id: u64, result: Result<Assertion, PasskeyError>) {
        if !self.authenticating_passkey(id) {
            return;
        }
        let assertion = match result {
            Ok(assertion) => assertion,
            Err(error) => {
                let category = match &error {
                    PasskeyError::Cancelled => "cancelled or timed out",
                    PasskeyError::NoCredential => "credential not found",
                    PasskeyError::InvalidOptions(_) => "unsupported or invalid options",
                    _ => "authenticator backend",
                };
                log::warn!("passkey authentication failed: {category}");
                let message = match error {
                    PasskeyError::Cancelled => {
                        "Passkey authentication was cancelled or timed out. Try again."
                    }
                    PasskeyError::NoCredential => {
                        "No WhatsApp passkey was found on the phone. Try again with a phone that has your passkey."
                    }
                    PasskeyError::InvalidOptions(_) => {
                        "WhatsApp requested passkey options this authenticator cannot use. Try again or update ZapFast."
                    }
                    _ => {
                        "Could not authenticate with the phone. Check Bluetooth permissions and internet access, then try again."
                    }
                };
                self.passkey_stage(id, PasskeyStage::Failed(message.into()));
                return;
            }
        };
        let Some(client) = self.client.clone() else {
            self.passkey_failed();
            return;
        };
        self.passkey_stage(id, PasskeyStage::Submitting);
        let sender = self.wa_sender.clone();
        let task = tokio::spawn(async move {
            let success = tokio::time::timeout(
                Duration::from_secs(30),
                client.send_passkey_response(assertion),
            )
            .await
            .is_ok_and(|result| result.is_ok());
            let _ = sender.send(RuntimeEvent::PasskeySent {
                attempt: id,
                success,
            });
        });
        self.passkey.attempt.as_mut().unwrap().task = Some(task);
    }

    pub(super) fn passkey_confirmation(
        &mut self,
        confirmation: &wa_events::PairPasskeyConfirmation,
    ) {
        let Some(id) = self.passkey.attempt.as_ref().map(|attempt| attempt.id) else {
            return;
        };
        if !matches!(
            self.status,
            LinkStatus::Passkey {
                stage: PasskeyStage::Submitting,
                ..
            }
        ) {
            return;
        }
        self.passkey_stage(id, PasskeyStage::Confirm(confirmation.code.clone()));
        if confirmation.skip_handoff_ux {
            // Upstream proves continuity for a re-link. Fresh links always wait
            // for the person to compare the code on both devices.
            self.confirm_passkey(id);
        }
    }

    pub(super) fn confirm_passkey(&mut self, id: u64) {
        if !self.passkey.is_current(id)
            || !matches!(
                self.status,
                LinkStatus::Passkey {
                    stage: PasskeyStage::Confirm(_),
                    ..
                }
            )
        {
            return;
        }
        let Some(client) = self.client.clone() else {
            self.passkey_failed();
            return;
        };
        self.passkey_stage(id, PasskeyStage::Submitting);
        let sender = self.wa_sender.clone();
        let task = tokio::spawn(async move {
            let success =
                tokio::time::timeout(Duration::from_secs(30), client.send_passkey_confirmation())
                    .await
                    .is_ok_and(|result| result.is_ok());
            let _ = sender.send(RuntimeEvent::PasskeySent {
                attempt: id,
                success,
            });
        });
        self.passkey.attempt.as_mut().unwrap().task = Some(task);
    }

    pub(super) fn passkey_sent(&mut self, id: u64, success: bool) {
        if self.passkey.is_current(id) && !success {
            self.passkey_failed();
        }
    }

    pub(super) fn expire_passkey(&mut self) {
        if self
            .passkey
            .attempt
            .as_ref()
            .and_then(|attempt| attempt.deadline)
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.passkey_failed();
        }
    }

    pub(super) fn passkey_failed(&mut self) {
        if let Some(attempt) = self.passkey.attempt.as_mut() {
            attempt.deadline = None;
            if let Some(task) = attempt.task.take() {
                task.abort();
            }
            let id = attempt.id;
            self.passkey_stage(
                id,
                PasskeyStage::Failed(
                    "Passkey linking failed. Try again to request a fresh challenge.".into(),
                ),
            );
        } else {
            self.set_status(LinkStatus::Failed(
                "Could not request a passkey challenge from WhatsApp. Try again.".into(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::backend::Event;

    const REQUEST: &str = r#"{"challenge":"AQID","timeout":600000,"rpId":"whatsapp.com","allowCredentials":[],"userVerification":"required","extensions":{"uvm":true}}"#;

    fn latest(events: &std::sync::mpsc::Receiver<Event>) -> LinkStatus {
        events
            .try_iter()
            .filter_map(|event| match event {
                Event::Link(status) => Some(status),
                _ => None,
            })
            .last()
            .expect("link status")
    }

    #[tokio::test]
    async fn challenge_waits_for_user_and_qr_rotation_cannot_hide_it() {
        let (mut worker, events, _, _) = super::super::receipt_tests::worker();
        let request = wa_events::PairPasskeyRequest::builder()
            .request_options_json(REQUEST.into())
            .build();
        worker
            .handle_wa_event(Arc::new(wa_events::Event::PairPasskeyRequest(request)))
            .await;
        assert_eq!(
            latest(&events),
            LinkStatus::Passkey {
                attempt: 1,
                stage: PasskeyStage::Ready
            }
        );
        let qr = wa_events::PairingQrCode::builder()
            .code("synthetic-qr".into())
            .timeout(Duration::from_secs(20))
            .build();
        worker
            .handle_wa_event(Arc::new(wa_events::Event::PairingQrCode(qr)))
            .await;
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn stopped_attempt_cannot_replace_new_challenge_or_rotate_its_secret() {
        let (mut worker, events, _, _) = super::super::receipt_tests::worker();
        worker.passkey_requested(REQUEST);
        worker.stop_bot().await;
        worker.passkey_requested(REQUEST);
        let ready = latest(&events);
        worker.passkey_progress(1, passkey::Progress::Qr("old-qr".into()));
        worker.passkey_assertion(1, Ok(Assertion::new(b"{}".to_vec(), vec![1])));
        worker.passkey_sent(1, false);
        worker.confirm_passkey(1);
        assert!(events.try_recv().is_err());
        assert_eq!(worker.status, ready);
        assert_eq!(
            ready,
            LinkStatus::Passkey {
                attempt: 2,
                stage: PasskeyStage::Ready
            }
        );
    }

    #[tokio::test]
    async fn fresh_link_requires_matching_attempt_and_explicit_code_confirmation() {
        let (mut worker, events, _, _) = super::super::receipt_tests::worker();
        worker.passkey_requested(REQUEST);
        worker.passkey_stage(1, PasskeyStage::Submitting);
        let _ = latest(&events);
        let confirmation = wa_events::PairPasskeyConfirmation::builder()
            .code("ABCD-EFGH".into())
            .skip_handoff_ux(false)
            .build();
        worker
            .handle_wa_event(Arc::new(wa_events::Event::PairPasskeyConfirmation(
                confirmation,
            )))
            .await;
        assert_eq!(
            latest(&events),
            LinkStatus::Passkey {
                attempt: 1,
                stage: PasskeyStage::Confirm("ABCD-EFGH".into())
            }
        );
        worker.confirm_passkey(2);
        worker.passkey_progress(1, passkey::Progress::Waiting);
        assert!(events.try_recv().is_err());
        assert_eq!(
            worker.status,
            LinkStatus::Passkey {
                attempt: 1,
                stage: PasskeyStage::Confirm("ABCD-EFGH".into())
            }
        );
    }

    #[tokio::test]
    async fn stopping_authentication_aborts_its_owned_task() {
        let (mut worker, _, _, _) = super::super::receipt_tests::worker();
        worker.passkey_requested(REQUEST);
        let (dropped, receiver) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let _guard = dropped;
            std::future::pending::<()>().await;
        });
        tokio::task::yield_now().await;
        worker.passkey.attempt.as_mut().unwrap().task = Some(task);
        worker.stop_bot().await;
        assert!(
            tokio::time::timeout(Duration::from_secs(1), receiver)
                .await
                .unwrap()
                .is_err()
        );
    }
    #[tokio::test]
    async fn phone_code_requested_before_passkey_cannot_hide_the_challenge() {
        let (mut worker, events, _, _) = super::super::receipt_tests::worker();
        worker.passkey_requested(REQUEST);
        let _ = latest(&events);
        worker
            .handle_command(crate::backend::Command::PairCode {
                connection: worker.passkey.connection,
                result: Ok("OLD-CODE".into()),
            })
            .await;
        assert!(events.try_recv().is_err());
        assert_eq!(
            worker.status,
            LinkStatus::Passkey {
                attempt: 1,
                stage: PasskeyStage::Ready,
            }
        );
    }
    #[tokio::test]
    async fn successful_iq_does_not_leave_an_unbounded_pairing_wait() {
        let (mut worker, events, _, _) = super::super::receipt_tests::worker();
        worker.passkey_requested(REQUEST);
        worker.passkey_stage(1, PasskeyStage::Submitting);
        worker.passkey.attempt.as_mut().unwrap().deadline =
            Some(Instant::now() - Duration::from_secs(1));
        worker.passkey_sent(1, true);
        let _ = latest(&events);
        worker.expire_passkey();
        assert!(matches!(
            latest(&events),
            LinkStatus::Passkey {
                attempt: 1,
                stage: PasskeyStage::Failed(_),
            }
        ));
        worker.expire_passkey();
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn old_phone_code_cannot_replace_a_new_connections_code() {
        let (mut worker, events, _, _) = super::super::receipt_tests::worker();
        let connection = worker.passkey.connection;
        worker.stop_bot().await;
        worker.pairing_phone = Some("synthetic".into());
        worker.pair_code = Some("NEW-CODE".into());
        worker
            .handle_command(crate::backend::Command::PairCode {
                connection,
                result: Ok("OLD-CODE".into()),
            })
            .await;
        assert!(events.try_recv().is_err());
        assert_eq!(worker.pair_code.as_deref(), Some("NEW-CODE"));
    }

    #[tokio::test]
    async fn retired_connection_cannot_finish_a_new_passkey_attempt() {
        let (mut worker, events, _, _) = super::super::receipt_tests::worker();
        let old_connection = worker.passkey.connection;
        worker.passkey_requested(REQUEST);
        worker.stop_bot().await;
        worker.passkey_requested(REQUEST);
        let _ = latest(&events);
        worker
            .link_event(
                old_connection,
                Arc::new(wa_events::Event::Connected(
                    wa_events::Connected::builder().build(),
                )),
            )
            .await;
        assert!(events.try_recv().is_err());
        assert_eq!(
            worker.status,
            LinkStatus::Passkey {
                attempt: 2,
                stage: PasskeyStage::Ready,
            }
        );
    }

    #[tokio::test]
    async fn helper_failure_keeps_its_original_error_after_the_attempt_deadline() {
        let (mut worker, events, _, _) = super::super::receipt_tests::worker();
        worker.passkey_requested(REQUEST);
        worker.passkey_stage(1, PasskeyStage::Starting);
        worker.passkey.attempt.as_mut().unwrap().deadline =
            Some(Instant::now() - Duration::from_secs(1));
        let _ = latest(&events);
        worker.passkey_assertion(1, Err(PasskeyError::InvalidOptions("synthetic".into())));
        let original = latest(&events);
        assert!(matches!(&original, LinkStatus::Passkey {
            stage: PasskeyStage::Failed(message), ..
        } if message.contains("options")));
        worker.expire_passkey();
        assert!(
            events.try_recv().is_err(),
            "a finished attempt must not overwrite its error"
        );
        assert_eq!(worker.status, original);
    }
}
