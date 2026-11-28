//! The crate tarball holds every file the tests that ship in it read.
//!
//! `Cargo.toml` lists what the crate contains. A test that reads a file from the
//! repository root -- a workflow, a manifest, a figure -- fails in the extracted
//! tarball if the list drops it, and nothing in a normal run shows that: the
//! checkout has every file. Nine tests did exactly that when the list was first
//! written, found only by extracting a package and running the suite there.
//!
//! So this reads the test sources for string literals that name a file which
//! exists at the repository root, and requires each to be matched by the list.
//! It cannot see a path assembled at run time, and does not pretend to; it keeps
//! the next literal path from being added without the list being told.

use std::path::Path;

/// Files the tests name that the crate deliberately does not ship.
///
/// `npm/` is the wrapper that downloads a released binary and was excluded before
/// the allowlist existed. The three tests that read it fail in the tarball, which
/// was already true and is a separate matter from this list.
const KNOWN_ABSENT: &[&str] = &["npm/"];

/// Literals that name a file and are not a read of it. `repository_guards.rs`
/// spells `docker/Dockerfile` as the value a parser is expected to return for a
/// workflow it carries inline; the image's own file is not opened. It also names
/// `tools/memory-bench/comparison_corpus.py` as the git path it refuses a commit
/// for touching, which it compares against `git` output rather than opening.
const NOT_READS: &[&str] = &[
    "docker/Dockerfile",
    "tools/memory-bench/comparison_corpus.py",
    // `cli_integration.rs` makes a `.git` of its own in a temporary directory,
    // so a command run there sees a project. Nothing opens the checkout's — in a
    // worktree that one is a file, which is why the literal is seen at all.
    ".git",
    // The same fixture writes that `.git/config`; the path reaches this scanner
    // as one joined path, which the `.git` entry above does not excuse.
    ".git/config",
];

fn include_patterns(manifest: &str) -> Vec<String> {
    let start = manifest
        .find("include = [")
        .expect("Cargo.toml has an include list");
    let body = &manifest[start..];
    let body = &body[..body.find("\n]").expect("the include list ends")];
    body.lines()
        .map(str::trim)
        .filter(|line| line.starts_with('"'))
        .filter_map(|line| line.split('"').nth(1))
        .map(|pattern| pattern.trim_start_matches('/').to_owned())
        .collect()
}

/// The string literals of a Rust source file, with comments and character
/// literals skipped.
///
/// Splitting on `"` and taking every second piece is wrong the first time a file
/// holds a `'"'`, an escaped quote or a raw string: from there on it reads the
/// gaps between literals and misses the literals. This one keeps its place.
fn literals(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut found = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let rest = &source[at..];
        if rest.starts_with("//") {
            at += rest.find('\n').unwrap_or(rest.len());
        } else if rest.starts_with("/*") {
            at += rest.find("*/").map_or(rest.len(), |end| end + 2);
        } else if bytes[at] == b'r'
            && (at == 0 || !(bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_'))
            && rest[1..].trim_start_matches('#').starts_with('"')
        {
            let hashes = rest[1..].len() - rest[1..].trim_start_matches('#').len();
            let close = format!("\"{}", "#".repeat(hashes));
            let body = at + 2 + hashes;
            let end = source[body..]
                .find(&close)
                .map_or(source.len(), |end| body + end);
            found.push(source[body..end].to_owned());
            at = end + close.len();
        } else if bytes[at] == b'"' {
            let mut end = at + 1;
            while end < bytes.len() && bytes[end] != b'"' {
                end += if bytes[end] == b'\\' { 2 } else { 1 };
            }
            found.push(source[at + 1..end.min(source.len())].to_owned());
            at = end + 1;
        } else if bytes[at] == b'\'' {
            // A character literal, or a lifetime, which has no closing quote.
            at += if bytes.get(at + 1) == Some(&b'\\') {
                rest.find("'")
                    .and_then(|_| rest[2..].find('\''))
                    .map_or(1, |end| end + 3)
            } else if bytes.get(at + 2) == Some(&b'\'') {
                3
            } else {
                1
            };
        } else {
            at += 1;
        }
    }
    found
}

/// Paths written as a chain of segments: `.join("assets").join("tokens.svg")` is
/// `assets/tokens.svg`, and no literal in it says so.
fn joined_paths(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = source;
    while let Some(start) = rest.find(".join(\"") {
        let mut segments = Vec::new();
        let mut cursor = &rest[start..];
        while let Some(after) = cursor.strip_prefix(".join(\"") {
            let Some(end) = after.find("\")") else { break };
            segments.push(&after[..end]);
            cursor = &after[end + 2..];
        }
        if segments.len() > 1 {
            found.push(segments.join("/"));
        }
        // Past the chain just read, so its tail is not read again as a chain.
        rest = if segments.is_empty() {
            &rest[start + 1..]
        } else {
            cursor
        };
    }
    found
}

/// `dir/**` is everything below `dir`, `dir/*.ext` is one level, anything else is
/// exact. These are the only shapes the list uses, and a new one fails loudly.
fn matches(pattern: &str, path: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix("/**") {
        return path.starts_with(&format!("{prefix}/"));
    }
    if let Some((directory, suffix)) = pattern.split_once("/*") {
        return path
            .strip_prefix(&format!("{directory}/"))
            .is_some_and(|rest| !rest.contains('/') && rest.ends_with(suffix));
    }
    assert!(
        !pattern.contains('*'),
        "a glob shape this test does not know: {pattern}"
    );
    path == pattern
}

#[test]
fn the_crate_ships_every_file_its_tests_read() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let patterns = include_patterns(&std::fs::read_to_string(root.join("Cargo.toml")).unwrap());
    assert!(patterns.len() > 5, "{patterns:?}");

    let mut missing = Vec::new();
    for entry in std::fs::read_dir(root.join("tests")).unwrap().flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        let named = literals(&source).into_iter().chain(joined_paths(&source));
        for literal in named.collect::<Vec<_>>().iter().map(String::as_str) {
            let plausible = literal.len() > 3
                && !literal.contains([' ', '{', '}', '\\', '%'])
                && (literal.contains('/') || literal.contains('.'));
            if !plausible || !root.join(literal).is_file() {
                continue;
            }
            let shipped = patterns.iter().any(|pattern| matches(pattern, literal));
            let excused = NOT_READS.contains(&literal)
                || KNOWN_ABSENT
                    .iter()
                    .any(|prefix| literal.starts_with(prefix));
            if !shipped && !excused {
                missing.push(format!(
                    "{} reads {literal}",
                    path.file_name().unwrap().to_string_lossy()
                ));
            }
        }
    }
    missing.sort();
    missing.dedup();
    assert!(
        missing.is_empty(),
        "these files are read by tests the crate ships and are not in its `include` list: {missing:#?}"
    );
}

#[test]
fn a_path_built_from_joins_is_read_as_the_path() {
    let source =
        r#"root.join(".github").join("workflows").join("ci.yml") root.join("a").join("b")"#;
    assert_eq!(joined_paths(source), [".github/workflows/ci.yml", "a/b"]);
}

#[test]
fn the_scanner_keeps_its_place_across_quotes_that_are_not_literals() {
    let source = r##"
        // "not.this" is a comment
        let a = '"'; let b = "esc\"aped.json"; let c = r#"raw "quoted".md"#; let d = "last.toml";
        fn f<'a>(x: &'a str) -> &'a str { x } let e = "after.svg";
    "##;
    assert_eq!(
        literals(source),
        [
            "esc\\\"aped.json",
            "raw \"quoted\".md",
            "last.toml",
            "after.svg"
        ]
    );
}
