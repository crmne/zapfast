//! Native outbound file drags. The archive's cached file is always copied.

use std::path::Path;

pub const SUPPORTED: bool = cfg!(windows);

#[derive(Debug)]
pub enum Failure {
    Unavailable,
    Unsupported,
    Native(String),
}

pub fn failure_message(locale: crate::i18n::Locale, failure: &Failure) -> String {
    let summary = crate::i18n::gettext(locale, "Could not drag the attachment");
    let details = match failure {
        Failure::Unavailable => crate::i18n::gettext(
            locale,
            "The attachment is unavailable. Download it before dragging it.",
        )
        .into_owned(),
        Failure::Unsupported => crate::i18n::gettext(
            locale,
            "Dragging attachments out is currently available on Windows only.",
        )
        .into_owned(),
        Failure::Native(details) => details.clone(),
    };
    format!("{summary}: {details}")
}

/// Run on the window thread after the view queues its drag action.
pub fn start(path: &Path) -> Result<(), Failure> {
    if !path.is_file() {
        return Err(Failure::Unavailable);
    }
    #[cfg(windows)]
    {
        native::start(path).map_err(|error| Failure::Native(error.to_string()))
    }
    #[cfg(not(windows))]
    {
        Err(Failure::Unsupported)
    }
}

#[cfg(windows)]
mod native {
    use std::path::Path;

    use windows::Win32::System::Com::IDataObject;
    use windows::Win32::System::Ole::{
        DROPEFFECT_COPY, IDropSource, OleInitialize, OleUninitialize,
    };
    use windows::Win32::UI::Shell::{
        BHID_DataObject, IShellItem, SHCreateItemFromParsingName, SHDoDragDrop,
    };
    use windows::core::{HSTRING, Result};

    struct Ole;

    impl Ole {
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
        fn drop(&mut self) {
            // SAFETY: Balances the successful initialization on this thread.
            unsafe {
                OleUninitialize();
            }
        }
    }

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
}

#[cfg(test)]
mod failure_tests {
    #[test]
    fn stable_drag_failures_are_translated_and_native_details_are_preserved() {
        let locale = crate::i18n::Locale::PortugueseBrazil;
        assert!(super::failure_message(locale, &super::Failure::Unavailable).contains("Baixe"));
        assert!(super::failure_message(locale, &super::Failure::Unsupported).contains("Windows"));
        assert_eq!(
            super::failure_message(locale, &super::Failure::Native("fixture details".into())),
            "Não foi possível arrastar o anexo: fixture details"
        );
    }
}
