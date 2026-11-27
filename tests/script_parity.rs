//! The five PATH helpers exist twice, once in each Windows script, because each
//! script is fetched and run on its own: `install.ps1` by `irm ... | iex`, the
//! uninstaller from beside the binary. A comment asking that the copies stay in
//! step is not a guard, and this is. A change to one copy that misses the other
//! fails here rather than in a user's PATH months later.

use std::collections::BTreeMap;
use std::path::Path;

/// The functions both scripts carry, by the name they give them.
const SHARED: &[&str] = &[
    "Read-UserPath",
    "Send-EnvironmentChange",
    "Write-UserPath",
    "Get-ComparablePath",
    "Test-SamePathEntry",
];

/// Each function's body, as its code lines with the indentation and the
/// comments taken out.
///
/// Comments are dropped because the two copies explain themselves to a reader
/// who has one of them open, and those sentences differ on purpose. What has to
/// match is the code.
fn functions(script: &str) -> BTreeMap<String, Vec<String>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join(script);
    let source = std::fs::read_to_string(&path).expect("read the script");

    let mut found = BTreeMap::new();
    let mut name: Option<String> = None;
    let mut body: Vec<String> = Vec::new();
    let mut depth = 0_i32;

    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(current) = &name {
            if !trimmed.starts_with('#') && !trimmed.is_empty() {
                body.push(trimmed.to_owned());
            }
            depth += trimmed.matches('{').count() as i32;
            depth -= trimmed.matches('}').count() as i32;
            if depth <= 0 {
                found.insert(current.clone(), std::mem::take(&mut body));
                name = None;
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("function ") {
            let declared = rest
                .split(['(', ' ', '{'])
                .next()
                .expect("a function has a name")
                .to_owned();
            name = Some(declared);
            body.clear();
            depth = 0;
        }
    }
    assert!(name.is_none(), "{script} ends inside a function");
    found
}

#[test]
fn the_two_windows_scripts_carry_the_same_path_helpers() {
    let install = functions("install.ps1");
    let uninstall = functions("uninstall.ps1");

    for shared in SHARED {
        let left = install
            .get(*shared)
            .unwrap_or_else(|| panic!("install.ps1 has no {shared}"));
        let right = uninstall
            .get(*shared)
            .unwrap_or_else(|| panic!("uninstall.ps1 has no {shared}"));
        assert_eq!(
            left, right,
            "{shared} is not the same code in both scripts; a fix to one has to be made in the other"
        );
    }
}
