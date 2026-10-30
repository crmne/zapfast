//! Compiles translations, embeds the icon and version information in Windows
//! executables, and carries the speech runtime when a release asks for it.

use sha2::{Digest, Sha256};

fn main() {
    fastframe_i18n::build::compile_catalogs("assets/i18n");
    carry_speech_runtime();

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

/// With ZAPFAST_ONNXRUNTIME_EMBED naming the folder packaging/onnxruntime.py
/// filled for this target, a Linux or Windows build carries that runtime, so
/// a portable copy whose updater replaced only the executable can still
/// transcribe (src/transcribe/embedded.rs). The library must be the one
/// packaging/onnxruntime.sha256 pins. macOS carries it in the bundle instead.
fn carry_speech_runtime() {
    println!("cargo::rustc-check-cfg=cfg(zapfast_embedded_runtime)");
    println!("cargo:rerun-if-env-changed=ZAPFAST_ONNXRUNTIME_EMBED");
    let Some(folder) = std::env::var_os("ZAPFAST_ONNXRUNTIME_EMBED") else {
        return;
    };
    let target = |key: &str| std::env::var(key).unwrap_or_default();
    let (rid, library) = match (
        target("CARGO_CFG_TARGET_OS").as_str(),
        target("CARGO_CFG_TARGET_ARCH").as_str(),
    ) {
        ("linux", "x86_64") => ("linux-x64", "libonnxruntime.so"),
        ("linux", "aarch64") => ("linux-arm64", "libonnxruntime.so"),
        ("windows", "x86_64") => ("win-x64", "onnxruntime.dll"),
        ("windows", "aarch64") => ("win-arm64", "onnxruntime.dll"),
        _ => return,
    };
    let manifest = std::path::PathBuf::from(target("CARGO_MANIFEST_DIR"));
    let folder = manifest.join(folder);
    let pins = std::fs::read_to_string(manifest.join("packaging/onnxruntime.sha256"))
        .expect("packaging/onnxruntime.sha256");
    println!("cargo:rerun-if-changed=packaging/onnxruntime.sha256");
    let member = format!("runtimes/{rid}/native/{library}");
    let expected = pins
        .lines()
        .filter_map(|line| line.split_once("  "))
        .find(|(_, name)| *name == member)
        .map(|(digest, _)| digest)
        .expect("onnxruntime.sha256 pins this target's library");
    let version =
        std::fs::read_to_string(manifest.join("packaging/flatpak/onnxruntime-sources.json"))
            .ok()
            .and_then(|pin| {
                let after = pin.split("/microsoft.ml.onnxruntime/").nth(1)?;
                Some(after.split('/').next()?.to_owned())
            })
            .expect("the runtime's version in onnxruntime-sources.json");
    let out = std::path::PathBuf::from(target("OUT_DIR")).join("onnxruntime");
    std::fs::create_dir_all(&out).expect("runtime folder in OUT_DIR");
    for (name, copy) in [
        (library, "library"),
        ("onnxruntime-LICENSE.txt", "LICENSE.txt"),
        ("onnxruntime-ThirdPartyNotices.txt", "ThirdPartyNotices.txt"),
    ] {
        let source = folder.join(name);
        println!("cargo:rerun-if-changed={}", source.display());
        let bytes =
            std::fs::read(&source).unwrap_or_else(|error| panic!("{}: {error}", source.display()));
        if copy == "library" {
            let digest = Sha256::digest(&bytes);
            let digest: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
            assert_eq!(
                digest,
                expected,
                "{} is not the library onnxruntime.sha256 pins",
                source.display()
            );
        }
        std::fs::write(out.join(copy), bytes).expect("runtime copy in OUT_DIR");
    }
    println!("cargo:rustc-cfg=zapfast_embedded_runtime");
    println!("cargo:rustc-env=ZAPFAST_EMBEDDED_RUNTIME_SHA256={expected}");
    println!(
        "cargo:rustc-env=ZAPFAST_EMBEDDED_RUNTIME_DIRECTORY=onnxruntime-{version}-{rid}-{}",
        &expected[..12]
    );
}
