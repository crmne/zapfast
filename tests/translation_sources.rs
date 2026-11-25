//! The translation template is extracted from the files `POTFILES` names. A
//! source that asks for a translation without being listed there keeps its
//! messages out of the template, and so out of every catalog.
use std::fs;
use std::path::{Path, PathBuf};

/// The sources the template is extracted from, as committed.
fn listed() -> Vec<String> {
    include_str!("../assets/i18n/POTFILES")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Every Rust source under `dir`, in a stable order.
fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
        .map(|entry| entry.expect("an entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
}

/// Whether `name` is called in `code`: the name on its own, then `(`.
fn calls(code: &str, name: &str) -> bool {
    code.match_indices(name).any(|(at, _)| {
        let own = code[..at]
            .chars()
            .next_back()
            .is_none_or(|last| !(last.is_alphanumeric() || last == '_'));
        own && code[at + name.len()..].starts_with('(')
    })
}

/// Whether a source's shipped code asks for a translation. Its test module
/// is left out: tests may translate without a catalog ever seeing it.
fn translates(source: &str) -> bool {
    // The shipped code ends at the first test module: `#[cfg(test)]` on a
    // `mod` of its own.
    let shipped = source
        .match_indices("#[cfg(test)]")
        .find_map(|(at, _)| {
            let after = &source[at + "#[cfg(test)]".len()..];
            after
                .trim_start()
                .starts_with("mod ")
                .then_some(&source[..at])
        })
        .unwrap_or(source);
    ["gettext", "pgettext", "ngettext", "translated"]
        .iter()
        .any(|name| calls(shipped, name))
}

#[test]
fn every_source_that_translates_is_listed() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut paths = Vec::new();
    sources(&root.join("src"), &mut paths);
    let listed = listed();
    let missing: Vec<String> = paths
        .iter()
        .filter_map(|path| {
            let relative = path
                .strip_prefix(&root)
                .expect("under the package")
                .to_string_lossy()
                .replace('\\', "/");
            // The demo ships in no catalog.
            if relative.starts_with("src/demo") || listed.contains(&relative) {
                return None;
            }
            let source = fs::read_to_string(path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            translates(&source).then_some(relative)
        })
        .collect();
    assert!(
        missing.is_empty(),
        "not in assets/i18n/POTFILES: {missing:#?}"
    );
}

#[test]
fn every_listed_source_is_there() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let listed = listed();
    let missing: Vec<&String> = listed
        .iter()
        .filter(|source| !root.join(source).is_file())
        .collect();
    assert!(
        missing.is_empty(),
        "in assets/i18n/POTFILES but absent: {missing:#?}"
    );
}
