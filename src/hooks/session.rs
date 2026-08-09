//! Working out which session this is, and which project it belongs to.
//!
//! An agent may or may not send its own session identifier, may or may not be
//! in the directory the session was opened in, and may be in a repository that
//! has just gained a git remote and therefore a different name than it had
//! yesterday. All of that is settled here before anything is written.

use std::path::{Path, PathBuf};

use crate::{Store, memory::normalize, project::ProjectDetection};

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
    // directory resolves to would destroy both. Only fold in memories that
    // were actually recorded from *this* directory.
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
    if let Err(error) = store.merge_project(&basename, project) {
        outcome
            .warnings
            .push(super::said("migrate project", &error));
    }
}

/// Compares a stored session directory with the current one, tolerating the
/// separator and case differences that Windows paths pick up along the way.
fn same_directory(recorded: &str, directory: &Path) -> bool {
    fn comparable(value: &str) -> String {
        let value = value
            .trim()
            .trim_end_matches(['/', '\\'])
            .replace('\\', "/");
        if cfg!(windows) {
            value.to_lowercase()
        } else {
            value
        }
    }

    comparable(recorded) == comparable(&directory.to_string_lossy())
}
