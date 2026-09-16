//! Optional, per-user setup for Linux packages. Runs in the theme worker.

use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone)]
pub(super) struct Setup {
    assets: PathBuf,
    home: PathBuf,
}

impl Setup {
    pub(super) fn discover() -> Option<Self> {
        let executable = std::env::current_exe().ok()?;
        Some(Self {
            assets: executable.parent()?.parent()?.join("share/zapfast/omarchy"),
            home: directories::BaseDirs::new()?.home_dir().to_path_buf(),
        })
    }

    pub(super) fn available(&self) -> bool {
        self.assets.is_dir()
            && self.home.join(".config/omarchy").is_dir()
            && self
                .home
                .join(".local/state/omarchy/current/theme")
                .is_dir()
    }

    pub(super) fn active(&self) -> bool {
        self.home.join(".config/omarchy").is_dir() && self.watch_directory().is_dir()
    }

    pub(super) fn watch_directory(&self) -> PathBuf {
        self.home.join(".local/state/omarchy/current")
    }

    /// Following the desktop must also work in Cargo and portable builds,
    /// before a package has installed its optional template and hook.
    pub(super) fn current_theme(&self) -> io::Result<super::custom::CustomTheme> {
        self.current_theme_with(read_colors)
    }

    fn current_theme_with(
        &self,
        colors: impl FnOnce(&Path) -> io::Result<String>,
    ) -> io::Result<super::custom::CustomTheme> {
        let current = self.watch_directory().join("theme");
        let rendered = current.join("zapfast.json");
        let text = if rendered.is_file() {
            read_small(&rendered)?
        } else {
            let custom = self.home.join(".config/omarchy/themed/zapfast.json.tpl");
            let template = if custom.is_file() {
                read_small(&custom)?
            } else {
                include_str!("../../contrib/omarchy/zapfast.json.tpl").to_owned()
            };
            render_seed(&template, &colors(&current.join("colors.toml"))?)?
        };
        Ok(super::custom::CustomTheme {
            filename: "omarchy.json".into(),
            palette: super::custom::parse_palette(&text).map_err(io::Error::other)?,
        })
    }

    pub(super) fn install(&self, themes: &Path) -> io::Result<()> {
        let config = self.home.join(".config/omarchy");
        let current = self.home.join(".local/state/omarchy/current/theme");
        // Only an installed package on a configured Omarchy desktop opts in.
        // Cargo builds, portable binaries and other desktops do nothing.
        if !self.available() {
            return Ok(());
        }
        let template = read_small(&self.assets.join("zapfast.json.tpl"))?;
        let hook = read_small(&self.assets.join("zapfast-theme"))?;
        let template_path = config.join("themed/zapfast.json.tpl");
        create_only(&template_path, template.as_bytes(), 0o644)?;
        create_only(
            &config.join("hooks/theme-set.d/zapfast-theme"),
            hook.as_bytes(),
            0o755,
        )?;

        let destination = themes.join("omarchy.json");
        if fs::symlink_metadata(&destination).is_ok() {
            return Ok(());
        }
        // Seed the current palette without reapplying the desktop theme.
        // Future changes use Omarchy's own template renderer and installed hook.
        let rendered = current.join("zapfast.json");
        let palette = if rendered.is_file() {
            read_small(&rendered)?
        } else {
            render_seed(
                &read_small(&template_path)?,
                &read_colors(&current.join("colors.toml"))?,
            )?
        };
        super::custom::parse_palette(&palette).map_err(io::Error::other)?;
        create_only(&destination, palette.as_bytes(), 0o644)
    }
}

fn read_colors(path: &Path) -> io::Result<String> {
    let output = Command::new("omarchy-theme-color")
        .arg("--file")
        .arg(path)
        .arg("--all")
        .stdin(Stdio::null())
        .output()?;
    if !output.status.success() || output.stdout.len() > 64 * 1024 {
        return Err(io::Error::other(
            "Omarchy's current colors could not be read",
        ));
    }
    String::from_utf8(output.stdout).map_err(io::Error::other)
}

fn read_small(path: &Path) -> io::Result<String> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 64 * 1024 {
        return Err(io::Error::other("Omarchy theme file exceeds 64 KiB"));
    }
    String::from_utf8(bytes).map_err(io::Error::other)
}

/// Publish complete files only when the destination does not already exist.
/// Even a concurrent user edit or a broken symbolic link must be preserved.
fn create_only(destination: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::other("missing parent"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".zapfast-setup-{:016x}", rand::random::<u64>()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        match fs::hard_link(&temporary, destination) {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
            result => result,
        }
    })();
    let _ = fs::remove_file(temporary);
    result
}

/// Resolve the color placeholders used by our shipped JSON template for its
/// first use. Omarchy itself remains responsible for later theme changes.
fn render_seed(template: &str, colors: &str) -> io::Result<String> {
    let colors: BTreeMap<_, _> = colors
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .collect();
    let mut output = String::new();
    let mut rest = template;
    while let Some((before, token)) = rest.split_once("{{") {
        output.push_str(before);
        let (token, after) = token
            .split_once("}}")
            .ok_or_else(|| io::Error::other("incomplete palette placeholder"))?;
        let words: Vec<_> = token.split_whitespace().collect();
        let get = |key: &str| {
            colors
                .get(key)
                .copied()
                .ok_or_else(|| io::Error::other("missing Omarchy color"))
        };
        match words.as_slice() {
            [key] => output.push_str(get(key)?),
            ["mix", from, to, percent] => {
                let percent: u32 = percent
                    .strip_suffix('%')
                    .and_then(|p| p.parse().ok())
                    .filter(|p| *p <= 100)
                    .ok_or_else(|| io::Error::other("unsupported palette mix"))?;
                let rgb = |value: &str| -> io::Result<u32> {
                    let hex = value
                        .strip_prefix('#')
                        .filter(|s| s.len() == 6)
                        .ok_or_else(|| io::Error::other("expected an RGB color"))?;
                    u32::from_str_radix(hex, 16).map_err(io::Error::other)
                };
                let (from, to) = (rgb(get(from)?)?, rgb(get(to)?)?);
                output.push('#');
                for shift in [16, 8, 0] {
                    let value = (((from >> shift) & 255) * (100 - percent)
                        + ((to >> shift) & 255) * percent
                        + 50)
                        / 100;
                    output.push_str(&format!("{value:02x}"));
                }
            }
            _ => return Err(io::Error::other("unsupported palette placeholder")),
        }
        rest = after;
    }
    output.push_str(rest);
    super::custom::parse_palette(&output).map_err(io::Error::other)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    const TEMPLATE: &str = include_str!("../../contrib/omarchy/zapfast.json.tpl");
    const HOOK: &str = include_str!("../../contrib/omarchy/zapfast-theme");

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("zapfast omarchy setup {}", rand::random::<u64>()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn first_palette_matches_omarchys_renderer_in_light_and_dark_themes() {
        for (colors, expected) in [
            (
                include_str!("../../tests/fixtures/omarchy/catppuccin.tsv"),
                include_str!("../../tests/fixtures/omarchy/catppuccin.json"),
            ),
            (
                include_str!("../../tests/fixtures/omarchy/catppuccin-latte.tsv"),
                include_str!("../../tests/fixtures/omarchy/catppuccin-latte.json"),
            ),
        ] {
            let actual = render_seed(TEMPLATE, colors).unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
                serde_json::from_str::<serde_json::Value>(expected).unwrap()
            );
        }
        assert!(render_seed(TEMPLATE, "mode\tdark\n").is_err());
        assert!(render_seed("{{ missing }}", "").is_err());
        assert!(render_seed("{{", "").is_err());
    }

    #[test]
    fn following_omarchy_needs_no_packaged_assets_or_user_hooks() {
        let directory = tempfile::tempdir().unwrap();
        let setup = Setup {
            assets: directory.path().join("missing-package-assets"),
            home: directory.path().join("home"),
        };
        let config = setup.home.join(".config/omarchy");
        let current = setup.watch_directory().join("theme");
        fs::create_dir_all(&config).unwrap();
        fs::create_dir_all(&current).unwrap();
        assert!(setup.active());
        assert!(!setup.available());
        for (colors, expected) in [
            (
                include_str!("../../tests/fixtures/omarchy/catppuccin.tsv"),
                include_str!("../../tests/fixtures/omarchy/catppuccin.json"),
            ),
            (
                include_str!("../../tests/fixtures/omarchy/catppuccin-latte.tsv"),
                include_str!("../../tests/fixtures/omarchy/catppuccin-latte.json"),
            ),
        ] {
            let theme = setup
                .current_theme_with(|path| {
                    assert_eq!(path, current.join("colors.toml"));
                    Ok(colors.into())
                })
                .unwrap();
            assert_eq!(
                theme.palette,
                super::super::custom::parse_palette(expected).unwrap()
            );
        }
        assert_eq!(
            fs::read_dir(&config).unwrap().count(),
            0,
            "following does not install desktop files"
        );
        fs::remove_dir(&current).unwrap();
        assert!(
            setup.active(),
            "keep the cached palette while Omarchy replaces its theme directory"
        );
    }

    #[test]
    fn setup_requires_both_a_package_and_an_omarchy_desktop() {
        let root = Scratch::new();
        let setup = Setup {
            assets: root.0.join("package/share/zapfast/omarchy"),
            home: root.0.join("user"),
        };
        let themes = root.0.join("profile/themes");
        setup.install(&themes).unwrap();
        assert!(!setup.home.exists());
        fs::create_dir_all(&setup.assets).unwrap();
        setup.install(&themes).unwrap();
        assert!(!setup.home.exists());
        assert!(!themes.exists());
    }

    #[test]
    fn setup_is_per_user_and_preserves_customizations_and_preferences() {
        let root = Scratch::new();
        let setup = Setup {
            assets: root.0.join("package/share/zapfast/omarchy"),
            home: root.0.join("user"),
        };
        let config = setup.home.join(".config/omarchy");
        let current = setup.home.join(".local/state/omarchy/current/theme");
        let themes = root.0.join("profile/themes");
        for path in [&setup.assets, &config, &current, &themes] {
            fs::create_dir_all(path).unwrap();
        }
        fs::write(setup.assets.join("zapfast.json.tpl"), TEMPLATE).unwrap();
        fs::write(setup.assets.join("zapfast-theme"), HOOK).unwrap();
        fs::write(current.join("zapfast.json"), "{}").unwrap();
        let settings = themes.parent().unwrap().join("settings.json");
        fs::write(&settings, "existing preferences").unwrap();
        setup.install(&themes).unwrap();
        let template = config.join("themed/zapfast.json.tpl");
        let hook = config.join("hooks/theme-set.d/zapfast-theme");
        assert_eq!(fs::read_to_string(&template).unwrap(), TEMPLATE);
        assert_eq!(fs::read_to_string(&hook).unwrap(), HOOK);
        assert_ne!(fs::metadata(&hook).unwrap().permissions().mode() & 0o100, 0);
        assert_eq!(
            fs::read_to_string(themes.join("omarchy.json")).unwrap(),
            "{}"
        );

        fs::write(&template, "user template").unwrap();
        fs::write(&hook, "user hook").unwrap();
        fs::write(themes.join("omarchy.json"), "user palette").unwrap();
        setup.install(&themes).unwrap();
        assert_eq!(fs::read_to_string(&template).unwrap(), "user template");
        assert_eq!(fs::read_to_string(&hook).unwrap(), "user hook");
        assert_eq!(
            fs::read_to_string(themes.join("omarchy.json")).unwrap(),
            "user palette"
        );
        assert_eq!(
            fs::read_to_string(&settings).unwrap(),
            "existing preferences"
        );
        assert_eq!(fs::read_dir(template.parent().unwrap()).unwrap().count(), 1);
        assert_eq!(fs::read_dir(hook.parent().unwrap()).unwrap().count(), 1);
        assert_eq!(fs::read_dir(&themes).unwrap().count(), 1);
    }

    #[test]
    fn publishing_never_replaces_an_existing_or_broken_link() {
        let root = Scratch::new();
        let outside = root.0.join("keep");
        fs::write(&outside, "unchanged").unwrap();
        for target in [&outside, &root.0.join("missing")] {
            let link = root.0.join("theme.json");
            symlink(target, &link).unwrap();
            create_only(&link, b"replacement", 0o644).unwrap();
            assert_eq!(fs::read_link(&link).unwrap(), *target);
            fs::remove_file(link).unwrap();
        }
        assert_eq!(fs::read_to_string(outside).unwrap(), "unchanged");
        assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
    }
}
