//! Every command the README shows has to be one the binary accepts.
//!
//! Documentation drifts in silence. A flag renamed, a subcommand moved, and the
//! README keeps promising the old spelling — the code compiles, the tests pass,
//! and the first person to hit it is somebody following the instructions.
//!
//! This ran once by hand and found nothing wrong, which is exactly when a check
//! is worth keeping: it is cheap now and it is what notices the next rename.

use std::path::Path;

use assert_cmd::Command;

/// Splits a shell-ish line, honouring the double quotes the README uses around
/// titles with spaces. Nothing here needs escapes or single quotes.
fn arguments(line: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for character in line.chars() {
        match character {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// Every `leteo …` line inside a fenced code block, with trailing comments cut.
fn documented_commands(markdown: &str) -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    let mut inside = false;
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            inside = !inside;
            continue;
        }
        let line = line.trim();
        if !inside || !line.starts_with("leteo ") {
            continue;
        }
        let line = line.split('#').next().unwrap_or(line).trim();
        let mut parts = arguments(line);
        if parts.is_empty() {
            continue;
        }
        parts.remove(0);
        if !parts.is_empty() {
            commands.push(parts);
        }
    }
    commands
}

#[test]
fn every_command_the_readme_shows_is_one_the_binary_accepts() {
    let readme = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"))
        .expect("read README.md");
    let commands = documented_commands(&readme);
    assert!(
        commands.len() > 20,
        "the extractor found only {} commands, so it has stopped matching the README",
        commands.len()
    );

    let mut rejected = Vec::new();
    for parts in &commands {
        // `--help` makes clap walk the subcommand path and validate every flag,
        // then stop before any argument is required and before a database is
        // opened. So this checks the spelling without doing anything.
        let output = Command::cargo_bin("leteo")
            .expect("find leteo test binary")
            .args(parts)
            .arg("--help")
            .output()
            .expect("run leteo");
        if !output.status.success() {
            let reason = String::from_utf8_lossy(&output.stderr);
            let reason = reason.lines().next().unwrap_or("").to_owned();
            rejected.push(format!("leteo {} -> {reason}", parts.join(" ")));
        }
    }

    assert!(
        rejected.is_empty(),
        "the README documents commands this binary refuses:\n  {}",
        rejected.join("\n  ")
    );
}

#[test]
fn the_extractor_reads_a_fenced_block_the_way_the_readme_writes_one() {
    // The guard above is only as good as this, so it is worth its own test:
    // an extractor that quietly matched nothing would pass every time.
    let markdown = "text\n\n```powershell\nleteo save \"A title with spaces\" body --type decision\n\
                    leteo stats   # a trailing comment\n```\n\nprose mentioning leteo stats\n";

    let commands = documented_commands(markdown);

    assert_eq!(
        commands,
        vec![
            vec![
                "save".to_owned(),
                "A title with spaces".to_owned(),
                "body".to_owned(),
                "--type".to_owned(),
                "decision".to_owned()
            ],
            vec!["stats".to_owned()],
        ],
        "prose outside a fence is not a command, and a comment is not an argument"
    );
}

/// Every `LETEO_*` name the binary reads from its own source.
///
/// Read out of the source rather than out of a list somebody maintains: a list
/// is one more thing to forget, and this check exists because two were
/// forgotten already.
fn variables_the_binary_reads(source: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![source.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|kind| kind != "rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            // `#[arg(env = "…")]` and `env::var("…")` are the only two ways
            // this binary reaches for the environment.
            for marker in [
                "env = \"LETEO_",
                "env::var(\"LETEO_",
                "env::var_os(\"LETEO_",
            ] {
                for (index, _) in text.match_indices(marker) {
                    let rest = &text[index + marker.len() - "LETEO_".len()..];
                    let name: String = rest
                        .chars()
                        .take_while(|c| c.is_ascii_uppercase() || *c == '_')
                        .collect();
                    if !name.is_empty() && !found.contains(&name) {
                        found.push(name);
                    }
                }
            }
        }
    }
    found.sort();
    found
}

#[test]
fn every_environment_variable_the_binary_honours_is_documented() {
    // Two were not: `LETEO_CLOUD_SERVER`, which is how somebody points the
    // client at a cloud, and `LETEO_AGENT_CLI`, which chooses the model that
    // judges conflicts. Both changed behaviour and neither appeared in the
    // table anybody would read to find them.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let readme = std::fs::read_to_string(root.join("README.md")).expect("read README.md");
    let table: String = readme
        .lines()
        .skip_while(|line| !line.starts_with("## Environment"))
        .take_while(|line| !line.starts_with("## License"))
        .collect::<Vec<_>>()
        .join("\n");

    let read = variables_the_binary_reads(&root.join("src"));
    assert!(
        read.len() > 5,
        "the scanner found only {read:?}, so it has stopped matching the source"
    );

    let missing: Vec<&String> = read
        .iter()
        .filter(|name| !table.contains(name.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "the binary reads these and the Environment table does not list them: {missing:?}"
    );
}

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
        if shown.ends_with("documented_commands.rs") {
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

/// Spells a small number the way the README's prose does.
fn spelled(count: usize) -> String {
    match count {
        10 => "ten".to_owned(),
        11 => "eleven".to_owned(),
        12 => "twelve".to_owned(),
        13 => "thirteen".to_owned(),
        14 => "fourteen".to_owned(),
        other => panic!("nobody has written the word for {other}"),
    }
}

#[test]
fn every_count_the_readme_writes_out_matches_the_list_it_describes() {
    // A count in prose is a second copy of the list it counts. The skill said
    // "the three that change or count the whole store" beside an enumeration of
    // them, and a fourth would have been named in that same sentence and left
    // the word wrong; the same shape is here three times over, on the first
    // page anybody reads about this crate. None of the three was checked: the
    // README's commands and its environment table were, and its arithmetic was
    // not.
    // Read with the line wrapping collapsed, so a reflowed paragraph does not
    // read as a changed claim: every sentence below is wrapped in the file.
    let readme = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"))
        .expect("read README.md");
    let readme = readme.split_whitespace().collect::<Vec<_>>().join(" ");

    let tools = leteo::mcp::PROFILE_AGENT.len() + leteo::mcp::PROFILE_ADMIN.len();
    assert!(
        readme.contains(&format!("A {tools}-tool MCP server")),
        "the README counts the MCP tools by hand and there are {tools}"
    );

    let clients = spelled(leteo::setup::supported_agents().len());
    assert!(
        readme.contains(&format!("setup support for {clients} MCP")),
        "the README counts the agents it can set up by hand and there are {clients}"
    );

    // The languages are not only counted but enumerated, and the count appears
    // three times over: once for the interface, once for the voice, and once to
    // say that `language` is not limited to them. All of them move together.
    let languages = leteo::settings::Interface::ALL;
    let spelled_languages = spelled(languages.len());
    for claim in [
        format!("{spelled_languages} offered for memories"),
        format!("It is the same {spelled_languages} languages"),
        format!("not limited to the {spelled_languages} above"),
    ] {
        assert!(
            readme.contains(&claim),
            "the README should say {claim:?}, because that is how many languages there are"
        );
    }
    let opening = format!(
        "{}{} languages",
        spelled_languages[..1].to_uppercase(),
        &spelled_languages[1..]
    );
    let at = readme
        .find(&opening)
        .expect("the README introduces the interface languages by counting them");
    let enumerated = &readme[at..readme.len().min(at + opening.len() + 200)];
    for language in languages {
        assert!(
            enumerated.contains(language.as_str()),
            "the README lists the interface languages and leaves out {}",
            language.as_str()
        );
    }
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

/// Every `.rs` file under a directory, as (path, text).
fn source_under(directory: &str) -> Vec<(String, String)> {
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

/// Every file and guard `openspec/` names in prose is one that exists.
///
/// `openspec/` is the written half of this project, and nothing checked it. The
/// README's commands, its environment table and its arithmetic all have guards;
/// the specs had none, and two of the four defects found in one review sitting
/// were there. `replication.md` listed four functions of
/// `src/store/replication.rs` under Known gaps that had been deleted, so the
/// spec described a hole that was already filled, in the file a reader goes to
/// in order to trust the rest.
///
/// The markdown link checker does not see these: a spec cites `src/store/
/// search.rs` and `a_guard_that_says_something` between backticks, which is
/// prose, not a link. This reads the backticks.
///
/// Paths a *user* ends up with are not repository paths — `settings.json`,
/// `.leteo/config.json`, the instruction files `setup` writes — so only a
/// citation that looks like a path *into this tree* is checked, and a name that
/// matches a file anywhere in it counts.
#[test]
fn every_file_and_guard_the_specs_name_is_one_that_exists() {
    let mut sources = source_under("src");
    // The specs cite the integration guards too, and those are not under `src`.
    // Collected here rather than through `source_under`, which insists on
    // finding more than fifteen files and there are three.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    for entry in std::fs::read_dir(&root).expect("read tests").flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|kind| kind == "rs") {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            sources.push((path.display().to_string(), text));
        }
    }
    let rust: String = sources
        .iter()
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    // Compared with forward slashes whatever the platform hands back, because
    // that is how a spec writes a path.
    let tracked: Vec<String> = sources
        .iter()
        .map(|(path, _)| path.replace('\\', "/"))
        .collect();

    // `specs/` only. A proposal under `changes/` argues for something that does
    // not exist yet — naming a table it would add is the whole point of one —
    // and `changes/README.md` names two files that are examples of how to name
    // a proposal rather than proposals.
    let documents = markdown_under("openspec/specs");
    assert!(
        documents.len() >= 7,
        "only {} documents found, so this has stopped reading openspec/specs/",
        documents.len()
    );

    let mut missing = Vec::new();
    let mut checked = 0usize;
    for (document, text) in &documents {
        for cited in text.split('`').skip(1).step_by(2) {
            let cited = cited.trim();
            // A path into this tree: has a directory in it and a suffix we own.
            let is_path = cited.contains('/')
                && cited.ends_with(".rs")
                && !cited.contains(' ')
                && !cited.contains('*');
            // A guard: long, snake_case, no punctuation.
            let is_guard = cited.len() > 14
                && cited
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                && cited.contains('_');
            if !is_path && !is_guard {
                continue;
            }
            checked += 1;
            let found = if is_path {
                tracked
                    .iter()
                    .any(|path| path == cited || path.ends_with(cited))
            } else {
                rust.contains(cited)
            };
            if !found {
                missing.push(format!("{document}: `{cited}`"));
            }
        }
    }
    assert!(
        checked > 40,
        "only {checked} citations examined, so the reader has stopped finding them"
    );
    missing.sort();
    missing.dedup();
    assert!(
        missing.is_empty(),
        "openspec/ names these, and they are not in the tree:\n  {}",
        missing.join("\n  ")
    );
}

/// Every markdown file under a directory, with its text.
fn markdown_under(directory: &str) -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(directory);
    let mut found = Vec::new();
    let mut stack = vec![root];
    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(&directory)
            .expect("read documents")
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|kind| kind == "md") {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                found.push((path.display().to_string(), text));
            }
        }
    }
    found
}
