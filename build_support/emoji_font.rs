//! Stages an optional, user-supplied emoji font for compile-time embedding.

use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// Writes the renderer's font choice and returns the input to watch for changes.
pub fn prepare(out_dir: &Path, input: Option<&Path>) -> io::Result<Option<PathBuf>> {
    let staged = out_dir.join("custom-emoji-font.ttf");
    let source = out_dir.join("emoji-font.rs");
    if let Some(input) = input {
        let input = input.canonicalize()?;
        let mut file = std::fs::File::open(&input)?;
        let mut signature = [0; 4];
        file.read_exact(&mut signature)?;
        if !matches!(&signature, b"\0\x01\0\0" | b"OTTO" | b"ttcf") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ZAPFAST_EMOJI_FONT must name a TrueType/OpenType font or font collection",
            ));
        }
        std::fs::copy(&input, staged)?;
        std::fs::write(
            source,
            "const CUSTOM_EMOJI_FONT: Option<&[u8]> = Some(include_bytes!(concat!(env!(\"OUT_DIR\"), \"/custom-emoji-font.ttf\")));\n",
        )?;
        Ok(Some(input))
    } else {
        match std::fs::remove_file(staged) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        std::fs::write(source, "const CUSTOM_EMOJI_FONT: Option<&[u8]> = None;\n")?;
        Ok(None)
    }
}
