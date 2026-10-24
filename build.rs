//! Compiles translations and embeds the icon and version information in
//! Windows executables.

#[path = "build_support/emoji_font.rs"]
mod emoji_font;

fn main() {
    println!("cargo:rerun-if-env-changed=ZAPFAST_EMOJI_FONT");
    println!("cargo:rustc-check-cfg=cfg(custom_emoji_font)");
    let out_dir =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo output directory"));
    let custom_font = std::env::var_os("ZAPFAST_EMOJI_FONT").map(std::path::PathBuf::from);
    if let Some(path) = emoji_font::prepare(&out_dir, custom_font.as_deref())
        .expect("Could not prepare ZAPFAST_EMOJI_FONT")
    {
        println!("cargo:rerun-if-changed={}", path.display());
        println!("cargo:rustc-cfg=custom_emoji_font");
    }
    fastframe_i18n::build::compile_catalogs("assets/i18n");

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
