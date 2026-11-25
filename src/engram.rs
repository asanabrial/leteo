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
///
/// The order is the copy order, and a table that references another has to come
/// after it: `sync_mutations.target_key` points at `sync_state`, and `INSERT OR
/// IGNORE` does not defer a foreign key the way it ignores a duplicate, so the
/// pair the other way round failed the whole adoption on the first Engram
/// database that had actually synced.
const TABLE_MAP: &[(&str, &str)] = &[
    ("sessions", "sessions"),
    ("observations", "observations"),
    (ENGRAM_PROMPTS, "prompts"),
    ("memory_relations", "memory_relations"),
    ("sync_chunks", "sync_chunks"),
    ("sync_state", "sync_state"),
    ("sync_mutations", "sync_mutations"),
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
    /// Source tables and columns the translation carried nothing for.
    ///
    /// Empty on a dry run, which writes nothing and so drops nothing. On a real
    /// adoption it is the answer rule 5 asks for: a table Leteo has no
    /// counterpart for, or a column it does not read, is named here rather than
    /// left to be discovered when a peer resurrects a memory the source had
    /// deleted.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dropped: Vec<Dropped>,
}

/// A source table or column an adoption did not carry.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Dropped {
    /// The table, named the way the source names it.
    pub table: String,
    /// Columns the source had that Leteo has no counterpart for. Empty means
    /// the whole table was left behind.
    pub columns: Vec<String>,
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
///
/// The copy runs in one `BEGIN IMMEDIATE` on the target, so it either lands
/// whole or not at all — which is also what makes a second run possible. The
/// earlier shape committed each statement on its own and deleted the target
/// first: a failure half way left a store nothing could retry, and a target
/// whose `observations` happened to be empty was thrown away with the prompts
/// and sessions it did hold.
pub fn adopt(source: &Path, target: &Path, dry_run: bool) -> Result<Adoption> {
    let found = inspect(source)?;
    if found.is_empty() {
        bail!(
            "the Engram database at {} holds no memories; nothing to adopt",
            source.display()
        );
    }

    if dry_run {
        // Nothing is opened, so nothing is created, and the target is read
        // through a read-only connection: a dry run pointed at a store by
        // mistake cannot disturb it.
        if target.is_file() {
            let occupied = occupied_tables_read_only(target)?;
            refuse_if_occupied(target, &occupied)?;
        }
        return Ok(Adoption {
            source: source.to_path_buf(),
            target: target.to_path_buf(),
            dry_run: true,
            found,
            adopted: None,
            dropped: Vec::new(),
        });
    }

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }

    // Opening the target is what builds Leteo's schema at its current version
    // for a file that does not exist yet. Nothing is deleted — not the file,
    // not a stale sidecar — because whatever is here is opened as it is and the
    // emptiness check inside the transaction is what decides whether it may be
    // written.
    let mut store = open_target(target)?;

    let snapshot = PathBuf::from(format!("{}.adopting", target.display()));
    let _ = std::fs::remove_file(&snapshot);
    {
        let reader = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("open {}", source.display()))?;
        reader
            .execute("VACUUM INTO ?1", [snapshot.to_string_lossy().as_ref()])
            .with_context(|| format!("snapshot {}", source.display()))?;
    }
    let translated = translate(&mut store, &snapshot, target);
    let _ = std::fs::remove_file(&snapshot);
    let (dropped, adopted) = translated?;

    Ok(Adoption {
        source: source.to_path_buf(),
        target: target.to_path_buf(),
        dry_run: false,
        found,
        adopted: Some(adopted),
        dropped,
    })
}

/// Opens the target as a Leteo store, or says why it cannot be one.
///
/// "I could not read it" is not "it is empty". The old guard treated a target
/// it could not read — a database locked by a running Leteo, or truncated by a
/// write that did not finish — as one holding nothing and deleted it while the
/// command reported success. A file nobody can read is the case where keeping
/// it matters most, because it is the one somebody may still recover from.
fn open_target(target: &Path) -> Result<crate::store::Store> {
    match crate::store::Store::open(crate::store::StoreConfig::new(target.to_path_buf())) {
        Ok(store) => Ok(store),
        Err(error) if target.is_file() => bail!(
            "{} exists but cannot be read as a Leteo database — it may be open in \
             another process, or damaged. Move it aside first, or point \
             --database somewhere else. ({error})",
            target.display()
        ),
        Err(error) => Err(error.into()),
    }
}

/// Names every mapped Leteo table that already holds rows, with how many.
///
/// Derived from [`TABLE_MAP`] rather than written out again, so a table added
/// there is guarded here too. `sync_state` is skipped: the baseline seeds it
/// with one row for the cloud target in every store, including one just
/// created, so it cannot tell a used store from an empty one. The content
/// tables can, and they are what a merge would fold two histories into.
fn occupied_tables(connection: &Connection) -> Result<Vec<(String, i64)>> {
    // In `TABLE_MAP` order — dependency order, so sessions are named before the
    // prompts and memories filed under them — with the duplicates a shared
    // Leteo name would produce removed.
    let mut tables: Vec<&str> = Vec::new();
    for (_, ours) in TABLE_MAP {
        if !tables.contains(ours) {
            tables.push(ours);
        }
    }
    let mut occupied = Vec::new();
    for table in tables {
        if table == "sync_state" || !table_exists(connection, table)? {
            continue;
        }
        let count: i64 =
            connection.query_row(&format!("SELECT COUNT(*) FROM main.{table}"), [], |row| {
                row.get(0)
            })?;
        if count > 0 {
            occupied.push((table.to_owned(), count));
        }
    }
    Ok(occupied)
}

fn occupied_tables_read_only(target: &Path) -> Result<Vec<(String, i64)>> {
    let connection = Connection::open_with_flags(target, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open {}", target.display()))?;
    occupied_tables(&connection).with_context(|| format!("read {}", target.display()))
}

/// Refuses a target that already holds memories, naming what is there.
///
/// Merging into a non-empty store is out of scope, and refusing beats a
/// silent replacement: there is no safe way to fold two histories together.
fn refuse_if_occupied(target: &Path, occupied: &[(String, i64)]) -> Result<()> {
    if occupied.is_empty() {
        return Ok(());
    }
    let held = occupied
        .iter()
        .map(|(table, count)| format!("{count} {table}"))
        .collect::<Vec<_>>()
        .join(", ");
    bail!(
        "{} already holds {held}; move it aside first, because adopting into a \
         store that already holds memories is refused rather than merged",
        target.display()
    )
}

fn table_exists(connection: &Connection, name: &str) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [name],
        |row| row.get(0),
    )?)
}

/// Copies a snapshot's rows into the target in one transaction, and reports
/// what it had no place for and what landed.
///
/// Everything between `BEGIN IMMEDIATE` and `COMMIT` is one result: either the
/// whole adoption lands or none of it does. The transaction is why a target
/// left half filled no longer exists, and therefore why a second run is
/// possible at all.
fn translate(
    store: &mut crate::store::Store,
    snapshot: &Path,
    target: &Path,
) -> Result<(Vec<Dropped>, Counts)> {
    // Read before the write transaction borrows the store: every normaliser
    // below bounds text by the same number the local door uses.
    let max_content_bytes = store.max_observation_length();

    // `ATTACH` cannot run inside a transaction, so it comes first — and before
    // the write lock, so the lock is held for the copy rather than the attach.
    store
        .connection()
        .execute(
            "ATTACH DATABASE ?1 AS engram",
            [snapshot.to_string_lossy().as_ref()],
        )
        .context("attach the Engram snapshot")?;

    let tx = store.write_transaction()?;

    // Under the write lock, so a second process cannot slip rows in between
    // this question and the copy that answers it.
    let occupied = occupied_tables(&tx)?;
    refuse_if_occupied(target, &occupied)?;

    // The indexes are built once at the end rather than row by row, which is
    // what a copy of any size spends its time on: every insert otherwise fires
    // three triggers, each tokenising a title and a body through `porter
    // unicode61`. The JSON import measures the same trade at nine times for
    // 4,013 memories — 13.3 seconds against 0.6 to write the rows and 0.9 to
    // rebuild — and an adoption carries the same rows through the same
    // triggers.
    //
    // SQLite does fire `AFTER INSERT` row triggers for `INSERT ... SELECT`, so
    // the copy would be indexed as it went without this. An earlier comment
    // here claimed they never fire for adopted rows and justified a hand-built
    // rebuild with it; both are gone. Dropping the triggers is what the rebuild
    // below pays for.
    let dropped_triggers: Vec<&str> = crate::store::schema::FULL_TEXT_TRIGGERS
        .iter()
        .copied()
        .filter(|name| {
            tx.execute_batch(&format!("DROP TRIGGER IF EXISTS {name};"))
                .is_ok()
        })
        .collect();

    let mut carried_a_project = Vec::new();
    for (theirs, ours) in TABLE_MAP {
        // Only the columns both sides have. Engram's own schema changed across
        // its releases, so a fixed list would fail against whichever version
        // someone happens to be leaving.
        let Some(columns) = shared_columns(&tx, theirs, ours)? else {
            continue;
        };
        if columns.is_empty() {
            continue;
        }
        if columns.iter().any(|name| name == "project") {
            carried_a_project.push(*ours);
        }
        let names = columns.join(", ");
        // Engram keeps a quarantined or superseded mutation out of transport,
        // and Leteo's transport is every row whose `acked_at` is null. Copying
        // them unchanged put them back on the wire: what Engram had held back
        // was offered to a peer again, and a hard delete it had already
        // resolved could be replayed. `disposition` is Engram's own column and
        // is read only where it exists — older Engram schemas predate it, and
        // the column is named here rather than interpolated from the file.
        let quarantined =
            *theirs == "sync_mutations" && has_column(&tx, "engram", theirs, "disposition")?;
        let filter = if quarantined {
            " WHERE disposition IS NULL OR disposition NOT IN ('quarantined', 'superseded')"
        } else {
            ""
        };
        tx.execute_batch(&format!(
            "INSERT OR IGNORE INTO main.{ours} ({names}) SELECT {names} FROM engram.{theirs}{filter};"
        ))
        .map_err(|error| anyhow::anyhow!("copy {theirs} into {ours}: {error}"))?;
    }
    let dropped = dropped_tables(&tx)?;
    normalize_projects(&tx, &carried_a_project)?;
    normalize_sessions(&tx, max_content_bytes)?;
    normalize_observations(&tx, max_content_bytes)?;
    normalize_prompts(&tx, max_content_bytes)?;
    normalize_relations(&tx, max_content_bytes)?;

    // Against the snapshot, not the live source: the snapshot is the bytes that
    // were copied, and the live file may have moved on since. A row an `INSERT
    // OR IGNORE` skipped — a duplicate the target's unique index refuses — is a
    // mismatch, and a mismatch aborts the whole adoption rather than committing
    // a store that is missing memories and looks complete.
    let expected = engram_counts(&tx)?;
    let adopted = leteo_counts(&tx)?;
    if adopted != expected {
        bail!("the adoption does not match the source: found {expected:?}, adopted {adopted:?}");
    }

    // In this order and not the other: the rebuild begins by writing the stems
    // of every memory that has none, and with the triggers already restored each
    // of those writes would also be indexed one row at a time, which is the cost
    // dropping them was for. What must not happen is committing without both.
    crate::store::schema::rebuild_present_indexes(&tx)?;
    for name in dropped_triggers {
        if let Some(sql) = crate::store::schema::full_text_trigger_sql(name) {
            tx.execute_batch(sql)?;
        }
    }

    tx.commit()?;
    store.connection().execute_batch("DETACH DATABASE engram")?;
    Ok((dropped, adopted))
}

/// Folds the observations an adoption carried onto the rules every other door
/// applies.
///
/// Adoption copies columns verbatim from a program Leteo does not control, so
/// the type is whatever Engram stored — `bug`, `manual`, `learning` — and a
/// search narrowed by `bugfix` or `discovery` never returns it. `doctor`'s
/// `observation_type_searchable` is what the omission looked like from outside:
/// a healthy-looking store holding memories no filtered search can reach. The
/// project fold above is the same act for a different column.
///
/// The text was copied unread for as long as the type was. A `<private>` span
/// Engram had not redacted came across verbatim and was then printed into every
/// context block and preview, and a body over the published bound was stored at
/// whatever size Engram held — the JSON import has applied both since #412, and
/// this was the remaining door. The title fold matters for the same reason it
/// does at the other doors: a newline in one ends the bullet it is printed in
/// and starts another, and what follows reads as a memory that does not exist.
///
/// The two derived values are recomputed here for the same reason, because the
/// copy carries Engram's: `review_after`, a function of the type and the day
/// the memory was written, and `normalized_hash`, taken of the body as the
/// store holds it, which is what dedupe compares. The hash is taken after the
/// text has been cut and redacted, so two runs of the same adoption agree and a
/// memory Engram duplicated is still seen as one.
fn normalize_observations(connection: &Connection, max_content_bytes: usize) -> Result<()> {
    let rows: Vec<(i64, String, String, String, Option<String>)> = {
        let mut statement = connection
            .prepare("SELECT id, type, title, content, created_at FROM main.observations")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for (id, kind, title, content, created_at) in rows {
        let kind = crate::memory::normalize::kind(&kind);
        let title = crate::memory::normalize::title(&title, max_content_bytes);
        let (content, hash) = crate::memory::normalize::stored_content(&content, max_content_bytes);
        let from = created_at
            .as_deref()
            .and_then(crate::timestamp::parse)
            .unwrap_or_else(|| chrono::Utc::now().naive_utc());
        let review = crate::memory::rules::review_after(&kind, from).map(crate::timestamp::format);
        connection.execute(
            "UPDATE main.observations
                SET type = ?1, title = ?2, content = ?3, normalized_hash = ?4, review_after = ?5
              WHERE id = ?6",
            rusqlite::params![kind, title, content, hash, review, id],
        )?;
    }
    Ok(())
}

/// The sessions an adoption carried, held to the rules their summaries and
/// directories get at every other door.
///
/// A summary is read back in every `mem_context`, which is why the JSON import
/// redacts and bounds it; a summary adopted here was copied as Engram held it,
/// so a token somebody had wrapped in the private marker was printed to every
/// agent that opened the project afterwards.
///
/// The directory is the sibling the import door already fixed: a relative
/// spelling names whatever place the reader happens to stand in, and the drift
/// check then resolves it against the wrong project. Engram writes an absolute
/// path today, so this changes nothing it produced and closes the case for a
/// store that did not.
fn normalize_sessions(connection: &Connection, max_content_bytes: usize) -> Result<()> {
    let rows: Vec<(String, Option<String>, String)> = {
        let mut statement =
            connection.prepare("SELECT id, summary, directory FROM main.sessions")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for (id, summary, directory) in rows {
        let summary =
            crate::memory::normalize::session_summary(summary.as_deref(), max_content_bytes);
        let directory = crate::project::recordable_directory(&directory);
        connection.execute(
            "UPDATE main.sessions SET summary = ?1, directory = ?2 WHERE id = ?3",
            rusqlite::params![summary, directory, id],
        )?;
    }
    Ok(())
}

/// The prompts an adoption carried, through the same helper the local and
/// replicated doors use.
///
/// A prompt is where a secret is most likely to be typed, so redaction is the
/// part that matters: a promise that only holds on the machine the prompt was
/// typed on is not one, and an adopted store held the span Engram had kept.
/// The project fold is repeated here rather than assumed from
/// `normalize_projects`, because that function skips a spelling its own fold
/// leaves alone and this one is one call for both fields.
fn normalize_prompts(connection: &Connection, max_content_bytes: usize) -> Result<()> {
    let rows: Vec<(i64, String, Option<String>)> = {
        let mut statement = connection.prepare("SELECT id, content, project FROM main.prompts")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for (id, content, project) in rows {
        let (content, project) = crate::memory::normalize::prompt_fields(
            &content,
            project.as_deref(),
            max_content_bytes,
        );
        connection.execute(
            "UPDATE main.prompts SET content = ?1, project = ?2 WHERE id = ?3",
            rusqlite::params![content, project, id],
        )?;
    }
    Ok(())
}

/// The relation verdicts an adoption carried, under the rules `judge_relation`
/// and `apply_relation_tx` already hold their writers to.
///
/// A confidence outside zero and one is stored as no confidence rather than as
/// a number no reader can interpret, and both texts are redacted and bounded:
/// a reason is printed beside the memory it is about, and an adopted one could
/// carry a `<private>` span that the original verdict had already lost.
fn normalize_relations(connection: &Connection, max_content_bytes: usize) -> Result<()> {
    type Row = (i64, Option<String>, Option<String>, Option<f64>);
    let rows: Vec<Row> = {
        let mut statement = connection
            .prepare("SELECT id, reason, evidence, confidence FROM main.memory_relations")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<f64>>(3)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for (id, reason, evidence, confidence) in rows {
        let reason = crate::memory::normalize::judgment_text(reason.as_deref(), max_content_bytes);
        let evidence =
            crate::memory::normalize::judgment_text(evidence.as_deref(), max_content_bytes);
        let confidence = confidence.filter(|value| crate::memory::rules::is_confidence(*value));
        connection.execute(
            "UPDATE main.memory_relations
                SET reason = ?1, evidence = ?2, confidence = ?3
              WHERE id = ?4",
            rusqlite::params![reason, evidence, confidence, id],
        )?;
    }
    Ok(())
}

/// The four entities counted in the attached Engram snapshot, under Engram's
/// names — `user_prompts`, not `prompts`.
///
/// A table missing from whichever Engram version this is counts as zero, the
/// same way `inspect` reads it, because the copy skips it for the same reason.
fn engram_counts(connection: &Connection) -> Result<Counts> {
    let count = |table: &str| -> i64 {
        connection
            .query_row(&format!("SELECT COUNT(*) FROM engram.{table}"), [], |row| {
                row.get(0)
            })
            .unwrap_or(0)
    };
    Ok(Counts {
        sessions: count("sessions"),
        observations: count("observations"),
        prompts: count(ENGRAM_PROMPTS),
        relations: count("memory_relations"),
    })
}

/// The same four, counted from the rows that just landed in the target.
fn leteo_counts(connection: &Connection) -> Result<Counts> {
    let count = |table: &str| -> Result<i64, rusqlite::Error> {
        connection.query_row(&format!("SELECT COUNT(*) FROM main.{table}"), [], |row| {
            row.get(0)
        })
    };
    Ok(Counts {
        sessions: count("sessions")?,
        observations: count("observations")?,
        prompts: count("prompts")?,
        relations: count("memory_relations")?,
    })
}

/// Every source table or column the translation carried nothing for.
///
/// Read from the attached source and from Leteo's own schema, so the report is
/// what the copy beside it actually left behind rather than a second guess at
/// it. A table with no [`TABLE_MAP`] counterpart is reported whole; a mapped
/// table reports the source columns Leteo does not read. Empty `columns` means
/// the table was left behind entirely.
fn dropped_tables(connection: &Connection) -> Result<Vec<Dropped>> {
    let mut names: Vec<String> = connection
        .prepare(
            "SELECT name FROM engram.sqlite_master
              WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        )?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    names.sort();
    let mut dropped = Vec::new();
    for theirs in names {
        // The name handed to `columns` is the constant `TABLE_MAP` matched on,
        // never the string read out of the adopted file: a name that does not
        // match a constant is reported whole and never looked up, so nothing
        // another program wrote reaches SQL here.
        let Some((mapped, ours)) = TABLE_MAP
            .iter()
            .find(|(source, _)| *source == theirs)
            .map(|(source, ours)| (*source, *ours))
        else {
            dropped.push(Dropped {
                table: theirs,
                columns: Vec::new(),
            });
            continue;
        };
        let source = columns(connection, "engram", mapped)?;
        let target = columns(connection, "main", ours)?;
        if source.is_empty() || target.is_empty() {
            dropped.push(Dropped {
                table: theirs,
                columns: Vec::new(),
            });
            continue;
        }
        let lost: Vec<String> = source
            .into_iter()
            .filter(|name| !target.contains(name))
            .collect();
        if !lost.is_empty() {
            dropped.push(Dropped {
                table: theirs,
                columns: lost,
            });
        }
    }
    Ok(dropped)
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
/// `UPDATE OR REPLACE`, not `OR IGNORE`, because two spellings may fold onto
/// one another where the column is unique — `sync_enrolled_projects` holds one
/// row per project. `OR IGNORE` skipped the conflicting row and left both
/// spellings in place, which is the opposite of what the sentence here used to
/// claim: every surface that takes a project name normalises it, so Leteo has
/// always treated `MyProject` and `myproject` as one project, and the fold is
/// where the duplicate stops being one. In a table whose project column is not
/// unique there is no conflict to replace and this is an ordinary update.
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
                    &format!("UPDATE OR REPLACE main.{table} SET project = ?1 WHERE project = ?2"),
                    rusqlite::params![normalized, spelling],
                )
                .map_err(|error| anyhow::anyhow!("normalise {table}.project: {error}"))?;
        }
    }
    Ok(())
}

/// The columns one schema's table has, in declaration order.
///
/// `PRAGMA <schema>.table_info` is the form that honours the schema. The
/// table-valued `schema.pragma_table_info(...)` reads the main database whatever
/// prefix it is given, which made every intersection return our own columns and
/// ask Engram for ones it never had.
fn columns(connection: &Connection, schema: &str, table: &str) -> Result<Vec<String>> {
    let mut statement = connection.prepare(&format!("PRAGMA {schema}.table_info({table})"))?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(names)
}

fn has_column(connection: &Connection, schema: &str, table: &str, name: &str) -> Result<bool> {
    Ok(columns(connection, schema, table)?
        .iter()
        .any(|column| column == name))
}

/// The columns a pair of tables share, or `None` when either side lacks one.
fn shared_columns(
    connection: &Connection,
    theirs: &str,
    ours: &str,
) -> Result<Option<Vec<String>>> {
    let theirs = columns(connection, "engram", theirs)?;
    let ours = columns(connection, "main", ours)?;
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
#[cfg(test)]
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
    fn an_adoption_does_not_rearm_what_engram_had_quarantined() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("engram.db");
        let target = temp.path().join("leteo.db");
        engram_database_for(&source, 3, "MyProj");
        {
            let connection = Connection::open(&source).unwrap();
            connection
                .execute_batch(
                    "ALTER TABLE user_prompts ADD COLUMN source_inbox_id TEXT;
                     CREATE TABLE sync_state (
                         target_key TEXT PRIMARY KEY,
                         lifecycle TEXT NOT NULL DEFAULT 'idle',
                         last_enqueued_seq INTEGER NOT NULL DEFAULT 0,
                         last_acked_seq INTEGER NOT NULL DEFAULT 0,
                         last_pulled_seq INTEGER NOT NULL DEFAULT 0,
                         consecutive_failures INTEGER NOT NULL DEFAULT 0,
                         updated_at TEXT NOT NULL DEFAULT (datetime('now')));
                     INSERT INTO sync_state (target_key) VALUES ('peer');
                     CREATE TABLE sync_mutations (
                         seq INTEGER PRIMARY KEY AUTOINCREMENT,
                         target_key TEXT NOT NULL,
                         entity TEXT NOT NULL,
                         entity_key TEXT NOT NULL,
                         op TEXT NOT NULL,
                         payload TEXT NOT NULL,
                         source TEXT NOT NULL DEFAULT 'local',
                         project TEXT NOT NULL DEFAULT '',
                         occurred_at TEXT NOT NULL DEFAULT (datetime('now')),
                         acked_at TEXT,
                         disposition TEXT NOT NULL DEFAULT 'pending');
                     INSERT INTO sync_mutations (target_key, entity, entity_key, op, payload, project, disposition)
                         VALUES ('peer', 'observation', 'obs-1', 'upsert', '{}', 'MyProj', 'pending'),
                                ('peer', 'observation', 'obs-2', 'upsert', '{}', 'MyProj', 'quarantined'),
                                ('peer', 'observation', 'obs-3', 'delete', '{}', 'MyProj', 'superseded');
                     CREATE TABLE sync_delete_tombstones (
                         sync_id TEXT PRIMARY KEY, entity TEXT NOT NULL, entity_key TEXT NOT NULL,
                         deleted_at TEXT NOT NULL DEFAULT (datetime('now')));
                     INSERT INTO sync_delete_tombstones (sync_id, entity, entity_key)
                         VALUES ('tomb-1', 'observation', 'obs-9');
                     CREATE TABLE sync_enrolled_projects (project TEXT PRIMARY KEY,
                         enrolled_at TEXT NOT NULL DEFAULT (datetime('now')));
                     INSERT INTO sync_enrolled_projects (project) VALUES ('MyProj'), ('myproj');",
                )
                .unwrap();
        }

        let report = adopt(&source, &target, false).unwrap();
        let connection = Connection::open(&target).unwrap();

        // What Engram had held back must not be offered to a peer again. Leteo's
        // transport is every row with a null `acked_at`, so a copied quarantine
        // is a mutation put back on the wire.
        let pending: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sync_mutations WHERE acked_at IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            pending, 1,
            "only the mutation Engram was still sending may be pending"
        );

        // And the fold leaves one spelling, not the two the source held.
        let enrolled: Vec<String> = connection
            .prepare("SELECT project FROM sync_enrolled_projects ORDER BY project")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            enrolled,
            vec!["myproj".to_owned()],
            "two spellings of one project must not survive the fold"
        );

        // And every table and column Leteo has no place for is named, rather
        // than skipped in silence.
        let dropped = &report.dropped;
        assert!(
            dropped
                .iter()
                .any(|item| item.table == "sync_delete_tombstones" && item.columns.is_empty()),
            "a hard-delete tombstone table is reported whole: {dropped:?}"
        );
        assert!(
            dropped.iter().any(|item| item.table == "sync_mutations"
                && item.columns.iter().any(|name| name == "disposition")),
            "the quarantine column is named: {dropped:?}"
        );
        assert!(
            dropped.iter().any(|item| item.table == "user_prompts"
                && item.columns.iter().any(|name| name == "source_inbox_id")),
            "the prompt inbox identity is named: {dropped:?}"
        );
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
        // The copy runs with the full-text triggers dropped, so a rebuild is
        // what puts these rows into the indexes. The triggers do fire for
        // `INSERT ... SELECT` — an earlier comment here claimed otherwise — so
        // this test is also what holds the rebuild in place: without it, every
        // inherited memory would be invisible.
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
        // The target is a real Leteo store that already holds someone's
        // memories. The old fixture here built an Engram-shaped file at the
        // target path, which is no longer a store adoption can open.
        {
            let mut store =
                crate::store::Store::open(crate::store::StoreConfig::new(target.clone())).unwrap();
            store.create_session("s1", "proj", "C:/repo").unwrap();
            for index in 0..3 {
                store
                    .add_observation(crate::AddObservation {
                        session_id: "s1".to_owned(),
                        kind: "discovery".to_owned(),
                        title: format!("memory {index}"),
                        content: "body".to_owned(),
                        tool_name: None,
                        project: Some("proj".to_owned()),
                        scope: "project".to_owned(),
                        topic_key: None,
                        prompt_sync_id: None,
                    })
                    .unwrap();
            }
        }

        let error = adopt(&source, &target, false).unwrap_err().to_string();
        assert!(
            error.contains("already holds") && error.contains("3 observations"),
            "the refusal should say what it found: {error}"
        );
        // Nothing was touched.
        let store = crate::store::Store::open(crate::store::StoreConfig::new(target)).unwrap();
        assert_eq!(store.stats().unwrap().total_observations, 3);
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

    /// The fast path checks the shape, not just the stamp.
    ///
    /// `prepare_once` skips `migrate` entirely when the stamp already equals
    /// `SCHEMA_VERSION`, so whatever that path accepts is opened without any of
    /// the checks in `migrate` — including the one that tells Engram's file
    /// apart. It therefore checks the shape itself, and this is the test that
    /// holds it.
    ///
    /// Stamped `SCHEMA_VERSION` rather than 1, and that is the whole point of
    /// having it beside the test below. While `SCHEMA_VERSION` was 1 the two
    /// were the same fixture: Engram stamps 1, so Engram's own file reached the
    /// fast path and covered the shape check by accident. Migration 18 moved
    /// the version to 18 and that coincidence ended — the check went uncovered
    /// without a single test changing, and the registered mutation that deletes
    /// it stopped being killed by anything. A fixture stamped at whatever this
    /// build understands cannot drift that way again.
    #[test]
    fn the_fast_path_refuses_a_file_stamped_current_that_is_not_leteos() {
        let temp = tempfile::TempDir::new().unwrap();
        let theirs = temp.path().join("engram.db");
        engram_database(&theirs, 3);
        Connection::open(&theirs)
            .unwrap()
            .execute_batch(&format!(
                "PRAGMA user_version = {}",
                crate::store::SCHEMA_VERSION
            ))
            .unwrap();

        let said = match crate::store::Store::open(crate::store::StoreConfig::new(theirs.clone())) {
            Ok(_) => panic!(
                "a file with no `prompts` table is not a Leteo store, whatever it is stamped"
            ),
            Err(error) => error.to_string(),
        };
        assert!(
            said.contains("Engram database") && said.contains("import --from-engram"),
            "the refusal has to name what the file is and what to do with it: {said}"
        );

        let still_theirs: bool = Connection::open(&theirs)
            .unwrap()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='user_prompts')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(still_theirs, "refusing must not have rewritten their file");
    }

    /// A failed adoption leaves the target exactly as it was, and runs again.
    ///
    /// The copy used to be one commit per statement, so a failure after the
    /// observations landed left them in a store the next run then refused as
    /// non-empty: a migration from Engram could strand a user half way with no
    /// way forward but deleting files by hand. The failure here is one the copy
    /// cannot survive — a `sync_mutations` row whose `target_key` has no
    /// `sync_state` parent, which the foreign key refuses — and it arrives
    /// after the observations, which is the point.
    #[test]
    fn a_failed_adoption_leaves_the_target_as_it_was_and_can_be_retried() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("engram.db");
        let target = temp.path().join("leteo.db");
        engram_database_for(&source, 3, "proj");
        {
            let connection = Connection::open(&source).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE sync_state (
                         target_key TEXT PRIMARY KEY,
                         lifecycle TEXT NOT NULL DEFAULT 'idle',
                         updated_at TEXT NOT NULL DEFAULT (datetime('now')));
                     CREATE TABLE sync_mutations (
                         seq INTEGER PRIMARY KEY AUTOINCREMENT,
                         target_key TEXT NOT NULL,
                         entity TEXT NOT NULL,
                         entity_key TEXT NOT NULL,
                         op TEXT NOT NULL,
                         payload TEXT NOT NULL,
                         project TEXT NOT NULL DEFAULT '',
                         occurred_at TEXT NOT NULL DEFAULT (datetime('now')));
                     INSERT INTO sync_mutations (target_key, entity, entity_key, op, payload, project)
                         VALUES ('ghost', 'observation', 'obs-1', 'upsert', '{}', 'proj');",
                )
                .unwrap();
        }

        let refused = adopt(&source, &target, false).unwrap_err().to_string();
        assert!(
            refused.contains("copy sync_mutations"),
            "the failure has to name what could not be carried: {refused}"
        );
        assert_eq!(
            read_counts(&target).unwrap().observations,
            0,
            "the observations the copy had already written have to be rolled back"
        );

        // Fixing the source is all that stands between here and a rerun.
        Connection::open(&source)
            .unwrap()
            .execute("INSERT INTO sync_state (target_key) VALUES ('ghost')", [])
            .unwrap();
        let adoption = adopt(&source, &target, false).unwrap();
        assert_eq!(adoption.adopted.unwrap().observations, 3);
    }

    /// A target holding prompts is refused, not thrown away.
    ///
    /// The old guard asked only whether `observations` was empty, then deleted
    /// the file with its WAL and SHM. A store whose memories had all been
    /// deleted, or one that only ever held prompts, looked empty and was
    /// destroyed.
    #[test]
    fn a_target_holding_prompts_is_never_deleted() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("engram.db");
        let target = temp.path().join("leteo.db");
        engram_database(&source, 2);

        let prompt_count = {
            let mut store =
                crate::store::Store::open(crate::store::StoreConfig::new(target.clone())).unwrap();
            store.create_session("s1", "proj", "C:/repo").unwrap();
            store
                .add_prompt(crate::AddPrompt {
                    session_id: "s1".to_owned(),
                    content: "una pregunta".to_owned(),
                    project: Some("proj".to_owned()),
                })
                .unwrap();
            store
                .connection()
                .query_row("SELECT COUNT(*) FROM prompts", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap()
        };

        let error = adopt(&source, &target, false).unwrap_err().to_string();
        assert!(
            error.contains("already holds") && error.contains("prompts"),
            "the refusal has to say what it found: {error}"
        );

        let after: i64 = Connection::open(&target)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM prompts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(after, prompt_count, "the prompts must still be there");
    }

    /// Adopted types fold the way every other door folds them, and the clock a
    /// type implies is taken from the day the memory was written.
    #[test]
    fn adopted_types_fold_like_every_other_door() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("engram.db");
        let target = temp.path().join("leteo.db");
        engram_database_for(&source, 0, "proj");
        Connection::open(&source)
            .unwrap()
            .execute_batch(
                "DELETE FROM observations;
                 INSERT INTO observations (sync_id, session_id, type, title, content, project, created_at)
                     VALUES ('obs-bug', 's1', 'bug', 'A bug', 'body', 'proj', '2026-01-01 00:00:00'),
                            ('obs-manual', 's1', 'manual', 'A manual', 'body', 'proj', '2026-01-01 00:00:00'),
                            ('obs-decision', 's1', 'decision', 'A decision', 'body', 'proj', '2026-01-01 00:00:00');",
            )
            .unwrap();

        adopt(&source, &target, false).unwrap();

        let connection = Connection::open(&target).unwrap();
        let kind = |sync_id: &str| -> String {
            connection
                .query_row(
                    "SELECT type FROM observations WHERE sync_id = ?1",
                    [sync_id],
                    |row| row.get(0),
                )
                .unwrap()
        };
        let review = |sync_id: &str| -> Option<String> {
            connection
                .query_row(
                    "SELECT review_after FROM observations WHERE sync_id = ?1",
                    [sync_id],
                    |row| row.get(0),
                )
                .unwrap()
        };
        assert_eq!(kind("obs-bug"), "bugfix");
        assert_eq!(kind("obs-manual"), "discovery");
        assert_eq!(review("obs-bug"), None, "a bug does not go stale");
        assert_eq!(review("obs-manual"), None, "neither does a discovery");

        // The one kind with a window is dated from when it was written, not
        // from when the store heard about it.
        let from = crate::timestamp::parse("2026-01-01 00:00:00").unwrap();
        let expected =
            crate::memory::rules::review_after("decision", from).map(crate::timestamp::format);
        assert!(expected.is_some(), "a decision has a window");
        assert_eq!(review("obs-decision"), expected);
    }

    /// A `<private>` span, a body over the bound and a title spanning lines are
    /// things the local and import doors have not stored since #412. Adoption
    /// copied all three through from a program Leteo does not control.
    #[test]
    fn adopted_content_passes_the_same_normalisers_as_every_other_door() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("engram.db");
        let target = temp.path().join("leteo.db");
        let bound = crate::store::StoreConfig::new(target.clone()).max_observation_length;
        engram_database_for(&source, 0, "proj");
        {
            let source = Connection::open(&source).unwrap();
            source
                .execute_batch(
                    "DELETE FROM observations;
                     DELETE FROM user_prompts;
                     ALTER TABLE memory_relations ADD COLUMN reason TEXT;
                     ALTER TABLE memory_relations ADD COLUMN evidence TEXT;
                     ALTER TABLE memory_relations ADD COLUMN confidence REAL;
                     UPDATE sessions
                        SET summary = 'closing <private>token</private> note',
                            directory = 'relative/place';",
                )
                .unwrap();
            source
                .execute(
                    "INSERT INTO observations (sync_id, session_id, type, title, content, project)
                     VALUES ('obs-private', 's1', 'decision', '  A
folded   title  ', 'kept <private>secret</private> visible', 'proj')",
                    [],
                )
                .unwrap();
            let long = format!("{}tail", "x".repeat(bound + 500));
            source
                .execute(
                    "INSERT INTO observations (sync_id, session_id, type, title, content, project)
                     VALUES ('obs-long', 's1', 'decision', 'A long body', ?1, 'proj')",
                    [&long],
                )
                .unwrap();
            source
                .execute(
                    "INSERT INTO user_prompts (session_id, content, project)
                     VALUES ('s1', 'why <private>token</private> here?', 'proj')",
                    [],
                )
                .unwrap();
            source
                .execute(
                    "INSERT INTO memory_relations
                        (sync_id, source_id, target_id, relation, judgment_status, reason,
                         evidence, confidence)
                     VALUES ('rel-private', 'obs-private', 'obs-long', 'related', 'judged',
                             'because <private>secret</private>', 'saw <private>secret</private>', 42.0)",
                    [],
                )
                .unwrap();
        }

        adopt(&source, &target, false).unwrap();

        let connection = Connection::open(&target).unwrap();
        let (title, content, hash): (String, String, String) = connection
            .query_row(
                "SELECT title, content, normalized_hash FROM observations WHERE sync_id = 'obs-private'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(title, "A folded title", "one line, whitespace folded");
        assert_eq!(content, "kept [REDACTED] visible");
        assert_eq!(
            hash,
            crate::memory::normalize::normalized_hash(&content),
            "dedupe hashes the body the store actually holds"
        );

        let long: String = connection
            .query_row(
                "SELECT content FROM observations WHERE sync_id = 'obs-long'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(long.len(), bound, "cut to the published bound");
        assert!(long.ends_with("... [truncated]"), "{long}");

        let (summary, directory): (String, String) = connection
            .query_row(
                "SELECT summary, directory FROM sessions WHERE id = 's1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(summary, "closing [REDACTED] note");
        assert_eq!(directory, "", "a relative directory names no fixed place");

        let prompt: String = connection
            .query_row("SELECT content FROM prompts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(prompt, "why [REDACTED] here?");

        let (reason, evidence, confidence): (String, String, Option<f64>) = connection
            .query_row(
                "SELECT reason, evidence, confidence FROM memory_relations WHERE sync_id = 'rel-private'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(reason, "because [REDACTED]");
        assert_eq!(evidence, "saw [REDACTED]");
        assert_eq!(
            confidence, None,
            "a confidence outside zero and one is no confidence"
        );
    }

    /// A copy that loses a row to the target's own constraints is not committed.
    ///
    /// `INSERT OR IGNORE` skips a duplicate silently, which is what makes a
    /// repeat idempotent — and the reason the count has to be checked. Here two
    /// source relations share a `sync_id`, which the target's unique index
    /// refuses; the mismatch aborts the adoption and the observations go with
    /// it.
    #[test]
    fn a_mismatch_rolls_back() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("engram.db");
        let target = temp.path().join("leteo.db");
        engram_database(&source, 3);
        Connection::open(&source)
            .unwrap()
            .execute_batch(
                "ALTER TABLE memory_relations RENAME TO memory_relations_old;
                 CREATE TABLE memory_relations (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     sync_id TEXT,
                     source_id TEXT,
                     target_id TEXT,
                     relation TEXT NOT NULL,
                     judgment_status TEXT NOT NULL,
                     created_at TEXT NOT NULL DEFAULT (datetime('now')),
                     updated_at TEXT NOT NULL DEFAULT (datetime('now')));
                 INSERT INTO memory_relations (sync_id, source_id, target_id, relation, judgment_status)
                     VALUES ('rel-1', 'obs-1', 'obs-2', 'related', 'judged'),
                            ('rel-1', 'obs-1', 'obs-3', 'related', 'judged');
                 DROP TABLE memory_relations_old;",
            )
            .unwrap();

        let error = adopt(&source, &target, false).unwrap_err().to_string();
        assert!(
            error.contains("does not match the source"),
            "the mismatch has to be named: {error}"
        );
        assert_eq!(read_counts(&target).unwrap().observations, 0);
    }
}
