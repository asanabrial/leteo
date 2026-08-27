//! Opening a database of any provenance, and moving it forward.

use super::*;

#[test]
fn two_processes_can_open_one_legacy_database_at_the_same_time() {
    use std::sync::Arc;
    use std::sync::Barrier;

    // Opening runs the legacy table rebuilds, which write. Starting two
    // agents at once against the same old database is ordinary, and a
    // deferred rebuild transaction used to fail the second one outright.
    let (temp, config) = legacy_database(EARLY_ENGRAM_SCHEMA);
    let barrier = Arc::new(Barrier::new(6));
    let openers = (0..6)
        .map(|_| {
            let config = config.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                Store::open(config).map(|_| ())
            })
        })
        .collect::<Vec<_>>();

    for opener in openers {
        opener
            .join()
            .expect("the opening thread did not panic")
            .expect("concurrent opens of a legacy database succeed");
    }
    drop(temp);
}

#[test]
fn the_migrated_schema_is_exactly_what_the_queries_expect() {
    let (_temp, store) = store();
    for (table, expected) in EXPECTED_COLUMNS {
        let mut statement = store
            .connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        let actual = statement
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<BTreeSet<_>, _>>()
            .unwrap();
        let expected = expected
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<BTreeSet<_>>();

        let missing = expected.difference(&actual).collect::<Vec<_>>();
        assert!(
            missing.is_empty(),
            "{table} is missing {missing:?}; a query reading them would fail at runtime"
        );
        let unexpected = actual.difference(&expected).collect::<Vec<_>>();
        assert!(
            unexpected.is_empty(),
            "{table} has {unexpected:?} that no code reads; a renamed column leaves the \
                 old one behind like this"
        );
    }
}

#[test]
fn every_shared_column_list_is_valid_sql_against_the_real_schema() {
    // The lists are strings pasted into queries, so a typo in one is only
    // found when something happens to run it. Preparing each one here means
    // that is at build-and-test time instead of in front of a user.
    let (_temp, store) = store();
    for (label, sql) in [
        (
            "OBSERVATION_COLUMNS",
            format!("SELECT {OBSERVATION_COLUMNS} FROM observations"),
        ),
        (
            "OBSERVATION_COLUMNS_JOINED",
            format!(
                "SELECT {OBSERVATION_COLUMNS_JOINED} FROM observations_fts fts \
                     CROSS JOIN observations o ON o.id = fts.rowid"
            ),
        ),
        (
            "PROMPT_COLUMNS",
            format!("SELECT {PROMPT_COLUMNS} FROM prompts"),
        ),
        (
            "PROMPT_COLUMNS_JOINED",
            format!(
                "SELECT {PROMPT_COLUMNS_JOINED} FROM prompts_fts fts \
                     CROSS JOIN prompts p ON p.id = fts.rowid"
            ),
        ),
        (
            "RELATION_COLUMNS",
            format!("SELECT {RELATION_COLUMNS} FROM memory_relations"),
        ),
        (
            "SYNC_MUTATION_COLUMNS",
            format!("SELECT {SYNC_MUTATION_COLUMNS} FROM sync_mutations"),
        ),
    ] {
        store
            .connection
            .prepare(&sql)
            .unwrap_or_else(|error| panic!("{label} does not match the schema: {error}"));
    }
}

/// A store carries the numbers its own queries are planned from.
///
/// Nothing here had ever run `ANALYZE`, so `sqlite_stat1` did not exist in any
/// store and every plan came from SQLite's built-in guesses. The listing below
/// is what that cost: it was answered by narrowing on `deleted_at` — which
/// excludes almost nothing — and sorting the remainder in a temporary B-tree.
/// On a real store of 3,719 memories that took 8.8 ms with the index that
/// answers it outright sitting unused beside it, and 0.01 ms once the planner
/// had the numbers to choose it.
///
/// Both halves are asserted, because either alone passes while the other is
/// broken: statistics nothing consults, or a plan that happens to be right on
/// a fixture too small to have a wrong one.
#[test]
fn opening_a_store_leaves_the_planner_the_numbers_it_plans_with() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("leteo.db");
    {
        let mut store = Store::open(StoreConfig::new(path.clone())).unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        // Enough of them that the sort is worth avoiding. A handful would be
        // planned either way and would assert nothing — the same trap that
        // kept the bm25 floor alive for two months.
        for index in 0..400 {
            store
                .add_observation(observation(
                    "s1",
                    &format!("Memoria número {index}"),
                    "Un cuerpo cualquiera, largo lo justo para ocupar sitio en la página.",
                ))
                .unwrap();
        }
    }

    // Reopened, because the statistics describe what was there when the store
    // was opened and the first open found an empty file.
    let store = Store::open(StoreConfig::new(path)).unwrap();
    let described: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM sqlite_stat1", [], |row| row.get(0))
        .expect("a store that has been opened holds statistics");
    assert!(
        described > 0,
        "opening a store with memories in it has to leave the planner something to plan with"
    );

    // The production statement, not a copy of it.
    let plan: Vec<String> = store
        .connection
        .prepare(&format!(
            "EXPLAIN QUERY PLAN {}",
            crate::store::observations::unfiltered_page_sql("")
        ))
        .unwrap()
        .query_map(rusqlite::params![20_i64, 0_i64], |row| {
            row.get::<_, String>(3)
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let joined = plan.join(" | ");
    assert!(
        !joined.contains("TEMP B-TREE"),
        "the listing has an index that answers it in order; sorting means the planner did not know: {joined}"
    );
}

/// A database that already exists gets the index the baseline hands out.
///
/// The listing's index ships in `0001_baseline_finalize.sql`, and a store
/// stamped `1` never runs the baseline again — so every database created
/// before it was added listed memories by scanning the table and sorting it,
/// for ever. On a real store of 3,719 memories that is 7.8 ms a page against
/// 0.01 ms, and no amount of statistics fixes it, because there is nothing to
/// choose.
///
/// The fixture is that store: stamped `1`, with the index removed.
#[test]
fn a_database_stamped_before_the_index_existed_is_given_it() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("leteo.db");
    {
        let store = Store::open(StoreConfig::new(path.clone())).unwrap();
        store
            .connection
            .execute_batch(
                "DROP INDEX IF EXISTS idx_obs_created_order;
                 PRAGMA user_version = 0;",
            )
            .unwrap();
    }

    let store = Store::open(StoreConfig::new(path)).unwrap();
    assert_eq!(
        schema_version(&store.connection).unwrap(),
        SCHEMA_VERSION,
        "an old store is brought forward rather than left where it was"
    );
    let indexes: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
              WHERE type = 'index' AND name = 'idx_obs_created_order'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        indexes, 1,
        "the migration exists so that a database older than the index is given it"
    );
}

#[test]
fn opening_stamps_the_schema_version_and_leaves_it_alone_afterwards() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("leteo.db");
    let store = Store::open(StoreConfig::new(path.clone())).unwrap();
    assert_eq!(
        schema_version(&store.connection).unwrap(),
        SCHEMA_VERSION,
        "a fresh database is stamped with the version that created it"
    );
    drop(store);

    // Reopening is a no-op, not another adoption.
    let store = Store::open(StoreConfig::new(path)).unwrap();
    assert_eq!(schema_version(&store.connection).unwrap(), SCHEMA_VERSION);
}

#[test]
fn an_unstamped_database_is_adopted_rather_than_rejected() {
    // This is every database that predates versioning: an old Leteo store
    // and every Engram one. They carry no version and must still open.
    let (temp, config) = legacy_database(EARLY_ENGRAM_SCHEMA);
    {
        let connection = Connection::open(&config.database_path).unwrap();
        assert_eq!(
            schema_version(&connection).unwrap(),
            0,
            "the fixture starts unstamped, like a real foreign database"
        );
    }

    let store = Store::open(config).unwrap();
    assert_eq!(
        schema_version(&store.connection).unwrap(),
        SCHEMA_VERSION,
        "adoption stamps the database so it never gets adopted twice"
    );
    drop(temp);
}

#[test]
fn a_database_from_a_newer_leteo_is_refused_instead_of_damaged() {
    // An older binary cannot know what a newer one changed. Writing the
    // shape it remembers would corrupt the file, so it declines to try.
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("leteo.db");
    Store::open(StoreConfig::new(path.clone())).unwrap();
    {
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(&format!("PRAGMA user_version = {}", SCHEMA_VERSION + 7))
            .unwrap();
    }

    let error = match Store::open(StoreConfig::new(path)) {
        Ok(_) => panic!("a database from the future must not open"),
        Err(error) => error.to_string(),
    };
    assert!(
        error.contains(&format!("schema version {}", SCHEMA_VERSION + 7))
            && error.contains("upgrade Leteo"),
        "the refusal should say what it found and what to do: {error}"
    );
}

#[test]
fn every_shipped_migration_is_numbered_in_order_and_within_reach() {
    let mut previous = 1;
    for (number, migration) in MIGRATIONS {
        assert!(
            *number > previous,
            "migration {number} does not come after {previous}; they must ascend"
        );
        // Every migration ships the file explaining itself, whether the file is
        // also what runs. A step carried out in Rust is the one most in need of
        // the prose, because the reasoning is nowhere near the numbers.
        let documentation = match migration {
            Migration::Sql(sql) => sql,
            Migration::Rust(documentation, _) => documentation,
        };
        assert!(
            !documentation.trim().is_empty(),
            "migration {number} is empty; a released number must do something"
        );
        previous = *number;
    }
    assert_eq!(
        previous, SCHEMA_VERSION,
        "SCHEMA_VERSION must match the last shipped migration"
    );
}

#[test]
fn an_export_stamps_the_format_version_not_the_build_version() {
    // These were the same string by coincidence. Tying the export to the
    // crate version meant the first release bump would have made Leteo
    // refuse its own earlier exports, and every Engram one.
    let (_temp, mut store) = store();
    store.create_session("s1", "Leteo", "C:/repo").unwrap();
    store
        .add_observation(observation("s1", "versioned", "body"))
        .unwrap();

    let exported: ExportData = serde_json::from_str(&store.export_json(None).unwrap()).unwrap();
    assert_eq!(exported.version, EXPORT_FORMAT_VERSION);

    let (_other_temp, mut other) = super::tests::store();
    let error = other
        .import_json(r#"{"version":"9.9.9","exported_at":"now","sessions":[]}"#)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("unsupported export format 9.9.9"),
        "unexpected error: {error}"
    );
}

#[test]
fn an_empty_full_text_index_is_reported_as_unhealthy() {
    let (_temp, mut store) = store();
    store.create_session("s1", "Leteo", "C:/repo").unwrap();
    store
        .add_observation(observation("s1", "Chose Postgres", "body"))
        .unwrap();
    assert!(store.doctor().unwrap().healthy);

    // Wipe the indexes the way a half-finished migration or a restored file
    // would leave them. `SELECT COUNT(*)` on an index still reads through to
    // the table and reports one row, so this used to pass as healthy while
    // search returned nothing.
    //
    // Both of them, because search reads both: wiping only the stemmed one
    // leaves the unstemmed one answering, which is the point of there being
    // two and would make the assertion below untrue for a good reason.
    store
        .connection
        .execute_batch(
            "INSERT INTO observations_fts(observations_fts) VALUES('delete-all');
             INSERT INTO observations_exact(observations_exact) VALUES('delete-all');",
        )
        .unwrap();
    assert!(
        store
            .search("Postgres", SearchOptions::default())
            .unwrap()
            .is_empty(),
        "the index really is empty"
    );

    let report = store.doctor().unwrap();
    assert!(
        !report.healthy,
        "a store whose search finds nothing is not healthy"
    );
    assert!(
        report
            .checks
            .iter()
            .any(|check| check.code == "observation_fts_sync" && !check.ok),
        "the mismatch should be reported by name: {:?}",
        report.checks
    );
}

/// A writer that meets a held lock waits for it instead of failing.
///
/// Two writers is the normal case: the MCP server, the HTTP server, the hooks
/// and the autosync thread all open the same file. With a deferred transaction
/// the second one used to fail immediately with "database is locked" — SQLite
/// refuses to run the busy handler for a reader that is upgrading, so the
/// five-second `busy_timeout` never applied. `write_transaction` asks for
/// `IMMEDIATE`, which is what makes the handler run.
///
/// # Why the lock is held for a fixed time rather than fought over
///
/// This used to run two threads writing as fast as they could — one bounded at
/// 600 observations, the other unbounded until told to stop — and assert that
/// neither ever saw `DatabaseBusy`. That asserts something the code cannot
/// promise. Under sustained contention the wait for a lock is a property of the
/// machine, and a loaded runner can exceed five seconds; when it does, SQLite
/// returns busy *correctly* and the test reads it as the promise broken. It
/// failed exactly that way in CI on a commit that touched no Rust at all, then
/// passed on a re-run of the same commit — the same shape of mistake as the
/// timing guards in `Store::open`, which is why that one measures
/// `budget_left_after_opening` rather than a stopwatch.
///
/// So the contention is arranged instead of provoked: one connection holds the
/// write lock for a known interval, and the second writer must come through it.
/// A slow machine makes the waiting longer, which is the direction that keeps
/// this true rather than the direction that breaks it.
#[test]
fn a_second_writer_waits_for_the_lock_instead_of_failing() {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    let temp = TempDir::new().unwrap();
    let path = temp.path().join("leteo.db");
    let mut one = Store::open(StoreConfig::new(path.clone())).unwrap();
    one.create_session("shared", "Leteo", "C:/repo").unwrap();

    // Opened before the lock is taken, because `Store::open` spends part of the
    // same budget waiting and would arrive at the write with less of it left
    // than a real second process has.
    let mut two = Store::open(StoreConfig::new(path.clone())).unwrap();

    // Long enough that the writer is certainly inside `add_observation` before
    // the lock is released, and far short of the five seconds it is allowed to
    // wait, so the margin absorbs a stall rather than being spent by one.
    let held = Duration::from_secs(1);

    let holder = rusqlite::Connection::open(&path).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();

    let (about_to_write, waiting) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        about_to_write.send(()).unwrap();
        let start = Instant::now();
        let outcome = two.add_observation(observation("shared", "b", "second writer"));
        (outcome, start.elapsed())
    });

    waiting.recv().unwrap();
    std::thread::sleep(held);
    holder.execute_batch("ROLLBACK").unwrap();
    drop(holder);

    let (outcome, waited) = writer.join().unwrap();
    outcome.expect("a writer that meets a held lock waits for it");

    // And it reached the lock while it was held, rather than tidily after it
    // was released — without this the assertion above would pass on a store
    // that never waited for anything. A tenth of the hold, because what is
    // being ruled out is "did not block at all".
    assert!(
        waited >= held / 10,
        "the second writer returned in {waited:?}, so it never met the lock \
         and this checked nothing"
    );
}

#[test]
fn opening_an_older_database_folds_the_types_already_written_to_it() {
    let temp = TempDir::new().unwrap();
    let config = StoreConfig::new(temp.path().join("leteo.db"));

    // A store written by the build before this fold existed.
    {
        let mut store = Store::open(config.clone()).unwrap();
        store.create_session("s1", "Leteo", "C:/repo").unwrap();
        for (title, kind) in [
            ("Fixed the leak", "bugfix"),
            ("Wrote the adapter", "implementation"),
        ] {
            let mut input = observation("s1", title, "body");
            input.kind = kind.to_owned();
            store.add_observation(input).unwrap();
        }
        // Behind the store's back, because writes are canonical now and
        // this is what the rows looked like before they were.
        store
            .connection
            .execute(
                "UPDATE observations SET type = 'bug' WHERE title = 'Fixed the leak'",
                [],
            )
            .unwrap();
        // Cleared rather than set to the version this ran at before. The
        // numbered migrations were folded into the baseline for the first
        // release, so the data rules now run once, on the way in, for a
        // database of unknown provenance — and an unstamped database is what
        // that means.
        store
            .connection
            .execute("PRAGMA user_version = 0", [])
            .unwrap();
    }

    let store = Store::open(config).unwrap();
    let kinds = |title: &str| -> String {
        store
            .connection
            .query_row(
                "SELECT type FROM observations WHERE title = ?1",
                params![title],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
    };
    assert_eq!(kinds("Fixed the leak"), "bugfix");
    // Not a synonym of anything documented, so it keeps its own word.
    assert_eq!(kinds("Wrote the adapter"), "implementation");
    assert_eq!(schema_version(&store.connection).unwrap(), SCHEMA_VERSION);
}

#[test]
fn a_project_that_arrived_with_capitals_stops_hiding_from_every_query() {
    // Almost every statement in the store compares a project raw —
    // `ifnull(project, '') = ?1` — against a value that went through
    // `normalize::project`, which lowercases. That is correct only while the
    // column is already lowercase, which the write path guarantees and
    // adoption does not: `engram::adopt` translates an Engram database across
    // without normalising anything.
    //
    // The damage is quiet in the way that matters. `find_candidates` is the
    // conflict detection `mem_save` runs on every save, and it filters by
    // project this way — so for an adopted memory it matched nothing and the
    // save reported `candidates: []`. That reads as "nothing here contradicts
    // this", when nothing was looked at.
    let (_temp, config) = legacy_database(PRE_CONFLICT_SCHEMA_WITH_OLD_FTS);
    {
        let connection = Connection::open(&config.database_path).unwrap();
        connection
            .execute_batch(
                "UPDATE sessions SET project = 'MyProject';
                 UPDATE observations SET project = 'MyProject';",
            )
            .unwrap();
    }

    let mut store = Store::open(config).unwrap();

    // The column now holds what the queries assume.
    let raw: Vec<String> = store
        .connection
        .prepare("SELECT DISTINCT ifnull(project, '') FROM observations")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(raw, ["myproject"], "the migration folds the case");

    // And the queries that assumed it can see the memory again. This is the
    // one that mattered: a save that cannot see its own project reports no
    // conflicts rather than reporting that it could not look.
    let session: String = store
        .connection
        .query_row("SELECT id FROM sessions LIMIT 1", [], |row| row.get(0))
        .unwrap();
    // Filler first, and load-bearing. A candidate has to score past the floor
    // `find_candidates` applies, and a bm25 term weight grows with how rare a
    // word is across the store: on a two-memory fixture a term both of them
    // carry scores *above* zero, so nothing reaches any floor at all.
    for index in 0..40 {
        store
            .add_observation(AddObservation {
                session_id: session.clone(),
                kind: "discovery".to_owned(),
                title: format!("Unrelated note {index} on deployment windows"),
                content: format!("Body {index}: staged rollout, canaries, rollback."),
                tool_name: None,
                project: Some("MyProject".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
    }
    let saved = store
        .add_observation(AddObservation {
            session_id: session,
            kind: "discovery".to_owned(),
            title: "Normalized tokenizer panic on edge case".to_owned(),
            content: "a second account of the same thing".to_owned(),
            tool_name: None,
            project: Some("MyProject".to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            prompt_sync_id: None,
        })
        .unwrap()
        .observation;
    let candidates = store
        .find_candidates(saved.id, CandidateOptions::default())
        .unwrap();
    assert!(
        !candidates.is_empty(),
        "a save has to see the memory it may be contradicting"
    );

    // And the full-text index was rebuilt, so the rows the migration rewrote
    // are still findable — an UPDATE goes around the triggers.
    assert!(
        !store
            .search("tokenizer panic", SearchOptions::default())
            .unwrap()
            .is_empty()
    );
}

/// The listings must narrow by project, not ask every row whether to.
///
/// `WHERE (?1 IS NULL OR project = ?1)` serves both callers with one prepared
/// statement and reads like the obvious thing to write. It also costs a full
/// table scan on every call, including the calls that name a project: SQLite
/// picks its plan before `?1` is bound, so a column in a disjunction with a
/// parameter is not an index term, and the plan that works either way is
/// `SCAN`. Measured on 3,587 memories: 5.7 ms against 0.015 ms.
///
/// A result-based test cannot see this — both forms return exactly the same
/// rows, which is precisely why it sat here unnoticed while a session opening
/// paid it four times. So the guard reads the source.
///
/// Not every disjunction is wrong. The three left in `search.rs` sit behind a
/// full-text `MATCH` or `topic_key =`, which drives the plan and leaves the
/// project a residual filter over a handful of candidates — measured at 0.008
/// against 0.007 ms, which is nothing. The rule is about the queries that have
/// no other index term to lean on.
#[test]
fn the_project_listings_narrow_rather_than_scan() {
    let listings = [
        ("count_observations", include_str!("../observations.rs")),
        ("recent_observations", include_str!("../observations.rs")),
        ("pinned_observations", include_str!("../observations.rs")),
        ("review_due", include_str!("../observations.rs")),
        ("recent_prompts", include_str!("../prompts.rs")),
        ("recent_sessions", include_str!("../sessions.rs")),
    ];
    for (function, source) in listings {
        let start = source
            .find(&format!("fn {function}("))
            .unwrap_or_else(|| panic!("{function} is in this file"));
        let body = &source[start..];
        let end = body.find("\n    }\n").map_or(body.len(), |at| at + 6);
        let body = &body[..end];

        assert!(
            body.contains("narrowing.equals(\"") || body.contains(".equals(\""),
            "{function} has to build its project narrowing, not embed one"
        );
        assert!(
            !body.contains("IS NULL OR"),
            "{function} asks every row whether the filter applies; that plan \
             is a SCAN whatever the parameter turns out to be"
        );
    }
}

/// The case 0004 could not reach: a store that was already past it.
///
/// 0004 folded the rows in front of it and declared the convention true by
/// construction. Adoption then went on writing rows that broke it — it builds a
/// fresh database, so every migration runs against nothing, and only afterwards
/// does it copy Engram's rows in. A store adopted after 0004 shipped is stamped
/// version 4 and holds exactly what 0004 existed to remove.
///
/// `--` rather than capitals, because that is the half 0004 never did.
/// `normalize::project` collapses repeated separators as well as folding case,
/// so `My--Project` is asked for as `my-project` and lowercasing alone leaves
/// it just as unreachable as it started.
#[test]
fn a_project_adopted_after_the_lowercasing_migration_is_still_folded() {
    let (_temp, config) = legacy_database(PRE_CONFLICT_SCHEMA_WITH_OLD_FTS);
    {
        // Open once so the database converges and is stamped current, which is
        // the state an adopted store is left in.
        let store = Store::open(config.clone()).unwrap();
        assert_eq!(schema_version(&store.connection).unwrap(), SCHEMA_VERSION);
    }
    {
        // Then write the way adoption does: straight past the write path, and
        // clear the stamp so the next open treats it as a database of unknown
        // provenance — which since the first release is the only kind there is
        // to converge.
        let connection = Connection::open(&config.database_path).unwrap();
        connection
            .execute_batch(
                "UPDATE sessions SET project = 'My--Project';
                 UPDATE observations SET project = 'My--Project';
                 PRAGMA user_version = 0;",
            )
            .unwrap();
    }

    let store = Store::open(config).unwrap();
    let raw: Vec<String> = store
        .connection
        .prepare("SELECT DISTINCT ifnull(project, '') FROM observations")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        raw,
        [crate::memory::normalize::project("My--Project")],
        "the separators collapse too, not just the case"
    );
    assert_eq!(schema_version(&store.connection).unwrap(), SCHEMA_VERSION);

    // Rebuilt, for the same reason 0004 rebuilt: these updates go around the
    // triggers that keep `project` current in the index.
    assert!(
        !store
            .search("tokenizer panic", SearchOptions::default())
            .unwrap()
            .is_empty()
    );
}

/// Nine hundred memories were stored, indexed, undamaged and unreachable.
///
/// `mem_session_summary` titled every summary `Session summary: <project>` and
/// nothing else, so a real store held 507 under one name. A title is the
/// highest-weighted field in the ranking — 5.0 against 1.0 for the body — and
/// it was identifying nothing: asked for its own title, such a memory competes
/// with five hundred identical ones and loses. Measured over that store, 9.6%
/// of summaries came back for their own title against 99.9% of the memories
/// with a title of their own. After this migration, 97.8%.
#[test]
fn session_summaries_are_retitled_by_what_each_session_was_for() {
    let (_temp, config) = legacy_database(PRE_CONFLICT_SCHEMA_WITH_OLD_FTS);
    {
        let store = Store::open(config.clone()).unwrap();
        assert_eq!(schema_version(&store.connection).unwrap(), SCHEMA_VERSION);
    }
    {
        let connection = Connection::open(&config.database_path).unwrap();
        // Three summaries with one name between them, which is the defect, and
        // the shape a real one has. The third has nothing worth lifting.
        for (id, body) in [
            (
                901,
                "## Goal\nAudit the cookie and privacy policies against the code\n\n## Notes\n.",
            ),
            (
                902,
                "## Goal\nRestore deterministic chunk ordering after the rebuild\n",
            ),
            (903, "## Goal\n2026-08-02\n"),
            // The shape that defeats a literal reading of the second line: a
            // title line, a blank, and only then the goal. 22 of 898 real
            // summaries look like this, and they are exactly the ones a
            // `substr` migration left with the name it was written to replace.
            (
                904,
                "# Session Summary — cleanup\n\n## Goal\nRetire the duplicated chunk writer\n",
            ),
        ] {
            connection
                .execute(
                    "INSERT INTO observations (id, sync_id, session_id, type, title, content,
                         project, scope, created_at, updated_at)
                     SELECT ?1, ?2, id, 'session_summary', 'Session summary: myproject', ?3,
                         'myproject', 'project', datetime('now'), datetime('now')
                     FROM sessions LIMIT 1",
                    params![id, format!("sum-{id}"), body],
                )
                .unwrap();
        }
        connection
            .execute_batch("PRAGMA user_version = 0; UPDATE sessions SET project = 'myproject';")
            .unwrap();
    }

    let store = Store::open(config).unwrap();
    let title = |id: i64| -> String {
        store
            .connection
            .query_row(
                "SELECT title FROM observations WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(
        title(901),
        "Session summary: myproject — Audit the cookie and privacy policies against the code"
    );
    assert_eq!(
        title(902),
        "Session summary: myproject — Restore deterministic chunk ordering after the rebuild"
    );
    // A bare date is not a headline. Keeping the old name beats inventing a
    // worse one.
    assert_eq!(title(903), "Session summary: myproject");
    // Blanks and headings are skipped rather than counted, so a summary that
    // opens with its own title line still gets a headline.
    assert_eq!(
        title(904),
        "Session summary: myproject — Retire the duplicated chunk writer"
    );

    // The point of all of it: the memory can now be found by its own title,
    // which needs the index rebuilt because an UPDATE goes around the triggers.
    let found = store
        .search("deterministic chunk ordering", SearchOptions::default())
        .unwrap();
    assert!(
        found.iter().any(|result| result.observation.id == 902),
        "a retitled summary has to be reachable by what it was retitled to"
    );
}

/// A store from the pre-release numbering is refused, and says both numbers.
///
/// Eleven migrations accumulated before anything shipped, and the numbering
/// started again at 1 when they were folded into the baseline. A database
/// carried through development is stamped somewhere in 2..=17 and this build
/// understands 1, so it is refused — deliberately, because from here on a
/// number above `SCHEMA_VERSION` means a newer build wrote the file, and
/// guessing which of the two it is would be worse than either.
///
/// The one store in the world in that position is re-stamped by hand. Code that
/// recognised the old numbering would outlive the reason for it, and this is a
/// test rather than that code: what it holds is that the refusal names what was
/// found and what is understood, so whoever meets it knows which it is.
#[test]
fn a_store_from_the_pre_release_numbering_is_refused_and_says_both_numbers() {
    for stamped in [2, 6, 15, 16, 17, SCHEMA_VERSION + 1] {
        let temp = tempfile::tempdir().unwrap();
        let config = StoreConfig::new(temp.path().join("old.db"));
        {
            let store = Store::open(config.clone()).unwrap();
            store
                .connection
                .execute_batch(&format!("PRAGMA user_version = {stamped}"))
                .unwrap();
        }
        let said = match Store::open(config) {
            Err(error) => error.to_string(),
            Ok(_) => panic!("a store stamped {stamped} is above what this build knows"),
        };
        assert!(
            said.contains(&stamped.to_string()) && said.contains(&SCHEMA_VERSION.to_string()),
            "the refusal names what it found and what it understands: {said}"
        );
    }
}

/// Several agents upgrading one database at once end with all of it, once.
///
/// This is what an upgrade looks like in practice: the hooks of whatever
/// sessions are open all fire at the same file, and today's migrations build a
/// second full-text index and delete a journal. Measured against a copy of a
/// real store, eight processes at once all returned their context and the file
/// came out at the current version with both indexes complete — and killing one
/// mid-migration left the old version stamped and nothing half-built, because
/// the whole run is one transaction.
///
/// Threads rather than processes so it stays a unit test; they exercise the
/// same `prepare` retry loop, which is what turns "database is locked" into
/// waiting.
#[test]
fn several_openers_of_a_database_that_needs_upgrading_all_get_it_whole() {
    use std::sync::{Arc, Barrier};

    let temp = TempDir::new().unwrap();
    let path = temp.path().join("leteo.db");
    {
        let mut store = Store::open(StoreConfig::new(path.clone())).unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        for index in 0..30 {
            store
                .add_observation(observation(
                    "s1",
                    &format!("Memoria número {index}"),
                    "Un cuerpo cualquiera.",
                ))
                .unwrap();
        }
        // Wound back to what a database from before today looks like.
        store
            .connection
            .execute_batch(
                "DROP INDEX IF EXISTS idx_obs_created_order;
                 DROP TRIGGER IF EXISTS obs_exact_insert;
                 DROP TRIGGER IF EXISTS obs_exact_delete;
                 DROP TRIGGER IF EXISTS obs_exact_update;
                 DROP TABLE IF EXISTS observations_exact;
                 PRAGMA user_version = 0;",
            )
            .unwrap();
    }

    let barrier = Arc::new(Barrier::new(6));
    let openers = (0..6)
        .map(|_| {
            let config = StoreConfig::new(path.clone());
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                Store::open(config).map(|_| ())
            })
        })
        .collect::<Vec<_>>();
    for opener in openers {
        opener
            .join()
            .expect("the opening thread did not panic")
            .expect("an upgrade several agents race is still an upgrade");
    }

    let store = Store::open(StoreConfig::new(path)).unwrap();
    assert_eq!(schema_version(&store.connection).unwrap(), SCHEMA_VERSION);
    let count = |sql: &str| -> i64 {
        store
            .connection
            .query_row(sql, [], |row| row.get(0))
            .unwrap()
    };
    assert_eq!(
        count("SELECT COUNT(*) FROM observations_exact_docsize"),
        count("SELECT COUNT(*) FROM observations"),
        "the second index holds every memory, and exactly once"
    );
    assert_eq!(
        count(
            "SELECT COUNT(*) FROM sqlite_master
              WHERE type = 'trigger' AND name LIKE 'obs_exact_%'"
        ),
        3,
        "an index without its triggers is one nothing keeps up to date"
    );
    assert_eq!(
        count(
            "SELECT COUNT(*) FROM sqlite_master
              WHERE type = 'index' AND name = 'idx_obs_created_order'"
        ),
        1
    );
    let integrity: String = store
        .connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
}

/// The type vocabulary reaches memories that were saved before it existed.
///
/// There is a fold at the door and a fold for a database of unknown
/// provenance, and between them they cover everything except the case that
/// actually happened: a store stamped with a version, holding rows written
/// before the vocabulary was. The canonicalising pass only runs on an
/// unstamped database, so a real store at version 11 kept eighteen memories
/// typed `manual` — the default value of `mem_save`'s `type`, which is to say
/// the word left behind by a caller who did not choose, and one no agent ever
/// searches for.
#[test]
fn a_type_the_vocabulary_folds_is_folded_in_a_store_that_was_already_stamped() {
    let temp = TempDir::new().unwrap();
    let config = StoreConfig::new(temp.path().join("leteo.db"));
    {
        let mut store = Store::open(config.clone()).unwrap();
        store.enroll_project("leteo").unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        for (title, kind) in [
            ("A memory nobody typed", "discovery"),
            ("Another one", "implementation"),
        ] {
            let mut input = observation("s1", title, "body");
            input.kind = kind.to_owned();
            store.add_observation(input).unwrap();
        }
        // Behind the store's back and stamped one version short, which is what
        // a store that has been running since before the fold looks like.
        store
            .connection
            .execute_batch(
                "UPDATE observations SET type = 'manual' WHERE title = 'A memory nobody typed';
                 PRAGMA user_version = 0;",
            )
            .unwrap();
    }

    let store = Store::open(config).unwrap();
    let kind_of = |title: &str| -> String {
        store
            .connection
            .query_row(
                "SELECT type FROM observations WHERE title = ?1",
                params![title],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
    };
    assert_eq!(
        kind_of("A memory nobody typed"),
        "discovery",
        "a word the vocabulary folds has to be folded wherever it is"
    );
    assert_eq!(
        kind_of("Another one"),
        "implementation",
        "and a word it does not recognise keeps its own, because an honest \
         unknown type still says something true"
    );
    // The indexes follow by their triggers, or a typed search would still find
    // the old word and miss the new one.
    let indexed: i64 = store
        .connection
        .query_row(
            "SELECT count(*) FROM observations_fts WHERE observations_fts MATCH 'type:manual'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(indexed, 0, "the full-text index still holds the old type");
}

/// Rows already filed under a directory find their project again.
///
/// `normalize::project` reduces a path-shaped name to its last segment, and
/// that runs at the door — so it only ever applied to what was written after
/// it. A real store had 44 prompts and sessions left behind, under three
/// distinct paths whose last segments were all projects it actually held.
///
/// Nothing found those rows as they stood: every read narrows by project, so
/// they sat in a project that existed nowhere else, out of every opening
/// context.
#[test]
fn prompts_and_sessions_filed_under_a_directory_are_moved_to_the_project() {
    let temp = TempDir::new().unwrap();
    let config = StoreConfig::new(temp.path().join("leteo.db"));
    {
        let mut store = Store::open(config.clone()).unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        store
            .add_prompt(AddPrompt {
                session_id: "s1".to_owned(),
                content: "a question that was asked".to_owned(),
                project: Some("leteo".to_owned()),
            })
            .unwrap();
        // Behind the store's back, which is the only way such a row can exist
        // now and exactly what an older store is full of.
        store
            .connection
            .execute_batch(
                r#"UPDATE prompts SET project = 'h:\repo\nas.archive';
                 UPDATE sessions SET project = '\users\asanabrial\skills\task-board\';
                 PRAGMA user_version = 0;"#,
            )
            .unwrap();
    }

    let store = Store::open(config).unwrap();
    let project_of = |table: &str| -> String {
        store
            .connection
            .query_row(&format!("SELECT project FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    };
    assert_eq!(project_of("prompts"), "nas.archive");
    assert_eq!(project_of("sessions"), "task-board");
}

/// The question `mem_review` asks is answered by an index, not by a scan.
///
/// It filters and orders by `datetime(review_after)`, so a plain index on the
/// column is not one SQLite can use — the same reason the ordering index is on
/// `datetime(created_at)`. Without this one the tool read every row and sorted
/// the answer in a temporary B-tree: 4.5 ms on a real store to answer with
/// nothing, and 14 ms once it holds sixty thousand memories.
#[test]
fn the_review_queue_is_read_through_its_own_index() {
    // Enough memories for the planner to prefer an index, and its statistics
    // refreshed the way opening a store refreshes them.
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    for index in 0..40 {
        let mut input = observation("s1", &format!("A decision {index}"), "body");
        input.kind = "decision".to_owned();
        store.add_observation(input).unwrap();
    }
    store.connection.execute_batch("ANALYZE").unwrap();
    let plan: Vec<String> = store
        .connection
        .prepare(
            "EXPLAIN QUERY PLAN
             SELECT id FROM observations
              WHERE deleted_at IS NULL AND review_after IS NOT NULL
                AND datetime(review_after) <= datetime('now')
              ORDER BY datetime(review_after) ASC, id ASC LIMIT 20",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let plan = plan.join(" | ");
    assert!(
        plan.contains("idx_obs_review_due"),
        "the review queue is scanned rather than looked up: {plan}"
    );
    assert!(
        !plan.contains("TEMP B-TREE"),
        "the index carries the ordering too, or it is only half an index: {plan}"
    );
}

/// A review clock left behind by a revision is wound for the type it ended up
/// being.
///
/// Only three kinds go stale. Revising a memory used to leave the clock alone,
/// and its type can change on that path, so a real store held both mistakes at
/// once: 19 memories asking to be reread that never go stale — bugfixes,
/// architecture notes, discoveries, all with `revision_count` above one — and
/// 14 decisions and preferences that would never be asked about at all.
///
/// The third case is the one that keeps this honest: a date already correct is
/// left exactly as it was, because the fix fills holes rather than rewriting a
/// column.
#[test]
fn a_review_clock_a_revision_left_behind_is_wound_for_the_type_it_became() {
    let temp = TempDir::new().unwrap();
    let config = StoreConfig::new(temp.path().join("leteo.db"));
    let settled;
    {
        let mut store = Store::open(config.clone()).unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        for (title, kind) in [
            ("A bugfix that thinks it goes stale", "bugfix"),
            ("A decision nobody will ever reread", "decision"),
            ("A preference nobody will ever reread", "preference"),
            ("A decision with a clock already right", "decision"),
        ] {
            let mut input = observation("s1", title, "body");
            input.kind = kind.to_owned();
            store.add_observation(input).unwrap();
        }
        settled = store
            .connection
            .query_row(
                "SELECT review_after FROM observations WHERE title = 'A decision with a clock already right'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        // What a store that ran before `reschedule_review` looks like: a clock
        // on something that does not go stale, and none on things that do.
        store
            .connection
            .execute_batch(
                "UPDATE observations SET review_after = datetime('now', '+40 days')
                  WHERE title = 'A bugfix that thinks it goes stale';
                 UPDATE observations SET review_after = NULL
                  WHERE title LIKE '%nobody will ever reread';
                 PRAGMA user_version = 0;",
            )
            .unwrap();
    }

    let store = Store::open(config).unwrap();
    let clock = |title: &str| -> Option<String> {
        store
            .connection
            .query_row(
                "SELECT review_after FROM observations WHERE title = ?1",
                params![title],
                |row| row.get::<_, Option<String>>(0),
            )
            .unwrap()
    };

    assert_eq!(
        clock("A bugfix that thinks it goes stale"),
        None,
        "a bugfix is as true in a year as it is today"
    );
    assert!(
        clock("A decision nobody will ever reread").is_some(),
        "a decision without a clock is one nobody is ever asked about"
    );
    assert!(clock("A preference nobody will ever reread").is_some());
    assert_eq!(
        clock("A decision with a clock already right"),
        Some(settled),
        "a clock that was already right is left alone"
    );

    // And the window is the one the rules say, counted from when the memory was
    // written rather than from whenever this happened to run.
    //
    // Against `rules::review_after` rather than against a span of days written
    // here. This asserted `175..=181` when the migration counted months of
    // thirty days, and a fourth opinion about how long six months is was
    // exactly what the consolidation was for.
    let due: String = clock("A decision nobody will ever reread").unwrap();
    let expected = crate::memory::rules::review_after("decision", chrono::Utc::now().naive_utc())
        .map(crate::timestamp::format)
        .expect("a decision has a window");
    assert_eq!(
        due.get(..10),
        expected.get(..10),
        "a decision is reread when the rules say, and nowhere else"
    );
}

/// The session list is grouped through a covering index, not through the table.
///
/// Every opening block asks which sessions were touched last, which means
/// counting a project's memories per session and taking each one's newest date.
/// `idx_obs_session` finds a session's rows and stops there, so the group read
/// the table for every one of them — bodies included, to use a count and a
/// date. On a real store that was 3.36 ms of a 7.53 ms opening block, spent on
/// five rows; 0.64 ms with the index, and 2.13 ms against 0.16 on the next
/// project down.
///
/// The plan is what is asserted rather than the time, because timing does not
/// transfer between SQLite builds and this does: what changed is that the table
/// is not read at all.
#[test]
fn the_session_list_is_grouped_through_a_covering_index() {
    let (_temp, mut store) = store();
    for session in 0..6 {
        let id = format!("s{session}");
        store.create_session(&id, "leteo", "C:/repo").unwrap();
        for index in 0..12 {
            store
                .add_observation(observation(
                    &id,
                    &format!("Una memoria {session}-{index}"),
                    "un cuerpo cualquiera",
                ))
                .unwrap();
        }
    }
    store.connection.execute_batch("ANALYZE").unwrap();
    let plan: Vec<String> = store
        .connection
        .prepare(
            "EXPLAIN QUERY PLAN
             SELECT s.id, s.project, s.started_at, s.ended_at, s.summary, COUNT(o.id),
                    MAX(datetime(COALESCE(o.created_at, s.started_at)))
               FROM sessions s
               LEFT JOIN observations o ON o.session_id = s.id AND o.deleted_at IS NULL
              WHERE s.project = ?1
              GROUP BY s.id
             HAVING COUNT(o.id) > 0 OR trim(ifnull(s.summary, '')) <> ''
              ORDER BY MAX(datetime(COALESCE(o.created_at, s.started_at))) DESC, s.id DESC
              LIMIT 5",
        )
        .unwrap()
        .query_map(["leteo"], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let plan = plan.join(" | ");
    assert!(
        plan.contains("COVERING INDEX idx_obs_session_activity"),
        "the group reads the table instead of the index: {plan}"
    );
}
