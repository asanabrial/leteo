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
/// A symbolic link is written through, so the link stays a link, and the file
/// keeps the permissions it had.
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
    let resolved = resolve_links(path)?;
    let path = resolved.as_path();
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path has no parent directory: {}", path.display()),
        )
    })?;
    fs::create_dir_all(parent)?;

    let previous = match fs::metadata(path) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };

    let (temporary_path, mut temporary_file) = create_temporary_file(path)?;
    let written = restrict(&temporary_file, visibility, previous.as_ref())
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

/// The file a write has to land in when `path` is a symbolic link.
///
/// A rename onto a link replaces the link, so a `CLAUDE.md` that points into a
/// dotfiles repository was detached from it by the first `leteo setup`, and the
/// repository never saw the change. The temporary file has to go beside the
/// file that is actually written, not beside the link: a rename only stays
/// atomic within one filesystem, and the link and its target need not share
/// one.
///
/// A link that cannot be followed is refused rather than written through or
/// replaced. A dangling one names a file that is not there to keep, and
/// replacing it would throw away where the person meant it to go; a loop and a
/// link to a directory have no file at their end at all.
fn resolve_links(path: &Path) -> io::Result<PathBuf> {
    let is_link =
        fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink());
    if !is_link {
        return Ok(path.to_owned());
    }

    let resolved = fs::canonicalize(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "{} is a symbolic link that cannot be followed to a file ({error}); \
                 it was left as it was",
                path.display()
            ),
        )
    })?;
    if resolved.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{} is a symbolic link to a directory ({}); it was left as it was",
                path.display(),
                resolved.display()
            ),
        ));
    }
    Ok(resolved)
}

/// An owner-only write is 0600 whatever was there; any other write keeps the
/// mode of the file it replaces. The new file would otherwise take the umask
/// default, which made a settings file its owner had restricted to 0600
/// readable by the group and by everyone else.
#[cfg(unix)]
fn restrict(
    file: &File,
    visibility: Visibility,
    previous: Option<&fs::Metadata>,
) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    match (visibility, previous) {
        (Visibility::OwnerOnly, _) => file.set_permissions(fs::Permissions::from_mode(0o600)),
        // The file-type bits in `st_mode` are not permissions and are not
        // meant to be handed back to `fchmod`.
        (Visibility::Inherited, Some(previous)) => file.set_permissions(
            fs::Permissions::from_mode(previous.permissions().mode() & 0o7777),
        ),
        (Visibility::Inherited, None) => Ok(()),
    }
}

#[cfg(not(unix))]
fn restrict(
    _file: &File,
    _visibility: Visibility,
    _previous: Option<&fs::Metadata>,
) -> io::Result<()> {
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

    #[cfg(unix)]
    #[test]
    fn a_write_through_a_symbolic_link_lands_in_the_file_it_names() {
        let temp = tempfile::tempdir().unwrap();
        let dotfiles = temp.path().join("dotfiles");
        fs::create_dir(&dotfiles).unwrap();
        let real = dotfiles.join("CLAUDE.md");
        fs::write(&real, "mine").unwrap();
        let link = temp.path().join("CLAUDE.md");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        replace(&link, b"mine and Leteo's").unwrap();

        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the link was replaced by a plain file"
        );
        assert_eq!(fs::read_to_string(&real).unwrap(), "mine and Leteo's");
        let beside_link: Vec<_> = fs::read_dir(temp.path()).unwrap().flatten().collect();
        assert_eq!(
            beside_link.len(),
            2,
            "left beside the link: {beside_link:?}"
        );
        let beside_file: Vec<_> = fs::read_dir(&dotfiles).unwrap().flatten().collect();
        assert_eq!(
            beside_file.len(),
            1,
            "left beside the file: {beside_file:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_chain_of_links_is_followed_to_the_file_at_its_end() {
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real.md");
        fs::write(&real, "old").unwrap();
        let middle = temp.path().join("middle.md");
        std::os::unix::fs::symlink(&real, &middle).unwrap();
        let outer = temp.path().join("outer.md");
        std::os::unix::fs::symlink(&middle, &outer).unwrap();

        replace(&outer, b"new").unwrap();

        assert_eq!(fs::read_to_string(&real).unwrap(), "new");
        assert!(
            fs::symlink_metadata(&outer)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(
            fs::symlink_metadata(&middle)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_file_keeps_the_mode_its_owner_gave_it() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        fs::write(&path, "{}").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        replace(&path, b"{\"a\":1}").unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o600, "the new file took the umask default");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_keeps_the_mode_of_the_file_it_names() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real.md");
        fs::write(&real, "old").unwrap();
        fs::set_permissions(&real, fs::Permissions::from_mode(0o640)).unwrap();
        let link = temp.path().join("link.md");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        replace(&link, b"new").unwrap();

        let mode = fs::metadata(&real).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o640);
    }

    #[cfg(unix)]
    #[test]
    fn a_private_write_stays_owner_only_whatever_the_file_was() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cloud.json");
        fs::write(&path, "{}").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        replace_private(&path, b"{\"token\":1}").unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_nothing_is_refused_and_left_as_it_was() {
        let temp = tempfile::tempdir().unwrap();
        let link = temp.path().join("CLAUDE.md");
        let missing = temp.path().join("not-there.md");
        std::os::unix::fs::symlink(&missing, &link).unwrap();

        let error = replace(&link, b"data").unwrap_err();

        assert!(error.to_string().contains("cannot be followed"), "{error}");
        assert_eq!(fs::read_link(&link).unwrap(), missing);
        assert!(!missing.exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_loop_of_links_is_refused_and_left_as_it_was() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("a.md");
        let second = temp.path().join("b.md");
        std::os::unix::fs::symlink(&second, &first).unwrap();
        std::os::unix::fs::symlink(&first, &second).unwrap();

        let error = replace(&first, b"data").unwrap_err();

        assert!(error.to_string().contains("cannot be followed"), "{error}");
        assert_eq!(fs::read_link(&first).unwrap(), second);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_a_directory_is_refused_and_left_as_it_was() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("dir");
        fs::create_dir(&directory).unwrap();
        let link = temp.path().join("CLAUDE.md");
        std::os::unix::fs::symlink(&directory, &link).unwrap();

        let error = replace(&link, b"data").unwrap_err();

        assert!(
            error.to_string().contains("symbolic link to a directory"),
            "{error}"
        );
        assert_eq!(fs::read_link(&link).unwrap(), directory);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
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
