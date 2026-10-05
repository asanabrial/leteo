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
    peer.apply_pulled_sync_mutation("cloud", &version_mutation)
        .unwrap();

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
    peer.apply_pulled_sync_mutation("cloud", &replay).unwrap();
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
    store
        .apply_pulled_sync_mutation("cloud", &mutation)
        .unwrap();

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
