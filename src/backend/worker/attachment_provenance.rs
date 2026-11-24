//! Native download metadata for complete chat attachments, before publication.

use std::{io, path::Path};

/// A filesystem without native metadata can still hold the attachment.
/// Other failures are diagnostic only: the verified bytes can still be saved.
pub(super) fn mark(path: &Path) {
    report(mark_file(path));
}

fn report(result: io::Result<()>) {
    if let Err(error) = result
        && error.kind() != io::ErrorKind::Unsupported
    {
        log::warn!(
            "could not mark attachment provenance ({:?}, OS code {:?})",
            error.kind(),
            error.raw_os_error()
        );
    }
}

#[cfg(windows)]
pub(super) fn mark_saved_copy(path: &Path, writable: std::fs::Permissions) -> io::Result<()> {
    let permissions = std::fs::symlink_metadata(path)?.permissions();
    if !permissions.readonly() {
        mark(path);
        return Ok(());
    }
    // A native copy can inherit read-only, which also blocks ADS creation.
    // Use the staging file's original permissions while marking, then restore
    // the copied permissions before making it visible at its final name.
    if let Err(error) = std::fs::set_permissions(path, writable) {
        report(Err(error));
        return Ok(());
    }
    mark(path);
    std::fs::set_permissions(path, permissions)
}

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{ERROR_INVALID_FUNCTION, ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED},
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_ID_INFO, FILE_SHARE_READ, FILE_SHARE_WRITE, FileIdInfo, GetFileInformationByHandle,
        GetFileInformationByHandleEx, GetVolumeInformationByHandleW, MoveFileExW,
    },
    System::SystemServices::FILE_NAMED_STREAMS,
};

#[cfg(windows)]
fn stream_path(path: &Path) -> std::path::PathBuf {
    let mut stream = path.as_os_str().to_owned();
    stream.push(":Zone.Identifier:$DATA");
    stream.into()
}

#[cfg(windows)]
const INTERNET_ZONE: &[u8] = b"[ZoneTransfer]\r\nZoneId=3\r\n";

#[cfg(windows)]
fn has_untrusted_zone(bytes: &[u8]) -> bool {
    // Profile files can be UTF-16LE with a BOM. In a narrow record,
    // optional fields may use a Windows code page. The zone syntax is ASCII.
    let value = if let Some(wide) = bytes.strip_prefix(&[0xff, 0xfe]) {
        let (wide, remainder) = wide.as_chunks::<2>();
        if !remainder.is_empty() {
            return false;
        }
        let wide: Vec<_> = wide.iter().copied().map(u16::from_le_bytes).collect();
        let Ok(value) = String::from_utf16(&wide) else {
            return false;
        };
        std::borrow::Cow::Owned(value)
    } else {
        String::from_utf8_lossy(bytes)
    };
    if value
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\r' | '\n' | '\t'))
    {
        return false;
    }
    let mut section = false;
    let mut seen_section = false;
    let mut zone = None;
    for line in value.lines().map(str::trim_ascii) {
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = name.eq_ignore_ascii_case("ZoneTransfer");
            if section && std::mem::replace(&mut seen_section, true) {
                return false;
            }
        } else if let Some((key, value)) = line.split_once('=') {
            if section && key.trim_ascii().eq_ignore_ascii_case("ZoneId") {
                if zone.is_some() {
                    return false;
                }
                zone = Some(value.trim_ascii());
            }
        } else {
            return false;
        }
    }
    matches!(zone, Some("3" | "4"))
}

#[cfg(windows)]
fn update_zone(
    stream: &mut std::fs::File,
    write: impl FnOnce(&mut std::fs::File) -> io::Result<()>,
) -> io::Result<()> {
    use std::io::{Read as _, Seek as _};
    const MAX_ZONE_BYTES: u64 = 64 * 1024;
    let mut existing = Vec::new();
    stream.take(MAX_ZONE_BYTES + 1).read_to_end(&mut existing)?;
    if existing.len() as u64 > MAX_ZONE_BYTES {
        return Err(io::ErrorKind::InvalidData.into());
    }
    if has_untrusted_zone(&existing) {
        return Ok(());
    }
    stream.rewind()?;
    stream.set_len(0)?;
    // A short write must never leave a prefix mistaken for a valid record.
    write(stream).inspect_err(|_| {
        let _ = stream.set_len(0);
    })
}

#[cfg(windows)]
fn mark_file(path: &Path) -> io::Result<()> {
    use std::{io::Write as _, os::windows::fs::OpenOptionsExt as _};

    // Hold the staging file open without sharing deletion so the name cannot
    // be replaced while opening the stream. Never follow a reparse point here.
    let base = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    use std::os::windows::fs::MetadataExt as _;
    let metadata = base.metadata()?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(stream_path(path))
    {
        Ok(mut stream) => update_zone(&mut stream, |stream| stream.write_all(INTERNET_ZONE)),
        Err(error) => {
            // FAT/exFAT and some shares report different errors for ADS.
            // Query the actual file's volume only when stream creation fails.
            use std::os::windows::io::AsRawHandle as _;
            let mut flags = 0;
            // SAFETY: the file handle and flags output live through the call.
            let queried = unsafe {
                GetVolumeInformationByHandleW(
                    base.as_raw_handle(),
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut flags,
                    std::ptr::null_mut(),
                    0,
                )
            };
            Err(stream_error(error, (queried != 0).then_some(flags)))
        }
    }
}

#[cfg(windows)]
fn stream_error(error: io::Error, volume_flags: Option<u32>) -> io::Error {
    if volume_flags.is_some_and(|flags| flags & FILE_NAMED_STREAMS == 0) {
        io::ErrorKind::Unsupported.into()
    } else {
        error
    }
}

#[cfg(target_os = "macos")]
const QUARANTINE: &std::ffi::CStr = c"com.apple.quarantine";

#[cfg(all(test, target_os = "macos"))]
fn c_path(path: &Path) -> io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt as _;
    std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::ErrorKind::InvalidInput.into())
}

#[cfg(target_os = "macos")]
fn quarantine_flags(value: &[u8]) -> Option<u16> {
    let value = std::str::from_utf8(value).ok()?;
    if value.chars().any(char::is_control) {
        return None;
    }
    let mut fields = value.split(';');
    let flags = fields.next()?;
    let seconds = fields.next()?;
    let agent = fields.next()?;
    let _event = fields.next();
    if flags.len() != 4
        || !flags.bytes().all(|b| b.is_ascii_hexdigit())
        || !seconds.bytes().all(|b| b.is_ascii_hexdigit())
        || u64::from_str_radix(seconds, 16).ok()? == 0
        || agent.is_empty()
        || fields.next().is_some()
    {
        return None;
    }
    u16::from_str_radix(flags, 16).ok()
}

#[cfg(target_os = "macos")]
fn quarantine_value(file: &std::fs::File) -> io::Result<Option<Vec<u8>>> {
    use std::os::fd::AsRawFd as _;
    let mut bytes = [0; 4096];
    // SAFETY: the descriptor, C string and writable buffer live through the call.
    let size = unsafe {
        libc::fgetxattr(
            file.as_raw_fd(),
            QUARANTINE.as_ptr(),
            bytes.as_mut_ptr().cast(),
            bytes.len(),
            0,
            0,
        )
    };
    if size >= 0 {
        return Ok(Some(bytes[..size as usize].to_vec()));
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ENOATTR) => Ok(None),
        Some(libc::ENOTSUP | libc::EOPNOTSUPP) => Err(io::ErrorKind::Unsupported.into()),
        _ => Err(error),
    }
}

#[cfg(target_os = "macos")]
fn set_quarantine_value(file: &std::fs::File, value: &[u8], options: i32) -> io::Result<()> {
    use std::os::fd::AsRawFd as _;
    // SAFETY: the descriptor, C string and input byte range live through the call.
    if unsafe {
        libc::fsetxattr(
            file.as_raw_fd(),
            QUARANTINE.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
            options,
        )
    } == 0
    {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ENOTSUP | libc::EOPNOTSUPP) => Err(io::ErrorKind::Unsupported.into()),
        _ => Err(error),
    }
}

#[cfg(target_os = "macos")]
fn mark_file(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    mark_quarantine(&file)
}

#[cfg(target_os = "macos")]
fn mark_quarantine(file: &std::fs::File) -> io::Result<()> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let existing = quarantine_value(file)?;
    if let Some(value) = &existing
        && let Some(flags) = quarantine_flags(value)
    {
        // WebKit's DOWNLOAD and USER_APPROVED bits. A new unpublished copy
        // needs its own approval. Keep the other flags and record fields.
        let flags_for_copy = (flags | 0x0001) & !0x0040;
        if flags_for_copy == flags {
            return Ok(());
        }
        let mut updated = value.clone();
        updated[..4].copy_from_slice(format!("{flags_for_copy:04x}").as_bytes());
        return set_quarantine_value(file, &updated, libc::XATTR_REPLACE);
    }
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_secs();
    // The macOS browser download record (0081) includes the download bit
    // (WebKit's QTN_FLAG_DOWNLOAD) and no user-approval bit. Its fields are
    // flags, Unix time in hex, agent, and the optional quarantine event ID.
    let value = format!("0081;{seconds:x};ZapFast;");
    let options = if existing.is_some() {
        libc::XATTR_REPLACE
    } else {
        libc::XATTR_CREATE
    };
    set_quarantine_value(file, value.as_bytes(), options)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn mark_file(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(target_os = "macos")]
pub(super) fn copy_saved_file(source: &std::fs::File, target: &std::fs::File) -> io::Result<()> {
    use std::os::fd::AsRawFd as _;
    use std::os::unix::fs::PermissionsExt as _;
    let metadata = source.metadata()?;
    // COPYFILE_STAT would also copy BSD flags such as Finder's UF_IMMUTABLE.
    // XATTR includes the resource fork and quarantine. Keep the ACL as well.
    // SAFETY: both file descriptors are live, and fcopyfile accepts a null state.
    if unsafe {
        libc::fcopyfile(
            source.as_raw_fd(),
            target.as_raw_fd(),
            std::ptr::null_mut(),
            libc::COPYFILE_DATA | libc::COPYFILE_XATTR | libc::COPYFILE_ACL,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    report(mark_quarantine(target));
    // Keep ordinary permissions without copying ownership or special mode bits.
    target.set_permissions(std::fs::Permissions::from_mode(
        metadata.permissions().mode() & 0o777,
    ))?;
    let mut times = std::fs::FileTimes::new();
    if let Ok(accessed) = metadata.accessed() {
        times = times.set_accessed(accessed);
    }
    if let Ok(modified) = metadata.modified() {
        times = times.set_modified(modified);
    }
    // Timestamp restoration is best effort on filesystems that lack it.
    let _ = target.set_times(times);
    Ok(())
}

#[cfg(target_os = "macos")]
pub(super) fn copy_publication_mark(source: &std::fs::File, target: &std::fs::File) {
    report(quarantine_value(source).and_then(|value| match value {
        Some(value) => set_quarantine_value(target, &value, libc::XATTR_CREATE),
        None => Ok(()),
    }));
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(super) fn no_exclusive_rename(error: &io::Error) -> bool {
    // tempfile can return the second hard-link error on EINVAL or ENOSYS.
    // On some volumes the hard-link operation reports EPERM.
    error.kind() == io::ErrorKind::Unsupported
        || error.raw_os_error().is_some_and(|code| {
            [
                libc::EINVAL,
                libc::ENOSYS,
                libc::EPERM,
                libc::ENOTSUP,
                libc::EOPNOTSUPP,
            ]
            .contains(&code)
        })
}

// A same-directory rename when a hard link is unavailable. The rename
// cannot replace a concurrent file.
#[cfg(windows)]
pub(super) fn rename_new(staging: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt as _;

    // Canonicalize each parent for long and Unicode paths without following
    // a link at either of the two file names.
    let wide = |path: &Path| -> io::Result<Vec<u16>> {
        let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
        let name = path.file_name().ok_or(io::ErrorKind::InvalidInput)?;
        Ok(parent
            .canonicalize()?
            .join(name)
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect())
    };
    let (staging, target) = (wide(staging)?, wide(target)?);
    // SAFETY: the paths are NUL-terminated UTF-16 buffers valid for the call.
    if unsafe { MoveFileExW(staging.as_ptr(), target.as_ptr(), 0) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
pub(super) fn rename_new(staging: &Path, target: &Path) -> io::Result<()> {
    let mut temporary = tempfile::TempPath::try_from_path(staging)?;
    // The worker already owns cleanup, and may still need the staged bytes
    // for a numbered name or for the cache fallback if publication fails.
    temporary.disable_cleanup(true);
    temporary
        .persist_noclobber(target)
        .map_err(|error| error.error)
}

#[cfg(windows)]
pub(super) fn same_file(left: &std::fs::File, right: &std::fs::File) -> io::Result<bool> {
    match (full_file_id(left)?, full_file_id(right)?) {
        (Some(left), Some(right)) => Ok(left == right),
        _ => {
            let left_info = legacy_file_info(left)?;
            let right_info = legacy_file_info(right)?;
            if legacy_volumes_differ(&left_info, &right_info) {
                return Ok(false);
            }
            Ok(legacy_file_id(left, left_info)? == legacy_file_id(right, right_info)?)
        }
    }
}

#[cfg(windows)]
fn full_file_id(file: &std::fs::File) -> io::Result<Option<(u64, [u8; 16])>> {
    use std::os::windows::io::AsRawHandle as _;
    let mut info = std::mem::MaybeUninit::<FILE_ID_INFO>::uninit();
    // SAFETY: the buffer has the type and size required for FileIdInfo.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            info.as_mut_ptr().cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    } == 0
    {
        let error = io::Error::last_os_error();
        return if file_id_info_unsupported(&error) {
            Ok(None)
        } else {
            Err(error)
        };
    }
    // SAFETY: a successful call initialized FILE_ID_INFO.
    Ok(usable_file_id(unsafe { info.assume_init() }))
}

#[cfg(windows)]
fn file_id_info_unsupported(error: &io::Error) -> bool {
    error.raw_os_error().is_some_and(|code| {
        [
            ERROR_INVALID_FUNCTION,
            ERROR_INVALID_PARAMETER,
            ERROR_NOT_SUPPORTED,
        ]
        .contains(&(code as u32))
    })
}

#[cfg(windows)]
fn usable_file_id(info: FILE_ID_INFO) -> Option<(u64, [u8; 16])> {
    let id = info.FileId.Identifier;
    (id != [0; 16] && id != [0xff; 16]).then_some((info.VolumeSerialNumber, id))
}

#[cfg(windows)]
fn legacy_file_info(file: &std::fs::File) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
    use std::os::windows::io::AsRawHandle as _;
    // A network provider can return partial information. Start with zeros.
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the handle is live and the output buffer is writable.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(info)
}

#[cfg(windows)]
fn legacy_volumes_differ(
    left: &BY_HANDLE_FILE_INFORMATION,
    right: &BY_HANDLE_FILE_INFORMATION,
) -> bool {
    // Compare the serials returned by the same API. Distinct known volumes
    // rule out equality even when one driver cannot supply a usable file ID.
    left.dwVolumeSerialNumber != 0
        && right.dwVolumeSerialNumber != 0
        && left.dwVolumeSerialNumber != right.dwVolumeSerialNumber
}

#[cfg(windows)]
fn legacy_file_id(
    file: &std::fs::File,
    info: BY_HANDLE_FILE_INFORMATION,
) -> io::Result<(u32, u64)> {
    let (fs, _) = volume_file_system(file)?;
    usable_legacy_file_id(&fs, info).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "Cannot verify attachment file identity",
        )
    })
}

#[cfg(windows)]
fn volume_file_system(file: &std::fs::File) -> io::Result<(String, u32)> {
    use std::os::windows::io::AsRawHandle as _;
    let mut fs = [0u16; 32];
    let mut flags = 0;
    // SAFETY: the handle is live and the output buffer is writable.
    if unsafe {
        GetVolumeInformationByHandleW(
            file.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut flags,
            fs.as_mut_ptr(),
            fs.len() as u32,
        ) == 0
    } {
        return Err(io::Error::last_os_error());
    }
    let fs = String::from_utf16_lossy(fs.split(|unit| *unit == 0).next().unwrap_or_default());
    Ok((fs, flags))
}

#[cfg(windows)]
pub(super) fn is_exfat_source_without_streams(file: &std::fs::File) -> bool {
    volume_file_system(file).is_ok_and(|(fs, flags)| exfat_without_streams(&fs, flags))
}

#[cfg(windows)]
fn exfat_without_streams(fs: &str, flags: u32) -> bool {
    fs.eq_ignore_ascii_case("exFAT") && flags & FILE_NAMED_STREAMS == 0
}

#[cfg(windows)]
fn usable_legacy_file_id(filesystem: &str, info: BY_HANDLE_FILE_INFORMATION) -> Option<(u32, u64)> {
    let id = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    // Microsoft documents the 64-bit IDs on NTFS and FAT. FAT IDs can
    // change after a rename. ReFS can alias unrelated files.
    (matches!(filesystem, "NTFS" | "FAT" | "FAT32")
        && info.dwVolumeSerialNumber != 0
        && id != 0
        && id != u64::MAX)
        .then_some((info.dwVolumeSerialNumber, id))
}

#[cfg(unix)]
pub(super) fn same_file(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;

    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn exfat_handle_copy_requires_a_volume_without_named_streams() {
        assert!(exfat_without_streams("exFAT", 0));
        assert!(!exfat_without_streams("exFAT", FILE_NAMED_STREAMS));
        assert!(!exfat_without_streams("NTFS", 0));
        assert!(!exfat_without_streams("", 0));
    }

    #[cfg(windows)]
    #[test]
    fn windows_file_identity_matches_independent_handles_and_hard_links() {
        let dir = tempfile::tempdir().unwrap();
        let (file, alias, other) = (
            dir.path().join("source"),
            dir.path().join("alias"),
            dir.path().join("other"),
        );
        std::fs::write(&file, b"fixture").unwrap();
        std::fs::write(&other, b"fixture").unwrap();
        std::fs::hard_link(&file, &alias).unwrap();
        let first = std::fs::File::open(&file).unwrap();
        let second = std::fs::File::open(&file).unwrap();
        let alias = std::fs::File::open(&alias).unwrap();
        let other = std::fs::File::open(&other).unwrap();
        assert!(same_file(&first, &second).unwrap());
        assert!(same_file(&first, &alias).unwrap());
        assert!(!same_file(&first, &other).unwrap());
        // Exercise the fallback query directly when the test volume supports it.
        if let Ok(first_id) = legacy_file_id(&first, legacy_file_info(&first).unwrap()) {
            assert_eq!(
                first_id,
                legacy_file_id(&alias, legacy_file_info(&alias).unwrap()).unwrap()
            );
            assert_ne!(
                first_id,
                legacy_file_id(&other, legacy_file_info(&other).unwrap()).unwrap()
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_file_identity_keeps_upper_bits_and_rejects_sentinels() {
        let mut id = FILE_ID_INFO::default();
        assert_eq!(usable_file_id(id), None);
        id.FileId.Identifier = [0xff; 16];
        assert_eq!(usable_file_id(id), None);
        id.FileId.Identifier = [1; 16];
        let baseline = usable_file_id(id).unwrap();
        id.FileId.Identifier[15] = 2;
        assert_ne!(baseline, usable_file_id(id).unwrap());
        id.FileId.Identifier[15] = 1;
        id.VolumeSerialNumber = 1 << 40;
        assert_ne!(baseline, usable_file_id(id).unwrap());

        for code in [
            ERROR_INVALID_FUNCTION,
            ERROR_INVALID_PARAMETER,
            ERROR_NOT_SUPPORTED,
        ] {
            assert!(file_id_info_unsupported(&io::Error::from_raw_os_error(
                code as i32
            )));
        }
        assert!(!file_id_info_unsupported(&io::Error::from_raw_os_error(5)));
        let mut legacy = BY_HANDLE_FILE_INFORMATION {
            dwVolumeSerialNumber: 7,
            ..Default::default()
        };
        assert_eq!(usable_legacy_file_id("NTFS", legacy), None);
        legacy.nFileIndexHigh = u32::MAX;
        legacy.nFileIndexLow = u32::MAX;
        assert_eq!(usable_legacy_file_id("FAT32", legacy), None);
        legacy.nFileIndexLow = 7;
        assert!(usable_legacy_file_id("FAT32", legacy).is_some());
        assert!(usable_legacy_file_id("NTFS", legacy).is_some());
        assert_eq!(usable_legacy_file_id("ReFS", legacy), None);
        assert_eq!(usable_legacy_file_id("exFAT", legacy), None);
        assert_eq!(usable_legacy_file_id("unknown", legacy), None);
        legacy.dwVolumeSerialNumber = 0;
        assert_eq!(usable_legacy_file_id("NTFS", legacy), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_legacy_volume_comparison_can_rule_out_equality_without_file_ids() {
        let refs = BY_HANDLE_FILE_INFORMATION {
            dwVolumeSerialNumber: 7,
            nFileIndexHigh: u32::MAX,
            nFileIndexLow: u32::MAX,
            ..Default::default()
        };
        let mut fat = BY_HANDLE_FILE_INFORMATION {
            dwVolumeSerialNumber: 8,
            nFileIndexLow: 42,
            ..Default::default()
        };
        assert_eq!(usable_legacy_file_id("ReFS", refs), None);
        assert!(legacy_volumes_differ(&refs, &fat));
        assert!(legacy_volumes_differ(&fat, &refs));
        fat.dwVolumeSerialNumber = 7;
        assert!(!legacy_volumes_differ(&refs, &fat));
        fat.dwVolumeSerialNumber = 0;
        assert!(!legacy_volumes_differ(&refs, &fat));
    }

    #[cfg(windows)]
    fn read(path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(stream_path(path))
    }

    #[cfg(target_os = "macos")]
    fn read(path: &Path) -> io::Result<Vec<u8>> {
        let path = c_path(path)?;
        let mut bytes = [0u8; 1024];
        // SAFETY: both C strings and the writable buffer live through the call.
        let size = unsafe {
            libc::getxattr(
                path.as_ptr(),
                QUARANTINE.as_ptr(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
                0,
                libc::XATTR_NOFOLLOW,
            )
        };
        if size < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(bytes[..size as usize].to_vec())
        }
    }

    pub(in crate::backend::worker) fn assert_download_mark(path: &Path) {
        #[cfg(windows)]
        assert_eq!(read(path).unwrap(), b"[ZoneTransfer]\r\nZoneId=3\r\n");
        #[cfg(target_os = "macos")]
        {
            let value = String::from_utf8(read(path).unwrap()).unwrap();
            let fields: Vec<_> = value.split(';').collect();
            assert_eq!(fields.len(), 4);
            assert_eq!(fields[0], "0081");
            assert!(u64::from_str_radix(fields[1], 16).unwrap() > 0);
            assert_eq!(&fields[2..], &["ZapFast", ""]);
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        assert!(path.is_file());
    }

    pub(in crate::backend::worker) fn assert_unmarked(path: &Path) {
        #[cfg(windows)]
        assert_eq!(read(path).unwrap_err().kind(), io::ErrorKind::NotFound);
        #[cfg(target_os = "macos")]
        assert_eq!(read(path).unwrap_err().raw_os_error(), Some(libc::ENOATTR));
        #[cfg(not(any(windows, target_os = "macos")))]
        assert!(path.is_file());
    }

    #[test]
    fn marking_preserves_bytes_and_is_idempotent() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("relatório 日本語.txt");
        let other = folder.path().join("unrelated.txt");
        std::fs::write(&file, b"attachment").unwrap();
        std::fs::write(&other, b"other").unwrap();
        mark_file(&file).unwrap();
        #[cfg(any(windows, target_os = "macos"))]
        let original = read(&file).unwrap();
        mark_file(&file).unwrap();
        #[cfg(any(windows, target_os = "macos"))]
        assert_eq!(read(&file).unwrap(), original);
        assert_download_mark(&file);
        assert_unmarked(&other);
        assert_eq!(std::fs::read(&file).unwrap(), b"attachment");
        assert_eq!(std::fs::read(&other).unwrap(), b"other");
        assert_eq!(std::fs::read_dir(folder.path()).unwrap().count(), 2);
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn existing_security_metadata_is_kept() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("existing.txt");
        std::fs::write(&file, b"fixture").unwrap();
        #[cfg(windows)]
        let existing = b"[ZoneTransfer]\r\nZoneId=4\r\n";
        #[cfg(windows)]
        std::fs::write(stream_path(&file), existing).unwrap();
        #[cfg(target_os = "macos")]
        let existing = b"0083;650912b5;Fixture;";
        #[cfg(target_os = "macos")]
        {
            let path = c_path(&file).unwrap();
            // SAFETY: both C strings and the input byte range are valid.
            assert_eq!(
                unsafe {
                    libc::setxattr(
                        path.as_ptr(),
                        QUARANTINE.as_ptr(),
                        existing.as_ptr().cast(),
                        existing.len(),
                        0,
                        libc::XATTR_NOFOLLOW,
                    )
                },
                0
            );
        }
        mark_file(&file).unwrap();
        assert_eq!(read(&file).unwrap(), existing);
        let copy = folder.path().join("copy.txt");
        super::super::save_attachment_as(&file, &copy).unwrap();
        assert_eq!(read(&copy).unwrap(), existing);
    }

    #[cfg(windows)]
    #[test]
    fn windows_repairs_insufficient_zones_on_the_new_copy_only() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let copy = dir.path().join("copy.txt");
        std::fs::write(&source, b"fixture").unwrap();
        for marker in [
            &b"[ZoneTransfer]\r\nZoneId=0\r\n"[..],
            b"[ZoneTransfer]\r\nZoneId=1\r\n",
            b"[ZoneTransfer]\r\nZoneId=2\r\n",
            b"",
            b"[ZoneTransfer]\r\nZone",
            b"ZoneId=4",
            b"[ZoneTransfer]\r\nZoneId=3\r\nZoneId=0\r\n",
        ] {
            std::fs::write(stream_path(&source), marker).unwrap();
            super::super::save_attachment_as(&source, &copy).unwrap();
            assert_eq!(read(&source).unwrap(), marker);
            assert_download_mark(&copy);
            assert_eq!(std::fs::read(&copy).unwrap(), b"fixture");
        }
        let existing = b"[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://example.invalid/\r\n";
        std::fs::write(stream_path(&copy), existing).unwrap();
        mark_file(&copy).unwrap();
        assert_eq!(read(&copy).unwrap(), existing);
        let utf16: Vec<_> = "\u{feff}[ZoneTransfer]\r\nZoneId=4\r\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        std::fs::write(stream_path(&copy), &utf16).unwrap();
        mark_file(&copy).unwrap();
        assert_eq!(read(&copy).unwrap(), utf16);
    }

    #[cfg(windows)]
    #[test]
    fn a_partial_ads_write_is_an_error_and_a_later_attempt_repairs_it() {
        use std::io::Write as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.txt");
        std::fs::write(&path, b"fixture").unwrap();
        let mut stream = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(stream_path(&path))
            .unwrap();
        let failure = update_zone(&mut stream, |stream| {
            stream.write_all(b"[ZoneTransfer]\r\nZone")?;
            Err(io::ErrorKind::WriteZero.into())
        })
        .unwrap_err();
        drop(stream);
        assert_eq!(failure.kind(), io::ErrorKind::WriteZero);
        assert!(read(&path).unwrap().is_empty());
        mark_file(&path).unwrap();
        assert_download_mark(&path);
        assert_eq!(std::fs::read(&path).unwrap(), b"fixture");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_new_copy_gets_download_quarantine_without_inheriting_approval() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let copy = dir.path().join("copy.txt");
        std::fs::write(&source, b"fixture").unwrap();
        let file = std::fs::File::open(&source).unwrap();
        for (value, expected) in [
            (
                &b"00c7;650912b5;Fixture;"[..],
                &b"0087;650912b5;Fixture;"[..],
            ),
            (b"0086;650912b5;Fixture;", b"0087;650912b5;Fixture;"),
            (b"", b""),
            (b"not a quarantine record", b""),
        ] {
            set_quarantine_value(&file, value, 0).unwrap();
            super::super::save_attachment_as(&source, &copy).unwrap();
            assert_eq!(read(&source).unwrap(), value);
            if expected.is_empty() {
                assert_download_mark(&copy);
            } else {
                assert_eq!(read(&copy).unwrap(), expected);
            }
            assert_eq!(std::fs::read(&copy).unwrap(), b"fixture");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finder_lock_stays_on_the_source_across_save_as_and_save_all() {
        use std::os::{
            fd::AsRawFd as _, macos::fs::MetadataExt as _, unix::fs::PermissionsExt as _,
        };

        struct FinderLock<'a>(&'a std::fs::File);
        impl Drop for FinderLock<'_> {
            fn drop(&mut self) {
                // SAFETY: the descriptor remains open through the guard's drop.
                unsafe { libc::fchflags(self.0.as_raw_fd(), 0) };
            }
        }

        for locked in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let source = dir.path().join("source.txt");
            let existing = dir.path().join("replace.txt");
            let new = dir.path().join("new.txt");
            std::fs::write(&source, b"fixture").unwrap();
            std::fs::write(&existing, b"previous").unwrap();
            let input = std::fs::File::open(&source).unwrap();
            let approved = b"00c7;650912b5;Fixture;";
            set_quarantine_value(&input, approved, 0).unwrap();
            input
                .set_permissions(std::fs::Permissions::from_mode(0o444))
                .unwrap();
            let flag = if locked { libc::UF_IMMUTABLE } else { 0 };
            let _unlock = FinderLock(&input);
            // SAFETY: input is the live handle of our synthetic fixture.
            assert_eq!(unsafe { libc::fchflags(input.as_raw_fd(), flag) }, 0);

            super::super::save_attachment_as(&source, &existing).unwrap();
            super::super::save_attachment_as(&source, &new).unwrap();
            let album = super::super::save_album_copy(&source, dir.path(), "album.txt").unwrap();
            for target in [&existing, &new, &album] {
                assert_eq!(std::fs::read(target).unwrap(), b"fixture");
                assert_eq!(read(target).unwrap(), b"0087;650912b5;Fixture;");
                let metadata = std::fs::metadata(target).unwrap();
                assert_eq!(metadata.st_flags() & libc::UF_IMMUTABLE, 0);
                assert_eq!(metadata.permissions().mode() & 0o777, 0o444);
                assert_eq!(std::fs::read(&source).unwrap(), b"fixture");
                assert_eq!(read(&source).unwrap(), approved);
                assert_eq!(
                    input.metadata().unwrap().st_flags() & libc::UF_IMMUTABLE,
                    flag
                );
            }
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 4);
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_read_only_copy_gets_a_mark_and_keeps_its_permissions() {
        let folder = tempfile::tempdir().unwrap();
        let source = folder.path().join("source.txt");
        let target = folder.path().join("copy.txt");
        std::fs::write(&source, b"fixture").unwrap();
        let mut permissions = std::fs::metadata(&source).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&source, permissions).unwrap();

        super::super::save_attachment_as(&source, &target).unwrap();
        assert_download_mark(&target);
        assert_unmarked(&source);
        assert!(std::fs::metadata(&target).unwrap().permissions().readonly());
        assert_eq!(std::fs::read(&target).unwrap(), b"fixture");
        assert_eq!(std::fs::read_dir(folder.path()).unwrap().count(), 2);
    }

    #[cfg(windows)]
    #[test]
    fn only_a_known_volume_without_streams_suppresses_stream_errors() {
        for (flags, expected) in [
            (Some(0), io::ErrorKind::Unsupported),
            (Some(FILE_NAMED_STREAMS), io::ErrorKind::PermissionDenied),
            (None, io::ErrorKind::PermissionDenied),
        ] {
            assert_eq!(
                stream_error(io::ErrorKind::PermissionDenied.into(), flags).kind(),
                expected
            );
        }
    }

    #[test]
    fn rename_keeps_the_mark_and_cannot_replace_a_collision() {
        let folder = tempfile::tempdir().unwrap();
        let staging = folder.path().join(".staged.part");
        let collision = folder.path().join("existing.txt");
        let target = folder.path().join("salvo 日本語.txt");
        std::fs::write(&staging, b"attachment").unwrap();
        std::fs::write(&collision, b"existing").unwrap();
        mark_file(&staging).unwrap();
        assert_eq!(
            rename_new(&staging, &collision).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        rename_new(&staging, &target).unwrap();
        assert_download_mark(&target);
        assert_unmarked(&collision);
        assert_eq!(std::fs::read(&target).unwrap(), b"attachment");
        assert_eq!(std::fs::read(&collision).unwrap(), b"existing");
        assert!(!staging.exists());
        #[cfg(any(windows, target_os = "macos"))]
        assert!(mark_file(&staging).is_err());
        assert!(!staging.exists());
    }
}
