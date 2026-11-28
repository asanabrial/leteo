//! Version history: the title and body a content-changing write replaced.

use super::*;

/// An upsert under a topic key that changes the text keeps the text it
/// replaced, byte for byte.
#[test]
fn an_upsert_that_changes_content_keeps_the_previous_version_byte_for_byte() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let first_title = "Almacenamiento — decisión";
    let first_body = "El cuerpo original, con acentos: ñ, é, ü.\nSegunda línea.";
    let mut topic = observation("s1", first_title, first_body);
    topic.topic_key = Some("architecture/storage".to_owned());
    let saved = store.add_observation(topic.clone()).unwrap().observation;

    topic.title = "Almacenamiento — revisado".to_owned();
    topic.content = "Un cuerpo distinto del primero.".to_owned();
    store.add_observation(topic).unwrap();

    let versions = store.observation_versions(&saved.sync_id).unwrap();
    assert_eq!(versions.len(), 1, "the replaced revision is kept once");
    assert_eq!(versions[0].revision, 1, "the revision_count it superseded");
    assert_eq!(versions[0].title, first_title);
    assert_eq!(
        versions[0].content.as_bytes(),
        first_body.as_bytes(),
        "the previous body comes back byte for byte"
    );
    assert!(!versions[0].replaced_at.is_empty());

    assert_eq!(
        store.get_observation(saved.id).unwrap().content,
        "Un cuerpo distinto del primero."
    );
}

/// A title is part of the version, so changing only the title keeps it.
#[test]
fn a_title_only_change_keeps_the_previous_title_and_body() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let saved = store
        .add_observation(observation("s1", "Título viejo", "cuerpo intacto"))
        .unwrap()
        .observation;

    let updated = store
        .update_observation_with_replaced(
            saved.id,
            None,
            UpdateObservation {
                title: Some("Título nuevo".to_owned()),
                ..UpdateObservation::default()
            },
        )
        .unwrap();
    assert_eq!(
        updated.replaced.expect("a title was replaced").bytes,
        "cuerpo intacto".len()
    );

    let versions = store.observation_versions(&saved.sync_id).unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].title, "Título viejo");
    assert_eq!(versions[0].content, "cuerpo intacto");
}

/// A change that only moves metadata loses no text, so it keeps no version.
#[test]
fn a_metadata_only_update_keeps_no_version() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let saved = store
        .add_observation(observation("s1", "Una decisión", "el cuerpo"))
        .unwrap()
        .observation;

    let updated = store
        .update_observation_with_replaced(
            saved.id,
            None,
            UpdateObservation {
                kind: Some("decision".to_owned()),
                scope: Some("personal".to_owned()),
                ..UpdateObservation::default()
            },
        )
        .unwrap();
    assert!(updated.replaced.is_none(), "no body was replaced");
    assert!(
        store
            .observation_versions(&saved.sync_id)
            .unwrap()
            .is_empty(),
        "a metadata-only change is out of the history's scope"
    );
    assert_eq!(
        updated.observation.revision_count, 2,
        "the revision count still moves, as it always did"
    );
}

/// A content-changing update keeps the body it replaced and reports its size.
#[test]
fn a_content_changing_update_keeps_the_replaced_body() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let saved = store
        .add_observation(observation("s1", "Título", "cuerpo uno"))
        .unwrap()
        .observation;

    let updated = store
        .update_observation_with_replaced(
            saved.id,
            None,
            UpdateObservation {
                content: Some("cuerpo dos".to_owned()),
                ..UpdateObservation::default()
            },
        )
        .unwrap();
    let replaced = updated.replaced.expect("a body was replaced");
    assert_eq!(replaced.bytes, "cuerpo uno".len());
    assert!(
        !replaced.shrunk,
        "two bodies of a similar size are no shrink"
    );

    let versions = store.observation_versions(&saved.sync_id).unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].revision, 1);
    assert_eq!(versions[0].content, "cuerpo uno");
}

/// The reply reports the replaced size, and calls out a body under half of it.
#[test]
fn the_reply_calls_out_a_body_that_shrank_beyond_the_threshold() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let long = "a".repeat(400);
    let mut topic = observation("s1", "Grande", &long);
    topic.topic_key = Some("architecture/shrink".to_owned());
    store.add_observation(topic.clone()).unwrap();

    // Under half: 150 is less than 200.
    topic.content = "b".repeat(150);
    let shrunk = store.add_observation(topic.clone()).unwrap();
    let replaced = shrunk.replaced.expect("the upsert replaced a body");
    assert_eq!(replaced.bytes, 400);
    assert!(replaced.shrunk, "150 bytes is under half of 400");

    // At the boundary it does not fire: 200 is exactly half, not under it.
    topic.content = "c".repeat(200);
    let kept = store.add_observation(topic).unwrap();
    let replaced = kept.replaced.expect("the upsert replaced a body");
    assert_eq!(replaced.bytes, 150);
    assert!(!replaced.shrunk, "exactly half is not under half");
}

/// The same call-out applies to `mem_update`'s door.
#[test]
fn an_update_reports_the_body_it_replaced_and_calls_out_a_shrink() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let saved = store
        .add_observation(observation("s1", "Grande", &"x".repeat(400)))
        .unwrap()
        .observation;

    let updated = store
        .update_observation_with_replaced(
            saved.id,
            None,
            UpdateObservation {
                content: Some("y".repeat(5).to_owned()),
                ..UpdateObservation::default()
            },
        )
        .unwrap();
    let replaced = updated.replaced.expect("the update replaced a body");
    assert_eq!(replaced.bytes, 400);
    assert!(replaced.shrunk, "five bytes is under half of four hundred");
    assert_eq!(updated.observation.content, "yyyyy");
}

/// The bound is the newest N, and the ones that survive are the newest.
#[test]
fn versions_stop_at_the_retention_bound_and_the_newest_survive() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let total = OBSERVATION_VERSION_RETENTION + 5;
    let mut topic = observation("s1", "Evoluciona", "versión 1");
    topic.topic_key = Some("architecture/evolves".to_owned());
    let saved = store.add_observation(topic.clone()).unwrap().observation;
    for n in 2..=total {
        topic.content = format!("versión {n}");
        store.add_observation(topic.clone()).unwrap();
    }

    let versions = store.observation_versions(&saved.sync_id).unwrap();
    assert_eq!(
        versions.len(),
        OBSERVATION_VERSION_RETENTION,
        "the published bound is the one applied"
    );
    // Newest first.
    assert_eq!(
        versions.first().unwrap().revision,
        (total - 1) as i64,
        "the newest version is the revision just superseded"
    );
    assert_eq!(
        versions.first().unwrap().content,
        format!("versión {}", total - 1)
    );
    let oldest_kept = total - OBSERVATION_VERSION_RETENTION;
    assert_eq!(versions.last().unwrap().revision, oldest_kept as i64);
    assert_eq!(
        versions.last().unwrap().content,
        format!("versión {oldest_kept}")
    );

    let older: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM observation_versions
              WHERE observation_sync_id = ?1 AND revision < ?2",
            rusqlite::params![saved.sync_id, oldest_kept as i64],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(older, 0, "everything older than the bound is gone");
}

/// A version survives even when the project it belongs to replicates nowhere.
#[test]
fn a_version_is_kept_locally_even_when_nothing_replicates() {
    let (_temp, mut store) = bare_store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let mut topic = observation("s1", "Local", "antes");
    topic.topic_key = Some("architecture/local".to_owned());
    let saved = store.add_observation(topic.clone()).unwrap().observation;
    topic.content = "después".to_owned();
    store.add_observation(topic).unwrap();

    assert_eq!(store.observation_versions(&saved.sync_id).unwrap().len(), 1);
    assert!(
        store
            .list_pending_sync_mutations("cloud", &["leteo".to_owned()], 100)
            .unwrap()
            .is_empty(),
        "nothing is journalled for a project nobody replicates"
    );
}

/// The version a peer records is the same one the origin kept.
#[test]
fn a_version_replicates_byte_for_byte_and_a_replay_is_a_no_op() {
    let (_temp, mut source) = store();
    source.create_session("s1", "leteo", "C:/repo").unwrap();
    let mut topic = observation("s1", "Replicada", "cuerpo original");
    topic.topic_key = Some("architecture/replicated".to_owned());
    let saved = source.add_observation(topic.clone()).unwrap().observation;
    topic.content = "cuerpo nuevo".to_owned();
    source.add_observation(topic).unwrap();

    let queued = source
        .list_pending_sync_mutations("cloud", &["leteo".to_owned()], 100)
        .unwrap();
    let version_mutation = queued
        .iter()
        .find(|mutation| mutation.entity == crate::sync::ENTITY_OBSERVATION_VERSION)
        .cloned()
        .expect("an upsert that replaced content queues its version");

    let (_peer_temp, mut peer) = store();
    peer.apply_pulled(&version_mutation).unwrap();

    let local = source.observation_versions(&saved.sync_id).unwrap();
    let remote = peer.observation_versions(&saved.sync_id).unwrap();
    assert_eq!(remote.len(), 1);
    assert_eq!(remote[0].revision, local[0].revision);
    assert_eq!(remote[0].title, local[0].title);
    assert_eq!(
        remote[0].content.as_bytes(),
        local[0].content.as_bytes(),
        "the replicated version carries the same bytes as the local one"
    );
    assert_eq!(
        remote[0].replaced_at, local[0].replaced_at,
        "the timestamp travels rather than being regenerated on arrival"
    );

    // A retried pull — the same payload under a new sequence — is ignored by
    // the unique index rather than storing a second copy.
    let mut replay = version_mutation;
    replay.seq += 1;
    peer.apply_pulled(&replay).unwrap();
    assert_eq!(
        peer.observation_versions(&saved.sync_id).unwrap().len(),
        1,
        "applying a version twice changes nothing"
    );
}

/// A store that predates the table is given it, indexes and all.
#[test]
fn a_store_that_predates_the_version_table_is_given_it() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("leteo.db");
    {
        let store = Store::open(StoreConfig::new(path.clone())).unwrap();
        store
            .connection
            .execute_batch(
                "DROP TABLE observation_versions;
                 PRAGMA user_version = 18;",
            )
            .unwrap();
    }

    let store = Store::open(StoreConfig::new(path)).unwrap();
    assert_eq!(
        schema_version(&store.connection).unwrap(),
        SCHEMA_VERSION,
        "the store is carried forward rather than left where it was"
    );
    let columns = table_info(&store.connection, "observation_versions").unwrap();
    for expected in [
        "id",
        "observation_sync_id",
        "revision",
        "title",
        "content",
        "replaced_at",
    ] {
        assert!(
            columns.iter().any(|column| column.name == expected),
            "the rebuilt table is missing {expected}"
        );
    }
    let indexes: Vec<String> = store
        .connection
        .prepare(
            "SELECT name FROM sqlite_master
              WHERE type = 'index' AND tbl_name = 'observation_versions'
              ORDER BY name",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for expected in [
        "idx_observation_versions_identity",
        "idx_observation_versions_read",
    ] {
        assert!(
            indexes.iter().any(|name| name == expected),
            "the rebuilt table is missing {expected}: {indexes:?}"
        );
    }
}

/// A version arriving over the wire goes through the same two rules a local
/// write did — redaction and the length bound — because a payload is a door like
/// any other. Without them a peer could store a `<private>` span the local path
/// can never write and hand it back through `include_history`.
#[test]
fn a_replicated_version_is_redacted_and_bounded_like_a_local_one() {
    let (_temp, mut store) = store();
    let secret = "ghp_secreto";
    let over_long = "t".repeat(store.max_observation_length() + 500);
    let mutation = SyncMutation {
        seq: 42,
        target_key: "cloud".to_owned(),
        entity: crate::sync::ENTITY_OBSERVATION_VERSION.to_owned(),
        entity_key: "obs-replicated".to_owned(),
        op: crate::sync::OP_UPSERT.to_owned(),
        payload: serde_json::json!({
            "sync_id": "obs-replicated",
            "revision": 1,
            "title": over_long,
            "content": format!("antes <private>{secret}</private> después"),
            "replaced_at": "2026-08-05 04:00:00",
        })
        .to_string(),
        source: "remote".to_owned(),
        project: "leteo".to_owned(),
        occurred_at: "2026-08-05 04:00:00".to_owned(),
        acked_at: None,
    };
    store.apply_pulled(&mutation).unwrap();

    let versions = store.observation_versions("obs-replicated").unwrap();
    assert_eq!(versions.len(), 1);
    assert!(
        !versions[0].content.contains(secret),
        "a private span never lands in the version history: {}",
        versions[0].content
    );
    assert!(versions[0].content.contains("[REDACTED]"));
    assert!(
        versions[0].title.len() <= store.max_observation_length(),
        "the title is bounded on this door too: {} bytes",
        versions[0].title.len()
    );
}

/// An export is this store written down: the version history travels with it,
/// and an import restores it through the same bound and identity the live path
/// uses.
#[test]
fn an_export_carries_the_version_history_and_an_import_restores_it() {
    let (_source_temp, mut source) = store();
    source.create_session("s1", "leteo", "C:/repo").unwrap();
    let mut topic = observation("s1", "Exportada", "cuerpo original");
    topic.topic_key = Some("architecture/exported".to_owned());
    let saved = source.add_observation(topic.clone()).unwrap().observation;
    topic.content = "cuerpo nuevo".to_owned();
    source.add_observation(topic).unwrap();

    let json = source.export_json(None).unwrap();
    let (_destination_temp, mut destination) = store();
    let imported = destination.import_json(&json).unwrap();
    assert_eq!(
        imported.observation_versions_imported, 1,
        "the export carried the version and the import counted it"
    );

    let before = source.observation_versions(&saved.sync_id).unwrap();
    let after = destination.observation_versions(&saved.sync_id).unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].revision, before[0].revision);
    assert_eq!(after[0].title, before[0].title);
    assert_eq!(
        after[0].content.as_bytes(),
        before[0].content.as_bytes(),
        "the restored version is byte-identical"
    );
    assert_eq!(
        after[0].replaced_at, before[0].replaced_at,
        "the timestamp travels rather than being regenerated"
    );

    // A second import of the same backup changes nothing: the identity index
    // makes it idempotent, as it does for a replayed payload.
    let again = destination.import_json(&json).unwrap();
    assert_eq!(again.observation_versions_imported, 0);
    assert_eq!(
        destination
            .observation_versions(&saved.sync_id)
            .unwrap()
            .len(),
        1
    );
}

/// A find/replace is a content-changing write like any other: it keeps the text
/// it replaced, moves the revision count once, and stores the hash of what it
/// wrote.
#[test]
fn a_find_and_replace_keeps_the_version_it_replaced() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let saved = store
        .add_observation(observation("s1", "Una decisión", "el método antiguo aquí"))
        .unwrap()
        .observation;

    let updated = store
        .update_observation_with_replaced(
            saved.id,
            None,
            UpdateObservation {
                find: Some("antiguo".to_owned()),
                replace: Some("nuevo".to_owned()),
                ..UpdateObservation::default()
            },
        )
        .unwrap();

    assert_eq!(updated.observation.content, "el método nuevo aquí");
    assert_eq!(
        updated.observation.revision_count,
        saved.revision_count + 1,
        "the revision count moves once"
    );
    let versions = store.observation_versions(&saved.sync_id).unwrap();
    assert_eq!(
        versions.len(),
        1,
        "exactly one version, the text it replaced"
    );
    assert_eq!(versions[0].revision, saved.revision_count);
    assert_eq!(versions[0].content, "el método antiguo aquí");

    let stored_hash: String = store
        .connection
        .query_row(
            "SELECT normalized_hash FROM observations WHERE id = ?1",
            [saved.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        stored_hash,
        crate::memory::normalize::normalized_hash("el método nuevo aquí"),
        "the row carries the hash of the edited text, not of the text it replaced"
    );
}

/// A `find` that names more than one span is refused, and the refusal leaves the
/// revision, the history and the journal exactly as they were.
#[test]
fn a_find_that_occurs_twice_changes_nothing() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let saved = store
        .add_observation(observation("s1", "Repetido", "aquí y aquí"))
        .unwrap()
        .observation;
    let queued = |store: &Store| -> i64 {
        store
            .connection
            .query_row("SELECT COUNT(*) FROM sync_mutations", [], |row| row.get(0))
            .unwrap()
    };
    let before = queued(&store);

    let refused = store.update_observation_with_replaced(
        saved.id,
        None,
        UpdateObservation {
            find: Some("aquí".to_owned()),
            replace: Some("allí".to_owned()),
            ..UpdateObservation::default()
        },
    );
    assert!(
        matches!(refused, Err(StoreError::EditAmbiguous { matches: 2 })),
        "{refused:?}"
    );

    let after = store.get_observation(saved.id).unwrap();
    assert_eq!(after.content, "aquí y aquí", "the body is untouched");
    assert_eq!(
        after.revision_count, saved.revision_count,
        "the revision count does not move"
    );
    assert!(
        store
            .observation_versions(&saved.sync_id)
            .unwrap()
            .is_empty(),
        "no version was kept"
    );
    assert_eq!(queued(&store), before, "and nothing was journalled");
}

/// A `find` whose occurrences overlap is refused too: the guarantee is that a
/// `find` naming more than one span changes nothing, and `111` sits twice in
/// `1111` though `str::matches` counts one.
#[test]
fn an_overlapping_find_is_refused_as_ambiguous_too() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let saved = store
        .add_observation(observation("s1", "Solapado", "1111"))
        .unwrap()
        .observation;

    let refused = store.update_observation_with_replaced(
        saved.id,
        None,
        UpdateObservation {
            find: Some("111".to_owned()),
            replace: Some("2".to_owned()),
            ..UpdateObservation::default()
        },
    );
    assert!(
        matches!(refused, Err(StoreError::EditAmbiguous { matches: 2 })),
        "{refused:?}"
    );
    assert_eq!(
        store.get_observation(saved.id).unwrap().content,
        "1111",
        "the body is untouched"
    );
}

/// A `find` that is not in the body is refused for the same reason, with its own
/// code: nothing is written and the store is left as it was.
#[test]
fn a_find_that_is_absent_changes_nothing() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let saved = store
        .add_observation(observation("s1", "Ausente", "el cuerpo de siempre"))
        .unwrap()
        .observation;
    let queued = |store: &Store| -> i64 {
        store
            .connection
            .query_row("SELECT COUNT(*) FROM sync_mutations", [], |row| row.get(0))
            .unwrap()
    };
    let before = queued(&store);

    let refused = store.update_observation_with_replaced(
        saved.id,
        None,
        UpdateObservation {
            find: Some("no está".to_owned()),
            replace: Some("da igual".to_owned()),
            ..UpdateObservation::default()
        },
    );
    assert!(
        matches!(refused, Err(StoreError::EditNotFound)),
        "{refused:?}"
    );

    let after = store.get_observation(saved.id).unwrap();
    assert_eq!(
        after.content, "el cuerpo de siempre",
        "the body is untouched"
    );
    assert_eq!(after.revision_count, saved.revision_count);
    assert!(
        store
            .observation_versions(&saved.sync_id)
            .unwrap()
            .is_empty()
    );
    assert_eq!(queued(&store), before, "and nothing was journalled");
}

/// A replacement goes through the same redaction a whole-body write does, so a
/// `<private>` span cannot arrive by the partial door and be handed back later.
#[test]
fn a_private_span_written_through_replace_is_not_stored() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let saved = store
        .add_observation(observation("s1", "Token", "usa TOKEN aquí"))
        .unwrap()
        .observation;

    let updated = store
        .update_observation_with_replaced(
            saved.id,
            None,
            UpdateObservation {
                find: Some("TOKEN".to_owned()),
                replace: Some("<private>ghp_secreto</private>".to_owned()),
                ..UpdateObservation::default()
            },
        )
        .unwrap();

    assert!(
        !updated.observation.content.contains("ghp_secreto"),
        "a private span never lands through a replace: {}",
        updated.observation.content
    );
    assert!(updated.observation.content.contains("[REDACTED]"));
}

/// The edit a find/replace makes is an ordinary revision, so the peer receives
/// it as an upsert carrying the edited body rather than as anything new.
#[test]
fn a_find_and_replace_reaches_the_peer_as_an_upsert() {
    let (_source_temp, mut source) = store();
    source.create_session("s1", "leteo", "C:/repo").unwrap();
    let saved = source
        .add_observation(observation("s1", "Replicada", "el método antiguo"))
        .unwrap()
        .observation;
    source
        .update_observation_with_replaced(
            saved.id,
            None,
            UpdateObservation {
                find: Some("antiguo".to_owned()),
                replace: Some("nuevo".to_owned()),
                ..UpdateObservation::default()
            },
        )
        .unwrap();

    let upsert = source
        .list_pending_sync_mutations("cloud", &["leteo".to_owned()], 100)
        .unwrap()
        .into_iter()
        .rfind(|mutation| mutation.entity == crate::sync::ENTITY_OBSERVATION)
        .expect("the edit queues an observation upsert");

    let (_peer_temp, mut peer) = store();
    peer.create_session("s1", "leteo", "C:/repo").unwrap();
    peer.apply_pulled(&upsert).unwrap();

    let (content, revision_count): (String, i64) = peer
        .connection
        .query_row(
            "SELECT content, revision_count FROM observations WHERE sync_id = ?1",
            [&saved.sync_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(content, "el método nuevo", "the edited body travelled");
    assert_eq!(
        revision_count,
        saved.revision_count + 1,
        "and it is the revision the edit made"
    );
}

/// Rows of `observation_versions` that carry a `sync_id`, counted on the table
/// itself: the read API would be a fine witness here, but the claim is about
/// what the database file still holds.
fn version_rows(store: &Store, sync_id: &str) -> i64 {
    store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM observation_versions WHERE observation_sync_id = ?1",
            [sync_id],
            |row| row.get(0),
        )
        .unwrap()
}

/// A memory rewritten twice under a topic key, and a second one beside it that
/// a delete of the first must leave alone.
fn memory_with_two_versions(store: &mut Store) -> (Observation, Observation) {
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let mut topic = observation("s1", "Primera", "cuerpo uno");
    topic.topic_key = Some("architecture/doomed".to_owned());
    let saved = store.add_observation(topic.clone()).unwrap().observation;
    topic.content = "cuerpo dos".to_owned();
    store.add_observation(topic.clone()).unwrap();
    topic.content = "cuerpo tres".to_owned();
    store.add_observation(topic).unwrap();

    let mut other = observation("s1", "Vecina", "texto uno");
    other.topic_key = Some("architecture/neighbour".to_owned());
    let neighbour = store.add_observation(other.clone()).unwrap().observation;
    other.content = "texto dos".to_owned();
    store.add_observation(other).unwrap();

    assert_eq!(version_rows(store, &saved.sync_id), 2);
    assert_eq!(version_rows(store, &neighbour.sync_id), 1);
    (saved, neighbour)
}

/// Deleting the row used to leave every earlier title and body in the file.
#[test]
fn a_hard_delete_of_one_memory_removes_its_versions_and_only_its_own() {
    let (_temp, mut store) = store();
    let (saved, neighbour) = memory_with_two_versions(&mut store);

    store.delete_observation(saved.id, None, true).unwrap();

    assert_eq!(version_rows(&store, &saved.sync_id), 0);
    assert_eq!(
        version_rows(&store, &neighbour.sync_id),
        1,
        "another memory's history is not this one's to remove"
    );
}

/// The lighter delete keeps the row, so it keeps what the row replaced.
#[test]
fn a_soft_delete_keeps_the_versions() {
    let (_temp, mut store) = store();
    let (saved, _neighbour) = memory_with_two_versions(&mut store);

    store.delete_observation(saved.id, None, false).unwrap();

    assert_eq!(version_rows(&store, &saved.sync_id), 2);
}

#[test]
fn a_hard_delete_of_a_session_removes_the_versions_of_its_memories() {
    let (_temp, mut store) = store();
    let (saved, neighbour) = memory_with_two_versions(&mut store);

    store.delete_session_and_contents("s1", true).unwrap();

    assert_eq!(version_rows(&store, &saved.sync_id), 0);
    assert_eq!(version_rows(&store, &neighbour.sync_id), 0);
}

#[test]
fn a_soft_delete_of_a_session_keeps_the_versions_of_its_memories() {
    let (_temp, mut store) = store();
    let (saved, neighbour) = memory_with_two_versions(&mut store);

    store.delete_session_and_contents("s1", false).unwrap();

    assert_eq!(version_rows(&store, &saved.sync_id), 2);
    assert_eq!(version_rows(&store, &neighbour.sync_id), 1);
}

#[test]
fn a_hard_delete_of_a_project_removes_the_versions_of_its_memories() {
    let (_temp, mut store) = store();
    let (saved, neighbour) = memory_with_two_versions(&mut store);

    store.delete_project("leteo", true).unwrap();

    assert_eq!(version_rows(&store, &saved.sync_id), 0);
    assert_eq!(version_rows(&store, &neighbour.sync_id), 0);
}

#[test]
fn a_soft_delete_of_a_project_keeps_the_versions_of_its_memories() {
    let (_temp, mut store) = store();
    let (saved, neighbour) = memory_with_two_versions(&mut store);

    store.delete_project("leteo", false).unwrap();

    assert_eq!(version_rows(&store, &saved.sync_id), 2);
    assert_eq!(version_rows(&store, &neighbour.sync_id), 1);
}

/// A delete that arrives from another machine is a door like the local one: the
/// peer holds the versions the origin sent, and the delete has to take them.
#[test]
fn a_hard_delete_applied_from_another_machine_removes_the_versions_it_holds() {
    let (_source_temp, mut source) = store();
    let (saved, _neighbour) = memory_with_two_versions(&mut source);
    let queued = source
        .list_pending_sync_mutations("cloud", &["leteo".to_owned()], 100)
        .unwrap();

    let (_peer_temp, mut peer) = store();
    for mutation in &queued {
        peer.apply_pulled(mutation).unwrap();
    }
    assert_eq!(
        version_rows(&peer, &saved.sync_id),
        2,
        "the peer holds the history before the delete reaches it"
    );

    source.delete_observation(saved.id, None, true).unwrap();
    let delete = source
        .list_pending_sync_mutations("cloud", &["leteo".to_owned()], 100)
        .unwrap()
        .into_iter()
        .rfind(|mutation| {
            mutation.entity == crate::sync::ENTITY_OBSERVATION
                && mutation.op == crate::sync::OP_DELETE
        })
        .expect("a hard delete queues a delete");
    peer.apply_pulled(&delete).unwrap();

    assert_eq!(version_rows(&peer, &saved.sync_id), 0);
}

fn journal_payloads(store: &Store, sync_id: &str) -> Vec<String> {
    let mut statement = store
        .connection
        .prepare("SELECT payload FROM sync_mutations WHERE entity_key = ?1 ORDER BY seq")
        .unwrap();
    statement
        .query_map([sync_id], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// The journal held every earlier title and body of a memory, sent or not, and a
/// hard delete took the version rows and left those.
#[test]
fn a_hard_delete_leaves_no_earlier_text_in_the_journal() {
    let (_temp, mut store) = store();
    let (saved, neighbour) = memory_with_two_versions(&mut store);
    let rows_before = journal_payloads(&store, &saved.sync_id).len();
    store
        .connection
        .execute(
            "UPDATE sync_mutations SET acked_at = datetime('now') WHERE seq = (
                 SELECT MIN(seq) FROM sync_mutations WHERE entity_key = ?1)",
            [&saved.sync_id],
        )
        .unwrap();

    store.delete_observation(saved.id, None, true).unwrap();

    let payloads = journal_payloads(&store, &saved.sync_id);
    for earlier in ["cuerpo uno", "cuerpo dos", "cuerpo tres", "Primera"] {
        assert!(
            payloads.iter().all(|payload| !payload.contains(earlier)),
            "{earlier:?} is still in the journal: {payloads:?}"
        );
    }
    assert_eq!(
        payloads.len(),
        rows_before + 1,
        "rows are redacted in place, and the delete adds its own"
    );
    assert!(
        journal_payloads(&store, &neighbour.sync_id)
            .iter()
            .any(|payload| payload.contains("texto uno")),
        "another memory's journal is not this one's to redact"
    );
}

/// What a peer pulls after the redaction has to apply without failing the pull.
#[test]
fn a_redacted_journal_still_applies_on_a_peer_and_ends_with_nothing() {
    let (_source_temp, mut source) = store();
    let (saved, _neighbour) = memory_with_two_versions(&mut source);
    source.delete_observation(saved.id, None, true).unwrap();
    let queued = source
        .list_pending_sync_mutations("cloud", &["leteo".to_owned()], 100)
        .unwrap();

    let (_peer_temp, mut peer) = store();
    for mutation in &queued {
        peer.apply_pulled(mutation).unwrap();
    }

    assert_eq!(version_rows(&peer, &saved.sync_id), 0);
    let memories: i64 = peer
        .connection
        .query_row(
            "SELECT COUNT(*) FROM observations WHERE sync_id = ?1",
            [&saved.sync_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(memories, 0);
}

/// The journal queues a version before its memory's upsert, so a version for a
/// memory that is absent is ordinary; one for a memory a hard delete removed is
/// not.
#[test]
fn a_version_that_arrives_after_the_hard_delete_does_not_come_back() {
    let (_source_temp, mut source) = store();
    let (saved, _neighbour) = memory_with_two_versions(&mut source);
    let versions: Vec<SyncMutation> = source
        .list_pending_sync_mutations("cloud", &["leteo".to_owned()], 100)
        .unwrap()
        .into_iter()
        .filter(|mutation| {
            mutation.entity == crate::sync::ENTITY_OBSERVATION_VERSION
                && mutation.entity_key == saved.sync_id
        })
        .collect();
    assert_eq!(versions.len(), 2);

    let (_peer_temp, mut peer) = store();
    let delete = SyncMutation {
        seq: 1,
        target_key: "cloud".to_owned(),
        entity: crate::sync::ENTITY_OBSERVATION.to_owned(),
        entity_key: saved.sync_id.clone(),
        op: crate::sync::OP_DELETE.to_owned(),
        payload: serde_json::json!({
            "sync_id": saved.sync_id,
            "session_id": "s1",
            "project": "leteo",
            "deleted": true,
            "hard_delete": true,
        })
        .to_string(),
        source: "remote".to_owned(),
        project: "leteo".to_owned(),
        occurred_at: "2026-08-05 04:00:00".to_owned(),
        acked_at: None,
    };
    peer.apply_pulled(&delete).unwrap();
    for (offset, mut version) in versions.into_iter().enumerate() {
        version.seq = 100 + offset as i64;
        peer.apply_pulled(&version).unwrap();
    }

    assert_eq!(version_rows(&peer, &saved.sync_id), 0);
}

/// A version whose memory has not arrived yet is legitimate, so the guard is the
/// recorded delete and not the absence of a memory.
#[test]
fn a_version_for_a_memory_that_has_not_arrived_is_still_kept() {
    let (_temp, mut store) = store();
    let mutation = SyncMutation {
        seq: 7,
        target_key: "cloud".to_owned(),
        entity: crate::sync::ENTITY_OBSERVATION_VERSION.to_owned(),
        entity_key: "obs-not-yet".to_owned(),
        op: crate::sync::OP_UPSERT.to_owned(),
        payload: serde_json::json!({
            "sync_id": "obs-not-yet",
            "revision": 1,
            "title": "T",
            "content": "C",
            "replaced_at": "2026-08-05 04:00:00",
        })
        .to_string(),
        source: "remote".to_owned(),
        project: "leteo".to_owned(),
        occurred_at: "2026-08-05 04:00:00".to_owned(),
        acked_at: None,
    };
    store.apply_pulled(&mutation).unwrap();
    assert_eq!(version_rows(&store, "obs-not-yet"), 1);
}

/// Version rows a hard delete made before #332 left behind are counted by
/// `doctor` and removed by `--repair`, and a live or soft-deleted memory's
/// history is not touched.
#[test]
fn doctor_counts_orphan_versions_and_repair_removes_them() {
    let (_temp, mut store) = store();
    let (saved, neighbour) = memory_with_two_versions(&mut store);
    store.delete_observation(neighbour.id, None, false).unwrap();
    store
        .connection
        .execute(
            "INSERT INTO observation_versions (observation_sync_id, revision, title, content)
             VALUES ('obs-long-gone', 1, 'T', 'a secret body')",
            [],
        )
        .unwrap();

    let (report, _) = store.doctor_scoped(Some("orphan_versions"), None).unwrap();
    let check = &report.checks[0];
    assert!(!check.ok, "{report:?}");
    assert!(
        check
            .detail
            .as_deref()
            .unwrap_or_default()
            .starts_with("1 version rows"),
        "{check:?}"
    );

    assert_eq!(store.remove_orphan_versions().unwrap(), 1);

    assert_eq!(version_rows(&store, "obs-long-gone"), 0);
    assert_eq!(version_rows(&store, &saved.sync_id), 2);
    assert_eq!(version_rows(&store, &neighbour.sync_id), 1);
    let (report, _) = store.doctor_scoped(Some("orphan_versions"), None).unwrap();
    assert!(report.checks[0].ok, "{report:?}");
}

fn observation_rows(store: &Store, sync_id: &str) -> i64 {
    store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM observations WHERE sync_id = ?1",
            [sync_id],
            |row| row.get(0),
        )
        .unwrap()
}

/// The delete redacts the journal entry of the memory itself, not only its
/// versions. A peer that applied the delete first must not put the redacted
/// entry back as a live memory with an empty title and body.
#[test]
fn a_redacted_upsert_arriving_after_the_delete_creates_no_row() {
    let (_source_temp, mut source) = store();
    let (saved, _neighbour) = memory_with_two_versions(&mut source);
    source.delete_observation(saved.id, None, true).unwrap();
    let queued = source
        .list_pending_sync_mutations("cloud", &["leteo".to_owned()], 100)
        .unwrap();
    let of_the_memory = |op: &str| {
        queued
            .iter()
            .find(|mutation| {
                mutation.entity == crate::sync::ENTITY_OBSERVATION
                    && mutation.entity_key == saved.sync_id
                    && mutation.op == op
            })
            .cloned()
            .unwrap_or_else(|| panic!("no {op} mutation for the deleted memory"))
    };
    let mut upsert = of_the_memory(crate::sync::OP_UPSERT);
    let mut delete = of_the_memory(crate::sync::OP_DELETE);
    let carried: serde_json::Value = serde_json::from_str(&upsert.payload).unwrap();
    assert!(
        carried.get("title").is_none() && carried.get("content").is_none(),
        "the journal entry has to be redacted for the test to mean anything: {carried}"
    );

    let (_peer_temp, mut peer) = store();
    delete.seq = 1;
    upsert.seq = 2;
    peer.apply_pulled(&delete).unwrap();
    peer.apply_pulled(&upsert).unwrap();

    assert_eq!(
        observation_rows(&peer, &saved.sync_id),
        0,
        "a redacted upsert is not a memory"
    );
}

/// The other half of the same guard: when the delete took the session with it,
/// inserting the memory again fails the foreign key and the pull stops there.
#[test]
fn an_upsert_for_a_deleted_memory_whose_session_is_gone_completes() {
    let (_temp, mut peer) = store();
    let sync_id = "obs-deleted-with-session";
    let delete = SyncMutation {
        seq: 1,
        target_key: "cloud".to_owned(),
        entity: crate::sync::ENTITY_OBSERVATION.to_owned(),
        entity_key: sync_id.to_owned(),
        op: crate::sync::OP_DELETE.to_owned(),
        payload: serde_json::json!({
            "sync_id": sync_id,
            "session_id": "gone-with-the-delete",
            "project": "leteo",
            "hard_delete": true,
        })
        .to_string(),
        source: "remote".to_owned(),
        project: "leteo".to_owned(),
        occurred_at: "2026-08-05 04:00:00".to_owned(),
        acked_at: None,
    };
    assert!(peer.apply_pulled(&delete).unwrap());
    let upsert = SyncMutation {
        seq: 2,
        entity: crate::sync::ENTITY_OBSERVATION.to_owned(),
        entity_key: sync_id.to_owned(),
        op: crate::sync::OP_UPSERT.to_owned(),
        payload: serde_json::json!({
            "sync_id": sync_id,
            "session_id": "gone-with-the-delete",
            "type": "decision",
            "title": "Pensaba subirlo",
            "content": "y el otro equipo borro",
            "project": "leteo",
            "scope": "project",
        })
        .to_string(),
        ..delete.clone()
    };
    let applied = peer.apply_pulled(&upsert);
    assert!(
        applied.is_ok(),
        "a pull must not stall on a session the delete removed: {applied:?}"
    );
    assert_eq!(observation_rows(&peer, sync_id), 0);
}

/// The decision the issue left open, with its test: a hard delete wins over an
/// edit that arrives after it, because the person who deleted the memory cannot
/// know which other machine still holds a copy.
#[test]
fn a_full_upsert_after_the_hard_delete_does_not_bring_the_text_back() {
    let (_temp, mut peer) = store();
    peer.create_session("s1", "leteo", "/tmp/leteo").unwrap();
    let sync_id = "obs-deleted-while-edited";
    let mutation = |seq: i64, op: &str, payload: serde_json::Value| SyncMutation {
        seq,
        target_key: "cloud".to_owned(),
        entity: crate::sync::ENTITY_OBSERVATION.to_owned(),
        entity_key: sync_id.to_owned(),
        op: op.to_owned(),
        payload: payload.to_string(),
        source: "remote".to_owned(),
        project: "leteo".to_owned(),
        occurred_at: "2026-08-05 04:00:00".to_owned(),
        acked_at: None,
    };
    let text = |body: &str| {
        serde_json::json!({
            "sync_id": sync_id,
            "session_id": "s1",
            "type": "decision",
            "title": "Donde deje la clave",
            "content": body,
            "project": "leteo",
            "scope": "project",
        })
    };

    assert!(
        peer.apply_pulled(&mutation(
            1,
            crate::sync::OP_UPSERT,
            text("la clave esta en la caja")
        ))
        .unwrap()
    );
    assert_eq!(observation_rows(&peer, sync_id), 1);
    assert!(
        peer.apply_pulled(&mutation(
            2,
            crate::sync::OP_DELETE,
            serde_json::json!({
                "sync_id": sync_id,
                "session_id": "s1",
                "project": "leteo",
                "hard_delete": true,
            }),
        ))
        .unwrap()
    );
    assert_eq!(observation_rows(&peer, sync_id), 0);

    // The peer that had been editing it pushes after the delete.
    let applied = peer.apply_pulled(&mutation(
        3,
        crate::sync::OP_UPSERT,
        text("la clave esta en la caja, revisada"),
    ));
    assert!(applied.is_ok(), "{applied:?}");
    assert_eq!(
        observation_rows(&peer, sync_id),
        0,
        "delete wins: the text a person removed does not come back"
    );
}

/// The soft-delete form of the same question: the delete wins over an edit that
/// was queued before it, and only a genuinely newer edit revives the memory.
///
/// The upsert cleared `deleted_at` unconditionally, so a peer's older edit
/// brought the memory back into search and into the opening block on the
/// machine that had deleted it, while the peer — which had already applied the
/// delete — went on holding it as gone.
#[test]
fn a_pulled_upsert_older_than_the_soft_delete_stays_deleted() {
    let (_temp, mut peer) = store();
    peer.create_session("s1", "leteo", "/tmp/leteo").unwrap();
    let sync_id = "obs-soft-deleted-while-edited";
    let mutation = |seq: i64, op: &str, payload: serde_json::Value| SyncMutation {
        seq,
        target_key: "cloud".to_owned(),
        entity: crate::sync::ENTITY_OBSERVATION.to_owned(),
        entity_key: sync_id.to_owned(),
        op: op.to_owned(),
        payload: payload.to_string(),
        source: "remote".to_owned(),
        project: "leteo".to_owned(),
        occurred_at: "2026-08-05 04:00:00".to_owned(),
        acked_at: None,
    };
    let edit = |body: &str, updated_at: &str| {
        serde_json::json!({
            "sync_id": sync_id,
            "session_id": "s1",
            "type": "decision",
            "title": "Donde deje la clave",
            "content": body,
            "project": "leteo",
            "scope": "project",
            "created_at": "2026-08-05 04:00:00",
            "updated_at": updated_at,
        })
    };
    let state = |store: &Store| {
        store
            .connection
            .query_row(
                "SELECT content, deleted_at FROM observations WHERE sync_id = ?1",
                [sync_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .unwrap()
    };

    assert!(
        peer.apply_pulled(&mutation(
            1,
            crate::sync::OP_UPSERT,
            edit("la clave esta en la caja", "2026-08-05 04:00:00")
        ))
        .unwrap()
    );
    assert_eq!(state(&peer).1, None);

    assert!(
        peer.apply_pulled(&mutation(
            2,
            crate::sync::OP_DELETE,
            serde_json::json!({
                "sync_id": sync_id,
                "session_id": "s1",
                "project": "leteo",
                "deleted": true,
                "deleted_at": "2026-08-05 05:00:00",
                "hard_delete": false,
            }),
        ))
        .unwrap()
    );
    assert_eq!(
        state(&peer).1.as_deref(),
        Some("2026-08-05 05:00:00"),
        "the delete is the local decision this test is about"
    );

    assert!(
        peer.apply_pulled(&mutation(
            3,
            crate::sync::OP_UPSERT,
            edit("la clave esta en la caja, revisada", "2026-08-05 04:30:00")
        ))
        .unwrap()
    );
    assert_eq!(
        state(&peer),
        (
            "la clave esta en la caja".to_owned(),
            Some("2026-08-05 05:00:00".to_owned())
        ),
        "an edit older than the delete leaves both the delete and the text alone"
    );

    assert!(
        peer.apply_pulled(&mutation(
            4,
            crate::sync::OP_UPSERT,
            edit("la clave se movio a la caja fuerte", "2026-08-05 06:00:00")
        ))
        .unwrap()
    );
    assert_eq!(
        state(&peer),
        ("la clave se movio a la caja fuerte".to_owned(), None),
        "a strictly newer edit is the only thing that may revive it"
    );
}

/// Both machines hold the same state after each has pulled the other: the one
/// that deleted keeps it deleted, and the one that edited learns the delete.
#[test]
fn a_soft_delete_and_an_earlier_edit_land_on_the_same_state_on_both_machines() {
    let (_a_temp, mut a) = store();
    let (_b_temp, mut b) = store();
    let enrolled = ["leteo".to_owned()];
    a.create_session("s1", "leteo", "C:/a").unwrap();
    let saved = a
        .add_observation(observation(
            "s1",
            "Donde deje la clave",
            "la clave esta en la caja",
        ))
        .unwrap()
        .observation;

    for mutation in a
        .list_pending_sync_mutations("cloud", &enrolled, 100)
        .unwrap()
    {
        b.apply_pulled(&mutation).unwrap();
    }
    let on_b = b
        .recent_observations(Some("leteo"), Some(10), false)
        .unwrap()[0]
        .id;
    b.update_observation(
        on_b,
        None,
        UpdateObservation {
            content: Some("la clave esta en la caja, revisada".to_owned()),
            ..UpdateObservation::default()
        },
    )
    .unwrap();

    a.delete_observation(saved.id, None, false).unwrap();

    for mutation in b
        .list_pending_sync_mutations("cloud", &enrolled, 100)
        .unwrap()
    {
        a.apply_pulled(&mutation).unwrap();
    }
    for mutation in a
        .list_pending_sync_mutations("cloud", &enrolled, 100)
        .unwrap()
    {
        b.apply_pulled(&mutation).unwrap();
    }

    let deleted_at = |store: &Store| {
        store
            .connection
            .query_row(
                "SELECT deleted_at FROM observations WHERE sync_id = ?1",
                [&saved.sync_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .unwrap()
    };
    assert!(
        deleted_at(&a).is_some(),
        "an older edit revived the memory on the machine that deleted it"
    );
    assert!(
        deleted_at(&b).is_some(),
        "and the machine that edited learned the delete"
    );
    assert_eq!(
        a.recent_observations(Some("leteo"), Some(10), false)
            .unwrap()
            .len(),
        b.recent_observations(Some("leteo"), Some(10), false)
            .unwrap()
            .len(),
        "both stores name the same live set"
    );
}
