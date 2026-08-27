use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub const SOURCE_CONFIG: &str = "config";
pub const SOURCE_GIT_REMOTE: &str = "git_remote";
pub const SOURCE_GIT_ROOT: &str = "git_root";
pub const SOURCE_GIT_CHILD: &str = "git_child";
pub const SOURCE_DIR_BASENAME: &str = "dir_basename";
pub const SOURCE_AMBIGUOUS: &str = "ambiguous";
pub const SOURCE_PROCESS_OVERRIDE: &str = "process_override";

/// What every entry point says when a project name is missing.
///
/// Four modules validated this independently and had settled on two wordings
/// for the same condition, so which one a user saw depended on whether they
/// came through the CLI, an MCP tool, the store, or sync.
pub const EMPTY_NAME: &str = "project name cannot be empty";

const CONFIG_DIRECTORY: &str = ".leteo";
const CONFIG_FILE: &str = "config.json";
const GIT_TIMEOUT: Duration = Duration::from_secs(2);
const CHILD_SCAN_TIMEOUT: Duration = Duration::from_millis(200);
/// How many directories a scan for child repositories looks inside.
///
/// It stopped at twenty, and separately the scan stopped as soon as it had
/// found two repositories — which was right while the answer was only "is this
/// directory ambiguous", and wrong the moment the same list became the choices
/// an agent offers and the whitelist a replay is checked against. On a real
/// workspace of 56 directories holding 27 repositories, an ambiguous save
/// offered two of them: the two first alphabetically, neither one anybody
/// works in. Answering "leteo" — 32nd by name — came back
/// `invalid_project_choice`, so from that directory a memory could be filed
/// only under a project its owner does not use.
///
/// Reading all 56 and asking each for a `.git` costs 0.3 ms, so what this
/// number was protecting is not the ordinary case. The deadline beside it is
/// the real guard, and it is what a pathological directory runs into; this
/// bounds the work when the deadline has not yet been reached, and two hundred
/// covers a workspace nobody would call unusual.
const CHILD_SCAN_LIMIT: usize = 200;

/// A complete, MCP-safe project detection result.
///
/// Detection problems are represented by `error_hint` instead of being
/// returned as errors, so callers can always serialize and return this value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectDetection {
    pub project: String,
    #[serde(rename = "project_source")]
    pub source: String,
    #[serde(rename = "project_path")]
    pub path: String,
    #[serde(default)]
    pub available_projects: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_hint: Option<String>,
}

/// What a detection says when the scan for sibling repositories ran out of
/// time.
///
/// Two sentences rather than one, because the two cases are not the same
/// answer. With two repositories already found the verdict is certain and only
/// the list is short; with one or none, nothing is known at all and the
/// basename is a guess. Both are `warning`s rather than silence — see
/// `detect_project`, where the absence of either was the defect.
pub const SCAN_TRUNCATED_LIST: &str =
    "the scan for repositories here ran out of time, so this list may be missing some";
pub const SCAN_UNFINISHED: &str = "the scan for repositories here ran out of time, so this project name is a guess from the \
     directory name and the directory may in fact hold several";

impl ProjectDetection {
    /// The warning that says detection could not finish, if that is what this
    /// one carries.
    ///
    /// `warning` holds three different things and only two of them are a
    /// problem. The third — that a single child repository was promoted — is
    /// detection working, and a caller that repeated it would put a line in
    /// front of somebody at every session in such a directory, which is how a
    /// warnings list stops being read. Asked here rather than by each caller,
    /// so the two that matter cannot be listed differently in two places.
    pub fn scan_warning(&self) -> Option<&str> {
        self.warning
            .as_deref()
            .filter(|warning| [SCAN_TRUNCATED_LIST, SCAN_UNFINISHED].contains(warning))
    }
}

/// Why an existing name was offered as a match, in the order they are tried.
pub const MATCH_CASE_INSENSITIVE: &str = "case-insensitive";
pub const MATCH_SUBSTRING: &str = "substring";
pub const MATCH_LEVENSHTEIN: &str = "levenshtein";

/// An existing project name that resembles a queried name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectMatch {
    pub name: String,
    pub match_type: String,
    pub distance: usize,
}

/// Finds existing project names that resemble `name` through case-insensitive
/// equality, substring containment, or a bounded edit distance. Identical names
/// are never reported.
pub fn find_similar(name: &str, existing: &[String], max_distance: usize) -> Vec<ProjectMatch> {
    let lowered = name.trim().to_lowercase();
    let effective_max = max_distance.min((lowered.chars().count() / 2).max(1));

    let mut case_matches = Vec::new();
    let mut substring_matches = Vec::new();
    let mut distance_matches = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    for candidate in existing {
        if candidate == name || !seen.insert(candidate.clone()) {
            continue;
        }
        let candidate_lowered = candidate.trim().to_lowercase();
        if candidate_lowered == lowered {
            case_matches.push(ProjectMatch {
                name: candidate.clone(),
                match_type: MATCH_CASE_INSENSITIVE.to_owned(),
                distance: 0,
            });
            continue;
        }
        if lowered.chars().count() >= 3
            && (candidate_lowered.contains(&lowered) || lowered.contains(&candidate_lowered))
        {
            substring_matches.push(ProjectMatch {
                name: candidate.clone(),
                match_type: MATCH_SUBSTRING.to_owned(),
                distance: 0,
            });
            continue;
        }
        let distance = levenshtein(&lowered, &candidate_lowered);
        if distance <= effective_max {
            distance_matches.push(ProjectMatch {
                name: candidate.clone(),
                match_type: MATCH_LEVENSHTEIN.to_owned(),
                distance,
            });
        }
    }

    distance_matches.sort_by(|left, right| {
        left.distance
            .cmp(&right.distance)
            .then_with(|| left.name.cmp(&right.name))
    });
    case_matches.extend(substring_matches);
    case_matches.extend(distance_matches);
    case_matches
}

/// Computes the Levenshtein distance with two rolling rows.
fn levenshtein(left: &str, right: &str) -> usize {
    let mut left: Vec<char> = left.chars().collect();
    let mut right: Vec<char> = right.chars().collect();
    if left.is_empty() {
        return right.len();
    }
    if right.is_empty() {
        return left.len();
    }
    if left.len() > right.len() {
        std::mem::swap(&mut left, &mut right);
    }

    let mut previous: Vec<usize> = (0..=left.len()).collect();
    let mut current = vec![0; left.len() + 1];
    for (row, right_char) in right.iter().enumerate() {
        current[0] = row + 1;
        for (column, left_char) in left.iter().enumerate() {
            let cost = usize::from(left_char != right_char);
            current[column + 1] = (previous[column + 1] + 1)
                .min(current[column] + 1)
                .min(previous[column] + cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[left.len()]
}

#[derive(Deserialize)]
struct ProjectConfig {
    project_name: String,
}

/// Detects the project associated with `directory` without ever returning an
/// error. An empty path means the current working directory.
pub fn detect_project(directory: impl AsRef<Path>) -> ProjectDetection {
    detect_project_within(directory, CHILD_SCAN_TIMEOUT)
}

/// [`detect_project`] with the scan budget named.
///
/// Only the default is used in anger. A caller that says the budget is a test
/// that wants a scan which cannot lose its race with a loaded machine — the
/// guard for an ambiguous directory did exactly that, and failed twice in
/// fourteen runs of the suite for want of a few milliseconds it had no reason
/// to be short of.
pub(crate) fn detect_project_within(
    directory: impl AsRef<Path>,
    budget: Duration,
) -> ProjectDetection {
    let requested = directory.as_ref();
    let requested = if requested.as_os_str().is_empty() {
        Path::new(".")
    } else {
        requested
    };
    let cwd = canonical_path(requested);
    let git_root = detect_git_root(&cwd);

    if let Some(result) = detect_from_config(&cwd, git_root.as_deref()) {
        return result;
    }

    if let Some(root) = git_root {
        if let Some(name) = detect_git_remote(&root) {
            return successful_detection(name, SOURCE_GIT_REMOTE, &root);
        }
        return successful_detection(path_basename(&root), SOURCE_GIT_ROOT, &root);
    }

    let (children, timed_out) = scan_child_repositories(&cwd, budget);
    detection_from_children(&cwd, &children, timed_out)
}

/// What a directory's own repositories say about which project it is.
///
/// Split out of [`detect_project`] so the four answers can be asked for
/// directly. The interesting two happen only when a filesystem scan runs out of
/// time, and a test that reproduces that by racing a deadline is a test that
/// checks nothing on the runs where the machine is quick — which is precisely
/// how the missing half of this went unnoticed.
fn detection_from_children(cwd: &Path, children: &[PathBuf], timed_out: bool) -> ProjectDetection {
    // A scan that ran out of time used to skip this whole decision and fall
    // through to the basename, which reports a check that could not run as a
    // detection that succeeded. Nothing said so: no warning, no hint, and — the
    // expensive part — `SOURCE_DIR_BASENAME` is a *successful* source, so the
    // guard that refuses a write into an ambiguous directory never fired.
    // Standing in a workspace of twenty-seven repositories, a slow enough scan
    // filed memories under the name of the folder containing them, silently,
    // and the ambiguity protection disappeared exactly when the machine was
    // least able to spare the attention to notice.
    match (children, timed_out) {
        // Two or more is ambiguous whether or not the scan finished: reaching
        // more entries cannot turn two repositories into one. What truncation
        // costs here is the completeness of the list, not the verdict, and the
        // list is what an agent offers and replays a choice against — so it
        // says that it may be short rather than presenting a partial whitelist
        // as the whole of it.
        ([_, _, ..], _) => {
            let available_projects = children
                .iter()
                .map(|child| normalize_project_name(&path_basename(child)))
                .collect();
            ProjectDetection {
                project: String::new(),
                source: SOURCE_AMBIGUOUS.to_owned(),
                path: path_string(cwd),
                available_projects,
                warning: timed_out.then(|| SCAN_TRUNCATED_LIST.to_owned()),
                error_hint: Some(
                    "ambiguous project: multiple git repositories found in cwd".to_owned(),
                ),
            }
        }
        ([child], false) => {
            let project = normalize_project_name(&path_basename(child));
            let mut result = successful_detection(&project, SOURCE_GIT_CHILD, child);
            result.warning = Some(format!("auto-promoted child repository: {project}"));
            result
        }
        // Out of time having found one or none, which is the case that knows
        // nothing: the entry it did not reach could have been the second
        // repository that makes this directory ambiguous. The basename is still
        // the best available answer — most directories hold no repositories at
        // all — but it is offered as a guess rather than as a finding, and a
        // single child is not promoted on the strength of a scan that never saw
        // whether it had company.
        (_, true) => {
            let mut result = successful_detection(path_basename(cwd), SOURCE_DIR_BASENAME, cwd);
            result.warning = Some(SCAN_UNFINISHED.to_owned());
            result
        }
        ([], false) => successful_detection(path_basename(cwd), SOURCE_DIR_BASENAME, cwd),
    }
}

/// Detects the current process directory. Failure to obtain it is folded into
/// the normal basename fallback by `detect_project`.
/// Answered once per process, because the answer cannot change within one.
///
/// Detection shells out to `git rev-parse --show-toplevel`, and often to
/// `git remote get-url origin` after it. Measured inside a running MCP server
/// against a real store, that costs about 13 ms a call — `mem_current_project`
/// took 152 ms over ten calls where `mem_search`, which reads the database,
/// took 19 ms.
///
/// It was paid per call, and the write path calls it: every `mem_save` without
/// a session id detects the project again. An agent saving ten memories in a
/// session spawned twenty git processes to be told the same thing twenty times.
///
/// Caching is safe because the input is the process's working directory and
/// Leteo never changes it — no `set_current_dir` anywhere in the tree. A hook
/// or a CLI command is one shot; an MCP server is launched in a directory and
/// stays there. `detect_project` still takes a path and is not cached, which is
/// what the tests use and what a caller with a directory in hand should use.
static CURRENT_PROJECT: std::sync::OnceLock<ProjectDetection> = std::sync::OnceLock::new();

pub fn detect_current_project() -> ProjectDetection {
    CURRENT_PROJECT
        .get_or_init(detect_current_project_uncached)
        .clone()
}

fn detect_current_project_uncached() -> ProjectDetection {
    match std::env::current_dir() {
        Ok(directory) => detect_project(directory),
        Err(error) => {
            let mut result = detect_project("");
            result.error_hint = Some(format!("failed to resolve current directory: {error}"));
            result
        }
    }
}

fn successful_detection(project: impl AsRef<str>, source: &str, path: &Path) -> ProjectDetection {
    ProjectDetection {
        project: normalize_project_name(project.as_ref()),
        source: source.to_owned(),
        path: path_string(path),
        available_projects: Vec::new(),
        warning: None,
        error_hint: None,
    }
}

fn detect_from_config(cwd: &Path, git_root: Option<&Path>) -> Option<ProjectDetection> {
    let Some(root) = git_root else {
        return read_config_at(cwd);
    };

    let mut current = cwd.to_path_buf();
    loop {
        if let Some(result) = read_config_at(&current) {
            return Some(result);
        }
        if same_path(&current, root) {
            break;
        }
        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent.to_path_buf();
    }
    None
}

fn read_config_at(project_directory: &Path) -> Option<ProjectDetection> {
    let config_path = project_directory.join(CONFIG_DIRECTORY).join(CONFIG_FILE);
    let contents = match fs::read(&config_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            return Some(config_error(
                project_directory,
                format!("cannot read {}: {error}", config_path.display()),
            ));
        }
    };

    let config: ProjectConfig = match serde_json::from_slice(&contents) {
        Ok(config) => config,
        Err(error) => {
            return Some(config_error(
                project_directory,
                format!("cannot parse {}: {error}", config_path.display()),
            ));
        }
    };
    let project = match validate_config_project_name(&config.project_name) {
        Ok(project) => project,
        Err(error) => return Some(config_error(project_directory, error)),
    };

    Some(successful_detection(
        project,
        SOURCE_CONFIG,
        project_directory,
    ))
}

fn config_error(project_directory: &Path, detail: String) -> ProjectDetection {
    ProjectDetection {
        project: String::new(),
        source: SOURCE_CONFIG.to_owned(),
        path: path_string(project_directory),
        available_projects: Vec::new(),
        warning: None,
        error_hint: Some(format!("invalid .leteo/config.json: {detail}")),
    }
}

fn validate_config_project_name(project_name: &str) -> Result<String, String> {
    let trimmed = project_name.trim();
    if trimmed.is_empty() {
        return Err("project_name is required".to_owned());
    }
    if trimmed.contains(['/', '\\']) {
        return Err("project_name must be a name, not a path".to_owned());
    }
    if trimmed.chars().any(char::is_control) {
        return Err("project_name contains control characters".to_owned());
    }
    Ok(normalize_project_name(trimmed))
}

/// Whether git has been pointed somewhere other than the filesystem says.
///
/// `GIT_DIR` and `GIT_WORK_TREE` override where git thinks a repository is, and
/// no amount of looking at directories can see that. When either is set the
/// shortcuts below are skipped and git is asked, because being fast about the
/// wrong repository files every memory under the wrong project.
fn git_is_redirected() -> bool {
    std::env::var_os("GIT_DIR").is_some() || std::env::var_os("GIT_WORK_TREE").is_some()
}

/// The repository root, by walking up for `.git` before asking git.
///
/// `git rev-parse --show-toplevel` costs about 20 ms — a process spawn, and git
/// reading its own configuration. Detection runs it on every hook invocation,
/// and `user-prompt-submit` runs on every prompt somebody types, synchronously,
/// before their message is sent. Two thirds of that hook's 67 ms was these two
/// subprocesses.
///
/// Walking up for `.git` is what `--show-toplevel` does. A `.git` that is a
/// *file* rather than a directory is a worktree or a submodule, and its
/// containing directory is still the toplevel git would name, so both count.
///
/// A walk that finds nothing is git's answer too, so it is not asked again.
///
/// The fallback used to run anyway, and it was kept for bare repositories —
/// where there is no `.git` to walk to. That reason was wrong: `git rev-parse
/// --show-toplevel` inside a bare repository fails with *this operation must
/// be run in a work tree*, so the subprocess returned nothing there as well.
/// It bought a process and no answers.
///
/// Measured on this machine, outside any repository: **33 ms with git on the
/// PATH and 9 ms without it**, for the same detection. Three quarters of the
/// call, on a path `user-prompt-submit` runs before the message somebody just
/// typed is sent.
///
/// `GIT_DIR` and `GIT_WORK_TREE` are the real exception — no amount of looking
/// at directories can see them — and they still go to git.
fn detect_git_root(directory: &Path) -> Option<PathBuf> {
    if let Some(root) = git_root_by_walking(directory) {
        return Some(root);
    }
    if !git_is_redirected() {
        return None;
    }
    let root = run_git(directory, &["rev-parse", "--show-toplevel"])?;
    if root.is_empty() {
        None
    } else {
        Some(canonical_path(Path::new(&root)))
    }
}

/// The nearest ancestor holding a `.git`, or nothing.
///
/// Split out from [`detect_git_root`] so it can be held to git's answer
/// directly. Tested through the fallback instead, a broken walk passes: the
/// subprocess quietly supplies the right answer and the shortcut is dead code
/// nobody notices.
///
/// Wants a canonical path. Git walks the physical working directory, so a
/// symlinked path can look repository-less from outside one git is standing
/// in. [`detect_project`] canonicalises before calling, and since the walk is
/// now the only answer there is nothing behind it to cover a caller that does
/// not.
fn git_root_by_walking(directory: &Path) -> Option<PathBuf> {
    if git_is_redirected() {
        return None;
    }
    let mut current = Some(directory);
    while let Some(candidate) = current {
        if candidate.join(".git").exists() {
            return Some(canonical_path(candidate));
        }
        current = candidate.parent();
    }
    None
}

/// The name of the `origin` remote, read from `.git/config` before asking git.
///
/// The same saving as [`detect_git_root`], for the same reason. The URL sits in
/// a plain INI file under `[remote "origin"]`, which is where `git remote
/// get-url origin` reads it from.
///
/// Only `origin`, only `url`, and only the first one — which is what the
/// command returns. A `.git` that is a file points elsewhere for its
/// configuration, so that case is left to git rather than guessed at.
/// What `.git/config` had to say about `origin`.
///
/// The distinction is the whole point: a file that is readable and names no
/// origin has *answered*, and asking git repeats the answer for 22 ms. Only a
/// file that could not be read is a question git still has to settle.
enum OriginUrl {
    Named(String),
    None,
    Unreadable,
}

fn detect_git_remote(git_root: &Path) -> Option<String> {
    let remote = match read_origin_url(git_root) {
        OriginUrl::Named(url) => url,
        // This repository has no origin, and the file said so. Spawning git to
        // hear it again cost 22 ms of every project detection — which is every
        // hook, which is every prompt somebody types. It was invisible because
        // the fallback is only reached when the fast path finds nothing, and
        // "found nothing" was being read as "could not look".
        OriginUrl::None => return None,
        OriginUrl::Unreadable => run_git(git_root, &["remote", "get-url", "origin"])?,
    };
    extract_repository_name(&remote).map(|name| normalize_project_name(&name))
}

fn read_origin_url(git_root: &Path) -> OriginUrl {
    if git_is_redirected() {
        return OriginUrl::Unreadable;
    }
    let config = git_root.join(".git").join("config");
    let Ok(text) = fs::read_to_string(config) else {
        // A `.git` that is a file — a worktree or a submodule — keeps its
        // configuration elsewhere, and that is git's business rather than a
        // path to guess at.
        return OriginUrl::Unreadable;
    };
    let mut inside_origin = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            // Both spellings git writes: `[remote "origin"]`, and the
            // subsection-less form a hand-edited file may carry.
            inside_origin = line.replace(' ', "") == "[remote\"origin\"]";
            continue;
        }
        if !inside_origin {
            continue;
        }
        if let Some((key, value)) = line.split_once('=')
            && key.trim() == "url"
        {
            let value = value.trim();
            return if value.is_empty() {
                OriginUrl::None
            } else {
                OriginUrl::Named(value.to_owned())
            };
        }
    }
    OriginUrl::None
}

// How many git processes this thread has started, counted only under test.
//
// Thread-local rather than static: the harness gives each test its own thread,
// so a counter here measures the test that reads it instead of whatever else
// happened to be running beside it.
#[cfg(test)]
thread_local! {
    static GIT_SPAWNS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn run_git(directory: &Path, arguments: &[&str]) -> Option<String> {
    #[cfg(test)]
    GIT_SPAWNS.with(|count| count.set(count.get() + 1));

    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = command.spawn().ok()?;
    let deadline = Instant::now() + GIT_TIMEOUT;
    // Asked often at first and then rarely, rather than every ten milliseconds.
    //
    // `git rev-parse` answers in about eleven milliseconds here, and a fixed
    // ten-millisecond sleep turns that into twenty: the work finishes early in
    // the second nap and nobody looks until it ends. Starting at a millisecond
    // and doubling costs at most one extra millisecond on a fast answer while
    // still leaving a stuck git to the deadline rather than to a spin.
    let mut nap = Duration::from_millis(1);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(nap);
                nap = (nap * 2).min(Duration::from_millis(10));
            }
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    if !status.success() {
        return None;
    }

    let mut bytes = Vec::new();
    child.stdout.take()?.read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).trim().to_owned())
}

fn extract_repository_name(remote: &str) -> Option<String> {
    let remote = remote.trim().trim_end_matches(['/', '\\']);
    let remote = remote.strip_suffix(".git").unwrap_or(remote);
    remote
        .rsplit(['/', '\\', ':'])
        .find(|part| !part.trim().is_empty())
        .map(|part| part.trim().to_owned())
}

/// The repositories directly inside `directory`, and whether the scan gave up
/// before reaching the end of it.
///
/// The budget is a parameter rather than the constant read inside, so that the
/// truncated answer can be asked for on purpose. It is the branch that decides
/// whether a write is refused as ambiguous, it fired only under load, and a
/// test that has to *race* a deadline to reach it is a test that watches
/// nothing on the runs where it loses — which is how it went unnoticed and how
/// the one guard near it flaked twice in fourteen runs.
fn scan_child_repositories(directory: &Path, budget: Duration) -> (Vec<PathBuf>, bool) {
    let mut entries: Vec<_> = match fs::read_dir(directory) {
        Ok(entries) => entries.filter_map(Result::ok).collect(),
        Err(_) => return (Vec::new(), false),
    };
    entries.sort_by_key(|entry| entry.file_name());

    let deadline = Instant::now() + budget;
    let mut scanned = 0;
    let mut repositories = Vec::new();
    for entry in entries {
        if Instant::now() >= deadline {
            return (repositories, true);
        }
        if scanned >= CHILD_SCAN_LIMIT {
            break;
        }

        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || is_noise_directory(&name) {
            continue;
        }
        scanned += 1;

        let child = entry.path();
        if child.join(".git").exists() {
            repositories.push(canonical_path(&child));
        }
    }
    (repositories, false)
}

fn is_noise_directory(name: &str) -> bool {
    matches!(
        name,
        "node_modules"
            | "vendor"
            | ".venv"
            | "__pycache__"
            | "target"
            | "dist"
            | "build"
            | ".idea"
            | ".vscode"
    )
}

/// A detected project name, spelled the way the store spells one.
///
/// This used to trim and lowercase and stop there, which is most of what
/// `normalize::project` does and not all of it — so a directory called
/// `my--project` was *detected* as `my--project` and *stored* as `my-project`.
/// Both are the same project and every query finds it, because the store
/// normalises again at its own door. What differed was what Leteo said: one
/// answer reported the project as `my--project` in its envelope and as
/// `my-project` on the memory inside it.
///
/// One rule, then. The empty fallback stays here because it is this module's
/// answer to "there was nothing to detect", not a normalisation.
fn normalize_project_name(name: &str) -> String {
    let normalized = crate::memory::normalize::project(name);
    if normalized.is_empty() {
        "unknown".to_owned()
    } else {
        normalized
    }
}

fn path_basename(path: &Path) -> String {
    path.file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn canonical_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let canonical = fs::canonicalize(&absolute).unwrap_or(absolute);
    remove_windows_verbatim_prefix(canonical)
}

fn same_path(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        path_string(left).eq_ignore_ascii_case(&path_string(right))
    } else {
        left == right
    }
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Strips the `\\?\` prefix Windows canonicalization adds. Agent launchers and
/// shells reject verbatim paths, so generated configuration must never hold one.
#[cfg(windows)]
pub(crate) fn remove_windows_verbatim_prefix(path: PathBuf) -> PathBuf {
    let value = path.to_string_lossy();
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = value.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path
}

#[cfg(not(windows))]
pub(crate) fn remove_windows_verbatim_prefix(path: PathBuf) -> PathBuf {
    path
}

#[cfg(test)]
mod tests {
    /// The working directory is asked about once, not once per caller.
    ///
    /// Detection shells out to `git rev-parse --show-toplevel`, and usually to
    /// `git remote get-url origin` after it — measured at **45 ms a call** in
    /// this repository. It was paid per call, and the write path calls it:
    /// every `mem_save` without a session id detected the project again, so an
    /// agent saving ten memories spawned up to twenty git processes to be told
    /// the same thing twenty times.
    ///
    /// Asserted structurally rather than by timing, because the test binary
    /// shares one process and another test may have filled the cell first — a
    /// stopwatch here would measure test ordering. If the memoisation is
    /// removed the cell stays empty and this fails, which is the thing worth
    /// catching.
    #[test]
    fn the_current_project_is_detected_once_per_process() {
        let first = detect_current_project();
        assert!(
            CURRENT_PROJECT.get().is_some(),
            "the answer has to be kept, not recomputed for the next caller"
        );
        assert_eq!(first, detect_current_project());
    }

    use super::*;
    use tempfile::TempDir;

    fn create_config(directory: &Path, body: &str) {
        let config_directory = directory.join(CONFIG_DIRECTORY);
        fs::create_dir_all(&config_directory).unwrap();
        fs::write(config_directory.join(CONFIG_FILE), body).unwrap();
    }

    fn init_git(directory: &Path) -> bool {
        Command::new("git")
            .arg("init")
            .arg("--quiet")
            .arg(directory)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    /// Nothing to find is not worth a process, and a bare repository proves it.
    ///
    /// The fallback to `git rev-parse --show-toplevel` ran whenever the walk
    /// came back empty, and the comment defending it named bare repositories:
    /// no `.git` to walk to, so ask git. Git refuses — *this operation must be
    /// run in a work tree* — and the fallback returned nothing after paying
    /// for a process. Outside a repository that cost 33 ms against 9 ms with
    /// git off the PATH, on the path every prompt takes.
    ///
    /// Both halves are asserted: that the answer is the same one git gives,
    /// and that it was reached without asking. Without the second, re-adding
    /// the subprocess passes.
    #[test]
    fn nothing_to_find_is_not_worth_a_git_process() {
        let temp = TempDir::new().unwrap();
        let plain = canonical_path(temp.path().join("one").join("two"));
        fs::create_dir_all(&plain).unwrap();
        let bare = canonical_path(temp.path());
        let has_git = Command::new("git")
            .args(["init", "--quiet", "--bare"])
            .arg(temp.path().join("bare.git"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());

        // A temporary directory inside somebody's repository would find one,
        // and there would be nothing here to prove.
        if git_root_by_walking(&plain).is_some() {
            return;
        }

        let mut places = vec![plain];
        if has_git {
            places.push(canonical_path(bare.join("bare.git")));
        }

        for place in places {
            // Compared against git rather than against an expectation written
            // here, so it stays honest if git changes its mind about where a
            // repository begins.
            assert_eq!(
                run_git(&place, &["rev-parse", "--show-toplevel"]),
                None,
                "git names no work tree at {}",
                place.display()
            );

            let spawned = GIT_SPAWNS.with(std::cell::Cell::get);
            assert_eq!(detect_git_root(&place), None);
            assert_eq!(
                GIT_SPAWNS.with(std::cell::Cell::get),
                spawned,
                "a walk that reached the root of the filesystem is git's answer \
                 too; asking again costs a process on every prompt"
            );
        }
    }

    /// The filesystem shortcuts answer what git answers.
    ///
    /// Detection used to spawn `git rev-parse --show-toplevel` and
    /// `git remote get-url origin` — about 45 ms together, paid on every hook
    /// invocation, and `user-prompt-submit` runs on every prompt somebody
    /// types before their message is sent. Walking up for `.git` and reading
    /// `.git/config` gives the same answers for a fraction of the cost, and
    /// this is what holds the two together: get the walk or the parse wrong
    /// and memories are filed under the wrong project, silently.
    ///
    /// Compared against git itself rather than against a hardcoded
    /// expectation, so it stays honest if git ever changes its mind.
    #[test]
    fn the_shortcuts_agree_with_git_about_the_root_and_the_remote() {
        let temp = TempDir::new().unwrap();
        let root = canonical_path(temp.path());
        if !init_git(&root) {
            return; // no git on this machine; nothing to compare against
        }
        // A second remote, written before origin, so a parser that takes the
        // first `url` it sees in any section gets a different answer from the
        // one git gives. Without it the fixture cannot tell the two apart.
        assert!(
            Command::new("git")
                .args(["-C", &path_string(&root), "remote", "add", "upstream"])
                .arg("https://github.com/elsewhere/Not-This-One.git")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        );

        assert!(
            Command::new("git")
                .args(["-C", &path_string(&root), "remote", "add", "origin"])
                .arg("https://github.com/someone/Named-By-Remote.git")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        );

        // The root, from a directory well inside the repository — the walk has
        // to climb, and land where git says. Asserted on the walk itself, not
        // on `detect_git_root`: through the fallback a broken walk passes,
        // because the subprocess supplies the answer and the shortcut becomes
        // dead code nobody notices.
        let nested = root.join("src").join("deep").join("deeper");
        fs::create_dir_all(&nested).unwrap();
        let expected = run_git(&nested, &["rev-parse", "--show-toplevel"])
            .map(|found| canonical_path(Path::new(&found)))
            .expect("git names the toplevel");
        assert_eq!(git_root_by_walking(&nested), Some(expected.clone()));
        assert_eq!(detect_git_root(&nested), Some(expected));

        let from_git = run_git(&root, &["remote", "get-url", "origin"]).expect("git names origin");
        assert!(
            matches!(read_origin_url(&root), OriginUrl::Named(url) if url == from_git.trim()),
            "the config answers with the url git gives"
        );
        assert_eq!(
            detect_project(&nested).project,
            "named-by-remote",
            "the whole detection, not just its parts"
        );

        let elsewhere = TempDir::new().unwrap();
        assert!(!elsewhere.path().join(".git").exists());
    }

    /// A `.leteo/config.json` cannot name a project that is not a name.
    ///
    /// The value becomes a directory component and a store key, so a path or
    /// an empty string is not a naming mistake but a way out of the store. A
    /// mutation run found both checks untested: delete either and everything
    /// still passed.
    #[test]
    fn a_configured_project_name_must_be_a_name() {
        for refused in [
            "",
            "   ",
            "	
",
            "../escape",
            r"..\escape",
            "nested/name",
        ] {
            assert!(
                validate_config_project_name(refused).is_err(),
                "{refused:?} was accepted as a project name"
            );
        }
        assert_eq!(
            validate_config_project_name("  My Project  "),
            Ok("my project".to_owned()),
            "an ordinary name is trimmed and normalised, not refused"
        );
    }

    /// A repository with no origin is not asked about twice.
    ///
    /// `read_origin_url` returning nothing used to mean "ask git", and for a
    /// repository that simply has no `origin` — which this one is — that
    /// spawned a subprocess to be told the same thing. It cost **22 ms of
    /// every project detection**, which is every hook, which is every prompt
    /// somebody types, and it hid behind a fallback that only runs when the
    /// fast path finds nothing: "found nothing" was being read as "could not
    /// look". Detection went from 22.53 ms to 0.07 ms.
    ///
    /// The three answers have to stay three. A readable config naming an
    /// origin, a readable config naming none, and a config that could not be
    /// read at all — only the last is a question git still has to settle.
    #[test]
    fn a_readable_config_with_no_origin_answers_for_itself() {
        let temp = TempDir::new().unwrap();
        let root = canonical_path(temp.path());
        let git = root.join(".git");
        fs::create_dir_all(&git).unwrap();

        // No config at all: git may know something this cannot see.
        assert!(matches!(read_origin_url(&root), OriginUrl::Unreadable));

        // A config with no remote in it has answered.
        fs::write(
            git.join("config"),
            "[core]
	repositoryformatversion = 0
	bare = false
",
        )
        .unwrap();
        assert!(
            matches!(read_origin_url(&root), OriginUrl::None),
            "a config that names no origin is an answer, not a gap"
        );

        // And one that names it gives it.
        fs::write(
            git.join("config"),
            "[core]
	bare = false
[remote \"origin\"]
	url = https://example.com/thing.git
",
        )
        .unwrap();
        assert!(
            matches!(read_origin_url(&root), OriginUrl::Named(url) if url.ends_with("thing.git")),
        );
    }

    /// The "no origin" answer does not reach a subprocess.
    ///
    /// Asserted on the source because there is nothing else to assert on: git
    /// returns the same nothing the config did, so behaviour is identical and
    /// only the 22 ms differ. A test that checks the *result* passes either
    /// way — which is exactly how this cost hid in the first place. Timing it
    /// instead would flake on a loaded machine.
    ///
    /// The same shape as the join-order guard in search: when a change costs
    /// only speed, the thing to pin is the code that spends it.
    #[test]
    fn the_no_origin_answer_does_not_reach_git() {
        const SOURCE: &str = include_str!("project.rs");
        let start = SOURCE
            .find("fn detect_git_remote(")
            .expect("detect_git_remote is in this file");
        let body = &SOURCE[start..];
        let end = body
            .find(
                "
}
",
            )
            .map_or(body.len(), |at| at + 2);
        let body = &body[..end];

        let none_arm = body
            .find("OriginUrl::None =>")
            .expect("the three answers are matched on");
        let unreadable_arm = body
            .find("OriginUrl::Unreadable =>")
            .expect("the three answers are matched on");
        let none_line = body[none_arm..]
            .lines()
            .next()
            .expect("the arm has a body on its line");

        assert!(
            !none_line.contains("run_git"),
            "a config that answered must not be asked again: {none_line}"
        );
        assert!(
            body[unreadable_arm..]
                .lines()
                .next()
                .is_some_and(|line| line.contains("run_git")),
            "and a config that could not be read still has to be"
        );
    }

    #[test]
    fn config_takes_precedence_and_uses_leteo_branding() {
        let temp = TempDir::new().unwrap();
        create_config(temp.path(), r#"{"project_name":" Canonical App "}"#);

        let result = detect_project(temp.path());

        assert_eq!(result.project, "canonical app");
        assert_eq!(result.source, SOURCE_CONFIG);
        assert_eq!(result.path, path_string(&canonical_path(temp.path())));
        assert!(result.error_hint.is_none());
    }

    #[test]
    fn nearest_config_is_found_without_leaving_the_git_root() {
        let temp = TempDir::new().unwrap();
        if !init_git(temp.path()) {
            return;
        }
        create_config(temp.path(), r#"{"project_name":"repo-root"}"#);
        let package = temp.path().join("packages").join("api");
        let nested = package.join("src");
        fs::create_dir_all(&nested).unwrap();
        create_config(&package, r#"{"project_name":"api-service"}"#);

        let result = detect_project(&nested);

        assert_eq!(result.project, "api-service");
        assert_eq!(result.source, SOURCE_CONFIG);
        assert_eq!(result.path, path_string(&canonical_path(&package)));
    }

    #[test]
    fn git_subdirectory_resolves_to_repository_root() {
        let temp = TempDir::new().unwrap();
        if !init_git(temp.path()) {
            return;
        }
        let nested = temp.path().join("src").join("domain");
        fs::create_dir_all(&nested).unwrap();

        let result = detect_project(&nested);

        assert_eq!(result.source, SOURCE_GIT_ROOT);
        assert_eq!(result.path, path_string(&canonical_path(temp.path())));
        assert_eq!(
            result.project,
            normalize_project_name(&path_basename(temp.path()))
        );
    }

    #[test]
    fn plain_directory_falls_back_to_normalized_basename() {
        let temp = TempDir::new().unwrap();
        let plain = temp.path().join("MyProject");
        fs::create_dir(&plain).unwrap();

        let result = detect_project(&plain);

        assert_eq!(result.project, "myproject");
        assert_eq!(result.source, SOURCE_DIR_BASENAME);
        assert!(result.error_hint.is_none());
    }

    #[test]
    fn multiple_child_repositories_are_structured_ambiguity() {
        let temp = TempDir::new().unwrap();
        for name in ["repo-alpha", "repo-beta"] {
            fs::create_dir_all(temp.path().join(name).join(".git")).unwrap();
        }

        let result = detect_project(temp.path());

        assert_eq!(result.project, "");
        assert_eq!(result.source, SOURCE_AMBIGUOUS);
        assert_eq!(result.available_projects, ["repo-alpha", "repo-beta"]);
        assert!(result.error_hint.is_some());
    }

    #[test]
    fn invalid_config_is_data_instead_of_a_returned_error() {
        let temp = TempDir::new().unwrap();
        create_config(temp.path(), r#"{"project_name":"../other"}"#);

        let result = detect_project(temp.path());

        assert_eq!(result.source, SOURCE_CONFIG);
        assert_eq!(result.project, "");
        assert!(
            result
                .error_hint
                .as_deref()
                .is_some_and(|hint| hint.contains(".leteo/config.json"))
        );
    }

    #[test]
    fn serialization_matches_the_mcp_envelope_fields() {
        let temp = TempDir::new().unwrap();
        let value = serde_json::to_value(detect_project(temp.path())).unwrap();

        assert!(value.get("project_source").is_some());
        assert!(value.get("project_path").is_some());
        assert!(value.get("available_projects").is_some());
        assert!(value.get("source").is_none());
        assert!(value.get("path").is_none());
    }

    #[test]
    fn malformed_or_missing_paths_never_escape_as_errors() {
        let temp = TempDir::new().unwrap();
        let missing = temp.path().join("missing");

        let result = detect_project(&missing);

        assert_eq!(result.project, "missing");
        assert_eq!(result.source, SOURCE_DIR_BASENAME);
        assert!(!result.path.is_empty());
    }

    #[test]
    fn similar_names_are_ordered_by_match_strength_and_exclude_exact_matches() {
        let existing = [
            "leteo",
            "Leteo",
            "leteo-cloud",
            "letea",
            "completely-different",
        ]
        .map(str::to_owned);

        let matches = find_similar("leteo", &existing, 3);
        let names = matches
            .iter()
            .map(|matched| matched.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["Leteo", "leteo-cloud", "letea"]);
        assert_eq!(matches[0].match_type, MATCH_CASE_INSENSITIVE);
        assert_eq!(matches[1].match_type, MATCH_SUBSTRING);
        assert_eq!(matches[2].match_type, MATCH_LEVENSHTEIN);
        assert_eq!(matches[2].distance, 1);
    }

    #[test]
    fn short_names_do_not_match_everything() {
        let existing = ["go", "golang-tools", "gg"].map(str::to_owned);

        let names = find_similar("go", &existing, 3)
            .into_iter()
            .map(|matched| matched.name)
            .collect::<Vec<_>>();
        assert_eq!(names, ["gg"]);
        assert_eq!(levenshtein("", "leteo"), 5);
        assert_eq!(levenshtein("leteo", ""), 5);
        assert_eq!(levenshtein("leteo", "leteo"), 0);
    }

    #[test]
    fn repository_name_supports_common_remote_formats() {
        assert_eq!(
            extract_repository_name("git@github.com:owner/Leteo.git").as_deref(),
            Some("Leteo")
        );
        assert_eq!(
            extract_repository_name("https://github.com/owner/leteo").as_deref(),
            Some("leteo")
        );
    }
}

#[cfg(test)]
mod detected_names {
    use super::*;

    /// A detected project is spelled the way a stored one is.
    ///
    /// Detection used to trim and lowercase and stop there, which is most of
    /// what `normalize::project` does and not all of it. A directory called
    /// `my--project` was detected as `my--project` and stored as
    /// `my-project`: the same project, found by every query because the store
    /// normalises again at its own door, and reported under two spellings in
    /// one answer — the envelope said one and the memory inside it said the
    /// other.
    #[test]
    fn a_detected_name_is_folded_the_way_a_stored_one_is() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["my--project", "my__project", "  Spaced  Out  "] {
            let directory = temp.path().join(name.trim());
            std::fs::create_dir_all(directory.join(".git")).unwrap();
            let detected = detect_project(&directory).project;
            assert_eq!(
                detected,
                crate::memory::normalize::project(&detected),
                "{name} is detected as something the store would rewrite"
            );
        }
        assert_eq!(
            detect_project(temp.path().join("my--project")).project,
            "my-project"
        );
    }

    /// A scan that ran out of time never passes for one that succeeded.
    ///
    /// This was the whole of the defect and it was silent in the worst way: a
    /// truncated scan skipped the ambiguity decision entirely and fell through
    /// to the directory basename, which is a *successful* source. So the guard
    /// that refuses a write into an ambiguous directory — the one that asks
    /// which project you meant and mints a recovery token — simply did not run,
    /// and memories were filed under the name of the folder holding the
    /// twenty-seven repositories. No warning, no hint, nothing to notice.
    ///
    /// Asked of the decision rather than of a race, because the only way to
    /// reach the truncated branch through the filesystem is to be unlucky, and
    /// a test that is right only when it is unlucky is a test that usually
    /// checks the other branch.
    #[test]
    fn a_scan_that_ran_out_of_time_says_so_instead_of_answering_confidently() {
        let cwd = Path::new("/workspace");
        let uno = PathBuf::from("/workspace/alpha");
        let dos = PathBuf::from("/workspace/beta");

        // Two found and out of time: still ambiguous, because reaching more
        // entries cannot turn two repositories into one. Only the list is short,
        // and it says so — it is the whitelist a replayed choice is checked
        // against, so presenting a partial one as whole is the older defect
        // this file already has a test for.
        let corto = detection_from_children(cwd, &[uno.clone(), dos.clone()], true);
        assert_eq!(corto.source, SOURCE_AMBIGUOUS, "{corto:?}");
        assert_eq!(corto.available_projects, ["alpha", "beta"]);
        assert_eq!(corto.warning.as_deref(), Some(SCAN_TRUNCATED_LIST));

        // One found and out of time: not promoted. The entry it never reached
        // could have been the second repository, and auto-promoting here claims
        // a certainty the scan did not buy.
        let solo = detection_from_children(cwd, std::slice::from_ref(&uno), true);
        assert_eq!(solo.source, SOURCE_DIR_BASENAME, "{solo:?}");
        assert_eq!(solo.warning.as_deref(), Some(SCAN_UNFINISHED));
        assert_ne!(solo.project, "alpha", "a guess is not a promotion");

        // None found and out of time: the basename is still the best answer,
        // because most directories hold no repositories at all — but it is
        // offered as a guess rather than as a finding.
        let vacio = detection_from_children(cwd, &[], true);
        assert_eq!(vacio.source, SOURCE_DIR_BASENAME, "{vacio:?}");
        assert_eq!(vacio.warning.as_deref(), Some(SCAN_UNFINISHED));

        // And a scan that finished keeps every answer it had. One child is
        // promoted, and none of these carries the warning.
        let promovido = detection_from_children(cwd, std::slice::from_ref(&uno), false);
        assert_eq!(promovido.source, SOURCE_GIT_CHILD, "{promovido:?}");
        assert_eq!(promovido.project, "alpha");
        let entero = detection_from_children(cwd, &[uno, dos], false);
        assert_eq!(entero.source, SOURCE_AMBIGUOUS, "{entero:?}");
        assert_eq!(entero.warning, None);
        assert_eq!(detection_from_children(cwd, &[], false).warning, None);
    }

    /// Only the two warnings that mean something went wrong are repeated.
    ///
    /// The hook path pushes these into its outcome, where `--verbose` and
    /// stderr show them. Promoting a single child repository also sets
    /// `warning`, and it is detection succeeding — repeated at every session
    /// opened anywhere near such a directory, it would be the line that teaches
    /// somebody to skip the warnings.
    #[test]
    fn only_a_scan_that_could_not_finish_is_worth_repeating() {
        let cwd = Path::new("/workspace");
        let uno = PathBuf::from("/workspace/alpha");
        let dos = PathBuf::from("/workspace/beta");

        let promovido = detection_from_children(cwd, std::slice::from_ref(&uno), false);
        assert!(promovido.warning.is_some(), "it does carry one");
        assert_eq!(
            promovido.scan_warning(),
            None,
            "but promoting the only repository there is is not a problem"
        );

        assert_eq!(
            detection_from_children(cwd, &[uno.clone(), dos], true).scan_warning(),
            Some(SCAN_TRUNCATED_LIST)
        );
        assert_eq!(
            detection_from_children(cwd, std::slice::from_ref(&uno), true).scan_warning(),
            Some(SCAN_UNFINISHED)
        );
        assert_eq!(
            detection_from_children(cwd, &[], false).scan_warning(),
            None
        );
    }

    /// A budget of nothing is a scan that reports itself truncated.
    ///
    /// The other half of the pair above: that one checks what the decision does
    /// with a truncated scan, this one checks that a scan out of time actually
    /// reports itself as one rather than as an empty directory.
    #[test]
    fn a_scan_with_no_time_left_reports_truncation_rather_than_emptiness() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("alpha").join(".git")).unwrap();
        std::fs::create_dir_all(temp.path().join("beta").join(".git")).unwrap();

        let (found, timed_out) = scan_child_repositories(temp.path(), Duration::ZERO);
        assert!(timed_out, "a spent budget is a truncated scan");
        assert!(found.is_empty(), "and it found nothing before giving up");

        let (found, timed_out) = scan_child_repositories(temp.path(), Duration::from_secs(30));
        assert!(!timed_out);
        assert_eq!(found.len(), 2, "{found:?}");
    }

    /// An ambiguous directory offers every project in it, not the first two.
    ///
    /// The scan stopped as soon as it had found two, which answered the only
    /// question it was asked at the time — is this directory ambiguous — and
    /// became wrong the moment the same list turned into the choices an agent
    /// offers and the whitelist a replay is checked against. On a real
    /// workspace of 56 directories holding 27 repositories it offered two: the
    /// first two alphabetically, neither one anybody works in. Answering with
    /// the project 32nd by name came back `invalid_project_choice`, so from
    /// that directory a memory could be filed only under a project its owner
    /// does not use.
    #[test]
    fn an_ambiguous_directory_offers_every_repository_it_holds() {
        let temp = tempfile::tempdir().unwrap();
        // Named so that the ones a person would pick sort last, which is the
        // shape that hid them: a scan that stops early stops at the top of the
        // alphabet.
        let esperados = ["aaa-primero", "bbb-segundo", "yyy-penultimo", "zzz-ultimo"];
        for nombre in esperados {
            let repo = temp.path().join(nombre);
            std::fs::create_dir_all(repo.join(".git")).unwrap();
        }
        for nombre in ["ccc-sin-git", "ddd-sin-git", "mmm-sin-git"] {
            std::fs::create_dir_all(temp.path().join(nombre)).unwrap();
        }

        // With the budget named rather than the default. Seven directories are
        // nothing to scan — the 56 of a real workspace cost 0.3 ms — but this
        // ran against a 200 ms deadline while fifteen other tests hammered the
        // same disk, and lost twice in fourteen runs of the suite. A test that
        // races a clock it has no reason to race is a test that reports the
        // machine's mood.
        let detection = detect_project_within(temp.path(), Duration::from_secs(30));
        assert_eq!(detection.source, SOURCE_AMBIGUOUS, "{detection:?}");
        assert_eq!(
            detection.available_projects.len(),
            esperados.len(),
            "ofrece todos los que hay: {:?}",
            detection.available_projects
        );
        assert_eq!(
            detection.warning, None,
            "a scan that finished says nothing about running out of time"
        );
        for nombre in esperados {
            assert!(
                detection.available_projects.iter().any(|p| p == nombre),
                "{nombre} no está entre los ofrecidos: {:?}",
                detection.available_projects
            );
        }
    }
}
