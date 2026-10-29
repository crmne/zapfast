//! Compiles translations.

fn main() {
    fastframe_i18n::build::compile_catalogs("../../assets/i18n");
}
