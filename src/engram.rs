//! Finding an existing Engram installation and taking its memories over.
//!
//! Leteo's schema follows Leteo's needs. It is not held to Engram's names, and
//! the two are free to drift apart — the translation between them lives here,
//! in the adapter, rather than holding the storage layer hostage.
//!
//! This is also why adoption copies rows rather than files: Engram's JSON
//! export carries sessions, observations and prompts but not the relation
//! verdicts, so exporting and importing would quietly drop every conflict
//! judgement the user ever made.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

/// What Engram calls the table Leteo calls `prompts`.
const ENGRAM_PROMPTS: &str = "user_prompts";

/// The tables an adoption carries, named on each side.
///
/// The only place that has to know both vocabularies. A Leteo table with no
/// counterpart is simply left empty, so adding one obliges nobody to touch
/// this list.
const TABLE_MAP: &[(&str, &str)] = &[
    ("sessions", "sessions"),
    ("observations", "observations"),
    (ENGRAM_PROMPTS, "prompts"),
    ("memory_relations", "memory_relations"),
    ("sync_chunks", "sync_chunks"),
    ("sync_mutations", "sync_mutations"),
    ("sync_state", "sync_state"),
    ("sync_enrolled_projects", "sync_enrolled_projects"),
    ("prompt_tombstones", "prompt_deletions"),
    ("sync_apply_deferred", "sync_deferred_mutations"),
    ("cloud_upgrade_state", "sync_upgrade_state"),
];

/// What an Engram installation holds, and where.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Installation {
    pub database: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary: Option<PathBuf>,
    pub sessions: i64,
    pub observations: i64,
    pub prompts: i64,
    pub relations: i64,
}

impl Installation {
    /// Whether there is anything worth taking over.
    pub fn is_empty(&self) -> bool {
        self.sessions == 0 && self.observations == 0 && self.prompts == 0
    }
}

/// What an adoption did, or would do.
#[derive(Debug, Clone, Serialize)]
pub struct Adoption {
    pub source: PathBuf,
    pub target: PathBuf,
    pub dry_run: bool,
    pub found: Installation,
    /// Counts read back from the adopted database. Absent on a dry run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adopted: Option<Counts>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct Counts {
    pub sessions: i64,
    pub observations: i64,
    pub prompts: i64,
    pub relations: i64,
}

/// The database an Engram install keeps its memories in, if one is there.
pub fn default_database() -> Option<PathBuf> {
    let home = crate::paths::home_dir().ok()?;
    let database = home.join(".engram").join("engram.db");
    database.is_file().then_some(database)
}

/// The Engram binary, if it is on the path. Only used to describe what was
/// found; nothing here runs it.
fn find_binary() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "engram.exe"
    } else {
        "engram"
    };
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|directory| directory.join(name))
            .find(|candidate| candidate.is_file())
    })
}

/// Reads what an Engram database contains without writing to it.
pub fn inspect(database: &Path) -> Result<Installation> {
    if !database.is_file() {
        bail!("no Engram database at {}", database.display());
    }
    // Engram's table names, deliberately: this reads Engram's database, and
    // `user_prompts` is what it calls the table Leteo calls `prompts`.
    let count = open_counter(database)?;
    Ok(Installation {
        database: database.to_path_buf(),
        binary: find_binary(),
        sessions: count("sessions"),
        observations: count("observations"),
        prompts: count(ENGRAM_PROMPTS),
        relations: count("memory_relations"),
    })
}

/// Takes an Engram installation's memories over.
///
/// The source is first snapshotted with `VACUUM INTO`, which folds in the
/// write-ahead log and reads through one consistent point. A plain file copy
/// would miss whatever a running Engram had not yet checkpointed, which for
/// someone migrating mid-session is exactly their most recent memories.
pub fn adopt(source: &Path, target: &Path, dry_run: bool) -> Result<Adoption> {
    let found = inspect(source)?;
    if found.is_empty() {
        bail!(
            "the Engram database at {} holds no memories; nothing to adopt",
            source.display()
        );
    }

    // Refusing beats merging: there is no safe way to fold two histories
    // together, and silently replacing memories would be unforgivable.
    //
    // Three answers, not two. "I could not read it" is not "it is empty", and
    // treating it as one deleted the file a few lines below — a database
    // locked by a running Leteo, or truncated by a write that did not finish,
    // counted as nothing and was replaced while the command reported success.
    // A file nobody can read is the case where keeping it matters most,
    // because it is the one somebody may still recover from.
    if target.is_file() {
        let existing = Connection::open_with_flags(target, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .ok()
            .and_then(|connection| {
                connection
                    .query_row("SELECT COUNT(*) FROM observations", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .ok()
            });
        match existing {
            Some(0) => {}
            Some(existing) => bail!(
                "{} already holds {existing} observations; move it aside first, \
                 because adopting would replace them",
                target.display()
            ),
            None => bail!(
                "{} exists but cannot be read as a Leteo database — it may be \
                 open in another process, or damaged. Adopting would delete it. \
                 Move it aside first, or point --database somewhere else.",
                target.display()
            ),
        }
    }

    if dry_run {
        return Ok(Adoption {
            source: source.to_path_buf(),
            target: target.to_path_buf(),
            dry_run: true,
            found,
            adopted: None,
        });
    }

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    // Start from nothing, sidecars included: a stale write-ahead log would be
    // read as belonging to the new file.
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(PathBuf::from(format!("{}{suffix}", target.display())));
    }

    let snapshot = PathBuf::from(format!("{}.adopting", target.display()));
    let _ = std::fs::remove_file(&snapshot);
    {
        let reader = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("open {}", source.display()))?;
        reader
            .execute("VACUUM INTO ?1", [snapshot.to_string_lossy().as_ref()])
            .with_context(|| format!("snapshot {}", source.display()))?;
    }
    let translated = translate(&snapshot, target);
    let _ = std::fs::remove_file(&snapshot);
    translated?;

    let adopted = read_counts(target)?;
    if adopted.observations != found.observations
        || adopted.sessions != found.sessions
        || adopted.prompts != found.prompts
        || adopted.relations != found.relations
    {
        bail!(
            "the adoption does not match the source: found {:?}, adopted {adopted:?}",
            Counts {
                sessions: found.sessions,
                observations: found.observations,
                prompts: found.prompts,
                relations: found.relations,
            }
        );
    }

    Ok(Adoption {
        source: source.to_path_buf(),
        target: target.to_path_buf(),
        dry_run: false,
        found,
        adopted: Some(adopted),
    })
}

/// Copies a snapshot's rows into a freshly migrated Leteo database.
fn translate(snapshot: &Path, target: &Path) -> Result<()> {
    // Opening the store builds Leteo's schema at its current version.
    let store = crate::store::Store::open(crate::store::StoreConfig::new(target.to_path_buf()))
        .with_context(|| format!("prepare {}", target.display()))?;
    let connection = store.connection();
    connection
        .execute(
            "ATTACH DATABASE ?1 AS engram",
            [snapshot.to_string_lossy().as_ref()],
        )
        .context("attach the Engram snapshot")?;

    let mut carried_a_project = Vec::new();
    for (theirs, ours) in TABLE_MAP {
        // Only the columns both sides have. Engram's own schema changed across
        // its releases, so a fixed list would fail against whichever version
        // someone happens to be leaving.
        let Some(columns) = shared_columns(connection, theirs, ours)? else {
            continue;
        };
        if columns.is_empty() {
            continue;
        }
        if columns.iter().any(|name| name == "project") {
            carried_a_project.push(*ours);
        }
        let names = columns.join(", ");
        connection
            .execute_batch(&format!(
                "INSERT OR IGNORE INTO main.{ours} ({names}) SELECT {names} FROM engram.{theirs};"
            ))
            .map_err(|error| anyhow::anyhow!("copy {theirs} into {ours}: {error}"))?;
    }
    normalize_projects(connection, &carried_a_project)?;

    // The triggers only fire on writes made through them, so the index has to
    // be built from what has just landed.
    connection.execute_batch(
        "INSERT INTO observations_fts(observations_fts) VALUES('rebuild');
         INSERT INTO prompts_fts(prompts_fts) VALUES('rebuild');",
    )?;
    connection.execute_batch("DETACH DATABASE engram")?;
    Ok(())
}

/// Folds adopted project names into the spelling every query looks for.
///
/// The store's convention is that `project` holds what `normalize::project`
/// produces — lowercase, with runs of `-` and `_` collapsed — and almost every
/// statement compares it the raw way, `ifnull(project, '') = ?1`, against a
/// value that has been through that function. Migration `0004` made the
/// convention true of the rows that existed when it ran.
///
/// It could not make it true of rows that arrive later, and this is where they
/// arrive. Adoption copies Engram's columns across verbatim, and Engram never
/// normalised; the migration also runs against the empty database this builds,
/// so it is long past by the time anything lands. A memory adopted as
/// `MyProject` would sit beside queries looking for `myproject` and go unread —
/// including by `find_candidates`, which is what `mem_save` uses to notice a
/// contradiction. That failure reports `candidates: []`: not "nothing here
/// disagrees" but "nothing was looked at".
///
/// So the fold happens here, at the one door rows come in by, rather than in a
/// migration that would have to be written again for the next adoption.
///
/// `UPDATE OR IGNORE` because two spellings may fold onto one another where the
/// column is unique. Losing the duplicate is the intent: every surface that
/// takes a project name normalises it, so Leteo has always treated `MyProject`
/// and `myproject` as one project — it just could not read one of them.
fn normalize_projects(connection: &Connection, tables: &[&str]) -> Result<()> {
    for table in tables {
        let spellings: Vec<String> = connection
            .prepare(&format!(
                "SELECT DISTINCT project FROM main.{table} WHERE project IS NOT NULL"
            ))?
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        for spelling in spellings {
            let normalized = crate::memory::normalize::project(&spelling);
            if normalized == spelling {
                continue;
            }
            connection
                .execute(
                    &format!("UPDATE OR IGNORE main.{table} SET project = ?1 WHERE project = ?2"),
                    rusqlite::params![normalized, spelling],
                )
                .map_err(|error| anyhow::anyhow!("normalise {table}.project: {error}"))?;
        }
    }
    Ok(())
}

/// The columns a pair of tables share, or `None` when either side lacks one.
fn shared_columns(
    connection: &Connection,
    theirs: &str,
    ours: &str,
) -> Result<Option<Vec<String>>> {
    let columns = |schema: &str, table: &str| -> Result<Vec<String>> {
        // `PRAGMA <schema>.table_info` is the form that honours the schema.
        // The table-valued `schema.pragma_table_info(...)` reads the main
        // database whatever prefix it is given, which made every intersection
        // return our own columns and ask Engram for ones it never had.
        let mut statement = connection.prepare(&format!("PRAGMA {schema}.table_info({table})"))?;
        let names = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(names)
    };
    let theirs = columns("engram", theirs)?;
    let ours = columns("main", ours)?;
    if theirs.is_empty() || ours.is_empty() {
        return Ok(None);
    }
    // The intersection is what keeps somebody else's column names out of the
    // SQL.
    //
    // These names are pasted into an `INSERT ... SELECT` that goes through
    // `execute_batch`, which runs every statement it is handed, so a column
    // called `x); DROP TABLE observations; --` in an adopted file would be four
    // statements. Every name that survives the filter is one both schemas have,
    // which means one Leteo wrote itself.
    //
    // Which side is iterated does not matter — the set is the same either way,
    // and a first draft of the comment here claimed otherwise. What matters is
    // that the filter is there at all, so the test below hands adoption exactly
    // that column and was checked against dropping the filter rather than
    // against reordering it.
    Ok(Some(
        ours.into_iter()
            .filter(|name| theirs.contains(name))
            .collect(),
    ))
}

/// Counts the rows of the Leteo database an adoption produced.
///
/// The table names here are Leteo's, where [`inspect`] uses Engram's. Most of
/// them coincide, and `prompts` is the one that does not — which is the whole
/// reason the counting mechanics are shared but the names are not.
fn read_counts(database: &Path) -> Result<Counts> {
    let count = open_counter(database)?;
    Ok(Counts {
        sessions: count("sessions"),
        observations: count("observations"),
        prompts: count("prompts"),
        relations: count("memory_relations"),
    })
}

/// Opens a database read-only and hands back a counter for its tables.
///
/// Read-only so a running Engram is never disturbed, and so this can never be
/// the thing that damages the data being rescued. A table that is not there
/// counts as zero rather than failing: the two callers read different schemas,
/// and neither is expected to hold all of the other's tables.
fn open_counter(database: &Path) -> Result<impl Fn(&str) -> i64> {
    let connection = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open {}", database.display()))?;
    Ok(move |table: &str| -> i64 {
        connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap_or(0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A column name out of somebody else's file never reaches the SQL.
    ///
    /// Adoption is the one path whose job is to read a database Leteo did not
    /// write, and the copy it builds interpolates a column list into
    /// `execute_batch`, which runs every statement it is given. A name like
    /// `x); DROP TABLE observations; --` would be four statements.
    ///
    /// It cannot happen because of the intersection: a name survives only if
    /// both schemas have it, so it is a name Leteo wrote itself. Which side is
    /// iterated makes no difference — the set is the same — and this test was
    /// checked by removing the filter rather than by reversing it, because
    /// reversing it passes and would have been a guard proving nothing.
    #[test]
    fn a_column_name_from_the_adopted_file_never_reaches_the_sql() {
        let hostile = "x); DROP TABLE observations; --";
        let temp = tempfile::tempdir().unwrap();
        let ours = temp.path().join("leteo.db");
        let theirs = temp.path().join("engram.db");

        // Their file: one column Leteo also has, and one nobody should repeat.
        let foreign = Connection::open(&theirs).unwrap();
        foreign
            .execute_batch(&format!(
                "CREATE TABLE observations (id INTEGER, title TEXT, \"{hostile}\" TEXT);"
            ))
            .unwrap();
        drop(foreign);

        let connection = Connection::open(&ours).unwrap();
        connection
            .execute_batch("CREATE TABLE observations (id INTEGER, title TEXT);")
            .unwrap();
        connection
            .execute(
                "ATTACH DATABASE ?1 AS engram",
                [theirs.to_string_lossy().as_ref()],
            )
            .unwrap();

        let shared = shared_columns(&connection, "observations", "observations")
            .unwrap()
            .expect("both tables exist");
        assert!(
            shared.iter().all(|name| name == "id" || name == "title"),
            "a name from the adopted file came back: {shared:?}"
        );
        assert!(
            !shared.iter().any(|name| name.contains("DROP")),
            "{shared:?}"
        );
        // And the shared ones are actually found, or this passes on an empty
        // answer while proving nothing.
        assert_eq!(shared.len(), 2, "{shared:?}");
    }

    /// Builds a database shaped like Engram's, with rows in it.
    ///
    /// Deliberately uses Engram's names, including `user_prompts`, so the
    /// translation is exercised rather than assumed.
    fn engram_database(path: &Path, observations: i64) {
        engram_database_for(path, observations, "proj");
    }

    fn engram_database_for(path: &Path, observations: i64, project: &str) {
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch(&format!(
                "CREATE TABLE sessions (id TEXT PRIMARY KEY, project TEXT NOT NULL,
                     directory TEXT NOT NULL, started_at TEXT NOT NULL,
                     ended_at TEXT, summary TEXT);
                 CREATE TABLE observations (id INTEGER PRIMARY KEY AUTOINCREMENT,
                     sync_id TEXT, session_id TEXT NOT NULL, type TEXT NOT NULL,
                     title TEXT NOT NULL, content TEXT NOT NULL, tool_name TEXT,
                     project TEXT, scope TEXT NOT NULL DEFAULT 'project',
                     created_at TEXT NOT NULL DEFAULT (datetime('now')),
                     updated_at TEXT NOT NULL DEFAULT (datetime('now')));
                 CREATE TABLE user_prompts (id INTEGER PRIMARY KEY AUTOINCREMENT,
                     session_id TEXT NOT NULL, content TEXT NOT NULL, project TEXT,
                     created_at TEXT NOT NULL DEFAULT (datetime('now')));
                 CREATE TABLE memory_relations (id INTEGER PRIMARY KEY AUTOINCREMENT,
                     sync_id TEXT UNIQUE, source_id TEXT, target_id TEXT,
                     relation TEXT NOT NULL, judgment_status TEXT NOT NULL,
                     created_at TEXT NOT NULL DEFAULT (datetime('now')),
                     updated_at TEXT NOT NULL DEFAULT (datetime('now')));
                 INSERT INTO sessions (id, project, directory, started_at)
                     VALUES ('s1', '{project}', '/tmp/proj', datetime('now'));
                 INSERT INTO user_prompts (session_id, content, project)
                     VALUES ('s1', 'why is it slow?', '{project}');
                 INSERT INTO memory_relations (sync_id, source_id, target_id, relation, judgment_status)
                     VALUES ('rel-1', 'obs-1', 'obs-2', 'related', 'judged');"
            ))
            .unwrap();
        for index in 0..observations {
            connection
                .execute(
                    "INSERT INTO observations (sync_id, session_id, type, title, content, project)
                     VALUES (?1, 's1', 'decision', ?2, 'body', ?3)",
                    rusqlite::params![format!("obs-{index}"), format!("memory {index}"), project],
                )
                .unwrap();
        }
    }

    #[test]
    fn an_engram_database_is_translated_whole_including_its_relations() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("engram.db");
        let target = temp.path().join("leteo.db");
        engram_database(&source, 5);

        let report = adopt(&source, &target, false).unwrap();
        let adopted = report.adopted.unwrap();
        assert_eq!(adopted.observations, 5);
        assert_eq!(adopted.sessions, 1);
        assert_eq!(adopted.prompts, 1);
        // The relation is the whole point: an export/import would lose it.
        assert_eq!(adopted.relations, 1);

        // And the source is untouched, so the user can go back.
        assert_eq!(inspect(&source).unwrap().observations, 5);
    }

    /// Adoption is the door migration `0004` could not stand at.
    ///
    /// It folded the project column of the rows that existed when it ran, and
    /// it runs here too — against the empty database `translate` opens, before
    /// a single row has landed. Everything Engram hands over arrives after it.
    #[test]
    fn an_adopted_project_arrives_spelled_the_way_every_query_asks_for_it() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("engram.db");
        let target = temp.path().join("leteo.db");
        engram_database_for(&source, 5, "My--Project");

        adopt(&source, &target, false).unwrap();

        // What the caller asks with: every surface normalises before querying,
        // so this is the only spelling the store is ever asked for.
        let normalized = crate::memory::normalize::project("My--Project");
        assert_eq!(normalized, "my-project");

        let store = crate::store::Store::open(crate::store::StoreConfig::new(target)).unwrap();
        assert_eq!(store.count_observations(Some(&normalized)).unwrap(), 5);

        // And in the tables the raw-way comparisons read, which is most of
        // them — `find_candidates` among them, so a save against an adopted
        // project can still notice it contradicts something.
        let connection = store.connection();
        for table in ["observations", "sessions", "prompts"] {
            let spellings: Vec<String> = connection
                .prepare(&format!("SELECT DISTINCT project FROM {table}"))
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert_eq!(spellings, vec![normalized.clone()], "{table}");
        }
    }

    #[test]
    fn the_adopted_database_uses_leteos_names_not_engrams() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("engram.db");
        let target = temp.path().join("leteo.db");
        engram_database(&source, 2);
        adopt(&source, &target, false).unwrap();

        let connection = Connection::open(&target).unwrap();
        let tables = connection
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            tables.iter().any(|name| name == "prompts"),
            "the prompt table should carry Leteo's name: {tables:?}"
        );
        assert!(
            !tables.iter().any(|name| name == "user_prompts"),
            "Engram's name must not survive the translation: {tables:?}"
        );
        // And the rows arrived under the new name.
        let prompts: i64 = connection
            .query_row("SELECT COUNT(*) FROM prompts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(prompts, 1);
    }

    /// Everything the source held is in the store afterwards, counted.
    ///
    /// `adopt` compares the target's counts to the source's and refuses when
    /// they differ — the last line of defence against a translation that drops
    /// rows quietly, which in a migration is the only kind of loss there is.
    /// A mutation deleting that comparison survived the whole suite, because
    /// nothing asserted the property it guards.
    ///
    /// Asserted on all four entities rather than on the memories alone. Three
    /// of them travel through different statements, and the one that was
    /// silently lost for a while was relations: an export carried them and a
    /// backup did not.
    #[test]
    fn every_kind_of_row_survives_being_adopted() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("engram.db");
        let target = temp.path().join("leteo.db");
        engram_database(&source, 7);

        let before = inspect(&source).unwrap();
        assert_eq!(
            (
                before.sessions,
                before.observations,
                before.prompts,
                before.relations
            ),
            (1, 7, 1, 1),
            "the fixture has to hold one of each, or this proves nothing"
        );

        adopt(&source, &target, false).unwrap();

        let after = read_counts(&target).unwrap();
        assert_eq!(after.sessions, before.sessions, "sessions");
        assert_eq!(after.observations, before.observations, "observations");
        assert_eq!(after.prompts, before.prompts, "prompts");
        assert_eq!(after.relations, before.relations, "relations");
    }

    /// A row the translation cannot place stops the adoption loudly.
    ///
    /// Tables are copied with `INSERT OR IGNORE`, which is what makes the copy
    /// idempotent — and the reason to check what that ignores. It skips a
    /// duplicate silently, which is the point; it does **not** skip a foreign
    /// key violation, which fails the whole statement instead. So an
    /// observation pointing at a session that is not there — legal in Engram's
    /// schema, which never declared the constraint — aborts the migration
    /// naming the table rather than being quietly left behind.
    ///
    /// Worth pinning because the two outcomes are a line apart in the SQLite
    /// documentation and opposite in consequence: the migration either refuses
    /// or loses a memory, and nothing else here tests a *row* rather than a
    /// whole table.
    #[test]
    fn a_memory_the_schema_refuses_fails_the_adoption_instead_of_vanishing() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("engram.db");
        let target = temp.path().join("leteo.db");
        engram_database(&source, 3);
        Connection::open(&source)
            .unwrap()
            .execute(
                "INSERT INTO observations (sync_id, session_id, type, title, content, project)
                 VALUES ('obs-orphan', 'a-session-that-is-not-there', 'decision',
                         'An orphan', 'body', 'proj')",
                [],
            )
            .unwrap();
        assert_eq!(inspect(&source).unwrap().observations, 4);

        let refused = adopt(&source, &target, false).unwrap_err().to_string();
        assert!(
            refused.contains("copy observations"),
            "the failure has to name what could not be carried: {refused}"
        );

        // And the ordinary source still adopts, so this refuses a broken row
        // rather than anything unusual.
        let clean = temp.path().join("clean.db");
        let clean_target = temp.path().join("clean-leteo.db");
        engram_database(&clean, 3);
        adopt(&clean, &clean_target, false).unwrap();
    }

    #[test]
    fn adopted_memories_are_searchable() {
        // Translation writes rows straight into the tables, so the full-text
        // triggers never fire for them; without a rebuild every inherited
        // memory would be invisible.
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("engram.db");
        let target = temp.path().join("leteo.db");
        engram_database(&source, 3);
        adopt(&source, &target, false).unwrap();

        let store = crate::store::Store::open(crate::store::StoreConfig::new(target)).unwrap();
        let found = store
            .search("memory", crate::SearchOptions::default())
            .unwrap();
        assert_eq!(found.len(), 3, "inherited memories must be searchable");
    }

    #[test]
    fn a_dry_run_reports_what_it_would_take_and_writes_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("engram.db");
        let target = temp.path().join("leteo.db");
        engram_database(&source, 3);

        let report = adopt(&source, &target, true).unwrap();
        assert!(report.dry_run);
        assert_eq!(report.found.observations, 3);
        assert!(report.adopted.is_none());
        assert!(!target.exists(), "a dry run must not create the target");
    }

    #[test]
    fn adopting_over_a_populated_database_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("engram.db");
        let target = temp.path().join("leteo.db");
        engram_database(&source, 2);
        // The target already holds someone's memories.
        engram_database(&target, 7);

        let error = adopt(&source, &target, false).unwrap_err().to_string();
        assert!(
            error.contains("already holds 7 observations"),
            "the refusal should say what it found: {error}"
        );
        // Nothing was touched.
        assert_eq!(inspect(&target).unwrap().observations, 7);
    }

    #[test]
    fn an_empty_engram_database_is_not_worth_adopting() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("engram.db");
        engram_database(&source, 0);
        let connection = Connection::open(&source).unwrap();
        connection
            .execute_batch("DELETE FROM sessions; DELETE FROM user_prompts;")
            .unwrap();
        drop(connection);

        let error = adopt(&source, &temp.path().join("leteo.db"), false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("no memories"), "unexpected error: {error}");
    }

    #[test]
    fn a_missing_database_is_reported_by_path() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("nowhere.db");
        let error = inspect(&missing).unwrap_err().to_string();
        assert!(error.contains("no Engram database at"), "{error}");
    }

    #[test]
    fn a_target_nobody_can_read_is_kept_rather_than_replaced() {
        // "I could not read it" is not "it is empty". A database locked by a
        // running Leteo, or truncated by a write that did not finish, counted
        // as nothing and was deleted a few lines later while the command
        // reported success. A file nobody can read is the case where keeping
        // it matters most: it is the one somebody may still recover from.
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("engram.db");
        engram_database(&source, 2);
        let target = temp.path().join("leteo.db");
        let theirs = b"not a database, but it is theirs".repeat(50);
        std::fs::write(&target, &theirs).unwrap();

        let error = adopt(&source, &target, false).unwrap_err().to_string();

        assert!(error.contains("cannot be read"), "{error}");
        assert!(
            error.contains("Move it aside"),
            "the refusal has to say what to do about it: {error}"
        );
        assert_eq!(
            std::fs::read(&target).unwrap(),
            theirs,
            "and not a byte of it may have been touched"
        );
    }

    #[test]
    fn an_empty_target_is_still_adopted_into() {
        // The refusal must not swallow the ordinary case: a store that exists
        // and holds nothing is exactly what a fresh install looks like.
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("engram.db");
        engram_database(&source, 2);
        let target = temp.path().join("leteo.db");
        crate::store::Store::open(crate::store::StoreConfig::new(&target)).unwrap();

        let adoption = adopt(&source, &target, false).unwrap();

        assert_eq!(adoption.adopted.unwrap().observations, 2);
    }
    /// Pointing Leteo at Engram's database says so, and says what to do.
    ///
    /// Found on a real one: a backup from July, 3,223 memories, opened with
    /// `leteo doctor --database`. The answer was `no such table: prompts` — an
    /// internal name, a SQLite error code, and no mention of the one command that
    /// exists for exactly this file.
    ///
    /// The cause is that `user_version = 1` means two things. Leteo stamps 1 on a
    /// database it has converged to its own baseline; Engram stamps 1 on its own
    /// schema. `migrate` documents adoption as covering "a brand new file, a Leteo
    /// database from before versioning, and an Engram one" — and it does, for one
    /// carrying no version at all. A real Engram database carries 1, skips
    /// adoption, and gets Leteo's migrations run against Engram's tables.
    ///
    /// So the shape decides it, not the stamp. And it is refused rather than
    /// converged: `leteo import --from-engram` snapshots the source and writes into
    /// a Leteo store, leaving Engram's own file alone. Rewriting another program's
    /// database because somebody passed `--database` by mistake is not a thing to
    /// do on their behalf.
    #[test]
    fn an_engram_database_is_named_rather_than_migrated() {
        let temp = tempfile::TempDir::new().unwrap();

        // Stamped the way Engram stamps it, which is the case that used to fall
        // through: unstamped ones were already adopted.
        let theirs = temp.path().join("engram.db");
        engram_database(&theirs, 3);
        Connection::open(&theirs)
            .unwrap()
            .execute_batch("PRAGMA user_version = 1")
            .unwrap();

        let said = match crate::store::Store::open(crate::store::StoreConfig::new(theirs.clone())) {
            Ok(_) => panic!("Engram's database is not a Leteo one"),
            Err(error) => error.to_string(),
        };
        assert!(
            said.contains("Engram database") && said.contains("import --from-engram"),
            "the refusal has to name what the file is and what to do with it: {said}"
        );
        assert!(
            !said.contains("no such table"),
            "and not leak the first table a migration happened to reach: {said}"
        );

        // Untouched, which is the reason for refusing rather than converging.
        let still_theirs: bool = Connection::open(&theirs)
            .unwrap()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='user_prompts')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(still_theirs, "refusing must not have rewritten their file");

        // The positive control, without which this guard would pass by refusing
        // everything: adoption still reads that database, and the store it writes
        // into still opens.
        let ours = temp.path().join("leteo.db");
        crate::engram::adopt(&theirs, &ours, false).expect("adoption reads Engram's database");
        let store = crate::store::Store::open(crate::store::StoreConfig::new(ours))
            .expect("and the store it wrote opens");
        assert_eq!(
            store.stats().unwrap().total_observations,
            3,
            "with the memories in it"
        );
    }
}
