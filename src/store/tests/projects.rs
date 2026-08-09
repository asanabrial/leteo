//! Whole projects: naming, merging, deleting.

use super::*;

#[test]
fn a_page_is_a_window_onto_a_list_that_says_how_long_it_is() {
    // The failure this guards is a screen that shows the first hundred rows
    // of three thousand and offers no way to the rest, with a heading that
    // reports the truncation as the count.
    let (_temp, mut store) = store();
    store.create_session("s1", "Leteo", "C:/repo").unwrap();
    for n in 0..25 {
        store
            .add_observation(observation("s1", &format!("Memory {n:02}"), "body"))
            .unwrap();
    }

    let first = store.paged_observations("", &[], 0, 10).unwrap();
    assert_eq!(first.rows.len(), 10);
    assert_eq!(first.total, 25, "the total is of the list, not of the page");
    assert_eq!(first.rows[0].title, "Memory 24", "newest first");

    let third = store.paged_observations("", &[], 20, 10).unwrap();
    assert_eq!(third.rows.len(), 5, "the last page is short");
    assert_eq!(third.rows[0].title, "Memory 04");

    // Past the end is empty rather than an error, which is what lets the
    // screen step back to the last page instead of reporting a failure.
    let past = store.paged_observations("", &[], 100, 10).unwrap();
    assert!(past.rows.is_empty());
    assert_eq!(past.total, 25, "and it still says how long the list is");

    // A page of a session reads the other way round, and pages the same.
    let entries = store.paged_session_observations("s1", 20, 10).unwrap();
    assert_eq!(entries.total, 25);
    assert_eq!(entries.rows[0].title, "Memory 20", "oldest first here");
}

#[test]
fn a_doctor_project_scope_reports_that_projects_counts() {
    let (_temp, mut store) = store();
    store.create_session("s1", "Leteo", "C:/repo").unwrap();
    store
        .add_observation(observation("s1", "scoped", "body"))
        .unwrap();

    let (_, stats) = store.doctor_scoped(None, Some("Leteo")).unwrap();
    let stats = stats.expect("an existing project reports its counts");
    assert_eq!(stats.name, "leteo");
    assert_eq!(stats.observation_count, 1);

    assert!(
        store
            .doctor_scoped(None, Some("no-such-project"))
            .unwrap_err()
            .to_string()
            .contains("unknown project")
    );
}

#[test]
fn a_set_of_projects_narrows_the_recent_lists() {
    let (_temp, mut store) = store();
    for project in ["leteo", "atlas", "quarry"] {
        store.create_session(project, project, "C:/repo").unwrap();
        let mut input = observation(project, "Something", "happened");
        input.project = Some(project.to_owned());
        store.add_observation(input).unwrap();
        store
            .add_prompt(AddPrompt {
                session_id: project.to_owned(),
                content: format!("what about {project}?"),
                project: Some(project.to_owned()),
            })
            .unwrap();
    }

    let named = |observations: &[Observation]| {
        let mut projects: Vec<String> = observations
            .iter()
            .map(|o| o.project.clone().unwrap_or_default())
            .collect();
        projects.sort();
        projects
    };

    // Empty means every project, not none. An `IN ()` clause is legal SQL
    // that matches nothing, so getting this backwards shows an empty screen
    // and looks like an empty store.
    assert_eq!(
        named(&store.paged_observations("", &[], 0, 10).unwrap().rows).len(),
        3
    );

    let two = ["leteo".to_owned(), "quarry".to_owned()];
    assert_eq!(
        named(&store.paged_observations("", &two, 0, 10).unwrap().rows),
        vec!["leteo".to_owned(), "quarry".to_owned()]
    );
    assert_eq!(store.paged_sessions("", &two, 0, 10).unwrap().total, 2);
    assert_eq!(store.paged_prompts("", &two, 0, 10).unwrap().total, 2);

    // A name that is not a project of this store simply matches nothing,
    // rather than being treated as no filter at all.
    let missing = ["nowhere".to_owned()];
    assert!(
        store
            .paged_observations("", &missing, 0, 10)
            .unwrap()
            .rows
            .is_empty()
    );

    // And the names are bound, not pasted: a project named like an injection
    // is a legal project name.
    let hostile = ["leteo') OR 1=1 --".to_owned()];
    assert!(
        store
            .paged_observations("", &hostile, 0, 10)
            .unwrap()
            .rows
            .is_empty()
    );
}

#[test]
fn project_export_and_json_import_are_scoped_idempotent_and_private_safe() {
    let (_source_temp, mut source) = store();
    source
        .create_session("shared", "project-b", "C:/shared")
        .unwrap();
    let mut owned = observation(
        "shared",
        "Owned <private>title</private>",
        "project-a export body",
    );
    owned.project = Some("project-a".to_owned());
    source.add_observation(owned).unwrap();
    source
        .add_prompt(AddPrompt {
            session_id: "shared".to_owned(),
            content: "project-a prompt".to_owned(),
            project: Some("project-a".to_owned()),
        })
        .unwrap();
    source
        .add_prompt(AddPrompt {
            session_id: "shared".to_owned(),
            content: "project-b prompt".to_owned(),
            project: Some("project-b".to_owned()),
        })
        .unwrap();

    let exported = source.export_project("PROJECT-A").unwrap();
    assert_eq!(exported.sessions.len(), 1);
    assert_eq!(exported.sessions[0].id, "shared");
    assert_eq!(exported.observations.len(), 1);
    assert_eq!(exported.prompts.len(), 1);
    let json = source.export_json(Some("project-a")).unwrap();
    assert_eq!(
        serde_json::from_str::<ExportData>(&json)
            .unwrap()
            .observations,
        exported.observations
    );

    let (_destination_temp, mut destination) = store();
    let first = destination.import_json(&json).unwrap();
    assert_eq!(first.sessions_imported, 1);
    assert_eq!(first.observations_imported, 1);
    assert_eq!(first.prompts_imported, 1);
    let second = destination.import_json(&json).unwrap();
    assert_eq!(second, ImportResult::default());
    assert_eq!(destination.stats().unwrap().total_observations, 1);
    assert_eq!(
        destination
            .connection
            .query_row("SELECT COUNT(*) FROM sync_mutations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );

    let mut unsupported = exported;
    unsupported.version = "999.0.0".to_owned();
    assert!(destination.import(&unsupported).is_err());
    assert!(source.export_project(" ").is_err());
}

#[test]
fn merge_projects_updates_all_entities_and_journals_active_rows() {
    let (_temp, mut store) = store();
    for project in ["old-project", "canonical-project"] {
        store.enroll_project(project).unwrap();
    }
    store
        .create_session("old-session", "Old--Project", "C:/old")
        .unwrap();
    let mut old_observation = observation("old-session", "Merge", "merge body");
    old_observation.project = Some("old-project".to_owned());
    let saved = store.add_observation(old_observation).unwrap().observation;
    let prompt = store
        .add_prompt(AddPrompt {
            session_id: "old-session".to_owned(),
            content: "merge prompt".to_owned(),
            project: Some("old-project".to_owned()),
        })
        .unwrap();
    store
        .connection
        .execute_batch(
            "UPDATE observations SET project = 'old_project';
                 UPDATE sessions SET project = 'old_project';
                 UPDATE prompts SET project = 'old_project';",
        )
        .unwrap();
    let before: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM sync_mutations", [], |row| row.get(0))
        .unwrap();

    let result = store
        .merge_projects(
            &["OLD--PROJECT".to_owned(), "old-project".to_owned()],
            "Canonical--Project",
        )
        .unwrap();
    assert_eq!(result.canonical, "canonical-project");
    assert_eq!(result.sources_merged, ["old-project"]);
    assert_eq!(result.observations_updated, 1);
    assert_eq!(result.sessions_updated, 1);
    assert_eq!(result.prompts_updated, 1);
    assert_eq!(
        store.get_observation(saved.id).unwrap().project.as_deref(),
        Some("canonical-project")
    );
    assert_eq!(
        store.get_session("old-session").unwrap().project,
        "canonical-project"
    );
    assert_eq!(
        store
            .recent_prompts(Some("canonical-project"), None)
            .unwrap()[0]
            .id,
        prompt.id
    );
    // What the journal holds afterwards, rather than how many rows it gained.
    // The merge queues the three moved rows under the canonical name and drops
    // what was still waiting under the old one — every row those described has
    // just been queued again, in its current state — so the net count is zero
    // and says nothing. `before` is kept because it is what makes that
    // sentence checkable.
    let journalled = |project: &str| -> i64 {
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM sync_mutations WHERE project = ?1",
                params![project],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(before, 3, "the old project had queued its own three rows");
    assert_eq!(journalled("canonical-project"), 3);
    assert_eq!(
        journalled("old-project"),
        0,
        "and nothing is still waiting under a name that now holds nothing"
    );
    assert!(
        !store
            .enrolled_projects()
            .unwrap()
            .contains(&"old-project".to_owned()),
        "an emptied project stops being replicated"
    );
    assert!(store.merge_projects(&[], "").is_err());
}

#[test]
fn pruning_removes_empty_projects_and_refuses_populated_ones() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    store.create_session("s2", "empty", "C:/empty").unwrap();
    store
        .add_observation(observation("s1", "Kept", "kept body"))
        .unwrap();
    store
        .add_prompt(AddPrompt {
            session_id: "s2".to_owned(),
            content: "empty prompt".to_owned(),
            project: Some("empty".to_owned()),
        })
        .unwrap();

    let result = store.prune_project("Empty").unwrap();
    assert_eq!(result.project, "empty");
    assert_eq!(result.sessions_deleted, 1);
    assert_eq!(result.prompts_deleted, 1);
    assert!(store.get_session("s2").is_err());
    assert!(store.prune_project("leteo").is_err());
    assert!(store.prune_project("  ").is_err());
}

#[test]
fn scanning_a_project_reports_candidates_and_only_inserts_when_applied() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    // Filler, and load-bearing: a candidate has to score past the floor the
    // scan applies, and a bm25 term weight grows with how rare the word is
    // across the store. Two memories score near zero however alike they are.
    for index in 0..40 {
        store
            .add_observation(observation(
                "s1",
                &format!("Unrelated note {index} on deployment windows"),
                &format!("Body {index}: staged rollout, canaries and a rollback plan."),
            ))
            .unwrap();
    }
    store
        .add_observation(observation(
            "s1",
            "Connection pooling limits",
            "the pool caps concurrent writers",
        ))
        .unwrap();
    store
        .add_observation(observation(
            "s1",
            "Connection pooling limits revisited",
            "the pool now caps concurrent readers",
        ))
        .unwrap();

    let dry_run = store
        .scan_project(ScanOptions {
            project: "Leteo".to_owned(),
            ..ScanOptions::default()
        })
        .unwrap();
    assert!(dry_run.dry_run);
    assert_eq!(dry_run.inspected, 42);
    assert!(dry_run.candidates_found >= 2);
    // What the apply would write, not zero because nothing was written.
    //
    // This asserted zero, which is what a dry run used to report because it
    // never looked: it skipped the loop that asks whether a pair is already
    // related, so both that count and this one came back empty whatever the
    // store held. The claim the test is named for is the one below — nothing
    // reaches the table — and that is unchanged.
    assert!(
        dry_run.relations_inserted >= 1,
        "a preview says what applying would do: {dry_run:?}"
    );
    assert!(
        store
            .list_relations(ListRelationsOptions::default())
            .unwrap()
            .is_empty(),
        "and writes nothing while saying it"
    );

    let applied = store
        .scan_project(ScanOptions {
            project: "leteo".to_owned(),
            apply: true,
            ..ScanOptions::default()
        })
        .unwrap();
    assert!(!applied.dry_run);
    assert_eq!(applied.relations_inserted, 1);
    assert_eq!(applied.already_related, 1);
    // The preview and the apply say the same thing, which is the only reason
    // to have a preview. The finder proposes a pair from both ends, so one of
    // the two candidates is the same two memories again — the apply used to
    // notice that only because its own first insert made the second one
    // already-related, and the dry run, writing nothing, counted both.
    assert_eq!(
        dry_run.relations_inserted, applied.relations_inserted,
        "preview {dry_run:?} against apply {applied:?}"
    );
    assert_eq!(dry_run.already_related, applied.already_related);
    let relations = store
        .list_relations(ListRelationsOptions::default())
        .unwrap();
    assert_eq!(relations.len(), 1);
    assert_eq!(relations[0].judgment_status, JUDGMENT_STATUS_PENDING);

    let repeated = store
        .scan_project(ScanOptions {
            project: "leteo".to_owned(),
            apply: true,
            ..ScanOptions::default()
        })
        .unwrap();
    assert_eq!(repeated.relations_inserted, 0);
    assert_eq!(repeated.already_related, 2);
}

#[test]
fn nothing_is_journalled_until_a_project_is_enrolled_and_then_it_catches_up() {
    // Every journal row carries a full JSON copy of what it describes, and
    // nothing removes an unacknowledged one — pruning only reaches rows the
    // cloud confirmed, and a cloud that was never configured confirms
    // nothing. A real store held 9 525 of them against 3 408 memories:
    // 14.5 MB of a 42 MB database, written for a reader that did not exist.
    let (_temp, mut store) = bare_store();
    store.create_session("s1", "Leteo", "C:/repo").unwrap();
    store
        .add_observation(observation("s1", "Before enrolling", "body"))
        .unwrap();
    store
        .add_prompt(AddPrompt {
            session_id: "s1".to_owned(),
            content: "why?".to_owned(),
            project: Some("Leteo".to_owned()),
        })
        .unwrap();
    let journalled = |store: &Store| -> i64 {
        store
            .connection
            .query_row("SELECT COUNT(*) FROM sync_mutations", [], |row| row.get(0))
            .unwrap()
    };
    assert_eq!(journalled(&store), 0, "nobody replicates this project yet");

    // Enrolling has to catch up, or the trade above would cost a project
    // its history: it would replicate whatever came next and silently never
    // send what it already held.
    assert!(store.enroll_project("leteo").unwrap());
    let after = journalled(&store);
    assert!(after >= 3, "session, memory and prompt all queued: {after}");
    for entity in ["observation", "session", "prompt"] {
        let count: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM sync_mutations WHERE entity = ?1 AND op = 'upsert'",
                [entity],
                |row| row.get(0),
            )
            .unwrap();
        assert!(count >= 1, "{entity} was not caught up");
    }

    // Enrolling again is not a second copy of everything.
    assert!(!store.enroll_project("leteo").unwrap());
    assert_eq!(journalled(&store), after);

    // And from here on it journals as it always did.
    store
        .add_observation(observation("s1", "After enrolling", "body"))
        .unwrap();
    assert!(journalled(&store) > after);
}

#[test]
fn semantic_compare_is_idempotent_journaled_and_rejects_cross_project() {
    let (_temp, mut store) = store();
    store.enroll_project("other").unwrap();
    store.create_session("same", "leteo", "C:/same").unwrap();
    store.create_session("other", "other", "C:/other").unwrap();
    let first = store
        .add_observation(observation("same", "Auth decision A", "first"))
        .unwrap()
        .observation;
    let second = store
        .add_observation(observation("same", "Auth decision B", "second"))
        .unwrap()
        .observation;
    let unrelated = store
        .add_observation(observation("same", "Database decision", "third"))
        .unwrap()
        .observation;
    let mut other_input = observation("other", "Other project auth", "other");
    other_input.project = Some("other".to_owned());
    let other = store.add_observation(other_input).unwrap().observation;

    let first_sync_id = store
        .judge_by_semantic(JudgeBySemanticParams {
            source_id: first.sync_id.clone(),
            target_id: second.sync_id.clone(),
            relation: RELATION_RELATED.to_owned(),
            confidence: Some(0.7),
            reasoning: Some("initial verdict".to_owned()),
            model: Some("model-v1".to_owned()),
        })
        .unwrap();
    let second_sync_id = store
        .judge_by_semantic(JudgeBySemanticParams {
            source_id: second.sync_id.clone(),
            target_id: first.sync_id.clone(),
            relation: RELATION_COMPATIBLE.to_owned(),
            confidence: Some(0.95),
            reasoning: Some("updated verdict".to_owned()),
            model: Some("model-v2".to_owned()),
        })
        .unwrap();
    assert_eq!(first_sync_id, second_sync_id);
    let relation = store.get_relation(&first_sync_id).unwrap();
    assert_eq!(relation.relation, RELATION_COMPATIBLE);
    assert_eq!(relation.marked_by_actor.as_deref(), Some("leteo"));
    assert_eq!(relation.marked_by_kind.as_deref(), Some("system"));
    assert_eq!(relation.marked_by_model.as_deref(), Some("model-v2"));
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM memory_relations
                     WHERE (source_id = ?1 AND target_id = ?2)
                        OR (source_id = ?2 AND target_id = ?1)",
                params![first.sync_id, second.sync_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    let journal_before_noop: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM sync_mutations WHERE entity = 'relation'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let noop = store
        .judge_by_semantic(JudgeBySemanticParams {
            source_id: first.sync_id.clone(),
            target_id: unrelated.sync_id.clone(),
            relation: RELATION_NOT_CONFLICT.to_owned(),
            confidence: Some(1.0),
            reasoning: Some("unrelated".to_owned()),
            model: None,
        })
        .unwrap();
    assert!(noop.is_empty());
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM memory_relations
                     WHERE (source_id = ?1 AND target_id = ?2)
                        OR (source_id = ?2 AND target_id = ?1)",
                params![first.sync_id, unrelated.sync_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM sync_mutations WHERE entity = 'relation'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        journal_before_noop
    );
    let payload: serde_json::Value = serde_json::from_str(
        &store
            .connection
            .query_row(
                "SELECT payload FROM sync_mutations
                     WHERE entity = 'relation' ORDER BY seq DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(payload["project"], "leteo");
    assert_eq!(payload["judgment_status"], JUDGMENT_STATUS_JUDGED);

    assert!(matches!(
        store.judge_by_semantic(JudgeBySemanticParams {
            source_id: first.sync_id.clone(),
            target_id: other.sync_id.clone(),
            relation: RELATION_CONFLICTS_WITH.to_owned(),
            confidence: Some(0.9),
            reasoning: Some("cross project".to_owned()),
            model: None,
        }),
        Err(StoreError::CrossProjectRelation { .. })
    ));
    let cross_id = normalize::sync_id("rel");
    store
        .save_relation(SaveRelationParams {
            sync_id: cross_id.clone(),
            source_id: first.sync_id,
            target_id: other.sync_id,
        })
        .unwrap();
    assert!(matches!(
        store.judge_relation(JudgeRelationParams {
            judgment_id: cross_id.clone(),
            relation: RELATION_SUPERSEDES.to_owned(),
            marked_by_actor: "agent".to_owned(),
            marked_by_kind: "agent".to_owned(),
            ..JudgeRelationParams::default()
        }),
        Err(StoreError::CrossProjectRelation { .. })
    ));
    assert_eq!(
        store.get_relation(&cross_id).unwrap().judgment_status,
        JUDGMENT_STATUS_PENDING
    );
}

#[test]
fn a_destructive_operation_refuses_a_project_that_names_nothing() {
    // These three are the only calls in Leteo that remove memories on purpose,
    // and each normalises its argument before using it. An empty name survives
    // normalisation as an empty string, and the queries beneath match on
    // `ifnull(project, '') = ?1` — so an unguarded empty name does not match
    // *nothing*, it matches **every row that has no project**. Deleting or
    // pruning by mistake is not the risk; deleting exactly the rows nobody
    // bothered to file is.
    //
    // All three refusals existed. None was tested: removing the one in
    // `delete_project` left the whole suite green.
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    store
        .add_observation(observation("s1", "A memory with a project", "body"))
        .unwrap();
    // A row that carries no project at all, which is what an empty name reaches.
    store
        .connection
        .execute("UPDATE observations SET project = NULL WHERE id = 1", [])
        .unwrap();

    for empty in ["", "   ", "\t"] {
        assert!(
            store.delete_project(empty, true).is_err(),
            "delete accepted {empty:?}"
        );
        assert!(
            store.prune_project(empty).is_err(),
            "prune accepted {empty:?}"
        );
        assert!(
            store.merge_projects(&["leteo".to_owned()], empty).is_err(),
            "merge accepted {empty:?} as the canonical name"
        );
    }

    // And the unfiled memory is still there.
    assert_eq!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM observations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn merging_a_project_into_itself_moves_nothing_and_says_so() {
    // Asking to merge `leteo` into `leteo` is not an error — it is what a
    // consolidation proposes when one of the candidates is already the
    // canonical name, and refusing it would make the caller filter its own
    // list. It has to be a no-op that reports one.
    //
    // Two mechanisms enforce it and only one is load-bearing: the loop skips
    // a source equal to the canonical, and `project_merge_variants` already
    // drops every candidate that normalises to the canonical — so with the
    // skip removed the variant set is empty, nothing is collected, and nothing
    // moves. A mutation deleting the skip survives this test, and the reason
    // is redundancy rather than a hole. Worth knowing instead of rediscovering.
    //
    // What is asserted is therefore the behaviour, so it holds whichever of
    // the two is doing the work: no rows counted as moved, and no sync
    // mutation journalled for a change that is not one.
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    for title in ["First", "Second", "Third"] {
        store
            .add_observation(observation("s1", title, "body"))
            .unwrap();
    }
    let journalled = |store: &Store| {
        store
            .connection
            .query_row("SELECT COUNT(*) FROM sync_mutations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
    };
    let before = journalled(&store);

    let result = store
        .merge_projects(&["leteo".to_owned(), "  LETEO  ".to_owned()], "leteo")
        .unwrap();

    assert_eq!(result.canonical, "leteo");
    assert!(
        result.sources_merged.is_empty(),
        "nothing was merged: {:?}",
        result.sources_merged
    );
    assert_eq!(
        (
            result.observations_updated,
            result.sessions_updated,
            result.prompts_updated
        ),
        (0, 0, 0),
        "a memory that did not move must not be counted as one"
    );
    assert_eq!(
        journalled(&store),
        before,
        "a rewrite onto the same project must not reach a peer as a change"
    );
}

/// Replication follows the memories when they move.
///
/// Nothing is journalled for a project nobody replicates, so merging an
/// enrolled project into a name that is not enrolled queued every moved row
/// into nowhere: the memories arrived under the new name, the peers were never
/// told again, and the enrolment went on naming the old project, which by then
/// held nothing at all. Replication stopped and nothing said so.
#[test]
fn merging_into_an_unenrolled_name_does_not_quietly_stop_replicating() {
    let (_temp, mut store) = store();
    store.enroll_project("old-project").unwrap();
    store
        .create_session("old-session", "old-project", "C:/old")
        .unwrap();
    let mut memory = observation("old-session", "Worth sending", "and worth keeping");
    memory.project = Some("old-project".to_owned());
    store.add_observation(memory).unwrap();

    // `canonical-project` is not enrolled, which is the ordinary case: a name
    // chosen at the moment of merging has never replicated anything.
    let result = store
        .merge_projects(&["old-project".to_owned()], "canonical-project")
        .unwrap();
    assert!(
        result.enrolment_moved,
        "the answer has to say that what leaves this machine changed"
    );

    let enrolled = store.enrolled_projects().unwrap();
    assert!(
        enrolled.contains(&"canonical-project".to_owned()),
        "the memories are still replicated: {enrolled:?}"
    );
    assert!(
        !enrolled.contains(&"old-project".to_owned()),
        "and the name that holds nothing is not: {enrolled:?}"
    );
    let waiting: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM sync_mutations WHERE project = 'canonical-project'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        waiting > 0,
        "and the moved rows are queued under the name they now have"
    );
}

/// Enrolling a project does not teach a peer about a memory it will never get.
///
/// The backfill queues the graph as well as the memories, and its own comment
/// says what it queues: relations pointing at two live memories *of this
/// project*. The condition only asked about the source. So a relation whose
/// target had moved to another project went out under this one, and the far
/// end — which counts the two memories a relation names before applying it —
/// files it in the deferred table, where it waits for a memory that is never
/// coming, on every retry, for ever.
///
/// Nothing strange is needed to get there: a pair is proposed inside one
/// project, judged there, and one of its memories is moved afterwards.
#[test]
fn a_relation_whose_other_half_left_the_project_is_not_queued_with_it() {
    let (_temp, mut store) = store();
    store.create_session("a", "proyecto-a", "C:/a").unwrap();
    let mut first = observation("a", "The first one", "a memory of project a");
    first.project = Some("proyecto-a".to_owned());
    let first = store.add_observation(first).unwrap().observation;
    let mut second = observation("a", "The second one", "another memory of project a");
    second.project = Some("proyecto-a".to_owned());
    let second = store.add_observation(second).unwrap().observation;
    let mut stays = observation("a", "The third one", "and one that does not move");
    stays.project = Some("proyecto-a".to_owned());
    let stays = store.add_observation(stays).unwrap().observation;

    // Two verdicts inside the project, which is the only place they can be made.
    for target in [&second, &stays] {
        store
            .judge_by_semantic(JudgeBySemanticParams {
                source_id: first.sync_id.clone(),
                target_id: target.sync_id.clone(),
                relation: "related".to_owned(),
                confidence: None,
                reasoning: None,
                model: None,
            })
            .unwrap();
    }
    // And then one of the four memories moves out.
    store.enroll_project("proyecto-b").unwrap();
    store
        .update_observation(
            second.id,
            UpdateObservation {
                project: Some("proyecto-b".to_owned()),
                ..UpdateObservation::default()
            },
        )
        .unwrap();

    store.enroll_project("proyecto-a").unwrap();
    let queued: Vec<String> = store
        .connection
        .prepare(
            "SELECT payload FROM sync_mutations
             WHERE entity = 'relation' AND project = 'proyecto-a'",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        queued.len(),
        1,
        "only the pair whose two memories are both here: {queued:?}"
    );
    assert!(
        queued[0].contains(&stays.sync_id),
        "and it is the one that stayed: {}",
        queued[0]
    );
    assert!(
        !queued[0].contains(&second.sync_id),
        "the memory that left is not named to this project's peer"
    );
}

/// An export is this store written down, field for field.
///
/// The existing round trip checks how many rows came back, which is what let
/// pinning disappear: `#[serde(skip)]` kept it out of the wire *and* out of an
/// export, so `leteo export` followed by `leteo import` came back with every
/// pin lost, silently, while the import statement had always had a column ready
/// for it. Counting rows cannot see that. Nor can it see a column added to the
/// table tomorrow and left out of the import's own list of column names, which
/// is the same failure with a different field.
///
/// So this populates every field a memory can carry, sends it through the JSON,
/// and compares the two memories whole. The list of key names is written out
/// deliberately: it is a change-detector, not a second copy of the model, and a
/// field added to `Observation` should make somebody decide here whether it
/// travels.
#[test]
fn a_round_trip_brings_back_every_field_a_memory_carries() {
    let (_source_temp, mut source) = store();
    source.create_session("s1", "leteo", "C:/repo").unwrap();
    source
        .add_prompt(AddPrompt {
            session_id: "s1".to_owned(),
            content: "la pregunta que motivo la memoria".to_owned(),
            project: Some("leteo".to_owned()),
        })
        .unwrap();
    let asked = source.recent_prompts(Some("leteo"), Some(1)).unwrap()[0]
        .sync_id
        .clone();

    let mut input = observation("s1", "Una decision", "el cuerpo de la decision");
    input.kind = "decision".to_owned();
    input.tool_name = Some("una-herramienta".to_owned());
    input.topic_key = Some("decision/exportada".to_owned());
    input.scope = "personal".to_owned();
    input.prompt_sync_id = Some(asked.clone());
    let saved = source.add_observation(input.clone()).unwrap().observation;
    // Revised and duplicated, so both counters are past their defaults, and
    // pinned and reviewed so `pinned` and `review_after` are set.
    source.add_observation(input.clone()).unwrap();
    source.add_observation(input).unwrap();
    source.pin_observation(saved.id).unwrap();
    source.mark_reviewed(saved.id).unwrap();
    // And one deleted memory, because an export carries those too and a backup
    // that forgets a deletion resurrects it on the next import.
    let removed = source
        .add_observation(observation("s1", "Borrada", "y exportada igual"))
        .unwrap()
        .observation;
    source.delete_observation(removed.id, false).unwrap();

    let json = source.export_json(None).unwrap();
    let (_destination_temp, mut destination) = store();
    destination.import_json(&json).unwrap();

    let same = |store: &Store, sync_id: &str| -> crate::memory::model::Observation {
        let found = store
            .search(
                "decision",
                SearchOptions {
                    scope: Some("personal".to_owned()),
                    ..SearchOptions::default()
                },
            )
            .unwrap();
        found
            .into_iter()
            .map(|result| result.observation)
            .find(|observation| observation.sync_id == sync_id)
            .unwrap_or_else(|| panic!("{sync_id} did not survive the round trip"))
    };
    let before = same(&source, &saved.sync_id);
    let after = same(&destination, &saved.sync_id);
    // The row id belongs to the store that issued it; everything else is the
    // memory and has to arrive unchanged.
    assert_eq!(
        crate::memory::model::Observation {
            id: 0,
            ..before.clone()
        },
        crate::memory::model::Observation { id: 0, ..after },
        "a field went missing between export and import"
    );
    // The fixture has to have set them, or the comparison above is two defaults
    // agreeing with each other.
    assert!(before.pinned, "{before:?}");
    assert!(before.review_after.is_some(), "{before:?}");
    assert_eq!(before.prompt_sync_id.as_deref(), Some(asked.as_str()));
    assert_eq!(before.tool_name.as_deref(), Some("una-herramienta"));
    assert_eq!(before.topic_key.as_deref(), Some("decision/exportada"));
    assert_eq!(before.scope, "personal");
    assert!(
        before.revision_count > 1 || before.duplicate_count > 1,
        "{before:?}"
    );

    // The deletion travelled rather than being dropped on the way.
    let raw: Option<String> = destination
        .connection()
        .query_row(
            "SELECT deleted_at FROM observations WHERE sync_id = ?1",
            rusqlite::params![removed.sync_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(raw.is_some(), "a deleted memory came back alive");

    // The other three entities travel by the same route and have their own
    // hand-written column lists in the import statement. A memory that arrives
    // whole into a session that lost its summary is still a lossy backup.
    source
        .end_session("s1", Some("lo que hizo esta sesion"))
        .unwrap();
    let json = source.export_json(None).unwrap();
    let (_second_temp, mut second) = store();
    second.import_json(&json).unwrap();
    assert_eq!(
        source.get_session("s1").unwrap(),
        second.get_session("s1").unwrap(),
        "the session did not survive the round trip"
    );
    let prompt_before = source.recent_prompts(Some("leteo"), Some(1)).unwrap();
    let prompt_after = second.recent_prompts(Some("leteo"), Some(1)).unwrap();
    assert_eq!(
        crate::memory::model::Prompt {
            id: 0,
            ..prompt_before[0].clone()
        },
        crate::memory::model::Prompt {
            id: 0,
            ..prompt_after
                .first()
                .cloned()
                .expect("the prompt did not survive")
        },
        "the prompt did not survive the round trip"
    );

    // And every field the model serialises is in the JSON, so a comparison
    // cannot pass by both sides skipping the same absent value.
    let exported: serde_json::Value = serde_json::from_str(&json).unwrap();
    let carried = exported["observations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["sync_id"] == saved.sync_id.as_str())
        .expect("the memory is in the export");
    for key in [
        "id",
        "sync_id",
        "session_id",
        "type",
        "title",
        "content",
        "tool_name",
        "project",
        "scope",
        "topic_key",
        "revision_count",
        "duplicate_count",
        "last_seen_at",
        "review_after",
        "prompt_sync_id",
        "pinned",
        "created_at",
        "updated_at",
    ] {
        assert!(
            carried.get(key).is_some(),
            "the export left out {key}; if that is deliberate, say so here"
        );
    }
}

/// A merge says what it left sharing a name.
///
/// A topic key holds one live memory per project and scope, and revising one
/// finds it by exactly that triple — so saving cannot produce two. A merge can:
/// each project may have had its own memory under one key, legitimately, and
/// afterwards they share it. Nothing is lost, but the revision path takes the
/// most recently updated of the two and the other stops being reachable by its
/// own key for good.
///
/// Reported rather than resolved, because which of the two is the memory and
/// which is the twin is a judgment about their contents, and a merge that threw
/// one away silently would be the worse answer. Found by driving four hundred
/// operations over three project names in an order nobody wrote by hand.
#[test]
fn a_merge_says_how_many_topic_keys_it_left_shared() {
    let (_temp, mut store) = store();
    for (session, project) in [("s1", "leteo"), ("s2", "leteo cloud")] {
        store.create_session(session, project, "C:/repo").unwrap();
        let mut input = observation(session, &format!("Memoria de {project}"), "un cuerpo");
        input.project = Some(project.to_owned());
        input.topic_key = Some("architecture/una-clave".to_owned());
        store.add_observation(input).unwrap();
    }
    // Apart, each project keeps its own memory under the key.
    let merged = store
        .merge_projects(&["leteo cloud".to_owned()], "leteo")
        .unwrap();
    assert_eq!(merged.observations_updated, 1);
    assert_eq!(
        merged.topic_key_collisions, 1,
        "the merge put two memories under one key and has to say so"
    );

    // Both are still there; neither was thrown away.
    let held: i64 = store
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM observations WHERE deleted_at IS NULL AND topic_key = ?1",
            rusqlite::params!["architecture/una-clave"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(held, 2);

    // And an ordinary merge, with nothing shared, says nothing.
    store.create_session("s3", "tercero", "C:/repo").unwrap();
    let mut input = observation("s3", "Sin clave compartida", "otro cuerpo");
    input.project = Some("tercero".to_owned());
    store.add_observation(input).unwrap();
    let quiet = store
        .merge_projects(&["tercero".to_owned()], "leteo")
        .unwrap();
    assert_eq!(
        quiet.topic_key_collisions, 1,
        "the pair from before is still shared"
    );
}

/// Two project lists, two questions, and both are described.
///
/// `stats` answers where anything has been happening: projects holding at
/// least one live memory, most recently written first. `projects list` is the
/// inventory. On a real store they differ by two of nineteen — a project with
/// only a session and one with only a prompt — and a bare `projects` beside
/// three totals reads as the inventory, which is why the field says what it is.
#[test]
fn the_two_project_lists_answer_two_different_questions() {
    let (_temp, mut store) = store();
    store
        .create_session("kept", "con-memoria", "C:/repo")
        .unwrap();
    store
        .add_observation(AddObservation {
            project: Some("con-memoria".to_owned()),
            ..observation("kept", "Una memoria", "un cuerpo")
        })
        .unwrap();
    // A project the store knows and no memory belongs to.
    store
        .create_session("bare", "sin-memoria", "C:/otro")
        .unwrap();

    let stats = store.stats().unwrap();
    assert_eq!(
        stats.projects,
        vec!["con-memoria".to_owned()],
        "stats names where the memories are"
    );
    let inventory: Vec<String> = store
        .list_projects_with_stats()
        .unwrap()
        .into_iter()
        .map(|project| project.name)
        .collect();
    assert!(
        inventory.contains(&"sin-memoria".to_owned()),
        "the inventory names a project with only a session: {inventory:?}"
    );
    assert!(
        inventory.contains(&"con-memoria".to_owned()),
        "{inventory:?}"
    );
}

/// A merge into a name the store never held says so.
///
/// Merging into a new name is a rename, and there is no other way to perform
/// one, so it is allowed — and it is also exactly what a typo in `to` looks
/// like. The two are the same call: a whole project walks into a misspelling,
/// the reply reports success, and the memories are findable only under the
/// mistake. Every other write refuses a project name nobody invented, and
/// `project_exists` was written for that; this path never asked it.
///
/// Reported rather than refused, for the reason the topic-key collisions
/// beside it are: which of the two this was is the caller's to know, and
/// refusing would remove the only way to rename a project.
#[test]
fn a_merge_that_invents_the_canonical_project_says_it_did() {
    let (_temp, mut store) = super::store();
    for proyecto in ["uno", "dos"] {
        store
            .create_session(&format!("s-{proyecto}"), proyecto, "C:/repo")
            .unwrap();
        store
            .add_observation(crate::memory::model::AddObservation {
                session_id: format!("s-{proyecto}"),
                kind: "discovery".to_owned(),
                title: format!("Una memoria de {proyecto} con titulo suficiente"),
                content: "Un cuerpo con texto de sobra.".to_owned(),
                tool_name: None,
                project: Some(proyecto.to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
    }

    // Into a project that was already there: there is nothing to announce.
    let ordinario = store.merge_projects(&["uno".to_owned()], "dos").unwrap();
    assert_eq!(ordinario.sources_merged, vec!["uno".to_owned()]);
    assert!(!ordinario.canonical_created, "{ordinario:?}");

    // Hacia uno que no existía: eso sí.
    let renombrado = store
        .merge_projects(&["dos".to_owned()], "proyecto-con-erratta")
        .unwrap();
    assert!(renombrado.canonical_created, "{renombrado:?}");
    assert_eq!(renombrado.observations_updated, 2, "{renombrado:?}");

    // And where nothing moved, the event is not invented: asking to merge a name
    // nobody holds into another name nobody holds creates no project at all.
    let vacio = store
        .merge_projects(&["fantasma-a".to_owned()], "fantasma-b")
        .unwrap();
    assert!(vacio.sources_merged.is_empty(), "{vacio:?}");
    assert!(!vacio.canonical_created, "{vacio:?}");
}
