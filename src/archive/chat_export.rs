//! Stream one chat to a readable file, optionally with its already-saved attachments.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, bail, ensure};
use rusqlite::params;
use zip::write::SimpleFileOptions;

use super::Archive;
use crate::model::Content;

const PAGE: i64 = 128;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub messages: u64,
    pub files: u64,
    pub missing: u64,
}

impl Archive {
    /// A separate, verified SQLCipher reader. No migrations or new archive may be created.
    pub(crate) fn export_reader(path: &Path) -> Result<Self> {
        ensure!(path.is_file(), "The archive is unavailable");
        let key = super::encryption::key_for(path)?;
        Ok(Self {
            connection: super::encryption::read_only(path, &key)?,
        })
    }

    /// Runs on a blocking task, never on the interface or the account's worker thread.
    pub(crate) fn export_chat(
        &self,
        chat: &str,
        target: &Path,
        include_media: bool,
        cancelled: &AtomicBool,
        progress: impl Fn(Counts),
    ) -> Result<Counts> {
        let _snapshot = self.connection.unchecked_transaction()?;
        let known = self
            .chat(chat)?
            .context("This chat is no longer available")?;
        ensure!(!known.locked, "Locked chats cannot be exported");
        let folder = target.parent().context("Choose an export folder")?;
        let mut transcript = tempfile::NamedTempFile::new_in(folder)?;
        writeln!(transcript, "ZapFast chat export: {}", one_line(&known.name))?;
        writeln!(
            transcript,
            "Times are UTC. Only locally saved messages are included."
        )?;
        writeln!(
            transcript,
            "No history or attachments were downloaded for this export.\n"
        )?;
        let mut bundle = include_media
            .then(|| tempfile::NamedTempFile::new_in(folder))
            .transpose()?;
        let mut zip = bundle
            .as_mut()
            .map(|file| zip::ZipWriter::new(file.as_file_mut()));
        let upper: i64 = self.connection.query_row(
            "SELECT COALESCE(MAX(rowid), 0) FROM messages WHERE chat = ?1",
            [chat],
            |row| row.get(0),
        )?;
        let mut cursor = (i64::MIN, i64::MIN, i64::MIN);
        let mut counts = Counts::default();
        loop {
            check_cancelled(cancelled)?;
            ensure!(
                self.chat(chat)?.is_some_and(|chat| !chat.locked),
                "This chat is no longer available for export"
            );
            // Match transcript ordering, including the phone's order for timestamp ties.
            // The rowid ceiling excludes messages arriving after the export starts.
            let mut query = self.connection.prepare(
                "SELECT rowid, timestamp, COALESCE(history_order, 9223372036854775807),
                        sender, sender_name, from_me, content, edited
                 FROM messages WHERE chat = ?1 AND rowid <= ?2
                   AND (timestamp, COALESCE(history_order, 9223372036854775807), rowid) > (?3, ?4, ?5)
                 ORDER BY timestamp, COALESCE(history_order, 9223372036854775807), rowid LIMIT ?6",
            )?;
            let rows = query
                .query_map(
                    params![chat, upper, cursor.0, cursor.1, cursor.2, PAGE],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, bool>(5)?,
                            row.get::<_, String>(6)?,
                            row.get::<_, bool>(7)?,
                        ))
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if rows.is_empty() {
                break;
            }
            for (rowid, at, order, sender, name, own, json, edited) in rows {
                check_cancelled(cancelled)?;
                let content: Content = serde_json::from_str(&json)
                    .map_err(|_| anyhow::anyhow!("An archived message could not be read"))?;
                let sender = if own {
                    "You".to_owned()
                } else {
                    name.filter(|name| !name.is_empty())
                        .unwrap_or_else(|| sender.split('@').next().unwrap_or(&sender).to_owned())
                };
                let stamp = jiff::Timestamp::from_second(at)?.to_zoned(jiff::tz::TimeZone::UTC);
                writeln!(
                    transcript,
                    "{} - {}: {}{}",
                    stamp.strftime("%d/%m/%Y, %H:%M:%S"),
                    one_line(&sender),
                    readable(&content),
                    if edited { " [edited]" } else { "" }
                )?;
                for (path, name) in attachments(&content) {
                    if let Some(zip) = zip.as_mut() {
                        let Some(path) = path else {
                            counts.missing += 1;
                            writeln!(transcript, "  {name} (attachment unavailable)")?;
                            continue;
                        };
                        let mut source = match std::fs::File::open(&path) {
                            Ok(file) => file,
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                                counts.missing += 1;
                                writeln!(transcript, "  {name} (attachment unavailable)")?;
                                continue;
                            }
                            Err(error) => return Err(error).context("An attachment could not be read; try exporting without attachments"),
                        };
                        ensure!(
                            source.metadata()?.is_file(),
                            "An attachment is not a regular file"
                        );
                        let entry = format!("media/{:06}-{}", counts.files + 1, safe_name(&name));
                        zip.start_file(
                            &entry,
                            SimpleFileOptions::default()
                                .large_file(true)
                                .compression_method(zip::CompressionMethod::Stored),
                        )?;
                        copy_cancelled(&mut source, zip, cancelled)?;
                        writeln!(transcript, "  {entry} (file attached)")?;
                        counts.files += 1;
                    } else {
                        writeln!(transcript, "  {name} (attachment omitted)")?;
                    }
                }
                counts.messages += 1;
                cursor = (at, order, rowid);
            }
            progress(counts);
        }
        if let Some(mut zip) = zip.take() {
            transcript.as_file_mut().seek(SeekFrom::Start(0))?;
            zip.start_file(
                "_chat.txt",
                SimpleFileOptions::default()
                    .large_file(true)
                    .compression_method(zip::CompressionMethod::Deflated),
            )?;
            copy_cancelled(transcript.as_file_mut(), &mut zip, cancelled)?;
            check_cancelled(cancelled)?;
            zip.finish()?;
        }
        drop(zip);
        check_cancelled(cancelled)?;
        let file = bundle.unwrap_or(transcript);
        file.as_file().sync_all()?;
        check_cancelled(cancelled)?;
        file.persist(target).map_err(|error| error.error)?;
        Ok(counts)
    }
}

/// Stops export work when the account-owned cancellation flag has been set.
pub(crate) fn check_cancelled(cancelled: &AtomicBool) -> Result<()> {
    if cancelled.load(Ordering::Acquire) {
        bail!("Export cancelled");
    }
    Ok(())
}

/// Copies in bounded chunks, checking cancellation before every read.
/// Read and write failures propagate so temporary output cannot be published as complete.
fn copy_cancelled(
    input: &mut impl Read,
    output: &mut impl Write,
    cancelled: &AtomicBool,
) -> Result<()> {
    let mut bytes = [0; 64 * 1024];
    loop {
        check_cancelled(cancelled)?;
        let read = input.read(&mut bytes)?;
        if read == 0 {
            return Ok(());
        }
        output.write_all(&bytes[..read])?;
    }
}

/// Keeps transcript headers on one line while leaving message bodies untouched.
fn one_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}

/// Builds a bounded filename component without separators, control characters or dot-only names.
/// The caller adds a numeric prefix to keep ZIP entries unique.
fn safe_name(name: &str) -> String {
    let name: String = name
        .chars()
        .take(100)
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let name = name.trim_matches(['.', ' ']);
    if name.is_empty() {
        "attachment".into()
    } else {
        name.to_owned()
    }
}

/// Formats saved message content as readable text without exposing raw protocol fields.
fn readable(content: &Content) -> String {
    match content {
        Content::Document {
            file_name, caption, ..
        } => format!(
            "Document: {file_name}{}",
            caption
                .as_ref()
                .map(|caption| format!("\n{caption}"))
                .unwrap_or_default()
        ),
        Content::StickerPack { name, caption, .. } => format!(
            "Sticker pack: {name}{}",
            caption
                .as_ref()
                .map(|caption| format!("\n{caption}"))
                .unwrap_or_default()
        ),
        Content::Contact {
            display_name,
            vcard,
        } => format!("Contact: {display_name}\n{vcard}"),
        Content::Poll {
            question, options, ..
        } => format!(
            "Poll: {question}\n{}",
            options
                .iter()
                .map(|option| format!("  - {option}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        Content::Location {
            latitude,
            longitude,
            name,
            address,
        } => format!(
            "Location: {}\n{}\nhttps://maps.google.com/?q={latitude},{longitude}",
            name.as_deref().unwrap_or(""),
            address.as_deref().unwrap_or("")
        ),
        Content::LiveLocation {
            latitude,
            longitude,
            ..
        } => format!(
            "Live location (last saved position): https://maps.google.com/?q={latitude},{longitude}"
        ),
        _ => content.full_summary(),
    }
}

/// Lists saved attachment slots and display names, including motion clips and carousel images.
/// Missing paths remain in the list so the transcript can report unavailable files.
fn attachments(content: &Content) -> Vec<(Option<PathBuf>, String)> {
    let mut files = Vec::new();
    if let Some(media) = content.media() {
        let name = match content {
            Content::Document { file_name, .. } => file_name.clone(),
            _ => media
                .path
                .as_ref()
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "attachment".into()),
        };
        files.push((media.path.clone(), name));
    }
    match content {
        Content::Image {
            motion: Some(motion),
            ..
        } => files.push((motion.path.clone(), "motion.mp4".into())),
        Content::Interactive {
            card: Some(card), ..
        } => {
            for (index, card) in card.carousel.iter().enumerate() {
                if let Some(media) = &card.image {
                    files.push((media.path.clone(), format!("card-{}.jpg", index + 1)));
                }
            }
        }
        _ => {}
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::tests::message;
    use crate::model::{Media, MediaState};

    const CHAT: &str = "fixture@s.whatsapp.net";

    /// Creates an isolated synthetic archive with one named chat for export tests.
    fn fixture() -> Archive {
        let archive = Archive::in_memory().unwrap();
        archive.ensure_chat(CHAT, "Synthetic chat").unwrap();
        archive
    }

    /// Builds a synthetic document row with an optional downloaded attachment.
    fn document(id: &str, path: Option<PathBuf>, name: &str) -> crate::model::Message {
        let mut row = message(CHAT, id, 100, false);
        row.content = Content::Document {
            media: Media {
                mime: "application/pdf".into(),
                size: 4,
                width: None,
                height: None,
                album: None,
                path,
                state: MediaState::Idle,
            },
            file_name: name.into(),
            caption: Some("Full document\ncaption".into()),
            pages: None,
        };
        row
    }

    /// Text export retains Unicode, multiline bodies and stable ordering across archive pages.
    #[test]
    fn text_export_crosses_page_boundaries_and_keeps_unicode_multiline_order() {
        let archive = fixture();
        for index in (0..270).rev() {
            let mut row = message(CHAT, &format!("row-{index}"), 100, false);
            row.sender_name = Some("Áda\nFixture".into());
            row.history_order = Some(index);
            row.content = Content::text(format!("line {index}\n第二行"));
            archive
                .insert_message(&row, Some(b"synthetic secret keys"))
                .unwrap();
        }
        archive.ensure_chat("other", "Other").unwrap();
        let mut other = message("other", "other", 100, true);
        other.content = Content::text("other account-shaped chat must stay out");
        archive.insert_message(&other, None).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("chat.txt");
        let counts = archive
            .export_chat(CHAT, &target, false, &AtomicBool::new(false), |_| {})
            .unwrap();
        assert_eq!(counts.messages, 270);
        let text = std::fs::read_to_string(target).unwrap();
        assert_eq!(text.matches("第二行").count(), 270);
        assert!(text.find("line 0\n").unwrap() < text.find("line 269\n").unwrap());
        assert!(text.contains("Áda Fixture"));
        assert!(!text.contains("synthetic secret keys"));
        assert!(!text.contains("must stay out"));
    }

    /// ZIP export assigns distinct safe names and reports unavailable attachment slots.
    #[test]
    fn zip_keeps_duplicate_files_separate_and_reports_missing_files() {
        let archive = fixture();
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.pdf");
        std::fs::write(&source, b"fixture bytes").unwrap();
        for id in ["first", "second"] {
            archive
                .insert_message(
                    &document(id, Some(source.clone()), "../../CON|fixture.pdf"),
                    None,
                )
                .unwrap();
        }
        archive
            .insert_message(&document("missing", None, "unavailable.pdf"), None)
            .unwrap();
        let mut once = message(CHAT, "once", 101, false);
        once.content = Content::PhoneOnly {
            view_once: true,
            live_location: false,
            once: None,
        };
        archive
            .insert_message(&once, Some(b"never export raw view-once keys"))
            .unwrap();
        let target = directory.path().join("chat.zip");
        let counts = archive
            .export_chat(CHAT, &target, true, &AtomicBool::new(false), |_| {})
            .unwrap();
        assert_eq!(
            counts,
            Counts {
                messages: 4,
                files: 2,
                missing: 1
            }
        );
        let mut zip = zip::ZipArchive::new(std::fs::File::open(target).unwrap()).unwrap();
        assert_eq!(zip.len(), 3);
        let mut names = Vec::new();
        for index in 0..zip.len() {
            let file = zip.by_index(index).unwrap();
            assert!(file.enclosed_name().is_some());
            names.push(file.name().unwrap().into_owned());
        }
        assert_ne!(names[0], names[1]);
        let mut text = String::new();
        zip.by_name("_chat.txt")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert!(text.contains("Full document\ncaption"));
        assert!(text.contains("attachment unavailable"));
        assert!(text.contains("View once"));
        assert!(!text.contains("never export raw"));
        let mut bytes = Vec::new();
        zip.by_name(&names[0])
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, b"fixture bytes");
    }

    /// Cancelled exports preserve the destination and remove their temporary files.
    #[test]
    fn cancellation_preserves_existing_output_and_removes_partial_files() {
        let archive = fixture();
        for index in 0..130 {
            archive
                .insert_message(&message(CHAT, &index.to_string(), 100 + index, true), None)
                .unwrap();
        }
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("existing.zip");
        std::fs::write(&target, b"original export").unwrap();
        let cancelled = AtomicBool::new(false);
        assert!(
            archive
                .export_chat(CHAT, &target, true, &cancelled, |_| cancelled
                    .store(true, Ordering::Release))
                .is_err()
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"original export");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    /// Privacy denial or invalid archived content prevents publishing partial output.
    #[test]
    fn locked_chats_and_unreadable_rows_cannot_publish_an_export() {
        let archive = fixture();
        archive
            .connection
            .execute("UPDATE chats SET locked = 1 WHERE id = ?1", [CHAT])
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("chat.txt");
        assert!(
            archive
                .export_chat(CHAT, &target, false, &AtomicBool::new(false), |_| {})
                .is_err()
        );
        assert!(!target.exists());
        archive
            .connection
            .execute("UPDATE chats SET locked = 0 WHERE id = ?1", [CHAT])
            .unwrap();
        archive
            .insert_message(&message(CHAT, "broken", 100, false), None)
            .unwrap();
        archive
            .connection
            .execute(
                r#"UPDATE messages SET content = '{"kind":"unrecognized"}' WHERE chat = ?1"#,
                [CHAT],
            )
            .unwrap();
        assert!(
            archive
                .export_chat(CHAT, &target, false, &AtomicBool::new(false), |_| {})
                .is_err()
        );
        assert!(!target.exists());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    /// The export reader fails closed without a valid key and cannot create or modify an archive.
    #[test]
    fn export_reader_requires_the_key_and_never_creates_or_writes_an_archive() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("archive.db");
        let key = [42; 32];
        let archive = Archive::open_with_key(&path, &key).unwrap();
        archive.ensure_chat(CHAT, "Synthetic").unwrap();
        let reader = super::super::encryption::read_only(&path, &key).unwrap();
        assert!(reader.execute("DELETE FROM chats", []).is_err());
        assert!(super::super::encryption::read_only(&path, &[1; 32]).is_err());
        let absent = directory.path().join("absent.db");
        assert!(super::super::encryption::read_only(&absent, &key).is_err());
        assert!(!absent.exists());
        assert!(archive.chat(CHAT).unwrap().is_some());
    }
}
