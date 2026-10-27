//! Account-bound scheduling around the process-wide native recognizer.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::{ChatId, Content, Event, MAX_TRANSCRIPTION_TRIES, Worker};
use crate::transcribe::{self, Error, Model, Progress, State};

#[derive(Clone)]
pub(super) struct Request {
    chat: ChatId,
    id: String,
    force: bool,
}

pub(super) enum Update {
    Progress {
        model: Model,
        generation: u64,
        state: State,
    },
    Prepared {
        generation: u64,
        result: Result<(), Error>,
    },
    Finished(Finished),
}

pub(super) struct Finished {
    chat: ChatId,
    id: String,
    model: Model,
    generation: u64,
    session: u64,
    text: Result<String, Error>,
}

impl Worker {
    pub(super) fn cancel_transcription(&mut self) {
        self.transcription_queue.clear();
        if let Some(cancel) = &self.transcription_cancel {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    pub(super) fn configure_transcription(&mut self, model: Model) {
        self.cancel_transcription();
        if let Some(cancel) = self.transcription_prepare_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.transcription_model = model;
        self.transcription_generation = self.transcription_generation.wrapping_add(1);
        self.transcription_failed = false;
        self.transcription_tries.clear();
    }

    fn transcription_progress(&self) -> Progress {
        let sender = self.transcription_tx.clone();
        let model = self.transcription_model;
        let generation = self.transcription_generation;
        Box::new(move |state| {
            let _ = sender.send(Update::Progress {
                model,
                generation,
                state,
            });
        })
    }

    pub(super) fn prepare_transcription(&mut self, model: Model) {
        if self.transcription_model != model {
            self.configure_transcription(model);
        }
        if self.transcription_prepare_cancel.is_some() {
            return;
        }
        self.transcription_failed = false;
        let cancel = Arc::new(AtomicBool::new(false));
        self.transcription_prepare_cancel = Some(Arc::clone(&cancel));
        let progress = self.transcription_progress();
        let sender = self.transcription_tx.clone();
        let generation = self.transcription_generation;
        std::thread::spawn(move || {
            let result = transcribe::prepare(model, cancel, progress);
            let _ = sender.send(Update::Prepared { generation, result });
        });
    }

    pub(super) fn transcribe_on_screen(&mut self, chat: ChatId, ids: Vec<String>, model: Model) {
        if self.transcription_model != model {
            self.configure_transcription(model);
        }
        if self
            .transcribing
            .as_ref()
            .is_some_and(|(current, _)| current != &chat)
        {
            self.cancel_transcription();
        }
        // An explicit retry keeps its place even though a cached transcript
        // excludes it from the viewport's list of voices without text.
        self.transcription_queue
            .retain(|request| request.force && request.chat == chat);
        for id in ids {
            let active = self
                .transcribing
                .as_ref()
                .is_some_and(|key| key == &(chat.clone(), id.clone()))
                && self
                    .transcription_cancel
                    .as_ref()
                    .is_none_or(|cancel| !cancel.load(Ordering::Relaxed));
            // An old job can still be finishing its current inference chunk
            // after a cancellation or model switch. It will not save a
            // result, so a new viewport request must queue its replacement.
            let queued = self
                .transcription_queue
                .iter()
                .any(|item| item.chat == chat && item.id == id);
            if !active && !queued {
                self.transcription_queue.push_back(Request {
                    chat: chat.clone(),
                    id,
                    force: false,
                });
            }
        }
        self.pump_transcription();
    }

    pub(super) fn transcribe_again(&mut self, chat: ChatId, id: String, model: Model) {
        let eligible = self
            .archive
            .message(&chat, &id)
            .ok()
            .flatten()
            .is_some_and(|row| {
                matches!(row.content, Content::Audio { media, voice_note: true, .. }
                if media.path.as_ref().is_some_and(|path| path.is_file()))
            });
        if !eligible {
            self.emit(Event::Transcription {
                chat,
                message: id,
                text: None,
            });
            return;
        }
        if self.transcription_model != model {
            self.configure_transcription(model);
        }
        self.transcription_failed = false;
        self.transcription_tries.remove(&(chat.clone(), id.clone()));
        if self.transcribing.as_ref() == Some(&(chat.clone(), id.clone()))
            && self
                .transcription_cancel
                .as_ref()
                .is_some_and(|cancel| !cancel.load(Ordering::Relaxed))
        {
            return;
        }
        self.transcription_queue
            .retain(|item| item.chat != chat || item.id != id);
        self.transcription_queue.push_front(Request {
            chat,
            id,
            force: true,
        });
        self.pump_transcription();
    }

    fn pump_transcription(&mut self) {
        if !self.privacy_ready {
            self.transcription_queue.clear();
            return;
        }
        if self.transcribing.is_some() || self.transcription_failed {
            return;
        }
        while let Some(request) = self.transcription_queue.pop_front() {
            let key = (request.chat.clone(), request.id.clone());
            if self.transcription_tries.get(&key).copied().unwrap_or(0) >= MAX_TRANSCRIPTION_TRIES {
                continue;
            }
            let Ok(Some(message)) = self.archive.message(&request.chat, &request.id) else {
                if request.force {
                    self.emit(Event::Transcription {
                        chat: request.chat,
                        message: request.id,
                        text: None,
                    });
                }
                continue;
            };
            if message.transcript.is_some() && !request.force {
                continue;
            }
            let Content::Audio {
                media,
                voice_note: true,
                ..
            } = &message.content
            else {
                if request.force {
                    self.emit(Event::Transcription {
                        chat: request.chat,
                        message: request.id,
                        text: None,
                    });
                }
                continue;
            };
            let Some(path) = media.path.as_ref().filter(|path| path.is_file()).cloned() else {
                if request.force {
                    self.emit(Event::Transcription {
                        chat: request.chat,
                        message: request.id,
                        text: None,
                    });
                }
                continue;
            };
            let cancel = Arc::new(AtomicBool::new(false));
            self.transcription_cancel = Some(Arc::clone(&cancel));
            self.transcribing = Some(key);
            self.emit(Event::TranscriptionStarted {
                chat: request.chat.clone(),
                message: request.id.clone(),
            });
            let sender = self.transcription_tx.clone();
            let progress = self.transcription_progress();
            let model = self.transcription_model;
            let generation = self.transcription_generation;
            let session = self.star_session;
            std::thread::spawn(move || {
                let text = transcribe::transcribe(model, &path, cancel, progress);
                let _ = sender.send(Update::Finished(Finished {
                    chat: request.chat,
                    id: request.id,
                    model,
                    generation,
                    session,
                    text,
                }));
            });
            return;
        }
    }

    pub(super) fn transcription_update(&mut self, update: Update) {
        match update {
            Update::Progress {
                model,
                generation,
                state,
            } => {
                if generation == self.transcription_generation && model == self.transcription_model
                {
                    self.emit(Event::TranscriptionModel { model, state });
                }
            }
            Update::Prepared { generation, result } => {
                if generation == self.transcription_generation {
                    self.transcription_prepare_cancel = None;
                    self.transcription_failed = matches!(result, Err(Error::Model(_)));
                    self.pump_transcription();
                }
            }
            Update::Finished(done) => self.transcription_finished(done),
        }
    }

    fn transcription_finished(&mut self, done: Finished) {
        let key = (done.chat.clone(), done.id.clone());
        if self.transcribing.as_ref() != Some(&key) {
            return;
        }
        self.transcribing = None;
        let cancelled = self
            .transcription_cancel
            .take()
            .is_some_and(|flag| flag.load(Ordering::Relaxed));
        if cancelled
            || done.generation != self.transcription_generation
            || done.session != self.star_session
        {
            self.emit(Event::Transcription {
                chat: done.chat,
                message: done.id,
                text: None,
            });
            self.pump_transcription();
            return;
        }
        let text = match done.text {
            Ok(text) if !text.is_empty() => {
                match self
                    .archive
                    .set_transcript(&done.chat, &done.id, &text, &done.model.engine())
                {
                    Ok(true) => Some(text),
                    Ok(false) => None,
                    Err(_) => {
                        log::warn!("could not store a voice transcript");
                        None
                    }
                }
            }
            Err(Error::Model(_)) => {
                // A failed model download/load stops the queue until the
                // reader retries. It is not a failure of every queued clip.
                self.transcription_failed = true;
                None
            }
            Err(Error::Cancelled) => None,
            Err(Error::Audio) | Ok(_) => {
                self.transcription_tries
                    .insert(key, MAX_TRANSCRIPTION_TRIES);
                None
            }
        };
        // A retry never deletes the old text on failure; the UI replaces it
        // only after a new answer was successfully stored.
        self.emit(Event::Transcription {
            chat: done.chat,
            message: done.id,
            text,
        });
        self.pump_transcription();
    }
}

#[cfg(test)]
mod tests {
    use super::super::receipt_tests::{PEER, worker};
    use super::*;

    #[test]
    fn a_visible_voice_is_requeued_while_its_cancelled_job_is_still_finishing() {
        let (mut worker, _, _, _) = worker();
        worker.transcribing = Some((PEER.into(), "voice".into()));
        let cancel = Arc::new(AtomicBool::new(false));
        worker.transcription_cancel = Some(Arc::clone(&cancel));
        worker.cancel_transcription();
        assert!(cancel.load(Ordering::Relaxed));
        // Reopening the chat can happen before an inference chunk notices
        // cancellation. The new viewport is already the list the app sent;
        // it must not need another frame or a restart to ask for this voice.
        worker.transcribe_on_screen(PEER.into(), vec!["voice".into()], Model::default());
        assert!(
            worker
                .transcription_queue
                .iter()
                .any(|request| request.id == "voice")
        );
    }

    #[test]
    fn the_same_live_job_is_not_queued_twice() {
        let (mut worker, _, _, _) = worker();
        worker.transcribing = Some((PEER.into(), "voice".into()));
        worker.transcription_cancel = Some(Arc::new(AtomicBool::new(false)));
        worker.transcribe_on_screen(PEER.into(), vec!["voice".into()], Model::default());
        assert!(worker.transcription_queue.is_empty());
    }

    #[test]
    fn changing_models_cancels_old_jobs_and_preparations() {
        let (mut worker, _, _, _) = worker();
        let job = Arc::new(AtomicBool::new(false));
        let preparation = Arc::new(AtomicBool::new(false));
        worker.transcription_cancel = Some(Arc::clone(&job));
        worker.transcription_prepare_cancel = Some(Arc::clone(&preparation));
        worker.transcription_queue.push_back(Request {
            chat: PEER.into(),
            id: "voice".into(),
            force: false,
        });
        worker.configure_transcription(Model::ParakeetUltra);
        assert!(job.load(Ordering::Relaxed));
        assert!(preparation.load(Ordering::Relaxed));
        assert!(worker.transcription_queue.is_empty());
        assert_eq!(worker.transcription_generation, 1);
    }

    #[test]
    fn a_failed_retry_keeps_the_saved_transcript() {
        let (mut worker, events, _, _) = worker();
        let row = crate::archive::tests::message(PEER, "voice", 10, false);
        worker.archive.insert_message(&row, None).unwrap();
        worker
            .archive
            .set_transcript(PEER, "voice", "saved text", "previous")
            .unwrap();
        worker.transcribing = Some((PEER.into(), "voice".into()));
        worker.transcription_finished(Finished {
            chat: PEER.into(),
            id: "voice".into(),
            model: Model::default(),
            generation: worker.transcription_generation,
            session: worker.star_session,
            text: Err(Error::Audio),
        });
        assert_eq!(
            worker
                .archive
                .transcripts(PEER, &["voice".into()])
                .unwrap()
                .get("voice")
                .map(String::as_str),
            Some("saved text")
        );
        assert!(
            events
                .try_iter()
                .any(|event| matches!(event, Event::Transcription { text: None, .. }))
        );
    }

    #[test]
    fn a_late_result_from_an_old_model_is_not_stored() {
        let (mut worker, events, _, _) = worker();
        worker.transcribing = Some((PEER.into(), "voice".into()));
        worker.transcription_generation = 2;
        worker.transcription_finished(Finished {
            chat: PEER.into(),
            id: "voice".into(),
            model: Model::default(),
            generation: 1,
            session: worker.star_session,
            text: Ok("old result".into()),
        });
        assert!(
            worker
                .archive
                .transcripts(PEER, &["voice".into()])
                .unwrap()
                .is_empty()
        );
        assert!(
            events
                .try_iter()
                .any(|event| matches!(event, Event::Transcription { text: None, .. }))
        );
    }
}
