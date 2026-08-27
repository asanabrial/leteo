//! Two stores kept in step.

use super::*;

#[test]
fn a_session_that_already_exists_queues_nothing_to_replicate() {
    // `create_session` is an ensure: it inserts or ignores, and the ordinary
    // caller is every `mem_save` that names no session — they all land in one
    // stable per-project session. The insert did nothing on the second call and
    // every call queued a mutation regardless, so the journal collected one
    // identical copy of that session per memory saved.
    //
    // Measured on a real store before this was fixed: 4,954 session rows for
    // 460 distinct sessions, one of them repeated 657 times, in a journal of
    // 15 MB against 11 MB of actual memories.
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let after_the_first = store.pending_sync_mutation_count("cloud").unwrap();
    assert_eq!(
        after_the_first, 1,
        "the session that was created is owed one"
    );

    for _ in 0..5 {
        store.create_session("s1", "leteo", "C:/repo").unwrap();
    }
    assert_eq!(
        store.pending_sync_mutation_count("cloud").unwrap(),
        after_the_first,
        "a row nothing changed about was queued to be replicated"
    );

    // Ending it is a real change and is still owed a mutation, or a peer would
    // never learn the session had closed.
    store.end_session("s1", Some("what it was for")).unwrap();
    assert_eq!(
        store.pending_sync_mutation_count("cloud").unwrap(),
        after_the_first + 1
    );
}

#[test]
fn replication_cannot_put_back_a_synonym_the_migration_removed() {
    let (_temp, mut store) = store();
    store.create_session("s1", "Leteo", "C:/repo").unwrap();
    let saved = store
        .add_observation(observation("s1", "Fixed the leak", "the pool leaked"))
        .unwrap()
        .observation;

    // What a peer still on the older schema sends: the same memory, typed
    // the way that peer types it.
    let mutation = SyncMutation {
        seq: 7,
        target_key: "cloud".to_owned(),
        entity: "observation".to_owned(),
        entity_key: saved.sync_id.clone(),
        op: crate::sync::OP_UPSERT.to_owned(),
        payload: serde_json::json!({
            "id": saved.id,
            "sync_id": saved.sync_id,
            "session_id": "s1",
            "type": "bug",
            "title": "Fixed the leak",
            "content": "the pool leaked, again",
            "project": "leteo",
            "scope": "project",
            "created_at": saved.created_at,
            "updated_at": saved.updated_at,
        })
        .to_string(),
        source: "remote".to_owned(),
        project: "leteo".to_owned(),
        occurred_at: saved.updated_at.clone(),
        acked_at: None,
    };
    assert!(
        store
            .apply_pulled_sync_mutation("cloud", &mutation)
            .unwrap()
    );

    let stored = store.get_observation(saved.id).unwrap();
    assert_eq!(stored.content, "the pool leaked, again", "the pull landed");
    assert_eq!(
        stored.kind, "bugfix",
        "and it did not undo what the migration folded"
    );
}

#[test]
fn opens_earliest_legacy_schema_preserving_data_and_sync_chunks() {
    let (_temp, config) = legacy_database(EARLY_ENGRAM_SCHEMA);
    let store = Store::open(config.clone()).unwrap();

    let before_reopen = observation_rows(&store.connection);
    assert_eq!(before_reopen.len(), 3);
    assert_eq!(
        before_reopen
            .iter()
            .map(|(_, _, content)| content.as_str())
            .collect::<Vec<_>>(),
        [
            "legacy null content",
            "legacy fixed content",
            "legacy duplicate content"
        ]
    );
    assert!(
        before_reopen
            .iter()
            .all(|(id, sync_id, _)| *id > 0 && sync_id.starts_with("obs-"))
    );
    assert_eq!(
        before_reopen
            .iter()
            .map(|(id, _, _)| *id)
            .collect::<BTreeSet<_>>()
            .len(),
        3
    );
    assert!(before_reopen.iter().any(|(id, _, _)| *id == 7));

    let found = store
        .search("legacy duplicate", SearchOptions::default())
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].observation.content, "legacy duplicate content");

    let chunk: (String, String) = store
        .connection
        .query_row(
            "SELECT target_key, imported_at FROM sync_chunks WHERE chunk_id = 'chunk-legacy'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        chunk,
        ("local".to_owned(), "2024-02-01 00:00:00".to_owned())
    );
    let mutation_project: String = store
        .connection
        .query_row(
            "SELECT project FROM sync_mutations WHERE entity_key = 'obs-old'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(mutation_project, "engram");
    let prompt: (String, String) = store
        .connection
        .query_row("SELECT sync_id, project FROM prompts", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert!(prompt.0.starts_with("prompt-"));
    assert_eq!(prompt.1, "");

    drop(store);
    let reopened = Store::open(config).unwrap();
    assert_eq!(observation_rows(&reopened.connection), before_reopen);
    assert_eq!(
        reopened
            .search("legacy fixed", SearchOptions::default())
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn the_index_stems_so_a_plural_finds_its_singular_and_writes_stay_in_sync() {
    let (_temp, mut store) = store();
    store.create_session("s1", "Leteo", "C:/repo").unwrap();
    for (title, content) in [
        ("Ran the evaluations", "the portfolio evaluations regressed"),
        ("Borradas las sesiones", "se borraron las sesiones antiguas"),
    ] {
        store
            .add_observation(observation("s1", title, content))
            .unwrap();
    }

    // Written after the index was built, through the triggers: a rebuild
    // that leaves later writes unindexed would pass every other test here.
    let found = |query: &str| {
        store
            .search(
                query,
                SearchOptions {
                    project: Some("leteo".to_owned()),
                    ..SearchOptions::default()
                },
            )
            .unwrap()
            .len()
    };

    // English, which is what Porter is for.
    assert_eq!(found("evaluation"), 1);
    assert_eq!(found("evaluations"), 1);
    // And Spanish, which it is not — but its first step strips a trailing
    // `s`, so the plural a user actually types still lands.
    assert_eq!(found("sesion"), 1);
    assert_eq!(found("sesiones"), 1);
    assert_eq!(found("borrada"), 1);

    assert_eq!(found("kubernetes"), 0);
}

#[test]
fn acknowledged_journal_rows_are_pruned_once_they_age_out() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    for index in 0..3 {
        store
            .add_observation(observation(
                "s1",
                &format!("Journalled {index}"),
                &format!("body {index}"),
            ))
            .unwrap();
    }
    let queued = store
        .list_pending_sync_mutations("cloud", &["leteo".to_owned()], 100)
        .unwrap();
    assert!(queued.len() >= 3);
    let sequences = queued
        .iter()
        .map(|mutation| mutation.seq)
        .collect::<Vec<_>>();

    // Freshly acknowledged rows stay: a few days of history is useful when
    // debugging a sync problem.
    store.ack_sync_mutation_seqs("cloud", &sequences).unwrap();
    let remaining: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM sync_mutations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(remaining, sequences.len() as i64);
    assert_eq!(store.pending_sync_mutation_count("cloud").unwrap(), 0);

    store
        .connection
        .execute(
            "UPDATE sync_mutations SET acked_at = datetime('now', '-30 days')",
            [],
        )
        .unwrap();
    store
        .add_observation(observation("s1", "Fresh after pruning", "body"))
        .unwrap();
    let fresh = store
        .list_pending_sync_mutations("cloud", &["leteo".to_owned()], 100)
        .unwrap()
        .iter()
        .map(|mutation| mutation.seq)
        .collect::<Vec<_>>();
    store.ack_sync_mutation_seqs("cloud", &fresh).unwrap();

    let remaining: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM sync_mutations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        remaining,
        fresh.len() as i64,
        "only the recently acknowledged rows survive"
    );
    // Sequences never restart, so a pruned journal cannot replay old rows.
    assert!(fresh.iter().all(|seq| *seq > sequences[0]));
}

#[test]
fn pending_mutation_counts_track_a_sync_target() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    store
        .add_observation(observation("s1", "Pending", "pending body"))
        .unwrap();

    // Local writes are journaled for the cloud target; the file-sync target
    // only records what an export actually shipped.
    let pending = store.pending_sync_mutation_count("cloud").unwrap();
    assert!(
        pending >= 2,
        "session and observation are queued: {pending}"
    );
    assert_eq!(
        store
            .pending_sync_mutation_count(LOCAL_SYNC_TARGET)
            .unwrap(),
        0
    );
    assert!(store.pending_sync_mutation_count("  ").is_err());
}

#[test]
fn deferred_rows_are_listed_and_inspected_by_sync_identifier() {
    let (_temp, store) = store();
    store
            .connection
            .execute(
                "INSERT INTO sync_deferred_mutations
                 (sync_id, entity, payload, apply_status, retry_count, first_seen_at, last_attempted_at)
                 VALUES ('rel_1', 'relation', '{\"source_id\":\"obs_1\"}', 'deferred', 2,
                         '2026-01-01 00:00:00', '2026-01-02 00:00:00')",
                [],
            )
            .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO sync_deferred_mutations
                 (sync_id, entity, payload, apply_status, retry_count, first_seen_at)
                 VALUES ('rel_2', 'relation', 'not json', 'dead', 9, '2026-01-03 00:00:00')",
            [],
        )
        .unwrap();

    let all = store.list_deferred(ListDeferredOptions::default()).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].sync_id, "rel_1");
    assert!(all[0].payload_valid);
    assert!(!all[1].payload_valid);

    let dead = store
        .list_deferred(ListDeferredOptions {
            status: Some("dead".to_owned()),
            ..ListDeferredOptions::default()
        })
        .unwrap();
    assert_eq!(dead.len(), 1);
    assert_eq!(dead[0].sync_id, "rel_2");
    assert_eq!(dead[0].retry_count, 9);

    let row = store.get_deferred("rel_1").unwrap();
    assert_eq!(row.entity, "relation");
    assert_eq!(
        row.last_attempted_at.as_deref(),
        Some("2026-01-02 00:00:00")
    );
    assert!(store.get_deferred("missing").is_err());
}

#[test]
fn cloud_sync_lease_is_exclusive_and_owner_guarded() {
    let (_temp, mut store) = store();
    let now = chrono::DateTime::parse_from_rfc3339("2026-07-27T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);

    assert!(
        store
            .acquire_sync_lease("cloud", "worker-a", Duration::from_secs(60), now)
            .unwrap()
    );
    assert!(
        !store
            .acquire_sync_lease(
                "cloud",
                "worker-b",
                Duration::from_secs(60),
                now + chrono::Duration::seconds(30),
            )
            .unwrap()
    );
    assert!(
        store
            .acquire_sync_lease(
                "cloud",
                "worker-b",
                Duration::from_secs(60),
                now + chrono::Duration::seconds(61),
            )
            .unwrap()
    );
    store.release_sync_lease("cloud", "worker-a").unwrap();
    assert_eq!(
        store
            .get_sync_state("cloud")
            .unwrap()
            .lease_owner
            .as_deref(),
        Some("worker-b")
    );
    store.release_sync_lease("cloud", "worker-b").unwrap();
    assert!(store.get_sync_state("cloud").unwrap().lease_owner.is_none());
}

#[test]
fn cloud_sync_pending_filter_and_exact_ack_preserve_unaccepted_rows() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    store
        .add_observation(observation("s1", "Cloud sync", "pending payload"))
        .unwrap();

    assert!(
        store
            .list_pending_sync_mutations("cloud", &["other".to_owned()], 100)
            .unwrap()
            .is_empty()
    );
    let pending = store
        .list_pending_sync_mutations("cloud", &["leteo".to_owned()], 100)
        .unwrap();
    assert!(pending.len() >= 2);
    store
        .ack_sync_mutation_seqs("cloud", &[pending[0].seq])
        .unwrap();
    let remaining = store
        .list_pending_sync_mutations("cloud", &["leteo".to_owned()], 100)
        .unwrap();
    assert_eq!(remaining.len(), pending.len() - 1);
    assert!(remaining.iter().all(|item| item.seq != pending[0].seq));
    assert!(remaining.iter().any(|item| item.seq == pending[1].seq));
}

#[test]
fn pulled_mutation_applies_once_and_advances_cursor_atomically() {
    let (_temp, mut store) = store();
    let mutation = SyncMutation {
        seq: 42,
        target_key: "cloud".to_owned(),
        entity: "session".to_owned(),
        entity_key: "remote-session".to_owned(),
        op: crate::sync::OP_UPSERT.to_owned(),
        payload: serde_json::json!({
            "id": "remote-session",
            "project": "leteo",
            "directory": "C:/remote",
            "started_at": "2026-07-27T12:00:00Z"
        })
        .to_string(),
        source: "remote".to_owned(),
        project: "leteo".to_owned(),
        occurred_at: "2026-07-27T12:00:00Z".to_owned(),
        acked_at: None,
    };

    assert!(
        store
            .apply_pulled_sync_mutation("cloud", &mutation)
            .unwrap()
    );
    assert!(
        !store
            .apply_pulled_sync_mutation("cloud", &mutation)
            .unwrap()
    );
    assert_eq!(
        store.get_session("remote-session").unwrap().project,
        "leteo"
    );
    assert_eq!(store.get_sync_state("cloud").unwrap().last_pulled_seq, 42);
}

#[test]
fn a_backfilled_project_arrives_at_a_peer_in_an_order_it_can_apply() {
    // Enrolling a project that already holds memories is the ordinary first
    // run of cloud sync, and it is the only path that queues a session and its
    // observations in one go. The backfill used to queue every observation
    // before any session, so the first memory a peer tried to apply failed
    // with `FOREIGN KEY constraint failed` — and so did every one after it.
    // The pull errors, backs off, refetches the same page and fails
    // identically, so the sync never recovers.
    // Nothing enrolled to begin with, because enrolment is what triggers the
    // backfill and a store already enrolled journals as it goes instead.
    let (_temp, mut store) = bare_store();
    store.create_session("s1", "leteo", "/tmp/leteo").unwrap();
    for n in 0..3 {
        store
            .add_observation(observation("s1", &format!("memory {n}"), "body"))
            .unwrap();
    }
    store
        .add_prompt(AddPrompt {
            session_id: "s1".to_owned(),
            content: "a question that was asked".to_owned(),
            project: Some("leteo".to_owned()),
        })
        .unwrap();

    // Everything above predates enrolment, so all of it is backfilled at once.
    assert!(store.enroll_project("leteo").unwrap());
    let pending = store
        .list_pending_sync_mutations("cloud", &["leteo".to_owned()], 100)
        .unwrap();
    assert!(pending.len() >= 5, "{pending:?}");

    // The session comes before anything that points at it. Stated as the rule
    // rather than as one expected order, because what matters is the
    // dependency, not which of observations or prompts follows first.
    let first_session = pending.iter().position(|m| m.entity == "session");
    let first_dependent = pending
        .iter()
        .position(|m| m.entity == "observation" || m.entity == "prompt");
    assert!(
        first_session < first_dependent,
        "the session has to be queued before the rows that reference it: {:?}",
        pending
            .iter()
            .map(|m| (m.seq, m.entity.as_str()))
            .collect::<Vec<_>>()
    );

    // And the proof that this is about more than tidiness: a peer applies the
    // page in order, and every single one has to land.
    let peer_temp = tempfile::TempDir::new().unwrap();
    let mut peer = Store::open(StoreConfig::new(peer_temp.path().join("peer.db"))).unwrap();
    for mutation in &pending {
        peer.apply_pulled_sync_mutation("cloud", mutation)
            .unwrap_or_else(|error| {
                panic!(
                    "seq {} ({}) could not be applied at the peer: {error}",
                    mutation.seq, mutation.entity
                )
            });
    }
    assert_eq!(
        peer.recent_observations(Some("leteo"), Some(10), true)
            .unwrap()
            .len(),
        3,
        "every backfilled memory reaches the peer"
    );
}

#[test]
fn a_peer_cannot_send_a_memory_that_belongs_to_nobody() {
    // `sync_id` is how a memory is recognised again — by the dedupe, by a
    // later update, by the next peer to receive it — and `session_id` ties it
    // to the conversation it came from. A row arriving without either is not a
    // memory, it is a hole: nothing can match it, nothing can revise it, and
    // every other row missing the same field looks like the same memory to
    // anything comparing them.
    //
    // Identity may come from **either** side, which is the part worth pinning:
    // `apply_sync_mutation_tx` fills an empty payload `sync_id` from the
    // mutation's `entity_key`, because the key is the identity and the payload
    // echoing it is redundant. So the row that must be refused is the one with
    // neither, and a payload that carries only the key has to still land.
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();

    // Distinct, increasing sequences: `apply_pulled_sync_mutation` skips
    // anything at or behind the cursor, so reusing one would silently drop the
    // second success rather than apply it.
    let mut next_seq = 0;
    let mut mutation = |entity: &str, key: &str, payload: serde_json::Value| {
        next_seq += 1;
        SyncMutation {
            seq: next_seq,
            target_key: "cloud".to_owned(),
            entity: entity.to_owned(),
            entity_key: key.to_owned(),
            op: crate::sync::OP_UPSERT.to_owned(),
            payload: payload.to_string(),
            source: "remote".to_owned(),
            project: "leteo".to_owned(),
            occurred_at: "2026-08-02T10:00:00Z".to_owned(),
            acked_at: None,
        }
    };
    let observation = |sync_id: &str, session: &str| {
        serde_json::json!({
            "sync_id": sync_id, "session_id": session, "type": "decision",
            "title": "a title", "content": "a body",
            "project": "leteo", "scope": "project",
        })
    };
    let prompt = |sync_id: &str, session: &str| {
        serde_json::json!({
            "sync_id": sync_id, "session_id": session,
            "content": "a question", "project": "leteo",
        })
    };

    // Neither the payload nor the key names it, or it belongs to no session.
    for (entity, key, body) in [
        ("observation", "", observation("", "s1")),
        ("observation", "   ", observation("   ", "s1")),
        ("observation", "obs-1", observation("obs-1", "")),
        ("prompt", "", prompt("", "s1")),
        ("prompt", "   ", prompt("   ", "s1")),
        ("prompt", "prompt-1", prompt("prompt-1", "")),
    ] {
        assert!(
            store
                .apply_pulled_sync_mutation("cloud", &mutation(entity, key, body.clone()))
                .is_err(),
            "a {entity} with no identity was written: key={key:?} {body}"
        );
    }

    assert!(
        store
            .apply_pulled_sync_mutation(
                "cloud",
                &mutation("observation", "obs-from-key", observation("", "s1"))
            )
            .is_ok(),
        "the mutation key is an identity even when the payload omits one"
    );
    assert!(
        store
            .apply_pulled_sync_mutation(
                "cloud",
                &mutation("observation", "obs-ok", observation("obs-ok", "s1"))
            )
            .is_ok()
    );
    assert_eq!(
        store
            .recent_observations(Some("leteo"), Some(10), true)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn enrolling_a_project_again_does_not_send_its_history_twice() {
    // Enrolling queues everything the project already holds, "as though it had
    // just been saved" — that is what lets the journal skip a project nobody
    // replicates without losing history. The same sentence makes whatever is
    // already queued for that project redundant: the backfill covers every row
    // it covered, in its current state.
    //
    // Left there, a project enrolled a second time sends its stale journal
    // first and then a full copy of itself. A real store carries 9,527 such
    // mutations across six unenrolled projects — 19 MB, more than every memory
    // in it — and every one would go over the wire before the backfill that
    // supersedes it.
    let (_temp, mut store) = bare_store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    store.enroll_project("leteo").unwrap();
    for index in 0..3 {
        store
            .add_observation(observation(
                "s1",
                &format!("Journalled while enrolled {index}"),
                "body",
            ))
            .unwrap();
    }
    let while_enrolled = store.pending_sync_mutation_count("cloud").unwrap();
    assert!(while_enrolled >= 3, "{while_enrolled}");

    // Off, and more work happens that nothing journals.
    store.unenroll_project("leteo").unwrap();
    store
        .add_observation(observation("s1", "Saved while nobody replicated", "body"))
        .unwrap();
    assert_eq!(
        store.pending_sync_mutation_count("cloud").unwrap(),
        while_enrolled,
        "an unenrolled project must journal nothing"
    );

    // On again: the backfill is the whole truth, so what it supersedes goes.
    store.enroll_project("leteo").unwrap();
    let after = store.pending_sync_mutation_count("cloud").unwrap();
    let rows: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM sync_mutations WHERE entity = 'observation' AND acked_at IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        rows, 4,
        "one queued mutation per memory, not one per memory per enrolment"
    );
    assert!(after < while_enrolled + 4 + 4, "{after} is two copies");
}

/// A journal nobody can send is not a backlog.
///
/// On a real store this was 9,527 unacknowledged mutations and 15.2 MB — 29%
/// of a 51.7 MB database — for a cloud that was never configured: no
/// `cloud.json`, nothing enrolled, and 4,954 of the rows sessions queued again
/// on every memory saved. `mem_doctor` reported it as a pending queue, which
/// reads as work waiting to go somewhere.
///
/// They cannot go anywhere. Journalling is gated on enrolment so nothing new
/// joins them, and `enroll_project` deletes a project's unacknowledged
/// mutations and backfills every row it holds — so enrolling discards them too.
/// Whichever way a store goes, they are thrown away.
#[test]
fn the_journal_of_a_project_nobody_replicates_is_dropped_on_upgrade() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("leteo.db");
    {
        let mut store = Store::open(StoreConfig::new(path.clone())).unwrap();
        store.enroll_project("replicado").unwrap();
        store
            .connection
            .execute_batch(
                "INSERT INTO sync_mutations (target_key, entity, entity_key, op, payload, project)
                 VALUES ('cloud','observation','obs-huerfana','upsert','{}','olvidado'),
                        ('cloud','session','ses-huerfana','upsert','{}','olvidado'),
                        ('cloud','observation','obs-enrolada','upsert','{}','replicado'),
                        ('cloud','observation','obs-acusada','upsert','{}','olvidado');
                 UPDATE sync_mutations SET acked_at = datetime('now')
                  WHERE entity_key = 'obs-acusada';
                 PRAGMA user_version = 0;",
            )
            .unwrap();
    }

    let store = Store::open(StoreConfig::new(path)).unwrap();
    let queued = |key: &str| -> i64 {
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM sync_mutations WHERE entity_key = ?1",
                [key],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(queued("obs-huerfana"), 0, "a project nobody replicates");
    assert_eq!(queued("ses-huerfana"), 0, "whatever the entity is");
    assert_eq!(
        queued("obs-enrolada"),
        1,
        "an enrolled project keeps its queue; this is the one that can still be sent"
    );
    assert_eq!(
        queued("obs-acusada"),
        1,
        "an acknowledged row is the record of what a peer already has, and the \
         retention window is what removes it"
    );
}

/// A pin is written down and not sent, and those are two different questions.
///
/// `#[serde(skip)]` on the field answered neither: it kept pinning out of the
/// wire — which is right, and what Engram settled on with a test of its own —
/// and out of `leteo export`, which is not. An export is this store written
/// down, the import statement has always had a column ready for the pin, and a
/// backup that quietly drops what somebody chose to keep in front is a lossy
/// backup.
///
/// Both halves are asserted here, because each alone invites undoing the
/// other: put the pin on the wire and one machine's shelf rearranges every
/// other machine's; take it out of the export and a restore loses it again.
#[test]
fn a_pin_survives_an_export_and_stays_off_the_wire() {
    let (temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let saved = store
        .add_observation(observation("s1", "Una memoria que se ancla", "Cuerpo."))
        .unwrap()
        .observation;
    store.pin_observation(saved.id).unwrap();
    // Edited *after* pinning, which is the only way a payload could carry the
    // pin: pinning itself queues nothing, so without this the wire half of
    // this test passes however the wire behaves — checked by letting the pin
    // through and watching it stay green.
    store
        .update_observation(
            saved.id,
            UpdateObservation {
                content: Some("Cuerpo cambiado después de anclarla.".to_owned()),
                ..Default::default()
            },
        )
        .unwrap();

    // Nothing a peer receives says it is pinned.
    let payloads: Vec<String> = store
        .connection
        .prepare("SELECT payload FROM sync_mutations WHERE entity = 'observation'")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(
        !payloads.is_empty(),
        "the fixture enrols its project, so the save is journalled"
    );
    for payload in &payloads {
        assert!(
            !payload.contains("\"pinned\""),
            "pinning is where this store looks, not what the memory is: {payload}"
        );
    }

    // And the export carries it, into a store that had never heard of it.
    let exported = store.export_json(None).unwrap();
    assert!(
        exported.contains("\"pinned\""),
        "an export is this store written down"
    );

    let elsewhere = TempDir::new().unwrap();
    let mut restored = Store::open(StoreConfig::new(elsewhere.path().join("leteo.db"))).unwrap();
    restored.import_json(&exported).unwrap();
    let there = restored
        .search("ancla", SearchOptions::default())
        .unwrap()
        .into_iter()
        .find(|found| found.observation.title.contains("se ancla"))
        .expect("the memory arrived");
    assert!(
        there.observation.pinned,
        "and it arrived still pinned, which is what the import column was always for"
    );
    drop(temp);
}

/// Enrolling a project sends its judged graph, not only its memories.
///
/// The backfill queued sessions, observations and prompts — and not a single
/// relation. So a project enrolled after any of it had been curated arrived at
/// the peer as bare memories: every *this supersedes that* and every *not a
/// conflict* verdict stayed on the machine that made them. On the other side
/// the superseded decisions read as current, and the same pairs come back up
/// for judgement to be answered a second time.
///
/// The same omission an export had, and was fixed there, one pipe at a time.
#[test]
fn enrolling_a_project_queues_the_verdicts_it_already_holds() {
    let (_temp, mut store) = bare_store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let old = store
        .add_observation(observation("s1", "Decisión vieja", "El método antiguo."))
        .unwrap()
        .observation;
    let new = store
        .add_observation(observation("s1", "Decisión nueva", "El método nuevo."))
        .unwrap()
        .observation;
    let proposed = store
        .save_relation(crate::memory::model::SaveRelationParams {
            sync_id: "rel-antes-de-enrolar".to_owned(),
            source_id: new.sync_id.clone(),
            target_id: old.sync_id.clone(),
        })
        .unwrap();
    store
        .judge_relation(crate::memory::model::JudgeRelationParams {
            judgment_id: proposed.sync_id.clone(),
            relation: "supersedes".to_owned(),
            reason: Some("la nueva manda".to_owned()),
            ..Default::default()
        })
        .unwrap();
    // Nothing is journalled for a project nobody replicates, so all of that is
    // waiting to be caught up rather than already queued.
    assert_eq!(queued_entities(&store), Vec::<String>::new());

    store.enroll_project("leteo").unwrap();

    let queued = queued_entities(&store);
    assert!(
        queued.contains(&"relation".to_owned()),
        "a verdict already made is part of what the project holds: {queued:?}"
    );
    // And last: a relation names two memories by sync_id, and a peer applies
    // mutations in the order they were queued.
    let first_relation = queued.iter().position(|entity| entity == "relation");
    let last_observation = queued.iter().rposition(|entity| entity == "observation");
    assert!(
        first_relation > last_observation,
        "a relation cannot arrive before the memories it names: {queued:?}"
    );
}

/// The entities of the journal, in the order they were queued.
fn queued_entities(store: &Store) -> Vec<String> {
    store
        .connection
        .prepare("SELECT entity FROM sync_mutations ORDER BY seq")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// A verdict that arrives before the memories it names is kept, not dropped.
///
/// A peer sends mutations in the order it queued them, but a store can join
/// mid-stream, and a relation names two memories by `sync_id`. Applied
/// eagerly it would fail; discarded it would be lost, and the pair would be
/// judged again on this side. So it waits in `sync_deferred_mutations` and is
/// replayed after every mutation that lands, which is the moment the second
/// memory could have arrived.
///
/// Only the listing of deferred rows was tested, against rows written by hand.
/// The cycle — deferred, replayed, cleared — was not.
#[test]
fn a_relation_that_arrives_first_waits_for_its_memories() {
    let (_temp, mut store) = bare_store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();

    let relation = SyncMutation {
        seq: 1,
        target_key: "cloud".to_owned(),
        entity: "relation".to_owned(),
        entity_key: "rel-adelantada".to_owned(),
        op: crate::sync::OP_UPSERT.to_owned(),
        payload: serde_json::json!({
            "sync_id": "rel-adelantada",
            "source_id": "obs-nueva",
            "target_id": "obs-vieja",
            "relation": "supersedes",
            "judgment_status": "judged",
            "created_at": "2026-01-01 00:00:00",
        })
        .to_string(),
        source: "remote".to_owned(),
        project: "leteo".to_owned(),
        occurred_at: "2026-01-01 00:00:00".to_owned(),
        acked_at: None,
    };
    assert!(
        store
            .apply_pulled_sync_mutation("cloud", &relation)
            .unwrap()
    );
    assert_eq!(
        store
            .count_relations(ListRelationsOptions::default())
            .unwrap(),
        0,
        "there is nothing to relate yet"
    );
    let waiting = store.get_deferred("rel-adelantada").unwrap();
    assert_eq!(
        waiting.apply_status, "deferred",
        "and it is kept, not dropped"
    );

    // The memories, one at a time: the first is not enough.
    let memory = |seq: i64, sync_id: &str, title: &str| SyncMutation {
        seq,
        target_key: "cloud".to_owned(),
        entity: "observation".to_owned(),
        entity_key: sync_id.to_owned(),
        op: crate::sync::OP_UPSERT.to_owned(),
        payload: serde_json::json!({
            "sync_id": sync_id,
            "session_id": "s1",
            "type": "decision",
            "title": title,
            "content": "Cuerpo.",
            "project": "leteo",
            "scope": "project",
            "created_at": "2026-01-01 00:00:00",
            "updated_at": "2026-01-01 00:00:00",
        })
        .to_string(),
        source: "remote".to_owned(),
        project: "leteo".to_owned(),
        occurred_at: "2026-01-01 00:00:00".to_owned(),
        acked_at: None,
    };
    store
        .apply_pulled_sync_mutation("cloud", &memory(2, "obs-vieja", "Decisión vieja"))
        .unwrap();
    assert_eq!(
        store
            .count_relations(ListRelationsOptions::default())
            .unwrap(),
        0,
        "one of the two is still missing"
    );

    store
        .apply_pulled_sync_mutation("cloud", &memory(3, "obs-nueva", "Decisión nueva"))
        .unwrap();
    assert_eq!(
        store
            .count_relations(ListRelationsOptions::default())
            .unwrap(),
        1,
        "the verdict lands the moment both memories exist"
    );
    assert!(
        store.get_deferred("rel-adelantada").is_err(),
        "and it stops waiting, or it would be replayed on every sync for ever"
    );
}

/// Everything a memory is worth sending survives the trip out and back.
///
/// The two halves of replication are written apart: `enqueue_observation`
/// serialises an `Observation`, and `apply_observation_upsert_tx` reads a
/// `SyncObservationPayload`. Nothing holds their field names together, and the
/// payload is `#[serde(default)]` — so a field renamed on one side arrives as
/// a default on the other, with no error anywhere. Both structs would still
/// compile, both tests of each half would still pass, and memories would
/// quietly cross the wire with a piece missing.
///
/// So this drives the real serialisation and reads it back the way the far end
/// does. Two fields are expected not to survive and both are deliberate:
/// `pinned`, which says where *this* store looks rather than what the memory
/// is, and `review_after`, which no payload carries — a peer recomputes it
/// from the type, which is what `Store::reschedule_review` is for.
#[test]
fn everything_a_memory_carries_is_the_same_on_both_sides_of_the_wire() {
    let original = Observation {
        id: 7,
        sync_id: "obs-round-trip".to_owned(),
        session_id: "s1".to_owned(),
        kind: "bugfix".to_owned(),
        title: "The pool leaked".to_owned(),
        content: "it was never returned on the error path".to_owned(),
        tool_name: Some("Explore".to_owned()),
        project: Some("leteo".to_owned()),
        scope: "project".to_owned(),
        topic_key: Some("bug/pool-leak".to_owned()),
        revision_count: 3,
        duplicate_count: 2,
        last_seen_at: Some("2026-08-05 04:00:00".to_owned()),
        review_after: Some("2027-02-05 04:00:00".to_owned()),
        prompt_sync_id: Some("prompt-abc".to_owned()),
        pinned: true,
        created_at: "2026-08-01 10:00:00".to_owned(),
        updated_at: "2026-08-05 04:00:00".to_owned(),
        deleted_at: None,
    };
    let mut travelling = original.clone();
    // What `enqueue_observation` does before it writes the payload.
    travelling.pinned = false;

    let json = serde_json::to_string(&travelling).unwrap();
    let arrived: crate::store::wire::SyncObservationPayload = serde_json::from_str(&json).unwrap();

    assert_eq!(arrived.sync_id, original.sync_id);
    assert_eq!(arrived.session_id, original.session_id);
    assert_eq!(
        arrived.kind, original.kind,
        "the type is renamed on both sides"
    );
    assert_eq!(arrived.title, original.title);
    assert_eq!(arrived.content, original.content);
    assert_eq!(arrived.tool_name, original.tool_name);
    assert_eq!(arrived.project, original.project);
    assert_eq!(arrived.scope, original.scope);
    assert_eq!(arrived.topic_key, original.topic_key);
    assert_eq!(arrived.prompt_sync_id, original.prompt_sync_id);
    assert_eq!(arrived.revision_count, original.revision_count);
    assert_eq!(arrived.duplicate_count, original.duplicate_count);
    assert_eq!(arrived.last_seen_at, original.last_seen_at);
    assert_eq!(arrived.created_at, original.created_at);
    assert_eq!(arrived.updated_at, original.updated_at);
    assert_eq!(arrived.deleted_at, original.deleted_at);
    assert!(!arrived.deleted, "a live memory does not arrive deleted");
    assert!(!arrived.hard_delete);

    // And the two that do not travel, named so that removing either from the
    // struct is a decision somebody makes rather than an accident.
    let sent: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(
        sent.get("pinned").is_none(),
        "pinning is where this store looks, not what the memory is"
    );
    assert!(
        sent.get("review_after").is_some(),
        "review_after is serialised by the model even though no payload reads \
         it - a peer recomputes it from the type, and if that ever changes this \
         is the line that has to change with it"
    );
}

/// The two ways in agree about what they store, given the same ugly input.
///
/// Three defects of exactly this shape turned up in one day: a replicated
/// prompt kept its `<private>` spans and ignored the length cap, a replicated
/// session kept whatever spelling of the project name it was sent, and a
/// replicated relation took any number as a confidence. Each was found by
/// reading one path and then the other, which is a method that works and does
/// not scale.
///
/// So this states the rule once and checks it: hand both paths the same
/// hostile input and the rows they leave have to match. It is deliberately
/// about the *rules* rather than the field names — the round trip above covers
/// those — because the rules are what drifted.
#[test]
fn a_replicated_write_and_a_typed_one_leave_the_same_row() {
    let (_temp, mut store) = store();
    let ugly_project = "  Leteo--Cloud  ";
    let ugly_content = "¿por qué? <private>ghp_secreto</private> y algo más después";

    store
        .create_session("typed", ugly_project, "C:/repo")
        .unwrap();
    store
        .add_prompt(AddPrompt {
            session_id: "typed".to_owned(),
            content: ugly_content.to_owned(),
            project: Some(ugly_project.to_owned()),
        })
        .unwrap();

    // Sent. Same words, same spelling, arriving from somewhere else.
    let session = SyncMutation {
        seq: 10,
        target_key: "cloud".to_owned(),
        entity: "session".to_owned(),
        entity_key: "sent".to_owned(),
        op: crate::sync::OP_UPSERT.to_owned(),
        payload: serde_json::json!({
            "id": "sent", "project": ugly_project, "directory": "C:/repo",
            "started_at": "2026-08-05 04:00:00",
        })
        .to_string(),
        source: "remote".to_owned(),
        project: "leteo-cloud".to_owned(),
        occurred_at: "2026-08-05 04:00:00".to_owned(),
        acked_at: None,
    };
    let prompt = SyncMutation {
        seq: 11,
        entity: "prompt".to_owned(),
        entity_key: "prompt-sent".to_owned(),
        payload: serde_json::json!({
            "sync_id": "prompt-sent", "session_id": "sent",
            "content": ugly_content, "project": ugly_project,
            "created_at": "2026-08-05 04:00:00",
        })
        .to_string(),
        ..session.clone()
    };
    for mutation in [&session, &prompt] {
        store.apply_pulled_sync_mutation("cloud", mutation).unwrap();
    }

    let project_of_session = |id: &str| store.get_session(id).unwrap().project;
    assert_eq!(
        project_of_session("typed"),
        project_of_session("sent"),
        "a session's project is spelled one way in this store, whoever sent it"
    );

    let stored_prompt = |session_id: &str| -> (String, String) {
        store
            .connection
            .query_row(
                "SELECT content, project FROM prompts WHERE session_id = ?1",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
    };
    assert_eq!(
        stored_prompt("typed"),
        stored_prompt("sent"),
        "and a prompt is redacted and filed the same way, whoever sent it"
    );
}

/// What a peer receives is the memory, minus exactly what is meant to stay home.
///
/// The export guard compares the two memories whole because counting rows is
/// what let pinning disappear. This is that question asked of the other route:
/// the payload a peer actually receives, applied by the code that receives it,
/// compared against what was sent.
///
/// Both ends can lose a field for different reasons. The sending end serialises
/// the model, so a new field travels by itself; the receiving end writes an
/// `INSERT` with a hand-written list of column names, which is the same shape
/// as the import statement that dropped the pin.
#[test]
fn a_memory_crossing_the_wire_arrives_whole_except_what_stays_home() {
    let (_source_temp, mut source) = store();
    source.enroll_project("leteo").unwrap();
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
    input.topic_key = Some("decision/replicada".to_owned());
    input.prompt_sync_id = Some(asked.clone());
    let saved = source.add_observation(input.clone()).unwrap().observation;
    source.add_observation(input).unwrap();
    source.pin_observation(saved.id).unwrap();
    // Something for the session to lose on the way, or comparing two empty
    // summaries proves nothing.
    source
        .end_session("s1", Some("lo que hizo esta sesion"))
        .unwrap();
    let sent = source.get_observation(saved.id).unwrap();
    assert!(
        sent.pinned,
        "the fixture has to pin it or nothing is proven"
    );
    assert_eq!(sent.prompt_sync_id.as_deref(), Some(asked.as_str()));
    assert!(sent.revision_count > 1, "{sent:?}");

    // And a judged relation, the fourth entity that crosses and the fourth
    // hand-written column list at the far end.
    let other = source
        .add_observation(observation("s1", "Otra decision", "que la primera supera"))
        .unwrap()
        .observation;
    let judged = source
        .judge_by_semantic(crate::memory::model::JudgeBySemanticParams {
            source_id: saved.sync_id.clone(),
            target_id: other.sync_id.clone(),
            relation: "supersedes".to_owned(),
            confidence: Some(0.9),
            reasoning: Some("porque la primera la deja atras".to_owned()),
            ..Default::default()
        })
        .unwrap();

    // The payloads the store actually queued, applied by the code that applies
    // them — sessions first, because a memory references one.
    let queued = source
        .list_pending_sync_mutations("cloud", &["*".to_owned()], 100)
        .unwrap();
    let (_destination_temp, mut destination) = store();
    let mut applied = 0;
    for mutation in &queued {
        if destination
            .apply_pulled_sync_mutation("cloud", mutation)
            .unwrap_or(false)
        {
            applied += 1;
        }
    }
    assert!(
        applied >= 3,
        "only {applied} of {} mutations applied",
        queued.len()
    );

    let arrived = destination
        .search("decision", SearchOptions::default())
        .unwrap()
        .into_iter()
        .map(|result| result.observation)
        .find(|observation| observation.sync_id == sent.sync_id)
        .expect("the memory did not cross");

    // Pinning is where this store looks, not what the memory is, so it stays
    // home on purpose — see `enqueue_observation`.
    assert!(
        !arrived.pinned,
        "a pin should not rearrange another machine"
    );
    // Everything else is the memory and has to arrive.
    assert_eq!(
        crate::memory::model::Observation {
            id: 0,
            pinned: false,
            ..sent.clone()
        },
        crate::memory::model::Observation {
            id: 0,
            pinned: false,
            ..arrived
        },
        "a field went missing between one store and the next"
    );

    // The session and the prompt crossed in the same batch and have their own
    // hand-written column lists at the far end, so they are compared too. A
    // memory that arrives whole into a session that lost its summary is still
    // a lossy replication.
    let session_here = source.get_session("s1").unwrap();
    let session_there = destination.get_session("s1").unwrap();
    assert_eq!(
        session_here, session_there,
        "the session did not cross whole"
    );
    assert!(
        session_here.summary.is_some() || session_here.ended_at.is_some(),
        "the fixture has to give the session something to lose: {session_here:?}"
    );

    let prompt_here = source.recent_prompts(Some("leteo"), Some(1)).unwrap();
    let prompt_there = destination.recent_prompts(Some("leteo"), Some(1)).unwrap();
    assert_eq!(prompt_here.len(), 1, "the fixture saved one prompt");
    assert_eq!(
        crate::memory::model::Prompt {
            id: 0,
            ..prompt_here[0].clone()
        },
        crate::memory::model::Prompt {
            id: 0,
            ..prompt_there
                .first()
                .cloned()
                .expect("the prompt did not cross")
        },
        "the prompt did not cross whole"
    );

    let relation_here = source.get_relation(&judged).unwrap();
    let relation_there = destination.get_relation(&judged).unwrap();
    assert_eq!(
        crate::memory::model::Relation {
            id: 0,
            ..relation_here.clone()
        },
        crate::memory::model::Relation {
            id: 0,
            ..relation_there
        },
        "the relation did not cross whole"
    );
    assert_eq!(relation_here.relation, "supersedes");
    assert!(relation_here.confidence.is_some(), "{relation_here:?}");
    assert!(relation_here.reason.is_some(), "{relation_here:?}");
}

/// A decision is due six months after it was decided, on every machine.
///
/// The review clock was counted from `now()`, which is the same thing as the
/// memory's own date for a local save and a different thing entirely for one
/// arriving over the wire — the path the function that sets it was written for.
/// The payload carries no `review_after`, so the receiving store started the
/// window again from the moment it heard about the memory.
///
/// Found by a guard comparing two stores, where the two dates differed by one
/// second and would have gone on differing by one second only for as long as
/// both ran inside the same one.
#[test]
fn a_review_clock_is_counted_from_the_memory_rather_than_from_the_hearing() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let mut input = observation("s1", "Una decision antigua", "tomada hace mucho");
    input.kind = "decision".to_owned();
    let saved = store.add_observation(input).unwrap().observation;

    // Backdated by five months, as a memory replicated late would be, and the
    // clock recomputed the way the wire recomputes it.
    store
        .connection()
        .execute(
            "UPDATE observations SET created_at = datetime('now', '-5 months'), review_after = NULL
             WHERE id = ?1",
            rusqlite::params![saved.id],
        )
        .unwrap();
    let tx = store.connection().unchecked_transaction().unwrap();
    crate::store::observations::reschedule_review(&tx, saved.id, "decision", None).unwrap();
    tx.commit().unwrap();

    let due: String = store
        .connection()
        .query_row(
            "SELECT review_after FROM observations WHERE id = ?1",
            rusqlite::params![saved.id],
            |row| row.get(0),
        )
        .unwrap();
    let due = crate::timestamp::parse(&due).expect("a decision carries a clock");
    let now = chrono::Utc::now().naive_utc();
    let months = (due - now).num_days() / 30;
    assert!(
        (0..=2).contains(&months),
        "a decision five months old is due in about one month, not {months}: {due}"
    );
}

#[test]
fn a_waiting_queue_says_since_when_and_not_only_how_many() {
    // `cloud status` answered with a count, and a count cannot tell a busy peer
    // from a dead one. The queue drains on nothing but an acknowledgement — the
    // prune deletes rows that were acked and have aged out — so an unreachable
    // peer keeps every row it ever took, and "pending: 100" reads the same on
    // the first morning and in the third month.
    let (_temp, mut store) = store();
    store.enroll_project("leteo").unwrap();

    // Before any write at all: enrolling names a project, it does not enqueue
    // anything, so there is no date because there is no queue.
    assert_eq!(
        store
            .oldest_pending_mutation(crate::cloud::CLOUD_SYNC_TARGET)
            .unwrap(),
        None,
        "nothing waiting is not a date"
    );

    // A session is itself a replicated write, which is why this comes after.
    store.create_session("s1", "leteo", "C:/repo").unwrap();

    for index in 0..3 {
        store
            .add_observation(observation(
                "s1",
                &format!("Memory {index}"),
                "a body worth replicating",
            ))
            .unwrap();
    }
    let pending = store
        .pending_sync_mutation_count(crate::cloud::CLOUD_SYNC_TARGET)
        .unwrap();
    assert!(pending >= 3, "three writes are three mutations: {pending}");

    // Backdated by hand, because the answer has to be the *oldest* one rather
    // than whichever row is convenient to read. Two months apart, so a query
    // that returned the newest, or the first inserted, or any single row, gives
    // a different answer from this one.
    store
        .connection()
        .execute(
            "UPDATE sync_mutations SET occurred_at = datetime('now', '-60 days')
             WHERE seq = (SELECT MIN(seq) FROM sync_mutations WHERE acked_at IS NULL)",
            [],
        )
        .unwrap();
    store
        .connection()
        .execute(
            "UPDATE sync_mutations SET occurred_at = datetime('now', '-1 day')
             WHERE seq = (SELECT MAX(seq) FROM sync_mutations WHERE acked_at IS NULL)",
            [],
        )
        .unwrap();
    let since = store
        .oldest_pending_mutation(crate::cloud::CLOUD_SYNC_TARGET)
        .unwrap()
        .expect("something is waiting, so there is a date");
    let sixty_days_ago =
        crate::timestamp::format(chrono::Utc::now().naive_utc() - chrono::Duration::days(60));
    assert_eq!(
        since[..10],
        sixty_days_ago[..10],
        "the age of the queue is its oldest row, not its newest: {since}"
    );

    // And an acknowledged row stops counting, which is the only way this
    // number ever goes down.
    store
        .connection()
        .execute(
            "UPDATE sync_mutations SET acked_at = datetime('now')
             WHERE occurred_at < datetime('now', '-30 days')",
            [],
        )
        .unwrap();
    let since = store
        .oldest_pending_mutation(crate::cloud::CLOUD_SYNC_TARGET)
        .unwrap()
        .expect("two are still waiting");
    assert_ne!(
        since[..10],
        sixty_days_ago[..10],
        "an acked row is not a waiting one"
    );
}

/// A reread somebody did is not undone by the next thing a peer sends.
///
/// The clock is derived and never travels: no payload carries `review_after`,
/// and the receiving store works it out from the memory's own date and type,
/// which is what makes both machines agree that a decision made in January is
/// due in July. `mark_reviewed` is the one act that moves it away from that
/// derivation — "I read this today, ask me in six months" — and it is local,
/// because there is nothing on the wire to carry it.
///
/// Local is fine. Local *and undone* is not, and that is the difference between
/// this and a pin: a pin survives because nothing overwrites it, while a clock
/// is recomputed on every arrival. Without the one condition that holds this,
/// an old decision somebody reread would fall due again the moment any peer
/// touched that memory, and again the time after that.
///
/// The suite had nothing on it: removing that condition broke no test.
#[test]
fn an_arriving_memory_does_not_undo_a_reread() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let mut input = observation("s1", "Una decision antigua", "tomada hace mucho tiempo");
    input.kind = "decision".to_owned();
    let saved = store.add_observation(input).unwrap().observation;

    // Old enough that the derived clock is in the past: this is the memory the
    // queue would keep offering.
    store
        .connection()
        .execute(
            "UPDATE observations SET created_at = datetime('now', '-13 months') WHERE id = ?1",
            rusqlite::params![saved.id],
        )
        .unwrap();
    // Read back, because the arriving payload carries `created_at` and a stale
    // copy of it would put the memory's date back to today — which makes the
    // recomputation land on almost the same answer and the assertion below
    // prove nothing. It did: with the original date in the payload, removing
    // the protection this guards changed the clock by under a second.
    let created_at = store.get_observation(saved.id).unwrap().created_at;
    store.mark_reviewed(saved.id).unwrap();
    let after_reading = store
        .get_observation(saved.id)
        .unwrap()
        .review_after
        .expect("a decision reread today is due again later");
    assert_eq!(
        store.count_review_due(Some("leteo")).unwrap(),
        0,
        "nothing is due, because it was just read"
    );

    let arriving = |kind: &str, seq: i64, updated_at: &str| SyncMutation {
        seq,
        target_key: "cloud".to_owned(),
        entity: "observation".to_owned(),
        entity_key: saved.sync_id.clone(),
        op: crate::sync::OP_UPSERT.to_owned(),
        payload: serde_json::json!({
            "id": saved.id,
            "sync_id": saved.sync_id,
            "session_id": "s1",
            "type": kind,
            "title": "Una decision antigua",
            "content": "tomada hace mucho tiempo, y editada en el otro lado",
            "project": "leteo",
            "scope": "project",
            "created_at": created_at,
            "updated_at": updated_at,
        })
        .to_string(),
        source: "remote".to_owned(),
        project: "leteo".to_owned(),
        occurred_at: updated_at.to_owned(),
        acked_at: None,
    };

    assert!(
        store
            .apply_pulled_sync_mutation("cloud", &arriving("decision", 1, "2027-01-01 00:00:00"))
            .unwrap()
    );
    let stored = store.get_observation(saved.id).unwrap();
    assert!(
        stored.content.contains("editada en el otro lado"),
        "the edit landed, so this is about what came with it"
    );
    assert_eq!(
        stored.review_after.as_deref(),
        Some(after_reading.as_str()),
        "an arriving edit must not wind back a clock somebody moved"
    );
    assert_eq!(
        store.count_review_due(Some("leteo")).unwrap(),
        0,
        "and the queue does not offer it again"
    );

    // The positive control, without which "never recompute" would pass: a type
    // that changes on the other side is a different window, and that one is
    // worked out afresh.
    assert!(
        store
            .apply_pulled_sync_mutation("cloud", &arriving("policy", 2, "2027-01-02 00:00:00"))
            .unwrap()
    );
    let after_kind_change = store
        .get_observation(saved.id)
        .unwrap()
        .review_after
        .expect("a policy has a window too");
    assert_ne!(
        after_kind_change, after_reading,
        "a different type is a different window, and is recomputed"
    );
}

/// A memory that leaves a replicated project tells the peer it is gone.
///
/// The queue writes under the project a row is in *now*, and drops anything
/// whose project nobody replicates — so moving a memory out of an enrolled
/// project into an unenrolled one queued nothing at all, and the peer went on
/// holding it under the old name, with the old body, for ever. Nothing said so.
///
/// That is the silence `merge_projects` was fixed for, and it needs a different
/// answer here. There the canonical project takes over the source's enrolment,
/// because the memories are the same set under a new name; here enrolling the
/// destination would start replicating a project nobody asked to replicate. So
/// what travels is the only thing true from where the peer stands: it is gone
/// from the project you are watching.
#[test]
fn a_memory_that_leaves_a_replicated_project_is_deleted_at_the_peer() {
    let (_temp, mut store) = super::store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    store.create_session("s2", "otro", "C:/repo").unwrap();
    store
        .connection()
        .execute(
            "INSERT OR IGNORE INTO sync_enrolled_projects (project) VALUES ('leteo')",
            [],
        )
        .unwrap();
    let saved = store
        .add_observation(crate::memory::model::AddObservation {
            session_id: "s1".to_owned(),
            kind: "decision".to_owned(),
            title: "Una memoria replicada que se muda".to_owned(),
            content: "Un cuerpo cualquiera.".to_owned(),
            tool_name: None,
            project: Some("leteo".to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            prompt_sync_id: None,
        })
        .unwrap()
        .observation;
    let cola = |store: &Store| -> Vec<(String, String)> {
        store
            .connection()
            .prepare(
                "SELECT op, project FROM sync_mutations
                  WHERE entity = 'observation' ORDER BY seq",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(
        cola(&store),
        vec![("upsert".to_owned(), "leteo".to_owned())],
        "guardada en un proyecto inscrito, viaja"
    );

    // Out, into a project nobody replicates: the peer has to be told.
    store
        .update_observation(
            saved.id,
            crate::memory::model::UpdateObservation {
                project: Some("otro".to_owned()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        cola(&store).last().cloned(),
        Some(("delete".to_owned(), "leteo".to_owned())),
        "se le dice al peer que ya no está donde miraba: {:?}",
        cola(&store)
    );

    let antes = cola(&store).len();
    store
        .update_observation(
            saved.id,
            crate::memory::model::UpdateObservation {
                title: Some("Editada estando fuera".to_owned()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(cola(&store).len(), antes, "fuera no se replica nada");

    store
        .connection()
        .execute(
            "INSERT OR IGNORE INTO sync_enrolled_projects (project) VALUES ('otro')",
            [],
        )
        .unwrap();
    store
        .update_observation(
            saved.id,
            crate::memory::model::UpdateObservation {
                project: Some("leteo".to_owned()),
                ..Default::default()
            },
        )
        .unwrap();
    let ultima = cola(&store).last().cloned();
    assert_eq!(
        ultima,
        Some(("upsert".to_owned(), "leteo".to_owned())),
        "entre inscritos viaja la fila, no una lápida: {:?}",
        cola(&store)
    );
}

/// And the peer can apply what it is sent, which the queue's shape does not say.
///
/// The guard above asserts what goes into the journal. A payload built by hand
/// beside the one the deletion path builds is exactly the kind that passes that
/// assertion and then fails at the far end, where a missing field defers the
/// mutation for ever and the memory the peer was told to drop stays.
///
/// So both mutations are carried across to a second store and applied by the
/// code that applies them: the upsert puts the memory there, and the tombstone
/// takes it away.
#[test]
fn the_tombstone_a_move_queues_is_one_a_peer_can_apply() {
    let (_origen_temp, mut origen) = super::store();
    origen.create_session("s1", "leteo", "C:/repo").unwrap();
    origen.create_session("s2", "otro", "C:/repo").unwrap();
    origen
        .connection()
        .execute(
            "INSERT OR IGNORE INTO sync_enrolled_projects (project) VALUES ('leteo')",
            [],
        )
        .unwrap();
    let saved = origen
        .add_observation(observation(
            "s1",
            "Una memoria que se muda de proyecto",
            "un cuerpo cualquiera",
        ))
        .unwrap()
        .observation;
    origen
        .update_observation(
            saved.id,
            crate::memory::model::UpdateObservation {
                project: Some("otro".to_owned()),
                ..Default::default()
            },
        )
        .unwrap();

    let mutaciones: Vec<crate::memory::model::SyncMutation> = origen
        .connection()
        .prepare(
            "SELECT seq, target_key, entity, entity_key, op, payload, source,
                    ifnull(project, '') AS project, occurred_at, acked_at
               FROM sync_mutations WHERE entity = 'observation' ORDER BY seq",
        )
        .unwrap()
        .query_map([], crate::store::rows::map_sync_mutation)
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        mutaciones.iter().map(|m| m.op.as_str()).collect::<Vec<_>>(),
        vec!["upsert", "delete"],
        "el upsert y la lápida, en ese orden"
    );

    let (_destino_temp, mut destino) = super::store();
    destino.create_session("s1", "leteo", "C:/repo").unwrap();
    let vivas = |store: &Store| -> i64 {
        store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM observations WHERE deleted_at IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert!(
        destino
            .apply_pulled_sync_mutation("cloud", &mutaciones[0])
            .unwrap(),
        "el peer acepta el upsert"
    );
    assert_eq!(vivas(&destino), 1, "y la memoria está allí");
    assert!(
        destino
            .apply_pulled_sync_mutation("cloud", &mutaciones[1])
            .unwrap(),
        "y acepta la lápida en vez de diferirla"
    );
    assert_eq!(
        vivas(&destino),
        0,
        "which is what takes it out of where it was looking"
    );
}

/// The journal's own vocabulary: what is asked for, what is enrolled, and what
/// a queued mutation infers.
///
/// Thirteen functions in `wire.rs` had no test naming any of them. They are
/// reached through the paths above, which is why nothing was obviously broken —
/// and it is also why a change to one of them shows up as a failure somewhere
/// else, in a test about something quite different, with the journal's own
/// rules never stated anywhere.
#[test]
fn the_journal_says_which_project_a_mutation_belongs_to() {
    // Nothing enrolled, because half of what this says is what enrolment
    // changes. `store()` enrols `leteo` for the tests that are about something
    // else.
    let (_temp, mut store) = bare_store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();

    // A target key is a name, and a blank one is a caller mistake rather than a
    // default.
    assert_eq!(
        crate::store::wire::require_sync_target("  peer-a  ").unwrap(),
        "peer-a"
    );
    let refusal =
        crate::store::wire::require_sync_target("   ").expect_err("a blank target is not a target");
    assert!(refusal.to_string().contains("required"), "{refusal}");

    let tx = store.connection().unchecked_transaction().unwrap();
    // Nothing is enrolled yet, and an empty project answers `true` so a write
    // with no project at all is never silently dropped from the journal.
    assert!(!crate::store::wire::is_enrolled_tx(&tx, "leteo").unwrap());
    assert!(crate::store::wire::is_enrolled_tx(&tx, "").unwrap());

    // And a mutation for a project nobody replicates is not queued. Enrolling
    // one queues what it already holds, so this loses no history.
    crate::store::wire::enqueue_mutation(
        &tx,
        "session",
        "s1",
        crate::sync::OP_UPSERT,
        &serde_json::json!({ "id": "s1", "project": "leteo" }),
        "leteo",
    )
    .unwrap();
    let queued: i64 = tx
        .query_row("SELECT COUNT(*) FROM sync_mutations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        queued, 0,
        "nothing is journalled for a project nobody syncs"
    );

    tx.execute(
        "INSERT INTO sync_enrolled_projects (project) VALUES ('leteo')",
        [],
    )
    .unwrap();
    assert!(crate::store::wire::is_enrolled_tx(&tx, "leteo").unwrap());

    // The project comes from the payload when the caller does not name one …
    crate::store::wire::enqueue_mutation(
        &tx,
        "session",
        "from-payload",
        crate::sync::OP_UPSERT,
        &serde_json::json!({ "id": "from-payload", "project": "Leteo" }),
        "",
    )
    .unwrap();
    // … and from the session when the payload has none either.
    crate::store::wire::enqueue_mutation(
        &tx,
        "prompt",
        "from-session",
        crate::sync::OP_UPSERT,
        &serde_json::json!({ "sync_id": "from-session", "session_id": "s1" }),
        "",
    )
    .unwrap();
    let projects: Vec<String> = tx
        .prepare("SELECT project FROM sync_mutations ORDER BY seq")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        projects,
        vec!["leteo".to_owned(), "leteo".to_owned()],
        "a mutation with no project of its own takes the one its payload or its session names"
    );
    tx.commit().unwrap();
}

/// Enrolling a project queues its sessions before anything that references
/// them.
///
/// The order is the whole correctness of the backfill: a peer applies mutations
/// in sequence, and both observations and prompts carry a foreign key to
/// `sessions`. Queued the other way round, the first memory a peer tries to
/// apply fails with `FOREIGN KEY constraint failed` — and so does every one
/// after it, on every retry, for ever.
#[test]
fn a_backfill_queues_a_session_before_what_points_at_it() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    store
        .add_observation(observation("s1", "Something worth keeping", "a body"))
        .unwrap();
    store
        .add_prompt(AddPrompt {
            session_id: "s1".to_owned(),
            content: "a question worth keeping".to_owned(),
            project: Some("leteo".to_owned()),
        })
        .unwrap();

    let tx = store.connection().unchecked_transaction().unwrap();
    crate::store::wire::backfill_project_tx(&tx, "leteo").unwrap();
    let order: Vec<String> = tx
        .prepare("SELECT entity FROM sync_mutations ORDER BY seq")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    tx.commit().unwrap();

    assert_eq!(
        order.first().map(String::as_str),
        Some("session"),
        "{order:?}"
    );
    let session_at = order.iter().position(|entity| entity == "session").unwrap();
    for entity in ["observation", "prompt"] {
        let at = order
            .iter()
            .position(|queued| queued == entity)
            .unwrap_or_else(|| panic!("the backfill queued no {entity}: {order:?}"));
        assert!(
            at > session_at,
            "{entity} is queued before the session it points at: {order:?}"
        );
    }
}

/// A deletion is queued as a deletion, and an acknowledged mutation is only
/// pruned once it is old.
#[test]
fn deletions_are_journalled_and_acknowledged_mutations_age_out() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let saved = store
        .add_observation(observation("s1", "Something worth keeping", "a body"))
        .unwrap()
        .observation;
    let tx = store.connection().unchecked_transaction().unwrap();
    crate::store::wire::enqueue_observation(&tx, &saved).unwrap();
    let prompt = Prompt {
        sync_id: "prompt-gone".to_owned(),
        session_id: "s1".to_owned(),
        project: "leteo".to_owned(),
        content: "a question that was withdrawn".to_owned(),
        id: 0,
        created_at: "2026-01-01 00:00:00".to_owned(),
    };
    crate::store::wire::enqueue_prompt_delete_tx(&tx, &prompt).unwrap();
    crate::store::wire::enqueue_session_delete_tx(&tx, "s1", "leteo").unwrap();
    // The last three, because the session and the memory above queued their own
    // on the way in — which is the ordinary case and not what this is about.
    let queued: Vec<(String, String)> = tx
        .prepare("SELECT entity, op FROM sync_mutations ORDER BY seq DESC LIMIT 3")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .map(|mut rows| {
            rows.reverse();
            rows
        })
        .unwrap();
    assert_eq!(
        queued,
        vec![
            ("observation".to_owned(), crate::sync::OP_UPSERT.to_owned()),
            ("prompt".to_owned(), crate::sync::OP_DELETE.to_owned()),
            ("session".to_owned(), crate::sync::OP_DELETE.to_owned()),
        ]
    );

    // A target has to exist before a mutation can be addressed to it:
    // `sync_mutations.target_key` references `sync_state`.
    tx.execute("INSERT INTO sync_state (target_key) VALUES ('peer-a')", [])
        .unwrap();

    // Acknowledged long ago, acknowledged just now, and never acknowledged.
    tx.execute(
        "UPDATE sync_mutations SET target_key = 'peer-a',
             acked_at = datetime('now', '-90 days') WHERE entity = 'observation'",
        [],
    )
    .unwrap();
    tx.execute(
        "UPDATE sync_mutations SET target_key = 'peer-a', acked_at = datetime('now')
         WHERE entity = 'prompt'",
        [],
    )
    .unwrap();
    tx.execute(
        "UPDATE sync_mutations SET target_key = 'peer-a' WHERE entity = 'session'",
        [],
    )
    .unwrap();
    let pruned = crate::store::wire::prune_acked_mutations_tx(&tx, "peer-a").unwrap();
    assert_eq!(
        pruned, 2,
        "only what a peer has had for long enough goes, and the two observations above were \
         acknowledged together"
    );
    let left: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM sync_mutations WHERE acked_at IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(left, 1, "the one acknowledged a moment ago is still here");
    tx.commit().unwrap();
}

/// A relation is journalled against the project that owns its ends, and one
/// whose ends had not arrived is replayed when they have.
#[test]
fn a_relation_waits_for_both_ends_and_is_replayed_when_they_arrive() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let source = store
        .add_observation(observation("s1", "The newer way", "a body"))
        .unwrap()
        .observation;
    let target = store
        .add_observation(observation("s1", "The older way", "a body"))
        .unwrap()
        .observation;
    let tx = store.connection().unchecked_transaction().unwrap();
    let relation = Relation {
        id: 0,
        sync_id: normalize::sync_id("rel"),
        source_id: source.sync_id.clone(),
        target_id: target.sync_id.clone(),
        relation: RELATION_SUPERSEDES.to_owned(),
        reason: None,
        evidence: None,
        confidence: None,
        judgment_status: "judged".to_owned(),
        marked_by_actor: None,
        marked_by_kind: None,
        marked_by_model: None,
        session_id: None,
        created_at: "2026-01-01 00:00:00".to_owned(),
        updated_at: "2026-01-01 00:00:00".to_owned(),
    };
    // The source's project decides, and an empty one falls back to the target's
    // — a relation belongs to the memories it is about, not to whoever judged.
    crate::store::wire::enqueue_relation_if_enrolled(&tx, &relation, "", "leteo").unwrap();
    let queued: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM sync_mutations WHERE entity = 'relation'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        queued, 1,
        "the target's project stands in for a blank source"
    );

    // A relation whose ends are not here yet is deferred rather than lost, and
    // an attempt against it is recorded with what went wrong.
    let deferred = normalize::sync_id("rel");
    let payload = serde_json::json!({
        "sync_id": deferred,
        "source_id": source.sync_id,
        "target_id": target.sync_id,
        "relation": RELATION_SUPERSEDES,
        "judgment_status": "judged",
    })
    .to_string();
    tx.execute(
        "INSERT INTO sync_deferred_mutations
             (sync_id, entity, payload, apply_status, retry_count, first_seen_at)
         VALUES (?1, 'relation', ?2, 'deferred', 0, datetime('now'))",
        rusqlite::params![deferred, payload],
    )
    .unwrap();
    crate::store::wire::record_deferred_attempt_tx(&tx, &deferred, 2, false, "both ends missing")
        .unwrap();
    let (status, retries, error): (String, i64, String) = tx
        .query_row(
            "SELECT apply_status, retry_count, last_error FROM sync_deferred_mutations
             WHERE sync_id = ?1",
            [&deferred],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!((status.as_str(), retries), ("deferred", 2));
    assert_eq!(error, "both ends missing");
    // And one that has run out of tries is retired rather than retried for ever.
    crate::store::wire::record_deferred_attempt_tx(&tx, &deferred, 9, true, "gave up").unwrap();
    let status: String = tx
        .query_row(
            "SELECT apply_status FROM sync_deferred_mutations WHERE sync_id = ?1",
            [&deferred],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "dead");

    // Both ends are here, so a replay of what is still deferred applies it.
    tx.execute(
        "UPDATE sync_deferred_mutations SET apply_status = 'deferred' WHERE sync_id = ?1",
        [&deferred],
    )
    .unwrap();
    crate::store::wire::replay_deferred_relations_tx(&tx, 50_000).unwrap();
    let applied: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM memory_relations WHERE sync_id = ?1",
            [&deferred],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(applied, 1, "the relation is stored once its ends exist");
    tx.commit().unwrap();
}

/// What a peer sent is applied by the same door, whatever it is about.
#[test]
fn what_a_peer_sent_is_applied_by_entity() {
    let (_temp, store) = store();
    let tx = store.connection().unchecked_transaction().unwrap();
    for (entity, key, payload) in [
        (
            "session",
            "peer-session",
            serde_json::json!({
                "id": "peer-session", "project": "leteo", "directory": "C:/peer",
                "started_at": "2026-01-01 00:00:00",
            }),
        ),
        (
            "observation",
            "obs-from-a-peer",
            serde_json::json!({
                "sync_id": "obs-from-a-peer", "session_id": "peer-session",
                "type": "decision", "title": "Written somewhere else",
                "content": "and carried here", "project": "leteo", "scope": "project",
            }),
        ),
    ] {
        crate::store::wire::apply_sync_mutation_tx(
            &tx,
            &SyncMutation {
                entity: entity.to_owned(),
                entity_key: key.to_owned(),
                op: crate::sync::OP_UPSERT.to_owned(),
                payload: payload.to_string(),
                source: "remote".to_owned(),
                ..SyncMutation::default()
            },
            50_000,
        )
        .unwrap();
    }
    let sessions: i64 = tx
        .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
        .unwrap();
    let memories: i64 = tx
        .query_row("SELECT COUNT(*) FROM observations", [], |row| row.get(0))
        .unwrap();
    assert_eq!((sessions, memories), (1, 1));

    // A second memory, because a relation needs two ends and one from a memory
    // to itself is not a claim the store keeps.
    crate::store::wire::apply_sync_mutation_tx(
        &tx,
        &SyncMutation {
            entity: "observation".to_owned(),
            entity_key: "obs-the-other-end".to_owned(),
            op: crate::sync::OP_UPSERT.to_owned(),
            payload: serde_json::json!({
                "sync_id": "obs-the-other-end", "session_id": "peer-session",
                "type": "decision", "title": "And the one it is about",
                "content": "also carried here", "project": "leteo", "scope": "project",
            })
            .to_string(),
            source: "remote".to_owned(),
            ..SyncMutation::default()
        },
        50_000,
    )
    .unwrap();

    let ends: Vec<String> = tx
        .prepare("SELECT sync_id FROM observations")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    crate::store::wire::apply_relation_upsert_tx(
        &tx,
        &SyncMutation {
            entity: "relation".to_owned(),
            entity_key: "rel-from-a-peer".to_owned(),
            op: crate::sync::OP_UPSERT.to_owned(),
            payload: serde_json::json!({
                "sync_id": "rel-from-a-peer",
                "source_id": ends[0], "target_id": ends[1],
                "relation": RELATION_RELATED, "judgment_status": "judged",
            })
            .to_string(),
            source: "remote".to_owned(),
            ..SyncMutation::default()
        },
        50_000,
    )
    .unwrap();
    let relations: i64 = tx
        .query_row("SELECT COUNT(*) FROM memory_relations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(relations, 1);
    tx.commit().unwrap();
}

/// Recovering has to put back every field the failures set, not just the
/// counter. `sync_state` keeps five of them — the lifecycle, the count, the
/// backoff, the reason and the last error — and a recovery that clears four
/// leaves a target that reports itself healthy while still showing the message
/// from the outage that ended. This drives a run of failures and then the
/// recovery, and reads all five back.
#[test]
fn failures_accumulate_and_recovering_clears_every_mark_they_left() {
    let (_temp, mut store) = store();
    let until = chrono::Utc::now() + chrono::Duration::minutes(5);

    // A store that has never synced is `idle`, which the schema puts there —
    // not `healthy`, which is a thing only a completed round trip may claim.
    let fresh = store.sync_state_if_any("cloud").unwrap().unwrap();
    assert_eq!(fresh.lifecycle, "idle");
    assert_eq!(fresh.consecutive_failures, 0);
    assert_eq!(fresh.last_error, None);

    // The message is trimmed on the way in: what a transport hands over ends in
    // a newline more often than not.
    store
        .mark_sync_failure("cloud", "  connection refused\n", until)
        .unwrap();
    let after_one = store.sync_state_if_any("cloud").unwrap().unwrap();
    assert_eq!(after_one.lifecycle, "backoff");
    assert_eq!(after_one.consecutive_failures, 1);
    assert_eq!(after_one.last_error.as_deref(), Some("connection refused"));
    assert!(after_one.backoff_until.is_some());

    // The second failure counts on top of the first rather than replacing it —
    // the count is what a backoff is computed from, so one that resets to 1
    // every time is a backoff that never grows.
    store
        .mark_sync_failure("cloud", "connection refused", until)
        .unwrap();
    store
        .mark_sync_failure("cloud", "gateway timeout", until)
        .unwrap();
    let after_three = store.sync_state_if_any("cloud").unwrap().unwrap();
    assert_eq!(after_three.consecutive_failures, 3);
    assert_eq!(after_three.last_error.as_deref(), Some("gateway timeout"));

    store.mark_sync_healthy("cloud").unwrap();
    let healthy = store.sync_state_if_any("cloud").unwrap().unwrap();
    assert_eq!(healthy.lifecycle, "healthy");
    assert_eq!(healthy.consecutive_failures, 0);
    assert_eq!(healthy.backoff_until, None);
    assert_eq!(healthy.last_error, None);
    assert_eq!(healthy.reason_code, None);
    assert_eq!(healthy.reason_message, None);

    // And what the journal was doing is untouched by either: the sequence
    // numbers are the sync's place in the queue, not its health.
    assert_eq!(healthy.last_acked_seq, after_three.last_acked_seq);
    assert_eq!(healthy.last_enqueued_seq, after_three.last_enqueued_seq);

    // A blank target is refused by both doors, not silently written to "".
    assert!(store.mark_sync_failure("  ", "x", until).is_err());
    assert!(store.mark_sync_healthy("").is_err());
}
