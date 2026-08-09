//! What two memories claim about each other.

use super::*;

#[test]
fn opens_post_conflict_snapshot_preserving_relations() {
    let (_temp, config) = legacy_database(POST_CONFLICT_SCHEMA);
    let store = Store::open(config.clone()).unwrap();

    let relation: (String, String, String, String) = store
        .connection
        .query_row(
            "SELECT source_id, target_id, relation, judgment_status
                 FROM memory_relations WHERE sync_id = 'rel-legacy'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        relation,
        (
            "obs-source".to_owned(),
            "obs-target".to_owned(),
            "conflicts_with".to_owned(),
            "judged".to_owned()
        )
    );
    assert_eq!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM sync_deferred_mutations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .search("Redis caching", SearchOptions::default())
            .unwrap()
            .len(),
        1
    );

    drop(store);
    let reopened = Store::open(config).unwrap();
    assert_eq!(
        reopened
            .connection
            .query_row(
                "SELECT COUNT(*) FROM memory_relations WHERE sync_id = 'rel-legacy'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
}

#[test]
fn soft_and_hard_delete_are_journaled_and_orphan_relations() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let first = store
        .add_observation(observation("s1", "First", "first searchable body"))
        .unwrap()
        .observation;
    let second = store
        .add_observation(observation("s1", "Second", "second body"))
        .unwrap()
        .observation;
    store
        .connection
        .execute(
            "INSERT INTO memory_relations
                 (sync_id, source_id, target_id, relation, judgment_status)
                 VALUES ('rel-1', ?1, ?2, 'related', 'judged')",
            params![first.sync_id, second.sync_id],
        )
        .unwrap();

    store.delete_observation(first.id, false).unwrap();
    assert!(
        store
            .get_observation(first.id)
            .unwrap()
            .deleted_at
            .is_some()
    );
    assert!(
        store
            .search("first searchable", SearchOptions::default())
            .unwrap()
            .is_empty()
    );
    let soft_payload: String = store
        .connection
        .query_row(
            "SELECT payload FROM sync_mutations
                 WHERE entity_key = ?1 AND op = 'delete' ORDER BY seq DESC LIMIT 1",
            [&first.sync_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&soft_payload).unwrap()["hard_delete"],
        false
    );

    store.delete_observation(first.id, true).unwrap();
    assert!(matches!(
        store.get_observation(first.id),
        Err(StoreError::ObservationNotFound(_))
    ));
    let relation_state: String = store
        .connection
        .query_row(
            "SELECT judgment_status FROM memory_relations WHERE sync_id = 'rel-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(relation_state, JUDGMENT_STATUS_ORPHANED);
    let hard_payload: String = store
        .connection
        .query_row(
            "SELECT payload FROM sync_mutations
                 WHERE entity_key = ?1 AND op = 'delete' ORDER BY seq DESC LIMIT 1",
            [&first.sync_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&hard_payload).unwrap()["hard_delete"],
        true
    );
    assert_eq!(store.get_observation(second.id).unwrap().id, second.id);
}

#[test]
fn replaying_deferred_relations_retries_then_retires_them_as_dead() {
    let (_temp, mut store) = store();
    store
        .connection
        .execute(
            "INSERT INTO sync_deferred_mutations
                 (sync_id, entity, payload, apply_status, retry_count, first_seen_at)
                 VALUES ('rel_missing', 'relation', ?1, 'deferred', 0, '2026-01-01 00:00:00')",
            [serde_json::json!({
                "sync_id": "rel_missing",
                "source_id": "obs_missing_a",
                "target_id": "obs_missing_b",
                "relation": "related",
                "judgment_status": "judged",
            })
            .to_string()],
        )
        .unwrap();
    store
            .connection
            .execute(
                "INSERT INTO sync_deferred_mutations
                 (sync_id, entity, payload, apply_status, retry_count, first_seen_at)
                 VALUES ('rel_broken', 'relation', 'not json', 'deferred', 0, '2026-01-02 00:00:00')",
                [],
            )
            .unwrap();

    // The unparsable payload is retired immediately; the missing-observation
    // one is kept for another attempt.
    let first = store.replay_deferred_sync_mutations().unwrap();
    assert_eq!(first.retried, 2);
    assert_eq!(first.succeeded, 0);
    assert_eq!(first.failed, 1);
    assert_eq!(first.dead, 1);
    assert_eq!(store.deferred_sync_counts().unwrap(), (1, 1));
    let row = store.get_deferred("rel_missing").unwrap();
    assert_eq!(row.retry_count, 1);
    assert_eq!(row.apply_status, "deferred");
    assert!(row.last_error.is_some());

    // Retrying past the threshold retires the row instead of looping forever.
    for _ in 0..4 {
        store.replay_deferred_sync_mutations().unwrap();
    }
    let row = store.get_deferred("rel_missing").unwrap();
    assert_eq!(row.apply_status, "dead");
    assert_eq!(row.retry_count, 5);
    assert_eq!(store.deferred_sync_counts().unwrap(), (0, 2));

    let exhausted = store.replay_deferred_sync_mutations().unwrap();
    assert_eq!(exhausted, ReplayDeferredResult::default());
}

#[test]
fn enrollment_controls_relation_journaling_and_is_idempotent() {
    let (_temp, mut store) = bare_store();
    assert!(store.enrolled_projects().unwrap().is_empty());

    assert!(store.enroll_project("Leteo").unwrap());
    assert!(!store.enroll_project("leteo").unwrap(), "already enrolled");
    assert_eq!(store.enrolled_projects().unwrap(), ["leteo"]);
    assert!(store.enroll_project("  ").is_err());

    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let first = store
        .add_observation(observation("s1", "Enrolled source", "source body"))
        .unwrap()
        .observation;
    let second = store
        .add_observation(observation("s1", "Enrolled target", "target body"))
        .unwrap()
        .observation;
    let relation = store
        .save_relation(SaveRelationParams {
            sync_id: normalize::sync_id("rel"),
            source_id: first.sync_id,
            target_id: second.sync_id,
        })
        .unwrap();
    store
        .judge_relation(JudgeRelationParams {
            judgment_id: relation.sync_id,
            relation: RELATION_RELATED.to_owned(),
            marked_by_actor: "agent".to_owned(),
            marked_by_kind: "agent".to_owned(),
            ..JudgeRelationParams::default()
        })
        .unwrap();
    let journaled: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM sync_mutations WHERE entity = 'relation'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(journaled, 1, "enrolled projects journal their relations");

    assert!(store.unenroll_project("LETEO").unwrap());
    assert!(!store.unenroll_project("leteo").unwrap());
    assert!(store.enrolled_projects().unwrap().is_empty());
}

/// A session summary is never proposed as a conflict candidate.
///
/// It matches easily — a session's worth of text catches whatever words a title
/// happens to use — and there is no verdict for it: it recounts a session
/// rather than claiming anything, so the agent can only answer `not_conflict`.
/// The summary here is written to score better than the real candidate, so the
/// assertion fails on the order alone if the filter goes away.
#[test]
fn a_session_summary_is_never_a_conflict_candidate() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let mut summary = observation("s1", "JWT auth token architecture", "what the session did");
    summary.kind = crate::memory::model::SESSION_SUMMARY.to_owned();
    let summary = store.add_observation(summary).unwrap().observation;
    let real = store
        .add_observation(observation(
            "s1",
            "JWT auth session architecture",
            "the first decision about auth",
        ))
        .unwrap()
        .observation;
    let saved = store
        .add_observation(observation(
            "s1",
            "JWT auth token architecture revisited",
            "the second decision about auth",
        ))
        .unwrap()
        .observation;

    let candidates = store
        .find_candidates(
            saved.id,
            CandidateOptions {
                limit: Some(3),
                bm25_floor: Some(0.0),
                ..CandidateOptions::default()
            },
        )
        .unwrap();

    assert!(
        candidates.iter().all(|c| c.sync_id != summary.sync_id),
        "a session summary was proposed for judgment: {:?}",
        candidates.iter().map(|c| &c.title).collect::<Vec<_>>()
    );
    assert_eq!(
        candidates.first().map(|c| c.sync_id.as_str()),
        Some(real.sync_id.as_str()),
        "the real candidate should lead once the summary is out of the way"
    );
}

#[test]
fn relations_cover_candidates_all_verbs_multi_actor_annotations_and_stats() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let first = store
        .add_observation(observation(
            "s1",
            "JWT auth session architecture",
            "first auth decision",
        ))
        .unwrap()
        .observation;
    let second = store
        .add_observation(observation(
            "s1",
            "JWT auth token architecture",
            "second auth decision",
        ))
        .unwrap()
        .observation;
    let relation_mutations_before: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM sync_mutations WHERE entity = 'relation'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    // Note the floor this passes, and see
    // `the_default_floor_keeps_the_matches_worth_proposing`: at `-100.0` the
    // comparison below it is true whichever way round it is written, which is
    // why every test here passed while the shipped default proposed nothing.
    let candidates = store
        .find_candidates(
            second.id,
            CandidateOptions {
                limit: Some(3),
                // Everything, said the way the comparison actually reads: no
                // bm25 score is above zero. It was `-100.0`, which meant the
                // same thing only while the comparison was the wrong way round.
                bm25_floor: Some(0.0),
                ..CandidateOptions::default()
            },
        )
        .unwrap();
    assert!(!candidates.is_empty());
    assert_eq!(candidates[0].sync_id, first.sync_id);
    assert!(candidates[0].judgment_id.starts_with("rel-"));
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM sync_mutations WHERE entity = 'relation'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        relation_mutations_before,
        "pending candidate rows are local-only"
    );

    let verbs = [
        RELATION_RELATED,
        RELATION_COMPATIBLE,
        RELATION_SCOPED,
        RELATION_CONFLICTS_WITH,
        RELATION_SUPERSEDES,
        RELATION_NOT_CONFLICT,
    ];
    let mut judged = Vec::new();
    for (index, verb) in verbs.into_iter().enumerate() {
        let judgment_id = if index == 0 {
            candidates[0].judgment_id.clone()
        } else {
            let sync_id = normalize::sync_id("rel");
            store
                .save_relation(SaveRelationParams {
                    sync_id: sync_id.clone(),
                    source_id: second.sync_id.clone(),
                    target_id: first.sync_id.clone(),
                })
                .unwrap();
            sync_id
        };
        let actor = format!("agent-{index}");
        let relation = store
            .judge_relation(JudgeRelationParams {
                judgment_id,
                relation: verb.to_owned(),
                reason: Some(format!("reason-{verb}")),
                evidence: Some(format!("evidence-{verb}")),
                confidence: Some(0.8),
                marked_by_actor: actor.clone(),
                marked_by_kind: "agent".to_owned(),
                marked_by_model: Some(format!("model-{index}")),
                session_id: Some("s1".to_owned()),
            })
            .unwrap();
        assert_eq!(relation.relation, verb);
        assert_eq!(relation.judgment_status, JUDGMENT_STATUS_JUDGED);
        assert_eq!(relation.marked_by_actor.as_deref(), Some(actor.as_str()));
        judged.push(relation);
    }

    // All six verbs now hold between these two, every one of them judged, and
    // `second` is the source of each. Only two of the six are a reason for
    // care, and only one of those two reads in a direction.
    let caveats = store
        .caveats_for(&[first.sync_id.clone(), second.sync_id.clone()])
        .unwrap();
    let verbs_on = |sync_id: &String| {
        let mut found: Vec<CaveatVerb> = caveats[sync_id].iter().map(|c| c.verb).collect();
        found.sort_by_key(|verb| verb.phrase());
        found
    };
    assert_eq!(
        verbs_on(&first.sync_id),
        [CaveatVerb::ConflictsWith, CaveatVerb::SupersededBy]
    );
    assert_eq!(
        verbs_on(&second.sync_id),
        [CaveatVerb::ConflictsWith],
        "it did the superseding, so nothing about it is stale"
    );
    let listed = store
        .list_relations(ListRelationsOptions {
            project: Some("LETEO".to_owned()),
            status: Some(JUDGMENT_STATUS_JUDGED.to_owned()),
            limit: Some(50),
            ..ListRelationsOptions::default()
        })
        .unwrap();
    assert_eq!(listed.len(), verbs.len());
    assert_eq!(
        store.get_relation_by_id(listed[0].id).unwrap().sync_id,
        listed[0].sync_id
    );
    assert_eq!(
        store
            .count_relations(ListRelationsOptions {
                project: Some("leteo".to_owned()),
                status: Some(JUDGMENT_STATUS_JUDGED.to_owned()),
                ..ListRelationsOptions::default()
            })
            .unwrap(),
        verbs.len() as i64
    );
    let stats = store.relation_stats(Some("leteo")).unwrap();
    for verb in verbs {
        assert_eq!(stats.by_relation[verb], 1);
    }
    assert_eq!(stats.by_judgment_status[JUDGMENT_STATUS_JUDGED], 6);
    assert_eq!(judged.last().unwrap().relation, RELATION_NOT_CONFLICT);
}

/// Saves a judged verdict between two memories and hands back both.
fn judged_pair(
    store: &mut Store,
    verb: &str,
    source_title: &str,
    target_title: &str,
) -> (Observation, Observation) {
    let source = store
        .add_observation(observation("s1", source_title, "source body"))
        .unwrap()
        .observation;
    let target = store
        .add_observation(observation("s1", target_title, "target body"))
        .unwrap()
        .observation;
    let relation = store
        .save_relation(SaveRelationParams {
            sync_id: normalize::sync_id("rel"),
            source_id: source.sync_id.clone(),
            target_id: target.sync_id.clone(),
        })
        .unwrap();
    store
        .judge_relation(JudgeRelationParams {
            judgment_id: relation.sync_id,
            relation: verb.to_owned(),
            marked_by_actor: "agent".to_owned(),
            marked_by_kind: "agent".to_owned(),
            ..JudgeRelationParams::default()
        })
        .unwrap();
    (source, target)
}

#[test]
fn a_supersession_warns_the_memory_that_was_overturned_and_not_the_one_that_did_it() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let (newer, older) = judged_pair(
        &mut store,
        RELATION_SUPERSEDES,
        "The new way",
        "The old way",
    );

    let caveats = store
        .caveats_for(&[older.sync_id.clone(), newer.sync_id.clone()])
        .unwrap();

    let on_older = &caveats[&older.sync_id];
    assert_eq!(on_older.len(), 1);
    assert_eq!(on_older[0].verb, CaveatVerb::SupersededBy);
    assert_eq!(on_older[0].other_id, newer.id);
    assert_eq!(on_older[0].other_title, "The new way");
    assert!(
        !caveats.contains_key(&newer.sync_id),
        "the memory that did the superseding still stands and needs no warning"
    );
}

#[test]
fn a_conflict_warns_both_ends_because_it_has_no_direction() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let (left, right) = judged_pair(
        &mut store,
        RELATION_CONFLICTS_WITH,
        "Tabs are the rule",
        "Spaces are the rule",
    );

    let caveats = store
        .caveats_for(&[left.sync_id.clone(), right.sync_id.clone()])
        .unwrap();

    assert_eq!(caveats[&left.sync_id][0].other_id, right.id);
    assert_eq!(caveats[&right.sync_id][0].other_id, left.id);
    for end in [&left.sync_id, &right.sync_id] {
        assert_eq!(caveats[end][0].verb, CaveatVerb::ConflictsWith);
    }
}

#[test]
fn the_verbs_that_say_two_memories_belong_together_raise_nothing() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    for verb in [RELATION_RELATED, RELATION_COMPATIBLE, RELATION_SCOPED] {
        let (source, target) = judged_pair(&mut store, verb, "One", "Another");
        let caveats = store
            .caveats_for(&[source.sync_id, target.sync_id])
            .unwrap();
        assert!(caveats.is_empty(), "{verb} is not a reason for care");
    }
}

#[test]
fn a_pair_nobody_has_judged_yet_is_a_guess_and_stays_quiet() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let source = store
        .add_observation(observation("s1", "First", "body"))
        .unwrap()
        .observation;
    let target = store
        .add_observation(observation("s1", "Second", "body"))
        .unwrap()
        .observation;
    // Saved by the candidate scan, never judged — which is the state seventy
    // relations in a real store have been sitting in for two months.
    store
        .save_relation(SaveRelationParams {
            sync_id: normalize::sync_id("rel"),
            source_id: source.sync_id.clone(),
            target_id: target.sync_id.clone(),
        })
        .unwrap();

    assert!(
        store
            .caveats_for(&[source.sync_id, target.sync_id])
            .unwrap()
            .is_empty(),
        "warning on an unconfirmed guess is how a hint turns into noise"
    );
}

#[test]
fn a_deleted_counterpart_stops_being_a_reason_for_care() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let (newer, older) = judged_pair(
        &mut store,
        RELATION_SUPERSEDES,
        "The new way",
        "The old way",
    );
    store.delete_observation(newer.id, false).unwrap();

    assert!(
        store.caveats_for(&[older.sync_id]).unwrap().is_empty(),
        "a memory cannot be superseded by one that is no longer there"
    );
}

#[test]
fn one_pair_judged_twice_is_still_one_reason_for_care() {
    // Two scans can both find the same pair and both get a verdict. A real
    // store holds two rows saying one memory supersedes another, and printing
    // that twice reads as two separate problems.
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let (newer, older) = judged_pair(
        &mut store,
        RELATION_SUPERSEDES,
        "The new way",
        "The old way",
    );
    let again = store
        .save_relation(SaveRelationParams {
            sync_id: normalize::sync_id("rel"),
            source_id: newer.sync_id.clone(),
            target_id: older.sync_id.clone(),
        })
        .unwrap();
    store
        .judge_relation(JudgeRelationParams {
            judgment_id: again.sync_id,
            relation: RELATION_SUPERSEDES.to_owned(),
            marked_by_actor: "another-agent".to_owned(),
            marked_by_kind: "agent".to_owned(),
            ..JudgeRelationParams::default()
        })
        .unwrap();

    let caveats = store
        .caveats_for(std::slice::from_ref(&older.sync_id))
        .unwrap();

    assert_eq!(caveats[&older.sync_id].len(), 1);
    assert_eq!(caveats[&older.sync_id][0].other_id, newer.id);
}

#[test]
fn asking_about_more_memories_than_sqlite_takes_parameters_for_still_answers() {
    // Each memory costs two bound parameters, and SQLite refuses a statement
    // past 32766 of them. `leteo context --limit` takes any number, so without
    // chunking a large enough project turned the whole opening context into an
    // error about SQL variables.
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let (newer, older) = judged_pair(
        &mut store,
        RELATION_SUPERSEDES,
        "The new way",
        "The old way",
    );

    let mut asked: Vec<String> = (0..20_000)
        .map(|index| format!("obs-absent-{index:06}"))
        .collect();
    asked.push(older.sync_id.clone());

    let caveats = store.caveats_for(&asked).unwrap();

    assert_eq!(caveats[&older.sync_id][0].other_id, newer.id);
    assert_eq!(caveats.len(), 1, "only the memory that was asked about");
}

#[test]
fn a_backup_and_restore_brings_the_judged_graph_home_with_it() {
    // An export used to be sessions, observations and prompts, so a store
    // exported and imported back came home with every relation gone — the
    // expensive half of the data, a model call per verdict, and since recall
    // started reading the graph, the half that says a memory was overturned.
    let (_source_temp, mut source) = store();
    source.create_session("s1", "leteo", "C:/repo").unwrap();
    let (newer, older) = judged_pair(
        &mut source,
        RELATION_SUPERSEDES,
        "The new way",
        "The old way",
    );

    let json = source.export_json(None).unwrap();
    let (_temp, mut restored) = store();
    let result = restored.import_json(&json).unwrap();

    assert_eq!(result.relations_imported, 1);
    assert_eq!(result.relations_skipped, 0);
    let caveats = restored
        .caveats_for(std::slice::from_ref(&older.sync_id))
        .unwrap();
    assert_eq!(caveats[&older.sync_id][0].other_title, "The new way");
    assert_eq!(
        restored.get_observation(newer.id).unwrap().title,
        "The new way"
    );
}

#[test]
fn restoring_the_same_backup_twice_does_not_double_the_graph() {
    let (_source_temp, mut source) = store();
    source.create_session("s1", "leteo", "C:/repo").unwrap();
    judged_pair(&mut source, RELATION_CONFLICTS_WITH, "Tabs", "Spaces");
    let json = source.export_json(None).unwrap();

    let (_temp, mut restored) = store();
    let first = restored.import_json(&json).unwrap();
    let second = restored.import_json(&json).unwrap();

    assert_eq!(first.relations_imported, 1);
    assert_eq!(
        (second.relations_imported, second.relations_skipped),
        (0, 0),
        "a relation already here is neither imported again nor a hole"
    );
}

#[test]
fn a_relation_whose_memories_did_not_come_along_is_counted_and_not_lost_in_silence() {
    let (_source_temp, mut source) = store();
    source.create_session("s1", "leteo", "C:/repo").unwrap();
    let (_newer, _older) = judged_pair(&mut source, RELATION_SUPERSEDES, "One", "Another");
    let mut data = source.export().unwrap();
    // The memories stay behind — what a narrower export, or a hand-edited
    // backup, can hand over.
    data.observations.clear();

    let (_temp, mut restored) = store();
    let result = restored.import(&data).unwrap();

    assert_eq!(result.observations_imported, 0);
    assert_eq!(result.relations_imported, 0);
    assert_eq!(result.relations_skipped, 1);
}

#[test]
fn an_export_written_before_relations_existed_still_imports() {
    // Every Engram export, and every Leteo one so far, has no `relations` key
    // at all. `default` has to cover that or a backup stops being readable.
    let (_temp, mut store) = store();
    let older = r#"{"version":"0.1.0","exported_at":"2026-01-01 00:00:00",
                    "sessions":[],"observations":[],"prompts":null}"#;

    let result = store.import_json(older).unwrap();

    assert_eq!(
        (result.relations_imported, result.relations_skipped),
        (0, 0)
    );
}

#[test]
fn a_pair_nobody_has_judged_yet_is_not_a_reason_for_care() {
    // `find_candidates` files a pending relation on every save — that is what
    // the judgement queue is made of. They are guesses about which memories
    // might be about the same thing, and nobody has looked yet.
    //
    // Rendering those as caveats would mark healthy memories "superseded by"
    // in the opening context of every session, on the strength of a title
    // resembling another title. An agent would learn to distrust what it is
    // handed. Only a judged relation is a claim about anything.
    //
    // Two clauses enforce that today and only one of them is load-bearing: a
    // pending row stores `relation = 'pending'`, so the `relation IN
    // ('supersedes', 'conflicts_with')` filter already excludes it and
    // removing `judgment_status = 'judged'` changes nothing reachable. That is
    // worth knowing rather than rediscovering — a mutation dropping the status
    // filter survives this test, and the reason is redundancy, not a hole.
    // What is asserted here is the behaviour, so it holds whichever clause is
    // doing the work, and it would catch a `save_relation` that ever started
    // writing a real verb before anybody judged.
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let newer = store
        .add_observation(observation("s1", "We indent with spaces now", "body"))
        .unwrap()
        .observation;
    let older = store
        .add_observation(observation("s1", "We indent with tabs", "body"))
        .unwrap()
        .observation;
    let pending = store
        .save_relation(SaveRelationParams {
            sync_id: normalize::sync_id("rel"),
            source_id: newer.sync_id,
            target_id: older.sync_id.clone(),
        })
        .unwrap();

    assert!(
        store
            .caveats_for(std::slice::from_ref(&older.sync_id))
            .unwrap()
            .is_empty(),
        "an unjudged pair is a question, not a verdict"
    );

    // And the moment somebody answers it, it counts.
    store
        .judge_relation(JudgeRelationParams {
            judgment_id: pending.sync_id,
            relation: RELATION_SUPERSEDES.to_owned(),
            marked_by_actor: "agent".to_owned(),
            marked_by_kind: "agent".to_owned(),
            ..JudgeRelationParams::default()
        })
        .unwrap();
    assert_eq!(
        store
            .caveats_for(std::slice::from_ref(&older.sync_id))
            .unwrap()
            .get(&older.sync_id)
            .map(Vec::len),
        Some(1),
        "a judged one does"
    );
}

#[test]
fn a_memory_that_was_deleted_stops_overturning_the_one_it_replaced() {
    // Deleting the newer memory takes back the claim it was making. Left in,
    // the older one carries "superseded by #N" forever, naming a memory the
    // agent cannot fetch — so the warning is unanswerable as well as wrong,
    // and the memory it discredits is the only one left.
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let (newer, older) = judged_pair(
        &mut store,
        RELATION_SUPERSEDES,
        "The new way",
        "The old way",
    );
    assert_eq!(
        store
            .caveats_for(std::slice::from_ref(&older.sync_id))
            .unwrap()
            .get(&older.sync_id)
            .map(Vec::len),
        Some(1),
        "the warning is there to begin with"
    );

    // Soft delete, which is what the TUI and `mem_delete` do by default.
    store.delete_observation(newer.id, false).unwrap();
    assert!(
        store
            .caveats_for(std::slice::from_ref(&older.sync_id))
            .unwrap()
            .is_empty(),
        "a memory nobody can read cannot overturn one they can"
    );
}

#[test]
fn a_verdict_has_to_use_a_verb_the_graph_reads_and_a_confidence_that_means_something() {
    // The verb is not a label, it is the whole of what a judgement does. The
    // caveat query — the thing that warns an agent a memory was overturned —
    // filters on `relation IN ('supersedes', 'conflicts_with')`. A verb
    // outside the six documented ones is therefore stored, reported as
    // judged, counted in the statistics, and **read by nothing**: the
    // judgement exists and has no effect, which is indistinguishable from
    // having judged the pair harmless.
    //
    // Confidence is narrower but the same shape: it is shown to a person
    // deciding whether to trust a verdict, and a number outside zero to one
    // is not a probability.
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let source = store
        .add_observation(observation("s1", "The new way", "body"))
        .unwrap()
        .observation;
    let target = store
        .add_observation(observation("s1", "The old way", "body"))
        .unwrap()
        .observation;
    let pending = |store: &mut Store| {
        store
            .save_relation(SaveRelationParams {
                sync_id: normalize::sync_id("rel"),
                source_id: source.sync_id.clone(),
                target_id: target.sync_id.clone(),
            })
            .unwrap()
            .sync_id
    };

    for refused in ["", "supersede", "SUPERSEDES", "overrides", "conflicts-with"] {
        let judgment_id = pending(&mut store);
        assert!(
            matches!(
                store.judge_relation(JudgeRelationParams {
                    judgment_id,
                    relation: refused.to_owned(),
                    marked_by_actor: "agent".to_owned(),
                    marked_by_kind: "agent".to_owned(),
                    ..JudgeRelationParams::default()
                }),
                Err(StoreError::InvalidRelationVerb { .. })
            ),
            "{refused:?} was accepted as a verdict the graph cannot read"
        );
    }

    for refused in [-0.1, 1.1, f64::NAN] {
        let judgment_id = pending(&mut store);
        assert!(
            store
                .judge_relation(JudgeRelationParams {
                    judgment_id,
                    relation: RELATION_SUPERSEDES.to_owned(),
                    confidence: Some(refused),
                    marked_by_actor: "agent".to_owned(),
                    marked_by_kind: "agent".to_owned(),
                    ..JudgeRelationParams::default()
                })
                .is_err(),
            "{refused} was accepted as a confidence"
        );
    }

    // And every documented verb goes through, so this refuses what the graph
    // cannot use rather than everything unfamiliar.
    for accepted in [
        RELATION_SUPERSEDES,
        RELATION_CONFLICTS_WITH,
        RELATION_RELATED,
        RELATION_COMPATIBLE,
        RELATION_SCOPED,
        RELATION_NOT_CONFLICT,
    ] {
        let judgment_id = pending(&mut store);
        store
            .judge_relation(JudgeRelationParams {
                judgment_id,
                relation: accepted.to_owned(),
                confidence: Some(0.8),
                marked_by_actor: "agent".to_owned(),
                marked_by_kind: "agent".to_owned(),
                ..JudgeRelationParams::default()
            })
            .unwrap_or_else(|error| panic!("{accepted} was refused: {error}"));
    }
}

#[test]
fn the_default_floor_keeps_the_matches_worth_proposing() {
    // Conflict detection runs on every save and had stopped proposing anything
    // at all. On a real store of 3,682 memories: ten saved through `mem_save`
    // in one day, zero candidates, and the last relation of any kind five days
    // old — while the query underneath returned four near-duplicates apiece,
    // scoring between -40 and -64.
    //
    // bm25 in SQLite is negative and more negative is better, so `-2.0` is a
    // ceiling on how weak a match may be. Comparing it the other way round kept
    // the matches close to zero — the ones that barely match at all — and threw
    // away everything worth showing anybody.
    //
    // It survived every test here because scores only reach those magnitudes
    // once a store has some size: the term weight grows with how rare a word is
    // across the corpus, so on a three-memory fixture everything scores near
    // zero and lands on the kept side of the comparison whichever way it is
    // written. The one test that set a floor at all set it to `-100.0`, which
    // is true both ways. So this fills the store first, and asserts on the
    // default rather than on a floor chosen to make the case pass.
    let (_temp, mut store) = store();
    store.create_session("s1", "Leteo", "C:/repo").unwrap();
    for index in 0..40 {
        store
            .add_observation(observation(
                "s1",
                &format!("Unrelated note {index} about scheduling and retries"),
                &format!("Body {index}: back-off, jitter and a queue that drains in order."),
            ))
            .unwrap();
    }
    let first = store
        .add_observation(observation(
            "s1",
            "The passive capture hash was taken before the redaction",
            "So the duplicate check could never match what the store kept.",
        ))
        .unwrap()
        .observation;
    let second = store
        .add_observation(observation(
            "s1",
            "The passive capture hash was taken before the redaction ran",
            "The duplicate check therefore never matched what the store kept.",
        ))
        .unwrap()
        .observation;

    // What the query underneath sees, so a failure says whether the match was
    // missing or thrown away.
    let best: f64 = store
        .connection
        .query_row(
            "SELECT bm25(observations_fts) FROM observations_fts fts
              JOIN observations o ON o.id = fts.rowid
             WHERE observations_fts MATCH 'passive OR capture OR hash OR redaction'
               AND o.id = ?1",
            [first.id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(best < -2.0, "the fixture is too small to bite: {best}");

    let candidates = store
        .find_candidates(second.id, CandidateOptions::default())
        .unwrap();
    assert!(
        !candidates.is_empty(),
        "a near-duplicate scoring {best} was not worth proposing"
    );
    assert_eq!(candidates[0].sync_id, first.sync_id);

    // And the floor still refuses what it is there to refuse: a memory sharing
    // only ordinary words is not a conflict worth a verdict.
    let unrelated = store
        .add_observation(observation(
            "s1",
            "Unrelated note 41 about scheduling and retries",
            "Back-off, jitter and a queue that drains in order.",
        ))
        .unwrap()
        .observation;
    let weak = store
        .find_candidates(
            unrelated.id,
            CandidateOptions {
                bm25_floor: Some(-1000.0),
                ..CandidateOptions::default()
            },
        )
        .unwrap();
    assert!(
        weak.iter().all(|candidate| candidate.score < -2.0),
        "the floor has to mean something: {weak:?}"
    );
}

#[test]
fn a_pair_already_ruled_on_is_not_proposed_a_second_time() {
    // `find_candidates` runs on every save and files a pending relation for
    // what it proposes. Nothing stopped it filing the same pair twice: a
    // memory saved again lands on the same row through the dedupe path, this
    // runs on that same id, and the pair is proposed afresh.
    //
    // On a real store fourteen pairs already carried more than one row, one of
    // them four — and that was while the floor bug kept this from proposing
    // anything at all. A duplicate costs a second model call for a question
    // already answered, and inflates the count of verdicts a session opening
    // reports as waiting.
    //
    // The batch scan has guarded against exactly this from the start, in
    // `scan_project`, which is where the shape came from.
    let (_temp, mut store) = store();
    store.create_session("s1", "Leteo", "C:/repo").unwrap();
    for index in 0..40 {
        store
            .add_observation(observation(
                "s1",
                &format!("Unrelated note {index} on deployment windows"),
                &format!("Body {index}: staged rollout, canaries and a rollback plan."),
            ))
            .unwrap();
    }
    let first = store
        .add_observation(observation(
            "s1",
            "The passive capture hash was taken before the redaction",
            "So the duplicate check could never match what the store kept.",
        ))
        .unwrap()
        .observation;
    let second = store
        .add_observation(observation(
            "s1",
            "The passive capture hash was taken before the redaction ran",
            "The duplicate check therefore never matched what the store kept.",
        ))
        .unwrap()
        .observation;

    let proposed = store
        .find_candidates(second.id, CandidateOptions::default())
        .unwrap();
    assert_eq!(proposed.len(), 1);
    assert_eq!(proposed[0].sync_id, first.sync_id);

    let again = store
        .find_candidates(second.id, CandidateOptions::default())
        .unwrap();
    assert!(
        again.is_empty(),
        "the same pair was proposed twice: {again:?}"
    );

    // Either way round: the relation is symmetric, so the older memory saved
    // again must not propose the newer one back.
    let reversed = store
        .find_candidates(first.id, CandidateOptions::default())
        .unwrap();
    assert!(
        reversed.iter().all(|c| c.sync_id != second.sync_id),
        "the same pair was proposed backwards: {reversed:?}"
    );

    let rows: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM memory_relations WHERE source_id = ?1 OR target_id = ?1",
            [&second.sync_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rows, 1, "one pair, one row to rule on");

    // And a preview still shows it, because a dry run reporting "nothing to
    // look at" for a store full of judged pairs would be a different lie.
    let preview = store
        .find_candidates(
            second.id,
            CandidateOptions {
                skip_insert: true,
                ..CandidateOptions::default()
            },
        )
        .unwrap();
    assert_eq!(preview.len(), 1, "a preview hides nothing");
}

/// The project a search is already restricted to must not score.
///
/// The candidate query was the only ranking in the store that weighted the
/// `project` column, and it runs on a set already narrowed to one project — so
/// the column holds the same value for every candidate and cannot tell them
/// apart, while still moving each of them by a different amount according to
/// its length. Titles say the project's name all the time: 328 of 600 on a
/// real store, `Session summary: leteo` and every title that mentions what it
/// is about.
///
/// Asserted as "the word changes nothing" rather than as "this memory is not a
/// candidate", because the second does not discriminate. That term matches
/// almost every document in the store, so its idf is negative and it can never
/// lift an unrelated memory past the floor on its own. What it does is reorder
/// genuine candidates — 84 of 600 real memories were proposed a different set —
/// and this is the smallest statement of that.
#[test]
fn a_title_that_says_the_project_name_scores_exactly_as_one_that_does_not() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let target = store
        .add_observation(observation(
            "s1",
            "Untangled the connection pool",
            "it leaked under load",
        ))
        .unwrap()
        .observation;
    // Both saved before either is scored, so the two calls see one corpus: a
    // word's weight depends on how many documents hold it, and saving between
    // the calls would move that underneath the comparison.
    let plain = store
        .add_observation(observation("s1", "Untangled the pool", "the same ground"))
        .unwrap()
        .observation;
    let with_project = store
        .add_observation(observation(
            "s1",
            "leteo: untangled the pool",
            "the same ground",
        ))
        .unwrap()
        .observation;

    let score_of = |store: &mut Store, source: i64| {
        store
            .find_candidates(
                source,
                CandidateOptions {
                    skip_insert: true,
                    limit: Some(9),
                    bm25_floor: Some(0.0),
                    ..CandidateOptions::default()
                },
            )
            .unwrap()
            .into_iter()
            .find(|candidate| candidate.id == target.id)
            .map(|candidate| candidate.score)
            .expect("the memory both titles are about is a candidate for both")
    };

    let without = score_of(&mut store, plain.id);
    let naming_it = score_of(&mut store, with_project.id);
    assert_eq!(
        without, naming_it,
        "naming the project the search is already inside must not change a score"
    );
}

/// A confidence a peer sends is held to the range this store means by it.
///
/// `judge_relation` checks 0..=1 before it writes and the replicating path
/// checked nothing, so a peer could put any number in a column every reader
/// treats as a probability.
///
/// Dropped rather than clamped, and rather than refused: clamping invents a
/// number nobody produced, and refusing loses a peer's judgment — which is why
/// normalisation lives on this path and rejection stays at the door. `NULL` is
/// what the store already means by "nobody said".
#[test]
fn a_confidence_outside_the_range_arrives_as_no_confidence() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let first = store
        .add_observation(observation("s1", "The pool leaked", "under load"))
        .unwrap()
        .observation;
    let second = store
        .add_observation(observation(
            "s1",
            "The pool was fixed",
            "it is returned now",
        ))
        .unwrap()
        .observation;

    // Distinct sequence numbers: the cursor only moves forward, so a second
    // mutation at the same seq is one the puller has already seen.
    let sent = |seq: i64, sync_id: &str, confidence: f64| SyncMutation {
        seq,
        target_key: "cloud".to_owned(),
        entity: "relation".to_owned(),
        entity_key: sync_id.to_owned(),
        op: crate::sync::OP_UPSERT.to_owned(),
        payload: serde_json::json!({
            "sync_id": sync_id,
            "source_id": second.sync_id,
            "target_id": first.sync_id,
            "relation": "supersedes",
            "judgment_status": "judged",
            "confidence": confidence,
        })
        .to_string(),
        source: "remote".to_owned(),
        project: "leteo".to_owned(),
        occurred_at: "2026-08-05 04:00:00".to_owned(),
        acked_at: None,
    };
    store
        .apply_pulled_sync_mutation("cloud", &sent(3, "rel-impossible", 5.0))
        .unwrap();
    store
        .apply_pulled_sync_mutation("cloud", &sent(4, "rel-ordinary", 0.75))
        .unwrap();

    let confidence_of = |sync_id: &str| -> Option<f64> {
        store
            .connection
            .query_row(
                "SELECT confidence FROM memory_relations WHERE sync_id = ?1",
                params![sync_id],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(
        confidence_of("rel-impossible"),
        None,
        "a number outside 0..=1 is not a probability and is not stored as one"
    );
    assert_eq!(
        confidence_of("rel-ordinary"),
        Some(0.75),
        "and an ordinary one arrives untouched"
    );
}

/// What somebody wrote to explain a judgment is held to the rules the rest is.
///
/// `mem_judge` takes a reason and a piece of evidence, both free text from an
/// agent, and both went into the row exactly as they arrived. So a token
/// wrapped in `<private>…</private>` while explaining why two memories argue
/// was stored and read back in every conflict listing — the same hole the
/// session summary had, in the last two write doors that were missing it.
///
/// Both paths in one test, because that is the lesson of the three fixes before
/// it: a rule applied at one door and not its sibling is how all of them
/// started.
#[test]
fn a_judgment_is_redacted_and_bounded_on_both_paths() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let older = store
        .add_observation(observation("s1", "Chose SQLite", "the engine decision"))
        .unwrap()
        .observation;
    let newer = store
        .add_observation(observation(
            "s1",
            "Chose Postgres",
            "the engine decision again",
        ))
        .unwrap()
        .observation;
    let relation = store
        .save_relation(crate::memory::model::SaveRelationParams {
            sync_id: crate::memory::normalize::sync_id("rel"),
            source_id: newer.sync_id.clone(),
            target_id: older.sync_id.clone(),
        })
        .unwrap();

    let judged = store
        .judge_relation(crate::memory::model::JudgeRelationParams {
            judgment_id: relation.sync_id.clone(),
            relation: crate::store::RELATION_SUPERSEDES.to_owned(),
            reason: Some("Same session. <private>the token is hunter2</private>".to_owned()),
            evidence: Some("x ".repeat(store.config.max_observation_length)),
            marked_by_actor: "agent".to_owned(),
            marked_by_kind: "agent".to_owned(),
            ..Default::default()
        })
        .unwrap();

    let reason = judged.reason.expect("the judgment kept its reason");
    assert!(!reason.contains("hunter2"), "{reason}");
    assert!(reason.contains("[REDACTED]"), "{reason}");
    assert!(
        judged.evidence.unwrap_or_default().len() <= store.config.max_observation_length,
        "evidence nobody bounded is evidence somebody can flood"
    );

    // The same, arriving from a peer. Replication never refuses, so this is the
    // path a judgment written by an older build lands on.
    let mutation = SyncMutation {
        seq: 11,
        target_key: "cloud".to_owned(),
        entity: "relation".to_owned(),
        entity_key: relation.sync_id.clone(),
        op: crate::sync::OP_UPSERT.to_owned(),
        payload: serde_json::json!({
            "sync_id": relation.sync_id,
            "source_id": newer.sync_id,
            "target_id": older.sync_id,
            "relation": "supersedes",
            "reason": "From a peer. <private>the token is hunter2</private>",
            "judgment_status": "judged",
            "project": "leteo",
            "created_at": "2026-08-05 10:00:00",
            "updated_at": "2026-08-05 10:00:00",
        })
        .to_string(),
        source: "remote".to_owned(),
        project: "leteo".to_owned(),
        occurred_at: "2026-08-05 10:00:00".to_owned(),
        acked_at: None,
    };
    assert!(
        store
            .apply_pulled_sync_mutation("cloud", &mutation)
            .unwrap()
    );
    let replicated = store
        .get_relation(&relation.sync_id)
        .unwrap()
        .reason
        .unwrap_or_default();
    assert!(!replicated.contains("hunter2"), "{replicated}");
    assert!(replicated.contains("[REDACTED]"), "{replicated}");
}

/// A verdict is about two memories, so both have to be there.
///
/// The replicated door has always said so: a relation whose ends have not
/// arrived is deferred rather than stored. The two local doors never asked,
/// because the project lookup they share answers with an empty project for a
/// memory that is not there and an empty project is what the cross-project
/// check skips over — so `mem_compare` would answer with a `sync_id` as though
/// something had been judged about a memory that had been hard-deleted.
///
/// Judging an existing relation is the same door from the other side: it sets
/// `judgment_status = 'judged'` outright, so a relation marked `orphaned` when
/// its memory was deleted could be brought back as a live verdict about
/// nothing.
#[test]
fn a_verdict_needs_both_of_the_memories_it_is_about() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let kept = store
        .add_observation(observation("s1", "La que se queda", "cuerpo uno"))
        .unwrap()
        .observation;
    let doomed = store
        .add_observation(observation("s1", "La que se borra", "cuerpo dos"))
        .unwrap()
        .observation;

    let verdict = |store: &mut Store, source: &str, target: &str| {
        store.judge_by_semantic(crate::memory::model::JudgeBySemanticParams {
            source_id: source.to_owned(),
            target_id: target.to_owned(),
            relation: "supersedes".to_owned(),
            confidence: Some(0.9),
            reasoning: Some("porque si".to_owned()),
            ..Default::default()
        })
    };

    // While both are there, the verdict is recorded.
    let judged = verdict(&mut store, &kept.sync_id, &doomed.sync_id).unwrap();
    assert!(!judged.is_empty());

    // Hard deletion takes the memory away and marks what was said about it.
    store.delete_observation(doomed.id, true).unwrap();
    let relation = store.get_relation(&judged).unwrap();
    assert_eq!(relation.judgment_status, "orphaned", "{relation:?}");

    // Now the same verdict is refused rather than recorded about nothing.
    let refused = verdict(&mut store, &kept.sync_id, &doomed.sync_id).unwrap_err();
    let message = refused.to_string();
    assert!(
        message.contains(&doomed.sync_id) && message.contains("not a memory this store holds"),
        "the refusal has to name which end is missing: {message}"
    );

    // And judging the orphaned relation does not bring it back as a live one.
    let refused = store
        .judge_relation(crate::memory::model::JudgeRelationParams {
            judgment_id: judged.clone(),
            relation: "supersedes".to_owned(),
            reason: None,
            evidence: None,
            confidence: None,
            marked_by_actor: "agent".to_owned(),
            marked_by_kind: "agent".to_owned(),
            session_id: None,
            marked_by_model: None,
        })
        .unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("not a memory this store holds"),
        "{refused}"
    );
    assert_eq!(
        store.get_relation(&judged).unwrap().judgment_status,
        "orphaned",
        "an orphaned verdict came back to life"
    );

    // A soft-deleted memory is still a row, and a judgment about it is still
    // about something the store holds.
    let soft = store
        .add_observation(observation("s1", "La que se oculta", "cuerpo tres"))
        .unwrap()
        .observation;
    store.delete_observation(soft.id, false).unwrap();
    assert!(verdict(&mut store, &kept.sync_id, &soft.sync_id).is_ok());
}

/// A save is asked about what stands out, not about whatever scored at all.
///
/// The gate was an absolute bm25 floor, and bm25 is not comparable between
/// queries: 399 of 400 real saves got the full three proposals, and a memory
/// about a paella came back with three questions about git branch flow — with
/// `judgment_required`, which the skill answers by telling the agent to ask the
/// user before ruling on a decision. The margin is relative to the median of
/// what this query matched, so it means the same thing for a two-word title and
/// a twelve-word one. See `CANDIDATE_MARGIN` for the label it was chosen
/// against.
///
/// The fixture is the shape that argument needs: a crowd of memories that share
/// one ordinary word with the title being saved, and one that restates it. The
/// crowd has to fill the sample, or the margin does not apply at all — which is
/// the other half of the rule and has its own assertion below.
#[test]
fn a_candidate_has_to_beat_the_ordinary_match_for_its_own_query() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let mut añadir = |titulo: &str, cuerpo: &str| -> i64 {
        store
            .add_observation(observation("s1", titulo, cuerpo))
            .unwrap()
            .observation
            .id
    };

    // The crowd: it shares a few ordinary words of the title and nothing of the
    // subject. They have to match well enough to clear the absolute floor, or the
    // guard would be green for a reason that is not its own — which is how it came
    // out the first time this was written.
    for index in 0..crate::store::RECALL_SAMPLE + 6 {
        añadir(
            &format!("El asunto de texto suelto que se quedó en la nota {index}"),
            &format!("Un cuerpo que no dice nada de índices ni de disparadores {index}."),
        );
    }
    // And the one that does say the same thing as what is about to be saved.
    let gemela = añadir(
        "El índice de texto completo se quedó sin disparadores tras la migración",
        "Las escrituras dejaron de llegar al índice y las búsquedas contestaban con lo de ayer.",
    );

    let guardada = añadir(
        "El índice de texto completo se quedó sin disparadores en la migración",
        "Otra vez lo mismo, escrito de nuevo en una sesión posterior.",
    );
    // With the absolute floor opened up, so the only thing filtering is the margin.
    //
    // It is the variable under test and it has to be isolated: bm25 grows with the
    // size of the index — the same match is worth -0.0 in a store of one memory
    // and -53 in one of three thousand — so in a test store the crowd does not
    // even reach the -2.0 floor and the guard would go green without the margin
    // doing anything. That is what happened the first two times this was written.
    // In the real store of 1,712 memories the crowd clears it without effort,
    // which is what the defect was about.
    let sin_suelo = || crate::memory::model::CandidateOptions {
        project: Some("leteo".to_owned()),
        skip_insert: true,
        bm25_floor: Some(0.0),
        ..Default::default()
    };
    let propuestas = store.find_candidates(guardada, sin_suelo()).unwrap();
    assert!(
        propuestas.iter().any(|c| c.id == gemela),
        "la que dice lo mismo se propone: {propuestas:?}"
    );
    assert_eq!(
        propuestas.len(),
        1,
        "y la multitud no, porque no destaca sobre lo corriente: {propuestas:?}"
    );

    // The other half of the rule: with no field to be the median of, the floor
    // decides.
    //
    // A store where everything that matches matches equally well — a few
    // revisions of one memory and nothing else — is exactly where a proposal is
    // worth most, and there the median sits on top of the best one and the margin
    // would throw them all away. The one that watches that half with teeth is
    // `saving_the_same_memory_again_asks_no_new_questions`, which broke the moment
    // the margin arrived; this one keeps it company rather than replacing it.
    let (_temp2, mut pequeño) = super::store();
    pequeño.create_session("s1", "leteo", "C:/repo").unwrap();
    // Ten, not two: below the sample but with enough corpus for bm25 to mean
    // anything. The score grows with the index — the same match is worth -0.0 in
    // a store of one memory and -24 in one of fifty — so two memories do not
    // even clear the absolute floor, and that is from before there was any margin
    // at all.
    for index in 0..8 {
        pequeño
            .add_observation(observation(
                "s1",
                &format!("El índice se quedó sin disparadores, dicho de otra forma {index}"),
                &format!("Las escrituras no llegaban al índice, otra vez {index}."),
            ))
            .unwrap();
    }
    let primera = pequeño
        .add_observation(observation(
            "s1",
            "El índice de texto completo se quedó sin disparadores tras la migración",
            "Las escrituras dejaron de llegar al índice.",
        ))
        .unwrap()
        .observation
        .id;
    let segunda = pequeño
        .add_observation(observation(
            "s1",
            "El índice de texto completo se quedó sin disparadores en la migración",
            "Otra vez lo mismo, escrito de nuevo.",
        ))
        .unwrap()
        .observation
        .id;
    let propuestas = pequeño
        .find_candidates(
            segunda,
            crate::memory::model::CandidateOptions {
                project: Some("leteo".to_owned()),
                skip_insert: true,
                bm25_floor: Some(0.0),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        propuestas.iter().any(|c| c.id == primera),
        "un store sin fondo sigue preguntando: {propuestas:?}"
    );
}

/// Moving a memory to another project retires the proposals it strands, and
/// only those.
///
/// A relation joins two memories of one project, so the moment one end walks
/// out the pair can never be judged again — measured, not assumed: the judgment
/// comes back "a relation joins two memories of one project, and these are in
/// leteo and otro". Nothing marked it, so it stayed `pending` and was counted
/// in every queue that counts pending rows, for good.
///
/// The care is in what is *not* touched. A judged verdict survives the move,
/// because `caveats_for` does not filter by project: a `supersedes` recorded
/// before the move still hangs its warning on the memory it overturned, and
/// tidying the proposal away must not take a real warning off six surfaces.
#[test]
fn moving_a_memory_out_of_a_project_retires_only_the_pending_proposals() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let queda = store
        .add_observation(observation("s1", "Stays put", "the fixed end"))
        .unwrap()
        .observation;
    let propuesta = store
        .add_observation(observation("s1", "Merely proposed", "nobody ruled on this"))
        .unwrap()
        .observation;
    let juzgada = store
        .add_observation(observation(
            "s1",
            "Already judged",
            "somebody ruled on this",
        ))
        .unwrap()
        .observation;

    let pendiente = crate::memory::normalize::sync_id("rel");
    store
        .save_relation(SaveRelationParams {
            sync_id: pendiente.clone(),
            source_id: queda.sync_id.clone(),
            target_id: propuesta.sync_id.clone(),
        })
        .unwrap();
    let veredicto = store
        .save_relation(SaveRelationParams {
            sync_id: crate::memory::normalize::sync_id("rel"),
            source_id: juzgada.sync_id.clone(),
            target_id: queda.sync_id.clone(),
        })
        .unwrap();
    store
        .judge_relation(crate::memory::model::JudgeRelationParams {
            judgment_id: veredicto.sync_id.clone(),
            relation: crate::store::RELATION_SUPERSEDES.to_owned(),
            marked_by_actor: "agent".to_owned(),
            marked_by_kind: "agent".to_owned(),
            ..Default::default()
        })
        .unwrap();

    // Both ends move out, one carrying a proposal and one a verdict.
    for id in [propuesta.id, juzgada.id] {
        store
            .update_observation(
                id,
                crate::memory::model::UpdateObservation {
                    project: Some("otro".to_owned()),
                    ..Default::default()
                },
            )
            .unwrap();
    }

    assert_eq!(
        store.get_relation(&pendiente).unwrap().judgment_status,
        crate::store::JUDGMENT_STATUS_ORPHANED,
        "a proposal nothing can ever settle stops being pending"
    );
    assert_eq!(
        store.count_pending_judgeable("leteo").unwrap(),
        0,
        "so the queue can reach zero"
    );
    assert_eq!(
        store
            .count_relations(crate::memory::model::ListRelationsOptions {
                project: Some("leteo".to_owned()),
                status: Some(crate::store::JUDGMENT_STATUS_PENDING.to_owned()),
                ..Default::default()
            })
            .unwrap(),
        0,
        "and nothing is left counted that nobody could act on"
    );

    // The verdict is untouched, and the warning it carries is still delivered.
    assert_eq!(
        store
            .get_relation(&veredicto.sync_id)
            .unwrap()
            .judgment_status,
        crate::store::JUDGMENT_STATUS_JUDGED,
        "a recorded verdict is not tidied away by a move"
    );
    let caveats = store
        .caveats_for(std::slice::from_ref(&queda.sync_id))
        .unwrap();
    assert!(
        caveats
            .get(&queda.sync_id)
            .is_some_and(|said| !said.is_empty()),
        "and it still warns the memory it overturned: {caveats:?}"
    );
}
