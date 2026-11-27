//! The microphone and speaker picked in Settings, for whatever opens one outside a call: voice
//! recording and the shared output. Calls take the same choice through their worker, which
//! checks it against the machine's own list.
//!
//! The app's settings stay the source of truth; this is the snapshot the sound threads read when
//! they open a device, so none of them has to reach into the window's state.

use std::sync::{Mutex, PoisonError};

use fastframe_audio::{Device, DeviceId};

/// The sound devices picked in Settings; `None` is the system default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preferred {
    pub microphone: Option<String>,
    pub speaker: Option<String>,
}

static PREFERRED: Mutex<Preferred> = Mutex::new(Preferred {
    microphone: None,
    speaker: None,
});

/// What Settings picked.
pub fn preferred() -> Preferred {
    PREFERRED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Takes a new choice. A recording already running keeps its microphone; the shared output moves
/// to a new speaker once what it plays has ended, so nothing is cut off mid-sound.
pub fn set_preferred(choice: Preferred) {
    let speaker_changed = {
        let mut current = PREFERRED.lock().unwrap_or_else(PoisonError::into_inner);
        let changed = current.speaker != choice.speaker;
        *current = choice;
        changed
    };
    if speaker_changed {
        super::output::retarget();
    }
}

/// The device to open for a choice. An id the machine no longer has is fastframe-audio's to
/// handle: it opens the system default instead, which is what an unplugged device should give.
pub fn device(choice: Option<&str>) -> Device {
    match choice.filter(|id| !id.is_empty()) {
        Some(id) => Device::Id(DeviceId::new(id)),
        None => Device::Default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_choice_opens_the_system_default() {
        assert!(matches!(device(None), Device::Default));
        assert!(matches!(device(Some("")), Device::Default));
    }

    #[test]
    fn a_choice_opens_that_device_by_its_id() {
        match device(Some("alsa_input.usb-mic")) {
            Device::Id(id) => assert_eq!(id.as_str(), "alsa_input.usb-mic"),
            other => panic!("expected a device id, got {other:?}"),
        }
    }
}
