pub fn search_key(text: &str) -> String {
    use icu_normalizer::DecomposingNormalizerBorrowed;
    use icu_properties::{CodePointMapData, CodePointMapDataBorrowed, props::GeneralCategory};

    if text.is_ascii() {
        return text.to_ascii_lowercase();
    }
    static MARKS: std::sync::OnceLock<CodePointMapDataBorrowed<'static, GeneralCategory>> =
        std::sync::OnceLock::new();
    let marks = MARKS.get_or_init(CodePointMapData::<GeneralCategory>::new);
    let decomposed = DecomposingNormalizerBorrowed::new_nfd().normalize(text);
    let mut stripped = String::with_capacity(decomposed.len());
    stripped.extend(
        decomposed
            .chars()
            .filter(|c| marks.get(*c) != GeneralCategory::NonspacingMark),
    );
    stripped
        .to_lowercase()
        .chars()
        .map(|c| if c == 'ς' { 'σ' } else { c })
        .collect()
}

pub fn search_matches(name: &str, needle: &str) -> bool {
    if name.contains(needle) {
        return true;
    }
    let mut parts = search_words(needle).peekable();
    if parts.peek().is_none() {
        return false;
    }
    let mut words = search_words(name);
    parts.all(|part| words.any(|word| word.starts_with(part)))
}

fn search_words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
        .filter(|word| !word.is_empty())
}

pub fn duration(seconds: u32) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

pub fn bytes(size: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = size as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{size} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn initials(name: &str) -> String {
    let mut words = name
        .split(|character: char| character.is_whitespace() || character == '-')
        .filter(|word| word.chars().any(char::is_alphanumeric));
    let first = words.next();
    let last = words.next_back();
    let mut initials = String::new();
    for word in [first, last].into_iter().flatten() {
        if let Some(character) = word.chars().find(|character| character.is_alphanumeric()) {
            initials.extend(character.to_uppercase());
        }
    }
    if initials.is_empty() {
        initials.push('#');
    }
    initials
}

/// The machine's hostname, or an empty string if it cannot be read.
pub fn hostname() -> String {
    gethostname::gethostname().to_string_lossy().into_owned()
}

pub fn hue(seed: &str) -> f32 {
    let mut hash: u32 = 2_166_136_261;
    for byte in seed.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    (hash % 360) as f32
}

pub fn editor_names(name: &str, first: Option<&str>) -> (String, String) {
    let name = name.trim();
    if let Some(first) = first.map(str::trim).filter(|first| !first.is_empty())
        && let Some(rest) = name.strip_prefix(first)
        && (rest.is_empty() || rest.starts_with(char::is_whitespace))
    {
        return (first.to_owned(), rest.trim().to_owned());
    }
    (name.to_owned(), String::new())
}
