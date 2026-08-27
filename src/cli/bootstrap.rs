use super::*;

#[derive(Default)]
pub(super) struct BackgroundAutosync {
    shutdown: Option<tokio::sync::watch::Sender<bool>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl BackgroundAutosync {
    /// Whether a loop actually started, rather than the configuration being
    /// absent or disabled.
    pub(super) fn is_running(&self) -> bool {
        self.handle.is_some()
    }

    pub(super) async fn shutdown(self) {
        if let Some(shutdown) = self.shutdown {
            let _ = shutdown.send(true);
        }
        if let Some(handle) = self.handle {
            // Joining a thread blocks, so it goes to the blocking pool instead
            // of stalling the runtime that is shutting down.
            let _ = tokio::task::spawn_blocking(move || handle.join()).await;
        }
    }
}

/// Starts cloud replication alongside `serve` and `mcp` when the persisted
/// configuration is complete and enabled.
///
/// The loop gets its own thread, its own single-threaded runtime, and its own
/// SQLite connection. rusqlite is synchronous and the store waits up to five
/// seconds for a write lock, so running the loop on the shared runtime would
/// let one contended `ack` block a worker that requests need — latency nobody
/// could explain from the outside. On its own thread it can block as long as
/// SQLite wants without touching the foreground.
pub(super) fn start_background_autosync(
    store_config: &StoreConfig,
    cloud_config_path: &Path,
) -> Result<BackgroundAutosync> {
    let config = crate::cloud::ClientConfig::load(cloud_config_path)?;
    if !config.is_runnable() {
        return Ok(BackgroundAutosync::default());
    }
    let store_config = store_config.clone();
    let (shutdown, receiver) = tokio::sync::watch::channel(false);
    let handle = std::thread::Builder::new()
        .name("leteo-autosync".to_owned())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    tracing::error!(%error, "background autosync could not start a runtime");
                    return;
                }
            };
            runtime.block_on(async move {
                let mut store = match Store::open(store_config) {
                    Ok(store) => store,
                    Err(error) => {
                        tracing::error!(%error, "background autosync could not open the store");
                        return;
                    }
                };
                let remote = match crate::cloud::RemoteClient::new(&config.server, &config.token) {
                    Ok(remote) => remote,
                    Err(error) => {
                        tracing::error!(%error, "background autosync could not reach the cloud");
                        return;
                    }
                };
                let autosync_config = crate::cloud::AutosyncConfig {
                    poll_interval: config.poll_interval(),
                    allowed_projects: config.projects.clone(),
                    ..crate::cloud::AutosyncConfig::default()
                };
                let mut autosync =
                    match crate::cloud::Autosync::new(&mut store, remote, autosync_config) {
                        Ok(autosync) => autosync,
                        Err(error) => {
                            tracing::error!(%error, "background autosync configuration is invalid");
                            return;
                        }
                    };
                if let Err(error) = autosync.run(receiver).await {
                    tracing::error!(%error, "background autosync stopped");
                }
            });
        })
        .context("start the background autosync thread")?;
    Ok(BackgroundAutosync {
        shutdown: Some(shutdown),
        handle: Some(handle),
    })
}

/// How many memories this store already holds, or `None` if it cannot be read.
///
/// `None` is not zero and callers must not flatten it into one. A first run has
/// no database and reads as `None`; so does a database another Leteo has open,
/// which on a machine where agents are running is most of the time, and so does
/// one that is damaged. Deciding anything by treating those as "empty" decides
/// it about a store that may hold everything somebody has.
pub(super) fn stored_observations(cli: &Cli) -> Option<i64> {
    store_config(cli)
        .ok()
        .and_then(|config| Store::open(config).ok())
        .and_then(|store| store.stats().ok())
        .map(|stats| stats.total_observations)
}

/// Whether this store is known to hold nothing.
///
/// The question two different setup decisions turn on, and they used to ask it
/// separately and answer it wrong the same way — by reading "could not tell" as
/// "nothing there". `Some(0)` is the only answer that means empty.
///
/// One offers to migrate an Engram installation over the top of this database,
/// and the other suppresses the warning that choosing a language leaves
/// everything already saved in the language it was written in. Both are about a
/// store that may hold every memory somebody has, and a database another Leteo
/// happens to have open reads exactly like a missing one.
pub(super) fn store_is_known_empty(cli: &Cli) -> bool {
    stored_observations(cli) == Some(0)
}

/// Describes an Engram installation worth adopting, when there is one.
///
/// Only offered while this Leteo store is still empty. Once someone has their
/// own memories here, adoption would refuse anyway, and suggesting it would be
/// an invitation to lose them.
pub(super) fn engram_offer(cli: &Cli) -> Option<serde_json::Value> {
    let found = crate::engram::inspect(&crate::engram::default_database()?).ok()?;
    if found.is_empty() {
        return None;
    }
    // Three answers, the same three `adopt` itself insists on: no database
    // here, a database that says it is empty, and a database nobody could
    // read. Only the first two are an invitation. The third used to count as
    // empty, so a store held open by another Leteo — which on a machine with
    // agents running is most of the time — was offered a migration over the
    // top of it. Adoption would refuse, but only after somebody had been told
    // to try.
    let target = store_config(cli).ok()?.database_path;
    if target.is_file() && !store_is_known_empty(cli) {
        return None;
    }
    Some(serde_json::json!({
        "detected": true,
        "database": found.database,
        "observations": found.observations,
        "sessions": found.sessions,
        "prompts": found.prompts,
        "relations": found.relations,
        "adopt_with": "leteo import --from-engram",
        "preview_with": "leteo import --from-engram --dry-run",
    }))
}

pub(super) fn data_directory(cli: &Cli) -> Result<PathBuf> {
    store_config(cli)?
        .database_path
        .parent()
        .map(Path::to_path_buf)
        .context("the Leteo database path has no parent directory")
}

pub(super) fn store_config(cli: &Cli) -> Result<StoreConfig> {
    if let Some(database) = &cli.database {
        return Ok(StoreConfig::new(absolutize(database)?));
    }
    let data_dir = match &cli.data_dir {
        Some(path) => absolutize(path)?,
        None => crate::paths::home_dir()
            .context("resolve home directory")?
            .join(".leteo"),
    };
    Ok(StoreConfig::in_data_dir(data_dir))
}

pub(super) fn absolutize(path: &PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.clone());
    }
    Ok(std::env::current_dir()?.join(path))
}

pub(super) fn print_json(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli_for(database: &std::path::Path) -> Cli {
        Cli {
            data_dir: None,
            database: Some(database.to_path_buf()),
            command: crate::cli::args::Command::Doctor {
                check: None,
                project: None,
                repair: false,
            },
        }
    }

    /// A store nobody can read is not an empty store.
    ///
    /// Both decisions this answer feeds got it wrong at once while `None` was
    /// flattened to zero. The setup wizard offered to migrate an Engram
    /// installation over the top of a database it had merely failed to open —
    /// and, because "empty" is also what suppresses it, withheld the warning
    /// that choosing a language leaves everything already saved in whatever
    /// language it was written in.
    ///
    /// It is not a rare state. A database another Leteo has open reads exactly
    /// like this, and on a machine with agents running that is most of the day.
    #[test]
    fn a_store_that_cannot_be_read_is_not_reported_as_empty() {
        let temp = tempfile::tempdir().unwrap();

        // No file at all: a genuine first run, and the only case where "there
        // is nothing here" is the truth.
        let missing = temp.path().join("absent.db");
        assert_eq!(stored_observations(&cli_for(&missing)), Some(0));

        // A file that is not a database. Damaged, half-written, or held by
        // something else — from here they are the same answer: no answer.
        let unreadable = temp.path().join("unreadable.db");
        std::fs::write(&unreadable, b"this is not a SQLite database").unwrap();
        assert_eq!(
            stored_observations(&cli_for(&unreadable)),
            None,
            "an unreadable store has to say so rather than say zero"
        );

        // And the decision both setup steps turn on. Asserting here rather
        // than on `engram_offer`: that one bails early when the machine has no
        // Engram installation worth adopting, so on most machines it would
        // pass without ever reaching the question being asked.
        assert!(
            store_is_known_empty(&cli_for(&missing)),
            "a first run with no database is the one case that is truly empty"
        );
        assert!(
            !store_is_known_empty(&cli_for(&unreadable)),
            "a database nobody could open must not count as an empty one"
        );
    }
}
