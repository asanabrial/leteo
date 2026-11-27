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
        if trimmed.starts_with('#') {
            // A comment is not compared, and its braces are not a block's: only
            // the code decides where a function ends.
            continue;
        }
        if let Some(current) = &name {
            if !trimmed.is_empty() {
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
            // The declaration is compared too, and its brace counts: starting
            // from zero ended the capture at the first nested block's `}`, which
            // left the fallbacks and the final comparison unchecked.
            body.clear();
            body.push(trimmed.to_owned());
            depth = trimmed.matches('{').count() as i32 - trimmed.matches('}').count() as i32;
        }
    }
    assert!(name.is_none(), "{script} ends inside a function");
    found
}

/// The top-level block that decides whether the registry is used at all.
///
/// It is not a function, so `functions` does not reach it, and it is the block
/// that picks between the registry and the API that expands every `%VAR%` — the
/// one place a one-sided change brings the defect back in one script only.
fn registry_open(script: &str) -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join(script);
    let source = std::fs::read_to_string(&path).expect("read the script");

    let mut block = Vec::new();
    let mut started = false;
    let mut warned = false;
    for line in source.lines() {
        let trimmed = line.trim();
        if !started {
            started = trimmed.starts_with("$onWindows =");
            if !started {
                continue;
            }
        }
        if trimmed.starts_with('#') {
            continue;
        }
        if !trimmed.is_empty() {
            warned = warned || trimmed.starts_with("Write-Warning");
            block.push(trimmed.to_owned());
        }
        if warned && trimmed == "}" {
            return block;
        }
    }
    panic!("{script} has no registry-open block ending after its warning");
}

#[test]
fn both_scripts_choose_the_registry_the_same_way() {
    assert_eq!(
        registry_open("install.ps1"),
        registry_open("uninstall.ps1"),
        "the block that decides whether the registry is used is not the same in both scripts"
    );
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
