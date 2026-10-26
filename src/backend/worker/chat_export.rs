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
    /// Cancels the current job and bounds the wait for blocked storage to one second.
    /// A running blocking task retains its cancellation flag and temporary-file cleanup.
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

    /// Marks only the named chat's active export for cancellation without blocking the worker.
    pub(super) fn cancel_chat_export_for(&self, chat: &str) {
        if let Some(job) = &self.chat_export.job
            && job.chat == chat
        {
            job.cancelled.store(true, Ordering::Release);
        }
    }

    /// Checks account and chat permissions before starting an account-owned native save picker.
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

    /// Accepts only the current picker result, rechecks permissions and starts a read-only export.
    /// Cancellation clears the job before inspecting the destination or opening the archive.
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
        if !self.chat_export.allowed || !self.privacy_ready {
            self.chat_export.job = None;
            self.emit(Event::ChatExportFinished {
                request,
                result: Err("Export cancelled".into()),
            });
            return;
        }
        let archive = self.dirs.archive_db();
        let roots = [self.dirs.state.clone(), self.dirs.cache.clone()];
        let commands = self.commands.clone();
        let chat = job.chat.clone();
        let cancelled = job.cancelled.clone();
        job.task = Some(tokio::task::spawn_blocking(move || {
            let result = (|| -> anyhow::Result<_> {
                crate::archive::chat_export::check_cancelled(&cancelled)?;
                let parent = target
                    .parent()
                    .context("Choose an export folder")?
                    .canonicalize()?;
                for root in &roots {
                    let root = root.parent().and_then(Path::parent).unwrap_or(root);
                    if let Ok(root) = root.canonicalize() {
                        anyhow::ensure!(
                            !parent.starts_with(root),
                            "Choose a folder outside ZapFast's application data"
                        );
                    }
                }
                crate::archive::chat_export::check_cancelled(&cancelled)?;
                let reader = Archive::export_reader(&archive)?;
                crate::archive::chat_export::check_cancelled(&cancelled)?;
                reader.export_chat(&chat, &target, include_media, &cancelled, |counts| {
                    let _ = commands.send(Command::ChatExportProgress { request, counts });
                })
            })();
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

    /// Forwards progress only while the originating request is still current.
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

    /// Clears and reports only the current job, ignoring results from superseded requests.
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

    /// A cancelled picker result clears busy state without opening the archive or destination.
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

    /// Archive setup errors return through the blocking task instead of the account worker.
    #[tokio::test]
    async fn unavailable_export_archive_reports_through_the_background_task() {
        let (mut worker, events, mut commands, _) = receipt_tests::worker();
        let directory = tempfile::tempdir().unwrap();
        worker.dirs =
            crate::paths::AppDirs::under(&directory.path().join("app")).account(&worker.dirs.id);
        worker.chat_export.allowed = true;
        worker.chat_export.job = Some(Job {
            request: 9,
            chat: "fixture".into(),
            cancelled: Arc::new(AtomicBool::new(false)),
            task: None,
        });
        let target = directory.path().join("export.txt");
        worker.chat_export_target(9, false, Some(target.clone()));
        assert!(worker.chat_export.job.as_ref().unwrap().task.is_some());
        assert!(events.try_recv().is_err());
        let command = tokio::time::timeout(Duration::from_secs(3), commands.recv())
            .await
            .unwrap()
            .unwrap();
        let Command::ChatExportFinished { request, result } = command else {
            panic!("expected the background export result");
        };
        assert_eq!(request, 9);
        assert_eq!(result.as_ref().unwrap_err(), "The archive is unavailable");
        worker.chat_export_finished(request, result);
        assert!(worker.chat_export.job.is_none());
        assert!(matches!(
            events.try_recv().unwrap(),
            Event::ChatExportFinished {
                request: 9,
                result: Err(_)
            }
        ));
        assert!(!worker.dirs.archive_db().exists());
        assert!(!target.exists());
    }

    /// A stalled blocking writer cannot make cancellation wait indefinitely.
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

    /// Denied exports never open a picker; stale results cannot replace a newer job.
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
