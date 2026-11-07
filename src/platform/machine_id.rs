use sha2::{Digest, Sha256};

pub fn hardware_fingerprint() -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"zapfast-node-locking-v1\0");

    #[cfg(windows)]
    {
        let guid = windows_machine_guid();
        hasher.update(guid.as_bytes());
        hasher.update(b"\0");

        let serial = windows_volume_serial();
        hasher.update(&serial.to_le_bytes());
    }

    #[cfg(target_os = "linux")]
    {
        let id = linux_machine_id();
        hasher.update(id.as_bytes());
        hasher.update(b"\0");
        let dmi = linux_dmi_uuid();
        hasher.update(dmi.as_bytes());
    }

    #[cfg(target_os = "macos")]
    {
        let uuid = macos_platform_uuid();
        hasher.update(uuid.as_bytes());
    }

    hasher.finalize().into()
}

#[cfg(windows)]
fn windows_machine_guid() -> String {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW,
    };

    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let subkey = wide(r"SOFTWARE\Microsoft\Cryptography");
    let value_name = wide("MachineGuid");

    let mut buffer = [0u16; 128];
    let mut size = (buffer.len() * std::mem::size_of::<u16>()) as u32;

    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            value_name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };

    if status != ERROR_SUCCESS {
        return "windows-default-guid".to_owned();
    }

    let len = (size as usize) >> 1;
    let slice = if len > 0 && buffer[len - 1] == 0 {
        &buffer[..len - 1]
    } else {
        &buffer[..len]
    };

    String::from_utf16_lossy(slice)
}

#[cfg(windows)]
fn windows_volume_serial() -> u32 {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetVolumeInformationW(
            lpRootPathName: *const u16,
            lpVolumeNameBuffer: *mut u16,
            nVolumeNameSize: u32,
            lpVolumeSerialNumber: *mut u32,
            lpMaximumComponentLength: *mut u32,
            lpFileSystemFlags: *mut u32,
            lpFileSystemNameBuffer: *mut u16,
            nFileSystemNameSize: u32,
        ) -> i32;
    }

    let root = [b'C' as u16, b':' as u16, b'\\' as u16, 0];
    let mut serial = 0u32;
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            std::ptr::null_mut(),
            0,
            &mut serial,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };

    if ok != 0 {
        return serial;
    }

    // Fallback to default volume if C:\ is not mapped
    let mut fallback_serial = 0u32;
    let fallback_ok = unsafe {
        GetVolumeInformationW(
            std::ptr::null(),
            std::ptr::null_mut(),
            0,
            &mut fallback_serial,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };

    if fallback_ok != 0 {
        return fallback_serial;
    }

    0x5A415046 // Constant fallback identifier ("ZAPF")
}

#[cfg(target_os = "linux")]
fn linux_machine_id() -> String {
    if let Ok(id) = std::fs::read_to_string("/etc/machine-id") {
        let trimmed = id.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }

    if let Ok(id) = std::fs::read_to_string("/var/lib/dbus/machine-id") {
        let trimmed = id.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }

    "linux-default-machine-id".to_owned()
}

#[cfg(target_os = "linux")]
fn linux_dmi_uuid() -> String {
    if let Ok(uuid) = std::fs::read_to_string("/sys/class/dmi/id/product_uuid") {
        let trimmed = uuid.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }
    String::new()
}

#[cfg(target_os = "macos")]
fn macos_platform_uuid() -> String {
    let output = std::process::Command::new("ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output();

    if let Ok(output) = output {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            if line.contains("IOPlatformUUID") {
                if let Some(uuid) = line.split('"').nth(3) {
                    return uuid.to_owned();
                }
            }
        }
    }

    "macos-default-platform-uuid".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_deterministic_and_nonzero() {
        let fp1 = hardware_fingerprint();
        let fp2 = hardware_fingerprint();
        assert_eq!(fp1, fp2);
        assert_ne!(fp1, [0u8; 32]);
    }
}
