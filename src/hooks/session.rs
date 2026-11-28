//! Working out which session this is, and which project it belongs to.
//!
//! An agent may or may not send its own session identifier, may or may not be
//! in the directory the session was opened in, and may be in a repository that
//! has just gained a git remote and therefore a different name than it had
//! yesterday. All of that is settled here before anything is written.

use std::path::{Path, PathBuf};

use crate::{
    Store,
    memory::normalize,
    project::{DirectoryTarget, ProjectDetection},
};

use super::{HookInput, HookOutcome};

/// The directory a hook is talking about, always absolute.
///
/// A relative `cwd` used to be stored as it arrived, so the session recorded a
/// directory that only means something beside the process that wrote it, and
/// the next hook — from another directory, or another run — could not match it
/// to the history it was supposed to join. `cwd` is relative to the process
/// that was started in it, which is the hook's own directory.
pub(super) fn resolve_directory(cwd: &str) -> PathBuf {
    resolve_directory_against(cwd, std::env::current_dir().ok())
}

/// [`resolve_directory`] with the process directory named, so the rule can be
/// asked directly instead of through whatever directory the test binary
/// happens to have been started in.
fn resolve_directory_against(cwd: &str, process: Option<PathBuf>) -> PathBuf {
    let cwd = cwd.trim();
    if cwd.is_empty() {
        return process.unwrap_or_else(|| PathBuf::from("."));
    }
    let directory = PathBuf::from(cwd);
    if directory.is_absolute() {
        return directory;
    }
    match process {
        Some(process) => process.join(directory),
        // No process directory to resolve against: the path stays relative,
        // which the empty-directory guard downstream already reports.
        None => directory,
    }
}

/// The project this hook files under, and the reason a named one was not used.
///
/// A hook cannot hand a caller an error and wait: a refused project name would
/// leave the event with nothing to record, and the agent in front of it with a
/// failure it cannot fix. So the name it would have used is refused, the
/// detected one is used in its place, and the sentence travels back with the
/// answer — `100,000` characters of project made every session-start in that
/// directory print a 100 KB warning, which is the opposite of telling anybody.
pub(super) fn resolve_project(
    input: &HookInput,
    detection: &ProjectDetection,
) -> (String, Option<String>) {
    let detected = normalize::project(&detection.project);
    let Some(named) = input
        .project
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return (detected, None);
    };
    match normalize::project_refusal(named) {
        None => (normalize::project(named), None),
        Some(refusal) => {
            let used = if detected.is_empty() {
                "an empty project".to_owned()
            } else {
                format!("\"{detected}\"")
            };
            (
                detected,
                Some(format!("{refusal}; filed under {used} instead")),
            )
        }
    }
}

/// Uses the agent's own session identifier when it sends one, so several agents
/// can share a project without clobbering each other's sessions.
///
/// The same fallback as [`resolve_project`], for the same reason, and it is the
/// id a state file is named after: a 10,000-character one made every later hook
/// in that session print `File name too long (os error 63)` and lose its state.
pub(super) fn resolve_session_id(input: &HookInput, project: &str) -> (String, Option<String>) {
    let session_id = input.session_id.trim();
    if session_id.is_empty() {
        return (crate::mcp::manual_session_id(project), None);
    }
    match normalize::session_id_refusal(session_id) {
        None => (session_id.to_owned(), None),
        Some(refusal) => (
            crate::mcp::manual_session_id(project),
            Some(format!(
                "{refusal}; this hook is recorded under an id derived from its project instead"
            )),
        ),
    }
}

pub(super) fn ensure_session(
    store: &mut Store,
    session_id: &str,
    project: &str,
    directory: &Path,
    remote: Option<&str>,
    outcome: &mut HookOutcome,
) {
    // A row that exists and already says which repository it is in has nothing
    // to learn, and a hook runs on every prompt: paying a write transaction to
    // repeat that answer would be paid on the commonest path this crate has.
    // A row that is missing a remote it can now be told is written once, and
    // only when the row is still this directory's and this project's: a
    // session an agent carried into another checkout must not be stamped with
    // that checkout's repository.
    let asked = directory.to_string_lossy();
    if let Ok(session) = store.get_session(session_id) {
        let same_project = normalize::project(&session.project) == normalize::project(project);
        let same_directory = session.directory.is_empty()
            || crate::project::comparable(&session.directory) == crate::project::comparable(&asked);
        let can_learn = session.remote.is_empty()
            && remote.is_some_and(|remote| !remote.is_empty())
            && same_project
            && same_directory;
        if !can_learn {
            return;
        }
    }
    let created = match remote {
        Some(remote) if !remote.is_empty() => {
            store.create_session_with_remote(session_id, project, &asked, remote)
        }
        _ => store.create_session(session_id, project, &asked),
    };
    match created {
        Ok(_) => outcome.session_created = true,
        Err(error) => outcome.warnings.push(super::said("create session", &error)),
    }
}

/// Folds memories saved under the plain directory name into the detected
/// project, which is what happens the first time a repository gains a remote.
pub(super) fn migrate_directory_project(
    store: &mut Store,
    directory: &Path,
    project: &str,
    outcome: &mut HookOutcome,
) {
    let basename = directory
        .file_name()
        .map(|name| normalize::project(&name.to_string_lossy()))
        .unwrap_or_default();
    if basename.is_empty() || basename == project {
        return;
    }

    // Matching names are not enough. A project called "api" may belong to a
    // completely different checkout, and merging it into whatever this
    // directory resolves to would destroy both. Only the sessions recorded
    // from *this* directory move, with their memories and prompts; a name that
    // was recorded here and also elsewhere used to be folded whole, taking the
    // other checkout's memories along, and its spelling variants (`my_api` for
    // `my-api`) with them.
    //
    // Asked of the one project, not of every project.
    //
    // This used to read `list_projects_with_stats`, which aggregates the whole
    // store and then keeps a single row. Measured end to end it made no
    // difference at this size — 6 ms of `GROUP BY` in isolation disappears
    // inside a hook's 56 ms — so this is not a speed-up and is not claimed as
    // one. It is kept because the cost of the old shape grows with the number
    // of projects in the store while the question does not: somebody with two
    // hundred projects pays for all of them to answer something about one.
    let mut target = DirectoryTarget::new(directory);
    let recorded_here = store
        .session_directories(&basename)
        .unwrap_or_default()
        .iter()
        .any(|recorded| target.matches(recorded));
    if !recorded_here {
        return;
    }
    match store.move_directory_sessions(&basename, project, directory) {
        Ok((_, 0)) => {}
        Ok((_, left_behind)) => outcome.warnings.push(format!(
            "left {left_behind} session{} under \"{basename}\" behind: recorded from another directory, \
             so not moved into \"{project}\"; `leteo projects merge` moves them if they belong there",
            if left_behind == 1 { "" } else { "s" }
        )),
        Err(error) => outcome
            .warnings
            .push(super::said("migrate project", &error)),
    }
}

/// The projects this directory's sessions were recorded under, when any of them
/// is not the project detection now resolves to.
///
/// The write path can refuse and hand the agent a recovery token; a hook cannot
/// ask anybody, so this is what it has to say instead: a session-start that
/// finds the directory's history under another name names it and points at
/// `leteo projects consolidate`. Asked after [`migrate_directory_project`] has
/// run, so the ordinary case it already folds — a repository that gained a
/// remote and took its directory's name — is not warned about as a split it
/// has just repaired.
///
/// The error is returned rather than folded into an empty list: a lookup that
/// could not run is not a lookup that found no drift, and the caller says which
/// it was.
pub(super) fn drifted_directory_projects(
    store: &Store,
    directory: &Path,
    project: &str,
) -> Result<Vec<String>, crate::store::StoreError> {
    store.recent_projects_in_directory(&directory.to_string_lossy(), project)
}
