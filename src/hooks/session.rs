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
    project::{ProjectDetection, same_directory},
};

use super::{HookInput, HookOutcome};

pub(super) fn resolve_directory(cwd: &str) -> PathBuf {
    if cwd.trim().is_empty() {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    } else {
        PathBuf::from(cwd)
    }
}

pub(super) fn resolve_project(input: &HookInput, detection: &ProjectDetection) -> String {
    input
        .project
        .as_deref()
        .map(normalize::project)
        .filter(|project| !project.is_empty())
        .unwrap_or_else(|| normalize::project(&detection.project))
}

/// Uses the agent's own session identifier when it sends one, so several agents
/// can share a project without clobbering each other's sessions.
pub(super) fn resolve_session_id(input: &HookInput, project: &str) -> String {
    let session_id = input.session_id.trim();
    if session_id.is_empty() {
        crate::mcp::manual_session_id(project)
    } else {
        session_id.to_owned()
    }
}

pub(super) fn ensure_session(
    store: &mut Store,
    session_id: &str,
    project: &str,
    directory: &Path,
    outcome: &mut HookOutcome,
) {
    if store.get_session(session_id).is_ok() {
        return;
    }
    match store.create_session(session_id, project, &directory.to_string_lossy()) {
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
    let recorded_here = store
        .session_directories(&basename)
        .unwrap_or_default()
        .iter()
        .any(|recorded| same_directory(recorded, directory));
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
