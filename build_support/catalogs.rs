//! Build-time gettext catalog preparation, also exercised by integration tests.

use std::error::Error;
use std::path::Path;

use polib::message::{CatalogMessageMutView, MessageView};

pub fn compile(source: &Path, destination: &Path) -> Result<(), Box<dyn Error>> {
    let mut catalog = polib::po_file::parse(source)?;
    let forms = catalog.metadata.plural_rules.nplurals;
    // Like msgfmt, do not ship fuzzy or unfinished translations. Checking every
    // plural form matters: an empty translation otherwise hides the whole label.
    for mut message in catalog.messages_mut() {
        let complete = match message.msgstr_plural() {
            Ok(translations) => {
                translations.len() == forms && translations.iter().all(|text| !text.is_empty())
            }
            Err(_) => message.msgstr().is_ok_and(|text| !text.is_empty()),
        };
        if message.is_fuzzy() || !complete {
            message.delete();
        }
    }
    let prepared = destination.with_extension("po");
    let mut writer = std::io::BufWriter::new(std::fs::File::create(&prepared)?);
    polib::po_file::write(&catalog, &mut writer)?;
    std::io::Write::flush(&mut writer)?;
    include_po::generate_rs_from_po(&prepared, destination)?;
    if forms == 1 {
        // include-po 0.2 leaves `n` unused for constant plural expressions,
        // such as Japanese's `0`. Keep its API and explicitly ignore the
        // argument instead of suppressing unused-variable diagnostics.
        let generated = std::fs::read_to_string(destination)?;
        let signature = "pub fn number_index(n: u64) -> u32 {";
        assert!(
            generated.contains(signature),
            "catalog generator API changed"
        );
        std::fs::write(
            destination,
            generated.replace(signature, &format!("{signature}\n    let _ = n;")),
        )?;
    }
    Ok(())
}
