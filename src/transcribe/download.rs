//! Direct HTTPS downloads from the catalog's pinned Hugging Face revisions.
//! Completed files survive an interrupted download; incomplete files never
//! become ONNX inputs. Audio is not part of any request.

use std::fs::{self, File, OpenOptions};
use std::future::Future;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use super::catalog::Asset;
use super::{Error, Model, State};

/// Verifies the selected model's cache and installs missing assets over the
/// proxy-aware HTTPS client. All filesystem and network work stays off the UI.
pub(super) fn ensure(
    model: Model,
    root: &Path,
    cancel: &Arc<AtomicBool>,
    progress: &dyn Fn(State),
) -> Result<PathBuf, Error> {
    let directory = model.directory(root);
    fs::create_dir_all(&directory)
        .map_err(|_| failed("Could not create the speech model folder."))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .map_err(|_| failed("Could not protect the speech model folder."))?;
    }
    progress(State::Checking);
    let runtime = Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| failed("Could not start the speech model downloader."))?,
    );
    // Bound connection stalls and idle reads, not the whole multi-gigabyte
    // transfer. A fixed total timeout makes healthy slow links restart from
    // zero forever. wait() still handles explicit cancellation promptly.
    let mut builder = reqwest::Client::builder()
        .user_agent("ZapFast speech models")
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 || attempt.url().scheme() != "https" {
                attempt.error("Speech model redirects must remain HTTPS")
            } else {
                attempt.follow()
            }
        }));
    if let Some(proxy) = crate::proxy::reqwest_proxy() {
        builder = builder.proxy(proxy);
    }
    let client = builder
        .build()
        .map_err(|_| failed("Could not prepare the speech model download."))?;
    install_files(model.assets(), &directory, cancel, progress, |asset| {
        let response = wait(&runtime, cancel, || client.get(model.url(asset)).send())?
            .error_for_status()
            .map_err(|_| {
                failed("Could not download the speech model from Hugging Face. Try again.")
            })?;
        if response
            .content_length()
            .is_some_and(|length| length != asset.bytes)
        {
            return Err(failed("The speech model download has an unexpected size."));
        }
        Ok(Box::new(Body {
            runtime: Arc::clone(&runtime),
            response,
            cancel: Arc::clone(cancel),
            pending: Vec::new(),
            at: 0,
        }) as Box<dyn Read>)
    })?;
    Ok(directory)
}

/// Builds a model error from a fixed diagnostic, never a URL or private path.
fn failed(message: &str) -> Error {
    Error::Model(message.to_owned())
}

/// Cancellation can interrupt both connection setup and a stalled body read,
/// not only the interval between chunks of an otherwise healthy download.
fn wait<T, F: Future<Output = Result<T, reqwest::Error>>>(
    runtime: &tokio::runtime::Runtime,
    cancel: &AtomicBool,
    create: impl FnOnce() -> F,
) -> Result<T, Error> {
    runtime.block_on(async {
        // reqwest creates its timeout timer in send(), not only when the
        // returned future is polled, so construction needs the reactor too.
        let future = create();
        tokio::pin!(future);
        loop {
            cancelled(cancel)?;
            tokio::select! {
                result = &mut future => return result.map_err(|_| failed("The speech model download was interrupted. Try again.")),
                _ = tokio::time::sleep(Duration::from_millis(100)) => {},
            }
        }
    })
}

struct Body {
    runtime: Arc<tokio::runtime::Runtime>,
    response: reqwest::Response,
    cancel: Arc<AtomicBool>,
    pending: Vec<u8>,
    at: usize,
}

impl Read for Body {
    /// Adapts asynchronous response chunks to the installer's bounded reads,
    /// retaining cancellation during stalled network operations.
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        while self.at == self.pending.len() {
            let chunk = wait(&self.runtime, &self.cancel, || self.response.chunk())
                .map_err(|_| std::io::Error::other("speech model download interrupted"))?;
            let Some(chunk) = chunk else {
                return Ok(0);
            };
            self.pending = chunk.to_vec();
            self.at = 0;
        }
        let count = output.len().min(self.pending.len() - self.at);
        output[..count].copy_from_slice(&self.pending[self.at..self.at + count]);
        self.at += count;
        Ok(count)
    }
}

/// Stops the installer between file/hash operations when the reader pauses.
fn cancelled(cancel: &AtomicBool) -> Result<(), Error> {
    if cancel.load(Ordering::Relaxed) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

/// Checks a cached file off the interface thread. A matching length alone is
/// not sufficient: a truncated or tampered cache is downloaded again.
fn verified(path: &Path, asset: &Asset, cancel: &AtomicBool) -> Result<bool, Error> {
    let Ok(mut file) = File::open(path) else {
        return Ok(false);
    };
    if file.metadata().map(|info| info.len()).ok() != Some(asset.bytes) {
        return Ok(false);
    }
    let mut sha = Sha256::new();
    let mut block = [0; 128 * 1024];
    loop {
        cancelled(cancel)?;
        let count = file
            .read(&mut block)
            .map_err(|_| failed("Could not verify a cached speech model."))?;
        if count == 0 {
            break;
        }
        sha.update(&block[..count]);
    }
    Ok(crate::settings::hex(&sha.finalize()) == asset.sha256)
}

/// Keeps verified files, streams each missing asset to a private partial file,
/// and publishes it only after size/hash validation and a durable write.
fn install_files(
    assets: &[Asset],
    directory: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(State),
    mut fetch: impl FnMut(&Asset) -> Result<Box<dyn Read>, Error>,
) -> Result<(), Error> {
    let total: u64 = assets.iter().map(|asset| asset.bytes).sum();
    let mut received = 0;
    for asset in assets {
        cancelled(cancel)?;
        let path = directory.join(asset.local);
        if verified(&path, asset, cancel)? {
            received += asset.bytes;
            continue;
        }
        progress(State::Downloading { received, total });
        let pending = path.with_extension(format!(
            "{}.part",
            path.extension()
                .and_then(|value| value.to_str())
                .unwrap_or("model")
        ));
        // A guard owns cleanup on every error, cancellation and unwind. It is
        // created before opening the file, so a failed write leaves nothing.
        let _pending = Pending(pending.clone());
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut output = options
            .open(&pending)
            .map_err(|_| failed("Could not save the speech model. Check free disk space."))?;
        let mut input = fetch(asset)?;
        let mut sha = Sha256::new();
        let mut count = 0u64;
        let mut block = [0; 128 * 1024];
        let mut last_report = Instant::now();
        loop {
            cancelled(cancel)?;
            let size = input.read(&mut block).map_err(|_| {
                if cancel.load(Ordering::Relaxed) {
                    Error::Cancelled
                } else {
                    failed("The speech model download was interrupted. Try again.")
                }
            })?;
            if size == 0 {
                break;
            }
            count += size as u64;
            if count > asset.bytes {
                return Err(failed("The speech model download is larger than expected."));
            }
            output
                .write_all(&block[..size])
                .map_err(|_| failed("Could not save the speech model. Check free disk space."))?;
            sha.update(&block[..size]);
            if last_report.elapsed() >= Duration::from_millis(150) {
                progress(State::Downloading {
                    received: received + count,
                    total,
                });
                last_report = Instant::now();
            }
        }
        cancelled(cancel)?;
        if count != asset.bytes || crate::settings::hex(&sha.finalize()) != asset.sha256 {
            return Err(failed(
                "The speech model failed its integrity check. Try again.",
            ));
        }
        output
            .sync_all()
            .map_err(|_| failed("Could not finish saving the speech model."))?;
        drop(output);
        // Only a corrupt copy can be here: a verified file was kept above.
        // Windows rename cannot replace it, so remove that one file first.
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|_| failed("Could not replace a damaged speech model."))?;
        }
        fs::rename(&pending, &path).map_err(|_| failed("Could not install the speech model."))?;
        received += count;
        progress(State::Downloading { received, total });
    }
    Ok(())
}

struct Pending(PathBuf);

impl Drop for Pending {
    /// Removes only this install attempt's partial file on every exit path.
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    const FIXTURE: Asset = Asset {
        remote: "fixture.onnx",
        local: "fixture.onnx",
        bytes: 5,
        sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
    };

    #[test]
    /// A pending connection must stop without waiting for the server timeout.
    fn cancellation_interrupts_a_pending_network_operation() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        runtime.spawn(async move {
            flag.store(true, Ordering::Relaxed);
        });
        let result = wait(&runtime, &cancel, || {
            std::future::pending::<Result<(), reqwest::Error>>()
        });
        assert!(matches!(result, Err(Error::Cancelled)));
    }

    #[test]
    /// Constructs timeout-bearing futures only after entering Tokio's reactor.
    fn network_futures_are_created_inside_the_reactor() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = wait(&runtime, &AtomicBool::new(false), || {
            let timer = tokio::time::sleep(Duration::from_millis(1));
            async move {
                timer.await;
                Ok::<_, reqwest::Error>(())
            }
        });
        assert!(result.is_ok());
    }

    #[test]
    /// A valid cached asset must cause no network request.
    fn a_verified_file_is_reused_without_a_request() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(FIXTURE.local), b"hello").unwrap();
        install_files(
            &[FIXTURE],
            root.path(),
            &AtomicBool::new(false),
            &|_| {},
            |_| panic!("a verified model must not download again"),
        )
        .unwrap();
    }

    #[test]
    /// Rejects wrong hashes and both size mismatches, cleaning every partial file.
    fn corrupt_and_oversized_files_never_become_model_inputs() {
        for bytes in [
            b"other".as_slice(),
            b"hello!".as_slice(),
            b"hell".as_slice(),
        ] {
            let root = tempfile::tempdir().unwrap();
            let result = install_files(
                &[FIXTURE],
                root.path(),
                &AtomicBool::new(false),
                &|_| {},
                |_| Ok(Box::new(Cursor::new(bytes.to_vec()))),
            );
            assert!(result.is_err());
            assert!(!root.path().join(FIXTURE.local).exists());
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
        }
    }

    #[test]
    /// Cancelling after fetch starts leaves no partially installed model input.
    fn a_cancelled_download_keeps_no_partial_file() {
        let root = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        let result = install_files(&[FIXTURE], root.path(), &cancel, &|_| {}, |_| {
            cancel.store(true, Ordering::Relaxed);
            Ok(Box::new(Cursor::new(b"hello")))
        });
        assert!(matches!(result, Err(Error::Cancelled)));
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    /// Matching length alone cannot make a corrupted cache entry reusable.
    fn a_same_size_corrupt_cache_is_repaired() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(FIXTURE.local);
        fs::write(&path, b"other").unwrap();
        install_files(
            &[FIXTURE],
            root.path(),
            &AtomicBool::new(false),
            &|_| {},
            |_| Ok(Box::new(Cursor::new(b"hello"))),
        )
        .unwrap();
        assert_eq!(fs::read(path).unwrap(), b"hello");
    }
}
