//! Counting and checking the store as a whole.

use super::*;

#[test]
fn an_export_written_by_go_imports_despite_its_null_lists() {
    // Go marshals an empty slice as `null`, so an Engram export with no
    // prompts writes `"prompts": null`. This is the exact shape the
    // upstream binary produced; it used to fail the whole import with
    // "invalid type: null, expected a sequence".
    let (_temp, mut store) = store();
    let engram_export = r#"{
          "version": "0.1.0",
          "exported_at": "2026-07-28 17:00:39",
          "sessions": [
            {
              "id": "manual-save-interop",
              "project": "interop",
              "directory": "H:\\REPO\\leteo",
              "started_at": "2026-07-28 17:00:39"
            }
          ],
          "observations": [
            {
              "id": 1,
              "sync_id": "obs-d68e66f5cda65b1e",
              "session_id": "manual-save-interop",
              "type": "manual",
              "title": "Chose Postgres over MySQL",
              "content": "**What**: picked Postgres",
              "project": "interop",
              "scope": "project",
              "revision_count": 1,
              "duplicate_count": 1,
              "last_seen_at": "2026-07-28 17:00:39",
              "created_at": "2026-07-28 17:00:39",
              "updated_at": "2026-07-28 17:00:39"
            }
          ],
          "prompts": null
        }"#;

    let imported = store.import_json(engram_export).unwrap();
    assert_eq!(imported.sessions_imported, 1);
    assert_eq!(imported.observations_imported, 1);
    assert_eq!(imported.prompts_imported, 0);
    assert_eq!(
        store
            .search("Postgres", SearchOptions::default())
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn a_validation_failure_reads_as_the_callers_mistake() {
    let (_temp, store) = store();
    let message = store
        .doctor_scoped(Some("bogus"), None)
        .unwrap_err()
        .to_string();
    // The message used to be wrapped twice, as "database error: Invalid
    // parameter name: ...", which named the wrong culprit.
    assert!(
        !message.contains("database error") && !message.contains("Invalid parameter name"),
        "a caller's mistake should not be dressed up as a SQLite failure: {message}"
    );
    assert!(message.starts_with("unknown check"), "{message}");
}

#[test]
fn a_doctor_check_selects_one_diagnostic_and_refuses_a_typo() {
    let (_temp, store) = store();

    let (full, stats) = store.doctor_scoped(None, None).unwrap();
    assert_eq!(
        full.checks.len(),
        DoctorCheck::CODES.len(),
        "the unfiltered report carries every code"
    );
    assert!(stats.is_none());

    let (one, _) = store.doctor_scoped(Some("journal_mode"), None).unwrap();
    assert_eq!(one.checks.len(), 1);
    assert_eq!(one.checks[0].code, "journal_mode");
    assert!(one.checks[0].ok, "a fresh store is in WAL");

    // A code nobody reports must say so rather than answer "all clear",
    // which is what an inert parameter used to do.
    let error = store.doctor_scoped(Some("not_a_check"), None).unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("unknown check") && message.contains("journal_mode"),
        "the error should list the valid codes: {message}"
    );
}

#[test]
fn an_export_never_captures_half_of_a_concurrent_write() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    // An export reads sessions and observations as two separate statements,
    // so it can land between a writer's session insert and the observation
    // that belongs to it: no session row yet, observation row already
    // there. The resulting chunk references a session it does not carry,
    // and the cloud rejects the whole push.
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("leteo.db");
    let mut writer = Store::open(StoreConfig::new(path.clone())).unwrap();
    let exporter = Store::open(StoreConfig::new(path)).unwrap();

    // Enough existing rows that reading them takes long enough for a writer
    // to commit in between.
    for index in 0..1_200 {
        let session = format!("seeded-session-{index}");
        writer.create_session(&session, "Leteo", "C:/repo").unwrap();
        writer
            .add_observation(observation(&session, "seeded", "existing history"))
            .unwrap();
    }

    let stop = Arc::new(AtomicBool::new(false));
    let writer_stop = Arc::clone(&stop);
    let writing = std::thread::spawn(move || {
        let mut index = 0_u32;
        while !writer_stop.load(Ordering::Relaxed) {
            let session = format!("concurrent-session-{index}");
            writer.create_session(&session, "Leteo", "C:/repo").unwrap();
            writer
                .add_observation(observation(
                    &session,
                    "concurrent",
                    "written while exporting",
                ))
                .unwrap();
            index += 1;
        }
        index
    });

    let mut checked = 0_u32;
    for _ in 0..150 {
        let data = exporter.export().unwrap();
        let sessions = data
            .sessions
            .iter()
            .map(|session| session.id.as_str())
            .collect::<std::collections::HashSet<_>>();
        for observation in &data.observations {
            assert!(
                sessions.contains(observation.session_id.as_str()),
                "the export carried observation {} without its session {:?}",
                observation.id,
                observation.session_id
            );
        }
        checked += 1;
    }

    stop.store(true, Ordering::Relaxed);
    let written = writing.join().unwrap();
    assert!(
        written > 0 && checked == 150,
        "the writer and exporter must have actually raced ({written} writes, {checked} exports)"
    );
}

#[test]
fn every_declared_code_is_one_the_doctor_actually_reports() {
    // `CODES` is what `--check` validates against, and the report is what the
    // filter matches on. Nothing holds the two together: rename a code in one
    // place and the count stays right, `--check` accepts the name, and the
    // filter matches nothing — an empty report that reads as "all clear",
    // which is the failure `doctor_scoped` was written to prevent.
    let (_temp, store) = store();

    let (report, _) = store.doctor_scoped(None, None).unwrap();
    let reported: BTreeSet<&str> = report
        .checks
        .iter()
        .map(|check| check.code.as_str())
        .collect();
    let declared: BTreeSet<&str> = DoctorCheck::CODES.iter().copied().collect();

    assert_eq!(
        reported, declared,
        "a code the doctor reports must be one --check accepts, and the other way round"
    );

    for code in DoctorCheck::CODES {
        let (narrowed, _) = store.doctor_scoped(Some(code), None).unwrap();
        assert_eq!(
            narrowed.checks.len(),
            1,
            "--check {code} has to select exactly its own diagnostic"
        );
        assert_eq!(narrowed.checks[0].code, *code);
    }
}

/// The two numbers that mattered when a store stopped opening.
///
/// A newer build migrates a store the first time it opens one — silently, one
/// way, and from then on every older binary on the machine refuses it. That
/// happened for real: a store stamped 4 against an installed CLI that read 3,
/// and the only place either number appeared was in the error from the binary
/// that could not open it. Answering "what is my store at?" meant opening the
/// file with something that was not Leteo.
///
/// They agree here by construction — a version this build does not understand
/// never reaches a report — and that is the point. The report is what you run
/// on the binary that *does* work, to learn what the other one has to match.
#[test]
fn the_doctor_says_what_the_store_is_at_and_what_this_build_reads() {
    let (_temp, store) = store();
    let report = store.doctor().unwrap();

    assert_eq!(report.schema_supported, SCHEMA_VERSION);
    assert_eq!(
        report.schema_version, SCHEMA_VERSION,
        "a store this build opened has been brought to the version it reads"
    );
    assert!(
        report.schema_version > 0,
        "an unstamped database is adopted and stamped before any report exists"
    );
}

/// A check that could not be made is not a check that failed.
///
/// SQLite's FTS5 integrity check is run by writing a magic row into the index,
/// so anything that stops a write stops the check: a database opened read-only,
/// one another process holds, a disk with nothing left on it. All of those were
/// reported as `the observation full-text index failed its integrity check`, in
/// the list of what is wrong with the store — so somebody whose only problem
/// was a file permission was told their full-text index had failed, which is
/// the kind of thing people rebuild an index over.
///
/// The verdict stays the same: a store that could not be inspected has not
/// passed. What changes is the sentence, and it is one sentence now — `issues`
/// and the check's own detail used to be written separately and contradicted
/// each other in the same reply.
#[test]
fn an_index_that_could_not_be_checked_does_not_read_as_one_that_failed() {
    let temp = TempDir::new().unwrap();
    let config = StoreConfig::new(temp.path().join("readonly.db"));
    {
        let mut store = Store::open(config.clone()).unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        store
            .add_observation(observation("s1", "A memory", "with a body"))
            .unwrap();
        let healthy = store.doctor().unwrap();
        assert!(healthy.healthy, "{:?}", healthy.issues);
    }

    // Read-only, which is a store Leteo can answer from and cannot check.
    for entry in std::fs::read_dir(temp.path()).unwrap().flatten() {
        let mut permissions = entry.metadata().unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(entry.path(), permissions).unwrap();
    }

    let store = Store::open(config).unwrap();
    let report = store.doctor().unwrap();

    assert!(
        !report.healthy,
        "a store that could not be inspected has not passed"
    );
    let integrity: Vec<&String> = report
        .checks
        .iter()
        .filter(|check| !check.ok && check.code.contains("fts_integrity"))
        .filter_map(|check| check.detail.as_ref())
        .collect();
    assert!(!integrity.is_empty(), "{report:?}");
    for detail in &integrity {
        assert!(
            detail.contains("could not be checked"),
            "the reason has to be the one that happened: {detail}"
        );
        assert!(
            !detail.contains("failed its integrity check"),
            "and not the one that did not: {detail}"
        );
    }
    for detail in integrity {
        assert!(report.issues.contains(detail), "{report:?}");
    }

    for entry in std::fs::read_dir(temp.path()).unwrap().flatten() {
        let mut permissions = entry.metadata().unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(entry.path(), permissions).unwrap();
    }
}

/// No sentence doctor prints carries the indentation of the code it lives in.
///
/// A message written across two lines of Rust keeps every space of the second
/// line unless the continuation is exact, and the result reads as a stutter:
/// `describes them, so                      nothing can be`. It has happened
/// five times in this codebase and four of them were caught by the guard over
/// the MCP hints — which does not see these, because `doctor`'s issues are
/// built where the check is made.
///
/// Every issue, from a store deliberately broken in every way the report knows
/// how to name.
#[test]
fn no_sentence_doctor_prints_carries_the_code_indentation_with_it() {
    let temp = TempDir::new().unwrap();
    let config = StoreConfig::new(temp.path().join("stutter.db"));
    let mut store = Store::open(config).unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    store
        .add_observation(observation("s1", "A memory", "with a body"))
        .unwrap();

    store
        .connection()
        .execute_batch(
            "UPDATE observations SET normalized_hash = 'not the hash of anything';
             INSERT INTO observations_fts(observations_fts) VALUES('delete-all');
             INSERT INTO observations_exact(observations_exact) VALUES('delete-all');
             INSERT INTO prompts_fts(prompts_fts) VALUES('delete-all');",
        )
        .unwrap();

    let report = store.doctor().unwrap();
    assert!(!report.healthy, "the store was broken on purpose");
    assert!(report.issues.len() >= 2, "{:?}", report.issues);

    for issue in &report.issues {
        assert!(
            !issue.contains("   "),
            "a run of spaces inside a sentence is source indentation that escaped: {issue:?}"
        );
        assert!(
            !issue.contains('\n'),
            "an issue is one line, because it is printed in a list: {issue:?}"
        );
    }
    for check in &report.checks {
        if let Some(detail) = &check.detail {
            assert!(!detail.contains("   "), "{}: {detail:?}", check.code);
        }
    }
}

/// A hash that has stopped describing its memory is found, and put back.
///
/// The hash is what dedupe compares — a save whose body matches an existing one
/// bumps that row instead of writing a second — so a hash that no longer
/// matches its own content is a memory nothing can ever be deduplicated
/// against, silently and for good. A real store of 3,940 held three, all from
/// one project on one day, none of them ever revised, and the text their hashes
/// were taken of is in no row of that store.
///
/// Nothing noticed. This is the same kind of invariant as the index checks
/// beside it: two things the store keeps that have to agree.
#[test]
fn a_hash_that_stopped_describing_its_memory_is_found_and_put_back() {
    let temp = TempDir::new().unwrap();
    let config = StoreConfig::new(temp.path().join("hashes.db"));
    let mut store = Store::open(config).unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let kept = store
        .add_observation(observation("s1", "Left alone", "a body nobody touched"))
        .unwrap()
        .observation;
    let broken = store
        .add_observation(observation("s1", "Broken", "a body whose hash will drift"))
        .unwrap()
        .observation;
    assert!(store.doctor().unwrap().healthy);

    store
        .connection()
        .execute(
            "UPDATE observations SET normalized_hash = 'not the hash of anything' WHERE id = ?1",
            rusqlite::params![broken.id],
        )
        .unwrap();

    let report = store.doctor().unwrap();
    assert!(!report.healthy);
    assert!(
        report
            .checks
            .iter()
            .any(|check| check.code == "observation_hash_sync" && !check.ok),
        "{report:?}"
    );

    assert_eq!(store.recompute_stale_hashes().unwrap(), 1);
    assert!(store.doctor().unwrap().healthy);
    // And nothing else moved: the hash is derived, the body is the truth.
    let hash_of = |id: i64| -> String {
        store
            .connection()
            .query_row(
                "SELECT normalized_hash FROM observations WHERE id = ?1",
                rusqlite::params![id],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(
        hash_of(kept.id),
        crate::memory::normalize::normalized_hash(&kept.content)
    );
    assert_eq!(
        store.get_observation(broken.id).unwrap().content,
        broken.content
    );
    assert_eq!(store.recompute_stale_hashes().unwrap(), 0);
}

/// A lost trigger is named, put back, and the index caught up with the edits.
///
/// The triggers are the whole mechanism keeping a full-text index level with
/// its table: nothing else writes to one. A store that has lost them goes on
/// answering searches — with yesterday's words, for good, and without a word of
/// complaint. `observation_fts_sync` cannot see it, because it compares how
/// many rows each side holds and editing a memory leaves both counts alone.
#[test]
fn a_full_text_trigger_that_went_missing_is_named_and_put_back() {
    let temp = TempDir::new().unwrap();
    let config = StoreConfig::new(temp.path().join("triggers.db"));
    let mut store = Store::open(config).unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let saved = store
        .add_observation(observation(
            "s1",
            "Un titulo cualquiera",
            "zurriagazo inicial",
        ))
        .unwrap()
        .observation;
    assert!(store.doctor().unwrap().healthy);

    store
        .connection()
        .execute_batch(
            "DROP TRIGGER obs_fts_update;
             DROP TRIGGER obs_exact_update;",
        )
        .unwrap();

    let report = store.doctor().unwrap();
    assert!(!report.healthy);
    let named = report
        .checks
        .iter()
        .find(|check| check.code == "full_text_triggers")
        .expect("the roll call is a check of its own");
    assert!(!named.ok, "{report:?}");
    let detail = named.detail.clone().unwrap_or_default();
    assert!(detail.contains("obs_fts_update"), "{detail:?}");
    assert!(detail.contains("obs_exact_update"), "{detail:?}");

    // The drift, before the repair: the edit reaches the row and not the index.
    store
        .update_observation(
            saved.id,
            crate::memory::model::UpdateObservation {
                content: Some("garrapinada posterior".to_owned()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        store
            .search("garrapinada", SearchOptions::default())
            .unwrap()
            .len(),
        0,
        "with the trigger gone the new word cannot be in the index"
    );

    let restored = store.restore_full_text_triggers().unwrap();
    assert_eq!(restored, vec!["obs_fts_update", "obs_exact_update"]);
    store.rebuild_full_text_indexes().unwrap();
    assert!(store.doctor().unwrap().healthy);
    assert_eq!(
        store
            .search("garrapinada", SearchOptions::default())
            .unwrap()
            .len(),
        1,
        "the rebuild is what recovers the edits made while the trigger was gone"
    );

    assert!(store.restore_full_text_triggers().unwrap().is_empty());
}

/// Every trigger on the roll call has a definition the repair can find.
///
/// The two lists are kept in different languages — the names in Rust, the SQL
/// in the migrations — and a rename on one side would leave `--repair`
/// reporting a trigger restored that it never wrote.
#[test]
fn every_named_trigger_has_its_statement_in_a_migration() {
    for name in crate::store::schema::FULL_TEXT_TRIGGERS {
        let sql = crate::store::schema::full_text_trigger_sql(name)
            .unwrap_or_else(|| panic!("{name} is on the roll call with no definition behind it"));
        assert!(sql.starts_with(&format!("CREATE TRIGGER {name} ")), "{sql}");
        assert!(sql.ends_with("END;"), "{sql}");
    }
}

/// What `doctor` says can be repaired, `--repair` repairs — and it says so.
///
/// The flag exists because a report that can see a broken index and offer
/// nothing is only half a diagnosis. Then the two checks written after it named
/// the remedy in their own sentence and the five it was built for did not, so
/// the person whose hash had drifted was told what to run and the person whose
/// index had gone empty was told a row count and left to work it out.
///
/// Both halves are asserted here, because either alone is a promise with no
/// second end: every failing check names the flag, and running what the flag
/// runs leaves the store healthy.
#[test]
fn every_break_the_report_can_repair_names_the_flag_and_is_repaired_by_it() {
    let temp = TempDir::new().unwrap();
    let config = StoreConfig::new(temp.path().join("remedy.db"));
    let mut store = Store::open(config).unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    store
        .add_observation(observation("s1", "A memory", "with a body"))
        .unwrap();
    store
        .add_prompt(crate::AddPrompt {
            session_id: "s1".to_owned(),
            content: "una pregunta cualquiera".to_owned(),
            project: Some("leteo".to_owned()),
        })
        .unwrap();

    // Every break `--repair` knows how to undo, at once: the three indexes
    // emptied under their own triggers, a hash that stopped describing its
    // memory, and two triggers gone.
    store
        .connection()
        .execute_batch(
            "UPDATE observations SET normalized_hash = 'not the hash of anything';
             INSERT INTO observations_fts(observations_fts) VALUES('delete-all');
             INSERT INTO observations_exact(observations_exact) VALUES('delete-all');
             INSERT INTO prompts_fts(prompts_fts) VALUES('delete-all');
             DROP TRIGGER obs_fts_update;
             DROP TRIGGER prompt_fts_update;",
        )
        .unwrap();

    let report = store.doctor().unwrap();
    assert!(!report.healthy, "the store was broken on purpose");
    let failed: Vec<&crate::memory::model::DoctorCheck> =
        report.checks.iter().filter(|check| !check.ok).collect();
    assert!(failed.len() >= 5, "{:?}", report.checks);
    for check in &failed {
        let detail = check.detail.clone().unwrap_or_default();
        assert!(
            detail.contains("--repair"),
            "{} says what is wrong and not what to do: {detail:?}",
            check.code
        );
    }

    // And what it names is what puts it back, in the order the flag runs it.
    store.restore_full_text_triggers().unwrap();
    store.rebuild_full_text_indexes().unwrap();
    store.recompute_stale_hashes().unwrap();
    let report = store.doctor().unwrap();
    assert!(report.healthy, "{:?}", report.issues);
}

/// Two checks that were only ever seen passing.
///
/// `every_break_the_report_can_repair_names_the_flag_and_is_repaired_by_it`
/// covers the five `--repair` undoes. These two it does not: a foreign key
/// violation is asserted absent on a healthy store and never made to happen,
/// and the full-text integrity check has a test for the branch where it *could
/// not run* and none for the branch where it ran and found corruption. A check
/// nobody has seen fail is a check that may be watching nothing.
///
/// The two sentences are the point as much as the two failures. "Failed its
/// integrity check" and "could not be checked" are different states and calling
/// one the other is how a healthy store gets reported as corrupt.
#[test]
fn the_checks_that_are_only_ever_seen_passing_can_still_go_red() {
    // A row whose session is not there. `foreign_keys` is enforced on every
    // connection Leteo opens, so the violation has to be written past it —
    // which is exactly how one arrives in real life: something else wrote to
    // the file without turning it on.
    {
        let (_temp, mut store) = store();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        store
            .add_observation(observation("s1", "A memory", "with a body"))
            .unwrap();
        assert!(store.doctor().unwrap().healthy, "healthy to begin with");

        store
            .connection()
            .execute_batch(
                "PRAGMA foreign_keys = OFF;
                 INSERT INTO observations (sync_id, session_id, type, title, content, scope)
                 VALUES ('obs-orphan', 'no-such-session', 'decision', 'Orphan', 'body', 'project');
                 PRAGMA foreign_keys = ON;",
            )
            .unwrap();
        let report = store.doctor().unwrap();
        let keys = report
            .checks
            .iter()
            .find(|check| check.code == "foreign_keys")
            .expect("the doctor reports foreign keys");
        assert!(
            !keys.ok,
            "an observation whose session does not exist is a violation: {report:?}"
        );
        assert!(
            keys.detail
                .as_ref()
                .is_some_and(|detail| detail.contains("foreign key")),
            "and it says which kind of fault it is: {:?}",
            keys.detail
        );
    }

    {
        // And an index whose shadow tables have lost a row: corruption that the
        // check runs and finds, rather than one it cannot look at.
        let (_temp, mut store) = store();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        for index in 0..4 {
            store
                .add_observation(observation(
                    "s1",
                    &format!("Memory {index}"),
                    "a body with words in it",
                ))
                .unwrap();
        }
        assert!(store.doctor().unwrap().healthy, "healthy to begin with");

        store
            .connection()
            .execute_batch(
                "DELETE FROM observations_fts_data
                 WHERE id = (SELECT MAX(id) FROM observations_fts_data);",
            )
            .unwrap();
        let report = store.doctor().unwrap();
        let integrity = report
            .checks
            .iter()
            .find(|check| check.code == "observation_fts_integrity")
            .expect("the doctor reports the observation index");
        assert!(
            !integrity.ok,
            "a mangled index is not a healthy one: {report:?}"
        );
        let detail = integrity.detail.clone().unwrap_or_default();
        assert!(
            detail.contains("failed its integrity check"),
            "the reason has to be the one that happened: {detail}"
        );
        assert!(
            !detail.contains("could not be checked"),
            "and not the one that did not: {detail}"
        );
    }
}

/// Two memories under one key is a state the store can reach, and now says so.
///
/// `memory-model.md` §10 states the invariant — one live memory per key, per
/// project, per scope, because the revision lookup finds it by exactly that
/// triple — and names the one operation that can break it. Merging two projects
/// can, since each may have had its own memory under one key, and the merge
/// reports how many it left sharing a name rather than choosing which to keep.
///
/// That report is a number in one reply. Afterwards nothing mentioned it again,
/// and `doctor` called the store healthy. The cost is not theoretical: driven
/// here, the next save under that key revises whichever row the lookup reaches
/// first, and the other can never be revised by its own key again.
///
/// Reached the way it is reached in life — by merging — rather than by writing
/// two rows past the door. A guard that fabricates a state the product cannot
/// produce proves the check works on something nobody will ever have.
#[test]
fn two_memories_under_one_key_is_reported_and_the_second_becomes_unreachable() {
    let (_temp, mut store) = store();
    for project in ["uno", "dos"] {
        store
            .create_session(&format!("s-{project}"), project, "C:/repo")
            .unwrap();
        store
            .add_observation(AddObservation {
                session_id: format!("s-{project}"),
                kind: "decision".to_owned(),
                title: format!("The {project} version"),
                content: format!("what {project} had to say"),
                tool_name: None,
                project: Some(project.to_owned()),
                scope: "project".to_owned(),
                topic_key: Some("shared/key".to_owned()),
                prompt_sync_id: None,
            })
            .unwrap();
    }
    let clean = store.doctor().unwrap();
    assert!(
        clean
            .checks
            .iter()
            .any(|check| check.code == "topic_key_uniqueness" && check.ok),
        "two projects with the same key are not a collision: {clean:?}"
    );

    // Nor is a deleted one beside a live one, which is the other half of what
    // the invariant says: the lookup that revises by key only ever sees the
    // live rows, so a tombstone under the same key collides with nothing.
    store
        .add_observation(AddObservation {
            session_id: "s-uno".to_owned(),
            kind: "decision".to_owned(),
            title: "One that was thrown away".to_owned(),
            content: "and left a tombstone under the key".to_owned(),
            tool_name: None,
            project: Some("uno".to_owned()),
            scope: "project".to_owned(),
            topic_key: Some("buried/key".to_owned()),
            prompt_sync_id: None,
        })
        .unwrap();
    let buried = live_under_key(&store, "buried/key")[0].0;
    store.delete_observation(buried, false).unwrap();
    store
        .add_observation(AddObservation {
            session_id: "s-uno".to_owned(),
            kind: "decision".to_owned(),
            title: "The one that took its place".to_owned(),
            content: "written under the same key afterwards".to_owned(),
            tool_name: None,
            project: Some("uno".to_owned()),
            scope: "project".to_owned(),
            topic_key: Some("buried/key".to_owned()),
            prompt_sync_id: None,
        })
        .unwrap();
    let with_a_tombstone = store.doctor().unwrap();
    assert!(
        with_a_tombstone
            .checks
            .iter()
            .any(|check| check.code == "topic_key_uniqueness" && check.ok),
        "a deleted memory does not share a key with a live one: {with_a_tombstone:?}"
    );

    let merged = store.merge_projects(&["dos".to_owned()], "uno").unwrap();
    assert_eq!(
        merged.topic_key_collisions, 1,
        "the merge says what it left behind"
    );

    let report = store.doctor().unwrap();
    let check = report
        .checks
        .iter()
        .find(|check| check.code == "topic_key_uniqueness")
        .expect("the doctor reports it");
    assert!(!check.ok, "and so does the store afterwards: {report:?}");
    assert!(
        check
            .detail
            .as_ref()
            .is_some_and(|detail| detail.contains("unreachable by that key")),
        "naming what it costs: {:?}",
        check.detail
    );
    assert!(!report.healthy, "which is not a healthy store");

    // What it costs, driven rather than asserted: a save under the shared key
    // revises one of the two and the other is left where it is.
    let before: Vec<(i64, i64)> = live_under_key(&store, "shared/key");
    assert_eq!(before.len(), 2, "{before:?}");
    store
        .add_observation(AddObservation {
            session_id: "s-uno".to_owned(),
            kind: "decision".to_owned(),
            title: "A third version".to_owned(),
            content: "written after the merge".to_owned(),
            tool_name: None,
            project: Some("uno".to_owned()),
            scope: "project".to_owned(),
            topic_key: Some("shared/key".to_owned()),
            prompt_sync_id: None,
        })
        .unwrap();
    let after = live_under_key(&store, "shared/key");
    assert_eq!(after.len(), 2, "still two, and still one key: {after:?}");
    let revised = after
        .iter()
        .filter(|(id, revisions)| before.iter().any(|(was, had)| was == id && revisions > had))
        .count();
    assert_eq!(
        revised, 1,
        "one of them took the save and the other did not: {before:?} then {after:?}"
    );
}

/// The live memories under one key, as (id, revision_count).
fn live_under_key(store: &Store, key: &str) -> Vec<(i64, i64)> {
    let mut statement = store
        .connection()
        .prepare(
            "SELECT id, revision_count FROM observations
              WHERE topic_key = ?1 AND deleted_at IS NULL ORDER BY id",
        )
        .unwrap();
    let rows = statement
        .query_map([key], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap();
    rows.collect::<Result<Vec<_>, _>>().unwrap()
}

/// An import leaves the store as though the rows had been written one by one.
///
/// They are not: the full-text triggers come off first and the indexes are
/// built once at the end, because every insert otherwise tokenises a title and
/// a body through `porter unicode61` three times over. On a real store — 4,013
/// memories, 486 sessions, 1,198 prompts, 326 relations — that is 13.3 seconds
/// against 1.5.
///
/// So what has to be asserted is the *outcome*, which the speed must not have
/// changed: every trigger back, every index level with its rows, and the
/// memories findable by their own words. A store that imported without its
/// triggers restored answers searches with yesterday's words for ever, and
/// nothing about the import would have looked wrong.
#[test]
fn an_import_leaves_the_indexes_and_the_triggers_it_found() {
    let (_temp, mut source) = store();
    source.create_session("s1", "leteo", "C:/repo").unwrap();
    for index in 0..12 {
        source
            .add_observation(observation(
                "s1",
                &format!("Una memoria {index}"),
                &format!("con un cuerpo distinto sobre engranajes {index}"),
            ))
            .unwrap();
    }
    source
        .add_prompt(crate::AddPrompt {
            session_id: "s1".to_owned(),
            content: "una pregunta sobre engranajes".to_owned(),
            project: Some("leteo".to_owned()),
        })
        .unwrap();
    let exported = source.export().unwrap();

    let (_target_temp, mut target) = store();
    let result = target.import(&exported).unwrap();
    assert_eq!(result.observations_imported, 12, "{result:?}");

    // Every trigger the schema names, by name rather than by count.
    let missing = crate::store::schema::missing_full_text_triggers(target.connection());
    assert!(missing.is_empty(), "left without {missing:?}");

    // Every index level with its rows, which is what the rebuild is for.
    let report = target.doctor().unwrap();
    assert!(report.healthy, "{:?}", report.issues);

    // And findable, which is the only assertion that would notice an index
    // rebuilt from nothing at all.
    let found = target
        .search("engranajes", crate::SearchOptions::default())
        .unwrap();
    assert_eq!(found.len(), 10, "the imported memories are searchable");

    // The triggers work afterwards too: an index that was rebuilt and then left
    // untriggered goes stale from the next write, which no count taken now
    // would show.
    target
        .add_observation(observation(
            "s1",
            "Una escrita despues del import",
            "con la palabra zarandaja dentro",
        ))
        .unwrap();
    assert_eq!(
        target
            .search("zarandaja", crate::SearchOptions::default())
            .unwrap()
            .len(),
        1,
        "a write after the import reaches the index"
    );
}

/// An answer the settings file gives that Leteo is reading past is named.
///
/// Each field already falls back to its own default when it cannot be read, and
/// that is deliberate: hooks read this file on every event, so one typo must not
/// take the rest of the file with it, and a half-written file has to be survived
/// rather than reported. What it left was a person with no way to find out.
/// `"context_size": "slimm"` is answered with the default size and not a word,
/// and the sign of it is a context the wrong length three weeks later.
///
/// So the same reading is done once more, out loud, where somebody asks what is
/// wrong. Both spellings of the mistake are named: a value that is not one of
/// the accepted ones, and a key that is not one of the five — `contextsize`
/// without its underscore was read past exactly as quietly.
#[test]
fn a_setting_that_is_read_past_is_named() {
    let temp = TempDir::new().unwrap();
    let store = Store::open(StoreConfig::new(temp.path().join("settings.db"))).unwrap();
    let settings = temp.path().join("settings.json");
    let verdict = |store: &Store| -> crate::memory::model::DoctorCheck {
        store
            .doctor()
            .unwrap()
            .checks
            .into_iter()
            .find(|check| check.code == "settings_readable")
            .expect("the doctor reports it")
    };

    assert!(verdict(&store).ok, "no file is not a broken file");
    std::fs::write(
        &settings,
        r#"{"language":"español","context_size":"slim","voice":"quiet"}"#,
    )
    .unwrap();
    assert!(verdict(&store).ok, "and neither is one that reads");

    std::fs::write(
        &settings,
        r#"{"language":"español","context_size":"slimm"}"#,
    )
    .unwrap();
    let said = verdict(&store);
    assert!(!said.ok, "a size nobody offers is being read past");
    let detail = said.detail.clone().unwrap_or_default();
    assert!(
        detail.contains("context_size") && detail.contains("slimm"),
        "naming the field and what was in it: {detail}"
    );
    assert!(
        !detail.contains("language"),
        "and not the answer beside it, which still counts: {detail}"
    );

    std::fs::write(&settings, r#"{"contextsize":"slim"}"#).unwrap();
    let said = verdict(&store);
    assert!(!said.ok, "a key nobody reads is being read past");
    assert!(
        said.detail
            .as_ref()
            .is_some_and(|detail| detail.contains("contextsize")),
        "{:?}",
        said.detail
    );

    std::fs::write(
        &settings,
        r#"{"voice":"loud","interface":"klingon","language":"español"}"#,
    )
    .unwrap();
    let detail = verdict(&store).detail.unwrap_or_default();
    assert!(
        detail.contains("voice") && detail.contains("interface"),
        "both of them: {detail}"
    );

    // And a file that is not JSON is one entry rather than five, because there
    // is nothing in it to blame a field for.
    std::fs::write(&settings, "{ not json at all").unwrap();
    let detail = verdict(&store).detail.unwrap_or_default();
    assert!(detail.contains("not JSON"), "{detail}");
    assert!(!detail.contains("context_size"), "{detail}");

    // The reading itself is unchanged: what is named here is still read past
    // rather than refused, which is what a hook depends on.
    std::fs::write(&settings, r#"{"context_size":"slimm"}"#).unwrap();
    assert_eq!(
        crate::settings::load(temp.path()).context_size(),
        crate::settings::ContextSize::default(),
        "the default is used, exactly as before"
    );
}

/// A memory filed under a word no filter can ask for is found and named.
///
/// The category is a search filter. `mem_save` folds the close synonyms and
/// keeps anything else verbatim — a word Leteo does not know is still what
/// somebody meant — and the save door says so at the moment it happens. Nothing
/// ever said it about the memories already in, so a store that collected them
/// before that hint existed had no way to find out.
///
/// Measured on a real store of 4,121: thirty-eight, under five words.
#[test]
fn a_type_no_filtered_search_can_name_is_reported_with_the_words() {
    let temp = TempDir::new().unwrap();
    let config = StoreConfig::new(temp.path().join("kinds.db"));
    let mut store = Store::open(config).unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();

    // Every kind an agent is taught, plus the one Leteo writes itself, and the
    // check has nothing to say about any of them.
    for kind in crate::memory::rules::KINDS {
        let mut add = observation("s1", &format!("A {kind}"), "a body");
        add.kind = (*kind).to_owned();
        store.add_observation(add).unwrap();
    }
    let mut summary = observation("s1", "What the session was for", "a body");
    summary.kind = crate::memory::model::SESSION_SUMMARY.to_owned();
    store.add_observation(summary).unwrap();
    let report = store.doctor().unwrap();
    assert!(
        report
            .checks
            .iter()
            .any(|check| check.code == "observation_type_searchable" && check.ok),
        "the eight and the summary are all filterable: {report:?}"
    );

    // Sized from the constant, and one word over it, or the cap this names is
    // one the fixture never reaches.
    let words = crate::store::diagnostics::UNSEARCHABLE_KIND_EXAMPLES + 1;
    for index in 0..words {
        // Descending counts, so the commonest word is named first and the
        // rarest is the one that falls off the end.
        for copy in 0..(words - index) {
            let mut add = observation("s1", &format!("Filed as w{index} no {copy}"), "a body");
            add.kind = format!("w{index}");
            store.add_observation(add).unwrap();
        }
    }
    let expected: i64 = (1..=words as i64).sum();

    let report = store.doctor().unwrap();
    assert!(!report.healthy);
    let check = report
        .checks
        .iter()
        .find(|check| check.code == "observation_type_searchable")
        .expect("the check is reported");
    assert!(!check.ok);
    let detail = check.detail.as_deref().unwrap_or_default();
    assert!(
        detail.starts_with(&format!("{expected} memories")),
        "the total counts every memory, not every word: {detail}"
    );
    assert!(
        detail.contains("w0 (9)") && detail.contains("w7 (2)"),
        "the words are named with their counts, commonest first: {detail}"
    );
    assert!(
        !detail.contains("w8") && detail.contains("and 1 more word(s)"),
        "the published cap is the cap that is applied: {detail}"
    );
    // And the remedy names the eight it is asking for, from the one list.
    for kind in crate::memory::rules::KINDS {
        assert!(detail.contains(kind), "{kind} is missing from: {detail}");
    }
}

/// The project list is answered per project, not by reading every memory.
///
/// Grouping the whole table by project and taking `MAX(created_at)` per group
/// returns exactly the same names in exactly the same order as seventeen index
/// seeks do, which is why this sat here costing nine tenths of `stats` — 7.7 ms
/// of 9.4 on a real store of 4,121 memories — with every result-based test
/// green. So this one reads the plan.
///
/// What the plan must not do is walk the table. `idx_obs_project` holds the
/// project and nothing else, so a row of it costs a lookup for `deleted_at` and
/// `created_at`; `idx_obs_project_order` is `(project, datetime(created_at)
/// DESC, id DESC)` and has each project's newest memory first, which is the
/// whole answer in one seek.
#[test]
fn the_project_list_seeks_per_project_rather_than_reading_every_memory() {
    let temp = TempDir::new().unwrap();
    let config = StoreConfig::new(temp.path().join("projects.db"));
    let mut store = Store::open(config).unwrap();

    // Enough rows that the planner has something to prefer, and enough per
    // project that walking them is visibly the wrong shape.
    for project in ["alpha", "beta", "gamma", "delta"] {
        store
            .create_session(&format!("s-{project}"), project, "C:/repo")
            .unwrap();
        for index in 0..60 {
            let mut add = observation(
                &format!("s-{project}"),
                &format!("{project} memory {index}"),
                "a body",
            );
            add.project = Some(project.to_owned());
            store.add_observation(add).unwrap();
        }
    }

    let plan: Vec<String> = store
        .connection()
        .prepare(&format!(
            "EXPLAIN QUERY PLAN {}",
            crate::store::diagnostics::PROJECTS_BY_RECENCY
        ))
        .unwrap()
        .query_map([], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let joined = plan.join("\n");
    assert!(
        joined.contains("COVERING INDEX idx_obs_project"),
        "the names come out of the index without touching the table:\n{joined}"
    );
    assert!(
        joined.contains("idx_obs_project_order"),
        "each project's newest memory is one seek into the ordered index:\n{joined}"
    );
    assert!(
        !joined.contains("SCAN observations"),
        "nothing here reads every memory:\n{joined}"
    );
}

/// And it says the same thing the old shape said, in the same order.
///
/// Newest first; a project every one of whose memories is deleted is not a
/// project this lists; and two projects whose newest memory lands on the same
/// timestamp come out by name rather than by whatever the scan happened to
/// produce.
#[test]
fn the_project_list_is_newest_first_and_leaves_out_the_ones_with_nothing_left() {
    let temp = TempDir::new().unwrap();
    let config = StoreConfig::new(temp.path().join("order.db"));
    let mut store = Store::open(config).unwrap();
    let write = |store: &mut Store, project: &str, title: &str, created: &str| -> i64 {
        store.create_session(project, project, "C:/repo").ok();
        let mut add = observation(project, title, "a body");
        add.project = Some(project.to_owned());
        let id = store.add_observation(add).unwrap().observation.id;
        store
            .connection()
            .execute(
                "UPDATE observations SET created_at = ?1 WHERE id = ?2",
                rusqlite::params![created, id],
            )
            .unwrap();
        id
    };

    write(&mut store, "oldest", "First", "2020-01-01 00:00:00");
    write(&mut store, "newest", "Last", "2030-01-01 00:00:00");
    // Two projects sharing a timestamp, so the tie-break is exercised rather
    // than assumed.
    write(&mut store, "zebra", "Tied", "2025-06-01 00:00:00");
    write(&mut store, "aardvark", "Tied too", "2025-06-01 00:00:00");
    let doomed = write(&mut store, "emptied", "Deleted", "2029-01-01 00:00:00");
    store.delete_observation(doomed, false).unwrap();
    // One that is still here, whose newest memory was deleted: it belongs where
    // its newest *live* memory puts it, which is last. Counting the deleted one
    // would put it second, and without this pair the clause that excludes it
    // from the maximum could be dropped with every test still green.
    write(&mut store, "pruned", "What is left", "2019-01-01 00:00:00");
    let recent = write(&mut store, "pruned", "Deleted since", "2029-06-01 00:00:00");
    store.delete_observation(recent, false).unwrap();

    assert_eq!(
        store.stats().unwrap().projects,
        vec![
            "newest".to_owned(),
            "aardvark".to_owned(),
            "zebra".to_owned(),
            "oldest".to_owned(),
            "pruned".to_owned()
        ]
    );

    // The tie above comes out by name, and on this build it would even without
    // being asked to: the distinct projects are read out of an index, so they
    // arrive alphabetically and the temporary sort keeps them that way. That is
    // the plan being helpful rather than the query being right, and a plan is
    // not a promise - so the clause is asserted where it is written.
    assert!(
        crate::store::diagnostics::PROJECTS_BY_RECENCY
            .trim_end()
            .ends_with("p.project"),
        "the last ordering key is the name, or two projects sharing a timestamp come out in \
         whatever order the plan happened to produce"
    );
}
