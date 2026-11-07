use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{LazyLock, RwLock};

use serde::{Deserialize, Serialize};

/// Custom JSON-based language pack for runtime UI translations.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LanguagePack {
    pub locale: String,
    pub name: String,
    pub strings: HashMap<String, String>,
}

impl LanguagePack {
    pub fn from_json_str(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn from_file(path: &Path) -> std::io::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

static REGISTRY: LazyLock<RwLock<HashMap<String, HashMap<String, String>>>> = LazyLock::new(|| {
    RwLock::new(HashMap::new())
});

/// JSON translation API enabling custom and dynamic language loading.
pub struct JsonI18n;

impl JsonI18n {
    pub fn register(pack: LanguagePack) {
        let mut reg = REGISTRY.write().unwrap_or_else(|p| p.into_inner());
        reg.insert(pack.locale, pack.strings);
    }

    pub fn load_file(path: impl AsRef<Path>) -> std::io::Result<()> {
        let pack = LanguagePack::from_file(path.as_ref())?;
        Self::register(pack);
        Ok(())
    }

    pub fn load_json(json: &str) -> Result<(), serde_json::Error> {
        let pack = LanguagePack::from_json_str(json)?;
        Self::register(pack);
        Ok(())
    }

    pub fn translate(locale: &str, key: &str) -> Option<String> {
        let reg = REGISTRY.read().unwrap_or_else(|p| p.into_inner());
        let map = reg.get(locale)?;
        map.get(key).cloned()
    }

    pub fn translate_or<'a>(locale: &str, key: &'a str) -> Cow<'a, str> {
        if let Some(translated) = Self::translate(locale, key) {
            return Cow::Owned(translated);
        }
        Cow::Borrowed(key)
    }

    pub fn list_locales() -> Vec<String> {
        let reg = REGISTRY.read().unwrap_or_else(|p| p.into_inner());
        reg.keys().cloned().collect()
    }
}
