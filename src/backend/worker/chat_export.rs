//! Account-owned export lifetime; file work uses its own read-only archive connection.
use super::*;
use crate::archive::chat_export::Counts;
use anyhow::Context;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
pub(super) struct State {
    pub allowed: bool,
    job: Option<Job>,
}

struct Job {
    request: u64,
    chat: ChatId,
    cancelled: Arc<AtomicBool>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancelled.store(true, Ordering::Release);
        }
    }
}

impl Worker {
    pub(super) async fn stop_chat_export(&mut self) {
        if let Some(job) = self.chat_export.job.take() {
            job.cancelled.store(true, Ordering::Release);
            let result = if let Some(mut task) = job.task {
                match tokio::time::timeout(Duration::from_secs(1), &mut task).await {
                    Ok(Ok(())) => Ok(None),
                    Ok(Err(_)) => Err("The export stopped unexpectedly".into()),
                    Err(_) => {
                        // Queued blocking tasks can be aborted; running ones retain their
                        // cancellation flag and temporary-file cleanup until I/O returns.
                        task.abort();
                        Err(
                            "Export cancellation is pending while storage finishes responding"
                                .into(),
                        )
                    }
                }
            } else {
                Ok(None)
            };
            self.emit(Event::ChatExportFinished {
                request: job.request,
                result,
            });
        }
    }

    pub(super) fn cancel_chat_export_for(&self, chat: &str) {
        if let Some(job) = &self.chat_export.job
            && job.chat == chat
        {
            job.cancelled.store(true, Ordering::Release);
        }
    }

    pub(super) async fn begin_chat_export(
        &mut self,
        request: u64,
        chat: ChatId,
        include_media: bool,
    ) {
        self.stop_chat_export().await;
        let chat = self.canonical_str(&chat);
        if !self.chat_export.allowed
            || !self.privacy_ready
            || !self
                .archive
                .chat(&chat)
                .ok()
                .flatten()
                .is_some_and(|chat| !chat.locked)
        {
            self.emit(Event::ChatExportFinished {
                request,
                result: Err("This chat is not available for export".into()),
            });
            return;
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        self.chat_export.job = Some(Job {
            request,
            chat,
            cancelled,
            task: None,
        });
        let commands = self.commands.clone();
        tokio::task::spawn_blocking(move || {
            let extension = if include_media { "zip" } else { "txt" };
            let target = rfd::FileDialog::new()
                .set_title("Export chat")
                .set_file_name(format!("Chat export.{extension}"))
                .add_filter(
                    if include_media {
                        "Chat with attachments"
                    } else {
                        "Text file"
                    },
                    &[extension],
                )
                .save_file();
            let _ = commands.send(Command::ChatExportTarget {
                request,
                include_media,
                target,
            });
        });
    }

    pub(super) fn chat_export_target(
        &mut self,
        request: u64,
        include_media: bool,
        target: Option<PathBuf>,
    ) {
        let Some(job) = self
            .chat_export
            .job
            .as_mut()
            .filter(|job| job.request == request)
        else {
            return;
        };
        if job.task.is_some() {
            return;
        }
        if job.cancelled.load(Ordering::Acquire) {
            self.chat_export.job = None;
            self.emit(Event::ChatExportFinished {
                request,
                result: Ok(None),
            });
            return;
        }
        let Some(target) = target else {
            self.chat_export.job = None;
            self.emit(Event::ChatExportFinished {
                request,
                result: Ok(None),
            });
            return;
        };
        let reader = (|| -> anyhow::Result<_> {
            anyhow::ensure!(
                self.chat_export.allowed
                    && self.privacy_ready
                    && !job.cancelled.load(Ordering::Acquire),
                "Export cancelled"
            );
            let parent = target
                .parent()
                .context("Choose an export folder")?
                .canonicalize()?;
            for root in [&self.dirs.state, &self.dirs.cache] {
                let root = root.parent().and_then(Path::parent).unwrap_or(root);
                if let Ok(root) = root.canonicalize() {
                    anyhow::ensure!(
                        !parent.starts_with(root),
                        "Choose a folder outside ZapFast's application data"
                    );
                }
            }
            self.archive.export_reader()
        })();
        let reader = match reader {
            Ok(reader) => reader,
            Err(error) => {
                self.chat_export.job = None;
                self.emit(Event::ChatExportFinished {
                    request,
                    result: Err(error.to_string()),
                });
                return;
            }
        };
        let commands = self.commands.clone();
        let chat = job.chat.clone();
        let cancelled = job.cancelled.clone();
        job.task = Some(tokio::task::spawn_blocking(move || {
            let result = reader.export_chat(&chat, &target, include_media, &cancelled, |counts| {
                let _ = commands.send(Command::ChatExportProgress { request, counts });
            });
            let result = if cancelled.load(Ordering::Acquire) {
                Ok(None)
            } else {
                result
                    .map(|counts| Some((target, counts)))
                    .map_err(|error| error.to_string())
            };
            let _ = commands.send(Command::ChatExportFinished { request, result });
        }));
    }

    pub(super) fn chat_export_progress(&self, request: u64, counts: Counts) {
        if self
            .chat_export
            .job
            .as_ref()
            .is_some_and(|job| job.request == request)
        {
            self.emit(Event::ChatExportProgress { request, counts });
        }
    }

    pub(super) fn chat_export_finished(
        &mut self,
        request: u64,
        result: Result<Option<(PathBuf, Counts)>, String>,
    ) {
        if self
            .chat_export
            .job
            .as_ref()
            .is_some_and(|job| job.request == request)
        {
            self.chat_export.job = None;
            self.emit(Event::ChatExportFinished { request, result });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::receipt_tests;
    use super::*;

    #[test]
    fn a_cancelled_picker_result_finishes_without_opening_the_archive_or_target() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        worker.chat_export.job = Some(Job {
            request: 7,
            chat: "fixture".into(),
            cancelled: Arc::new(AtomicBool::new(false)),
            task: None,
        });
        worker.cancel_chat_export_for("fixture");
        worker.chat_export_target(7, true, Some(PathBuf::from("not-opened.zip")));
        assert!(worker.chat_export.job.is_none());
        assert!(matches!(
            events.try_recv().unwrap(),
            Event::ChatExportFinished {
                request: 7,
                result: Ok(None)
            }
        ));
        worker.chat_export_target(7, true, None);
        assert!(events.try_recv().is_err(), "a late duplicate stays stale");
    }

    #[tokio::test]
    async fn cancellation_does_not_wait_forever_for_blocking_storage() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let (release, blocked) = std::sync::mpsc::channel();
        struct Release(std::sync::mpsc::Sender<()>);
        impl Drop for Release {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let release = Release(release);
        let (started, ready) = tokio::sync::oneshot::channel();
        let (finished, done) = tokio::sync::oneshot::channel();
        let task = tokio::task::spawn_blocking(move || {
            let _ = started.send(());
            let _ = blocked.recv();
            let _ = finished.send(flag.load(Ordering::Acquire));
        });
        ready.await.unwrap();
        worker.chat_export.job = Some(Job {
            request: 8,
            chat: "fixture".into(),
            cancelled: cancelled.clone(),
            task: Some(task),
        });
        tokio::time::timeout(Duration::from_secs(3), worker.stop_chat_export())
            .await
            .unwrap();
        assert!(cancelled.load(Ordering::Acquire));
        assert!(worker.chat_export.job.is_none());
        assert!(matches!(
            events.try_recv().unwrap(),
            Event::ChatExportFinished {
                request: 8,
                result: Err(_)
            }
        ));
        drop(release);
        assert!(
            tokio::time::timeout(Duration::from_secs(3), done)
                .await
                .unwrap()
                .unwrap()
        );
    }

    #[tokio::test]
    async fn withheld_exports_never_open_a_picker_and_stale_callbacks_leave_new_work_alone() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        worker.begin_chat_export(1, "fixture".into(), true).await;
        assert!(worker.chat_export.job.is_none());
        assert!(matches!(
            events.try_recv().unwrap(),
            Event::ChatExportFinished {
                request: 1,
                result: Err(_)
            }
        ));
        let cancelled = Arc::new(AtomicBool::new(false));
        worker.chat_export.job = Some(Job {
            request: 3,
            chat: "fixture".into(),
            cancelled: cancelled.clone(),
            task: None,
        });
        worker.chat_export_finished(2, Ok(None));
        worker.chat_export_progress(2, Counts::default());
        assert_eq!(worker.chat_export.job.as_ref().unwrap().request, 3);
        assert!(events.try_recv().is_err());
        worker.cancel_chat_export_for("other");
        assert!(!cancelled.load(Ordering::Acquire));
        worker.stop_chat_export().await;
        assert!(cancelled.load(Ordering::Acquire));
        assert!(worker.chat_export.job.is_none());
    }
}
