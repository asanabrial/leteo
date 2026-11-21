//! What a memory cites, and whether it still resolves.
//!
//! A memory that names a commit or a file is read as current for as long as it
//! is shown, and nothing ever looked at whether the thing it names is still
//! there. `docs/does-memory-save-tokens.md` measured it on twelve sampled
//! memories: three cited a commit that resolves to nothing and one a stale path.
//! This is the cheap half of the answer — it finds them, it changes nothing.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How many memory ids the report names before it counts the rest.
///
/// A published limit is the limit that is applied: the ids are what somebody
/// opens next, and a store with four hundred stale memories would otherwise put
/// four hundred numbers into one line of a reply that has a size budget.
pub const EXAMPLE_IDS: usize = 10;

/// How long the single `git cat-file` may take.
///
/// Far longer than the two seconds project detection allows itself, because this
/// is one process answering every hash of a project and not a question asked on
/// the path of somebody's keystroke; and still bounded, so a stuck git is a
/// check that says it could not run rather than a `doctor` that never returns.
const GIT_TIMEOUT: Duration = Duration::from_secs(30);

/// The shortest abbreviation of a commit this reads. Git's own default is seven.
const SHORTEST_ABBREVIATION: usize = 7;
const FULL_HASH: usize = 40;
/// The length of an MD5 digest, which is the one hexadecimal word of that size
/// that turns up in prose often enough to be worth refusing by name. No
/// abbreviation anybody writes is thirty-two characters long.
const MD5_DIGEST: usize = 32;

/// Words that, written just before a short hash, say it is a commit.
const COMMIT_WORDS: [&str; 4] = ["commit", "commits", "sha", "merge"];

/// The extensions that make a name with no directory in it look like a path.
const SOURCE_EXTENSIONS: [&str; 36] = [
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "java", "kt", "kts", "swift", "c",
    "h", "cc", "cpp", "hpp", "cs", "rb", "php", "sh", "ps1", "sql", "md", "toml", "json", "yml",
    "yaml", "lock", "html", "css", "scss", "vue", "svelte", "gradle",
];

/// Where the opt-in check looks: the directory it is run from, and the project
/// whose memories it reads when the caller named one.
///
/// Both are the caller's to supply because the store knows neither: it has no
/// working directory, and an MCP server's is not the one a person is standing in.
#[derive(Debug, Clone)]
pub struct Scope {
    pub directory: PathBuf,
    pub project: Option<String>,
}

/// The commits and paths one piece of text cites, each once, in the order the
/// text first names them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Citations {
    pub commits: Vec<String>,
    pub paths: Vec<String>,
}

impl Citations {
    pub fn is_empty(&self) -> bool {
        self.commits.is_empty() && self.paths.is_empty()
    }
}

/// Everything `text` cites.
pub fn extract(text: &str) -> Citations {
    Citations {
        commits: commits_in(text),
        paths: paths_in(text),
    }
}

/// The commits `text` cites.
///
/// A forty-character hexadecimal word is a commit wherever it stands: nothing
/// else is that long and that regular except a digest of another kind, and the
/// decision for this check was to count those.
///
/// An abbreviation, seven to thirty-nine characters, is a commit only when the
/// text says so, because a bare seven-character hexadecimal word is far more
/// often something else — `deadbee` the word, a colour, a short id, a version.
/// It says so in one of two ways: the word is the whole of a span in backticks,
/// or it follows `commit`, `commits`, `sha` or `merge`, with at most a colon and
/// blanks between. Either way it has to contain a digit and a letter, which is
/// what keeps `defaced` and `1234567` out, and it may not be thirty-two long,
/// which is an MD5 digest.
///
/// What this deliberately misses: a hash in the middle of a list after the first
/// (`commits a1b2c3d and e4f5a6b` finds the first), an abbreviation written with
/// no word and no backticks, one in capitals, one made only of digits or only of
/// letters, and a SHA-256 name of sixty-four characters, which no repository
/// that uses SHA-1 can resolve. A word with a `-` or `_` against it is not a
/// hash either: `abc1234-fix` is a branch, and a UUID's first group is not a
/// commit.
pub fn commits_in(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    let mut index = 0;
    while index < bytes.len() {
        if !is_word_byte(bytes[index]) {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && is_word_byte(bytes[index]) {
            index += 1;
        }
        let word = &text[start..index];
        if !word.bytes().all(is_lower_hex) {
            continue;
        }
        let cites = match word.len() {
            FULL_HASH => true,
            MD5_DIGEST => false,
            length if (SHORTEST_ABBREVIATION..FULL_HASH).contains(&length) => {
                has_digit_and_letter(word)
                    && (in_backticks(bytes, start, index) || follows_commit_word(&text[..start]))
            }
            _ => false,
        };
        if cites && seen.insert(word) {
            found.push(word.to_owned());
        }
    }
    found
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

fn has_digit_and_letter(word: &str) -> bool {
    word.bytes().any(|byte| byte.is_ascii_digit())
        && word.bytes().any(|byte| byte.is_ascii_lowercase())
}

fn in_backticks(bytes: &[u8], start: usize, end: usize) -> bool {
    start > 0 && bytes[start - 1] == b'`' && bytes.get(end) == Some(&b'`')
}

/// Whether the text before a word ends in one of [`COMMIT_WORDS`].
fn follows_commit_word(before: &str) -> bool {
    let without_blanks = before.trim_end_matches([' ', '\t']);
    let without_colon = without_blanks.strip_suffix(':').unwrap_or(without_blanks);
    let without_colon = without_colon.trim_end_matches([' ', '\t']);
    // A word glued to the hash (`commitabc1234`) is one longer word and was
    // never split from it, so there is always a blank or a colon to have
    // removed; nothing removed means the hash begins the line or follows a
    // symbol, and neither names a commit.
    if without_colon.len() == before.len() {
        return false;
    }
    let word_start = without_colon
        .rfind(|character: char| !character.is_ascii_alphabetic())
        .map_or(0, |at| at + 1);
    let word = &without_colon[word_start..];
    let glued = without_colon[..word_start]
        .chars()
        .next_back()
        .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_');
    !glued
        && COMMIT_WORDS
            .iter()
            .any(|candidate| word.eq_ignore_ascii_case(candidate))
}

/// The repository-relative paths `text` cites in backticks.
///
/// Only a span in backticks is read, and only one that is the whole of a path:
/// repository-relative, with a `/` in it or a known source extension, after a
/// trailing `:line`, `:start-end` or `:line:column` is taken off. Backslashes
/// are read as `/`, so a path written on Windows is the same citation.
///
/// Refused outright, because each is something else or cannot be tested against
/// a checkout: an absolute path, a drive, `~`, a URL or anything else with a
/// colon left in it, a `..`, a glob or a placeholder (`*?[]{}<>$`), a quote, a
/// space, and an `@` (a scoped package, not a file). A fenced block is not read
/// at all, since its lines are code and not a statement about the repository.
///
/// What this deliberately misses: a path written without backticks, and a line
/// reference spelled `#L12` or `(line 12)`.
pub fn paths_in(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        // A fence opened in the middle of a line, or a block still open: either
        // way what follows is code, and an odd count of backticks on the line
        // would pair the wrong ones.
        if fenced || line.contains("```") {
            continue;
        }
        let parts: Vec<&str> = line.split('`').collect();
        // An odd number of parts is an even number of backticks: every span is
        // closed. Otherwise the last backtick opened nothing.
        let closed = if parts.len() % 2 == 1 {
            parts.len()
        } else {
            parts.len() - 1
        };
        for span in parts[..closed].iter().skip(1).step_by(2) {
            if let Some(path) = path_candidate(span)
                && seen.insert(path.clone())
            {
                found.push(path);
            }
        }
    }
    found
}

const NOT_IN_A_PATH: &[char] = &[
    '*', '?', '[', ']', '{', '}', '<', '>', '$', '|', ';', '&', '"', '\'', '(', ')', '=', ',', '!',
    '#', '%', '^', '@', '~',
];

/// The citation a backticked span is, in the form it is checked, or nothing.
fn path_candidate(span: &str) -> Option<String> {
    let span = span.trim();
    if span.is_empty()
        || span.len() > 260
        || span.contains("://")
        || span.contains("..")
        || span.contains(NOT_IN_A_PATH)
        || span.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return None;
    }
    let mut path = span.replace('\\', "/");
    for _ in 0..2 {
        match path.rsplit_once(':') {
            Some((head, tail)) if is_line_reference(tail) => path = head.to_owned(),
            _ => break,
        }
    }
    if path.contains(':') || path.starts_with('/') || path.starts_with('-') {
        return None;
    }
    let path = path.strip_prefix("./").unwrap_or(&path);
    let path = path.strip_suffix('/').unwrap_or(path);
    if path.is_empty() || path.split('/').any(|part| part.is_empty() || part == ".") {
        return None;
    }
    (path.contains('/') || has_source_extension(path)).then(|| path.to_owned())
}

fn is_line_reference(text: &str) -> bool {
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    match text.split_once('-') {
        Some((start, end)) => digits(start) && digits(end),
        None => digits(text),
    }
}

fn has_source_extension(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').is_some_and(|(stem, extension)| {
        !stem.is_empty()
            && SOURCE_EXTENSIONS
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
    })
}

/// What looking for a cited path in a checkout found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathVerdict {
    Present,
    Missing,
    /// Nothing in the checkout can say whether this was a path at all: it has
    /// no source extension and its first directory is not one this repository
    /// has, which is what `and/or`, `read/write` and `2026/11/21` look like.
    /// A directory that was deleted outright is missed this way, and that is
    /// the price of not calling every `TCP/IP` stale.
    NotAPath,
    /// The filesystem would not say.
    Unreadable,
}

/// Looks for a path from [`paths_in`] in the checkout at `root`.
pub fn judge_path(root: &Path, path: &str) -> PathVerdict {
    let relative: PathBuf = path.split('/').collect();
    match root.join(&relative).try_exists() {
        Ok(true) => PathVerdict::Present,
        Ok(false) => {
            let first = path.split('/').next().unwrap_or(path);
            let known_directory = root.join(first).exists();
            if has_source_extension(path) || known_directory {
                PathVerdict::Missing
            } else {
                PathVerdict::NotAPath
            }
        }
        Err(_) => PathVerdict::Unreadable,
    }
}

/// Which of `hashes` git holds an object for, in the order given.
///
/// One process for all of them, not one each: a real store has a thousand
/// memories with a forty-character word in them. An object that is in the
/// repository and reachable from nothing still resolves, and so does an
/// abbreviation that more than one object begins with — in both cases the
/// citation did not name nothing, which is what this reports.
pub fn resolving_commits(root: &Path, hashes: &[String]) -> Result<Vec<bool>, String> {
    if hashes.is_empty() {
        return Ok(Vec::new());
    }
    let mut input = String::with_capacity(hashes.len() * (FULL_HASH + 1));
    for hash in hashes {
        input.push_str(hash);
        input.push('\n');
    }
    let output = crate::project::run_git_with_input(
        root,
        &["cat-file", "--batch-check"],
        input,
        GIT_TIMEOUT,
    )?;
    parse_batch_check(&output, hashes.len())
}

/// Reads what `git cat-file --batch-check` printed for `expected` names.
///
/// A line for an object is `<oid> <type> <size>`, one for a name git does not
/// have is `<name> missing`, and an abbreviation several objects begin with is
/// `<name> ambiguous`. Read by line and by position, line endings as git writes
/// them on the platform, so a Windows CRLF does not leave a `\r` for the match
/// to trip on. A count that does not equal the number asked is git answering
/// something other than the question, and is refused.
pub fn parse_batch_check(output: &str, expected: usize) -> Result<Vec<bool>, String> {
    let answers: Vec<bool> = output
        .lines()
        .map(|line| !line.trim_end().ends_with(" missing"))
        .collect();
    if answers.len() != expected {
        return Err(format!(
            "git answered {} lines for {expected} names",
            answers.len()
        ));
    }
    Ok(answers)
}

/// What a project's memories cite and what of it no longer resolves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assessment {
    pub memories_read: usize,
    pub commits: usize,
    pub missing_commits: usize,
    pub paths: usize,
    pub missing_paths: usize,
    pub unreadable_paths: usize,
    /// Every memory with at least one missing citation, smallest id first.
    pub stale: Vec<i64>,
}

/// Tests everything `memories` cite against the checkout at `root`.
///
/// Fails, and says why, when git could not be asked: a count of commits that
/// did not resolve is only a count if git answered, and one that guessed "none"
/// would read as a clean store.
pub fn assess(root: &Path, memories: &[(i64, String)]) -> Result<Assessment, String> {
    let cited: Vec<(i64, Citations)> = memories
        .iter()
        .map(|(id, text)| (*id, extract(text)))
        .filter(|(_, citations)| !citations.is_empty())
        .collect();

    let mut hashes: Vec<String> = Vec::new();
    let mut paths: Vec<String> = Vec::new();
    for (_, citations) in &cited {
        for hash in &citations.commits {
            if !hashes.contains(hash) {
                hashes.push(hash.clone());
            }
        }
        for path in &citations.paths {
            if !paths.contains(path) {
                paths.push(path.clone());
            }
        }
    }

    let resolved = resolving_commits(root, &hashes)?;
    let missing_commits: HashSet<&str> = hashes
        .iter()
        .zip(&resolved)
        .filter(|(_, resolves)| !**resolves)
        .map(|(hash, _)| hash.as_str())
        .collect();

    let verdicts: HashMap<&str, PathVerdict> = paths
        .iter()
        .map(|path| (path.as_str(), judge_path(root, path)))
        .collect();
    let counted = |wanted: PathVerdict| verdicts.values().filter(|v| **v == wanted).count();

    let stale: BTreeSet<i64> = cited
        .iter()
        .filter(|(_, citations)| {
            citations
                .commits
                .iter()
                .any(|hash| missing_commits.contains(hash.as_str()))
                || citations
                    .paths
                    .iter()
                    .any(|path| verdicts.get(path.as_str()) == Some(&PathVerdict::Missing))
        })
        .map(|(id, _)| *id)
        .collect();

    Ok(Assessment {
        memories_read: memories.len(),
        commits: hashes.len(),
        missing_commits: missing_commits.len(),
        paths: paths.len() - counted(PathVerdict::NotAPath),
        missing_paths: counted(PathVerdict::Missing),
        unreadable_paths: counted(PathVerdict::Unreadable),
        stale: stale.into_iter().collect(),
    })
}

impl Assessment {
    /// The sentence `doctor` reports, naming no more than [`EXAMPLE_IDS`] ids.
    pub fn describe(&self, project: &str) -> String {
        let limits = "Nothing was hidden, changed or written. A commit resolves whenever git holds the object, reachable or not, so a dangling one passes, and a shallow clone cannot see older history; paths are looked for in the current checkout only, so one that exists on another branch reads as missing, and a file name with no directory is looked for at the repository root only";
        let unreadable = if self.unreadable_paths > 0 {
            format!(
                "; {} path(s) could not be read and are not counted either way",
                self.unreadable_paths
            )
        } else {
            String::new()
        };
        if self.commits + self.paths == 0 && self.unreadable_paths == 0 {
            return format!(
                "{} memories of project {project} were read and none of them cites a commit or a path. {limits}",
                self.memories_read
            );
        }
        if self.stale.is_empty() {
            return format!(
                "{} memories of project {project} were read and every one of the {} commit(s) and {} path(s) they cite resolves{unreadable}. {limits}",
                self.memories_read, self.commits, self.paths
            );
        }
        let shown: Vec<String> = self
            .stale
            .iter()
            .take(EXAMPLE_IDS)
            .map(i64::to_string)
            .collect();
        let rest = self.stale.len().saturating_sub(EXAMPLE_IDS);
        let and_more = if rest > 0 {
            format!(", and {rest} more")
        } else {
            String::new()
        };
        format!(
            "{} of {} memories of project {project} cite a commit or path that no longer resolves: {} of {} commit(s) and {} of {} path(s){unreadable}. Memory ids: {}{and_more}. {limits}",
            self.stale.len(),
            self.memories_read,
            self.missing_commits,
            self.commits,
            self.missing_paths,
            self.paths,
            shown.join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const REAL: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn a_forty_character_hash_is_a_commit_wherever_it_stands() {
        assert_eq!(commits_in(&format!("fixed in {REAL}.")), [REAL]);
        assert_eq!(commits_in(&format!("({REAL})")), [REAL]);
        assert_eq!(commits_in(&"0".repeat(40)), ["0".repeat(40)]);
        assert_eq!(
            commits_in(&format!("{REAL} and again {REAL}")),
            [REAL],
            "a hash cited twice is one citation"
        );
    }

    #[test]
    fn an_abbreviation_is_a_commit_in_backticks_or_after_the_word_commit() {
        assert_eq!(commits_in("see `a1b2c3d` for it"), ["a1b2c3d"]);
        assert_eq!(commits_in("landed in commit a1b2c3d"), ["a1b2c3d"]);
        assert_eq!(commits_in("Commit: a1b2c3d4e5"), ["a1b2c3d4e5"]);
        assert_eq!(commits_in("the sha a1b2c3d"), ["a1b2c3d"]);
        assert_eq!(commits_in("merge\ta1b2c3d"), ["a1b2c3d"]);
    }

    #[test]
    fn what_looks_like_a_hash_but_is_not_a_citation_is_left_alone() {
        for text in [
            "a bare a1b2c3d with nothing to say it is a commit",
            "colour `#ff8800aa` in the theme",
            "colour #ff8800aa in the theme",
            "md5 `9e107d9d372bb6826bd81d3542a419d6` of the file",
            "commit 9e107d9d372bb6826bd81d3542a419d6",
            "the word `defaced` and the commit defaced",
            "a number `1234567` and commit 1234567",
            "commit A1B2C3D in capitals",
            "commit a1b2c3d-fix is a branch",
            "uuid commit 123e4567-e89b-12d3-a456-426614174000",
            "commita1b2c3d glued to its word",
            "a1b2c3d",
            "`a1b2c3`",
            "sha256 `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`",
            "`x a1b2c3d4`",
        ] {
            assert!(commits_in(text).is_empty(), "{text:?} was read as a commit");
        }
    }

    #[test]
    fn a_hash_in_the_middle_of_a_longer_word_is_not_one() {
        assert!(commits_in(&format!("x{REAL}")).is_empty());
        assert!(commits_in(&format!("{REAL}0")).is_empty());
        assert!(commits_in(&format!("`{REAL}0`")).is_empty());
    }

    #[test]
    fn a_path_in_backticks_is_a_citation_and_its_line_number_is_not_part_of_it() {
        assert_eq!(paths_in("see `src/store/mod.rs`"), ["src/store/mod.rs"]);
        assert_eq!(paths_in("at `src/store/mod.rs:120`"), ["src/store/mod.rs"]);
        assert_eq!(
            paths_in("at `src/store/mod.rs:120-140`"),
            ["src/store/mod.rs"]
        );
        assert_eq!(
            paths_in("at `src/store/mod.rs:120:7`"),
            ["src/store/mod.rs"]
        );
        assert_eq!(paths_in("the `Cargo.toml`"), ["Cargo.toml"]);
        assert_eq!(paths_in("the dir `src/store/`"), ["src/store"]);
        assert_eq!(paths_in("the `./src/lib.rs`"), ["src/lib.rs"]);
        assert_eq!(
            paths_in("`src/a.rs` then `src/b.rs` then `src/a.rs`"),
            ["src/a.rs", "src/b.rs"]
        );
    }

    #[test]
    fn a_windows_path_is_the_same_citation_as_a_posix_one() {
        assert_eq!(paths_in(r"see `src\store\mod.rs:12`"), ["src/store/mod.rs"]);
        assert_eq!(paths_in("a\r\nsee `src/a.rs`\r\nb\r\n"), ["src/a.rs"]);
    }

    #[test]
    fn what_is_not_a_repository_relative_path_is_refused() {
        for text in [
            "`/etc/hosts`",
            "`C:\\Users\\x\\a.rs`",
            "`C:/Users/x/a.rs`",
            "`~/repo/a.rs`",
            "`https://example.com/a/b.rs`",
            "`../outside/a.rs`",
            "`src/../a.rs`",
            "`src/*.rs`",
            "`src/<dir>/a.rs`",
            "`src/{a,b}/c.rs`",
            "`$HOME/a.rs`",
            "`cargo test --all`",
            "`@scope/package`",
            "`-rf/a.rs`",
            "`host:src/a.rs`",
            "`v1.2.3`",
            "`foo`",
            "`.rs`",
            "`a.exe`",
            "``",
            "an unclosed `src/a.rs",
            "```\n`src/a.rs`\n```",
        ] {
            assert!(paths_in(text).is_empty(), "{text:?} was read as a path");
        }
    }

    #[test]
    fn a_path_outside_backticks_is_not_read() {
        assert!(paths_in("it lives in src/store/mod.rs, plainly").is_empty());
    }

    #[test]
    fn a_present_path_is_present_and_a_deleted_one_is_missing() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir_all(temp.path().join("src")).unwrap();
        std::fs::write(temp.path().join("src").join("here.rs"), "").unwrap();
        std::fs::write(temp.path().join("Cargo.toml"), "").unwrap();

        assert_eq!(judge_path(temp.path(), "src/here.rs"), PathVerdict::Present);
        assert_eq!(judge_path(temp.path(), "Cargo.toml"), PathVerdict::Present);
        assert_eq!(judge_path(temp.path(), "src"), PathVerdict::Present);
        assert_eq!(judge_path(temp.path(), "src/gone.rs"), PathVerdict::Missing);
        assert_eq!(judge_path(temp.path(), "gone/old.rs"), PathVerdict::Missing);
        assert_eq!(judge_path(temp.path(), "src/gone"), PathVerdict::Missing);
        assert_eq!(judge_path(temp.path(), "Gone.toml"), PathVerdict::Missing);
        assert_eq!(
            judge_path(temp.path(), "read/write"),
            PathVerdict::NotAPath,
            "a slash alone, in a directory the repository does not have, is not a path"
        );
    }

    #[test]
    fn git_answers_are_read_by_position_whatever_the_line_ending() {
        let lf = format!("{REAL} commit 245\nabc1234 missing\nabc ambiguous\n");
        assert_eq!(parse_batch_check(&lf, 3).unwrap(), [true, false, true]);
        let crlf = lf.replace('\n', "\r\n");
        assert_eq!(parse_batch_check(&crlf, 3).unwrap(), [true, false, true]);
        assert!(parse_batch_check(&lf, 2).is_err());
        assert!(parse_batch_check("", 1).is_err());
        assert_eq!(parse_batch_check("", 0).unwrap(), Vec::<bool>::new());
    }

    /// One process for every hash, which is what makes a thousand memories a
    /// matter of milliseconds and not of a thousand `git` spawns.
    #[test]
    fn every_hash_of_a_project_is_asked_of_git_in_one_process() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let git = |arguments: &[&str]| {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
                .args(arguments)
                .stdout(std::process::Stdio::null())
                .status()
                .expect("these tests need git on the PATH");
            assert!(status.success(), "git {arguments:?}");
        };
        git(&["init", "--quiet"]);
        git(&["commit", "--quiet", "--allow-empty", "-m", "x"]);

        let memories: Vec<(i64, String)> =
            (0..50).map(|n| (n, format!("fixed in {n:040x}"))).collect();
        let before = crate::project::GIT_SPAWNS.with(std::cell::Cell::get);
        let assessment = assess(root, &memories).unwrap();
        let spawned = crate::project::GIT_SPAWNS.with(std::cell::Cell::get) - before;

        assert_eq!(assessment.missing_commits, 50);
        assert_eq!(assessment.stale.len(), 50);
        assert_eq!(spawned, 1);
    }

    #[test]
    fn the_report_names_ten_ids_and_counts_the_rest() {
        let assessment = Assessment {
            memories_read: 40,
            commits: 3,
            missing_commits: 2,
            paths: 5,
            missing_paths: 1,
            unreadable_paths: 0,
            stale: (1..=EXAMPLE_IDS as i64 + 3).collect(),
        };
        let text = assessment.describe("demo");
        assert!(text.contains("13 of 40 memories of project demo"), "{text}");
        assert!(
            text.contains("1, 2, 3, 4, 5, 6, 7, 8, 9, 10, and 3 more"),
            "{text}"
        );
        assert!(!text.contains("11,"), "{text}");
        assert!(text.contains("another branch"), "{text}");
    }
}
