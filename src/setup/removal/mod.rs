//! Taking Leteo off a machine entirely.
//!
//! [`super::uninstall`] takes Leteo out of *one* agent. This takes it out of
//! all of them and then removes what the install left behind: the store, the
//! settings, the reminder clocks, and — where the platform allows it — the
//! binary itself.
//!
//! # What it deliberately does not touch
//!
//! A `.leteo/config.json` **inside a repository** is not part of an install. It
//! names the project that checkout belongs to and is usually tracked, so it is
//! the repository's file. An uninstaller that walked the filesystem removing
//! every `.leteo/` it found would delete other people's tracked files, on a
//! machine where the person asked only to remove a program. The data directory
//! and that one share a name and nothing else.
//!
//! # The binary
//!
//! Unix allows unlinking a running executable — the file goes and the image
//! stays mapped until the process ends — so this removes it and the job is
//! finished in one command.
//!
//! Windows holds the image open and refuses, and there is no honest way around
//! that from inside the process being removed. So there the binary is reported
//! rather than deleted, and `uninstall.ps1` finishes it: PowerShell reads a
//! script into memory before running it, which is what lets that file delete
//! the binary, its own directory, the PATH entry and its registry key. The
//! installer registers that script as the `UninstallString`, which is also the
//! only reason Leteo appears in Windows' list of installed programs at all.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{SetupOptions, agents};

/// What one agent's removal did.
#[derive(Debug, Clone, Serialize)]
pub struct AgentRemoval {
    pub agent: &'static str,
    /// Whether Leteo was in this agent before anything was done.
    pub was_configured: bool,
    /// How many of that agent's files changed.
    pub files_changed: usize,
    /// Why this one failed, when it did. One agent's failure is not a reason
    /// to leave Leteo installed in the other eleven.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Everything an uninstall found, and what became of it.
#[derive(Debug, Clone, Serialize)]
pub struct Removal {
    /// True when nothing was actually touched.
    pub dry_run: bool,
    pub agents: Vec<AgentRemoval>,
    pub data_dir: PathBuf,
    /// Whether the directory itself went.
    ///
    /// False when something Leteo did not put there kept it alive, which is not
    /// a failure — see [`Removal::complete`].
    pub data_dir_removed: bool,
    /// Whether Leteo's own files in it are gone.
    ///
    /// The one that decides whether the uninstall did its job. The directory
    /// surviving because it holds a note somebody filed beside the store is a
    /// success with a leftover, not a partial removal.
    pub data_removed: bool,
    /// What the store held, counted before it went.
    ///
    /// `None` when it could not be read, which is not a reason to refuse: a
    /// store too broken to count is a store somebody is especially likely to
    /// be uninstalling.
    pub memories: Option<i64>,
    /// The running binary, when it could be located.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary: Option<PathBuf>,
    pub binary_removed: bool,
    /// What is left, and what to do about it. Never empty on Windows.
    pub remaining: Vec<String>,
}

impl Removal {
    /// Whether every agent came out cleanly.
    pub fn complete(&self) -> bool {
        self.agents.iter().all(|agent| agent.error.is_none()) && (self.data_removed || self.dry_run)
    }
}

/// Takes Leteo out of every agent it knows about, then off the machine.
///
/// Ordered so that the parts needing the most knowledge happen while that
/// knowledge is still there: the agents first, because resolving twelve config
/// files is the one step the shell scripts could not repeat, then the data, then
/// the binary.
pub fn uninstall_everything(options: &SetupOptions, data_dir: &Path) -> Removal {
    let memories = count_memories(data_dir);
    let mut removed = Removal {
        dry_run: options.dry_run,
        agents: Vec::new(),
        data_dir: data_dir.to_path_buf(),
        data_dir_removed: false,
        data_removed: false,
        memories,
        binary: std::env::current_exe().ok(),
        binary_removed: false,
        remaining: Vec::new(),
    };

    for adapter in agents::REGISTRY {
        let was_configured = super::is_configured(adapter.slug, options);
        // Run even where nothing is configured. `is_configured` answers about
        // the MCP entry, and an agent can still be carrying a protocol block or
        // a stale hook from an older install — which is exactly the leftover
        // somebody uninstalling wants gone.
        let outcome = super::uninstall(adapter.slug, options);
        removed.agents.push(match outcome {
            Ok(result) => AgentRemoval {
                agent: adapter.slug,
                was_configured,
                files_changed: result.changed_files(),
                error: None,
            },
            Err(error) => AgentRemoval {
                agent: adapter.slug,
                was_configured,
                files_changed: 0,
                error: Some(error.to_string()),
            },
        });
    }

    if !options.dry_run && data_dir.exists() {
        remove_data_directory(&mut removed, data_dir);
    }

    remove_binary(&mut removed, options.dry_run);
    removed
}

/// Everything Leteo creates in its data directory, by name.
///
/// Named rather than globbed, and the directory is emptied rather than deleted,
/// because `LETEO_DATA_DIR` points wherever somebody told it to. `remove_dir_all`
/// on that path is a program that deletes a directory it does not own — which
/// is exactly how other tools have taken people's own files with them on the way
/// out. Nothing here removes a thing it did not put there.
///
/// The suffixes cover SQLite's sidecars and the copies a migration leaves
/// behind; the prefixes cover the dated backups.
const DATA_DIR_FILES: &[&str] = &["leteo.db", "settings.json", "cloud.json", "store.db"];
const DATA_DIR_PREFIXES: &[&str] = &["leteo.db", "store.db", "backup-"];
const DATA_DIR_SUBDIRECTORIES: &[&str] = &["hooks"];

/// Empties the data directory of Leteo's own files, and removes it only if that
/// left nothing behind.
///
/// A file somebody else put in there keeps the directory alive and is reported,
/// rather than being taken along with the rest. Uninstalling a memory tool
/// should not be a way to lose a note that happened to be filed beside it.
fn remove_data_directory(removed: &mut Removal, data_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(data_dir) else {
        removed
            .remaining
            .push(format!("{}: could not be read", data_dir.display()));
        return;
    };
    let mut foreign = Vec::new();
    let failures_before = removed.remaining.len();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_ours = DATA_DIR_FILES.contains(&name.as_str())
            || DATA_DIR_SUBDIRECTORIES.contains(&name.as_str())
            || DATA_DIR_PREFIXES
                .iter()
                .any(|prefix| name.starts_with(prefix));
        if !is_ours {
            foreign.push(name);
            continue;
        }
        let outcome = if entry.path().is_dir() {
            std::fs::remove_dir_all(entry.path())
        } else {
            std::fs::remove_file(entry.path())
        };
        if let Err(error) = outcome {
            removed
                .remaining
                .push(format!("{}: {error}", entry.path().display()));
        }
    }
    // Leteo's own files are gone unless removing one of them failed.
    removed.data_removed = removed.remaining.len() == failures_before;
    if foreign.is_empty() {
        match std::fs::remove_dir(data_dir) {
            Ok(()) => removed.data_dir_removed = true,
            Err(error) => removed
                .remaining
                .push(format!("{}: {error}", data_dir.display())),
        }
    } else {
        removed.remaining.push(format!(
            "{} was kept: it holds {} that Leteo did not put there ({})",
            data_dir.display(),
            if foreign.len() == 1 {
                "a file"
            } else {
                "files"
            },
            foreign.join(", ")
        ));
    }
}

/// Removes the running binary where the platform allows it, and says so where
/// it does not.
#[cfg(not(windows))]
fn remove_binary(removed: &mut Removal, dry_run: bool) {
    let Some(binary) = removed.binary.clone() else {
        return;
    };
    if dry_run {
        return;
    }
    match std::fs::remove_file(&binary) {
        Ok(()) => removed.binary_removed = true,
        Err(error) => removed
            .remaining
            .push(format!("{}: {error}", binary.display())),
    }
}

#[cfg(windows)]
fn remove_binary(removed: &mut Removal, _dry_run: bool) {
    let Some(binary) = removed.binary.clone() else {
        return;
    };
    // Not an error and not a failure to report as one: Windows holds a running
    // image open, and the only thing that can finish this is the script that is
    // not the binary.
    removed.remaining.push(format!(
        "{} is still here: Windows cannot delete a running program. \
         Remove Leteo from Settings > Installed apps, or run uninstall.ps1 \
         beside it, which also takes the PATH entry and the registry key.",
        binary.display()
    ));
}

/// What the store holds, without opening it for writing.
///
/// Read-only and forgiving: this runs to put a number in front of somebody
/// before they destroy it, and a store that cannot answer must not stop them.
fn count_memories(data_dir: &Path) -> Option<i64> {
    let database = data_dir.join("leteo.db");
    if !database.exists() {
        return None;
    }
    let connection = rusqlite::Connection::open_with_flags(
        &database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()?;
    connection
        .query_row(
            "SELECT COUNT(*) FROM observations WHERE deleted_at IS NULL",
            [],
            |row| row.get(0),
        )
        .ok()
}

#[cfg(test)]
mod tests;
