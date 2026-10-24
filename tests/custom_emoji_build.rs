//! Build-time font staging never requires a proprietary asset in the checkout.

#[path = "../build_support/emoji_font.rs"]
mod emoji_font;

#[test]
fn the_default_build_has_no_custom_asset() {
    let output = tempfile::tempdir().unwrap();
    assert_eq!(emoji_font::prepare(output.path(), None).unwrap(), None);
    let source = std::fs::read_to_string(output.path().join("emoji-font.rs")).unwrap();
    assert!(source.contains("= None;"));
    assert!(!output.path().join("custom-emoji-font.ttf").exists());
}

#[test]
fn a_font_is_copied_exactly_and_can_be_disabled_in_the_same_build_directory() {
    let output = tempfile::tempdir().unwrap();
    let input =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts/NotoColorEmoji.ttf");
    assert_eq!(
        emoji_font::prepare(output.path(), Some(&input)).unwrap(),
        Some(input.canonicalize().unwrap())
    );
    assert_eq!(
        std::fs::read(output.path().join("custom-emoji-font.ttf")).unwrap(),
        std::fs::read(input).unwrap()
    );
    let source = std::fs::read_to_string(output.path().join("emoji-font.rs")).unwrap();
    assert!(source.contains("Some(include_bytes!"));
    emoji_font::prepare(output.path(), None).unwrap();
    assert!(!output.path().join("custom-emoji-font.ttf").exists());
    let source = std::fs::read_to_string(output.path().join("emoji-font.rs")).unwrap();
    assert!(source.contains("= None;"));
}

#[test]
fn missing_and_non_font_inputs_fail_instead_of_silently_changing_the_build() {
    let output = tempfile::tempdir().unwrap();
    let missing = output.path().join("missing.ttf");
    assert!(emoji_font::prepare(output.path(), Some(&missing)).is_err());
    let invalid = output.path().join("not-a-font.ttf");
    std::fs::write(&invalid, b"This is not a font").unwrap();
    assert_eq!(
        emoji_font::prepare(output.path(), Some(&invalid))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::InvalidData
    );
    assert!(!output.path().join("emoji-font.rs").exists());
}
