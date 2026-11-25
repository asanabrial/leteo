//! The fallback in both uninstall scripts names what the binary owns, and a
//! shell script cannot read `DATA_DIR_FILES`. This is what keeps their copies
//! from drifting: a name added to the list that the scripts never learn about
//! would survive an uninstall, and a glob put back would take a migration
//! backup with it — the only way back from a one-way migration.
//!
//! It reads code lines only; a comment is skipped, as it is everywhere here.

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
