//! The fallback in both uninstall scripts names what the binary owns, and a
//! shell script cannot read `DATA_DIR_FILES`. This is what keeps their copies
//! from drifting: a name added to the list that the scripts never learn about
//! would survive an uninstall, and a glob put back would take a migration
//! backup with it — the only way back from a one-way migration.
//!
//! It reads code lines only; a comment is skipped, as it is everywhere here.
//!
//! Reading the text is not enough on its own: `leteo.db.*`, `leteo.db?*` or
//! `rm -f "$DATA_DIR"/*` match the migration backups and leave every literal
//! below intact. So the shell fallback is also run, against a directory holding
//! what it must keep beside what it must take, and what is left is what is
//! asserted.

use std::collections::BTreeSet;
use std::path::Path;

fn script_lines(script: &str) -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join(script);
    std::fs::read_to_string(&path)
        .expect("read the script")
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .map(str::to_owned)
        .collect()
}

/// The names a script removes, read from the one line that assigns them.
fn named_files(script: &str, assignment: &str) -> BTreeSet<String> {
    let lines = script_lines(script);
    let matching: Vec<&String> = lines
        .iter()
        .filter(|line| line.contains(assignment))
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "{script} has to name the files on exactly one line: {matching:?}"
    );
    matching[0]
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-'))
        .filter(|word| word.contains('.'))
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_uninstallers_remove_exactly_the_files_the_binary_owns() {
    let expected: BTreeSet<String> = leteo::setup::DATA_DIR_FILES
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    for (script, assignment) in [
        ("uninstall.sh", "DATA_DIR_FILES="),
        ("uninstall.ps1", "$dataDirFiles ="),
    ] {
        assert_eq!(named_files(script, assignment), expected, "{script}");

        let code = script_lines(script).join("\n");
        for prefix in leteo::setup::DATA_DIR_PREFIXES {
            assert!(
                code.contains(&format!("{prefix}*")),
                "{script} does not remove what starts with {prefix}"
            );
        }
        for directory in leteo::setup::DATA_DIR_SUBDIRECTORIES {
            assert!(
                code.contains(&format!("/{directory}")) || code.contains(&format!("'{directory}'")),
                "{script} does not remove the {directory} directory"
            );
        }
    }
}

#[test]
fn neither_uninstaller_reaches_for_a_glob_on_the_store() {
    for script in ["uninstall.sh", "uninstall.ps1"] {
        for line in script_lines(script) {
            assert!(
                !line.contains("leteo.db*"),
                "{script} globs the store: {line}"
            );
            assert!(
                !line.contains("store.db*"),
                "{script} globs the store: {line}"
            );
        }
    }
}

/// The shell fallback, run rather than read.
///
/// Unix only: this is the script that runs there, and `cfg(unix)` keeps the
/// Windows runners out of a test they cannot run. With no `leteo` in the
/// install directory the script takes its own fallback path, which is the block
/// this file is about; the binary would otherwise be asked to uninstall, and
/// its judgment — not the script's — would decide what goes.
#[cfg(unix)]
mod fallback {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// Files a data directory can hold that Leteo did not write: two migration
    /// copies a glob would take, and a person's note, which is what keeps the
    /// data directory itself.
    const STRANGERS: &[&str] = &[
        "leteo.db.bak-before-migrate",
        "leteo.db.pre-schema-7",
        "notes.txt",
    ];

    /// The two shapes the prefix list claims: `leteo backup` writes the file,
    /// and a directory of that name is Leteo's too, because `DATA_DIR_PREFIXES`
    /// in `src/setup/removal/mod.rs` names the prefix and
    /// `a_data_directory_of_only_leteos_own_files_goes_entirely` holds the
    /// binary to it. The issue that asked for this test listed a `backup-2026/`
    /// among the survivors, which is the opposite of that rule; the fixture
    /// holds both shapes and the assertion below is the rule's answer.
    const OWNED_BACKUPS: &[&str] = &["backup-20260802.db", "backup-2026/keep.txt"];

    fn write(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("a parent directory");
        }
        std::fs::write(path, b"a file").expect("write a file");
    }

    #[test]
    fn the_shell_fallback_removes_what_the_binary_owns_and_nothing_else() {
        let root = tempfile::tempdir().expect("a temporary root");
        let install: PathBuf = root.path().join("bin");
        let data: PathBuf = root.path().join("data");
        std::fs::create_dir_all(&install).expect("the install directory");
        write(&install.join("uninstall.sh"));
        // The script's probe is `[ -x "$INSTALL_DIR/leteo" ]`, and with it false
        // the only thing left that deletes anything is the fallback below: no
        // other line of the script removes a file from the data directory.
        assert!(!install.join("leteo").exists());

        for owned in leteo::setup::DATA_DIR_FILES {
            write(&data.join(owned));
        }
        write(&data.join("hooks").join("hook.sh"));
        for backup in OWNED_BACKUPS {
            write(&data.join(backup));
        }
        for stranger in STRANGERS {
            write(&data.join(stranger));
        }

        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("scripts")
            .join("uninstall.sh");
        let output = Command::new("sh")
            .arg(&script)
            .arg("-y")
            .env_clear()
            // `say_kept` reads the directory with `ls`, and `rm` is not a
            // builtin, so the shell needs the PATH it was given.
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", root.path())
            .env("LETEO_INSTALL_DIR", &install)
            .env("LETEO_DATA_DIR", &data)
            .output()
            .expect("run the shell fallback");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "the fallback exited with {}:\n{stdout}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );

        for owned in leteo::setup::DATA_DIR_FILES {
            assert!(
                !data.join(owned).exists(),
                "{owned} survived the fallback:\n{stdout}"
            );
        }
        assert!(
            !data.join("hooks").exists(),
            "the hooks directory survived the fallback:\n{stdout}"
        );
        for backup in OWNED_BACKUPS {
            assert!(
                !data.join(backup).exists(),
                "{backup} is named by the prefix Leteo owns and survived the fallback:\n{stdout}"
            );
        }
        assert!(
            !install.join("uninstall.sh").exists(),
            "the installer's own script survived the fallback:\n{stdout}"
        );
        for stranger in STRANGERS {
            assert!(
                data.join(stranger).exists(),
                "{stranger} was taken by the fallback:\n{stdout}"
            );
        }
        assert!(
            data.is_dir(),
            "the data directory went with the files it still holds:\n{stdout}"
        );
        assert!(
            install.is_dir(),
            "the install directory went, and it is shared with every other tool:\n{stdout}"
        );
    }
}
