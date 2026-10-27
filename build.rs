//! Compiles translations and embeds Windows icon and version information.

fn main() {
    fastframe_i18n::build::compile_catalogs("assets/i18n");
    link_macos_availability_runtime();

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

/// The prebuilt static ONNX Runtime contains Objective-C availability checks.
/// Rust links with -nodefaultlibs, so Clang's availability shim is not added
/// automatically for the macOS 11 release target. Link that compiler runtime
/// explicitly instead of changing deployment targets or per-build flags.
fn link_macos_availability_runtime() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    println!("cargo:rerun-if-env-changed=DEVELOPER_DIR");
    let output = std::process::Command::new("xcrun")
        .args(["clang", "-print-file-name=libclang_rt.osx.a"])
        .output()
        .expect("Clang is required to link the native speech runtime");
    let path = std::path::PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    assert!(
        output.status.success() && path.is_file(),
        "Clang's macOS availability runtime was not found"
    );
    println!(
        "cargo:rustc-link-search=native={}",
        path.parent().expect("compiler runtime directory").display()
    );
    println!("cargo:rustc-link-lib=static=clang_rt.osx");
}
