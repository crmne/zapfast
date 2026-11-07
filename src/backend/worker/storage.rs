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
}
struct Job {
    generation: u64,
    cutoff: i64,
    cursor: i64,
    raw_done: bool,
    videos: VecDeque<PathBuf>,
    in_flight: bool,
    report: Report,
    cancel: Arc<AtomicBool>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl Worker {
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
            report: Report::default(),
            cancel: Arc::new(AtomicBool::new(false)),
        });
        self.emit(Event::StorageOptimization {
            running: true,
            report: Report::default(),
        });
    }
    pub(super) fn pump_storage(&mut self) {
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
                let job = self.smart_storage.job.take().unwrap();
                let mut report = job.report;
                if report.messages > 0 && self.archive.reclaim_storage().is_err() {
                    report.failed += 1;
                }
                self.smart_storage.due = Some(Instant::now() + Duration::from_secs(24 * 60 * 60));
                self.emit(Event::StorageOptimization {
                    running: false,
                    report,
                });
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
        for (row, packed) in packed {
            match self.archive.store_packed_raw(&row, &packed) {
                Ok(true) => {
                    job.report.messages += 1;
                    job.report.saved_bytes += (row.raw.len() - packed.len()) as u64;
                }
                Ok(false) => {}
                Err(_) => job.report.failed += 1,
            }
        }
        let report = job.report;
        self.emit(Event::StorageOptimization {
            running: true,
            report,
        });
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
