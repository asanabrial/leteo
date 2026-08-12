//! Guards over the repository itself, rather than over what it documents.
//!
//! None of these reads the README. They sat beside the documentation checks
//! because that file was written first, and being there made them look like
//! checks on prose — which is the one thing they are not. Each watches
//! something the compiler cannot see: a source file no `mod` declares and which
//! therefore never compiles, a test that would open the store somebody keeps
//! their real memories in, a schema column nothing writes, a sentence that
//! reaches a person carrying the indentation of the Rust source it was
//! formatted in, and an npm package that would hand somebody a different
//! version of Leteo than the one it says it is.

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

/// Every column is written by something, or is one of the six that are not.
///
/// Found by counting non-null values on a real store and then looking for a
/// writer: six columns across two tables have neither. Three of them — the
/// embedding trio — are argued for in the baseline migration itself, and the
/// other three said nothing at all, so a reader of the schema would reasonably
/// have assumed a memory can expire and a relation can be superseded by a later
/// one. Neither happens.
///
/// The list is the point rather than an exemption: a seventh dead column fails
/// here, and so does one of these six coming to life, because then the sentence
/// in `store-and-schema.md` §10 has stopped being true and somebody has to say
/// what it does now.
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
/// The number is written in six places and read by six different things:
/// `Cargo.toml` for crates.io, `server.json` twice for the MCP registry,
/// `npm/package.json` for the wrapper — which turns it into a release URL —
/// and the marketplace and plugin manifests, which are what decides whether an
/// installed plugin ever receives an update.
///
/// None of them can see the others. A release that bumps the crate and forgets
/// `server.json` publishes a registry entry pointing at the previous version;
/// one that forgets a plugin manifest leaves every installed plugin on the
/// hooks of the release before, silently, because Claude Code only offers an
/// update when that field moves.
///
/// So they are held to `CARGO_PKG_VERSION`, which is the one a build already
/// reads. Adding a seventh manifest and not adding it here is the only way to
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
        (".claude-plugin/marketplace.json", 1),
        ("plugin/claude-code/.claude-plugin/plugin.json", 1),
        ("plugin/codex/.codex-plugin/plugin.json", 1),
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
