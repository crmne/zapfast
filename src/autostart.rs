//! Starting ZapFast in the tray when the person logs in.
//!
//! The platform's own login-item entry is the only record: Settings reads it
//! back rather than keeping a copy that could disagree with it.

use std::io;
use std::path::{Path, PathBuf};

/// Argument that starts ZapFast without a window.
pub const HIDDEN: &str = "--start-hidden";

/// Whether this installation can register itself to start at login.
pub fn supported() -> bool {
    // A Flatpak cannot write the host's autostart folder; that needs the
    // background portal instead.
    std::env::var_os("FLATPAK_ID").is_none() && executable().is_some()
}

/// Whether ZapFast is registered to start at login.
pub fn enabled() -> bool {
    platform::enabled()
}

/// Registers or removes the login entry for the running executable.
pub fn set(enabled: bool) -> io::Result<()> {
    if !enabled {
        return platform::remove();
    }
    let executable = executable()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "cannot locate ZapFast"))?;
    platform::install(&executable)
}

/// The file to start: an AppImage runs from a temporary mount, so its own
/// path is the stable one.
fn executable() -> Option<PathBuf> {
    std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;

    fn entry() -> Option<PathBuf> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".config")))?;
        Some(config.join("autostart").join("zapfast.desktop"))
    }

    pub fn enabled() -> bool {
        entry().is_some_and(|path| path.is_file())
    }

    pub fn install(executable: &Path) -> io::Result<()> {
        let path = entry().ok_or_else(|| io::Error::other("no configuration directory"))?;
        std::fs::create_dir_all(path.parent().expect("autostart folder"))?;
        std::fs::write(path, desktop_entry(executable))
    }

    pub fn remove() -> io::Result<()> {
        match entry().map(std::fs::remove_file) {
            Some(Err(error)) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    pub(super) fn desktop_entry(executable: &Path) -> String {
        format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=ZapFast\n\
             Comment=Start ZapFast in the tray\n\
             Exec={} {HIDDEN}\n\
             Icon=zapfast\n\
             Terminal=false\n\
             X-GNOME-Autostart-enabled=true\n",
            exec_quote(&executable.to_string_lossy())
        )
    }

    /// Quotes a path for a desktop entry's `Exec` key.
    fn exec_quote(path: &str) -> String {
        let mut quoted = String::from("\"");
        for character in path.chars() {
            if matches!(character, '"' | '`' | '$' | '\\') {
                quoted.push('\\');
            }
            if character == '%' {
                quoted.push('%');
            }
            quoted.push(character);
        }
        quoted.push('"');
        quoted
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    const LABEL: &str = "me.paolino.zapfast";

    fn entry() -> Option<PathBuf> {
        let home = std::env::var_os("HOME")?;
        Some(
            Path::new(&home)
                .join("Library/LaunchAgents")
                .join(format!("{LABEL}.plist")),
        )
    }

    pub fn enabled() -> bool {
        entry().is_some_and(|path| path.is_file())
    }

    pub fn install(executable: &Path) -> io::Result<()> {
        let path = entry().ok_or_else(|| io::Error::other("no home directory"))?;
        std::fs::create_dir_all(path.parent().expect("LaunchAgents folder"))?;
        std::fs::write(path, launch_agent(executable))
    }

    pub fn remove() -> io::Result<()> {
        match entry().map(std::fs::remove_file) {
            Some(Err(error)) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    pub(super) fn launch_agent(executable: &Path) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>{HIDDEN}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>ProcessType</key>
    <string>Interactive</string>
</dict>
</plist>
"#,
            xml_escape(&executable.to_string_lossy())
        )
    }

    fn xml_escape(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
    };

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE: &str = "ZapFast";

    fn wide(text: &str) -> Vec<u16> {
        std::ffi::OsStr::new(text)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    pub fn enabled() -> bool {
        let (key, value) = (wide(RUN), wide(VALUE));
        // SAFETY: null-terminated UTF-16 strings; a null buffer only asks
        // whether the value exists.
        unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ) == ERROR_SUCCESS
        }
    }

    pub fn install(executable: &Path) -> io::Result<()> {
        let command = wide(&format!("\"{}\" {HIDDEN}", executable.display()));
        let (key, value) = (wide(RUN), wide(VALUE));
        // SAFETY: null-terminated UTF-16 strings; the byte length includes
        // the terminator, as REG_SZ requires.
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                REG_SZ,
                command.as_ptr().cast(),
                (command.len() * 2) as u32,
            )
        };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(status as i32))
        }
    }

    pub fn remove() -> io::Result<()> {
        let (key, value) = (wide(RUN), wide(VALUE));
        // SAFETY: null-terminated UTF-16 strings.
        let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) };
        const ERROR_FILE_NOT_FOUND: u32 = 2;
        if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(status as i32))
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod platform {
    use super::*;

    pub fn enabled() -> bool {
        false
    }

    pub fn install(_executable: &Path) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    pub fn remove() -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    #[test]
    fn the_desktop_entry_starts_hidden_and_quotes_the_path() {
        let entry =
            super::platform::desktop_entry(std::path::Path::new("/opt/Zap Fast/100%/zap\"fast"));
        assert!(entry.contains("Exec=\"/opt/Zap Fast/100%%/zap\\\"fast\" --start-hidden\n"));
        assert!(entry.starts_with("[Desktop Entry]\nType=Application\n"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_launch_agent_starts_hidden_and_escapes_the_path() {
        let plist = super::platform::launch_agent(std::path::Path::new(
            "/Applications/A&B.app/Contents/MacOS/zapfast",
        ));
        assert!(
            plist.contains("<string>/Applications/A&amp;B.app/Contents/MacOS/zapfast</string>")
        );
        assert!(plist.contains("<string>--start-hidden</string>"));
    }
}
