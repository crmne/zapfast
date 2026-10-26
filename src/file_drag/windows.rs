use std::path::Path;

use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::Ole::{DROPEFFECT_COPY, IDropSource, OleInitialize, OleUninitialize};
use windows::Win32::UI::Shell::{
    BHID_DataObject, IShellItem, SHCreateItemFromParsingName, SHDoDragDrop,
};
use windows::core::{HSTRING, Result};

struct Ole;

impl Ole {
    /// Initializes OLE on the current UI thread and owns its matching uninitialization.
    fn initialize() -> Result<Self> {
        // SAFETY: All calls and the matching uninitialization run on the
        // UI thread. A successful S_FALSE also needs OleUninitialize.
        unsafe {
            OleInitialize(None)?;
        }
        Ok(Self)
    }
}

impl Drop for Ole {
    /// Balances this UI thread's successful OLE initialization.
    fn drop(&mut self) {
        // SAFETY: Balances the successful initialization on this thread.
        unsafe {
            OleUninitialize();
        }
    }
}

/// Creates a Shell file-drop data object without reading attachment contents.
fn data_object(path: &Path) -> Result<IDataObject> {
    let name = HSTRING::from(path.as_os_str());
    // SAFETY: The string lives through the call. The returned COM objects
    // own their references. BHID_DataObject supplies the Shell file formats,
    // including CF_HDROP, without reading the attachment contents here.
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(&name, None)?;
        item.BindToHandler(None, &BHID_DataObject)
    }
}

/// Runs the Windows Shell drag operation with copy as its only allowed effect.
pub(super) fn start(path: &Path) -> Result<()> {
    let _ole = Ole::initialize()?;
    let data = data_object(path)?;
    // SAFETY: Run on the UI thread during a primary-button drag. The Shell
    // provides its own drop source and generic drag image for null handles.
    // COPY is the only allowed effect, so the cached source cannot be moved.
    unsafe {
        SHDoDragDrop(None, &data, None::<&IDropSource>, DROPEFFECT_COPY)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
    use windows::Win32::System::Ole::CF_HDROP;

    #[test]
    fn a_unicode_file_provides_the_native_file_drop_format() {
        let file = tempfile::Builder::new()
            .prefix("zapfast-drag-写真-")
            .suffix(".png")
            .tempfile()
            .unwrap();
        let _ole = Ole::initialize().unwrap();
        let data = data_object(file.path()).unwrap();
        let format = FORMATETC {
            cfFormat: CF_HDROP.0,
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
            ..Default::default()
        };
        // SAFETY: The format descriptor lives through the call and contains
        // no borrowed target-device data. This queries, without starting a drag.
        unsafe {
            data.QueryGetData(&format).ok().unwrap();
        }
        assert!(file.path().is_file());
    }
}
