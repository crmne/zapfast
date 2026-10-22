//! Compiles translations, the macOS speech helper, and embeds the icon and
//! version information in Windows executables.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    fastframe_i18n::build::compile_catalogs("assets/i18n");
    build_speech_helper();

    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=packaging/windows/zapfast.ico");
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("packaging/windows/zapfast.ico")
            .set("ProductName", "ZapFast")
            .set("FileDescription", "ZapFast");
        if let Err(error) = resource.compile() {
            println!("cargo:warning=Windows resources not embedded: {error}");
        }
    }
}

/// Builds `zapfast-asr`, the Swift helper that reaches the system's speech
/// model, beside the app binary. Nothing to do on other platforms.
fn build_speech_helper() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    const SOURCE: &str = "packaging/macos/asr/zapfast-asr.swift";
    println!("cargo:rerun-if-changed={SOURCE}");

    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    // `target/<triple>/<profile>/build/<package>/out`: the binary's own folder
    // is three levels up.
    let Some(profile) = out.ancestors().nth(3) else {
        return;
    };
    let helper = profile.join("zapfast-asr");
    if is_fresh(&helper, Path::new(SOURCE)) {
        return;
    }
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        _ => return,
    };
    // The speech API the helper uses arrived in macOS 26. An older Mac simply
    // does not offer transcription (`crate::transcribe::available`).
    let target = format!("{arch}-apple-macos26.0");
    let _ = std::fs::remove_file(&helper);
    let built = Command::new("xcrun")
        .args([
            "swiftc",
            "-O",
            "-parse-as-library",
            "-target",
            &target,
            "-o",
        ])
        .arg(&helper)
        .arg(SOURCE)
        .status();
    match built {
        Ok(status) if status.success() && helper.is_file() => {}
        Ok(status) => {
            let message =
                format!("the speech helper did not compile ({status}); transcription is off");
            if std::env::var("PROFILE").as_deref() == Ok("release") {
                panic!("{message}");
            }
            println!("cargo:warning={message}");
        }
        Err(error) => {
            let message = format!("could not run swiftc ({error}); transcription is off");
            if std::env::var("PROFILE").as_deref() == Ok("release") {
                panic!("{message}");
            }
            println!("cargo:warning={message}");
        }
    }
}

fn is_fresh(output: &Path, source: &Path) -> bool {
    let (Ok(output), Ok(source)) = (output.metadata(), source.metadata()) else {
        return false;
    };
    match (output.modified(), source.modified()) {
        (Ok(output), Ok(source)) => output >= source,
        _ => false,
    }
}
