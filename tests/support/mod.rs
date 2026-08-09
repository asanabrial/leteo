//! Helpers shared by the guards in this directory.
//!
//! An integration test is a crate of its own, so anything two of them need has
//! to live in a module both declare. `tests/support/mod.rs` and not
//! `tests/support.rs`: Cargo builds every `.rs` directly under `tests/` as a
//! test binary, and a file of helpers with no tests in it would be one that
//! passes without ever checking anything.

use std::path::Path;

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
