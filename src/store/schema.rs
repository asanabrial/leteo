//! Bringing a database of any provenance up to the shape this build expects.

use super::*;

/// Schema version 1, in two halves because order matters.
///
/// Embedded at build time, so the binary needs no files beside it. The tables
/// come first; the triggers reference columns a legacy database only gains
/// while being adopted, so they come last.
pub(super) const BASELINE_TABLES_SQL: &str =
    include_str!("../../migrations/0001_baseline_tables.sql");

pub(super) const BASELINE_FINALIZE_SQL: &str =
    include_str!("../../migrations/0001_baseline_finalize.sql");

/// The data half of the baseline, applied to whatever arrives unstamped.
///
/// Types folded onto the documented set, project names lowercased and their
/// repeated separators collapsed, and the full-text index rebuilt once at the
/// end because an external-content FTS5 table does not notice a plain `UPDATE`.
/// Everything after the tables, folded back into the baseline.
///
/// Eleven numbered migrations lived here until nothing had been released and
/// the numbering started again at 1. There is one migration now and it is this
/// schema. See the head of the file for why that is the rule applying rather
/// than an exception to it.
pub(super) const BASELINE_AFTER_TABLES_SQL: &str =
    include_str!("../../migrations/0001_baseline_after_the_tables.sql");

pub(super) const BASELINE_NORMALIZE_SQL: &str =
    include_str!("../../migrations/0001_baseline_normalize.sql");

/// Every schema change after the baseline, in the order they must be applied.
///
/// One entry, numbered 18, and it was empty until that one. Five numbered
/// migrations were folded into the baseline before anything shipped: nothing in
/// the wild had run them, so there was no history to preserve — only a history
/// to invent. A database converges on the baseline by inspection rather than by
/// replaying versions it never had, which is the only thing that could ever
/// work for an Engram database anyway.
///
/// Append only from here. A *released* migration is never edited, because the
/// databases that already ran it will not run it again — editing one changes
/// what new databases get and silently splits the two apart.
///
/// ```text
/// migrations/
///   0001_baseline_tables.sql     <- version 1, the shape everything converges to
///   0001_baseline_normalize.sql  <- and the data rules that go with it
///   0001_baseline_after_the_tables.sql  <- everything else, folded back in
///   0018_review_clocks_in_calendar_months.sql  <- the first one after it
///   0019_something.sql                  <- add here, and bump SCHEMA_VERSION
/// ```
///
/// The next number is 19 rather than 2 because 2 through 17 are spent history
/// and are refused rather than migrated; `LAST_PRE_RELEASE_VERSION` owns that
/// band and says why.
///
/// Eleven of them accumulated before the first release: ten collapsed into one
/// numbered 16, then one more numbered 17. They are the baseline now and the
/// numbering starts again at 1, because a version that never shipped has no
/// population to split — the only thing renumbering could strand is a database
/// somebody carried through development, and there is exactly one, re-stamped
/// by hand rather than by code that would outlive its reason.
///
/// What that costs is stated so nobody rediscovers it: a store stamped above
/// `SCHEMA_VERSION` is refused at `open`, by design, because from here on that
/// means a newer build wrote it — and one stamped inside the pre-release band
/// is refused too, for the different reason `LAST_PRE_RELEASE_VERSION` records.
pub(super) const MIGRATIONS: &[(i32, Migration)] = &[(
    18,
    Migration::Rust(
        REVIEW_CLOCKS_IN_CALENDAR_MONTHS,
        review_clocks_in_calendar_months,
    ),
)];

pub(super) const REVIEW_CLOCKS_IN_CALENDAR_MONTHS: &str =
    include_str!("../../migrations/0018_review_clocks_in_calendar_months.sql");

/// How a migration is carried out.
///
/// SQL for nearly all of them, and Rust for the one that has to apply a rule
/// the code already implements. `0006` derives a title from a summary's body
/// and `normalize::headline` decides what that title is — expressed twice, the
/// two disagreed at once: the SQL took the literal second line, so 21 summaries
/// whose second line is blank kept the name the migration existed to replace,
/// while the Rust skipped the blank and found the answer. One rule, one
/// implementation, and the file keeps the prose explaining why.
///
/// Constructed again as of migration 18, which is the `Rust` variant's second
/// use and the reason the variant was kept while nothing used it. The parts
/// that are not obvious, and that a rebuild would have had to rediscover: one
/// transaction around the whole run, the stamp *after* the step rather than
/// before, and a `Rust` arm for a rule the code already owns.
pub(super) enum Migration {
    // The half that is not carried yet. Migration 18 happens to be a `Rust`
    // one, so this arm is unconstructed for the same reason the whole enum
    // was until it existed — and it is `expect` rather than `allow` for the
    // same reason too: the first migration that is plain SQL makes this
    // constructed again and fails the build here, which is the reminder to
    // delete the attribute. An `allow` would sit there permitting a lint that
    // no longer fires.
    #[expect(dead_code, reason = "the arm the first SQL migration uses")]
    Sql(&'static str),
    /// The documentation, and the step that does the work.
    Rust(&'static str, fn(&Connection) -> Result<(), rusqlite::Error>),
}

/// The schema version this build understands.
///
/// Databases are stamped with it in `PRAGMA user_version`. A database carrying
/// a higher number was written by a newer Leteo and is refused, because an
/// older binary cannot know what changed and would write rows the new shape
/// does not expect.
/// Numbering skips 2 through 17 because they are spent: canonical types,
/// stemmed index, lowercase projects, normalised projects, summary headlines,
/// and the eleven that followed them. (1 is the baseline, and is the one number
/// in that stretch that is not skipped.) All six were folded into the baseline, and a database
/// that ran *all* of them holds the same schema as one carrying `1` — which is
/// the claim that made re-stamping look safe, and which is true of a stamp of
/// 17 and not of a stamp of 8. That is why those numbers are refused rather
/// than migrated; see `LAST_PRE_RELEASE_VERSION`. It is also why the first real
/// migration is numbered above all of them instead
/// of re-stamping them down and colliding with the numbers a released build
/// hands out.
///
/// Migration 18 is that first one, so this is now 18. The paragraph above was
/// written before it existed and turned out to be an instruction rather than a
/// description — and it undercounts what has to be cleared. Six numbers are
/// history in the sense it means, but *seventeen* have been stamped on a real
/// file: eleven more migrations, `0007` through `0017`, accumulated before
/// anything shipped and were folded into the same baseline. Numbering this 7
/// would have collided with one of them, and a development store already
/// stamped 7 would have read as current and skipped the repair in silence. 18
/// is above every number any file has carried.
///
/// What raising it past them does *not* do is make them migratable: see
/// `LAST_PRE_RELEASE_VERSION` and the refusal in `migrate`. They stop being
/// ambiguous and stay refused.
pub(crate) const SCHEMA_VERSION: i32 = 18;

/// The highest number stamped by the numbering that predates any release.
///
/// Written out rather than derived as `SCHEMA_VERSION - 1`, which is the same
/// number today and would be wrong the moment a nineteenth migration exists:
/// the band that has to be refused is fixed history, and `SCHEMA_VERSION` is
/// not. Nothing below 2 belongs to it — 0 is unstamped and 1 is the baseline.
pub(super) const LAST_PRE_RELEASE_VERSION: i32 = 17;

/// Applies the pragmas and the schema, waiting out another process that is
/// doing the same thing.
///
/// Converting a rollback journal with `PRAGMA journal_mode = WAL` needs an
/// exclusive lock, and SQLite does not run the busy handler for that
/// conversion — so the `busy_timeout` set just above does not cover it. Several
/// agents starting at once against a fresh install or an imported database
/// would leave one winner and the rest failing to open with "database is
/// locked", which reads as data loss from the outside.
///
/// Every step is idempotent, so waiting and repeating is safe.
pub(super) fn prepare(connection: &Connection, wait: Duration) -> Result<(), StoreError> {
    // The same budget the connection's `busy_timeout` gets, not a second five
    // seconds of its own. Two independent clocks meant a hook told to give up
    // after two seconds took four and a half: this loop spent its own deadline
    // first, and then the write waited again. A caller that says how long it
    // can wait is saying it about the whole open, not about one of the two
    // places that wait.
    let deadline = std::time::Instant::now() + wait;
    let mut backoff = Duration::from_millis(10);
    loop {
        match prepare_once(connection) {
            Ok(()) => return Ok(()),
            Err(error) if is_busy(&error) && std::time::Instant::now() < deadline => {
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(Duration::from_millis(250));
            }
            Err(error) => return Err(error),
        }
    }
}

fn prepare_once(connection: &Connection) -> Result<(), StoreError> {
    // The pragmas stay outside: `journal_mode` cannot run inside a transaction,
    // and `foreign_keys` is ignored within one.
    //
    // `journal_mode = WAL` is the single most expensive thing a hook does
    // before it does any work: 1.1 ms of an 11 ms process, against tens of
    // microseconds for the two below it. The database is already in WAL — that
    // lives in the file header and survives closing — so setting it again on
    // every open looks like pure waste, and it is not.
    //
    // Measured in fresh processes, which is the only way to measure a
    // first-time cost, alternating the three so the file cache could not hand
    // one of them the result:
    //
    // ```text
    //                                      open    pragma   first query   total
    //   PRAGMA journal_mode = WAL         0.215     0.869        0.036    1.120
    //   read it, set it only if not WAL   0.213     0.854        0.032    1.099
    //   do not touch it                   0.233     0.001        0.923    1.157
    // ```
    //
    // The cost is attaching to the WAL index — opening `-shm`, reading the
    // header — and the first statement that touches the database pays it
    // whether or not this line is here. Removing it moves 0.9 ms to whatever
    // runs next; reading it first to skip the write costs the same as writing
    // it. So it stays, and this note is here so the next person to spot the
    // apparently-redundant pragma does not spend the afternoon on it again.
    connection.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;",
    )?;
    // A database already at this version is not migrated, and asking for the
    // write lock to find that out is what every hook was doing.
    //
    // When there *is* something to migrate, the whole schema goes in one
    // transaction: migration is a long sequence of "is this here yet?" checks
    // followed by the change each one guards, and `execute_batch` would put
    // every statement in its own implicit transaction — so two processes could
    // both read "the column is missing" and both add it, or both drop a trigger
    // and both recreate it. Holding the write lock for the whole run makes
    // every one of those pairs atomic, and it is the only way the checks mean
    // anything.
    //
    // `BEGIN IMMEDIATE` takes the lock up front — deliberately, because the
    // migration writes and a deferred upgrade fails instantly under a second
    // writer. But the overwhelmingly common open has nothing to migrate: the
    // transaction begins, reads one pragma, and commits.
    //
    // Reading the version needs no lock at all, and a database already at it
    // has nothing for `migrate` to do — the loop there only applies what is
    // numbered above the stamp. So the lock is now taken by the opens that are
    // going to write, which on any machine is the first one after an upgrade
    // and no others.
    //
    // Worth 0.5 ms: a hook with no work went from 10.78 ms to 10.30 against a
    // real store, over 41 runs. The reason to expect more — several agents
    // queueing on one write lock — was measured and could not be separated:
    // sixteen concurrent hooks are sixteen process creations on sixteen cores,
    // and that swamps everything, before and after alike. So the honest claim
    // is the half millisecond and the removal of a serialisation point that
    // exists by construction whether or not a stopwatch can see it.
    //
    // And that the database is ours. Engram stamps `user_version = 1` as well,
    // so with the numbering restarted at 1 this fast path matched another
    // program's file exactly and opened it as Leteo's own: `migrate` tells the
    // two apart by shape and never got the chance.
    //
    // Migration 18 moved `SCHEMA_VERSION` to 18 and that particular collision
    // is gone, since Engram's file is stamped 1 and no longer reaches here. The
    // shape test stays, and the reason is now the general one rather than the
    // Engram one: this path skips every check in `migrate`, so whatever it
    // accepts is opened unexamined, and a stamp is a number anybody can write.
    // It also stopped being covered when the collision went — the fixture that
    // killed the mutation for it was Engram's own stamp of 1 — which is why
    // `the_fast_path_refuses_a_file_stamped_current_that_is_not_leteos` now
    // stamps a fixture at whatever this build understands.
    //
    // One lookup in `sqlite_master`, which is what the shape test costs.
    if schema_version(connection)? == SCHEMA_VERSION && table_exists(connection, "prompts")? {
        refresh_statistics(connection);
        return Ok(());
    }
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    migrate(&transaction)?;
    transaction.commit()?;
    refresh_statistics(connection);
    Ok(())
}

/// Gives the planner the numbers it plans with.
///
/// Nothing in Leteo had ever run `ANALYZE`, so no store has ever held a
/// `sqlite_stat1` and every query was planned from SQLite's built-in guesses.
/// One of those guesses is expensive: the paged listing — `WHERE deleted_at IS
/// NULL ORDER BY datetime(created_at) DESC, id DESC` — takes the index on
/// `deleted_at`, which narrows nothing because almost nothing is deleted, and
/// then sorts what is left in a temporary B-tree. Measured on a real store of
/// 3,719 memories: **8.8 ms, with an index that answers the whole thing sitting
/// unused beside it**. That index was added the same day for exactly this
/// query, and it changed nothing, because a planner with no statistics never
/// chose it.
///
/// With statistics the same query is 0.01 ms and full-text search is twice as
/// fast. Every other hot query — recent sessions, distinct prompts, recent
/// memories, conflict candidates — measured the same before and after, so this
/// is not a trade.
///
/// `0x10002` rather than the bare pragma: the plain form only considers tables
/// this connection has already queried, and at open that is none of them.
/// `0x10000` drops that condition. It costs about 9 ms on a store that has
/// never been analysed and 0.0 ms on one that has, so it is paid once and then
/// only again when the store has grown enough for SQLite to think the numbers
/// have gone stale.
///
/// A failure is not an error. A store on read-only media, or one another writer
/// holds right now, plans the way it always did — which is how every store has
/// planned until now.
fn refresh_statistics(connection: &Connection) {
    if let Err(error) = connection.execute_batch("PRAGMA optimize = 0x10002") {
        tracing::debug!(%error, "could not refresh query planner statistics");
    }
}

/// Reports which schema version a database is stamped with.
pub(super) fn schema_version(connection: &Connection) -> Result<i32, rusqlite::Error> {
    connection.query_row("PRAGMA user_version", [], |row| row.get(0))
}

/// Whether the database holds a table by this name.
///
/// One question in one place: migration asks it to tell an Engram database from
/// a Leteo one, and the index rebuild asks it to skip an index a migration has
/// not created yet.
fn table_exists(connection: &Connection, name: &str) -> Result<bool, rusqlite::Error> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [name],
        |row| row.get(0),
    )
}

/// Brings a database to [`SCHEMA_VERSION`], or explains why it cannot.
///
/// Four cases, and the second is the one no migration library models:
///
/// - **Stamped above what this build knows.** Refused. A newer Leteo wrote it,
///   and this binary would corrupt it by writing the older shape.
/// - **Unstamped.** Adopted: converged to the baseline by inspection, then
///   stamped 1. Covers a brand new file, a Leteo database from before
///   versioning, and an Engram one.
/// - **Stamped inside the pre-release band, 2..=17.** Refused, and for a
///   different reason from the first: what those numbers did lives in the
///   folded baseline, which runs for an unstamped database and no other. See
///   [`LAST_PRE_RELEASE_VERSION`].
/// - **Stamped 1 or above.** Every migration numbered above it is applied in
///   order.
pub(super) fn migrate(connection: &Connection) -> Result<(), StoreError> {
    let mut version = schema_version(connection)?;
    // Stores from before the first release carry the old numbering, and what
    // that buys them is that they are no longer *ambiguous*: every one of those
    // numbers is under `SCHEMA_VERSION` since migration 18, so a `6` can no
    // longer read as "written by something newer", and they used to be
    // re-stamped down to `1` only to avoid exactly that.
    //
    // It does not make them migratable, and this comment said it did until the
    // refusal below was written. The verification it rested on — a fully
    // migrated store compared against a fresh one, same 22 tables, same
    // columns, same indexes — was carried out on a store that had run the whole
    // pre-release chain, which is what a stamp of 17 means and not what a stamp
    // of 6 means. The folded file is reachable from the unstamped branch alone.
    if version > SCHEMA_VERSION {
        return Err(StoreError::SchemaTooNew {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }

    // Raising `SCHEMA_VERSION` to 18 stopped these numbers being ambiguous —
    // below 18 a stamp can only be old — and that is not the same as making
    // them safe. What 0007 through 0017 did lives in `BASELINE_AFTER_TABLES_SQL`
    // and runs only for a database stamped 0, so falling through here would
    // bring such a store forward without the migrations ABOVE its own number —
    // a store stamped 8 ran 0002 through 0008 and holds what those did; what it
    // has never seen is 0009 through 0017 — then stamp it current and hand it
    // to a fast path that never looks again.
    //
    // How bad that is depends on where in the band it stopped, and the worst
    // case is the low end: `observations_exact` and its triggers are created by
    // the block marked `was 0008_exact_index.sql`, so a store stamped 2 lacks
    // the table while `doctor --repair` would install `obs_exact_*` triggers
    // over it regardless. A store stamped 8 has that table and is missing the
    // nine blocks above it instead.
    //
    // The comment above said these would be brought forward like any other old
    // database, and it is corrected there rather than contradicted here: it was
    // written against a numbering in which every one of them had run, which is
    // what a stamp of 17 means and not what a stamp of 8 means.
    if (2..=LAST_PRE_RELEASE_VERSION).contains(&version) {
        return Err(StoreError::SchemaFromPreRelease {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }

    // Engram's database, told apart by its shape rather than by its stamp.
    //
    // The unstamped case is handled below by `adopt_to_baseline`, which this
    // function's own documentation says covers an Engram database — and it
    // does, for one that carries no `user_version`. A real one carries `1`,
    // which is also what Leteo stamps a database it has converged, so the two
    // are indistinguishable by version. Skipping adoption, the loop then ran
    // migrations written for Leteo's baseline against Engram's tables and the
    // caller got `no such table: prompts`.
    //
    // Refused rather than converged, and that is the whole point of saying it
    // here: `leteo import --from-engram` snapshots the source and writes into a
    // Leteo store, leaving Engram's file as it was. Converting it in place
    // because somebody passed `--database` by mistake would rewrite another
    // program's database on their behalf.
    if table_exists(connection, "user_prompts")? && !table_exists(connection, "prompts")? {
        return Err(StoreError::EngramDatabase);
    }

    if version == 0 {
        adopt_to_baseline(connection)?;
        // The data rules, after the structure they depend on and before the
        // stamp, so a database interrupted here is retried rather than
        // recorded as finished.
        connection.execute_batch(BASELINE_NORMALIZE_SQL)?;
        summary_headlines(connection)?;
        // In the position the numbered migrations used to run from: after the
        // tables exist and the data rules have folded what arrived, before the
        // stamp — so a database interrupted here is retried rather than
        // recorded as finished.
        connection.execute_batch(BASELINE_AFTER_TABLES_SQL)?;
        connection.execute_batch("PRAGMA user_version = 1")?;
        version = 1;
    }

    for (number, migration) in MIGRATIONS {
        if *number > version {
            match migration {
                Migration::Sql(sql) => connection.execute_batch(sql)?,
                Migration::Rust(documentation, step) => {
                    step(connection).inspect_err(|error| {
                        // A migration that fails otherwise surfaces as a bare
                        // SQLite line with nothing saying which step raised it
                        // or what it was for. The first line of its file says
                        // both.
                        tracing::error!(
                            migration = number,
                            %error,
                            "{}",
                            documentation
                                .lines()
                                .next()
                                .unwrap_or("migration failed")
                                .trim_start_matches("-- ")
                        );
                    })?;
                }
            }
            connection.execute_batch(&format!("PRAGMA user_version = {number}"))?;
            version = *number;
        }
    }

    // A build that ships migrations must end at the version it claims.
    if version != SCHEMA_VERSION {
        connection.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
    }
    Ok(())
}

/// Migration `0006`: gives each session summary a title of its own.
///
/// The rule lives in [`crate::memory::normalize::headline`], which is also what
/// titles a summary as it is written, so a memory repaired here and a memory
/// saved tomorrow get the same name from the same code.
///
/// Written in Rust because the rule is not expressible in SQL without becoming
/// a second, weaker version of itself: the headline is the first line that is
/// neither blank nor a heading, and a `substr` of the second line is only the
/// same thing when a summary happens to have that shape. It usually does — and
/// 21 of 898 did not, opening with a title line, a blank, and then `## Goal`.
/// Those are precisely the ones a literal reading leaves behind.
fn summary_headlines(connection: &Connection) -> Result<(), rusqlite::Error> {
    let rows = {
        let mut statement = connection.prepare(
            "SELECT id, ifnull(project, ''), content FROM observations
             WHERE type = 'session_summary' AND deleted_at IS NULL
               -- Already carries one: this ran before, or the row was written
               -- by a build that titles them properly.
               AND title NOT LIKE '%—%'",
        )?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };

    let mut update = connection.prepare("UPDATE observations SET title = ?1 WHERE id = ?2")?;
    let mut changed = 0;
    for (id, project, content) in rows {
        // Nothing worth lifting — a bare date, a stray bullet. The plain name
        // it already has beats an invented one.
        let Some(headline) = crate::memory::normalize::headline(&content, SUMMARY_HEADLINE_CHARS)
        else {
            continue;
        };
        update.execute(rusqlite::params![
            format!("Session summary: {project} — {headline}"),
            id
        ])?;
        changed += 1;
    }

    // `title` is a column of both full-text indexes, and the rebuild is belt
    // and braces rather than the only thing keeping them current: `obs_fts_update`
    // is a bare `AFTER UPDATE ON observations` with no `UPDATE OF` list, so it
    // fires on these writes too. An earlier version of this comment said the
    // updates went around the triggers, which was true of 0004 and 0005 — they
    // ran before the triggers existed — and stopped being true when this step
    // moved after `adopt_to_baseline`. Skipped when nothing moved, because a
    // rebuild on every open of an already-migrated store is not free.
    if changed > 0 {
        connection
            .execute_batch("INSERT INTO observations_fts(observations_fts) VALUES('rebuild');")?;
    }
    Ok(())
}

/// Migration 18: the review clocks the baseline counted SQLite's way.
///
/// `0001_baseline_after_the_tables.sql` fills an empty clock with
/// `datetime(created_at, '+6 months')`, and `rules::review_after` computes the
/// same window with `chrono::checked_add_months`. They disagree when the day of
/// the month does not exist in the target month — chrono clamps to the last
/// day, SQLite forms 2027-02-31 and rolls it forward to 2027-03-03. Measured
/// across 2026-2029: 27 such days for the six-month window, 19 for three, 1 for
/// twelve.
///
/// A row is repaired only when what it holds is exactly what the baseline's SQL
/// would have produced from its own `created_at`, and is not what the rule
/// says. That condition is the whole safety of this step rather than caution
/// about it: `mark_reviewed` writes `review_after` from `Utc::now()`, so a
/// memory somebody has confirmed deliberately carries a clock that is *not*
/// `created_at` plus the window, and recomputing every row would silently undo
/// every review the store has recorded.
fn review_clocks_in_calendar_months(connection: &Connection) -> Result<(), rusqlite::Error> {
    let rows = {
        let mut statement = connection.prepare(
            "SELECT CAST(id AS INTEGER), CAST(type AS TEXT),
                    CAST(created_at AS TEXT), CAST(review_after AS TEXT)
               FROM observations
              WHERE review_after IS NOT NULL",
        )?;
        // `CAST`, `Option` and raw bytes together, and it takes all three to
        // stop one row making a store unopenable. An adopted table keeps its own
        // column definitions when `migrate_legacy_observations_table` finds `id`
        // already a primary key and skips the rebuild, and `CREATE TABLE IF NOT
        // EXISTS` will not reshape it — so on a database this migration will
        // meet, these columns may be nullable and may hold any type at all.
        //
        // `Option` answers NULL. `CAST` answers INTEGER and REAL. Neither
        // answers a blob that is not valid UTF-8: `CAST` reinterprets bytes
        // without validating them, and reading the result as `String` then fails
        // in `ValueRef::as_str`. Any of those failures aborts the step, rolls the
        // transaction back, and repeats on every subsequent open, because the
        // stamp is written after the step — so the bytes are taken raw and
        // decoded lossily, and a row that decodes to something the predicate
        // rejects keeps a clock three days late. That is the failure worth
        // having.
        statement
            .query_map([], |row| {
                // `get_ref` rather than `get`, because what has to be survived
                // is a TEXT value whose bytes are not valid UTF-8, and
                // `get::<String>` rejects exactly that in `ValueRef::as_str`.
                // The `CAST` above turns every other storage class into TEXT —
                // a blob is reinterpreted rather than validated, which is where
                // such bytes come from — so the `Blob` arm is belt and braces
                // for a value reaching here uncast. `Vec<u8>` was tried first
                // and refuses text, which an adoption test caught.
                let text = |column: usize| -> rusqlite::Result<Option<String>> {
                    use rusqlite::types::ValueRef;
                    Ok(match row.get_ref(column)? {
                        ValueRef::Text(raw) | ValueRef::Blob(raw) => {
                            Some(String::from_utf8_lossy(raw).into_owned())
                        }
                        _ => None,
                    })
                };
                Ok((row.get::<_, i64>(0)?, text(1)?, text(2)?, text(3)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };

    // The windows the baseline actually wrote, frozen here as the literals it
    // used, and used for BOTH halves of this step — recognising the clock it
    // wrote and computing the one that replaces it.
    //
    // Not `rules::review_months` and not `rules::review_after`, and this is the
    // one place in the crate where reading them would be wrong. Everywhere else
    // those are the single copy of the rule that AGENTS.md rule 3 asks for. Here
    // the migration is frozen history: it is released, it is append-only, and it
    // must give every database the same answer whenever it happens to run. Read
    // the live table and it stops matching what the baseline wrote on the day
    // the windows change — repairing nothing and stamping the rest of the world
    // 18 with the rolled-forward clocks still in it — and, worse, a store that
    // ran it before that day and one that ran it after would hold different
    // clocks for the same memory. That is the population split the append-only
    // rule exists to prevent, produced by the migration meant to close one.
    const AS_THE_BASELINE_COUNTED: &[(&str, u32)] =
        &[("decision", 6), ("policy", 12), ("preference", 3)];

    // The baseline's own arithmetic, asked of the same engine that wrote the
    // value. Reimplementing SQLite's month rollover in Rust to recognise its
    // output would be a third opinion about months, which is the defect.
    let mut rolled_forward = connection.prepare("SELECT datetime(?1, '+' || ?2 || ' months')")?;
    let mut update =
        connection.prepare("UPDATE observations SET review_after = ?1 WHERE id = ?2")?;

    let mut repaired = 0_usize;
    let mut skipped = 0_usize;
    for (id, kind, created_at, stored) in rows {
        let (Some(kind), Some(created_at), Some(stored)) = (kind, created_at, stored) else {
            skipped += 1;
            continue;
        };
        // Decoding replaced something, so this row is not the bytes the baseline
        // wrote and cannot be compared against them. It would fall out of a
        // predicate below anyway; counted here so the report can tell a row that
        // could not be read from one that needed nothing.
        if [&kind, &created_at, &stored]
            .iter()
            .any(|value| value.contains(char::REPLACEMENT_CHARACTER))
        {
            skipped += 1;
            continue;
        }
        let Some(&(_, months)) = AS_THE_BASELINE_COUNTED
            .iter()
            .find(|(known, _)| *known == kind)
        else {
            continue;
        };
        let Some(from) = crate::timestamp::parse(&created_at) else {
            skipped += 1;
            continue;
        };
        // Clamping, which is what `rules::review_after` does and what the rule
        // means; spelled out here rather than called for the reason the frozen
        // table above gives.
        let Some(by_the_rule) = from
            .checked_add_months(chrono::Months::new(months))
            .map(crate::timestamp::format)
        else {
            skipped += 1;
            continue;
        };
        if stored == by_the_rule {
            continue;
        }
        // `Option`, because SQLite returns NULL for a timestamp its own parser
        // will not read. `timestamp::parse` trims leading whitespace and
        // SQLite's does not, so the two do not accept exactly the same set, and
        // a row in the gap would otherwise be an `InvalidColumnType` that makes
        // the store unopenable.
        let by_the_baseline: Option<String> =
            rolled_forward.query_row(rusqlite::params![created_at, months], |row| row.get(0))?;
        if by_the_baseline.as_deref() != Some(stored.as_str()) {
            continue;
        }
        update.execute(rusqlite::params![by_the_rule, id])?;
        repaired += 1;
    }
    // A row this could not read is not a row that needed nothing, and the
    // difference is invisible from the outside otherwise. Not an error: a
    // timestamp one parser takes and the other does not costs a clock three
    // days, and refusing to open the store over it would cost everything.
    if skipped > 0 {
        tracing::debug!(
            repaired,
            skipped,
            "some review clocks could not be read well enough to check"
        );
    }
    Ok(())
}

/// How much of a session's opening line its title carries.
///
/// Long enough to tell two sessions apart — on a real store of 898 summaries
/// this leaves 828 distinct titles where there had been 16 — and short enough
/// that a list of them stays a list.
pub(crate) const SUMMARY_HEADLINE_CHARS: usize = 72;

fn is_busy(error: &StoreError) -> bool {
    error.is_busy()
}

/// Brings a database of unknown provenance up to schema version 1.
///
/// **This function is frozen. Do not add to it.** New schema changes go in a
/// new file under `migrations/`, applied by number.
///
/// It exists because Leteo opens databases it did not create: an Engram store,
/// an early Engram schema, or a Leteo database written before versioning
/// existed. None of those record a version, so there is no step to resume from
/// and the only way forward is to inspect what is actually there and converge.
/// That is what the `add_column_if_missing` calls and the table rebuilds below
/// do. Once a database is stamped, this never runs against it again.
fn adopt_to_baseline(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch(BASELINE_TABLES_SQL)?;

    for (name, definition) in [
        ("sync_id", "TEXT"),
        ("scope", "TEXT NOT NULL DEFAULT 'project'"),
        ("topic_key", "TEXT"),
        ("normalized_hash", "TEXT"),
        ("revision_count", "INTEGER NOT NULL DEFAULT 1"),
        ("duplicate_count", "INTEGER NOT NULL DEFAULT 1"),
        ("last_seen_at", "TEXT"),
        ("pinned", "BOOLEAN NOT NULL DEFAULT 0"),
        ("updated_at", "TEXT NOT NULL DEFAULT ''"),
        ("deleted_at", "TEXT"),
    ] {
        add_column_if_missing(connection, "observations", name, definition)?;
    }
    migrate_legacy_observations_table(connection)?;

    for (name, definition) in [
        ("review_after", "TEXT"),
        ("prompt_sync_id", "TEXT"),
        ("expires_at", "TEXT"),
        ("embedding", "BLOB"),
        ("embedding_model", "TEXT"),
        ("embedding_created_at", "TEXT"),
    ] {
        add_column_if_missing(connection, "observations", name, definition)?;
    }
    add_column_if_missing(connection, "prompts", "sync_id", "TEXT")?;
    add_column_if_missing(
        connection,
        "sync_mutations",
        "project",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(connection, "sync_state", "reason_code", "TEXT")?;
    add_column_if_missing(connection, "sync_state", "reason_message", "TEXT")?;

    // Relations gained columns over several upstream releases, and adopting a
    // database from before one of them would otherwise succeed and then fail on
    // the first relation query. `CREATE TABLE IF NOT EXISTS` cannot reshape a
    // table that already exists, so each addition has to be checked for.
    for (name, definition) in [
        ("reason", "TEXT"),
        ("evidence", "TEXT"),
        ("confidence", "REAL"),
        ("marked_by_actor", "TEXT"),
        ("marked_by_kind", "TEXT"),
        ("marked_by_model", "TEXT"),
        ("session_id", "TEXT"),
        // Inherited and never used. Leteo tracks a supersession by the
        // `supersedes` verb on the relation itself, so nothing reads or writes
        // these two — a real store has three hundred relations and not one
        // non-null value in either. They are added anyway because a database
        // being adopted has them, and a column list that disagrees with the
        // file it validates is worse than a column nobody fills.
        ("superseded_at", "TEXT"),
        ("superseded_by_relation_id", "INTEGER"),
        // These two are read into a `String`, so a NULL would fail the row.
        // SQLite only allows a constant default on `ALTER TABLE`, which rules
        // out `datetime('now')`; an empty timestamp is honest about a row that
        // predates the column.
        ("created_at", "TEXT NOT NULL DEFAULT ''"),
        ("updated_at", "TEXT NOT NULL DEFAULT ''"),
    ] {
        add_column_if_missing(connection, "memory_relations", name, definition)?;
    }

    migrate_sync_chunks_table(connection)?;
    migrate_fts_topic_key(connection)?;
    connection.execute_batch(BASELINE_FINALIZE_SQL)?;
    rebuild_full_text(connection)?;
    Ok(())
}

/// How many rows a full-text index actually holds.
///
/// FTS5 keeps one row per indexed document in its `_docsize` shadow table, and
/// that is the only cheap way to tell a populated index from an empty one: the
/// virtual table itself reads through to the content table.
///
/// A missing shadow table yields `-1`, which cannot equal a real row count, so
/// the check that uses this fails loudly rather than assuming the best.
pub(super) fn indexed_row_count(connection: &Connection, index: &str) -> i64 {
    connection
        .query_row(
            &format!("SELECT COUNT(*) FROM {index}_docsize"),
            [],
            |row| row.get(0),
        )
        .unwrap_or(-1)
}

/// Reindexes the full-text tables from the rows they cover.
///
/// The triggers only fire on writes made after they exist, so a database
/// arriving with rows already in it — the whole point of adoption — would keep
/// every one of those memories out of search.
///
/// This rebuilds unconditionally rather than checking first, because there is
/// nothing cheap to check: these are external-content tables, so `COUNT(*)` on
/// the index reads through to the base table and agrees with it even when the
/// inverted index is empty. Adoption happens once per database, so the cost is
/// paid once.
fn rebuild_full_text(connection: &Connection) -> Result<(), rusqlite::Error> {
    rebuild_present_indexes(connection)
}

/// Every full-text index this database has, rebuilt from the rows it covers.
///
/// Which ones exist depends on how far the schema has come: adoption runs
/// before any migration, so at that point there is no unstemmed index to
/// rebuild — migration 8 creates it and rebuilds it in the same file. Naming
/// them here and skipping what is absent means one function serves both that
/// moment and a repair on a fully migrated store, rather than two lists that
/// can disagree about what an index is.
pub(super) fn rebuild_present_indexes(connection: &Connection) -> Result<(), rusqlite::Error> {
    for index in FULL_TEXT_INDEXES {
        if table_exists(connection, index)? {
            connection.execute(
                &format!("INSERT INTO {index}({index}) VALUES('rebuild')"),
                [],
            )?;
        }
    }
    Ok(())
}

/// The full-text indexes, in the order a report lists them.
pub(super) const FULL_TEXT_INDEXES: &[&str] =
    &["observations_fts", "observations_exact", "prompts_fts"];

/// The triggers that keep those indexes level with the rows they cover.
///
/// They are the whole mechanism: nothing else writes to a full-text index, so a
/// store that has lost one stops seeing edits to the table it watches — for
/// good, and without a word. `observation_fts_sync` cannot see it either,
/// because it compares how many rows each side holds and a memory that was
/// edited rather than added leaves both counts alone. So a title changed today
/// is still findable only by the words it had yesterday, and the report says
/// the store is healthy.
///
/// Listed here so something can ask. The definitions live in the two migrations
/// that create them — the baseline for the stemmed and prompt indexes, and
/// migration 8 for the unstemmed one — and this is only the roll call.
pub(super) const FULL_TEXT_TRIGGERS: &[&str] = &[
    "obs_fts_insert",
    "obs_fts_delete",
    "obs_fts_update",
    "obs_exact_insert",
    "obs_exact_delete",
    "obs_exact_update",
    "prompt_fts_insert",
    "prompt_fts_delete",
    "prompt_fts_update",
];

/// The two migrations that own the trigger definitions.
///
/// The second was `0008_exact_index.sql` and is now the file the ten
/// pre-release migrations were collapsed into: the same `CREATE TRIGGER`
/// statements, in a longer document. What matters to the reader below is that
/// each trigger is still a `CREATE TRIGGER name … END;` with the terminator on
/// a line of its own, which is unchanged.
///
/// Read rather than copied. A restore has to write the same SQL the migration
/// would have written, and the way to be sure of that is to take it from the
/// migration — a second copy in Rust would be right on the day it was written
/// and drift the first time either index gains a column.
const FULL_TEXT_TRIGGER_SOURCES: &[&str] = &[
    BASELINE_FINALIZE_SQL,
    include_str!("../../migrations/0001_baseline_after_the_tables.sql"),
];

/// The statement that creates one of them, lifted out of its migration.
///
/// `CREATE TRIGGER name … END;` — the terminator is a line of its own in both
/// files, and every body inside them is a single `INSERT` per line, so a `END;`
/// at the start of a line ends the trigger and nothing else.
pub(super) fn full_text_trigger_sql(name: &str) -> Option<&'static str> {
    let opening = format!("CREATE TRIGGER {name} ");
    FULL_TEXT_TRIGGER_SOURCES.iter().find_map(|source| {
        let start = source.find(&opening)?;
        let rest = &source[start..];
        let end = rest.find("\nEND;")? + "\nEND;".len();
        Some(&rest[..end])
    })
}

/// Puts back the triggers this database has lost, and says which.
///
/// The index they feed is stale by exactly the writes that happened while they
/// were gone, so a caller has to rebuild it afterwards — restoring the trigger
/// only stops the drift growing, it does not undo it. `doctor --repair` does
/// both, in that order.
pub(super) fn restore_full_text_triggers(
    connection: &Connection,
) -> Result<Vec<&'static str>, rusqlite::Error> {
    let missing = missing_full_text_triggers(connection);
    for name in &missing {
        let Some(sql) = full_text_trigger_sql(name) else {
            continue;
        };
        connection.execute_batch(sql)?;
    }
    Ok(missing)
}

/// Which of those triggers this database is missing.
pub(super) fn missing_full_text_triggers(connection: &Connection) -> Vec<&'static str> {
    FULL_TEXT_TRIGGERS
        .iter()
        .filter(|name| {
            !connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'trigger' AND name = ?1)",
                    [*name],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap_or(true)
        })
        .copied()
        .collect()
}

pub(super) fn table_info(
    connection: &Connection,
    table: &str,
) -> Result<Vec<TableColumn>, rusqlite::Error> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = statement.query_map([], |row| {
        Ok(TableColumn {
            name: row.get(1)?,
            primary_key: row.get(5)?,
        })
    })?;
    rows.collect()
}

pub(super) fn add_column_if_missing(
    connection: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), rusqlite::Error> {
    if table_info(connection, table)?
        .iter()
        .any(|existing| existing.name == column)
    {
        return Ok(());
    }
    connection.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
        [],
    )?;
    Ok(())
}

fn migrate_sync_chunks_table(connection: &Connection) -> Result<(), rusqlite::Error> {
    let columns = table_info(connection, "sync_chunks")?;
    let target_key = columns.iter().find(|column| column.name == "target_key");
    let chunk_id = columns.iter().find(|column| column.name == "chunk_id");
    if target_key.is_some_and(|column| column.primary_key == 1)
        && chunk_id.is_some_and(|column| column.primary_key == 2)
    {
        return Ok(());
    }

    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS sync_chunks_new (
             target_key TEXT NOT NULL DEFAULT 'local',
             chunk_id TEXT NOT NULL,
             imported_at TEXT NOT NULL DEFAULT (datetime('now')),
             PRIMARY KEY (target_key, chunk_id)
         );",
    )?;
    if target_key.is_some() {
        connection.execute_batch(
            "INSERT OR IGNORE INTO sync_chunks_new (target_key, chunk_id, imported_at)
             SELECT CASE
                        WHEN trim(ifnull(target_key, '')) = '' THEN 'local'
                        ELSE trim(target_key)
                    END,
                    chunk_id,
                    imported_at
             FROM sync_chunks;",
        )?;
    } else {
        connection.execute_batch(
            "INSERT OR IGNORE INTO sync_chunks_new (target_key, chunk_id, imported_at)
             SELECT 'local', chunk_id, imported_at FROM sync_chunks;",
        )?;
    }
    connection.execute_batch(
        "DROP TABLE sync_chunks;
         ALTER TABLE sync_chunks_new RENAME TO sync_chunks;",
    )?;
    Ok(())
}

fn migrate_legacy_observations_table(connection: &Connection) -> Result<(), rusqlite::Error> {
    let columns = table_info(connection, "observations")?;
    let Some(id) = columns.iter().find(|column| column.name == "id") else {
        return Ok(());
    };
    if id.primary_key == 1 {
        return Ok(());
    }

    connection.execute_batch(
        "CREATE TABLE observations_migrated (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             sync_id TEXT,
             session_id TEXT NOT NULL,
             type TEXT NOT NULL,
             title TEXT NOT NULL,
             content TEXT NOT NULL,
             tool_name TEXT,
             project TEXT,
             scope TEXT NOT NULL DEFAULT 'project',
             topic_key TEXT,
             normalized_hash TEXT,
             revision_count INTEGER NOT NULL DEFAULT 1,
             duplicate_count INTEGER NOT NULL DEFAULT 1,
             last_seen_at TEXT,
             pinned BOOLEAN NOT NULL DEFAULT 0,
             created_at TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at TEXT NOT NULL DEFAULT (datetime('now')),
             deleted_at TEXT,
             FOREIGN KEY (session_id) REFERENCES sessions(id)
         );

         INSERT INTO observations_migrated (
             id, sync_id, session_id, type, title, content, tool_name, project,
             scope, topic_key, normalized_hash, revision_count, duplicate_count,
             last_seen_at, pinned, created_at, updated_at, deleted_at
         )
         SELECT CASE
                    WHEN id IS NULL THEN NULL
                    WHEN ROW_NUMBER() OVER (PARTITION BY id ORDER BY rowid) = 1
                        THEN CAST(id AS INTEGER)
                    ELSE NULL
                END,
                'obs-' || lower(hex(randomblob(16))),
                session_id,
                -- `discovery`, not `manual`, for a foreign row that names no
                -- type: `manual` is outside the eight a typed search asks for,
                -- so it would arrive already invisible to one.
                -- `normalize::kind` folds the same word for the same reason.
                COALESCE(NULLIF(type, ''), 'discovery'),
                COALESCE(NULLIF(title, ''), 'Untitled observation'),
                COALESCE(content, ''),
                tool_name,
                project,
                CASE WHEN scope IS NULL OR scope = '' THEN 'project' ELSE scope END,
                NULLIF(topic_key, ''),
                normalized_hash,
                CASE WHEN revision_count IS NULL OR revision_count < 1
                    THEN 1 ELSE revision_count END,
                CASE WHEN duplicate_count IS NULL OR duplicate_count < 1
                    THEN 1 ELSE duplicate_count END,
                last_seen_at,
                0,
                COALESCE(NULLIF(created_at, ''), datetime('now')),
                COALESCE(NULLIF(updated_at, ''), NULLIF(created_at, ''), datetime('now')),
                deleted_at
         FROM observations
         ORDER BY rowid;

         DROP TABLE observations;
         ALTER TABLE observations_migrated RENAME TO observations;",
    )?;
    connection.execute_batch(REBUILD_OBSERVATION_INDEX)?;
    Ok(())
}

/// Rebuilding the memory index from nothing.
///
/// Two migrations need this — one rewrites `observations` underneath it, the
/// other adds a column to it — and each carried its own copy of the same twelve
/// lines. These are the columns every search reads, so a change made in one
/// copy and not the other would leave two databases with different indexes
/// depending on which version each was upgraded from, and nothing would say so.
///
/// The triggers are dropped and left off deliberately.
/// `0001_baseline_finalize.sql` runs immediately after and owns their
/// definitions; recreating them here would have them dropped and recreated a
/// moment later, and would leave two copies of *that* SQL to keep in step.
const REBUILD_OBSERVATION_INDEX: &str = "    DROP TRIGGER IF EXISTS obs_fts_insert;
     DROP TRIGGER IF EXISTS obs_fts_update;
     DROP TRIGGER IF EXISTS obs_fts_delete;
     DROP TABLE IF EXISTS observations_fts;
     CREATE VIRTUAL TABLE observations_fts USING fts5(
         title, content, tool_name, type, project, topic_key,
         content='observations', content_rowid='id'
     );
     INSERT INTO observations_fts(rowid, title, content, tool_name, type, project, topic_key)
     SELECT id, title, content, tool_name, type, project, topic_key
     FROM observations
     WHERE deleted_at IS NULL;";

fn migrate_fts_topic_key(connection: &Connection) -> Result<(), rusqlite::Error> {
    let has_topic_key = connection.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM pragma_table_xinfo('observations_fts') WHERE name = 'topic_key'
         )",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    if has_topic_key {
        return Ok(());
    }

    // The triggers are dropped and left off. `0001_baseline_finalize.sql` runs
    // immediately after this and owns their definitions; recreating them here
    // would only have them dropped and recreated again a moment later, and
    // would leave two copies of the same SQL to be kept in step by hand.
    connection.execute_batch(REBUILD_OBSERVATION_INDEX)
}
