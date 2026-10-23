//! Persistent media recovery independent of visible chats and user exports.

use super::*;
use crate::archive::MediaJob;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MediaDiagnostic {
    enabled: bool,
    user_paused: bool,
    privacy_paused: bool,
    connected: bool,
    runnable: bool,
    phone_runnable: bool,
    phone_recovery: bool,
    large_files: bool,
    target_since: i64,
    cdn_pending: u64,
    needs_phone: u64,
    downloaded: u64,
    unavailable: u64,
    policy_too_large: u64,
    cancelled: u64,
    schedule_known: bool,
    cdn_due: u64,
    phone_due: u64,
    active: u64,
    retry_at: i64,
    phone_quota_retry_at: i64,
    bytes: Option<u64>,
    budget_mib: u32,
    budget_reached: bool,
}

impl MediaDiagnostic {
    fn line(&self, account: u32, snapshot_at: i64) -> String {
        format!(
            "[MEDIA_JOB] version=1 account={account} snapshot_at={snapshot_at} enabled={} user_paused={} privacy_paused={} connected={} runnable={} phone_runnable={} phone_recovery={} large_files={} target_since={} cdn_pending={} needs_phone={} downloaded={} unavailable={} policy_too_large={} cancelled={} schedule_known={} cdn_due={} phone_due={} active={} retry_at={} phone_quota_retry_at={} bytes={} budget_mib={} budget_reached={}",
            u8::from(self.enabled),
            u8::from(self.user_paused),
            u8::from(self.privacy_paused),
            u8::from(self.connected),
            u8::from(self.runnable),
            u8::from(self.phone_runnable),
            u8::from(self.phone_recovery),
            u8::from(self.large_files),
            self.target_since,
            self.cdn_pending,
            self.needs_phone,
            self.downloaded,
            self.unavailable,
            self.policy_too_large,
            self.cancelled,
            u8::from(self.schedule_known),
            self.cdn_due,
            self.phone_due,
            self.active,
            self.retry_at,
            self.phone_quota_retry_at,
            self.bytes.map_or(-1_i128, i128::from),
            self.budget_mib,
            u8::from(self.budget_reached),
        )
    }

    fn emit(&self, account: u32, snapshot_at: i64) {
        log::info!("{}", self.line(account, snapshot_at));
    }
}

impl Worker {
    pub(super) fn start_media_usage_scan(&mut self) {
        if self.media_scan_in_flight.is_some() {
            return;
        }
        self.media_scan_generation = self.media_scan_generation.wrapping_add(1);
        let generation = self.media_scan_generation;
        self.media_scan_in_flight = Some(generation);
        self.media_scan_started_bytes = self.media_scan_completed_bytes;
        let root = self.dirs.media_cache_dir();
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                crate::archive::storage::scan_managed_media(&root)
                    .map_err(|error| error.to_string())
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result);
            let _ = commands.send(Command::MediaScanFinished { generation, result });
        });
    }

    pub(super) fn apply_media_scan_finished(
        &mut self,
        generation: u64,
        result: Result<crate::archive::MediaScanResult, String>,
    ) {
        if self.media_scan_in_flight != Some(generation) {
            return;
        }
        self.media_scan_in_flight = None;
        let completed_since_start = self
            .media_scan_completed_bytes
            .saturating_sub(self.media_scan_started_bytes);
        self.background_media_usage = Some((
            Instant::now(),
            result
                .ok()
                .map(|scan| scan.bytes.saturating_add(completed_since_start)),
        ));
    }

    fn background_media_size(&self, job: &MediaJob, unknown: u64) -> u64 {
        let Ok(Some(mut message)) = self.archive.message(&job.chat, &job.id) else {
            return unknown;
        };
        if job.card == -2 {
            return unknown;
        }
        message
            .content
            .media_at_mut((job.card >= 0).then_some(job.card as usize))
            .map_or(
                unknown,
                |media| {
                    if media.size == 0 { unknown } else { media.size }
                },
            )
    }

    fn background_media_reserved_bytes(&self, unknown: u64) -> u64 {
        self.background_media
            .iter()
            .map(|(chat, id, card)| {
                self.background_media_size(
                    &MediaJob {
                        chat: chat.clone(),
                        id: id.clone(),
                        card: *card,
                    },
                    unknown,
                )
            })
            .fold(0, u64::saturating_add)
    }

    fn background_media_transfer_active(&self, job: &MediaJob) -> bool {
        if job.card == -2 {
            self.motion_downloads
                .contains(&(job.chat.clone(), job.id.clone()))
        } else {
            self.downloads.contains(&(
                job.chat.clone(),
                job.id.clone(),
                (job.card >= 0).then_some(job.card as usize),
            ))
        }
    }

    pub(super) async fn pump_background_media(&mut self) {
        if self
            .background_media_usage
            .as_ref()
            .is_none_or(|(checked, _)| checked.elapsed() >= Duration::from_secs(30))
        {
            self.start_media_usage_scan();
        }
        let bytes = self
            .background_media_usage
            .as_ref()
            .and_then(|(_, bytes)| *bytes);
        let limit = u64::from(self.background_media_budget_mib) * 1024 * 1024;
        let download_limit = if self.background_media_large_files {
            crate::model::MANUAL_DOWNLOAD_LIMIT
        } else {
            ATTACHMENT_DOWNLOAD_LIMIT
        };
        let reserved = self.background_media_reserved_bytes(download_limit);
        let budget_reached =
            bytes.is_none_or(|bytes| limit != 0 && bytes.saturating_add(reserved) >= limit);
        // Accounts which enabled archival media before date-scoped recovery keep
        // their original unscoped behaviour.
        let target_since = self.background_media_target_since.unwrap_or(i64::MIN);
        if let Ok(counts) = self.archive.media_import_counts(target_since) {
            let now = crate::util::now();
            let schedule = self.archive.media_import_schedule(target_since, now);
            let schedule_known = schedule.is_ok();
            let schedule = schedule.unwrap_or_default();
            let phone_quota_retry_at = match self
                .archive
                .media_phone_quota_available(now, self.background_media_phone_requests_per_day)
            {
                Ok(true) => 0,
                Ok(false) => now.div_euclid(86_400).saturating_add(1) * 86_400,
                Err(_) => -1,
            };
            let runnable = self.background_media_enabled
                && !self.background_history.paused
                && self.privacy_ready
                && self.status.is_connected()
                && self.client.is_some();
            let phone_runnable = runnable
                && self.background_media_phone_recovery
                && !self.syncing
                && self.pending_older.is_empty()
                && !self.poll_history.active()
                && self
                    .automatic_history_last_request
                    .is_none_or(|last| last.elapsed() >= Duration::from_secs(20))
                && self
                    .automatic_history
                    .media_ready(self.background_media_phone_requests_per_day, now);
            let media_diagnostic = MediaDiagnostic {
                enabled: self.background_media_enabled,
                user_paused: self.history_policy_paused,
                privacy_paused: self.history_privacy_paused,
                connected: self.status.is_connected(),
                runnable,
                phone_runnable,
                phone_recovery: self.background_media_phone_recovery,
                large_files: self.background_media_large_files,
                target_since,
                cdn_pending: counts.cdn_pending,
                needs_phone: counts.needs_phone,
                downloaded: counts.downloaded,
                unavailable: counts.unavailable,
                policy_too_large: counts.policy_too_large,
                cancelled: counts.cancelled,
                schedule_known,
                cdn_due: schedule.cdn_due,
                phone_due: schedule.phone_due,
                active: self.background_media.len() as u64,
                retry_at: schedule.retry_at.unwrap_or(0),
                phone_quota_retry_at,
                bytes,
                budget_mib: self.background_media_budget_mib,
                budget_reached,
            };
            let media_changed = self
                .background_media_diagnostic
                .as_ref()
                .is_none_or(|(_, previous)| previous != &media_diagnostic);
            let media_heartbeat = self
                .background_media_diagnostic
                .as_ref()
                .is_none_or(|(emitted, _)| emitted.elapsed() >= Duration::from_secs(60));
            if media_changed || media_heartbeat {
                media_diagnostic.emit(self.dirs.id.as_str().parse().unwrap_or(0), now);
                self.log_history_job_status();
                self.background_media_diagnostic = Some((Instant::now(), media_diagnostic));
            }
            let mut history: crate::model::HistoryImportProgress = self
                .archive
                .history_import_progress(now)
                .map(Into::into)
                .unwrap_or_default();
            if matches!(
                history.state,
                crate::model::ImportRunState::Running | crate::model::ImportRunState::BackingOff
            ) && let Some(retry_at) = self
                .automatic_history
                .history_daily_retry_at(self.background_history.daily_budget, now)
            {
                if history.retry_at.is_none_or(|at| retry_at >= at) {
                    history.reason = None;
                }
                history.state = crate::model::ImportRunState::BackingOff;
                history.retry_at = Some(history.retry_at.unwrap_or(0).max(retry_at));
            }
            if self.syncing
                && history.target_since.is_some()
                && history.state == crate::model::ImportRunState::Running
            {
                history.state = crate::model::ImportRunState::WaitingForInitialSync;
            }
            self.emit(Event::ArchiveProgress(
                crate::model::ArchiveRecoveryProgress {
                    history,
                    media: crate::model::MediaImportProgress {
                        cdn_pending: counts.cdn_pending,
                        needs_phone: counts.needs_phone,
                        downloaded: counts.downloaded,
                        unavailable: counts.unavailable,
                        policy_too_large: counts.policy_too_large,
                        cancelled: counts.cancelled,
                        bytes,
                        budget_reached,
                    },
                    connected: self.status.is_connected(),
                },
            ));
        }
        if !self.background_media_enabled
            || self.background_history.paused
            || !self.privacy_ready
            || !self.status.is_connected()
            || self.client.is_none()
        {
            return;
        }
        if self.archive.seed_media_jobs().is_err() {
            log::warn!("could not seed background media jobs");
            return;
        }
        self.audit_completed_media().await;
        if budget_reached {
            return;
        }
        let available = 2_usize.saturating_sub(self.background_media.len());
        if available == 0 || self.downloads.len() >= 6 {
            return;
        }
        let now = crate::util::now();
        self.pump_background_phone_media(now);
        let jobs = match self.archive.due_media_jobs_since(
            now,
            target_since,
            available + self.downloads.len(),
        ) {
            Ok(jobs) => jobs,
            Err(_) => {
                log::warn!("could not read background media jobs");
                return;
            }
        };
        for job in jobs {
            if self.background_media.len() >= 2 || self.downloads.len() >= 6 {
                break;
            }
            let card = (job.card >= 0).then_some(job.card as usize);
            if self
                .background_media
                .contains(&(job.chat.clone(), job.id.clone(), job.card))
                || self.background_media_transfer_active(&job)
            {
                continue;
            }
            let mut message = match self.archive.message(&job.chat, &job.id) {
                Ok(Some(message)) => message,
                Ok(None) => continue,
                Err(_) => {
                    log::warn!("could not read background media metadata");
                    break;
                }
            };
            let declared = self.background_media_size(&job, download_limit);
            if declared > download_limit {
                if self.archive.mark_media_job_policy_too_large(&job).is_err() {
                    log::warn!("could not preserve background media size exclusion");
                }
                continue;
            }
            if limit != 0
                && bytes
                    .unwrap_or(0)
                    .saturating_add(self.background_media_reserved_bytes(download_limit))
                    .saturating_add(declared)
                    > limit
            {
                break;
            }
            let path = if job.card == -2 {
                match &message.content {
                    Content::Image {
                        motion: Some(motion),
                        ..
                    } => motion.path.as_ref(),
                    _ => None,
                }
            } else {
                message
                    .content
                    .media_at_mut(card)
                    .and_then(|media| media.path.as_ref())
            };
            if path.is_some_and(|path| path.is_file()) {
                if self
                    .archive
                    .finish_media_job(&job, now, true, false)
                    .is_err()
                {
                    log::warn!("could not complete cached background media job");
                }
                continue;
            }
            if self.archive.start_media_job(&job, now).is_err() {
                log::warn!("could not persist background media attempt");
                break;
            }
            self.background_media
                .insert((job.chat.clone(), job.id.clone(), job.card));
            if job.card == -2 {
                self.download_motion_with_limit(job.chat, job.id, declared);
            } else {
                self.download_media_managed_with_limit(job.chat, job.id, card, declared);
            }
        }
    }

    fn pump_background_phone_media(&mut self, now: i64) {
        if !self.background_media_phone_recovery
            || !self.background_media.is_empty()
            || self.downloads.len() >= 6
        {
            return;
        }
        let target_since = self.background_media_target_since.unwrap_or(i64::MIN);
        let Ok(Some(job)) = self
            .archive
            .due_phone_media_jobs(now, target_since, 1)
            .map(|mut jobs| jobs.pop())
        else {
            return;
        };
        if self.background_media_transfer_active(&job) {
            return;
        }
        let download_limit = if self.background_media_large_files {
            crate::model::MANUAL_DOWNLOAD_LIMIT
        } else {
            ATTACHMENT_DOWNLOAD_LIMIT
        };
        let admitted = self.background_media_size(&job, download_limit);
        let budget = u64::from(self.background_media_budget_mib) * 1024 * 1024;
        let used = self
            .background_media_usage
            .as_ref()
            .and_then(|(_, bytes)| *bytes);
        if admitted > download_limit
            || used.is_none_or(|used| budget != 0 && used.saturating_add(admitted) > budget)
        {
            if admitted > download_limit
                && self.archive.mark_media_job_policy_too_large(&job).is_err()
            {
                log::warn!("could not preserve background phone-media size exclusion");
            }
            return;
        }
        if !self
            .archive
            .media_phone_quota_available(now, self.background_media_phone_requests_per_day)
            .unwrap_or(false)
            || !self.reserve_automatic_media_phone(self.background_media_phone_requests_per_day)
            || !self
                .archive
                .reserve_phone_media_job(&job, now, self.background_media_phone_requests_per_day)
                .unwrap_or(false)
        {
            return;
        }
        let key = (job.chat.clone(), job.id.clone(), job.card);
        self.background_media_phone_permits.insert(key.clone());
        self.background_media.insert(key);
        if job.card == -2 {
            self.download_motion_with_limit(job.chat, job.id, admitted);
        } else {
            self.download_media_managed_with_limit(
                job.chat.clone(),
                job.id.clone(),
                (job.card >= 0).then_some(job.card as usize),
                admitted,
            );
        }
    }

    pub(super) async fn audit_completed_media(&mut self) {
        // Keep the same account, lock, privacy and connection gates as scheduling.
        if !self.background_media_enabled
            || self.background_history.paused
            || !self.privacy_ready
            || !self.status.is_connected()
            || self.client.is_none()
        {
            return;
        }
        let jobs = match self.archive.completed_media_audit_batch(crate::util::now()) {
            Ok(jobs) => jobs,
            Err(_) => {
                log::warn!("could not read completed media audit");
                return;
            }
        };
        let root = self.dirs.media_cache_dir();
        for (rowid, job) in jobs {
            let mut message = match self.archive.message(&job.chat, &job.id) {
                Ok(Some(message)) => message,
                Ok(None) => continue,
                Err(_) => {
                    log::warn!("could not read completed media metadata");
                    return;
                }
            };
            let supported = if job.card == -2 {
                match &message.content {
                    Content::Image {
                        motion: Some(motion),
                        ..
                    } => Some(motion.path.clone()),
                    _ => None,
                }
            } else {
                message
                    .content
                    .media_at_mut((job.card >= 0).then_some(job.card as usize))
                    .map(|media| media.path.clone())
            };
            if let Some(path) = supported {
                let missing = match &path {
                    None => true,
                    Some(path) => {
                        let (root, path) = (root.clone(), path.clone());
                        tokio::task::spawn_blocking(move || managed_file_missing(&root, &path))
                            .await
                            .is_ok_and(|result| result.is_ok_and(|missing| missing))
                    }
                };
                if missing {
                    match self.archive.requeue_missing_media(&job, path.as_deref()) {
                        Ok(true) => {
                            self.background_media_usage = None;
                            self.emit_message(&job.chat, &job.id);
                        }
                        Ok(false) => {}
                        Err(_) => {
                            log::warn!("could not requeue missing managed media");
                            return;
                        }
                    }
                }
            }
            if self.archive.advance_completed_media_audit(rowid).is_err() {
                log::warn!("could not preserve completed media audit cursor");
                return;
            }
        }
    }

    pub(super) fn background_media_finished(
        &mut self,
        chat: &str,
        id: &str,
        card: i64,
        result: &Result<PathBuf, String>,
    ) {
        let key = (chat.to_owned(), id.to_owned(), card);
        self.background_media_phone_permits.remove(&key);
        let owned = self.background_media.remove(&key);
        let user_cancelled = self.background_media_cancelled.remove(&key);
        if !owned && result.is_err() {
            return;
        }
        if let Ok(path) = result
            && path.starts_with(self.dirs.media_cache_dir())
            && let Ok(metadata) = std::fs::metadata(path)
        {
            self.media_scan_completed_bytes = self
                .media_scan_completed_bytes
                .saturating_add(metadata.len());
            self.background_media_usage =
                self.background_media_usage.and_then(|(checked, bytes)| {
                    bytes.map(|bytes| (checked, Some(bytes.saturating_add(metadata.len()))))
                });
        }
        let policy_too_large = result.as_ref().err().is_some_and(|error| {
            error.contains("exceeds the download limit") || error == ATTACHMENT_LIMIT_ERROR
        });
        let unavailable = result.as_ref().err().is_some_and(|error| {
            error.contains("No longer available")
                || error.contains("download keys are missing")
                || error.contains("without its original download identity")
                || error.contains("no downloadable")
        });
        let job = MediaJob {
            chat: chat.to_owned(),
            id: id.to_owned(),
            card,
        };
        if owned && user_cancelled {
            if self.archive.pause_media_job(&job).is_err() {
                log::warn!("could not preserve background media cancellation");
            }
            return;
        }
        let phone_recovery = owned && self.archive.media_job_needs_phone(&job).unwrap_or(false);
        if phone_recovery {
            if result.is_ok() {
                self.automatic_phone_succeeded();
            }
            let saved = if policy_too_large {
                self.archive.mark_media_job_policy_too_large(&job)
            } else {
                self.archive.finish_phone_media_job(
                    &job,
                    crate::util::now(),
                    result.is_ok(),
                    unavailable,
                )
            };
            if saved.is_err() {
                log::warn!("could not persist background phone-media result");
            }
            return;
        }
        if owned && policy_too_large {
            if self.archive.mark_media_job_policy_too_large(&job).is_err() {
                log::warn!("could not preserve background media size exclusion");
            }
            return;
        }
        if owned
            && result
                .as_ref()
                .err()
                .is_some_and(|error| error == ATTACHMENT_PHONE_REQUIRED)
        {
            if self.archive.defer_media_job_to_phone(&job).is_err() {
                log::warn!("could not preserve attachment waiting for phone recovery");
            }
            return;
        }
        if self
            .archive
            .finish_media_job(&job, crate::util::now(), result.is_ok(), unavailable)
            .is_err()
        {
            // Lease remains retriable; never pretend a failed archive write completed.
            log::warn!("could not persist background media completion");
        }
    }
}

/// Inspect metadata only, without following links or traversing outside this account.
fn managed_file_missing(root: &std::path::Path, path: &std::path::Path) -> std::io::Result<bool> {
    use std::{io, path::Component};
    let relative = path.strip_prefix(root).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "File is outside managed storage",
        )
    })?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid managed file path",
        ));
    }
    let mut current = root.to_path_buf();
    for part in std::iter::once(None).chain(relative.components().map(Some)) {
        if let Some(part) = part {
            current.push(part.as_os_str());
        }
        let metadata = match std::fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
            Err(error) => return Err(error),
        };
        #[cfg(windows)]
        let is_link = {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & 0x400 != 0
        };
        #[cfg(not(windows))]
        let is_link = metadata.file_type().is_symlink();
        if is_link {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Managed file path contains a link",
            ));
        }
        if current == path && !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Managed attachment is not a file",
            ));
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_diagnostic_contains_only_fixed_policy_and_aggregate_fields() {
        let diagnostic = MediaDiagnostic {
            enabled: true,
            user_paused: false,
            privacy_paused: false,
            connected: true,
            runnable: true,
            phone_runnable: false,
            phone_recovery: false,
            large_files: false,
            target_since: 1_698_105_600,
            cdn_pending: 25,
            needs_phone: 3,
            downloaded: 40,
            unavailable: 2,
            policy_too_large: 1,
            cancelled: 0,
            schedule_known: true,
            cdn_due: 20,
            phone_due: 3,
            active: 2,
            retry_at: 1_792_443_660,
            phone_quota_retry_at: 0,
            bytes: Some(1_048_576),
            budget_mib: 20_480,
            budget_reached: false,
        };
        assert_eq!(
            diagnostic.line(4, 1_792_443_602),
            "[MEDIA_JOB] version=1 account=4 snapshot_at=1792443602 enabled=1 user_paused=0 privacy_paused=0 connected=1 runnable=1 phone_runnable=0 phone_recovery=0 large_files=0 target_since=1698105600 cdn_pending=25 needs_phone=3 downloaded=40 unavailable=2 policy_too_large=1 cancelled=0 schedule_known=1 cdn_due=20 phone_due=3 active=2 retry_at=1792443660 phone_quota_retry_at=0 bytes=1048576 budget_mib=20480 budget_reached=0"
        );
    }

    fn stage_background_job(worker: &mut Worker, chat: &str, id: &str) -> MediaJob {
        stage_background_job_with_size(worker, chat, id, 5)
    }

    fn stage_background_job_with_size(
        worker: &mut Worker,
        chat: &str,
        id: &str,
        size: u64,
    ) -> MediaJob {
        worker.archive.ensure_chat(chat, "Synthetic").unwrap();
        let mut image = wa::message::ImageMessage::default();
        image.mimetype = Some("image/jpeg".into());
        image.file_length = Some(size);
        let mut raw = wa::Message::default();
        raw.image_message = MessageField::some(image);
        let mut row = crate::archive::tests::message(chat, id, 100, false);
        row.content = classify(&raw).unwrap();
        worker
            .archive
            .insert_message(&row, Some(&raw.encode_to_vec()))
            .unwrap();
        let job = MediaJob {
            chat: chat.into(),
            id: id.into(),
            card: -1,
        };
        worker.archive.seed_media_jobs().unwrap();
        job
    }

    #[test]
    fn declared_size_admission_reserves_unknown_and_honours_large_file_opt_in() {
        let (mut worker, _, _, _) = super::super::receipt_tests::worker();
        let chat = "fixture@s.whatsapp.net";
        let automatic = ATTACHMENT_DOWNLOAD_LIMIT;
        let manual = crate::model::MANUAL_DOWNLOAD_LIMIT;
        let unknown = stage_background_job_with_size(&mut worker, chat, "unknown", 0);
        let exact = stage_background_job_with_size(&mut worker, chat, "exact", automatic);
        let over = stage_background_job_with_size(&mut worker, chat, "over", automatic + 1);
        let maximum = stage_background_job_with_size(&mut worker, chat, "maximum", manual);

        assert_eq!(worker.background_media_size(&unknown, automatic), automatic);
        assert_eq!(worker.background_media_size(&exact, automatic), automatic);
        assert!(worker.background_media_size(&over, automatic) > automatic);
        assert!(worker.background_media_size(&over, manual) <= manual);
        assert_eq!(worker.background_media_size(&maximum, manual), manual);
    }

    #[test]
    fn completions_keep_one_scan_in_flight_and_only_count_managed_files() {
        let (mut worker, _, _, _) = super::super::receipt_tests::worker();
        let chat = "fixture@s.whatsapp.net";
        let job = stage_background_job(&mut worker, chat, "managed");
        let root = worker.dirs.media_cache_dir();
        std::fs::create_dir_all(&root).unwrap();
        let managed = root.join("managed.bin");
        std::fs::write(&managed, [0_u8; 5]).unwrap();
        worker.background_media_usage = Some((Instant::now(), Some(10)));
        worker.media_scan_in_flight = Some(7);
        worker.media_scan_started_bytes = 0;
        worker
            .background_media
            .insert((job.chat.clone(), job.id.clone(), job.card));
        worker.background_media_finished(chat, "managed", -1, &Ok(managed));
        assert_eq!(worker.media_scan_in_flight, Some(7));
        assert_eq!(worker.background_media_usage.unwrap().1, Some(15));

        let external = tempfile::NamedTempFile::new().unwrap();
        let external_job = stage_background_job(&mut worker, chat, "external");
        worker.background_media.insert((
            external_job.chat.clone(),
            external_job.id.clone(),
            external_job.card,
        ));
        worker.background_media_finished(chat, "external", -1, &Ok(external.path().to_path_buf()));
        assert_eq!(worker.background_media_usage.unwrap().1, Some(15));

        worker.media_scan_completed_bytes = 8;
        worker.apply_media_scan_finished(7, Ok(crate::archive::MediaScanResult { bytes: 20 }));
        assert_eq!(worker.media_scan_in_flight, None);
        assert_eq!(worker.background_media_usage.unwrap().1, Some(28));
    }

    #[test]
    fn cancellation_wins_for_phone_recovery() {
        let (mut worker, _, _, _) = super::super::receipt_tests::worker();
        let chat = "fixture@s.whatsapp.net";
        let job = stage_background_job(&mut worker, chat, "cancelled");
        worker.archive.defer_media_job_to_phone(&job).unwrap();
        let key = (job.chat.clone(), job.id.clone(), job.card);
        worker.background_media.insert(key.clone());
        worker.background_media_phone_permits.insert(key.clone());
        worker.background_media_cancelled.insert(key);
        worker.background_media_finished(chat, "cancelled", -1, &Err("cancelled".into()));
        let counts = worker.archive.media_import_counts(i64::MIN).unwrap();
        assert_eq!(counts.cancelled, 1);
        assert_eq!(counts.needs_phone, 0);
        assert!(worker.background_media_phone_permits.is_empty());
    }

    #[test]
    fn unavailable_phone_media_does_not_trip_the_shared_account_failure_gate() {
        let (mut worker, _, _, _) = super::super::receipt_tests::worker();
        let chat = "fixture@s.whatsapp.net";
        let job = stage_background_job(&mut worker, chat, "unavailable");
        worker.archive.defer_media_job_to_phone(&job).unwrap();
        worker
            .background_media
            .insert((job.chat.clone(), job.id.clone(), job.card));

        worker.background_media_finished(
            chat,
            "unavailable",
            -1,
            &Err("No longer available on WhatsApp's servers".into()),
        );

        assert_eq!(worker.automatic_history.retry_at(), 0);
        assert!(!worker.automatic_history.needs_attention());
        let counts = worker.archive.media_import_counts(i64::MIN).unwrap();
        assert_eq!(counts.unavailable, 1);
        assert_eq!(counts.needs_phone, 0);
    }

    #[test]
    fn archive_preservation_failure_cannot_mark_a_download_complete() {
        let (mut worker, _, _, _) = super::super::receipt_tests::worker();
        let chat = "fixture@s.whatsapp.net";
        let job = stage_background_job(&mut worker, chat, "changed");
        worker
            .archive
            .set_content(
                chat,
                "changed",
                &Content::Text {
                    text: "Synthetic".into(),
                    preview: None,
                },
                false,
            )
            .unwrap();
        worker
            .background_media
            .insert((job.chat.clone(), job.id.clone(), job.card));
        let path = worker.dirs.media_cache_dir().join("changed.jpg");

        worker.downloaded(job.chat, job.id, None, Ok(path));

        let counts = worker.archive.media_import_counts(i64::MIN).unwrap();
        assert_eq!(counts.downloaded, 0);
        assert_eq!(counts.cdn_pending, 1);
    }

    #[test]
    fn reconnect_revoke_clears_exact_phone_permits() {
        let (mut worker, _, _, _) = super::super::receipt_tests::worker();
        let revocations = worker.background_media_phone_revocations.subscribe();
        worker.background_media_phone_permits.insert((
            "chat@s.whatsapp.net".into(),
            "media".into(),
            -1,
        ));
        worker.background_media_phone_permits.insert((
            "chat@s.whatsapp.net".into(),
            "motion".into(),
            -2,
        ));
        worker.revoke_background_media_phone_permits();
        assert!(worker.background_media_phone_permits.is_empty());
        assert!(
            revocations.has_changed().unwrap(),
            "a running transfer cannot use authorization captured before reconnect"
        );
    }

    #[test]
    fn motion_transfers_are_visible_to_background_deduplication() {
        let (mut worker, _, _, _) = super::super::receipt_tests::worker();
        let job = MediaJob {
            chat: "chat@s.whatsapp.net".into(),
            id: "motion".into(),
            card: -2,
        };
        assert!(!worker.background_media_transfer_active(&job));
        worker
            .motion_downloads
            .insert((job.chat.clone(), job.id.clone()));
        assert!(worker.background_media_transfer_active(&job));
    }

    #[test]
    fn missing_managed_file_probe_preserves_external_and_unexpected_paths() {
        let root = tempfile::tempdir().unwrap();
        let existing = root.path().join("attachment");
        std::fs::write(&existing, b"synthetic").unwrap();
        assert!(!managed_file_missing(root.path(), &existing).unwrap());
        std::fs::remove_file(&existing).unwrap();
        assert!(managed_file_missing(root.path(), &existing).unwrap());
        let external = tempfile::tempdir().unwrap();
        assert!(managed_file_missing(root.path(), &external.path().join("missing")).is_err());
        assert!(managed_file_missing(root.path(), &root.path().join("../missing")).is_err());
        let directory = root.path().join("directory");
        std::fs::create_dir(&directory).unwrap();
        assert!(managed_file_missing(root.path(), &directory).is_err());
    }
}
