//! The speech runtime a Linux or Windows release carries inside ZapFast
//! (build.rs), for a copy without one installed beside it, such as a portable
//! copy whose updater replaced only the executable. Transcription writes it
//! into the cache the first time it needs it and checks it before each use.

use std::path::{Path, PathBuf};

use super::Error;
// The writing below is only built where a runtime is carried, and for tests.
#[cfg(any(zapfast_embedded_runtime, test))]
use sha2::{Digest, Sha256};
#[cfg(any(zapfast_embedded_runtime, test))]
use std::fs::{self, File, OpenOptions};
#[cfg(any(zapfast_embedded_runtime, test))]
use std::io::{self, Read, Write};

/// Whether this build carries a runtime.
pub(super) const CARRIED: bool = cfg!(zapfast_embedded_runtime);

#[cfg(any(zapfast_embedded_runtime, test))]
#[cfg(windows)]
const LIBRARY: &str = "onnxruntime.dll";
#[cfg(any(zapfast_embedded_runtime, test))]
#[cfg(not(windows))]
const LIBRARY: &str = "libonnxruntime.so";

/// The carried runtime under `root`, the cache's runtime folder, in a folder
/// named for its version, architecture and hash, written there if needed.
#[cfg(zapfast_embedded_runtime)]
pub(super) fn library(root: &Path) -> Result<PathBuf, Error> {
    const BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/onnxruntime/library"));
    install(
        &root.join(env!("ZAPFAST_EMBEDDED_RUNTIME_DIRECTORY")),
        BYTES,
        env!("ZAPFAST_EMBEDDED_RUNTIME_SHA256"),
        &[
            (
                "onnxruntime-LICENSE.txt",
                include_bytes!(concat!(env!("OUT_DIR"), "/onnxruntime/LICENSE.txt")),
            ),
            (
                "onnxruntime-ThirdPartyNotices.txt",
                include_bytes!(concat!(
                    env!("OUT_DIR"),
                    "/onnxruntime/ThirdPartyNotices.txt"
                )),
            ),
        ],
    )
}

#[cfg(not(zapfast_embedded_runtime))]
pub(super) fn library(_root: &Path) -> Result<PathBuf, Error> {
    Err(Error::Model(
        "This copy of ZapFast does not include the speech runtime. Download ZapFast again from \
         https://zapfast.rocks/download/ to add it."
            .to_owned(),
    ))
}

#[cfg(any(zapfast_embedded_runtime, test))]
/// Puts `library`, whose SHA-256 must be `sha256`, and its notices in
/// `directory`, reusing a copy already there that still matches.
fn install(
    directory: &Path,
    library: &[u8],
    sha256: &str,
    notices: &[(&str, &[u8])],
) -> Result<PathBuf, Error> {
    if crate::settings::hex(&Sha256::digest(library)) != sha256 {
        return Err(Error::Model(
            "This copy of ZapFast carries a damaged speech runtime. Download ZapFast again from \
             https://zapfast.rocks/download/."
                .to_owned(),
        ));
    }
    let failed = |error: io::Error| {
        log::warn!("could not write the speech runtime: {error}");
        Error::Model(format!(
            "Could not write the speech runtime to {}: {error}. Check that the folder is \
             writable and the disk has room, then turn transcription off and on.",
            directory.display()
        ))
    };
    private_directory(directory).map_err(failed)?;
    let path = directory.join(LIBRARY);
    if !intact(&path, sha256).map_err(failed)? {
        publish(directory, LIBRARY, library).map_err(failed)?;
        if !intact(&path, sha256).map_err(failed)? {
            return Err(failed(io::Error::other("the written copy does not match")));
        }
    }
    for (name, bytes) in notices {
        if fs::read(directory.join(name)).ok().as_deref() != Some(*bytes) {
            publish(directory, name, bytes).map_err(failed)?;
        }
    }
    Ok(path)
}

#[cfg(any(zapfast_embedded_runtime, test))]
/// Creates the runtime's folder and its parent, refusing either as a link.
fn private_directory(directory: &Path) -> io::Result<()> {
    for folder in directory.parent().into_iter().chain([directory]) {
        match fs::symlink_metadata(folder) {
            Ok(info) if info.is_dir() => {}
            Ok(_) => {
                return Err(io::Error::other(format!(
                    "{} is not a folder",
                    folder.display()
                )));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir_all(folder)?,
            Err(error) => return Err(error),
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(any(zapfast_embedded_runtime, test))]
/// Whether `path` is a regular file with this hash. Anything else there, a
/// link included, is removed so a fresh copy can take its place.
fn intact(path: &Path, sha256: &str) -> io::Result<bool> {
    let info = match fs::symlink_metadata(path) {
        Ok(info) => info,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if info.is_file() {
        let mut file = File::open(path)?;
        let mut hash = Sha256::new();
        let mut block = vec![0; 1 << 20];
        loop {
            let count = file.read(&mut block)?;
            if count == 0 {
                break;
            }
            hash.update(&block[..count]);
        }
        if crate::settings::hex(&hash.finalize()) == sha256 {
            return Ok(true);
        }
    }
    fs::remove_file(path)?;
    Ok(false)
}

#[cfg(any(zapfast_embedded_runtime, test))]
/// Writes `bytes` beside their destination and moves them into place, so a
/// reader never sees a partial file.
fn publish(directory: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    let partial = directory.join(format!(".{name}.{}.part", std::process::id()));
    let written = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&partial, directory.join(name))
    })();
    if written.is_err() {
        let _ = fs::remove_file(&partial);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUNTIME: &[u8] = b"synthetic runtime";

    fn sha256(bytes: &[u8]) -> String {
        crate::settings::hex(&Sha256::digest(bytes))
    }

    #[test]
    fn the_carried_runtime_is_written_once_and_checked_on_each_use() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("speech-runtime").join("onnxruntime-test");
        let notices = [("onnxruntime-LICENSE.txt", b"license".as_slice())];
        let path = install(&directory, RUNTIME, &sha256(RUNTIME), &notices).unwrap();
        assert_eq!(path, directory.join(LIBRARY));
        assert_eq!(fs::read(&path).unwrap(), RUNTIME);
        assert_eq!(
            fs::read(directory.join("onnxruntime-LICENSE.txt")).unwrap(),
            b"license"
        );
        // Only the runtime's own folder is written, with no partial file left.
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 2);
        // A damaged copy is replaced.
        fs::write(&path, b"damaged").unwrap();
        install(&directory, RUNTIME, &sha256(RUNTIME), &notices).unwrap();
        assert_eq!(fs::read(&path).unwrap(), RUNTIME);
    }

    #[test]
    fn a_runtime_that_does_not_match_its_pin_is_never_written() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("onnxruntime-test");
        assert!(install(&directory, RUNTIME, &sha256(b"other"), &[]).is_err());
        assert!(!directory.exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_place_of_the_runtime_or_its_folder_is_not_followed() {
        let root = tempfile::tempdir().unwrap();
        let elsewhere = root.path().join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        let directory = root.path().join("onnxruntime-test");
        // A linked folder is refused rather than written through.
        std::os::unix::fs::symlink(&elsewhere, &directory).unwrap();
        assert!(install(&directory, RUNTIME, &sha256(RUNTIME), &[]).is_err());
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
        // A linked file is replaced by a real copy, leaving its target alone.
        fs::remove_file(&directory).unwrap();
        fs::create_dir(&directory).unwrap();
        let target = elsewhere.join("target");
        fs::write(&target, b"untouched").unwrap();
        std::os::unix::fs::symlink(&target, directory.join(LIBRARY)).unwrap();
        let path = install(&directory, RUNTIME, &sha256(RUNTIME), &[]).unwrap();
        assert!(
            !fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&path).unwrap(), RUNTIME);
        assert_eq!(fs::read(&target).unwrap(), b"untouched");
    }
}
