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
    // The index after the last character that is not a letter, which is that
    // character's index plus its own width. Adding one assumed it was ASCII, and
    // a Spanish memory ("revisión commit abc1234") puts `ó` there: the slice
    // landed inside it and took the whole command down.
    let word_start = without_colon
        .char_indices()
        .rev()
        .find(|(_, character)| !character.is_ascii_alphabetic())
        .map_or(0, |(at, character)| at + character.len_utf8());
    let (before_word, word) = without_colon.split_at(word_start);
    let glued = before_word
        .chars()
        .next_back()
        .is_some_and(|character| character.is_alphanumeric() || character == '_');
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
///
/// A name with no directory in it is written that way because the memory was
/// about the file and not its place: measured on a real store, 115 of the
/// roughly 190 paths that did not exist where written were such names (`mod.rs`,
/// `store.rs`), most of them present further down. It resolves at the root, as
/// before, or when `tracked_names` holds a file of that name anywhere.
pub fn judge_path(root: &Path, path: &str, tracked_names: Option<&HashSet<String>>) -> PathVerdict {
    let relative: PathBuf = path.split('/').collect();
    match root.join(&relative).try_exists() {
        Ok(true) => PathVerdict::Present,
        Ok(false)
            if !path.contains('/') && tracked_names.is_some_and(|names| names.contains(path)) =>
        {
            PathVerdict::Present
        }
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
        &[0],
    )?;
    parse_batch_check(&output, hashes.len())
}

/// Which of `paths` the repository's ignore rules cover, asked of git in one
/// process.
///
/// `check-ignore` exits 1 when none is ignored, which is an answer and not a
/// failure; anything else is git not answering. Names are NUL-separated both
/// ways, so a name git would otherwise quote comes back as it went in. A path
/// that is tracked is never reported, which is what keeps a committed file under
/// an ignored-looking directory a citation.
pub fn ignored_paths(root: &Path, paths: &[String]) -> Result<HashSet<String>, String> {
    if paths.is_empty() {
        return Ok(HashSet::new());
    }
    let mut input = paths.join("\0");
    input.push('\0');
    let output = crate::project::run_git_with_input(
        root,
        &["check-ignore", "--stdin", "-z"],
        input,
        GIT_TIMEOUT,
        &[0, 1],
    )?;
    Ok(output
        .split('\0')
        .filter(|name| !name.is_empty())
        .map(|name| name.replace('\\', "/"))
        .collect())
}

/// The base name of every file the repository tracks, from one `ls-files`.
///
/// Separators are normalised before the base name is taken, so a listing with
/// either one gives the same set.
pub fn tracked_base_names(root: &Path) -> Result<HashSet<String>, String> {
    let output = crate::project::run_git_with_input(
        root,
        &["ls-files", "-z"],
        String::new(),
        GIT_TIMEOUT,
        &[0],
    )?;
    Ok(base_names(&output))
}

/// The base names in a NUL-separated listing, whichever separator its paths use.
fn base_names(listing: &str) -> HashSet<String> {
    listing
        .split('\0')
        .filter(|name| !name.is_empty())
        .filter_map(|name| {
            name.replace('\\', "/")
                .rsplit('/')
                .next()
                .map(str::to_owned)
        })
        .collect()
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
    /// Cited paths the repository's ignore rules cover, which are not counted.
    pub ignored_paths: usize,
    /// Why git could not be asked about ignored paths, when it could not.
    pub ignore_rules_unreadable: Option<String>,
    /// Why git could not list the tracked files, when it was needed and could not.
    pub tracked_files_unreadable: Option<String>,
    /// Every memory with at least one missing citation, smallest id first.
    pub stale: Vec<i64>,
}

/// Tests everything `memories` cite against the checkout at `root`.
///
/// Fails, and says why, when git could not be asked: a count of commits that
/// did not resolve is only a count if git answered, and one that guessed "none"
/// would read as a clean store.
pub fn assess(root: &Path, memories: &[(i64, String)]) -> Result<Assessment, String> {
    // One memory's text must not be able to take the report down. Extraction is
    // written to be panic-free by construction; this is the net under it, so a
    // text nobody thought of makes the check say it could not run, and why.
    let mut cited: Vec<(i64, Citations)> = Vec::new();
    for (id, text) in memories {
        let citations = std::panic::catch_unwind(|| extract(text))
            .map_err(|_| format!("the text of memory {id} could not be read for citations"))?;
        if !citations.is_empty() {
            cited.push((*id, citations));
        }
    }

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

    // What git ignores is a build output or a scratch file, not a statement
    // about the repository's content: `target/release/leteo.exe` is never in a
    // checkout and was counted stale on a real store. If git cannot say, the
    // paths are judged as they stand and the report says it could not.
    let (ignored, ignore_rules_unreadable) = match ignored_paths(root, &paths) {
        Ok(ignored) => (ignored, None),
        Err(reason) => (HashSet::new(), Some(reason)),
    };
    let considered: Vec<&String> = paths.iter().filter(|p| !ignored.contains(*p)).collect();
    // The listing of tracked files is only worth a process when some bare name
    // is not at the root, which is what it exists to look further for.
    let wants_tracked = considered
        .iter()
        .any(|path| !path.contains('/') && judge_path(root, path, None) == PathVerdict::Missing);
    let (tracked, tracked_files_unreadable) = if wants_tracked {
        match tracked_base_names(root) {
            Ok(names) => (Some(names), None),
            Err(reason) => (None, Some(reason)),
        }
    } else {
        (None, None)
    };
    let verdicts: HashMap<&str, PathVerdict> = considered
        .iter()
        .map(|path| (path.as_str(), judge_path(root, path, tracked.as_ref())))
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
        paths: considered
            .len()
            .saturating_sub(counted(PathVerdict::NotAPath)),
        missing_paths: counted(PathVerdict::Missing),
        unreadable_paths: counted(PathVerdict::Unreadable),
        ignored_paths: ignored.len(),
        ignore_rules_unreadable,
        tracked_files_unreadable,
        stale: stale.into_iter().collect(),
    })
}

impl Assessment {
    /// The sentence `doctor` reports, naming no more than [`EXAMPLE_IDS`] ids.
    pub fn describe(&self, project: &str) -> String {
        let mut notes = String::new();
        if self.ignored_paths > 0 {
            notes.push_str(&format!(
                "{} cited path(s) that git ignores were skipped. ",
                self.ignored_paths
            ));
        }
        if let Some(reason) = &self.ignore_rules_unreadable {
            notes.push_str(&format!(
                "The ignore rules could not be read ({reason}), so paths git ignores were judged like any other. "
            ));
        }
        if let Some(reason) = &self.tracked_files_unreadable {
            notes.push_str(&format!(
                "The tracked files could not be listed ({reason}), so a file name with no directory was looked for at the repository root only. "
            ));
        }
        let limits = format!(
            "{notes}Nothing was hidden, changed or written. A commit resolves whenever git holds the object, reachable or not, so a dangling one passes, and a shallow clone cannot see older history; paths are looked for in the current checkout only, so one that exists on another branch reads as missing, and a file name with no directory resolves when the repository root has it or any tracked file carries that name"
        );
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

    /// The shape that panicked: a multi-byte letter directly before the blank
    /// that precedes a hash, so the look-behind for the word `commit` reads
    /// across it.
    #[test]
    fn a_multi_byte_character_before_a_commit_word_or_a_hash_is_read_not_cut() {
        assert_eq!(commits_in("revisión commit a1b2c3d"), ["a1b2c3d"]);
        assert_eq!(commits_in("el commit é1b2c3d4"), Vec::<String>::new());
        assert_eq!(commits_in("añadió: a1b2c3d4"), Vec::<String>::new());
        assert_eq!(commits_in("ñcommit a1b2c3d"), Vec::<String>::new());
        assert_eq!(commits_in("éMerge a1b2c3d"), Vec::<String>::new());
        assert_eq!(commits_in("é commit: a1b2c3d"), ["a1b2c3d"]);
        assert_eq!(commits_in("日本語 sha a1b2c3d"), ["a1b2c3d"]);
        assert_eq!(commits_in("🎉 commit a1b2c3d"), ["a1b2c3d"]);
        assert_eq!(commits_in("🎉 a1b2c3d"), Vec::<String>::new());
    }

    #[test]
    fn multi_byte_text_around_every_kind_of_citation_leaves_it_intact() {
        let hash = "0123456789abcdef0123456789abcdef01234567";
        for (before, after) in [("é", "é"), ("🎉", "🎉"), ("日", "本"), ("", "ñ"), ("ñ", "")]
        {
            let full = format!("{before} {hash} {after}");
            assert_eq!(commits_in(&full), [hash], "{full:?}");
            let tight = format!("{before}`a1b2c3d`{after}");
            assert_eq!(commits_in(&tight), ["a1b2c3d"], "{tight:?}");
            let worded = format!("{before} commit a1b2c3d{after}");
            assert_eq!(commits_in(&worded), ["a1b2c3d"], "{worded:?}");
            let path = format!("revisión{before} `src/store/search.rs:12` añadió{after}");
            assert_eq!(paths_in(&path), ["src/store/search.rs"], "{path:?}");
            let inside = format!("`src/{before}x/a.rs{after}`");
            let _ = paths_in(&inside);
        }
        assert_eq!(
            paths_in("`src/ñandú/a.rs` y `日本/b.rs:3-4`"),
            ["src/ñandú/a.rs", "日本/b.rs"]
        );
    }

    #[test]
    fn a_text_that_ends_at_a_citation_or_at_a_backtick_is_read() {
        let hash = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(commits_in(&format!("é{hash}")), [hash]);
        assert_eq!(commits_in(&format!("é {hash}")), [hash]);
        assert_eq!(commits_in("é commit a1b2c3d"), ["a1b2c3d"]);
        assert!(commits_in("é commit ").is_empty());
        assert!(commits_in("é `").is_empty());
        assert!(commits_in("commit").is_empty());
        assert!(commits_in("").is_empty());
        assert!(paths_in("é `").is_empty());
        assert!(paths_in("`").is_empty());
        assert!(paths_in("ñ`src/a.rs").is_empty());
        assert_eq!(paths_in("é `src/a.rs`"), ["src/a.rs"]);
    }

    #[test]
    fn an_unclosed_backtick_or_fence_followed_by_multi_byte_text_is_read() {
        assert!(paths_in("abre `src/a.rs y sigue con añadió 日本 🎉").is_empty());
        assert_eq!(
            paths_in("`src/a.rs` luego `src/b.rs y revisión"),
            ["src/a.rs"]
        );
        assert!(paths_in("```\n`src/a.rs` añadió\nsin cerrar 日本 🎉").is_empty());
        assert!(
            paths_in("```rust\nlet é = 1;\n```\n`src/ok.rs` ñ").contains(&"src/ok.rs".to_owned())
        );
        assert!(commits_in("```\ncommit é a1b2c3d\nsin cerrar 🎉").is_empty());
    }

    #[test]
    fn a_text_that_cannot_be_read_says_so_instead_of_panicking() {
        let temp = TempDir::new().unwrap();
        let memories = vec![(1, "revisión commit a1b2c3d 🎉 `src/ñ.rs`".to_owned())];
        // Not a repository: git is asked only if a hash is found, and it is,
        // so the answer is an error or a count — never a panic.
        let _ = assess(temp.path(), &memories);
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

        let judged = |path: &str| judge_path(temp.path(), path, None);
        assert_eq!(judged("src/here.rs"), PathVerdict::Present);
        assert_eq!(judged("Cargo.toml"), PathVerdict::Present);
        assert_eq!(judged("src"), PathVerdict::Present);
        assert_eq!(judged("src/gone.rs"), PathVerdict::Missing);
        assert_eq!(judged("gone/old.rs"), PathVerdict::Missing);
        assert_eq!(judged("src/gone"), PathVerdict::Missing);
        assert_eq!(judged("Gone.toml"), PathVerdict::Missing);
        assert_eq!(
            judged("read/write"),
            PathVerdict::NotAPath,
            "a slash alone, in a directory the repository does not have, is not a path"
        );
    }

    #[test]
    fn a_bare_name_resolves_when_any_tracked_file_carries_it() {
        let temp = TempDir::new().unwrap();
        let tracked: HashSet<String> = ["mod.rs".to_owned()].into();
        assert_eq!(
            judge_path(temp.path(), "mod.rs", Some(&tracked)),
            PathVerdict::Present
        );
        assert_eq!(
            judge_path(temp.path(), "other.rs", Some(&tracked)),
            PathVerdict::Missing
        );
        assert_eq!(
            judge_path(temp.path(), "src/mod.rs", Some(&tracked)),
            PathVerdict::Missing,
            "a path with a directory is not rescued by a file of that name elsewhere"
        );
    }

    /// A repository with a nested `mod.rs`, an ignore rule, and an untracked
    /// file at the root.
    fn repository_for_names(root: &Path) {
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
        std::fs::create_dir_all(root.join("src").join("deep").join("nested")).unwrap();
        std::fs::write(
            root.join("src").join("deep").join("nested").join("mod.rs"),
            "",
        )
        .unwrap();
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        std::fs::write(root.join("untracked.md"), "").unwrap();
        // The build directory exists, as it does in a real checkout, so a
        // cited file inside it that is not there reads as missing unless the
        // ignore rules are consulted.
        std::fs::create_dir_all(root.join("target")).unwrap();
        git(&["add", "src", ".gitignore"]);
        git(&["commit", "--quiet", "-m", "x"]);
    }

    #[test]
    fn names_and_ignored_paths_are_judged_by_what_git_tracks_and_ignores() {
        let temp = TempDir::new().unwrap();
        repository_for_names(temp.path());
        let memories: Vec<(i64, String)> = vec![
            (1, "see `mod.rs`".to_owned()),
            (2, "see `nothing.rs`".to_owned()),
            (3, "built `target/release/leteo.exe`".to_owned()),
            (4, "see `untracked.md`".to_owned()),
            (5, r"see `src\deep\nested\mod.rs`".to_owned()),
            (6, "see `src/gone.rs`".to_owned()),
        ];
        let assessment = assess(temp.path(), &memories).unwrap();

        assert_eq!(assessment.stale, [2, 6], "{assessment:?}");
        assert_eq!(assessment.ignored_paths, 1);
        assert_eq!(
            assessment.paths, 5,
            "the ignored path is not counted at all"
        );
        assert_eq!(assessment.missing_paths, 2);
        assert!(assessment.ignore_rules_unreadable.is_none());
        assert!(assessment.tracked_files_unreadable.is_none());
        assert!(
            assessment
                .describe("p")
                .contains("1 cited path(s) that git ignores were skipped")
        );
    }

    #[test]
    fn a_listing_is_reduced_to_base_names_whatever_its_separators() {
        let names = base_names("src/a/mod.rs\0src\\b\\store.rs\0top.md\0");
        let expected: HashSet<String> = ["mod.rs", "store.rs", "top.md"].map(str::to_owned).into();
        assert_eq!(names, expected);
    }

    #[test]
    fn no_path_being_ignored_is_an_answer_and_not_a_failure() {
        let temp = TempDir::new().unwrap();
        repository_for_names(temp.path());
        let assessment = assess(temp.path(), &[(1, "see `src/gone.rs`".to_owned())]).unwrap();
        assert!(
            assessment.ignore_rules_unreadable.is_none(),
            "{assessment:?}"
        );
        assert_eq!(assessment.ignored_paths, 0);
    }

    #[test]
    fn git_not_answering_is_said_and_paths_are_judged_as_they_stand() {
        let temp = TempDir::new().unwrap();
        // Not a repository: both questions fail, and neither failure is a pass.
        let memories = vec![(1, "see `mod.rs` and `target/x.exe`".to_owned())];
        let assessment = assess(temp.path(), &memories).unwrap();
        assert!(assessment.ignore_rules_unreadable.is_some());
        assert!(assessment.tracked_files_unreadable.is_some());
        assert_eq!(assessment.stale, [1]);
        let text = assessment.describe("p");
        assert!(text.contains("ignore rules could not be read"), "{text}");
        assert!(text.contains("tracked files could not be listed"), "{text}");
    }

    #[test]
    fn the_whole_check_spawns_a_fixed_number_of_git_processes() {
        let temp = TempDir::new().unwrap();
        repository_for_names(temp.path());
        let few: Vec<(i64, String)> = vec![
            (1, format!("fixed in {}", "0".repeat(40))),
            (2, "see `nothing.rs` and `target/a.exe`".to_owned()),
        ];
        let many: Vec<(i64, String)> = (0..60)
            .flat_map(|n| {
                [
                    (n * 2, format!("fixed in {n:040x}")),
                    (n * 2 + 1, format!("see `gone{n}.rs` and `target/{n}.exe`")),
                ]
            })
            .collect();

        let spawned = |memories: &[(i64, String)]| {
            let before = crate::project::GIT_SPAWNS.with(std::cell::Cell::get);
            assess(temp.path(), memories).unwrap();
            crate::project::GIT_SPAWNS.with(std::cell::Cell::get) - before
        };
        assert_eq!(
            spawned(&[(1, "see `src/gone.rs`".to_owned())]),
            1,
            "the tracked files are listed only when a bare name needs them"
        );
        assert_eq!(spawned(&few), 3, "cat-file, check-ignore, ls-files");
        assert_eq!(
            spawned(&many),
            3,
            "the same however many memories there are"
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
            ignored_paths: 0,
            ignore_rules_unreadable: None,
            tracked_files_unreadable: None,
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
