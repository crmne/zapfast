//! Optional local voice transcription, with subprocesses isolated from the UI thread.

use crate::backend::Waker;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
};

#[derive(Debug, PartialEq)]
pub enum State {
    Running,
    Ready(String),
    Failed,
}

enum ResultEvent {
    Detected(bool),
    Finished {
        generation: u64,
        key: (String, String),
        result: State,
    },
}

pub struct Transcription {
    available: bool,
    generation: u64,
    revision: u64,
    revisions: HashMap<String, u64>,
    states: HashMap<(String, String), State>,
    tx: mpsc::Sender<ResultEvent>,
    rx: mpsc::Receiver<ResultEvent>,
}

impl Default for Transcription {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            available: false,
            generation: 0,
            revision: 0,
            revisions: HashMap::new(),
            states: HashMap::new(),
            tx,
            rx,
        }
    }
}

fn cli(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn transcribe(executable: &Path, audio: &Path, cache: &Path) -> State {
    let result = (|| -> std::io::Result<String> {
        std::fs::create_dir_all(cache)?;
        let scratch = tempfile::Builder::new()
            .prefix("transcription-")
            .tempdir_in(cache)?;
        let output = scratch.path().join("transcript.txt");
        let status = cli(executable).arg(audio).arg("-o").arg(&output).status()?;
        if !status.success() {
            return Err(std::io::Error::other("transcription failed"));
        }
        std::fs::read_to_string(output)
    })();
    match result {
        Ok(text) if !text.trim().is_empty() => State::Ready(text.trim().to_owned()),
        _ => State::Failed,
    }
}

impl Transcription {
    #[cfg(any(test, feature = "demo"))]
    pub fn sample(&mut self, chat: &str, message: &str, state: State) {
        self.available = true;
        self.set_state((chat.to_owned(), message.to_owned()), state);
    }

    pub fn detect(&self, waker: Waker) {
        let tx = self.tx.clone();
        std::thread::Builder::new()
            .name("transcription-detect".into())
            .spawn(move || {
                let available = cli(Path::new("yapsnap"))
                    .arg("--help")
                    .status()
                    .is_ok_and(|status| status.success());
                let _ = tx.send(ResultEvent::Detected(available));
                waker.wake();
            })
            .ok();
    }

    fn set_state(&mut self, key: (String, String), state: State) {
        self.revision = self.revision.wrapping_add(1);
        self.revisions.insert(key.0.clone(), self.revision);
        self.states.insert(key, state);
    }

    pub fn revision(&self, chat: &str) -> u64 {
        self.revisions.get(chat).copied().unwrap_or_default()
    }

    pub fn available(&self) -> bool {
        self.available
    }

    pub fn state(&self, chat: &str, message: &str) -> Option<&State> {
        self.states.get(&(chat.to_owned(), message.to_owned()))
    }

    pub fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.states.clear();
        self.revisions.clear();
    }

    pub fn restore(&mut self, chat: &str, transcripts: Vec<(String, String)>) {
        for (message, text) in transcripts {
            let key = (chat.to_owned(), message);
            if !self.states.contains_key(&key) {
                self.set_state(key, State::Ready(text));
            }
        }
    }

    pub fn poll(&mut self) -> Vec<(String, String, String)> {
        let mut completed = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            match event {
                ResultEvent::Detected(available) => self.available = available,
                ResultEvent::Finished {
                    generation,
                    key,
                    result,
                } if generation == self.generation => {
                    if let State::Ready(text) = &result {
                        completed.push((key.0.clone(), key.1.clone(), text.clone()));
                    }
                    self.set_state(key, result);
                }
                _ => {}
            }
        }
        completed
    }

    pub fn start(
        &mut self,
        chat: String,
        message: String,
        audio: PathBuf,
        cache: PathBuf,
        waker: Waker,
    ) {
        let key = (chat, message);
        if !self.available
            || matches!(
                self.states.get(&key),
                Some(State::Running | State::Ready(_))
            )
        {
            return;
        }
        self.set_state(key.clone(), State::Running);
        let tx = self.tx.clone();
        let generation = self.generation;
        let worker_key = key.clone();
        if std::thread::Builder::new()
            .name("voice-transcription".into())
            .spawn(move || {
                let result = transcribe(Path::new("yapsnap"), &audio, &cache);
                let _ = tx.send(ResultEvent::Finished {
                    generation,
                    key: worker_key,
                    result,
                });
                waker.wake();
            })
            .is_err()
        {
            self.set_state(key, State::Failed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlink_discards_late_results_and_keeps_detection() {
        let mut transcription = Transcription::default();
        transcription.tx.send(ResultEvent::Detected(true)).unwrap();
        transcription
            .tx
            .send(ResultEvent::Finished {
                generation: 0,
                key: ("chat".into(), "voice".into()),
                result: State::Ready("Synthetic speech".into()),
            })
            .unwrap();
        transcription.clear();
        assert!(transcription.poll().is_empty());
        assert!(transcription.available());
        assert_eq!(transcription.state("chat", "voice"), None);
    }

    #[test]
    fn saved_transcripts_restore_without_yapsnap_and_do_not_get_saved_again() {
        let mut transcription = Transcription::default();
        transcription.restore("chat", vec![("voice".into(), "Synthetic speech".into())]);
        assert!(!transcription.available());
        assert_eq!(
            transcription.state("chat", "voice"),
            Some(&State::Ready("Synthetic speech".into()))
        );
        let revision = transcription.revision("chat");
        transcription.restore("chat", vec![("voice".into(), "Synthetic speech".into())]);
        assert_eq!(transcription.revision("chat"), revision);
        assert!(transcription.poll().is_empty());
    }

    #[test]
    fn completed_transcripts_are_saved_once() {
        let mut transcription = Transcription::default();
        transcription
            .tx
            .send(ResultEvent::Finished {
                generation: 0,
                key: ("chat".into(), "voice".into()),
                result: State::Ready("Synthetic speech".into()),
            })
            .unwrap();
        assert_eq!(
            transcription.poll(),
            vec![("chat".into(), "voice".into(), "Synthetic speech".into())]
        );
        assert!(transcription.poll().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn cli_reads_output_handles_failures_and_removes_scratch() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = tempfile::tempdir().unwrap();
        let executable = fixture.path().join("fake-yapsnap");
        std::fs::write(
            &executable,
            "#!/bin/sh\nprintf ' Synthetic speech \\n' > \"$3\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let cache = fixture.path().join("cache");
        assert_eq!(
            transcribe(&executable, Path::new("synthetic audio.ogg"), &cache),
            State::Ready("Synthetic speech".into())
        );
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0);
        std::fs::write(&executable, "#!/bin/sh\nexit 1\n").unwrap();
        assert_eq!(
            transcribe(&executable, Path::new("synthetic.ogg"), &cache),
            State::Failed
        );
    }
}
