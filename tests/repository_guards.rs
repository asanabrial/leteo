//! Guards over the repository itself, rather than over what it documents.
//!
//! Most of these watch something the compiler cannot see: a source file no
//! `mod` declares and which therefore never compiles, a test that would open
//! the store somebody keeps their real memories in, a schema column nothing
//! writes, a sentence that reaches a person carrying the indentation of the
//! Rust source it was formatted in, and an npm package that would hand somebody
//! a different version of Leteo than the one it says it is.
//!
//! None of them reads a comment. Two once did — one holding the sentences that
//! say how many tests need a database against the attributes, one holding the
//! coverage note in `ci.yml` against the two workflows — and both were removed
//! when this repository settled that comments are not something its tests check,
//! at any level. Prose is still read where it is published rather than
//! commented: the README's description of `assets/tokens.svg`, and the strings
//! this binary hands to whoever runs it.

use std::path::Path;

mod support;

use support::source_under;

#[test]
fn every_source_file_is_reachable_from_a_module_declaration() {
    // A `.rs` file no `mod` names is not compiled, and Rust says nothing about
    // it: no warning, no error, and every test inside it silently never runs.
    // A file of tests added without its `mod` line looks exactly like a file of
    // tests that pass.
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    let mut stack = vec![source.clone()];
    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(&directory).expect("read src").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|kind| kind == "rs") {
                files.push(path);
            }
        }
    }
    assert!(
        files.len() > 50,
        "only found {} files under src/",
        files.len()
    );

    let mut orphans = Vec::new();
    for path in &files {
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        // `lib.rs` and `main.rs` are roots; `mod.rs` is named by its directory.
        if matches!(stem.as_ref(), "lib" | "main" | "mod") {
            continue;
        }
        // The parent module and nowhere else. Searching the whole tree for
        // `mod <stem>;` looked simpler and was wrong: `store/prompts.rs` and
        // `store/tests/prompts.rs` share a stem, so one declaration covered
        // both and the orphan hid behind its namesake. This test found that in
        // itself, by being broken on purpose and failing to fail.
        let directory = path.parent().unwrap_or(&source);
        let parent = if directory == source {
            source.join("lib.rs")
        } else {
            directory.join("mod.rs")
        };
        let declarations = std::fs::read_to_string(&parent).unwrap_or_default();
        let declared = declarations.contains(&format!("mod {stem};"));
        if !declared {
            orphans.push(path.strip_prefix(&source).unwrap_or(path).to_owned());
        }
    }

    assert!(
        orphans.is_empty(),
        "no `mod` in the sibling mod.rs names these. Either nothing in them is \
         compiled, or they use the `foo.rs` + `foo/bar.rs` layout, which every \
         other module here avoids: put them in `foo/mod.rs` + `foo/bar.rs` like \
         the rest. {orphans:?}"
    );
}

/// No test can reach the store somebody actually keeps their memories in.
///
/// Every test builds its own database under a temporary directory and throws it
/// away. Nothing enforced that, and the failure it guards against is silent and
/// unrecoverable: a test that opened the default data directory would run its
/// fixtures, its deletions and its migrations against a real store, and the
/// first anybody would know is memories missing.
///
/// Two ways in, so two things are checked. `in_data_dir` builds a store from a
/// directory rather than a file, and the default that feeds it is the home
/// directory; and a `Cli` built without `--database` or `--data-dir` falls back
/// to the same place. Neither belongs in a test.
///
/// Source-level, in the spirit of the checks beside it: it is cheap, it runs
/// every time, and it is what notices the next person reaching for a shortcut.
#[test]
fn no_test_opens_the_real_store() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    let mut stack = vec![root.join("src"), root.join("tests")];
    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(&directory)
            .expect("read a source tree")
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|kind| kind == "rs") {
                files.push(path);
            }
        }
    }

    let mut offenders = Vec::new();
    let mut checked = 0usize;
    for path in files {
        let text = std::fs::read_to_string(&path).expect("read a source file");
        let shown = path.to_string_lossy().replace('\\', "/");
        // Not this file: the words it looks for are written in it, so it would
        // find itself. Nothing else in here opens a store.
        if shown.ends_with("repository_guards.rs") {
            continue;
        }
        let is_test_file =
            shown.contains("/tests/") || path.file_name().is_some_and(|name| name == "tests.rs");
        if !is_test_file && !text.contains("#[cfg(test)]") {
            continue;
        }
        checked += 1;
        // Only the test halves of a mixed file: production code is allowed to
        // resolve the real data directory, because that is its job.
        let tested = match text.split_once("#[cfg(test)]") {
            Some((_, rest)) if !is_test_file => rest.to_owned(),
            _ => text,
        };
        let lines: Vec<&str> = tested
            .lines()
            .map(|line| line.split("//").next().unwrap_or(""))
            .collect();
        for (number, code) in lines.iter().enumerate() {
            // The default data directory, by either of the two names it has.
            let default_directory = code.contains("in_data_dir")
                || code.contains("home_dir()") && code.contains("join");
            // And a database named by an absolute path that something opens. A
            // test that opens one is reading a file that is not its own,
            // whoever's it happens to be — the store this machine runs on, a
            // copy of it taken for a measurement, a colleague's. Every one of
            // them exists before the test and outlives it, and the whole point
            // of a temporary directory is that neither is true.
            //
            // Two conditions, because either alone is wrong. `"C:/repo"` is a
            // session's directory, stored as text and never opened, and there
            // are dozens of those; `/home/someone/.engram/engram.db` is a
            // fixture describing an installation the test never touches. So the
            // path has to name a database *and* an opener has to be within
            // reach of it — the two are rarely on the same line, because the
            // path is usually bound to a variable first.
            let names_a_database = code.contains(".db")
                && (code.contains(":/") || code.contains(":\\") || code.contains("\"/"));
            let opened_nearby = names_a_database
                && lines[number.saturating_sub(3)..(number + 4).min(lines.len())]
                    .iter()
                    .any(|near| {
                        near.contains("Store::open")
                            || near.contains("StoreConfig::new")
                            || near.contains("Connection::open")
                            || near.contains("--database")
                            || near.contains("--data-dir")
                    });
            if default_directory || opened_nearby {
                offenders.push(format!("{}:{}", path.display(), number + 1));
            }
        }
    }
    assert!(
        checked > 20,
        "the sweep found almost no test code: {checked}"
    );
    assert!(
        offenders.is_empty(),
        "these tests reach for the default data directory, which is where somebody's \
         real memories live; build a store under a temporary directory instead: {offenders:?}"
    );
}

/// Every column is written by something, or is named in `WRITTEN_BY_NOTHING`.
///
/// Found by counting non-null values on a real store and then looking for a
/// writer: some columns across two tables have neither. The embedding trio is
/// argued for in the baseline migration itself, and the rest said nothing at
/// all, so a reader of the schema would reasonably have assumed a memory can
/// expire and a relation can be superseded by a later one. Neither happens.
///
/// The list is the point rather than an exemption: a dead column missing from
/// it fails here, and so does one of the named ones coming to life, because
/// then the sentence in `store-and-schema.md` §10 has stopped being true and
/// somebody has to say what it does now. How many there are belongs to the
/// list.
#[test]
fn every_column_has_a_writer_or_is_named_as_having_none() {
    // Written down in the spec, and here, in the same words.
    const WRITTEN_BY_NOTHING: &[(&str, &str)] = &[
        ("observations", "embedding"),
        ("observations", "embedding_model"),
        ("observations", "embedding_created_at"),
        ("observations", "expires_at"),
        ("memory_relations", "superseded_at"),
        ("memory_relations", "superseded_by_relation_id"),
    ];

    // `src/store` and not `src`: every statement this binary runs is built
    // there, and the same word means something else in two other modules —
    // `expires_at` is a claim on a JWT in `cloud/auth.rs` and a recovery
    // token's clock in `mcp/mod.rs`, neither of which is this column.
    let source = source_under("src/store");
    // The schema's own column lists name every column by definition, so they
    // are not evidence that anything writes one.
    let elsewhere: String = source
        .iter()
        .filter(|(path, _)| {
            // Not the schema's own column lists, which name every column by
            // definition, and not the tests beside it, which assert the schema
            // is the shape the schema says it is.
            !path.ends_with("schema.rs") && !path.contains("tests")
        })
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    // Read from the baseline rather than from an open store, so this needs no
    // new public function on `Store` to exist only for a test. No migration
    // adds a column — checked below — so the baseline is the whole list.
    let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut later = String::new();
    for entry in std::fs::read_dir(&migrations)
        .expect("read migrations")
        .flatten()
    {
        later.push_str(&std::fs::read_to_string(entry.path()).unwrap_or_default());
    }
    assert!(
        !later.to_uppercase().contains("ADD COLUMN"),
        "a migration adds a column, so the baseline is no longer the whole list"
    );
    let baseline = std::fs::read_to_string(migrations.join("0001_baseline_tables.sql"))
        .expect("read baseline");

    let mut unwritten = Vec::new();
    for table in ["observations", "memory_relations", "sessions", "prompts"] {
        let columns = columns_of(&baseline, table);
        assert!(columns.len() > 5, "{table} has {} columns", columns.len());
        for column in columns {
            if !elsewhere.contains(&column) {
                unwritten.push((table, column));
            }
        }
    }

    let named: Vec<(&str, String)> = WRITTEN_BY_NOTHING
        .iter()
        .map(|(table, column)| (*table, (*column).to_owned()))
        .collect();
    let unexpected: Vec<&(&str, String)> = unwritten
        .iter()
        .filter(|entry| !named.contains(entry))
        .collect();
    assert!(
        unexpected.is_empty(),
        "these columns are written by nothing and `store-and-schema.md` §10 does \
         not say so: {unexpected:?}"
    );
    let revived: Vec<&(&str, String)> = named
        .iter()
        .filter(|entry| !unwritten.contains(entry))
        .collect();
    assert!(
        revived.is_empty(),
        "these are named as written by nothing and something now writes them, so \
         `store-and-schema.md` §10 has to say what they do: {revived:?}"
    );
}

/// The column names of one `CREATE TABLE` in a migration.
fn columns_of(sql: &str, table: &str) -> Vec<String> {
    let start = sql
        .find(&format!("CREATE TABLE IF NOT EXISTS {table} ("))
        .unwrap_or_else(|| panic!("{table} is created in the baseline"));
    let body = &sql[start..];
    let end = body
        .find(
            "
);",
        )
        .unwrap_or(body.len());
    body[..end]
        .lines()
        .skip(1)
        .filter_map(|line| {
            let line = line.trim();
            let word = line.split_whitespace().next()?;
            // Table-level clauses rather than columns.
            let keyword = word.to_uppercase();
            if line.starts_with("--")
                || ["FOREIGN", "PRIMARY", "UNIQUE", "CHECK", "CONSTRAINT"]
                    .contains(&keyword.as_str())
            {
                return None;
            }
            Some(word.trim_end_matches(',').to_owned())
        })
        .collect()
}

/// Every string literal in a file, roughly: what is between unescaped quotes.
///
/// Roughly is enough for what reads this. It skips line comments so a `"` in
/// prose does not open a literal that swallows the rest of the file, and it
/// understands `\"` and `'"'`. Raw strings are skipped whole — this tree uses
/// them for JSON fixtures and regexes rather than for sentences — but they have
/// to be *recognised*, because a scanner that carries its state across lines and
/// meets a `"#` counts it as an opening quote and reads every file after it
/// inside out.
///
/// A literal spans lines, because the two ways of breaking one across two
/// source lines are exactly what this is looking for: with a trailing backslash
/// the break and the indentation after it are eaten, and without one they both
/// stay in the string. A line-at-a-time reader sees the first half end in
/// trailing spaces and the second half begin outside any literal, so it finds
/// nothing — which is what it found when the second kind was put back on the
/// Polish line to check that it would be caught.
fn string_literals(text: &str) -> Vec<String> {
    let characters: Vec<char> = text.chars().collect();
    let mut found = Vec::new();
    let mut current = String::new();
    let mut inside = false;
    let mut at = 0;
    while at < characters.len() {
        let character = characters[at];
        if inside {
            match character {
                '\\' => {
                    // The continuation this whole check is about: a backslash
                    // at the end of a source line eats the break *and* every
                    // space after it. A reader that skips only the break sees
                    // the indentation as content and reports the whole tree as
                    // broken — which is what this one did, in eighty-four
                    // places, all of them written correctly.
                    //
                    // The break is `\r\n` in a checked-out tree on Windows,
                    // where the files are CRLF and the index is LF. A reader
                    // that only knew `\n` found four sound literals guilty and
                    // would have found any number more.
                    let mut next = at + 1;
                    if characters.get(next) == Some(&'\r') {
                        next += 1;
                    }
                    if characters.get(next) == Some(&'\n') {
                        at = next + 1;
                        while characters.get(at).is_some_and(|c| *c == ' ') {
                            at += 1;
                        }
                        continue;
                    }
                    at += 1;
                }
                '"' => {
                    found.push(std::mem::take(&mut current));
                    inside = false;
                }
                _ => current.push(character),
            }
            at += 1;
            continue;
        }
        match character {
            // A line comment, which may hold a lone quote in prose.
            '/' if characters.get(at + 1) == Some(&'/') => {
                while at < characters.len() && characters[at] != '\n' {
                    at += 1;
                }
            }
            // `r"`, `r#"`, `br##"` — stepped over whole, closing hashes and all.
            'r' => {
                let mut hashes = at + 1;
                while characters.get(hashes) == Some(&'#') {
                    hashes += 1;
                }
                if characters.get(hashes) == Some(&'"') {
                    let width = hashes - at - 1;
                    let mut end = hashes + 1;
                    while end < characters.len() {
                        if characters[end] == '"'
                            && characters[end + 1..]
                                .iter()
                                .take(width)
                                .filter(|&&c| c == '#')
                                .count()
                                == width
                        {
                            break;
                        }
                        end += 1;
                    }
                    at = end + 1 + width;
                } else {
                    at += 1;
                }
            }
            // The char literal `'"'`, which is a quote that opens nothing.
            '\'' if characters.get(at + 1) == Some(&'"') => at += 3,
            '"' => {
                inside = true;
                at += 1;
            }
            _ => at += 1,
        }
    }
    found
}

/// A run of four or more spaces between two words, which prose never has.
fn joined_without_the_backslash(segment: &str) -> bool {
    let bytes: Vec<char> = segment.chars().collect();
    let word = |c: char| c.is_alphanumeric() || matches!(c, ',' | ';' | '.' | ':' | '?' | '!');
    let mut run = 0;
    let mut before = false;
    for (at, &character) in bytes.iter().enumerate() {
        if character == ' ' {
            run += 1;
            if run == 1 {
                before = at > 0 && word(bytes[at - 1]);
            }
        } else {
            if run >= 4 && before && character.is_alphanumeric() {
                return true;
            }
            run = 0;
        }
    }
    false
}

/// The same break, made the other way: no backslash at all, so the line break
/// survives with the indentation behind it.
///
/// A sentence that ends mid-word and resumes four columns in is one sentence
/// wearing the shape of the source file. A paragraph break is not — those
/// resume at the left margin, which is where every deliberate multi-line
/// literal in this tree puts them.
fn broken_by_the_line_it_was_written_on(before: &str, after: &str) -> bool {
    // Not a semicolon: that is how a SQL statement ends, and the two-statement
    // rebuild in `engram.rs` is laid out on purpose.
    // Trailing spaces and all: a line that ends "…przez " is as much mid-sentence
    // as one that ends "…przez", and reading the last character without
    // trimming let exactly that mutation through.
    let ends_mid_sentence = before
        .trim_end()
        .chars()
        .last()
        .is_some_and(|c| c.is_alphanumeric() || matches!(c, ',' | ':'));
    let indented = after.len() - after.trim_start_matches(' ').len() >= 4;
    ends_mid_sentence && indented && after.trim_start().starts_with(char::is_alphanumeric)
}

/// A query, not a sentence.
///
/// Statements are laid out across lines with their clauses indented under each
/// other, which is the same shape as a sentence broken by the source it was
/// written on and is nothing like the same mistake — nobody reads these but
/// SQLite, and the layout is what makes them reviewable.
fn looks_like_sql(literal: &str) -> bool {
    [
        "SELECT ",
        "INSERT ",
        "UPDATE ",
        "DELETE ",
        "CREATE ",
        "WHERE ",
        "FROM ",
        "ORDER BY ",
        "IS NULL",
    ]
    .iter()
    .any(|keyword| literal.contains(keyword))
}

/// A document, not a sentence.
///
/// The fixtures that stand in for what a subagent wrote are markdown, with
/// headings and lists and fenced blocks, and they are laid out in the source the
/// way they would arrive. A line of one that begins four columns in is a line of
/// a document, not a sentence that lost its backslash.
fn looks_like_a_document(literal: &str) -> bool {
    literal
        .lines()
        .any(|line| line.trim_start().starts_with('#') || line.trim_start().starts_with("```"))
}

#[test]
fn no_sentence_carries_the_indentation_of_the_source_it_was_written_in() {
    // A backslash at the end of a source line eats the line break *and* the
    // indentation after it; without one, both survive into the string. So a
    // sentence broken across two source lines to fit the width arrives at
    // whoever reads it with fourteen spaces in the middle of it.
    //
    // The tool descriptions have had their own guard since three of them
    // shipped that way. This is the same mistake everywhere else it can be
    // made, and it had been made in four more places: a hook warning about a
    // payload Leteo could not read, and the Polish `due` line, which is text a
    // person reads in their terminal — plus assertion messages, which are text
    // a person reads at the moment a build fails and they are least inclined to
    // squint at it.
    //
    // `src/i18n` is exempt and only that: those files are the key legends the
    // wizard prints, where the runs of spaces are the columns.
    let mut offenders = Vec::new();
    let mut examined = 0;
    let mut multi_line = 0;
    for (path, text) in source_under("src") {
        if path.contains("i18n") {
            continue;
        }
        for literal in string_literals(&text) {
            examined += 1;
            if literal.contains('\n') {
                multi_line += 1;
            }
            if looks_like_sql(&literal) || looks_like_a_document(&literal) {
                continue;
            }
            // A literal whose *several* lines have runs is a table — the
            // verdict legend inside the judging prompt is one — and a table's
            // alignment is the point of it.
            let segments: Vec<&str> = literal.split("\\n").flat_map(|part| part.lines()).collect();
            let runs = segments
                .iter()
                .filter(|segment| joined_without_the_backslash(segment))
                .count();
            let breaks = segments
                .windows(2)
                .filter(|pair| broken_by_the_line_it_was_written_on(pair[0], pair[1]))
                .count();
            if (runs == 1 && segments.len() < 3) || (breaks > 0 && runs + breaks == 1) {
                offenders.push(format!("{path}: {literal:?}"));
            }
        }
    }
    // What the reader above reached, because a scanner that desynchronises on
    // one raw string reads every file after it inside out and finds nothing.
    // Both numbers are far under what the tree holds, so they say the reader is
    // still running rather than pinning a figure that has to be edited.
    assert!(
        examined > 4000 && multi_line > 100,
        "the reader found {examined} literals, {multi_line} of them spanning lines, \
         so it has stopped reading this tree"
    );
    assert!(
        offenders.is_empty(),
        "these sentences carry the indentation of the source they were written in:\n{}",
        offenders.join("\n")
    );
}

/// The npm wrapper delivers this version, for the platforms this repository
/// builds.
///
/// The wrapper downloads a release archive named after its own version and
/// hands it to the caller as `leteo`. So its `package.json` version is not
/// packaging metadata, it is a URL: published one number behind, `npx leteo`
/// silently fetches the previous release and every user of that route is on
/// software the release notes do not describe. Nothing in npm or in Cargo would
/// say a word — the two version numbers live in different ecosystems and
/// neither reads the other.
///
/// The target table is held the same way and for a sharper reason. A platform
/// added to the release matrix and not to the wrapper does not fail loudly
/// there: `resolveTarget` answers "no prebuilt Leteo for linux-riscv64", which
/// reads exactly like a platform nobody builds for, so the person who sees it
/// goes and builds from source instead of reporting anything.
#[test]
fn the_npm_wrapper_ships_this_version_for_the_targets_this_repository_builds() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    let manifest = std::fs::read_to_string(root.join("npm").join("package.json"))
        .expect("read npm/package.json");
    let published = manifest
        .lines()
        .find_map(|line| {
            let line = line.trim();
            line.strip_prefix("\"version\":")
                .map(|rest| rest.trim().trim_matches(&[' ', '"', ','][..]).to_owned())
        })
        .expect("npm/package.json names a version");
    assert_eq!(
        published,
        env!("CARGO_PKG_VERSION"),
        "npm/package.json publishes {published} and the crate is {}, so `npx leteo` \
         would fetch the wrong release",
        env!("CARGO_PKG_VERSION")
    );

    // Read from the workflow rather than from a list written here, so the
    // release matrix stays the one place a target is added.
    let workflow =
        std::fs::read_to_string(root.join(".github").join("workflows").join("release.yml"))
            .expect("read release.yml");
    let mut built: Vec<String> = workflow
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("target: ")
                .map(|target| target.trim().to_owned())
        })
        .collect();
    built.sort();
    built.dedup();
    assert!(
        built.len() > 3,
        "only found {} targets in release.yml, so this is no longer reading it",
        built.len()
    );

    let wrapper = std::fs::read_to_string(root.join("npm").join("bin").join("leteo.js"))
        .expect("read npm/bin/leteo.js");
    let mut offered: Vec<String> = wrapper
        .lines()
        .filter_map(|line| {
            let (_, rest) = line.split_once("triple: \"")?;
            let (triple, _) = rest.split_once('"')?;
            Some(triple.to_owned())
        })
        .collect();
    offered.sort();
    offered.dedup();

    assert_eq!(
        offered, built,
        "npm/bin/leteo.js offers {offered:?} and release.yml builds {built:?}"
    );
}

/// Every manifest in the repository publishes the version this crate is.
///
/// The number is written in more files than anybody holds in their head, and
/// each is read by something different: `Cargo.toml` for crates.io,
/// `server.json` for the MCP registry — once for the server and once for each
/// package it offers — `npm/package.json` for the wrapper, which turns it into
/// a release URL, and the marketplace and plugin manifests, which are what
/// decides whether an installed plugin ever receives an update.
///
/// How many that is belongs to the `manifests` table below and not to the
/// paragraph above, which used to total them and to disagree with that table
/// about `server.json` — a count in prose contradicting the code under it, in
/// the file that guards counts. The first repair of that sentence replaced its
/// wrong total with a wrong distance, which is why neither it nor this one
/// carries a numeral now.
///
/// None of them can see the others. A release that bumps the crate and forgets
/// `server.json` publishes a registry entry pointing at the previous version;
/// one that forgets a plugin manifest leaves every installed plugin on the
/// hooks of the release before, silently, because Claude Code only offers an
/// update when that field moves.
///
/// So they are held to `CARGO_PKG_VERSION`, which is the one a build already
/// reads. Adding a manifest and not adding it to that table is the only way to
/// fall out of this, and that is a visible act rather than a forgotten one.
#[test]
fn every_manifest_publishes_the_version_this_crate_is() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let crate_version = env!("CARGO_PKG_VERSION");

    // How many `"version"` lines each file is expected to carry, because a
    // check that finds the first and stops would have passed `server.json`
    // while its second one said something else.
    let manifests = [
        // Three: the server's own, and one for each package it offers — the
        // crate and the npm wrapper. It was two until the npm package was
        // declared, and this line is what said so.
        ("server.json", 3),
        ("npm/package.json", 1),
        // Two: one marketplace entry per bundle a client installs from here.
        // Both ZCode and Claude Code read this same file, so it grew a second
        // entry when the ZCode bundle shipped, and this line is what said so.
        (".claude-plugin/marketplace.json", 2),
        ("plugin/claude-code/.claude-plugin/plugin.json", 1),
        ("plugin/codex/.codex-plugin/plugin.json", 1),
        ("plugin/zcode/.zcode-plugin/plugin.json", 1),
    ];

    for (relative, expected_count) in manifests {
        let path = root.join(relative);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let published: Vec<String> = text
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                line.strip_prefix("\"version\":")
                    .map(|rest| rest.trim().trim_matches(&[' ', '"', ','][..]).to_owned())
            })
            .collect();

        assert_eq!(
            published.len(),
            expected_count,
            "{relative} carries {} version fields and this expects {expected_count}; \
             if one was added it needs checking too",
            published.len()
        );
        for version in published {
            assert_eq!(
                version, crate_version,
                "{relative} publishes {version} and the crate is {crate_version}"
            );
        }
    }
}

/// Both published packages name the same MCP server `server.json` does.
///
/// The MCP registry will not accept a submission whose packages do not echo the
/// server's own name back at it, and it looks in a different place for each: the
/// `mcpName` field of the *published* npm tarball, and the marker
/// `mcp-name: <name>` in the crate's rendered readme. Both are refused when
/// absent or different. That is what makes this worth a guard rather than a
/// comment — the failure lands after the tag exists, and it cannot be repaired
/// for that version, because neither registry lets a published version be
/// replaced. The only fix is another release.
///
/// Which is how v0.2.0 reached crates.io, npm, GHCR and the GitHub release
/// while the MCP registry got nothing: `server.json` had named the server since
/// the beginning and `npm/package.json` had never carried the field at all.
///
/// The readme half was already correct and is held here anyway, because it is
/// the same claim in a third file and the first version of this guard checked
/// two of the three. It is also the more fragile of the two: it sits mid-
/// paragraph in ordinary prose, written out in the open rather than in an HTML
/// comment because crates.io strips those when it renders, so there is nothing
/// about its appearance that warns an editor off reflowing it.
#[test]
fn the_published_packages_name_the_server_that_server_json_names() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    let field = |relative: &str, key: &str| -> String {
        let path = root.join(relative);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let needle = format!("\"{key}\":");
        text.lines()
            .map(str::trim)
            .find_map(|line| line.strip_prefix(&needle))
            .map(|rest| rest.trim().trim_matches(&[' ', '"', ','][..]).to_owned())
            .unwrap_or_else(|| panic!("{relative} carries no {key}"))
    };

    // `server.json`'s first `"name"` is the server's own. The names below it
    // belong to arguments and environment variables, which is why this reads
    // the first and the file keeps the server's name at the top. A reordering
    // that put one of those first fails this loudly rather than passing wrongly,
    // because none of them can equal the server's name.
    let declared = field("server.json", "name");

    let published = field("npm/package.json", "mcpName");
    assert_eq!(
        published, declared,
        "npm/package.json says mcpName {published} and server.json names the \
         server {declared}; the MCP registry reads the published tarball and \
         would refuse the submission after the tag exists"
    );

    // The readme is matched as the substring the registry itself looks for,
    // not as a whole line: it sits inside a sentence, and asserting the shape
    // of that sentence would fail on a rewording that kept the marker intact.
    let readme = std::fs::read_to_string(root.join("README.md")).expect("read README.md");
    let marker = format!("mcp-name: {declared}");
    assert!(
        readme.contains(&marker),
        "README.md carries no `{marker}`, and it is what the MCP registry reads \
         to believe this repository owns the crate — Cargo.toml ships this file \
         as the crate readme, so the submission would be refused after the tag \
         exists"
    );
}

/// The `tags:` block belonging to the `metadata-action` step beginning at `from`.
///
/// Read as text rather than through a YAML parser because every other guard in
/// this file reads these workflows the same way. An earlier draft of this
/// sentence also claimed a parser would burden the release build, which is
/// false: it would be a dev-dependency, and `cargo build --release` compiles
/// none of those.
fn tag_patterns_after(lines: &[&str], from: usize) -> Vec<String> {
    let mut patterns = Vec::new();
    let mut opened_at = None;
    for line in lines.iter().skip(from + 1) {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if let Some(key) = opened_at {
            // A blank line inside a block scalar is part of it. YAML ends the
            // block at the first non-empty line indented no further than the
            // key that opened it — the next input, the next step, or a comment
            // between them. Breaking on the blank instead would have stopped
            // reading at a list somebody had grouped for readability, and a
            // pattern added below that gap would have gone unheld.
            if trimmed.is_empty() {
                continue;
            }
            if indent <= key {
                break;
            }
            // A `#` line inside this block is a comment to the action, so a
            // note added to one list and not the other is not a difference
            // between them. Trailing space is normalised for the same reason:
            // it makes no difference visible in the rendered YAML, and a build
            // should not fail over one. Not measured: whether the action
            // itself trims — if it did not, the result would be an invalid tag
            // and a loud failure rather than a quiet one.
            if trimmed.starts_with('#') {
                continue;
            }
            patterns.push(trimmed.trim_end().to_owned());
            continue;
        }
        if trimmed.starts_with("- ") {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("tags:") {
            let rest = rest.trim();
            // `tags: |` opens a block scalar; `tags: type=semver,...` is the
            // whole list on one line. Reading only the block form returned an
            // empty list and then reported the step as having no patterns,
            // which is a different thing from having them on one line.
            if rest.is_empty() || rest.starts_with('|') || rest.starts_with('>') {
                opened_at = Some(indent);
            } else {
                patterns.push(rest.to_owned());
                break;
            }
        }
    }
    patterns
}

/// Every `metadata-action` step in `release.yml` derives the version alike.
///
/// `docker/metadata-action` computes `org.opencontainers.image.version` from
/// whichever tag patterns it is given, and `release.yml` runs it twice: once in
/// the per-architecture job, which uses it for `labels:` alone, and once in the
/// merge, which publishes the tags. #14 restored the first of those without its
/// patterns, so it fell back to the action's defaults and would have published
/// `version=v0.1.3` beside a tag reading `0.1.3` — one version string spelled
/// two ways in one file. Two blind reviewers found it independently, and the
/// repair was to write the patterns out in both places.
///
/// Which left a hand-written second copy, and AGENTS.md rule 3 says how those
/// end. This is a guard over that duplication rather than a removal of it.
///
/// It holds that there are at least two such steps, that every pattern is a
/// plain `type=semver` one, that the first pattern written is the first the
/// action resolves, and that the lists match. That third one was assumed in
/// prose for a while before it was held; the loop that holds it says why beside
/// itself.
///
/// Not established: GitHub Actions has no include — it does not honour YAML
/// anchors — and the documented way to share a value is a workflow-level `env`
/// referenced from a step's `with:`. That was not tried. Whether a reference
/// that failed to resolve would fail the step or would quietly yield an empty
/// `tags:`, sending the action back to the defaults that caused the defect
/// above, has not been measured here. No workflow in this repository uses that
/// form, and `release.yml` runs on `v*` only, so a release would be the first
/// thing to exercise it — which is a reason to be careful where it is
/// introduced, not evidence that it does not work. A single `env` list held by
/// a guard like this one would answer the duplication and the check together,
/// and whoever can exercise it should prefer that to this.
#[test]
fn every_metadata_action_derives_the_version_from_the_same_patterns() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workflow =
        std::fs::read_to_string(root.join(".github").join("workflows").join("release.yml"))
            .expect("read release.yml");

    let lines: Vec<&str> = workflow.lines().collect();
    let blocks: Vec<(usize, Vec<String>)> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            // Either spelling, and either quoting. A step that needs no
            // `id:` is written `- uses: ...`, which is how every other action
            // step in this workflow is written, and matching only the bare
            // form would have left a third one unheld while this guard
            // reported success. The dash is stripped without assuming one
            // space after it, and the value without assuming it is unquoted.
            let step = line.trim();
            let step = step.strip_prefix('-').unwrap_or(step).trim_start();
            step.strip_prefix("uses:").is_some_and(|action| {
                action
                    .trim()
                    .trim_start_matches(['"', '\''])
                    .starts_with("docker/metadata-action")
            })
        })
        .map(|(index, _)| (index + 1, tag_patterns_after(&lines, index)))
        .collect();

    assert!(
        blocks.len() > 1,
        "found {} metadata-action steps in release.yml. Either one has been removed — which is \
         the defect this guard exists for, since the per-architecture step is where the labels \
         come from — or it is written in a form this scan no longer matches",
        blocks.len()
    );

    // The premise the leader check below rests on, held rather than asserted in
    // prose. `metadata-action` sorts the parsed tags by their `priority`
    // attribute and takes the first of that order; this guard reads the first
    // pattern WRITTEN. Those are the same tag only while every pattern carries
    // one priority, which is true of the default and stops being true the
    // moment a list says otherwise — silently, because both lists would still
    // match each other and `{{version}}` would still be written first while the
    // action labelled the image from whichever pattern won the sort. That
    // mutation passed this guard until #62.
    //
    // Refused rather than resolved. Ordering the patterns here would mean
    // reimplementing the action's sort, which is a second opinion about
    // somebody else's behaviour and the thing AGENTS.md rule 3 is about; and
    // the case worth catching is somebody reaching for the attribute at all,
    // not the order they end up with.
    for (line, patterns) in &blocks {
        for pattern in patterns {
            // Lower-cased with spaces removed. Recalled rather than measured,
            // and marked as such because the note on `tag_patterns_after` above
            // says the same about this action: `metadata-action` appears to
            // lower-case and trim each attribute key before reading it, so
            // `Priority=` and `priority =` reach it as the same input and did
            // not reach a literal `contains` as one. If that recollection is
            // wrong the guard merely refuses more than it must, which fails at
            // test time rather than in a release.
            //
            // More than the attribute #62 names, because more than one thing
            // separates the pattern written first from the one resolved first:
            // a `priority` reorders the list, an `enable=false` removes a tag
            // from it, and a different `type=` carries a different default
            // priority. That last is why the type is asserted here rather than
            // left to the comment below, which used to claim it and hold
            // nothing.
            let normalised: String = pattern
                .chars()
                .filter(|character| !character.is_whitespace())
                .flat_map(char::to_lowercase)
                .collect();
            assert!(
                normalised.starts_with("type=semver")
                    && !normalised.contains("priority=")
                    && !normalised.contains("enable="),
                "the metadata-action at release.yml:{line} gives {pattern:?} a pattern \
                 this guard cannot see past. It reads the first pattern written; the \
                 action takes the first that is `type=semver`, survives `enable` and \
                 sorts by `priority`. Resolve that here before allowing any of them"
            );
        }
    }

    let (first_line, first) = &blocks[0];
    // The first entry, not merely some entry. `metadata-action` sorts the
    // parsed tags by priority — the loop above is what keeps them all plain
    // `type=semver`, which is one default, and refuses a pattern naming a
    // priority of its own — and the sort is stable, so input order survives it.
    // The `version` output, which becomes `org.opencontainers.image.version`, is
    // taken from the first tag of that list. A list containing the right
    // pattern is not the same as a list led by it: `{{version}}` sitting
    // behind `{{major}}.{{minor}}` labels the image `0.1` while publishing
    // `0.1.3`.
    let Some(leads) = first.first() else {
        panic!(
            "the metadata-action at release.yml:{first_line} has no `tags:` input at all, so it \
             falls back to the action's own defaults and labels the image with the raw ref name"
        );
    };
    // `pattern={{version}}` and not merely `{{version}}`, because a substring
    // test also accepts `pattern=v{{version}}`. That is not the #14 mismatch —
    // the equality assert below keeps both lists the same, so a `v` would reach
    // the label and the tag alike. What it breaks is this workflow's own
    // verification: the merge computes `version="${GITHUB_REF_NAME#v}"` and
    // inspects `ghcr.io/<repo>:${version}`, so a `v`-prefixed pattern publishes
    // a tag that step then cannot find. Beyond what #62 asked for, and taken
    // because it is the other half of the assertion #62 hardened.
    // Normalised the same way the loop above normalises, for the reason it
    // gives: a spelling the action reads identically should not fail here on
    // whitespace or case, with a message about the leading pattern.
    let leads_normalised: String = leads
        .chars()
        .filter(|character| !character.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    assert!(
        leads_normalised.contains("pattern={{version}}"),
        "the metadata-action at release.yml:{first_line} leads with {leads:?}. \
         `org.opencontainers.image.version` is taken from the first pattern that resolves, so \
         unless that is the full semver the label disagrees with the tag published beside it"
    );

    for (line, patterns) in &blocks[1..] {
        assert_eq!(
            patterns, first,
            "the metadata-action steps at release.yml:{first_line} and release.yml:{line} were \
             given different tag patterns. Both derive `org.opencontainers.image.version` and one \
             of them also publishes the tags, so a difference here is an image whose version label \
             disagrees with its own tag"
        );
    }
}

/// The README describes the token figure for a reader who cannot see it, and
/// `assets/tokens.svg` describes itself for one who opens it alone. Those are
/// two descriptions of one picture, and only the second is generated.
///
/// So the first drifts, and it did: review of the change that added the figure
/// found a median hard-copied beside the list it came from, a run count spelled
/// out beside the two lists that add up to it, and an off-protocol caveat the
/// picture carried three times and the text replacing the picture carried not
/// at all.
///
/// Making the `alt` attribute *be* the figure's own `aria-label` does not remove
/// the copy — `README.md` still holds every byte of it. It makes the copy
/// checkable, which is what this is. Editing the same number in both still
/// passes, and nothing here compares the figure to what its generator would
/// produce.
#[test]
fn the_figure_describes_itself_the_same_way_twice() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let svg = std::fs::read_to_string(root.join("assets").join("tokens.svg"))
        .expect("read assets/tokens.svg");
    let readme = std::fs::read_to_string(root.join("README.md")).expect("read README.md");

    let aria = between(&svg, "aria-label=\"", "\"").expect(
        "assets/tokens.svg should carry an aria-label — it is the only description a reader \
         who opens the figure on its own ever gets",
    );
    assert!(
        aria.len() > 200 && aria.contains("off-protocol"),
        "the figure's aria-label is {} characters and {} name the off-protocol run, so this \
         guard is no longer reading the description it was written for",
        aria.len(),
        if aria.contains("off-protocol") {
            "does"
        } else {
            "does not"
        }
    );

    let img = readme
        .lines()
        .find(|line| line.contains("src=\"assets/tokens.svg\""))
        .expect("README.md should embed assets/tokens.svg");
    let alt = between(img, "alt=\"", "\"").expect("the figure's <img> should carry alt text");

    if alt != aria {
        let shared = alt
            .char_indices()
            .zip(aria.chars())
            .find(|((_, a), b)| a != b)
            .map_or(alt.len().min(aria.len()), |((i, _), _)| i);
        let window = |s: &str| {
            let from = shared.saturating_sub(30);
            format!("...{}...", &s[from..s.len().min(shared + 40)])
        };
        panic!(
            "README.md's alt text for the token figure has drifted from the figure's own \
             aria-label, at character {shared}. The aria-label is generated by \
             tools/chart/tokens.py from the runs; the alt is not, so it is the one that \
             goes stale — regenerate and copy it across rather than editing it.\n\n  \
             aria-label: {}\n  alt:        {}",
            window(aria),
            window(alt)
        );
    }
}

/// Returns what sits between the first `open` and the next `close` after it.
fn between<'a>(haystack: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let rest = &haystack[haystack.find(open)? + open.len()..];
    Some(&rest[..rest.find(close)?])
}

/// Every platform the release caches is a scope CI writes for the same Dockerfile.
///
/// The release derives its buildx cache scope from its own matrix —
/// `scope=${{ matrix.platform }}` — and `ci.yml` writes those strings out by
/// hand. That is a second copy, and #59 is what a second copy costs: #14 gave
/// the release matrix its per-platform scope and left `ci.yml` on buildx's
/// default, so the two stopped meeting and every release recompiled the amd64
/// image from cold, with nothing failing and no symptom but a slow build.
///
/// The pairing is what makes this a guard rather than a spell-check. A scope is
/// shared only if the same Dockerfile writes it, so `ci.yml` naming
/// `linux/amd64` on the MCP image would leave the release importing a scope no
/// build of its own image fills — this issue's defect with the string present.
/// Both sides therefore read `cache-to` scopes together with the `file:` of the
/// step writing them, and the release's Dockerfile is read from the step whose
/// scope is the matrix rather than named here.
///
/// What it does not do is bind a scope to the architecture that writes it. The
/// pairing is scope-to-Dockerfile and nothing else: were `ci.yml`'s amd64 job
/// to write `scope=linux/arm64` and the arm64 job `scope=linux/amd64`, both
/// strings are still written for `docker/Dockerfile` and this passes, while
/// each release leg imports the other architecture's layers. Reading that
/// needs the job a step belongs to — the amd64 build names no `platforms:`, so
/// only its `runs-on` says which architecture it is — and this parser has no
/// notion of a job. That is a separate guard; the sentence above is what this
/// one holds.
///
/// Read from the workflows rather than from a list written here, for the reason
/// `the_npm_wrapper_ships_this_version_for_the_targets_this_repository_builds`
/// gives about the same matrix: the release stays the one place a platform is
/// added. Adding a third one now fails here until `ci.yml` writes its scope too.
#[test]
fn every_release_cache_scope_is_one_ci_writes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".github")
        .join("workflows");
    let release = std::fs::read_to_string(root.join("release.yml")).expect("read release.yml");
    let ci = std::fs::read_to_string(root.join("ci.yml")).expect("read ci.yml");

    // Read from the `cache-to` that uses the expression rather than from
    // anywhere in the file: a `contains` over the whole text would also be
    // satisfied by a comment mentioning it after the value beneath had changed,
    // which is a test reading a comment by the back door.
    let matrix_scope = cache_scopes_by_file(&release)
        .into_iter()
        .find(|export| export.scope == "${{ matrix.platform }}");
    let Some(release_export) = matrix_scope else {
        panic!(
            "no `cache-to` in release.yml derives its scope from `matrix.platform`, so the platforms read below are no longer the scopes it uses and this guard is watching nothing"
        );
    };
    let release_dockerfile = release_export.file;

    assert!(
        !release_dockerfile.is_empty(),
        "release.yml's matrix-scoped `cache-to` has no `file:` in its own step, so this guard has \
         no Dockerfile to match `ci.yml` against and an unattributed scope on either side would \
         compare equal to one on the other"
    );

    // A list marker is stripped because a matrix entry may name the platform
    // first, and that spelling is invisible to a bare `platform: ` match — the
    // shape this guard is least able to afford missing, since a platform it
    // cannot see is one it cannot demand a scope for.
    let platforms: Vec<&str> = release
        .lines()
        .map(|line| cut_trailing_comment(line.trim()))
        .map(|line| strip_list_marker(line).unwrap_or(line))
        .filter_map(|line| line.strip_prefix("platform: "))
        .map(str::trim)
        .collect();
    assert!(
        !platforms.is_empty(),
        "no `platform:` entries in release.yml, so this guard is no longer reading its matrix"
    );

    // Only what `ci.yml` writes counts, and only from a build of the same
    // Dockerfile. A `cache-from` alone would read a scope nobody fills, which
    // is the shape of the defect rather than the fix.
    let written: Vec<&str> = cache_scopes_by_file(&ci)
        .into_iter()
        .filter(|export| export.file == release_dockerfile)
        .map(|export| export.scope)
        .collect();

    for platform in &platforms {
        assert!(
            written.contains(platform),
            "release.yml builds `{release_dockerfile}` for `{platform}` and caches it under that scope, but no `cache-to` in ci.yml writes that scope from a build of the same file, so a release starts from cold however warm a `main` run left it — and it is `main` that pays, since a tag build cannot read a pull request's cache. ci.yml writes {written:?} for `{release_dockerfile}`"
        );
    }
}

/// Every `cache-to` naming a scope exports every stage, to the backend the
/// other workflow reads.
///
/// The pairing guard above reads `scope=` and nothing else on the line, so
/// `cache-to: type=gha,scope=linux/amd64` satisfies it while buildx's default
/// `mode=min` exports the final stage only — and `docker/Dockerfile` is
/// multi-stage, so the `cargo build --release` layer the scope exists to keep
/// warm would not be in what the release imports. That is #59's symptom
/// restored with every guard green, which is the one thing a guard cannot
/// afford. `type=` is the same shape one backend out.
#[test]
fn every_scoped_cache_export_writes_every_stage_where_the_other_workflow_reads_it() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".github")
        .join("workflows");
    for name in ["ci.yml", "release.yml"] {
        let workflow = std::fs::read_to_string(root.join(name)).expect("read the workflow");
        let exports = cache_scopes_by_file(&workflow);
        assert!(
            !exports.is_empty(),
            "no `cache-to` in {name} names a scope, so this guard is watching nothing"
        );
        for export in exports {
            let missing = export.missing();
            assert!(
                missing.is_empty(),
                "`cache-to: {}` in {name} names `scope={}` without {}, so what it exports is not what the other workflow imports: `mode=max` is what puts a multi-stage build's earlier layers in the manifest at all, and `type=gha` is the only backend either workflow reads",
                export.options,
                export.scope,
                missing.join(" and ")
            );
        }
    }
}

/// A step's `file:` survives a list written inside that step, and does not
/// survive the next step.
///
/// Read from a fixture because neither workflow writes the shape: `ci.yml`
/// spells `tags:` inline, so a guard over the real files would pass whatever
/// the parser did with a block-style one. That is how the previous rule
/// shipped — it dropped the name at every list item, so a `tags:` entry blanked
/// the Dockerfile and turned the pairing guard red with a message about caches
/// — and nothing in the tree could have shown it.
#[test]
fn a_list_inside_a_step_does_not_take_the_dockerfile_with_it() {
    let workflow = "\
jobs:
  images:
    steps:
      - uses: docker/build-push-action@v6
        with:
          file: docker/Dockerfile
          tags:
            - leteo:ci
          cache-to: type=gha,mode=max,scope=linux/amd64
      - uses: docker/build-push-action@v6
        with:
          cache-to: type=gha,mode=max,scope=stray
";
    let attributed: Vec<(&str, &str)> = cache_scopes_by_file(workflow)
        .iter()
        .map(|export| (export.file, export.scope))
        .collect();
    assert_eq!(
        attributed,
        vec![("docker/Dockerfile", "linux/amd64"), ("", "stray")],
        "a first pair that lost its name is the `tags:` list read as a step boundary, which fails \
         the pairing guard for a cache defect that is not there; a second pair that gained one is \
         a Dockerfile carried across a step boundary, which credits a scope to a build that never \
         wrote it"
    );
}

/// A `cache-to:` a workflow writes: the scope it names, the `file:` of the step
/// writing it, and the options it carries beside the scope.
struct CacheTo<'a> {
    file: &'a str,
    scope: &'a str,
    options: &'a str,
}

impl CacheTo<'_> {
    /// The options an export naming a scope needs and does not carry.
    ///
    /// `mode=max` because `docker/Dockerfile` is multi-stage and buildx's
    /// default `mode=min` exports the final stage only: the
    /// `cargo build --release` layer these scopes exist to keep warm would not
    /// be in the manifest the release imports. `type=gha` because a scope
    /// exported to any other backend is a string the release's own `type=gha`
    /// import never reads. Either one missing is #59's symptom back with the
    /// scope present and the pairing guard green.
    fn missing(&self) -> Vec<&'static str> {
        ["type=gha", "mode=max"]
            .into_iter()
            .filter(|option| !self.options.split(',').any(|part| part.trim() == *option))
            .collect()
    }
}

/// The `cache-to` exports a workflow writes, each paired with the `file:` of
/// the step writing it.
///
/// `file:` precedes `cache-to:` inside a step's `with:` block, so the nearest
/// one above is that step's own — but only within the step, which is why the
/// name is dropped at a list item indented less than the `file:` that set it.
/// A list item nested deeper is inside that same step: a block-style `tags:`
/// entry is one, and the previous rule — drop the name at every list item —
/// blanked the Dockerfile there and would have failed the pairing guard above
/// with a message about caches, which is a failure naming a cause that is not
/// there. Carried the other way, across a step boundary, the name would credit
/// one step's scope to the Dockerfile of whichever step last named one, and a
/// step may legitimately omit `file:`: the action defaults it to
/// `{context}/Dockerfile`. That is the string-on-the-wrong-build case this
/// guard exists for, so the attribution has to fail rather than guess. An
/// empty name matches no Dockerfile the release builds, so it fails loudly.
///
/// A trailing comment is cut before anything is read, so that `scope=` inside
/// one is not mistaken for the value beside it. Without that, `cache-to:
/// type=gha,mode=max # scope=linux/amd64` would count as writing a scope the
/// build does not write — which is this guard's own defect, and would make a
/// liar of the sentence above about comments. The cut looks for any whitespace
/// before the `#`, not a space: YAML ends a scalar at either, so matching only
/// the space leaves the same hole open behind a tab.
///
/// The scope is read up to the next option rather than to end of line, because
/// `scope=` is last only by convention and `type=gha,scope=x,mode=max` means
/// the same thing.
fn cache_scopes_by_file(workflow: &str) -> Vec<CacheTo<'_>> {
    let mut file = "";
    let mut file_indent = 0;
    let mut exports = Vec::new();
    for raw in workflow.lines() {
        let indent = raw.len() - raw.trim_start().len();
        let line = cut_trailing_comment(raw.trim());
        if strip_list_marker(line).is_some() && indent < file_indent {
            file = "";
        }
        if let Some(name) = line.strip_prefix("file: ") {
            file = name;
            file_indent = indent;
        } else if let Some(options) = line.strip_prefix("cache-to:").map(str::trim)
            && let Some(rest) = options.split("scope=").nth(1)
        {
            exports.push(CacheTo {
                file,
                scope: rest.split(',').next().unwrap_or(rest),
                options,
            });
        }
    }
    exports
}

/// A scalar ends at a `#` preceded by whitespace — a tab as readily as a space
/// — so the search is for either. Matching only the space is how the first
/// version of this let `cache-to: type=gha,mode=max<tab># scope=linux/amd64`
/// count as writing a scope, which is the defect the cut exists to prevent.
fn cut_trailing_comment(line: &str) -> &str {
    match line
        .char_indices()
        .find(|(i, c)| *c == '#' && (*i == 0 || line[..*i].ends_with(char::is_whitespace)))
    {
        Some((at, _)) => line[..at].trim_end(),
        None => line,
    }
}

/// What follows a YAML list marker, if this line carries one.
///
/// `-` then any amount of whitespace, or `-` alone on its line: all three are
/// one list item, and matching only `"- "` missed two of them. That mattered in
/// both directions — an entry the platform scrape could not see is a platform
/// it cannot demand a scope for, and a step boundary the parser does not notice
/// is a Dockerfile name carried into a step that never named one.
///
/// Flow style, `- {platform: linux/riscv64}`, is still missed. Reading that
/// needs a YAML parser rather than a prefix, and neither workflow here writes
/// one; the doc above says what this guard covers rather than claiming the key
/// is unmissable.
fn strip_list_marker(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('-')?;
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then(|| rest.trim_start())
}
