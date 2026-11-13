//! One bounded storage job per account. Encoding runs off the worker, and
//! only the current account/session can publish a verified result.
use super::*;
use crate::smart_storage::{self, Report, Stamp, VideoResult};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
pub(super) struct State {
    enabled: bool,
    due: Option<Instant>,
    sequence: u64,
    job: Option<Job>,
    compaction: Option<tokio::task::JoinHandle<()>>,
}
struct Job {
    generation: u64,
    cutoff: i64,
    cursor: i64,
    raw_done: bool,
    videos: VecDeque<PathBuf>,
    in_flight: bool,
    compacting: bool,
    report: Report,
    cancel: Arc<AtomicBool>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl Worker {
    pub(super) fn storage_compacting(&self) -> bool {
        self.smart_storage
            .job
            .as_ref()
            .is_some_and(|job| job.compacting)
    }
    /// Idle compaction yields to foreground commands and incoming events.
    pub(super) fn storage_foreground_work(&mut self) {
        if let Some(job) = self.smart_storage.job.as_ref().filter(|job| job.compacting) {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
    pub(super) async fn wait_storage_compaction(&mut self) {
        if let Some(task) = self.smart_storage.compaction.take() {
            let _ = task.await;
        }
    }
    pub(super) fn storage_compacted(
        &mut self,
        session: u64,
        generation: u64,
        saved: Result<u64, ()>,
    ) {
        if !self
            .smart_storage
            .job
            .as_ref()
            .is_some_and(|job| session == self.account_session && job.generation == generation)
        {
            return;
        }
        let job = self.smart_storage.job.take().unwrap();
        let mut report = job.report;
        match saved {
            Ok(bytes) => report.saved_bytes += bytes,
            Err(()) if !job.cancel.load(Ordering::Relaxed) => report.failed += 1,
            Err(()) => {}
        }
        self.smart_storage.due = Some(Instant::now() + Duration::from_secs(24 * 60 * 60));
        self.emit(Event::StorageOptimization {
            running: false,
            report,
        });
    }
    pub(super) fn configure_storage(&mut self, enabled: bool) {
        if self.smart_storage.enabled != enabled {
            self.smart_storage.enabled = enabled;
            if enabled {
                self.smart_storage.due = Some(Instant::now());
            } else {
                self.stop_storage();
            }
        }
    }
    pub(super) fn stop_storage(&mut self) {
        if let Some(job) = self.smart_storage.job.take() {
            self.emit(Event::StorageOptimization {
                running: false,
                report: job.report,
            });
        }
        self.smart_storage.due = Some(Instant::now() + Duration::from_secs(24 * 60 * 60));
    }
    pub(super) fn start_storage(&mut self) {
        if self.smart_storage.job.is_some() {
            return;
        }
        if !self.privacy_ready {
            self.emit(Event::StorageOptimization {
                running: false,
                report: Report {
                    failed: 1,
                    ..Default::default()
                },
            });
            return;
        }
        // Remove only our abandoned temporary encodes, before starting a job.
        if let Ok(entries) = std::fs::read_dir(self.dirs.media_cache_dir()) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with(".zapfast-optimize-")
                    && name.ends_with(".mp4")
                    && entry.file_type().is_ok_and(|kind| kind.is_file())
                {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        let cutoff = crate::util::now() - smart_storage::AGE;
        let videos = match self.archive.old_cached_videos(cutoff) {
            Ok(videos) => videos.into(),
            Err(_) => {
                self.emit(Event::StorageOptimization {
                    running: false,
                    report: Report {
                        failed: 1,
                        ..Default::default()
                    },
                });
                return;
            }
        };
        self.smart_storage.sequence += 1;
        self.smart_storage.job = Some(Job {
            generation: self.smart_storage.sequence,
            cutoff,
            cursor: 0,
            raw_done: false,
            videos,
            in_flight: false,
            compacting: false,
            report: Report::default(),
            cancel: Arc::new(AtomicBool::new(false)),
        });
        self.emit(Event::StorageOptimization {
            running: true,
            report: Report::default(),
        });
    }
    pub(super) fn pump_storage(&mut self) {
        // Stop can detach a canceled job before its blocking task has exited.
        // Retain its handle until it is finished; logout/shutdown must join it.
        if self
            .smart_storage
            .compaction
            .as_ref()
            .is_some_and(|task| !task.is_finished())
        {
            return;
        }
        if !self.privacy_ready || self.syncing || self.call.is_some() {
            return;
        }
        if self.smart_storage.job.is_none()
            && self.smart_storage.enabled
            && self
                .smart_storage
                .due
                .is_some_and(|due| due <= Instant::now())
        {
            self.start_storage();
        }
        let Some(job) = self.smart_storage.job.as_mut() else {
            return;
        };
        if job.in_flight {
            return;
        }
        if !job.raw_done {
            let rows = match self.archive.storage_raw_batch(job.cutoff, job.cursor) {
                Ok(rows) => rows,
                Err(_) => {
                    job.report.failed += 1;
                    job.raw_done = true;
                    return;
                }
            };
            if rows.is_empty() {
                job.raw_done = true;
            } else {
                job.in_flight = true;
                job.cursor = rows.last().map_or(job.cursor, |row| row.rowid);
                let generation = job.generation;
                let session = self.account_session;
                let commands = self.commands.clone();
                let cancel = job.cancel.clone();
                tokio::task::spawn_blocking(move || {
                    let mut packed = Vec::new();
                    let mut failed = 0;
                    for row in rows {
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        match smart_storage::pack(&row.raw) {
                            Ok(Some(bytes)) => packed.push((row, bytes)),
                            Ok(None) => {}
                            Err(_) => failed += 1,
                        }
                    }
                    let _ = commands.send(Command::StorageRawDone {
                        session,
                        generation,
                        packed,
                        failed,
                    });
                });
                return;
            }
        }
        // Skip unavailable or already-optimized files without encoding again.
        for _ in 0..32 {
            let Some(path) = job.videos.pop_front() else {
                if self.archive.compaction_due().unwrap_or(false) {
                    job.in_flight = true;
                    job.compacting = true;
                    let generation = job.generation;
                    let session = self.account_session;
                    let cancel = job.cancel.clone();
                    let path = self.dirs.archive_db();
                    let commands = self.commands.clone();
                    self.smart_storage.compaction = Some(tokio::task::spawn_blocking(move || {
                        let size = || {
                            [&path, &path.with_extension("db-wal")]
                                .into_iter()
                                .filter_map(|path| std::fs::metadata(path).ok())
                                .map(|meta| meta.len())
                                .sum::<u64>()
                        };
                        let before = size();
                        let saved = (|| {
                            if cancel.load(Ordering::Relaxed) {
                                return Err(());
                            }
                            let archive = crate::archive::Archive::open(&path).map_err(|_| ())?;
                            archive
                                .reclaim_storage_cancelable(cancel.clone())
                                .map_err(|_| ())?;
                            Ok(before.saturating_sub(size()))
                        })();
                        let _ = commands.send(Command::StorageCompacted {
                            session,
                            generation,
                            saved,
                        });
                    }));
                } else {
                    let job = self.smart_storage.job.take().unwrap();
                    self.smart_storage.due =
                        Some(Instant::now() + Duration::from_secs(24 * 60 * 60));
                    self.emit(Event::StorageOptimization {
                        running: false,
                        report: job.report,
                    });
                }
                return;
            };
            let Ok(stamp) = Stamp::of(&path) else {
                continue;
            };
            if self.archive.video_optimized(&path, &stamp).unwrap_or(false) {
                continue;
            }
            job.in_flight = true;
            let generation = job.generation;
            let session = self.account_session;
            let cancel = job.cancel.clone();
            let commands = self.commands.clone();
            let cache = self.dirs.media_cache_dir();
            tokio::task::spawn_blocking(move || {
                let result =
                    smart_storage::prepare_video(path.clone(), &cache, stamp.clone(), cancel);
                let _ = commands.send(Command::StorageVideoDone {
                    session,
                    generation,
                    path,
                    stamp,
                    result,
                });
            });
            return;
        }
    }
    pub(super) fn storage_raw_done(
        &mut self,
        session: u64,
        generation: u64,
        packed: Vec<(crate::archive::RawRow, Vec<u8>)>,
        failed: u64,
    ) {
        let Some(job) = self
            .smart_storage
            .job
            .as_mut()
            .filter(|job| session == self.account_session && job.generation == generation)
        else {
            return;
        };
        job.in_flight = false;
        job.report.failed += failed;
        match self.archive.store_packed_batch(&packed) {
            Ok((messages, bytes)) => {
                job.report.messages += messages;
                job.report.packed_bytes += bytes;
            }
            Err(_) => job.report.failed += packed.len() as u64,
        }
        let report = job.report;
        self.emit(Event::StorageOptimization {
            running: true,
            report,
        });
        self.pump_storage();
    }
    pub(super) fn storage_video_done(
        &mut self,
        session: u64,
        generation: u64,
        path: PathBuf,
        stamp: Stamp,
        result: VideoResult,
    ) {
        let Some(job) = self
            .smart_storage
            .job
            .as_mut()
            .filter(|job| session == self.account_session && job.generation == generation)
        else {
            return;
        };
        job.in_flight = false;
        if !self
            .archive
            .video_is_old(&path, job.cutoff)
            .unwrap_or(false)
        {
            return;
        }
        let mark = match result {
            VideoResult::Ready(video) => match video.publish(&self.dirs.media_cache_dir()) {
                Ok(saved) => {
                    if saved > 0 {
                        job.report.videos += 1;
                        job.report.saved_bytes += saved;
                    }
                    saved > 0
                }
                Err(_) => {
                    job.report.failed += 1;
                    false
                }
            },
            VideoResult::Skipped => Stamp::of(&path).ok().as_ref() == Some(&stamp),
            VideoResult::Unavailable => {
                job.report.video_unavailable = true;
                job.videos.clear();
                false
            }
            VideoResult::Failed => {
                job.report.failed += 1;
                false
            }
        };
        if mark && let Ok(stamp) = Stamp::of(&path) {
            let _ = self.archive.mark_video_optimized(&path, &stamp);
        }
        let report = job.report;
        self.emit(Event::StorageOptimization {
            running: true,
            report,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn restarting_after_stop_keeps_the_previous_compaction_handle() {
        let (mut worker, _, _, _) = super::super::receipt_tests::worker();
        let previous = tokio::spawn(std::future::pending::<()>());
        let previous_id = previous.id();
        worker.smart_storage.compaction = Some(previous);
        worker
            .archive
            .set_meta("storage_compaction_due", "1")
            .unwrap();
        worker.start_storage();
        worker.smart_storage.job.as_mut().unwrap().raw_done = true;
        worker.pump_storage();
        assert_eq!(
            worker.smart_storage.compaction.as_ref().unwrap().id(),
            previous_id
        );
        assert!(!worker.smart_storage.job.as_ref().unwrap().in_flight);
        worker.smart_storage.compaction.as_ref().unwrap().abort();
        worker.stop_storage();
        worker.wait_storage_compaction().await;
        assert!(worker.smart_storage.compaction.is_none());
    }

    #[test]
    fn foreground_work_cancels_idle_compaction_and_stale_results_are_ignored() {
        let (mut worker, events, _, _) = super::super::receipt_tests::worker();
        worker.start_storage();
        let job = worker.smart_storage.job.as_mut().unwrap();
        job.compacting = true;
        job.in_flight = true;
        job.report.messages = 2;
        job.report.packed_bytes = 1000;
        let generation = job.generation;
        let cancel = job.cancel.clone();
        worker.storage_compacted(worker.account_session + 1, generation, Ok(999));
        assert!(worker.smart_storage.job.is_some());
        worker.storage_foreground_work();
        assert!(cancel.load(Ordering::Relaxed));
        worker.storage_compacted(worker.account_session, generation, Err(()));
        assert!(worker.smart_storage.job.is_none());
        let reports = events
            .try_iter()
            .filter_map(|event| match event {
                Event::StorageOptimization {
                    running: false,
                    report,
                } => Some(report),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].messages, 2);
        assert_eq!(reports[0].packed_bytes, 1000);
        assert_eq!(reports[0].saved_bytes, 0);
        assert_eq!(reports[0].failed, 0);
        worker.storage_compacted(worker.account_session, generation, Ok(999));
        assert!(events.try_iter().next().is_none());
    }
}
