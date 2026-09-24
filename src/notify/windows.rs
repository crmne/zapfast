//! Windows toast identity, click handling, and compact sender avatars,
//! including portable builds.

use super::NotificationTarget;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RegCloseKey, RegCreateKeyW, RegSetValueExW,
};
use winrt_notification::{IconCrop, Toast};

const APPLICATION_ID: &str = "me.paolino.zapfast";

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn register_identity() -> std::io::Result<()> {
    let path = wide(&format!(
        r"Software\Classes\AppUserModelId\{APPLICATION_ID}"
    ));
    let name = wide("DisplayName");
    let value = wide("ZapFast");
    let mut key = std::ptr::null_mut();
    // All buffers are NUL-terminated UTF-16 and remain alive during each call.
    let status = unsafe { RegCreateKeyW(HKEY_CURRENT_USER, path.as_ptr(), &mut key) };
    if status != 0 {
        return Err(std::io::Error::from_raw_os_error(status as i32));
    }
    let status = unsafe {
        RegSetValueExW(
            key,
            name.as_ptr(),
            0,
            REG_SZ,
            value.as_ptr().cast(),
            (value.len() * size_of::<u16>()) as u32,
        )
    };
    // Close the key even when writing its display name failed.
    unsafe { RegCloseKey(key) };
    if status != 0 {
        return Err(std::io::Error::from_raw_os_error(status as i32));
    }
    Ok(())
}

fn notification(title: &str, body: &str, picture: Option<&Path>, system_sound: bool) -> Toast {
    let toast = Toast::new(APPLICATION_ID).title(title).text1(body);
    // Without the system sound the toast is silent; a custom sound is played
    // by ZapFast itself.
    let toast = if system_sound {
        toast
    } else {
        toast.sound(None)
    };
    if let Some(picture) = picture {
        toast.icon(picture, IconCrop::Circular, "Sender")
    } else {
        toast
    }
}

pub(super) fn show(
    title: &str,
    body: &str,
    picture: Option<&Path>,
    system_sound: bool,
    target: NotificationTarget,
    opened: Arc<Mutex<Vec<NotificationTarget>>>,
    wake: impl Fn() + Send + 'static,
) -> anyhow::Result<()> {
    static REGISTERED: OnceLock<Result<(), String>> = OnceLock::new();
    if let Err(error) = REGISTERED.get_or_init(|| register_identity().map_err(|e| e.to_string())) {
        anyhow::bail!("notification identity unavailable: {error}");
    }
    // Clicking the toast runs this on the notification system's own thread, so
    // the wake and the queue write never touch the interface thread.
    notification(title, body, picture, system_sound)
        .on_activated(move |_| {
            opened
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(target.clone());
            wake();
            Ok(())
        })
        .show()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installer_shortcuts_use_the_toast_identity() {
        let installer = include_str!("../../packaging/windows/zapfast.iss");
        let shortcuts: Vec<_> = installer
            .lines()
            .filter(|line| {
                line.starts_with("Name: \"{autoprograms}")
                    || line.starts_with("Name: \"{autodesktop}")
            })
            .collect();
        assert_eq!(shortcuts.len(), 2);
        for shortcut in shortcuts {
            assert!(shortcut.contains(&format!("AppUserModelID: \"{APPLICATION_ID}\"")));
        }
    }
}
