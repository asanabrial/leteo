//! Helpers shared by the guards in this directory.
//!
//! An integration test is a crate of its own, so anything two of them need has
//! to live in a module both declare. `tests/support/mod.rs` and not
//! `tests/support.rs`: Cargo builds every `.rs` directly under `tests/` as a
//! test binary, and a file of helpers with no tests in it would be one that
//! passes without ever checking anything.

use std::path::Path;

/// Spells a small number the way this repository's prose does.
///
/// Guards that compare a computed count against a sentence spelling it out
/// need this, and the table lived separately in two of them with different
/// ranges. Neither covered the small numbers, so the first caller to hand one
/// over got a panic naming no file and no fix.
///
/// `dead_code` is allowed because each file under `tests/` is its own binary
/// with its own copy of this module: only `documented_commands.rs` calls this
/// now, which makes it unused in every other one.
#[allow(dead_code)]
pub fn spelled(count: usize) -> &'static str {
    match count {
        1 => "one",
        2 => "two",
        3 => "three",
        4 => "four",
        5 => "five",
        6 => "six",
        7 => "seven",
        8 => "eight",
        9 => "nine",
        10 => "ten",
        11 => "eleven",
        12 => "twelve",
        13 => "thirteen",
        14 => "fourteen",
        15 => "fifteen",
        other => panic!(
            "`support::spelled` has no word for {other}; add it there rather than \
             rounding the sentence that needs it"
        ),
    }
}

/// Every `.rs` file under a directory, as (path, text).
pub fn source_under(directory: &str) -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(directory);
    let mut found = Vec::new();
    let mut stack = vec![root];
    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(&directory)
            .expect("read source")
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|kind| kind == "rs") {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                found.push((path.display().to_string(), text));
            }
        }
    }
    assert!(found.len() > 15, "only found {} source files", found.len());
    found
}
