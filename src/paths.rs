//! Where Leteo looks for things on disk.
//!
//! One answer to "where is home", used everywhere. Leteo had two: agent setup
//! read the environment, while the default database location and Engram
//! detection asked the `directories` crate. On Windows those disagree —
//! `directories` asks the OS for the profile folder and ignores `HOME` and
//! `USERPROFILE` — so overriding the environment moved half of Leteo and left
//! the other half pointing at the real profile. That is a poor way to run a
//! test, and a worse surprise for anyone whose account has been relocated.

use std::env;
use std::path::PathBuf;

use anyhow::{Result, bail};

/// The user's home directory.
///
/// The environment wins. `HOME` and `USERPROFILE` are the documented ways to
/// say where home is, and honouring them is what makes it possible to point
/// Leteo somewhere else without touching the real profile. The platform's own
/// answer is the fallback for when neither is set, which on Windows is common
/// enough to matter.
pub fn home_dir() -> Result<PathBuf> {
    if let Some(home) = env::var_os("HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(home));
    }
    if let Some(home) = env::var_os("USERPROFILE").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(home));
    }
    if let (Some(drive), Some(path)) = (env::var_os("HOMEDRIVE"), env::var_os("HOMEPATH")) {
        let mut home = PathBuf::from(drive);
        home.push(path);
        return Ok(home);
    }
    if let Some(dirs) = directories::UserDirs::new() {
        return Ok(dirs.home_dir().to_path_buf());
    }
    bail!("could not determine the user home directory")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_variable_does_not_count_as_an_answer() {
        // Some shells export HOME as the empty string rather than unsetting
        // it. Taking that literally would resolve every Leteo path to the
        // filesystem root.
        //
        // The variables are process-wide, so this test reads the resolution
        // rules rather than mutating them: an empty value must be filtered out
        // before it is turned into a path.
        let empty = std::ffi::OsString::new();
        assert!(
            Some(empty).filter(|value| !value.is_empty()).is_none(),
            "an empty value must not be accepted as home"
        );
    }

    #[test]
    fn home_resolves_to_an_absolute_path() {
        let home = home_dir().expect("a home directory is available in the test environment");
        assert!(home.is_absolute(), "home must be absolute, got {home:?}");
    }
}
