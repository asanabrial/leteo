use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{SetupOptions, agents};

#[derive(Debug, Clone, Serialize)]
pub struct AgentRemoval {
    pub agent: &'static str,
    pub was_configured: bool,
    pub files_changed: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Removal {
    pub dry_run: bool,
    pub agents: Vec<AgentRemoval>,
    pub data_dir: PathBuf,
    pub data_dir_removed: bool,
    pub data_removed: bool,
    pub memories: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary: Option<PathBuf>,
    pub binary_removed: bool,
    /// The model files taken out of the places the binary looks for them; on a
    /// dry run, the ones that would be.
    pub model_files: Vec<PathBuf>,
    /// False when a model file was found and could not be deleted; a model that
    /// was never installed is not a failure.
    pub model_removed: bool,
    pub remaining: Vec<String>,
}

/// The first thing `uninstall --yes` says on stderr, before it judges any model
/// file. `scripts/uninstall.sh` and `uninstall.ps1` look for this line to learn
/// that the binary started, because a shell's exit code cannot: under dash an
/// exec that fails with ENOEXEC is re-run as a script and exits 2, which is
/// indistinguishable from the binary failing. `tests/uninstall_marker.rs` keeps
/// the scripts' copies of the text equal to this one.
pub const UNINSTALL_STARTED: &str = "leteo uninstall: started";

impl Removal {
    pub fn complete(&self) -> bool {
        self.agents.iter().all(|agent| agent.error.is_none())
            && (self.data_removed || self.dry_run)
            && (self.model_removed || self.dry_run)
    }
}

pub fn uninstall_everything(options: &SetupOptions, data_dir: &Path) -> Removal {
    uninstall_everything_for(options, data_dir, std::env::current_exe().ok())
}

/// [`uninstall_everything`] for a given executable, so the removal of what sits
/// beside it can be tested without being run against the test binary's own
/// directory.
fn uninstall_everything_for(
    options: &SetupOptions,
    data_dir: &Path,
    exe: Option<PathBuf>,
) -> Removal {
    let memories = count_memories(data_dir);
    let mut removed = Removal {
        dry_run: options.dry_run,
        agents: Vec::new(),
        data_dir: data_dir.to_path_buf(),
        data_dir_removed: false,
        data_removed: false,
        memories,
        binary: exe,
        binary_removed: false,
        model_files: Vec::new(),
        model_removed: false,
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

    // Before the data directory, because `<data dir>/model` is one of the places
    // and would otherwise keep the directory as something foreign.
    remove_model(&mut removed, data_dir);

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

/// The model, from every place the binary looks for it except `LETEO_MODEL_DIR`.
///
/// The places are [`crate::semantic::locations_for`] -- the list the lookup
/// itself reads -- and the files are [`crate::semantic::MODEL_FILES`], so an
/// installer that puts the model somewhere the binary finds it is also somewhere
/// this takes it from. The variable is left out because it names a directory
/// somebody else chose and may share with other things.
///
/// A file goes only if it is a regular file whose SHA-256 is its pin: a name is
/// not proof that Leteo wrote it, and `data/model` or `bin/model` is somewhere a
/// person may keep their own `config.json`. A file with the name and another
/// hash is kept and named, which includes the model an older release installed;
/// reported rather than deleted is the intended trade. A location that is itself
/// a symbolic link is not followed, for the same reason. Then a directory only
/// if that left it empty: `model/`, and `share/leteo` above it, never `share/`.
///
/// On a dry run nothing is touched and `model_files` is what would go.
fn remove_model(removed: &mut Removal, data_dir: &Path) {
    let dry_run = removed.dry_run;
    let mut failed = false;
    // The binary's own directory is listed twice when it is reached through a
    // link, once resolved and once not, and each file would otherwise be judged
    // and reported under both spellings.
    let mut judged = std::collections::HashSet::new();
    let real_data_dir = resolved(data_dir);
    for directory in crate::semantic::locations_for(removed.binary.as_deref(), data_dir, None) {
        if std::fs::symlink_metadata(&directory).is_ok_and(|meta| meta.file_type().is_symlink()) {
            if has_model_named_entry(&directory) {
                removed.remaining.push(format!(
                    "{} was kept: it is a symbolic link, and Leteo does not delete through one",
                    directory.display()
                ));
            }
            continue;
        }
        let real_directory = resolved(&directory);
        let mut took_any = false;
        for (name, pin) in crate::semantic::MODEL_FILES {
            let file = directory.join(name);
            let Ok(meta) = std::fs::symlink_metadata(&file) else {
                continue;
            };
            if !judged.insert(real_directory.join(name)) {
                continue;
            }
            if !meta.is_file() {
                removed.remaining.push(format!(
                    "{} was kept: it is not a regular file",
                    file.display()
                ));
                continue;
            }
            match hashes_to(&file, pin) {
                Ok(true) => {}
                Ok(false) => {
                    removed.remaining.push(format!(
                        "{} was kept: it is not the model this build installs",
                        file.display()
                    ));
                    continue;
                }
                Err(error) => {
                    failed = true;
                    removed
                        .remaining
                        .push(format!("{}: could not be read: {error}", file.display()));
                    continue;
                }
            }
            if dry_run {
                removed.model_files.push(file);
                continue;
            }
            match std::fs::remove_file(&file) {
                Ok(()) => {
                    took_any = true;
                    removed.model_files.push(file);
                }
                Err(error) => {
                    failed = true;
                    removed
                        .remaining
                        .push(format!("{}: {error}", file.display()));
                }
            }
        }
        if took_any && remove_if_empty(removed, &directory) {
            // A share/leteo that holds something else is somebody else's, and
            // is named as such. Not when it is the data directory: with
            // `LETEO_DATA_DIR` at `.../share/leteo` the names match, and the
            // store beside the model would be reported as a stranger, or the
            // directory removed here before `remove_data_directory` judged it.
            if let Some(parent) = directory.parent()
                && resolved(parent) != real_data_dir
                && parent.file_name().is_some_and(|name| name == "leteo")
                && parent
                    .parent()
                    .and_then(Path::file_name)
                    .is_some_and(|name| name == "share")
            {
                remove_if_empty(removed, parent);
            }
        }
    }
    removed.model_removed = !failed;
}

/// Whether a directory, reached through whatever it is, holds a model file's name.
fn has_model_named_entry(directory: &Path) -> bool {
    crate::semantic::MODEL_FILES
        .iter()
        .any(|(name, _)| std::fs::symlink_metadata(directory.join(name)).is_ok())
}

fn hashes_to(file: &Path, pin: &str) -> std::io::Result<bool> {
    Ok(crate::semantic::sha256_hex(&std::fs::read(file)?) == pin)
}

/// A path as the filesystem resolves it: through every link that exists, with
/// whatever does not exist yet kept as written.
///
/// Every comparison of two locations goes through this, so both sides are
/// normalised the same way. `canonicalize` alone fails on a path that is not
/// there, and falling back to the raw path then sets an unresolved spelling
/// against a resolved one -- a data directory that does not exist yet, under a
/// link, would not equal the same directory reached from the other side.
fn resolved(path: &Path) -> PathBuf {
    if let Ok(real) = path.canonicalize() {
        return real;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => resolved(parent).join(name),
        _ => path.to_path_buf(),
    }
}

/// Removes a directory that is empty, and says why one that stayed did.
///
/// "Holds files Leteo did not put there" is said only of a directory that does
/// hold something; a permission or a busy handle on an empty one is reported as
/// the I/O error it is, so nobody goes looking for a stranger who is not there.
/// Returns whether the directory is gone.
fn remove_if_empty(removed: &mut Removal, directory: &Path) -> bool {
    let error = match std::fs::remove_dir(directory) {
        Ok(()) => return true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
        Err(error) => error,
    };
    let strangers: Vec<String> = std::fs::read_dir(directory)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    removed.remaining.push(if strangers.is_empty() {
        format!("{}: {error}", directory.display())
    } else {
        format!(
            "{} was kept: it holds {} that Leteo did not put there ({})",
            directory.display(),
            if strangers.len() == 1 {
                "a file"
            } else {
                "files"
            },
            strangers.join(", ")
        )
    });
    false
}

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
