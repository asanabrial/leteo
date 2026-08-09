//! Files Leteo writes but does not own.
//!
//! `CLAUDE.md` and its eleven counterparts belong to the person, not to Leteo,
//! and most of them hold instructions that were written by hand. `fs::write`
//! truncates the file and then fills it, so a crash, a full disk or a killed
//! terminal in between leaves a file that is neither what it was nor what it
//! was going to be.
//!
//! That state is not hypothetical. `setup::upsert_memory_protocol` spliced its
//! block between two markers, and a file truncated mid-write kept the opening
//! marker and lost the closing one; the run after that spliced from the first
//! marker to the wrong end and took the person's own notes with it. The splice
//! now survives a damaged file — but a write that cannot damage one in the
//! first place is the better half of the fix.
//!
//! The sync journal has always written this way, because a half-written chunk
//! is a corrupt replica. The same reasoning applies to a half-written
//! instruction file, so the machinery moved here rather than staying in
//! `sync`.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Distinguishes the temporary files of concurrent writers in one process.
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

/// Puts `data` at `path`, leaving the previous contents untouched if anything
/// goes wrong.
///
/// The data lands in a temporary file beside the target, is flushed to the
/// device, and only then replaces the target in one step. A reader either sees
/// the whole of the old file or the whole of the new one, never a prefix of
/// either.
///
/// Errors name the target rather than the temporary file: which of the two the
/// operating system refused is Leteo's business, and the path the person cares
/// about is the one they asked for.
pub fn replace(path: &Path, data: &[u8]) -> io::Result<()> {
    write_then_rename(path, data, Visibility::Inherited)
}

/// Like [`replace`], for a file nobody but its owner may read.
///
/// The mode is set on the temporary file before the rename, so the contents are
/// never on disk under a name another user can open. Writing first and
/// restricting afterwards — which is what the cloud credentials did — leaves a
/// window whose length is a scheduling accident.
///
/// On Windows the file inherits the ACL of the directory it is created in,
/// which under the profile directory is already scoped to the account — so
/// `restrict` is a no-op there and the ordering below cannot be observed from
/// a Windows test. It is checked by eye and by the Unix half of CI.
///
/// Two of this module's promises are unobservable in-process and are recorded
/// here rather than covered by a test that would only appear to check them:
/// this ordering, and the `sync_all` that makes the contents durable before
/// the rename. The third — that a reader is never shown a truncated file —
/// is the one that separates this from `fs::write`, and it is guarded.
pub fn replace_private(path: &Path, data: &[u8]) -> io::Result<()> {
    write_then_rename(path, data, Visibility::OwnerOnly)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Visibility {
    Inherited,
    OwnerOnly,
}

fn write_then_rename(path: &Path, data: &[u8], visibility: Visibility) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path has no parent directory: {}", path.display()),
        )
    })?;
    fs::create_dir_all(parent)?;

    let (temporary_path, mut temporary_file) = create_temporary_file(path)?;
    let written = restrict(&temporary_file, visibility)
        .and_then(|()| temporary_file.write_all(data))
        .and_then(|()| temporary_file.sync_all());
    drop(temporary_file);
    if let Err(error) = written {
        let _ = fs::remove_file(&temporary_path);
        return Err(error);
    }

    if let Err(error) = rename_replacing(&temporary_path, path) {
        let _ = fs::remove_file(&temporary_path);
        return Err(error);
    }

    // Flushing the directory is what makes the rename itself durable. Not every
    // platform allows opening one, and where it fails there is nothing to
    // recover: the file's own contents reached the device before the rename.
    if let Ok(directory) = File::open(parent) {
        let _ = directory.sync_all();
    }
    Ok(())
}

/// Opens a file beside the target that no other writer can be holding.
///
/// The name carries the process id and a counter, so two Leteos writing the
/// same file at once — a hook and a `setup`, say — cannot land on the same
/// temporary path. `create_new` is what makes that a guarantee rather than a
/// hope: it fails instead of opening a file somebody else made.
fn create_temporary_file(target: &Path) -> io::Result<(PathBuf, File)> {
    let parent = target.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path has no parent directory: {}", target.display()),
        )
    })?;
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("path has no file name: {}", target.display()),
            )
        })?;

    for _ in 0..128 {
        let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".{file_name}.tmp.{}.{sequence}",
            std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("exhausted temporary names beside {}", target.display()),
    ))
}

#[cfg(unix)]
fn restrict(file: &File, visibility: Visibility) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    if visibility == Visibility::OwnerOnly {
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn restrict(_file: &File, _visibility: Visibility) -> io::Result<()> {
    Ok(())
}

#[cfg(not(windows))]
fn rename_replacing(source: &Path, target: &Path) -> io::Result<()> {
    fs::rename(source, target)
}

#[cfg(windows)]
fn rename_replacing(source: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    // Both paths are in the same directory, so MoveFileExW provides an atomic
    // replacement. `fs::rename` refuses an existing target on Windows.
    let renamed = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if renamed == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_was_not_there_is_created_along_with_its_directory() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("nested").join("deeper").join("notes.md");

        replace(&path, b"first").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "first");
    }

    #[test]
    fn replacing_leaves_no_temporary_file_behind() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("notes.md");

        replace(&path, b"first").unwrap();
        replace(&path, b"second").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
        let leftovers: Vec<_> = fs::read_dir(temp.path())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name())
            .filter(|name| name != "notes.md")
            .collect();
        assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
    }

    #[test]
    fn a_write_that_cannot_finish_leaves_the_previous_contents_alone() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("notes.md");
        replace(&path, b"what was there before").unwrap();

        // A directory where the temporary file wants to go: the rename cannot
        // succeed, and the question is what the target holds afterwards.
        let blocker = temp.path().join("notes.md.d");
        fs::create_dir(&blocker).unwrap();
        assert!(replace(&blocker, b"never lands").is_err());

        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "what was there before",
            "an unrelated failure must not touch a file that was written"
        );
    }

    /// Somebody reading the file is never shown a truncated one.
    ///
    /// This is the only property separating this module from `fs::write`, and
    /// every other test here passes for both: a truncating write also creates
    /// the file, also leaves no temporary behind, also does not touch
    /// unrelated files. A mutation replacing the whole body with `fs::write`
    /// survived all four — the module lost its reason to exist and the suite
    /// stayed green.
    ///
    /// `fs::write` empties the file and then fills it, so anything holding it
    /// open across that window sees nothing, or half. That window is what took
    /// somebody's own notes out of their `CLAUDE.md`. Replacing by rename has
    /// no such window: the reader keeps the file it opened, whole.
    ///
    /// The two platforms get there differently and the assertion holds for
    /// both. On Unix the rename succeeds and the old file lives on under the
    /// open handle. On Windows it is refused while the handle is there — Rust
    /// opens without `FILE_SHARE_DELETE` — so the write fails and nothing
    /// changes at all. Either way the reader is never shown a hole, which is
    /// the promise; *which* of the two happens is the operating system's
    /// business.
    #[test]
    fn a_reader_holding_the_file_is_never_shown_a_hole() {
        use std::io::Read;

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("notes.md");
        replace(&path, b"notes somebody wrote by hand").unwrap();

        let mut reader = File::open(&path).unwrap();
        let _ = replace(&path, b"a much longer body written over the top of it");

        let mut seen = String::new();
        reader.read_to_string(&mut seen).unwrap();
        assert_eq!(
            seen, "notes somebody wrote by hand",
            "the reader was shown a file being emptied underneath it"
        );
    }

    #[test]
    fn two_writers_in_one_process_do_not_share_a_temporary_name() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("notes.md");

        let (first_path, _first) = create_temporary_file(&target).unwrap();
        let (second_path, _second) = create_temporary_file(&target).unwrap();

        assert_ne!(first_path, second_path);
    }
}
