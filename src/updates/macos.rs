use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, ensure};

use super::install::{self, Installation, Prepared};

const IDENTIFIER: &str = "me.paolino.fastsapp";

pub(super) fn bundle_root(executable: &Path) -> Result<&Path> {
    let root = executable
        .ancestors()
        .nth(3)
        .context("Missing app bundle")?;
    ensure!(
        root.extension().is_some_and(|extension| extension == "app")
            && root.join("Contents/MacOS/zapfast") == executable,
        "Move the app to Applications, then open it to update."
    );
    Ok(root)
}

fn plist(bundle: &Path, key: &str) -> Result<String> {
    let output = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", &format!("Print :{key}")])
        .arg(bundle.join("Contents/Info.plist"))
        .output()?;
    ensure!(output.status.success(), "The app bundle is missing {key}");
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn identity(bundle: &Path) -> Result<()> {
    ensure!(
        plist(bundle, "CFBundleIdentifier")? == IDENTIFIER
            && plist(bundle, "CFBundleExecutable")? == "zapfast"
            && plist(bundle, "CFBundlePackageType")? == "APPL",
        "The download is not a ZapFast app bundle"
    );
    Ok(())
}

pub(super) fn detect(executable: &Path) -> Result<()> {
    ensure!(
        !executable.starts_with("/Volumes")
            && !executable
                .components()
                .any(|part| part.as_os_str() == "AppTranslocation"),
        "Move the app to Applications, then open it to update."
    );
    let bundle = bundle_root(executable)?;
    let prefixes = [
        Some(PathBuf::from("/opt/homebrew")),
        Some(PathBuf::from("/usr/local")),
        std::env::var_os("HOMEBREW_PREFIX").map(PathBuf::from),
    ];
    for prefix in prefixes.into_iter().flatten() {
        ensure!(
            !["zapfast", "fastsapp"]
                .iter()
                .any(|name| cask_owns(&prefix.join("Caskroom").join(name), bundle)),
            "Update this installation with Homebrew."
        );
    }
    identity(bundle)
}

fn cask_owns(cask: &Path, bundle: &Path) -> bool {
    let Ok(bundle) = bundle.canonicalize() else {
        return false;
    };
    fs::read_dir(cask).is_ok_and(|versions| {
        versions.flatten().any(|version| {
            ["ZapFast.app", "FastsApp.app"].iter().any(|name| {
                version
                    .path()
                    .join(name)
                    .canonicalize()
                    .is_ok_and(|installed| installed == bundle)
            })
        })
    })
}

fn team(bundle: &Path) -> Result<Option<String>> {
    let verify = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(bundle)
        .output()?;
    ensure!(
        verify.status.success(),
        "The app signature could not be verified"
    );
    let output = Command::new("/usr/bin/codesign")
        .args(["--display", "--verbose=4"])
        .arg(bundle)
        .output()?;
    ensure!(
        output.status.success(),
        "Cannot read the app signing identity"
    );
    Ok(String::from_utf8(output.stderr)?
        .lines()
        .find_map(|line| line.strip_prefix("TeamIdentifier="))
        .filter(|value| *value != "not set")
        .map(str::to_owned))
}

fn validate(bundle: &Path, installation: &Installation, version: &str) -> Result<()> {
    identity(bundle)?;
    ensure!(
        plist(bundle, "CFBundleShortVersionString")? == version,
        "The app bundle has the wrong version"
    );
    let incoming = team(bundle)?;
    if let Some(current) = team(bundle_root(&installation.executable)?)? {
        ensure!(
            incoming.as_deref() == Some(current.as_str()),
            "The update was signed by a different publisher"
        );
        let assessment = Command::new("/usr/sbin/spctl")
            .args(["--assess", "--type", "execute"])
            .arg(bundle)
            .output()?;
        ensure!(
            assessment.status.success(),
            "macOS could not approve this update for launch"
        );
    }
    install::verify_version(&bundle.join("Contents/MacOS/zapfast"), version)
}

struct Mounted(PathBuf);

fn mountpoint(archive: &Path) -> Result<PathBuf> {
    let mount = archive
        .parent()
        .context("Missing update directory")?
        .join(format!("mounted-{:016x}", rand::random::<u64>()));
    fs::create_dir(&mount)?;
    Ok(mount)
}

impl Mounted {
    fn open(archive: &Path) -> Result<Self> {
        // A failed detach can leave the previous attempt mounted. Never
        // traverse that volume or reuse its directory on a later attempt.
        let mount = mountpoint(archive)?;
        let output = Command::new("/usr/bin/hdiutil")
            .args(["attach", "-readonly", "-nobrowse", "-mountpoint"])
            .arg(&mount)
            .arg(archive)
            .output()?;
        if !output.status.success() {
            let _ = fs::remove_dir(&mount);
            anyhow::bail!("macOS could not open the downloaded disk image");
        }
        Ok(Self(mount))
    }

    fn bundle(&self) -> Result<PathBuf> {
        image_bundle(&self.0)
    }
}

fn image_bundle(root: &Path) -> Result<PathBuf> {
    for name in ["ZapFast.app", "FastsApp.app"] {
        let bundle = root.join(name);
        match fs::symlink_metadata(&bundle) {
            Ok(metadata) => {
                ensure!(
                    metadata.is_dir(),
                    "The disk image has an invalid app bundle"
                );
                return Ok(bundle);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    anyhow::bail!("The disk image has no ZapFast app bundle")
}

impl Drop for Mounted {
    fn drop(&mut self) {
        let detached = Command::new("/usr/bin/hdiutil")
            .arg("detach")
            .arg(&self.0)
            .output()
            .is_ok_and(|output| output.status.success());
        if detached {
            let _ = fs::remove_dir(&self.0);
        }
    }
}

pub(super) fn validate_download(
    archive: &Path,
    installation: &Installation,
    version: &str,
) -> Result<()> {
    let mounted = Mounted::open(archive)?;
    validate(&mounted.bundle()?, installation, version)
}

pub(super) fn replace(prepared: &Prepared) -> Result<()> {
    let target = bundle_root(&prepared.installation.executable)?;
    let backup = prepared.directory.join("previous");
    let candidate = prepared.directory.join("ZapFast.app");
    ensure!(
        !backup.exists() && !candidate.exists(),
        "This update was already applied"
    );
    let mounted = Mounted::open(&prepared.payload)?;
    let source = mounted.bundle()?;
    validate(&source, &prepared.installation, &prepared.version)?;
    let copy = Command::new("/usr/bin/ditto")
        .arg(&source)
        .arg(&candidate)
        .output()?;
    ensure!(
        copy.status.success(),
        "Could not copy the downloaded app bundle"
    );
    validate(&candidate, &prepared.installation, &prepared.version)?;
    drop(mounted);
    fs::rename(target, &backup).context("Cannot back up the current app bundle")?;
    if let Err(error) = fs::rename(&candidate, target) {
        fs::rename(&backup, target).context("Could not restore the previous app bundle")?;
        return Err(error).context("Could not replace the app bundle");
    }
    Ok(())
}

/// A signed executable is sealed to its bundle's Info.plist and resources,
/// and macOS kills a copy taken out of the bundle. The helper therefore runs
/// from a copy of the whole running bundle.
pub(super) fn helper(prepared: &Prepared) -> Result<PathBuf> {
    let source = bundle_root(&prepared.installation.executable)?;
    let bundle = prepared.directory.join("helper.app");
    ensure!(!bundle.exists(), "The update helper was already prepared");
    let copy = Command::new("/usr/bin/ditto")
        .arg(source)
        .arg(&bundle)
        .output()?;
    ensure!(copy.status.success(), "Could not prepare the update helper");
    Ok(bundle.join("Contents/MacOS/zapfast"))
}

pub(super) fn restore(prepared: &Prepared) -> Result<()> {
    let backup = prepared.directory.join("previous");
    if backup.is_dir() {
        let target = bundle_root(&prepared.installation.executable)?;
        if target.exists() {
            fs::rename(target, prepared.directory.join("failed.app"))
                .context("Could not move the failed update aside")?;
        }
        fs::rename(backup, target).context("Could not restore the previous app bundle")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renamed_images_prefer_the_new_bundle_and_still_accept_old_images() {
        let root =
            std::env::temp_dir().join(format!("zapfast-image-test-{}", rand::random::<u64>()));
        fs::create_dir(&root).unwrap();
        assert!(image_bundle(&root).is_err());
        let legacy = root.join("FastsApp.app");
        fs::create_dir(&legacy).unwrap();
        assert_eq!(image_bundle(&root).unwrap(), legacy);
        let app = root.join("ZapFast.app");
        fs::create_dir(&app).unwrap();
        assert_eq!(image_bundle(&root).unwrap(), app);
        fs::remove_dir(&app).unwrap();
        std::os::unix::fs::symlink(&legacy, &app).unwrap();
        assert!(
            image_bundle(&root).is_err(),
            "never follow an app-bundle symlink"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn homebrew_ownership_survives_the_bundle_rename() {
        let root =
            std::env::temp_dir().join(format!("zapfast-cask-test-{}", rand::random::<u64>()));
        for name in ["ZapFast.app", "FastsApp.app"] {
            let installed = root.join("Applications").join(name);
            let version = root.join("Caskroom/zapfast/0.8.0");
            fs::create_dir_all(&installed).unwrap();
            fs::create_dir_all(&version).unwrap();
            std::os::unix::fs::symlink(&installed, version.join(name)).unwrap();
            assert!(cask_owns(&root.join("Caskroom/zapfast"), &installed));
            assert!(!cask_owns(&root.join("Caskroom/unrelated"), &installed));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn another_mount_attempt_leaves_the_previous_volume_alone() {
        let directory =
            std::env::temp_dir().join(format!("zapfast-mount-test-{}", rand::random::<u64>()));
        fs::create_dir(&directory).unwrap();
        let archive = directory.join("update.dmg");
        let first = mountpoint(&archive).unwrap();
        fs::write(first.join("still-mounted"), b"existing volume").unwrap();
        let second = mountpoint(&archive).unwrap();
        assert_ne!(first, second);
        assert_eq!(
            fs::read(first.join("still-mounted")).unwrap(),
            b"existing volume"
        );
        assert!(second.is_dir());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn homebrew_app_symlink_does_not_claim_other_copies() {
        let directory =
            std::env::temp_dir().join(format!("zapfast-cask-test-{}", rand::random::<u64>()));
        let installed = directory.join("Applications/ZapFast.app");
        let cask = directory.join("Caskroom/zapfast");
        let copy = directory.join("dev/ZapFast.app");
        for path in [&installed, &copy, &cask.join("0.7.1")] {
            fs::create_dir_all(path).unwrap();
        }
        std::os::unix::fs::symlink(&installed, cask.join("0.7.1/ZapFast.app")).unwrap();
        assert!(cask_owns(&cask, &installed));
        assert!(!cask_owns(&cask, &copy));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rollback_restores_resources_and_executable_together() {
        let directory =
            std::env::temp_dir().join(format!("zapfast-mac-test-{}", rand::random::<u64>()));
        let app = directory.join("ZapFast.app");
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        fs::write(app.join("Contents/MacOS/zapfast"), b"new executable").unwrap();
        fs::write(app.join("Contents/Info.plist"), b"new metadata").unwrap();
        let installation = Installation {
            executable: app.join("Contents/MacOS/zapfast"),
            kind: install::Kind::MacBundle,
        };
        let stage = install::staging(&installation).unwrap();
        assert_eq!(stage.parent(), Some(directory.as_path()));
        let backup = stage.join("previous");
        fs::create_dir_all(backup.join("Contents/MacOS")).unwrap();
        fs::write(backup.join("Contents/MacOS/zapfast"), b"old executable").unwrap();
        fs::write(backup.join("Contents/Info.plist"), b"old metadata").unwrap();
        let prepared = Prepared {
            installation,
            directory: stage.clone(),
            payload: stage.join("update.dmg"),
            sha256: String::new(),
            version: "0.7.2".into(),
        };
        restore(&prepared).unwrap();
        assert_eq!(
            fs::read(app.join("Contents/MacOS/zapfast")).unwrap(),
            b"old executable"
        );
        assert_eq!(
            fs::read(app.join("Contents/Info.plist")).unwrap(),
            b"old metadata"
        );
        assert_eq!(
            fs::read(stage.join("failed.app/Contents/Info.plist")).unwrap(),
            b"new metadata"
        );
        assert!(!backup.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn helper_runs_from_a_whole_copy_of_the_bundle() {
        let directory =
            std::env::temp_dir().join(format!("zapfast-helper-test-{}", rand::random::<u64>()));
        let app = directory.join("ZapFast.app");
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        fs::write(app.join("Contents/MacOS/zapfast"), b"executable").unwrap();
        fs::write(app.join("Contents/Info.plist"), b"metadata").unwrap();
        let installation = Installation {
            executable: app.join("Contents/MacOS/zapfast"),
            kind: install::Kind::MacBundle,
        };
        let stage = install::staging(&installation).unwrap();
        let prepared = Prepared {
            installation,
            directory: stage.clone(),
            payload: stage.join("update.dmg"),
            sha256: String::new(),
            version: "0.7.2".into(),
        };
        let executable = helper(&prepared).unwrap();
        assert_eq!(executable, stage.join("helper.app/Contents/MacOS/zapfast"));
        assert_eq!(fs::read(&executable).unwrap(), b"executable");
        assert_eq!(
            fs::read(stage.join("helper.app/Contents/Info.plist")).unwrap(),
            b"metadata"
        );
        assert!(helper(&prepared).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn only_expected_bundle_layout_is_accepted() {
        assert_eq!(
            bundle_root(Path::new(
                "/Applications/ZapFast.app/Contents/MacOS/zapfast"
            ))
            .unwrap(),
            Path::new("/Applications/ZapFast.app")
        );
        for path in [
            "/Applications/ZapFast.app/zapfast",
            "/tmp/Contents/MacOS/zapfast",
            "/usr/local/bin/zapfast",
        ] {
            assert!(bundle_root(Path::new(path)).is_err());
        }
        assert!(
            detect(Path::new(
                "/Volumes/ZapFast/ZapFast.app/Contents/MacOS/zapfast"
            ))
            .is_err()
        );
        assert!(
            detect(Path::new(
                "/private/var/folders/test/AppTranslocation/test/ZapFast.app/Contents/MacOS/zapfast"
            ))
            .is_err()
        );
    }
}
