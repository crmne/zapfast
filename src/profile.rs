//! The profile this process runs as.
//!
//! A profile is one linked device with its own files, window, tray item, and
//! control port. `default` keeps the directories and the port ZapFast has
//! always used, so an existing setup sees no change; any other name runs
//! beside it as an independent instance.

use std::str::FromStr;
use std::sync::OnceLock;

/// Profile used when neither `--profile` nor `ZAPFAST_PROFILE` is given.
pub const DEFAULT: &str = "default";

/// Loopback port used when neither `--port` nor `ZAPFAST_PORT` is given.
pub const DEFAULT_PORT: u16 = 47_119;

/// Longest accepted profile name.
const MAX_NAME: usize = 32;

/// A validated profile name. It becomes part of a directory name and of the
/// window identifier, so it is kept to characters that are safe in both.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile(String);

impl Default for Profile {
    fn default() -> Self {
        Self(DEFAULT.to_owned())
    }
}

impl Profile {
    /// The name as written on the command line, `default` included.
    pub fn name(&self) -> &str {
        &self.0
    }

    /// Whether this is the original setup, which keeps every earlier path.
    pub fn is_default(&self) -> bool {
        self.0 == DEFAULT
    }

    /// Directory and window identifier: `zapfast`, or `zapfast-work`.
    pub fn slug(&self) -> String {
        if self.is_default() {
            "zapfast".to_owned()
        } else {
            format!("zapfast-{}", self.0)
        }
    }

    /// Name shown to the user: `ZapFast`, or `ZapFast (work)`.
    pub fn label(&self) -> String {
        if self.is_default() {
            "ZapFast".to_owned()
        } else {
            format!("ZapFast ({})", self.0)
        }
    }
}

impl std::fmt::Display for Profile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for Profile {
    type Err = InvalidProfile;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let accepted = |byte: &u8| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-' || *byte == b'_'
        };
        if value.is_empty() || value.len() > MAX_NAME || !value.as_bytes().iter().all(accepted) {
            return Err(InvalidProfile);
        }
        Ok(Self(value.to_owned()))
    }
}

/// A name that cannot be part of a directory or window identifier.
#[derive(Debug, PartialEq, Eq)]
pub struct InvalidProfile;

impl std::fmt::Display for InvalidProfile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "a profile name is 1 to {MAX_NAME} characters of a-z, 0-9, '-' or '_'"
        )
    }
}

impl std::error::Error for InvalidProfile {}

static RUNNING: OnceLock<Profile> = OnceLock::new();

/// Records the profile for the whole process. `main` calls this before
/// anything reads it; a later call is ignored.
pub fn announce(profile: Profile) {
    let _ = RUNNING.set(profile);
}

/// The profile this process runs as. Tests and library use get `default`.
pub fn running() -> &'static Profile {
    RUNNING.get_or_init(Profile::default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_profile_keeps_every_earlier_name() {
        let profile = Profile::default();
        assert!(profile.is_default());
        assert_eq!(profile.slug(), "zapfast");
        assert_eq!(profile.label(), "ZapFast");
        assert_eq!("default".parse::<Profile>().unwrap(), profile);
    }

    #[test]
    fn a_named_profile_is_a_sibling_of_the_default_one() {
        let profile: Profile = "work".parse().unwrap();
        assert!(!profile.is_default());
        assert_eq!(profile.slug(), "zapfast-work");
        assert_eq!(profile.label(), "ZapFast (work)");
    }

    #[test]
    fn names_that_would_escape_a_directory_are_refused() {
        for name in [
            "",
            "..",
            "../escape",
            "work/one",
            "Work",
            "work profile",
            "work.1",
            &"w".repeat(MAX_NAME + 1),
        ] {
            assert_eq!(name.parse::<Profile>(), Err(InvalidProfile), "{name:?}");
        }
        for name in [
            "work",
            "w",
            "voiper-2",
            "second_account",
            &"w".repeat(MAX_NAME),
        ] {
            assert!(name.parse::<Profile>().is_ok(), "{name:?}");
        }
    }
}
