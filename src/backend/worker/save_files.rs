//! Saves the files of messages selected in a chat together, in the download
//! folder's `<account>/<chat>` layout (`download_folders`), downloading the
//! ones not here yet first. Nothing is copied until every file has settled,
//! so the folder fills at once; a file that cannot be downloaded is named in
//! the report and the others are saved.

use super::*;

/// Downloads one run keeps going at once: enough to keep the link busy,
/// few enough that a long selection does not start dozens of transfers.
const PARALLEL: usize = 3;

#[derive(Default)]
pub(super) struct State {
    runs: Vec<Run>,
    /// Set while downloads are being started, so one that settles at once
    /// (with no connection) does not start the next from inside it.
    starting: bool,
}

/// The files of one selection, saved together once all have settled.
struct Run {
    chat: ChatId,
    files: Vec<SavedFile>,
}

struct SavedFile {
    id: String,
    mime: String,
    /// A document's own name. Other files keep their downloaded file's name.
    name: Option<String>,
    /// What a report calls a file without a name of its own.
    kind: &'static str,
    step: Step,
}

enum Step {
    Queued,
    Downloading,
    Ready(PathBuf),
    Failed(String),
}

impl Run {
    /// Marks the next queued file as downloading and returns its message,
    /// unless [`PARALLEL`] downloads are already going.
    fn start_next(&mut self) -> Option<String> {
        let busy = self
            .files
            .iter()
            .filter(|file| matches!(file.step, Step::Downloading))
            .count();
        if busy >= PARALLEL {
            return None;
        }
        let file = self
            .files
            .iter_mut()
            .find(|file| matches!(file.step, Step::Queued))?;
        file.step = Step::Downloading;
        Some(file.id.clone())
    }

    /// Files a download's result for message `id`. A file not started yet
    /// takes only a success (its bubble's own download beat the run to it):
    /// a failed automatic download says nothing about the one the run asks
    /// for. Whether the result belonged to this run.
    fn settle(&mut self, id: &str, result: &Result<PathBuf, String>) -> bool {
        let Some(file) = self.files.iter_mut().find(|file| {
            file.id == id
                && match file.step {
                    Step::Downloading => true,
                    Step::Queued => result.is_ok(),
                    Step::Ready(_) | Step::Failed(_) => false,
                }
        }) else {
            return false;
        };
        file.step = match result {
            Ok(path) => Step::Ready(path.clone()),
            Err(error) => Step::Failed(error.clone()),
        };
        true
    }

    /// Whether every file is downloaded or has failed.
    fn settled(&self) -> bool {
        self.files
            .iter()
            .all(|file| matches!(file.step, Step::Ready(_) | Step::Failed(_)))
    }
}

impl Worker {
    /// Starts saving the files of `messages`, which belong to `chat`. Files
    /// already downloaded are ready at once; the rest download, a few at a
    /// time, exempt from the automatic download limit since the reader asked.
    pub(super) fn save_files(&mut self, chat: ChatId, messages: Vec<String>) {
        let files: Vec<SavedFile> = messages
            .into_iter()
            .filter_map(|id| {
                let message = self.archive.message(&chat, &id).ok().flatten()?;
                let media = message.content.file()?;
                let step = match &media.path {
                    Some(path) if path.is_file() => Step::Ready(path.clone()),
                    _ => Step::Queued,
                };
                Some(SavedFile {
                    mime: media.mime.clone(),
                    name: match &message.content {
                        Content::Document { file_name, .. } if !file_name.trim().is_empty() => {
                            Some(file_name.clone())
                        }
                        _ => None,
                    },
                    kind: kind_of(&message.content),
                    step,
                    id,
                })
            })
            .collect();
        if files.is_empty() {
            self.emit(Event::Error(
                "The selected messages have no files to download".into(),
            ));
            return;
        }
        self.file_saves.runs.push(Run { chat, files });
        self.advance_file_saves();
    }

    /// A download finished: a run waiting for it moves on.
    pub(super) fn file_save_settled(
        &mut self,
        chat: &str,
        id: &str,
        card: Option<usize>,
        result: &Result<PathBuf, String>,
    ) {
        if card.is_some() {
            return;
        }
        let mut ours = false;
        for run in self
            .file_saves
            .runs
            .iter_mut()
            .filter(|run| run.chat == chat)
        {
            ours |= run.settle(id, result);
        }
        if ours {
            self.advance_file_saves();
        }
    }

    /// Starts what the runs may download now, then saves every run whose
    /// files have all settled.
    fn advance_file_saves(&mut self) {
        if self.file_saves.starting {
            return;
        }
        self.file_saves.starting = true;
        while let Some((chat, id)) = self
            .file_saves
            .runs
            .iter_mut()
            .find_map(|run| Some((run.chat.clone(), run.start_next()?)))
        {
            // A download that cannot start settles at once. While `starting`
            // holds, that only records its result; the run finishes below.
            self.download_media(chat, id, None, true);
        }
        self.file_saves.starting = false;
        let (done, waiting): (Vec<Run>, Vec<Run>) = std::mem::take(&mut self.file_saves.runs)
            .into_iter()
            .partition(Run::settled);
        self.file_saves.runs = waiting;
        for run in done {
            self.finish_file_save(run);
        }
    }

    /// Copies a settled run's files into the chat's folder under the
    /// account's number, in the chosen download folder or else the system's
    /// Downloads folder, and reports what was saved and what was not.
    fn finish_file_save(&mut self, run: Run) {
        let root = self.download_folder.clone().or_else(|| {
            directories::UserDirs::new().and_then(|dirs| dirs.download_dir().map(Path::to_path_buf))
        });
        let Some(root) = root else {
            self.emit(Event::Error(
                "There is no Downloads folder to save the files in. Choose a download folder in Settings > Files".into(),
            ));
            return;
        };
        let folder = self.chat_download_folder(&root, &run.chat);
        if let Err(error) = download_folders::create_folder(&root, &folder) {
            self.emit(Event::Error(format!(
                "Could not create the folder {}: {error}",
                folder.display()
            )));
            return;
        }
        let total = run.files.len();
        let mut ready = Vec::new();
        let mut failed = Vec::new();
        for file in run.files {
            match file.step {
                Step::Ready(path) => {
                    let name = file
                        .name
                        .as_deref()
                        .and_then(|name| named_media_path(Path::new(""), &file.mime, name))
                        .or_else(|| path.file_name().map(PathBuf::from))
                        .map_or_else(
                            || "attachment".to_owned(),
                            |name| name.to_string_lossy().into_owned(),
                        );
                    ready.push((path, name));
                }
                Step::Failed(error) => failed.push(format!(
                    "{} ({error})",
                    file.name.as_deref().unwrap_or(file.kind)
                )),
                Step::Queued | Step::Downloading => {}
            }
        }
        let events = self.events.clone();
        let waker = self.waker.clone();
        tokio::task::spawn_blocking(move || {
            let saved = copy_into(&folder, ready, &mut failed);
            let place = folder.display();
            let event = if failed.is_empty() {
                Event::Info(if saved == 1 {
                    format!("Saved 1 file to {place}")
                } else {
                    format!("Saved {saved} files to {place}")
                })
            } else {
                Event::Error(format!(
                    "Saved {saved} of {total} files to {place}. Not saved: {}",
                    failed.join("; ")
                ))
            };
            let _ = events.send(event);
            if saved > 0 {
                let _ = events.send(Event::FilesSaved(folder));
            }
            waker.wake();
        });
    }
}

/// Copies each `(source, name)` into `folder` and returns how many are
/// there now. A file already in the folder, or saved there before with the
/// same bytes, is not copied again; another file of the same name is never
/// replaced, and the copy takes a numbered name beside it. Failures are
/// added to `failed`.
fn copy_into(folder: &Path, files: Vec<(PathBuf, String)>, failed: &mut Vec<String>) -> usize {
    let mut saved = 0;
    for (source, name) in files {
        if source.parent() == Some(folder) || same_bytes(&source, &folder.join(&name)) {
            saved += 1;
            continue;
        }
        match save_album_copy(&source, folder, &name) {
            Ok(_) => saved += 1,
            Err(error) => failed.push(format!("{name} ({error})")),
        }
    }
    saved
}

/// Whether `copy` is a regular file holding exactly what `source` holds.
fn same_bytes(source: &Path, copy: &Path) -> bool {
    use std::io::Read;
    let regular = |path: &Path| {
        std::fs::symlink_metadata(path)
            .ok()
            .filter(|metadata| metadata.file_type().is_file())
    };
    let (Some(left), Some(right)) = (regular(source), regular(copy)) else {
        return false;
    };
    if left.len() != right.len() {
        return false;
    }
    let (Ok(mut left), Ok(mut right)) = (std::fs::File::open(source), std::fs::File::open(copy))
    else {
        return false;
    };
    let mut a = vec![0u8; 64 * 1024];
    let mut b = vec![0u8; 64 * 1024];
    loop {
        let Ok(read) = left.read(&mut a) else {
            return false;
        };
        if read == 0 {
            return true;
        }
        if right.read_exact(&mut b[..read]).is_err() || a[..read] != b[..read] {
            return false;
        }
    }
}

/// What a report calls a file without a name of its own.
fn kind_of(content: &Content) -> &'static str {
    match content {
        Content::Image { .. } => "Photo",
        Content::Video { gif: true, .. } => "GIF",
        Content::Video { .. } => "Video",
        Content::Audio {
            voice_note: true, ..
        } => "Voice message",
        Content::Audio { .. } => "Audio",
        _ => "File",
    }
}

#[cfg(test)]
mod tests {
    use super::super::receipt_tests::{self, PEER};
    use super::*;
    use crate::model::MediaState;

    const ME: &str = "15550001111@s.whatsapp.net";

    fn media(path: Option<PathBuf>, mime: &str) -> Media {
        Media {
            mime: mime.into(),
            size: 4,
            width: None,
            height: None,
            album: None,
            path,
            state: MediaState::Idle,
        }
    }

    fn message(id: &str, content: Content) -> Message {
        Message {
            content,
            ..receipt_tests::own_message(id, 100)
        }
    }

    fn document(id: &str, name: &str, path: Option<PathBuf>) -> Message {
        message(
            id,
            Content::Document {
                media: media(path, "application/pdf"),
                file_name: name.into(),
                caption: None,
                pages: None,
            },
        )
    }

    fn photo(id: &str, path: Option<PathBuf>) -> Message {
        message(
            id,
            Content::Image {
                caption: None,
                media: media(path, "image/jpeg"),
                motion: None,
            },
        )
    }

    /// A worker for chat `PEER`, called Alice, of the account +15550001111,
    /// saving into `root`.
    fn prepare(worker: &mut Worker, root: &Path) {
        worker.archive.ensure_chat(PEER, "Alice").unwrap();
        worker.me_pn = Some(ME.into());
        worker.download_folder = Some(root.to_owned());
    }

    fn file(dir: &Path, name: &str, body: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    fn insert(worker: &Worker, messages: &[Message]) {
        for message in messages {
            worker.archive.insert_message(message, None).unwrap();
        }
    }

    fn save(ids: &[&str]) -> Command {
        Command::SaveFiles {
            chat: PEER.into(),
            messages: ids.iter().map(|id| (*id).to_owned()).collect(),
        }
    }

    /// The run's report: the first info or error the worker sends.
    fn report(events: &std::sync::mpsc::Receiver<Event>) -> Event {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match events.recv_timeout(left) {
                Ok(event @ (Event::Info(_) | Event::Error(_))) => return event,
                Ok(_) => {}
                Err(error) => panic!("no report: {error}"),
            }
        }
    }

    fn reported(events: &std::sync::mpsc::Receiver<Event>) -> bool {
        events
            .try_iter()
            .any(|event| matches!(event, Event::Info(_) | Event::Error(_)))
    }

    fn names(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(folder)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// The files go together into the chat's folder under the account's
    /// number; text and stickers among the selected messages are left out.
    #[tokio::test]
    async fn selected_files_are_saved_together_under_the_account_number() {
        let (mut worker, events, _inbox, _wa) = receipt_tests::worker();
        let root = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        prepare(&mut worker, root.path());
        let sticker = message(
            "sticker",
            Content::Sticker {
                media: media(Some(file(cache.path(), "s.webp", b"sticker")), "image/webp"),
                animated: false,
            },
        );
        insert(
            &worker,
            &[
                document(
                    "doc",
                    "Report.pdf",
                    Some(file(cache.path(), "x-doc.pdf", b"report")),
                ),
                photo("pic", Some(file(cache.path(), "x-pic.jpg", b"photo"))),
                message("note", Content::text("Synthetic note")),
                sticker,
            ],
        );
        worker
            .handle_command(save(&["doc", "pic", "note", "sticker"]))
            .await;
        let folder = root.path().join("+15550001111").join("Alice");
        match report(&events) {
            Event::Info(text) => {
                assert!(text.contains("Saved 2 files"), "{text}");
                assert!(text.contains(&folder.display().to_string()), "{text}");
            }
            other => panic!("expected a success report, got {other:?}"),
        }
        assert_eq!(names(&folder), ["Report.pdf", "x-pic.jpg"]);
        assert_eq!(std::fs::read(folder.join("Report.pdf")).unwrap(), b"report");
        assert_eq!(std::fs::read(folder.join("x-pic.jpg")).unwrap(), b"photo");
        // Then the window is asked to open the folder.
        match events.recv_timeout(Duration::from_secs(10)) {
            Ok(Event::FilesSaved(opened)) => assert_eq!(opened, folder),
            other => panic!("expected the folder to open, got {other:?}"),
        }
    }

    /// A file still downloading holds the whole run back, and once it
    /// arrives every file is saved together.
    #[tokio::test]
    async fn files_still_downloading_are_saved_once_they_arrive() {
        let (mut worker, events, _inbox, _wa) = receipt_tests::worker();
        let root = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        prepare(&mut worker, root.path());
        insert(
            &worker,
            &[
                photo("ready", Some(file(cache.path(), "ready.jpg", b"ready"))),
                document("late", "Late.pdf", None),
            ],
        );
        // Its bubble's own download is already under way.
        worker
            .downloads
            .insert((PEER.to_owned(), "late".to_owned(), None));
        worker.handle_command(save(&["ready", "late"])).await;
        let folder = root.path().join("+15550001111").join("Alice");
        assert!(!reported(&events), "nothing is reported before it arrives");
        assert!(
            names(&folder).is_empty(),
            "nothing is saved before it arrives"
        );
        let arrived = file(cache.path(), "late-download.pdf", b"late");
        worker
            .handle_command(Command::Downloaded {
                card: None,
                chat: PEER.into(),
                id: "late".into(),
                result: Ok(arrived),
            })
            .await;
        match report(&events) {
            Event::Info(text) => assert!(text.contains("Saved 2 files"), "{text}"),
            other => panic!("expected a success report, got {other:?}"),
        }
        assert_eq!(names(&folder), ["Late.pdf", "ready.jpg"]);
    }

    /// A file that cannot be downloaded is named with the reason, and the
    /// others are still saved.
    #[tokio::test]
    async fn a_file_that_cannot_download_is_named_and_the_rest_are_saved() {
        let (mut worker, events, _inbox, _wa) = receipt_tests::worker();
        let root = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        prepare(&mut worker, root.path());
        insert(
            &worker,
            &[
                photo("ready", Some(file(cache.path(), "ready.jpg", b"ready"))),
                // Not downloaded, and there is no connection to fetch it.
                document("missing", "Missing.pdf", None),
            ],
        );
        worker.handle_command(save(&["ready", "missing"])).await;
        match report(&events) {
            Event::Error(text) => {
                assert!(text.contains("Saved 1 of 2 files"), "{text}");
                assert!(text.contains("Missing.pdf"), "{text}");
                assert!(text.contains("Not connected to WhatsApp"), "{text}");
            }
            other => panic!("expected a failure report, got {other:?}"),
        }
        let folder = root.path().join("+15550001111").join("Alice");
        assert_eq!(names(&folder), ["ready.jpg"]);
    }

    /// Saving the same files again keeps one copy of each, and a different
    /// file already called the same is never replaced.
    #[tokio::test]
    async fn saving_again_keeps_one_copy_and_never_replaces_another_file() {
        let (mut worker, events, _inbox, _wa) = receipt_tests::worker();
        let root = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        prepare(&mut worker, root.path());
        insert(
            &worker,
            &[document(
                "doc",
                "Report.pdf",
                Some(file(cache.path(), "x-doc.pdf", b"ours")),
            )],
        );
        worker.handle_command(save(&["doc"])).await;
        assert!(matches!(report(&events), Event::Info(_)));
        worker.handle_command(save(&["doc"])).await;
        assert!(matches!(report(&events), Event::Info(_)));
        let folder = root.path().join("+15550001111").join("Alice");
        assert_eq!(names(&folder), ["Report.pdf"]);

        std::fs::write(folder.join("Report.pdf"), b"theirs").unwrap();
        worker.handle_command(save(&["doc"])).await;
        assert!(matches!(report(&events), Event::Info(_)));
        assert_eq!(names(&folder), ["Report (2).pdf", "Report.pdf"]);
        assert_eq!(std::fs::read(folder.join("Report.pdf")).unwrap(), b"theirs");
        assert_eq!(
            std::fs::read(folder.join("Report (2).pdf")).unwrap(),
            b"ours"
        );
    }

    /// A file downloaded straight into that folder is not copied beside itself.
    #[tokio::test]
    async fn a_file_already_in_the_folder_is_not_copied_again() {
        let (mut worker, events, _inbox, _wa) = receipt_tests::worker();
        let root = tempfile::tempdir().unwrap();
        prepare(&mut worker, root.path());
        let folder = worker.download_dir(PEER);
        // Published beside an earlier file of the same name.
        let path = file(&folder, "Report (1).pdf", b"report");
        insert(&worker, &[document("doc", "Report.pdf", Some(path))]);
        worker.handle_command(save(&["doc"])).await;
        assert!(matches!(report(&events), Event::Info(_)));
        assert_eq!(names(&folder), ["Report (1).pdf"]);
    }

    /// A long selection does not start every download at once.
    #[test]
    fn a_run_downloads_a_few_files_at_a_time() {
        let mut run = Run {
            chat: PEER.into(),
            files: (0..PARALLEL + 2)
                .map(|n| SavedFile {
                    id: format!("m{n}"),
                    mime: "image/jpeg".into(),
                    name: None,
                    kind: "Photo",
                    step: Step::Queued,
                })
                .collect(),
        };
        let started: Vec<String> = std::iter::from_fn(|| run.start_next()).collect();
        assert_eq!(started.len(), PARALLEL);
        assert!(run.settle("m0", &Err("Fixture failure".into())));
        assert_eq!(run.start_next().as_deref(), Some(&*format!("m{PARALLEL}")));
        assert_eq!(run.start_next(), None);
        assert!(!run.settled());
    }
}
