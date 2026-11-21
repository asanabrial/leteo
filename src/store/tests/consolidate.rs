//! Merging several memories into one, and what the merge hides.

use super::*;

fn count(store: &Store, sql: &str) -> i64 {
    store
        .connection
        .query_row(sql, [], |row| row.get(0))
        .unwrap()
}

fn a_merge(source_ids: Vec<i64>, expected_project: &str) -> ConsolidateObservations {
    ConsolidateObservations {
        session_id: "s1".to_owned(),
        kind: "decision".to_owned(),
        title: "The merged decision".to_owned(),
        content: "one decision in place of the ones it replaces".to_owned(),
        tool_name: None,
        project: Some("leteo".to_owned()),
        scope: "project".to_owned(),
        topic_key: None,
        source_ids,
        expected_project: Some(expected_project.to_owned()),
    }
}

#[test]
fn a_consolidation_inserts_the_replacement_and_one_judged_relation_per_source() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let first = store
        .add_observation(observation("s1", "First duplicate", "the first body"))
        .unwrap()
        .observation;
    let second = store
        .add_observation(observation("s1", "Second duplicate", "the second body"))
        .unwrap()
        .observation;
    let third = store
        .add_observation(observation("s1", "Third duplicate", "the third body"))
        .unwrap()
        .observation;

    let outcome = store
        .consolidate_observations(a_merge(vec![first.id, second.id, third.id], "leteo"))
        .unwrap();

    assert_eq!(outcome.observation.title, "The merged decision");
    assert_eq!(outcome.observation.project.as_deref(), Some("leteo"));
    assert_eq!(outcome.sources, vec![first.id, second.id, third.id]);
    assert_eq!(outcome.relations.len(), 3);
    for (relation, source) in outcome.relations.iter().zip([&first, &second, &third]) {
        assert_eq!(relation.relation, RELATION_SUPERSEDES);
        assert_eq!(relation.judgment_status, JUDGMENT_STATUS_JUDGED);
        assert_eq!(relation.source_id, outcome.observation.sync_id);
        assert_eq!(relation.target_id, source.sync_id);
        // And it is the row the store holds, not one assembled for the reply.
        let stored = store.get_relation(&relation.sync_id).unwrap();
        assert_eq!(stored.target_id, source.sync_id);
        assert_eq!(stored.judgment_status, JUDGMENT_STATUS_JUDGED);
    }
}

#[test]
fn a_consolidation_naming_a_source_in_another_project_writes_nothing() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    store.create_session("s2", "otro", "C:/otro").unwrap();
    let first = store
        .add_observation(observation("s1", "First duplicate", "the first body"))
        .unwrap()
        .observation;
    let second = store
        .add_observation(observation("s1", "Second duplicate", "the second body"))
        .unwrap()
        .observation;
    let mut elsewhere = observation("s2", "Elsewhere", "a body in another project");
    elsewhere.project = Some("otro".to_owned());
    let elsewhere = store.add_observation(elsewhere).unwrap().observation;

    let observations_before = count(&store, "SELECT COUNT(*) FROM observations");
    let relations_before = count(&store, "SELECT COUNT(*) FROM memory_relations");

    let refused = store
        .consolidate_observations(a_merge(vec![first.id, second.id, elsewhere.id], "leteo"))
        .unwrap_err();
    assert!(
        matches!(refused, StoreError::ProjectMismatch { .. }),
        "the odd source has to be refused by name: {refused}"
    );

    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM observations"),
        observations_before,
        "a refused merge leaves no replacement row"
    );
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM memory_relations"),
        relations_before,
        "and no relation"
    );
    assert_eq!(
        count(
            &store,
            "SELECT COUNT(*) FROM observations WHERE title = 'The merged decision'"
        ),
        0
    );
}

#[test]
fn a_consolidation_naming_a_source_that_is_not_there_writes_nothing() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let first = store
        .add_observation(observation("s1", "First duplicate", "the first body"))
        .unwrap()
        .observation;
    let second = store
        .add_observation(observation("s1", "Second duplicate", "the second body"))
        .unwrap()
        .observation;

    let observations_before = count(&store, "SELECT COUNT(*) FROM observations");
    let relations_before = count(&store, "SELECT COUNT(*) FROM memory_relations");

    let refused = store
        .consolidate_observations(a_merge(vec![first.id, second.id, 9_999], "leteo"))
        .unwrap_err();
    assert!(
        matches!(refused, StoreError::ObservationNotFound(9_999)),
        "a source that does not exist has to be named: {refused}"
    );
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM observations"),
        observations_before
    );
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM memory_relations"),
        relations_before
    );
}

#[test]
fn a_consolidation_hides_its_sources_and_a_reversal_restores_them() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let older = store
        .add_observation(observation(
            "s1",
            "The pool leaked",
            "the pool was never returned to the caller",
        ))
        .unwrap()
        .observation;
    let other = store
        .add_observation(observation(
            "s1",
            "The pool was small",
            "a small pool, sized by hand",
        ))
        .unwrap()
        .observation;
    // Pinned, because the context lists pins on their own and the hiding rule
    // has to reach that door too. A second pin stays, so the list is not empty
    // and the assertion is watching something.
    let bystander = store
        .add_observation(observation(
            "s1",
            "A pinned bystander",
            "not part of the merge at all",
        ))
        .unwrap()
        .observation;
    store.pin_observation(older.id, None).unwrap();
    store.pin_observation(bystander.id, None).unwrap();

    let outcome = store
        .consolidate_observations(ConsolidateObservations {
            content: "the pool leaked and was never returned to the caller; it was small"
                .to_owned(),
            ..a_merge(vec![older.id, other.id], "leteo")
        })
        .unwrap();

    let search = |store: &Store, query: &str| {
        store
            .search(
                query,
                SearchOptions {
                    project: Some("leteo".to_owned()),
                    ..SearchOptions::default()
                },
            )
            .unwrap()
    };

    let found = search(&store, "pool leaked");
    assert!(
        found.iter().all(|result| result.observation.id != older.id),
        "a superseded memory is not a search result: {found:?}"
    );
    assert!(
        found
            .iter()
            .any(|result| result.observation.id == outcome.observation.id),
        "the memory that replaced it is: {found:?}"
    );

    let recent = store.recent_memories(Some("leteo"), None, 50).unwrap();
    assert!(
        recent
            .iter()
            .all(|observation| observation.id != older.id && observation.id != other.id),
        "the context leaves both sources out: {recent:?}"
    );
    assert!(
        recent
            .iter()
            .any(|observation| observation.id == outcome.observation.id)
    );

    // The raw recent listing — the CLI `recent` command, the Obsidian view and
    // the conflict scan — is the sibling of the context above, and hid them
    // until this was added.
    let raw_recent = store
        .recent_observations(Some("leteo"), Some(50), true)
        .unwrap();
    assert!(
        raw_recent
            .iter()
            .all(|observation| observation.id != older.id && observation.id != other.id),
        "the raw recent listing leaves both sources out: {raw_recent:?}"
    );
    assert!(
        raw_recent
            .iter()
            .any(|observation| observation.id == outcome.observation.id)
    );

    let (pinned, _) = store.pinned_observations(Some("leteo"), None, 50).unwrap();
    assert!(
        pinned.iter().all(|observation| observation.id != older.id),
        "a pinned source is hidden as well: {pinned:?}"
    );
    assert!(
        pinned
            .iter()
            .any(|observation| observation.id == bystander.id),
        "and the pins that stand are still listed: {pinned:?}"
    );

    // Hidden from the listings, not from a direct read: the memory comes back
    // by id and still says what overturned it.
    let by_id = store.get_observation(older.id).unwrap();
    assert_eq!(by_id.title, "The pool leaked");
    let caveats = store
        .caveats_for(std::slice::from_ref(&older.sync_id))
        .unwrap();
    assert_eq!(caveats[&older.sync_id][0].verb, CaveatVerb::SupersededBy);

    // A re-verdict restores the source. `related` says the two belong together,
    // which is a claim the listing does not hide.
    let relation = &outcome.relations[0];
    store
        .judge_relation(JudgeRelationParams {
            judgment_id: relation.sync_id.clone(),
            relation: RELATION_RELATED.to_owned(),
            marked_by_actor: "agent".to_owned(),
            marked_by_kind: "agent".to_owned(),
            ..JudgeRelationParams::default()
        })
        .unwrap();
    assert!(
        search(&store, "pool leaked")
            .iter()
            .any(|result| result.observation.id == older.id),
        "reversing the relation brings the memory back"
    );

    // Removing the row is the other reversal, and it restores the other source.
    let relation = &outcome.relations[1];
    store
        .connection
        .execute(
            "DELETE FROM memory_relations WHERE sync_id = ?1",
            rusqlite::params![relation.sync_id],
        )
        .unwrap();
    assert!(
        search(&store, "pool was small")
            .iter()
            .any(|result| result.observation.id == other.id),
        "deleting the relation brings the memory back"
    );
}

#[test]
fn deleting_the_memory_that_replaced_two_others_brings_them_back() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let older = store
        .add_observation(observation(
            "s1",
            "The pool leaked",
            "the pool was never returned to the caller",
        ))
        .unwrap()
        .observation;
    let other = store
        .add_observation(observation(
            "s1",
            "The pool was small",
            "a small pool, sized by hand",
        ))
        .unwrap()
        .observation;
    let outcome = store
        .consolidate_observations(a_merge(vec![older.id, other.id], "leteo"))
        .unwrap();

    let found = |store: &Store, query: &str, id: i64| {
        store
            .search(
                query,
                SearchOptions {
                    project: Some("leteo".to_owned()),
                    ..SearchOptions::default()
                },
            )
            .unwrap()
            .iter()
            .any(|result| result.observation.id == id)
    };
    assert!(
        !found(&store, "pool leaked", older.id) && !found(&store, "pool was small", other.id),
        "while the replacement is live the sources stay hidden"
    );

    store
        .delete_observation(outcome.observation.id, None, false)
        .unwrap();
    assert_eq!(
        count(
            &store,
            "SELECT COUNT(*) FROM memory_relations WHERE judgment_status = 'judged' \
             AND relation = 'supersedes'"
        ),
        2,
        "the verdicts survive a soft delete, which is why the listing has to ask"
    );
    assert!(
        found(&store, "pool leaked", older.id) && found(&store, "pool was small", other.id),
        "a source is hidden only while the memory that replaced it is live"
    );
    let recent = store.recent_memories(Some("leteo"), None, 50).unwrap();
    assert!(
        recent.iter().any(|observation| observation.id == older.id)
            && recent.iter().any(|observation| observation.id == other.id),
        "the context lists them again as well: {recent:?}"
    );
}

#[test]
fn a_verdict_whose_source_row_is_gone_hides_nothing() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let (newer, older) = {
        let newer = store
            .add_observation(observation("s1", "The new way", "the new body"))
            .unwrap()
            .observation;
        let older = store
            .add_observation(observation("s1", "The old way", "the old body"))
            .unwrap()
            .observation;
        (newer, older)
    };
    let outcome = store
        .consolidate_observations(a_merge(vec![older.id, newer.id], "leteo"))
        .unwrap();
    store
        .connection
        .execute(
            "DELETE FROM observations WHERE sync_id = ?1",
            rusqlite::params![outcome.observation.sync_id],
        )
        .unwrap();

    let recent = store.recent_memories(Some("leteo"), None, 50).unwrap();
    assert!(
        recent.iter().any(|observation| observation.id == older.id),
        "a source that no longer exists cannot hide its target: {recent:?}"
    );
}

#[test]
fn an_unjudged_supersedes_relation_hides_nothing() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let older = store
        .add_observation(observation("s1", "The pool leaked", "the old body"))
        .unwrap()
        .observation;
    let newer = store
        .add_observation(observation("s1", "The pool was fixed", "the new body"))
        .unwrap()
        .observation;

    let relation = store
        .save_relation(SaveRelationParams {
            sync_id: normalize::sync_id("rel"),
            source_id: newer.sync_id.clone(),
            target_id: older.sync_id.clone(),
        })
        .unwrap();
    // The verb is set directly, because `save_relation` writes it as `pending`
    // alongside the status. Left that way this test would only ever ask whether
    // a relation with no superseding verb is hidden — which passes with the
    // status clause deleted, and did, until the break was run. A pending verdict
    // that already names `supersedes` is the shape the clause exists for.
    store
        .connection
        .execute(
            "UPDATE memory_relations SET relation = ?1 WHERE sync_id = ?2",
            rusqlite::params![RELATION_SUPERSEDES, relation.sync_id],
        )
        .unwrap();

    let found = store
        .search(
            "pool leaked",
            SearchOptions {
                project: Some("leteo".to_owned()),
                ..SearchOptions::default()
            },
        )
        .unwrap();
    assert!(
        found.iter().any(|result| result.observation.id == older.id),
        "a pending pair is a guess and hides nothing: {found:?}"
    );
}

#[test]
fn a_consolidation_refuses_an_empty_or_repeated_source_list() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let first = store
        .add_observation(observation("s1", "First duplicate", "the first body"))
        .unwrap()
        .observation;

    let empty = store
        .consolidate_observations(a_merge(Vec::new(), "leteo"))
        .unwrap_err();
    assert!(
        matches!(empty, StoreError::ConsolidationSources { .. }),
        "a merge of nothing is refused by name: {empty}"
    );

    let repeated = store
        .consolidate_observations(a_merge(vec![first.id, first.id], "leteo"))
        .unwrap_err();
    assert!(
        matches!(repeated, StoreError::ConsolidationSources { .. }),
        "one source twice would record two relations to one memory: {repeated}"
    );

    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM observations"),
        1,
        "and neither refusal wrote a replacement"
    );
    assert_eq!(count(&store, "SELECT COUNT(*) FROM memory_relations"), 0);
}
