//! Local palette files and their asynchronous catalog.

use super::Palette;
use egui::Color32;
use std::{
    io::Read,
    path::{Component, Path, PathBuf},
    sync::mpsc,
};

/// A local JSON palette, identified by its filename in the themes directory.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CustomTheme {
    pub filename: String,
    pub palette: Palette,
}

pub fn label(filename: &str) -> &str {
    if filename == "omarchy.json" {
        "Omarchy"
    } else {
        filename
    }
}

/// A damaged optional cache must not make the rest of settings unreadable.
pub fn read_cached_theme<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<CustomTheme>, D::Error> {
    use serde::Deserialize;
    let value = serde_json::Value::deserialize(deserializer)?;
    if value.is_null() {
        return Ok(None);
    }
    match serde_json::from_value(value) {
        Ok(theme) => Ok(Some(theme)),
        Err(error) => {
            log::warn!("ignoring an unreadable cached theme: {error}");
            Ok(None)
        }
    }
}

#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum ThemeBase {
    #[default]
    Dark,
    Light,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFile {
    #[serde(default)]
    base: ThemeBase,
    #[serde(default)]
    colors: std::collections::BTreeMap<String, String>,
}

pub(super) fn parse_palette(text: &str) -> Result<Palette, String> {
    let file: ThemeFile = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let mut palette = match file.base {
        ThemeBase::Dark => Palette::dark(),
        ThemeBase::Light => Palette::light(),
    };
    for (name, value) in &file.colors {
        let hex = value
            .strip_prefix('#')
            .ok_or_else(|| format!("{name}: expected #RRGGBB or #RRGGBBAA"))?;
        if !matches!(hex.len(), 6 | 8) || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("{name}: expected #RRGGBB or #RRGGBBAA"));
        }
        let color = u32::from_str_radix(hex, 16).map_err(|error| error.to_string())?;
        let color = if hex.len() == 6 {
            Color32::from_rgb((color >> 16) as u8, (color >> 8) as u8, color as u8)
        } else {
            Color32::from_rgba_unmultiplied(
                (color >> 24) as u8,
                (color >> 16) as u8,
                (color >> 8) as u8,
                color as u8,
            )
        };
        match name.as_str() {
            "window" => palette.window = color,
            "panel" => palette.panel = color,
            "surface" => palette.surface = color,
            "surface_hover" => palette.surface_hover = color,
            "surface_active" => palette.surface_active = color,
            "outline" => palette.outline = color,
            "text" => palette.text = color,
            "secondary" => palette.secondary = color,
            "dim" => palette.dim = color,
            "accent" => palette.accent = color,
            "accent_hover" => palette.accent_hover = color,
            "on_accent" => palette.on_accent = color,
            "danger" => palette.danger = color,
            "warning" => palette.warning = color,
            "overlay" => palette.overlay = color,
            "shadow" => palette.shadow = color,
            "chat" => palette.chat = color,
            "bubble_in" => palette.bubble_in = color,
            "bubble_out" => palette.bubble_out = color,
            "link" => palette.link = color,
            "read" => palette.read = color,
            _ => return Err(format!("unknown color: {name}")),
        }
    }
    // Spotifast palettes share the sixteen interface colours. Derive chat-only
    // colours when importing one, while keeping explicit ZapFast overrides.
    if file.colors.contains_key("window") && !file.colors.contains_key("chat") {
        palette.chat = palette.window;
    }
    if file.colors.contains_key("surface") && !file.colors.contains_key("bubble_in") {
        palette.bubble_in = palette.surface;
    }
    if file.colors.contains_key("accent") {
        if !file.colors.contains_key("bubble_out") {
            palette.bubble_out = palette.surface.lerp_to_gamma(palette.accent, 0.18);
        }
        if !file.colors.contains_key("link") {
            palette.link = palette.accent;
        }
        if !file.colors.contains_key("read") {
            palette.read = palette.accent;
        }
    }
    if file.colors.contains_key("panel") && !file.colors.contains_key("overlay") {
        palette.overlay = palette.panel;
    }
    Ok(palette)
}

// A palette has twenty-one colors. These limits also bound directory work and
// diagnostics, not just the bytes read from one file. Never recurse.
const MAX_FILE_BYTES: u64 = 64 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 512;
const MAX_THEMES: usize = 128;

fn filename_is_local(filename: &str) -> bool {
    let mut parts = Path::new(filename).components();
    matches!(parts.next(), Some(Component::Normal(_)))
        && parts.next().is_none()
        && !filename.contains('\\')
        && filename.ends_with(".json")
}

fn read_theme(directory: &Path, filename: &str) -> Result<CustomTheme, String> {
    if !filename_is_local(filename) {
        return Err("expected a JSON filename in the themes folder".into());
    }
    let path = directory.join(filename);
    let metadata = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("expected a regular file, not a directory or symbolic link".into());
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err("theme exceeds the 64 KiB file limit".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .and_then(|file| file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("theme exceeds the 64 KiB file limit".into());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| "expected UTF-8 JSON".to_string())?;
    Ok(CustomTheme {
        filename: filename.into(),
        palette: parse_palette(text)?,
    })
}

#[derive(Default)]
struct Loaded {
    themes: Vec<CustomTheme>,
    problem: Option<String>,
    follows_omarchy: bool,
    system_theme: Option<CustomTheme>,
    #[cfg(target_os = "linux")]
    watch: Option<super::watch::ThemeWatch>,
}

fn discover(directory: &Path, selected: Option<&str>) -> Loaded {
    let mut loaded = Loaded::default();
    // Resolve the saved choice directly so a large catalog cannot displace it.
    // It is still a single validated filename, never an arbitrary path.
    if let Some(filename) = selected {
        match read_theme(directory, filename) {
            Ok(theme) => loaded.themes.push(theme),
            Err(error) => log::warn!("unable to load selected theme {filename:?}: {error}"),
        }
    }
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound {
                loaded.problem =
                    Some("The themes folder could not be read. See the log for details.".into());
                log::warn!("unable to read themes at {}: {error}", directory.display());
            }
            return loaded;
        }
    };
    let mut names = Vec::new();
    for (index, entry) in entries.take(MAX_DIRECTORY_ENTRIES + 1).enumerate() {
        if index == MAX_DIRECTORY_ENTRIES {
            loaded.problem = Some("The themes folder has more than 512 entries. Keep fewer files there to list the custom palettes.".into());
            // Do not offer a different arbitrary subset depending on filesystem order.
            return loaded;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                log::warn!("unable to read theme entry: {error}");
                continue;
            }
        };
        let filename = entry.file_name();
        let Some(filename) = filename.to_str() else {
            continue;
        };
        if filename_is_local(filename) && Some(filename) != selected {
            match entry.file_type() {
                Ok(kind) if kind.is_file() => names.push(filename.to_owned()),
                Ok(_) => {}
                Err(error) => log::warn!("unable to inspect theme {filename:?}: {error}"),
            }
        }
    }
    names.sort();
    if names.len() + loaded.themes.len() > MAX_THEMES {
        loaded.problem = Some("Only 128 custom palettes can be listed. Keep fewer JSON files in the themes folder to see the rest.".into());
    }
    for filename in names.into_iter().take(MAX_THEMES - loaded.themes.len()) {
        match read_theme(directory, &filename) {
            Ok(theme) => loaded.themes.push(theme),
            Err(error) => log::warn!("unable to load theme {filename:?}: {error}"),
        }
    }
    loaded.themes.sort_by(|a, b| a.filename.cmp(&b.filename));
    loaded
}

struct Scan {
    directory: PathBuf,
    selected: Option<String>,
    waker: crate::backend::Waker,
}

/// Background discovery at launch or on request. Keep at most one scan running
/// and one pending request, so rapid theme changes cannot accumulate workers.
/// Selection lives in Settings, never in the worker's result.
#[derive(Default)]
pub struct Catalog {
    themes: Vec<CustomTheme>,
    problem: Option<String>,
    receiver: Option<mpsc::Receiver<Loaded>>,
    pending: Option<Scan>,
    follows_omarchy: bool,
    system_theme: Option<CustomTheme>,
    presets: bool,
    #[cfg(target_os = "linux")]
    watch: Option<super::watch::ThemeWatch>,
    #[cfg(target_os = "linux")]
    setup: Option<super::omarchy::Setup>,
    #[cfg(target_os = "linux")]
    setup_pending: bool,
}

impl Catalog {
    /// Normal launches load the bundled palettes and the active desktop theme.
    /// Demo profiles remain isolated from the desktop and its files.
    pub fn enable_desktop_themes(&mut self) {
        self.presets = true;
        #[cfg(target_os = "linux")]
        {
            self.setup = super::omarchy::Setup::discover();
            self.setup_pending = true;
        }
    }

    pub fn needs_reload(&self) -> bool {
        #[cfg(target_os = "linux")]
        if let Some(watch) = &self.watch {
            return watch.take_changed();
        }
        false
    }

    pub fn start(
        &mut self,
        directory: PathBuf,
        selected: Option<String>,
        waker: &crate::backend::Waker,
    ) {
        let scan = Scan {
            directory,
            selected,
            waker: waker.clone(),
        };
        if self.loading() {
            self.pending = Some(scan);
        } else {
            self.scan(scan);
        }
    }

    fn scan(&mut self, scan: Scan) {
        let presets = self.presets;
        #[cfg(target_os = "linux")]
        let needs_watch = presets && self.watch.is_none();
        #[cfg(target_os = "linux")]
        let setup = self.setup.clone();
        #[cfg(target_os = "linux")]
        let install = std::mem::take(&mut self.setup_pending);
        let waker = scan.waker.clone();
        self.spawn(&waker, move || {
            #[cfg(target_os = "linux")]
            if install
                && let Some(setup) = &setup
                && let Err(error) = setup.install(&scan.directory)
            {
                log::warn!("unable to prepare the optional Omarchy theme: {error}");
            }
            let selected_file = scan.selected.as_deref().filter(|filename| {
                !presets || !super::presets::contains(filename) || scan.directory.join(filename).exists()
            });
            let mut loaded = discover(&scan.directory, selected_file);
            if presets {
                for theme in super::presets::themes() {
                    if selected_file == Some(theme.filename.as_str()) && scan.directory.join(&theme.filename).exists()
                        && !loaded.themes.iter().any(|local| local.filename == theme.filename) {
                        // A broken user override keeps the cached selection; it
                        // must not silently turn back into the bundled default.
                        continue;
                    }
                    if !loaded
                        .themes
                        .iter()
                        .any(|local| local.filename == theme.filename)
                    {
                        loaded.themes.push(theme);
                    }
                }
                loaded.themes.sort_by(|a, b| a.filename.cmp(&b.filename));
            }
            #[cfg(target_os = "linux")]
            let loaded = {
                let mut loaded = loaded;
                if needs_watch {
                    let system = setup
                        .as_ref()
                        .filter(|setup| setup.active())
                        .map(|setup| setup.watch_directory());
                    let watch = std::fs::create_dir_all(&scan.directory)
                        .map_err(notify::Error::io)
                        .and_then(|()| {
                            super::watch::ThemeWatch::new(
                                &scan.directory,
                                system.as_deref(),
                                &scan.waker,
                            )
                        });
                    match watch {
                        Ok(watch) => loaded.watch = Some(watch),
                        Err(error) => log::warn!("unable to watch theme changes: {error}"),
                    }
                }
                if let Some(setup) = &setup
                    && setup.active()
                {
                    loaded.follows_omarchy = true;
                    match setup.current_theme() {
                        Ok(theme) => {
                            loaded.themes.retain(|old| old.filename != "omarchy.json");
                            loaded.themes.push(theme.clone());
                            loaded.system_theme = Some(theme);
                        }
                        Err(error) => {
                            log::warn!("unable to read the current Omarchy palette: {error}");
                            loaded.problem.get_or_insert_with(|| "The Omarchy palette could not be loaded. Keeping the last usable appearance. See the log for details.".into());
                        }
                    }
                }

                loaded
            };
            loaded
        });
    }

    fn spawn(
        &mut self,
        waker: &crate::backend::Waker,
        load: impl FnOnce() -> Loaded + Send + 'static,
    ) {
        let (sender, receiver) = mpsc::channel();
        let wake = waker.clone();
        match std::thread::Builder::new()
            .name("zapfast-themes".into())
            .spawn(move || {
                let result = load();
                if sender.send(result).is_ok() {
                    wake.wake();
                }
            }) {
            Ok(_) => self.receiver = Some(receiver),
            Err(error) => {
                log::warn!("unable to start the theme loader: {error}");
                self.problem = Some(
                    "Custom themes could not be loaded. Run zapfast reload-themes to try again."
                        .into(),
                );
            }
        }
    }

    pub fn themes(&self) -> &[CustomTheme] {
        &self.themes
    }

    /// Live Omarchy comes first on its desktop; other local palettes retain
    /// their catalogue order. A leftover generated file is not a live option
    /// when the integration is unavailable.
    pub fn picker_themes(&self) -> impl Iterator<Item = &CustomTheme> {
        self.themes
            .iter()
            .filter(|theme| self.follows_omarchy && theme.filename == "omarchy.json")
            .chain(
                self.themes
                    .iter()
                    .filter(|theme| theme.filename != "omarchy.json"),
            )
    }

    pub fn find(&self, filename: &str) -> Option<&CustomTheme> {
        self.themes.iter().find(|theme| theme.filename == filename)
    }

    pub fn follows_omarchy(&self) -> bool {
        self.follows_omarchy
    }

    pub fn system_theme(&self) -> Option<&CustomTheme> {
        self.system_theme.as_ref()
    }

    pub fn loading(&self) -> bool {
        self.receiver.is_some()
    }

    pub fn detail(&self, selected: Option<&str>) -> &str {
        if self.loading() {
            return "Loading local themes…";
        }
        if selected.is_some_and(|filename| self.find(filename).is_none()) {
            return "The selected theme is unavailable. Keeping the last usable appearance. See the log for details.";
        }
        self.problem.as_deref().unwrap_or("")
    }

    pub fn poll(&mut self) -> bool {
        let Some(receiver) = &self.receiver else {
            return false;
        };
        let result = receiver.try_recv();
        if matches!(result, Err(mpsc::TryRecvError::Empty)) {
            return false;
        }
        self.receiver = None;
        if let Some(scan) = self.pending.take() {
            // Keep the last accepted palette until the latest request finishes.
            // Publishing this superseded result could briefly restore old colors.
            self.scan(scan);
            return false;
        }
        match result {
            Ok(loaded) => {
                self.themes = loaded.themes;
                self.problem = loaded.problem;
                self.follows_omarchy = loaded.follows_omarchy;
                self.system_theme = loaded.system_theme;
                #[cfg(target_os = "linux")]
                if loaded.watch.is_some() {
                    self.watch = loaded.watch;
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.problem = Some(
                    "Custom themes could not be loaded. Run zapfast reload-themes to try again."
                        .into(),
                );
            }
            Err(mpsc::TryRecvError::Empty) => unreachable!("handled above"),
        }
        true
    }

    /// Deterministic theme menus for native demo captures, without desktop setup.
    #[cfg(any(test, feature = "demo"))]
    pub fn preview(themes: Vec<CustomTheme>, follows_omarchy: bool) -> Self {
        let system_theme = follows_omarchy
            .then(|| {
                themes
                    .iter()
                    .find(|theme| theme.filename == "omarchy.json")
                    .cloned()
            })
            .flatten();
        Self {
            themes,
            follows_omarchy,
            system_theme,
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn from_themes(themes: Vec<CustomTheme>) -> Self {
        Self {
            themes,
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn load_test(&mut self, load: impl FnOnce() -> Vec<CustomTheme> + Send + 'static) {
        self.spawn(&crate::backend::Waker::default(), move || Loaded {
            themes: load(),
            ..Loaded::default()
        });
    }

    #[cfg(test)]
    pub(crate) fn load_system_test(&mut self, theme: Option<CustomTheme>, follows: bool) {
        self.spawn(&crate::backend::Waker::default(), move || Loaded {
            system_theme: theme,
            follows_omarchy: follows,
            ..Loaded::default()
        });
    }
}

#[cfg(test)]
mod custom_theme_tests {
    #[test]
    fn bundled_choices_are_available_without_local_files_and_valid_overrides_win() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("Nord.json"),
            r##"{"base":"light","colors":{"accent":"#102030"}}"##,
        )
        .unwrap();
        let mut catalog = super::Catalog {
            presets: true,
            ..Default::default()
        };
        catalog.start(
            directory.path().into(),
            Some("Nord.json".into()),
            &Default::default(),
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !catalog.poll() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(catalog.themes().len(), 8);
        let nord = catalog.find("Nord.json").unwrap();
        assert!(!nord.palette.dark);
        assert_eq!(
            nord.palette.accent,
            egui::Color32::from_rgb(0x10, 0x20, 0x30)
        );
        assert!(catalog.find("Tokyo Night.json").is_some());
        for filename in [
            "Rose Pine.json",
            "Rose Pine Moon.json",
            "Rose Pine Dawn.json",
        ] {
            assert!(catalog.find(filename).is_some());
        }
        assert_eq!(
            std::fs::read_dir(directory.path()).unwrap().count(),
            1,
            "bundled defaults do not replace or create user palette files"
        );
    }

    #[test]
    fn picker_places_live_omarchy_first_only_when_the_integration_is_available() {
        for available in [false, true] {
            let catalog = super::Catalog::preview(
                ["Catppuccin.json", "Tokyo Night.json", "omarchy.json"]
                    .into_iter()
                    .map(|filename| super::CustomTheme {
                        filename: filename.into(),
                        palette: super::Palette::dark(),
                    })
                    .collect(),
                available,
            );
            let names: Vec<_> = catalog
                .picker_themes()
                .map(|theme| theme.filename.as_str())
                .collect();
            assert_eq!(
                names,
                if available {
                    vec!["omarchy.json", "Catppuccin.json", "Tokyo Night.json"]
                } else {
                    vec!["Catppuccin.json", "Tokyo Night.json"]
                }
            );
            assert!(
                catalog.find("omarchy.json").is_some(),
                "menu filtering must preserve cached selections"
            );
        }
    }

    use super::*;

    #[test]
    fn reloads_coalesce_and_never_publish_a_superseded_palette() {
        let dir =
            std::env::temp_dir().join(format!("zapfast-theme-reload-{}", rand::random::<u64>()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("latest.json"), br#"{"base":"light"}"#).unwrap();
        let accepted = CustomTheme {
            filename: "accepted.json".into(),
            palette: Palette::dark(),
        };
        let mut catalog = Catalog::from_themes(vec![accepted.clone()]);
        let (finish, worker) = mpsc::channel();
        catalog.load_test(move || {
            worker.recv().unwrap();
            vec![CustomTheme {
                filename: "stale.json".into(),
                palette: Palette::light(),
            }]
        });
        for _ in 0..100 {
            catalog.start(dir.join("superseded"), None, &Default::default());
        }
        catalog.start(dir.clone(), Some("latest.json".into()), &Default::default());
        assert!(!catalog.poll(), "requests do not replace the running scan");
        assert_eq!(catalog.themes(), std::slice::from_ref(&accepted));
        finish.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !catalog.poll() {
            assert_eq!(
                catalog.themes(),
                std::slice::from_ref(&accepted),
                "hold the accepted colors"
            );
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(!catalog.loading());
        assert_eq!(catalog.themes().len(), 1);
        assert_eq!(catalog.themes()[0].filename, "latest.json");
        assert_eq!(catalog.themes()[0].palette, Palette::light());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn overrides_inherit_the_base_and_support_alpha() {
        let palette =
            parse_palette(r##"{"base":"light","colors":{"text":"#ebdbb2","shadow":"#00000080"}}"##)
                .unwrap();
        assert_eq!(palette.window, Palette::light().window);
        assert!(!palette.dark);
        assert_eq!(palette.text, Color32::from_rgb(235, 219, 178));
        assert_eq!(palette.shadow, Color32::from_black_alpha(128));
        assert_eq!(parse_palette("{}").unwrap(), Palette::dark());
    }

    #[test]
    fn invalid_themes_are_rejected() {
        for text in [
            r##"{"colors":{"text":"#fff"}}"##,
            r##"{"colors":{"text":"#zzzzzz"}}"##,
            r##"{"colors":{"typo":"#ffffff"}}"##,
            r#"{"base":"system"}"#,
            r#"{"typo":true}"#,
            "not json",
        ] {
            assert!(parse_palette(text).is_err(), "{text}");
        }
    }

    #[test]
    fn discovery_sorts_valid_files_and_skips_invalid_files() {
        let dir = std::env::temp_dir().join(format!("zapfast-themes-{}", rand::random::<u64>()));
        assert!(discover(&dir, None).themes.is_empty());
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in [
            ("z.json", "{}"),
            ("a.json", "{}"),
            ("bad.json", "invalid"),
            ("ignored.txt", "{}"),
        ] {
            std::fs::write(dir.join(name), text).unwrap();
        }
        let themes = discover(&dir, None).themes;
        assert_eq!(
            themes
                .iter()
                .map(|theme| theme.filename.as_str())
                .collect::<Vec<_>>(),
            ["a.json", "z.json"]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reads_are_bounded_and_cannot_escape_to_other_files() {
        let root =
            std::env::temp_dir().join(format!("zapfast-theme-bounds-{}", rand::random::<u64>()));
        let dir = root.join("themes");
        std::fs::create_dir_all(&dir).unwrap();
        let mut boundary = vec![b' '; MAX_FILE_BYTES as usize];
        boundary[..2].copy_from_slice(b"{}");
        std::fs::write(dir.join("boundary.json"), &boundary).unwrap();
        assert_eq!(
            read_theme(&dir, "boundary.json").unwrap().palette,
            Palette::dark()
        );
        boundary.push(b' ');
        std::fs::write(dir.join("too-large.json"), boundary).unwrap();
        std::fs::write(dir.join("invalid-utf8.json"), [0xff, 0xfe]).unwrap();
        std::fs::create_dir(dir.join("directory.json")).unwrap();
        std::fs::write(root.join("outside.json"), b"{}").unwrap();
        for filename in [
            "too-large.json",
            "invalid-utf8.json",
            "directory.json",
            "../outside.json",
            "..\\outside.json",
            "/outside.json",
        ] {
            assert!(read_theme(&dir, filename).is_err(), "{filename}");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("outside.json"), dir.join("link.json")).unwrap();
            assert!(read_theme(&dir, "link.json").is_err());
            assert!(
                discover(&dir, Some("link.json"))
                    .themes
                    .iter()
                    .all(|theme| theme.filename != "link.json")
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn catalog_limits_keep_the_saved_selection_without_an_arbitrary_partial_listing() {
        let dir =
            std::env::temp_dir().join(format!("zapfast-theme-catalog-{}", rand::random::<u64>()));
        std::fs::create_dir(&dir).unwrap();
        for index in 0..MAX_THEMES + 1 {
            std::fs::write(dir.join(format!("{index:03}.json")), b"{}").unwrap();
        }
        std::fs::write(dir.join("selected.json"), b"{}").unwrap();
        let loaded = discover(&dir, Some("selected.json"));
        assert_eq!(loaded.themes.len(), MAX_THEMES);
        assert!(
            loaded
                .themes
                .iter()
                .any(|theme| theme.filename == "selected.json")
        );
        assert!(loaded.problem.unwrap().contains("128"));
        for index in 0..MAX_DIRECTORY_ENTRIES {
            std::fs::write(dir.join(format!("ignored-{index}.txt")), b"ignored").unwrap();
        }
        let loaded = discover(&dir, Some("selected.json"));
        assert_eq!(
            loaded
                .themes
                .iter()
                .map(|theme| theme.filename.as_str())
                .collect::<Vec<_>>(),
            ["selected.json"]
        );
        assert!(loaded.problem.unwrap().contains("512"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
