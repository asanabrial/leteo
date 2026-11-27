//! The Windows PATH code exists twice, once in each script, because each script
//! is fetched and run on its own: `install.ps1` by `irm ... | iex`, the
//! uninstaller from beside the binary. A comment asking that the copies stay in
//! step is not a guard, and this is. A change to one copy that misses the other
//! fails here rather than in a user's PATH months later.

use std::path::Path;

/// The functions both scripts carry, by the name they give them.
const SHARED: &[&str] = &[
    "Read-UserPath",
    "Send-EnvironmentChange",
    "Write-UserPath",
    "Get-ComparablePath",
    "Test-SamePathEntry",
];

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The shared region: the block that chooses the registry, then the five
/// helpers, from the line that decides `$onWindows` through the closing brace
/// of `Test-SamePathEntry`.
///
/// A region rather than one capture per function, and bounded by two things both
/// scripts have: the line that decides `$onWindows`, and the brace that closes
/// `Test-SamePathEntry` at the indentation the region started at. No brace
/// count decides where a function ends — a count that comes out wrong compares a
/// prefix of both copies and passes. If the closing brace is not found, this
/// panics instead of comparing less.
///
/// Comments are dropped because the two copies explain themselves to a reader
/// who has one of them open, and those sentences differ on purpose. What has to
/// match is the code, the here-string containing the P/Invoke declaration
/// included.
fn shared_region(script: &str) -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join(script);
    let source = std::fs::read_to_string(&path).expect("read the script");

    let mut region = Vec::new();
    let mut start_indent = None;
    let mut last_helper = false;

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        let start = match start_indent {
            Some(start) => start,
            None => {
                if !trimmed.starts_with("$onWindows = (") {
                    continue;
                }
                start_indent = Some(indent(line));
                indent(line)
            }
        };
        region.push(trimmed.to_owned());
        last_helper = last_helper || trimmed.starts_with("function Test-SamePathEntry");
        if last_helper && trimmed == "}" && indent(line) == start {
            return region;
        }
    }
    panic!("{script} has no shared PATH region ending at Test-SamePathEntry's brace");
}

#[test]
fn the_two_windows_scripts_carry_the_same_path_code() {
    let install = shared_region("install.ps1");
    let uninstall = shared_region("uninstall.ps1");

    for shared in SHARED {
        let declaration = format!("function {shared}");
        assert!(
            install.iter().any(|line| line.starts_with(&declaration)),
            "install.ps1 no longer declares {shared}"
        );
        assert!(
            uninstall.iter().any(|line| line.starts_with(&declaration)),
            "uninstall.ps1 no longer declares {shared}"
        );
    }

    assert_eq!(
        install, uninstall,
        "the shared PATH code is not the same in both scripts; a fix to one has to be made in the other"
    );
}
