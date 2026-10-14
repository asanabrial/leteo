//! The Snowball index: every memory stemmed in the language it was written in.

use super::*;
use crate::settings::Interface;

fn store_in(language: Interface) -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    let mut config = StoreConfig::new(temp.path().join("leteo.db"));
    config.memory_language = Some(language);
    let mut store = Store::open(config).unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    (temp, store)
}

fn reopen_in(temp: &TempDir, language: Interface) -> Store {
    let mut config = StoreConfig::new(temp.path().join("leteo.db"));
    config.memory_language = Some(language);
    Store::open(config).unwrap()
}

/// What a question finds by the strict pass alone, so that neither a widened or
/// nearest rescue nor a corrected term can stand in for the stemmer being asked
/// about.
fn strict(store: &Store, query: &str) -> Vec<String> {
    let (found, _, corrections) = store
        .search_with_more_and_corrections(query, SearchOptions::default())
        .unwrap();
    if !corrections.is_empty() {
        return Vec::new();
    }
    found
        .into_iter()
        .filter(|result| !result.partial)
        .map(|result| result.observation.title)
        .collect()
}

fn languages(store: &Store) -> Vec<(String, String)> {
    let mut statement = store
        .connection
        .prepare(
            "SELECT o.title, s.language FROM observation_stems s
             JOIN observations o ON o.id = s.observation_id ORDER BY o.id",
        )
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Every column of every stems row, as SQLite's own text, in id order.
///
/// A snapshot rather than a field, so a change to a row the language does not
/// carry — the stems themselves, or the copied identifiers — is a difference
/// here too. `quote()` is what makes it comparable: it renders a value as the
/// bytes the table holds, NULL included, so two equal snapshots are two
/// identical rows.
fn stem_rows(store: &Store) -> Vec<String> {
    let mut statement = store
        .connection
        .prepare(
            "SELECT quote(observation_id) || '|' || quote(language) || '|'
                    || quote(title) || '|' || quote(content) || '|'
                    || quote(tool_name) || '|' || quote(type) || '|'
                    || quote(project) || '|' || quote(topic_key)
               FROM observation_stems ORDER BY observation_id",
        )
        .unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// One memory and one question per language, the question reaching the memory
/// only through another inflection of a word it holds, at least three edits from
/// it so neither `porter` nor the typo stage can answer. The English store is the
/// control, as in the Spanish test below.
#[test]
fn every_language_with_a_stemmer_finds_a_memory_by_another_inflection() {
    let cases = [
        (
            Interface::Spanish,
            "Cuánto tarda la reindexación completa",
            "reindexar",
        ),
        (
            Interface::Portuguese,
            "A reindexação completa leva quatro minutos",
            "reindexar",
        ),
        (
            Interface::French,
            "Meilisearch tolère les fautes de frappe",
            "tolérait",
        ),
        (
            Interface::German,
            "Wir müssen jeden Aufruf verarbeiten",
            "Verarbeitungen",
        ),
        (
            Interface::Italian,
            "La reindicizzazione completa dura quattro minuti",
            "reindicizzare",
        ),
        (
            Interface::Romanian,
            "Meilisearch tolerează greșeli de scriere",
            "greșelilor",
        ),
        (
            Interface::Dutch,
            "We willen alle teksten vertalen",
            "vertalingen",
        ),
        (
            Interface::Swedish,
            "Migrering av sökningen",
            "migreringarna",
        ),
    ];
    for (language, title, question) in cases {
        let (_temp, mut store) = store_in(language);
        store
            .add_observation(observation("s1", title, "Anteckning."))
            .unwrap();
        assert_eq!(strict(&store, question), [title], "{language:?}");

        let (_other, mut english) = store_in(Interface::English);
        english
            .add_observation(observation("s1", title, "Anteckning."))
            .unwrap();
        assert!(strict(&english, question).is_empty(), "{language:?}");
    }
}

/// `porter` is an English algorithm, and `ejecuta` and `ejecutaron` are two words
/// to it — three edits apart, which is past what the typo stage reads as the
/// same word, so a pass here is the stemmer and not a correction. The control is the same memory in a store that writes English: the
/// question finds nothing there, which is what makes the first half of this a
/// measurement of the Spanish stemmer and not of anything `porter` already did.
#[test]
fn a_spanish_store_finds_a_memory_by_another_inflection_of_its_word() {
    let (_temp, mut spanish) = store_in(Interface::Spanish);
    spanish
        .add_observation(observation(
            "s1",
            "El cron ejecuta la conciliación bancaria",
            "Cada noche.",
        ))
        .unwrap();
    assert_eq!(
        strict(&spanish, "ejecutaron"),
        ["El cron ejecuta la conciliación bancaria"]
    );
    assert_eq!(
        strict(&spanish, "conciliaciones"),
        ["El cron ejecuta la conciliación bancaria"]
    );

    let (_other, mut english) = store_in(Interface::English);
    english
        .add_observation(observation(
            "s1",
            "El cron ejecuta la conciliación bancaria",
            "Cada noche.",
        ))
        .unwrap();
    assert!(strict(&english, "ejecutaron").is_empty());
}

#[test]
fn a_spanish_inflection_in_the_body_is_found_as_well_as_one_in_the_title() {
    let (_temp, mut store) = store_in(Interface::Spanish);
    store
        .add_observation(observation(
            "s1",
            "Una nota",
            "La migración de los datos terminó sin errores.",
        ))
        .unwrap();
    assert_eq!(strict(&store, "migraciones"), ["Una nota"]);
}

/// The language is the one the row was written in, and it survives a change of
/// setting: a store that wrote Spanish and now writes English still holds Spanish
/// memories, and the question is stemmed for them because the *rows* say so.
#[test]
fn each_row_keeps_the_language_it_was_stemmed_in_through_a_change_of_setting() {
    let (temp, mut store) = store_in(Interface::Spanish);
    store
        .add_observation(observation("s1", "Ejecuta el cron", "Cada noche."))
        .unwrap();
    drop(store);

    let mut store = reopen_in(&temp, Interface::English);
    store
        .add_observation(observation("s1", "Runs the nightly job", "Every night."))
        .unwrap();

    assert_eq!(
        languages(&store),
        [
            ("Ejecuta el cron".to_owned(), "es".to_owned()),
            ("Runs the nightly job".to_owned(), "en".to_owned()),
        ]
    );
    assert_eq!(strict(&store, "ejecutaron"), ["Ejecuta el cron"]);
    assert_eq!(strict(&store, "running"), ["Runs the nightly job"]);
}

/// Both languages in one store, each found by its own morphology and neither
/// answering for the other.
#[test]
fn a_store_holding_both_languages_answers_each() {
    let (_temp, mut store) = store_in(Interface::Spanish);
    store
        .add_observation(observation("s1", "Ejecuta el cron", "Cada noche."))
        .unwrap();
    store
        .add_observation(observation("s1", "The cron runs nightly", "Reconciles."))
        .unwrap();
    assert_eq!(strict(&store, "ejecutaron"), ["Ejecuta el cron"]);
    assert_eq!(strict(&store, "running"), ["The cron runs nightly"]);
}

/// A revision re-stems: the words the memory used to hold stop finding it and the
/// words it holds now start to.
#[test]
fn a_revision_by_topic_key_is_stemmed_again() {
    let (_temp, mut store) = store_in(Interface::Spanish);
    let mut first = observation("s1", "Tarea", "El cron ejecuta la conciliación.");
    first.topic_key = Some("architecture/cron".to_owned());
    store.add_observation(first).unwrap();
    let mut second = observation("s1", "Tarea", "El cron sincroniza los saldos.");
    second.topic_key = Some("architecture/cron".to_owned());
    store.add_observation(second).unwrap();

    assert_eq!(strict(&store, "sincronizaron"), ["Tarea"]);
    assert!(strict(&store, "ejecutaron").is_empty());
}

#[test]
fn an_edit_through_update_is_stemmed_again() {
    let (_temp, mut store) = store_in(Interface::Spanish);
    let saved = store
        .add_observation(observation(
            "s1",
            "Tarea",
            "El cron ejecuta la conciliación.",
        ))
        .unwrap()
        .observation;
    store
        .update_observation(
            saved.id,
            None,
            UpdateObservation {
                content: Some("El cron sincroniza los saldos.".to_owned()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(strict(&store, "sincronizaron"), ["Tarea"]);
    assert!(strict(&store, "ejecutaron").is_empty());
}

/// A pulled mutation writes through its own `INSERT` and `UPDATE`, which no
/// other write path shares.
#[test]
fn a_memory_pulled_from_a_peer_is_stemmed() {
    let (_temp, mut store) = store_in(Interface::Spanish);
    let payload = |content: &str| {
        serde_json::json!({
            "sync_id": "obs-remote-1",
            "session_id": "s1",
            "type": "discovery",
            "title": "Nota remota",
            "content": content,
            "project": "leteo",
            "scope": "project",
            "created_at": "2026-01-01 00:00:00",
            "updated_at": "2026-01-01 00:00:00",
        })
        .to_string()
    };
    let mutation = |seq: i64, content: &str| SyncMutation {
        seq,
        target_key: "cloud".to_owned(),
        entity: "observation".to_owned(),
        entity_key: "obs-remote-1".to_owned(),
        op: crate::sync::OP_UPSERT.to_owned(),
        payload: payload(content),
        source: "remote".to_owned(),
        project: "leteo".to_owned(),
        occurred_at: "2026-01-01 00:00:00".to_owned(),
        acked_at: None,
    };
    store
        .apply_pulled_sync_mutation("cloud", &mutation(1, "El cron ejecuta la conciliación."))
        .unwrap();
    assert_eq!(strict(&store, "ejecutaron"), ["Nota remota"]);

    store
        .apply_pulled_sync_mutation("cloud", &mutation(2, "El cron sincroniza los saldos."))
        .unwrap();
    assert_eq!(strict(&store, "sincronizaron"), ["Nota remota"]);
    assert!(strict(&store, "ejecutaron").is_empty());
}

#[test]
fn an_import_stems_what_it_brings() {
    let (_source_temp, mut source) = store_in(Interface::Spanish);
    source
        .add_observation(observation("s1", "El cron ejecuta", "Cada noche."))
        .unwrap();
    let exported = source.export().unwrap();

    let (_target_temp, mut target) = store_in(Interface::Spanish);
    target.import(&exported).unwrap();
    assert_eq!(strict(&target, "ejecutaron"), ["El cron ejecuta"]);
    assert!(
        crate::store::schema::missing_full_text_triggers(target.connection()).is_empty(),
        "the import put every trigger back"
    );
}

#[test]
fn a_project_merge_moves_the_stems_with_the_memory() {
    let (_temp, mut store) = store_in(Interface::Spanish);
    store
        .add_observation(observation("s1", "El cron ejecuta", "Cada noche."))
        .unwrap();
    store.merge_project("leteo", "otro").unwrap();

    let narrowed = |project: &str| {
        store
            .search(
                "ejecutaron",
                SearchOptions {
                    project: Some(project.to_owned()),
                    ..Default::default()
                },
            )
            .unwrap()
            .len()
    };
    assert_eq!(narrowed("otro"), 1);
    assert_eq!(narrowed("leteo"), 0);
}

#[test]
fn deleting_a_memory_removes_its_stems_and_their_index_entries() {
    let (_temp, mut store) = store_in(Interface::Spanish);
    let saved = store
        .add_observation(observation("s1", "El cron ejecuta", "Cada noche."))
        .unwrap()
        .observation;
    store.delete_observation(saved.id, None, true).unwrap();

    assert!(languages(&store).is_empty());
    let indexed: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM observations_stemmed WHERE observations_stemmed MATCH 'ejecut'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(indexed, 0);
}

/// A repair has to put back what a lost trigger cost, and the stems are one more
/// thing a lost trigger costs: rebuilding the index over a stems table that never
/// saw the memory would leave it unfindable however often it ran.
#[test]
fn a_repair_stems_the_memories_written_while_the_triggers_were_gone() {
    let (_temp, mut store) = store_in(Interface::Spanish);
    for name in crate::store::schema::FULL_TEXT_TRIGGERS {
        store
            .connection
            .execute_batch(&format!("DROP TRIGGER IF EXISTS {name};"))
            .unwrap();
    }
    store
        .add_observation(observation("s1", "El cron ejecuta", "Cada noche."))
        .unwrap();
    assert!(strict(&store, "ejecutaron").is_empty());

    store.restore_full_text_triggers().unwrap();
    store.rebuild_full_text_indexes().unwrap();
    assert_eq!(strict(&store, "ejecutaron"), ["El cron ejecuta"]);
}

/// The migration stems the rows that were there, in the language the setting
/// names when it runs.
#[test]
fn the_migration_stems_existing_rows_in_the_language_of_the_setting() {
    let (temp, mut store) = store_in(Interface::English);
    store
        .add_observation(observation("s1", "El cron ejecuta", "Cada noche."))
        .unwrap();
    store
        .connection
        .execute_batch(
            "DROP TRIGGER obs_stems_insert; DROP TRIGGER obs_stems_update;
             DROP TRIGGER stems_fts_insert; DROP TRIGGER stems_fts_delete;
             DROP TRIGGER stems_fts_update;
             DROP TABLE observations_stemmed; DROP TABLE observation_stems;
             PRAGMA user_version = 20;",
        )
        .unwrap();
    drop(store);

    let store = reopen_in(&temp, Interface::Spanish);
    assert_eq!(
        languages(&store),
        [("El cron ejecuta".to_owned(), "es".to_owned())]
    );
    assert_eq!(strict(&store, "ejecutaron"), ["El cron ejecuta"]);
}

#[test]
fn a_connection_that_never_registered_the_functions_cannot_write_a_memory() {
    let (temp, store) = store_in(Interface::Spanish);
    drop(store);
    let raw = Connection::open(temp.path().join("leteo.db")).unwrap();
    let refused = raw
        .execute(
            "INSERT INTO observations (session_id, type, title, content, scope)
             VALUES ('s1', 'discovery', 'x', 'y', 'project')",
            [],
        )
        .unwrap_err();
    assert!(
        refused.to_string().contains("no such function"),
        "{refused}"
    );
}

#[test]
fn doctor_names_a_store_whose_stems_went_missing_and_the_repair_puts_them_back() {
    let (_temp, mut store) = store_in(Interface::Spanish);
    store
        .add_observation(observation("s1", "El cron ejecuta", "Cada noche."))
        .unwrap();
    assert!(store.doctor().unwrap().healthy);

    store
        .connection
        .execute("DELETE FROM observation_stems", [])
        .unwrap();
    let report = store.doctor().unwrap();
    assert!(
        report
            .checks
            .iter()
            .any(|check| check.code == "observation_stems_sync" && !check.ok),
        "{:?}",
        report.issues
    );

    store.rebuild_full_text_indexes().unwrap();
    assert!(store.doctor().unwrap().healthy);
    assert_eq!(strict(&store, "ejecutaron"), ["El cron ejecuta"]);
}

/// The one language the setting offers that still has no second stemmer is
/// Galician. A second gaining an arm, or Galician gaining one, is a reason to
/// revisit this.
#[test]
fn the_language_without_a_stemmer_is_the_one_the_spec_names() {
    let without: Vec<Interface> = Interface::ALL
        .into_iter()
        .filter(|language| *language != Interface::English)
        .filter(|language| crate::stemming::algorithm(*language).is_none())
        .collect();
    assert_eq!(without, [Interface::Galician]);
}

fn language_of(store: &Store, title: &str) -> String {
    store
        .connection
        .query_row(
            "SELECT s.language FROM observation_stems s JOIN observations o ON o.id = s.observation_id
             WHERE o.title = ?1",
            [title],
            |row| row.get(0),
        )
        .unwrap()
}

/// A Spanish memory in a store whose setting has since become English, which is
/// the starting point of every identifier-only test below.
fn spanish_memory_in_an_english_store() -> (TempDir, Store) {
    let (temp, mut store) = store_in(Interface::Spanish);
    store
        .add_observation(observation("s1", "El cron ejecuta", "Cada noche."))
        .unwrap();
    drop(store);
    let store = reopen_in(&temp, Interface::English);
    (temp, store)
}

#[test]
fn a_project_merge_keeps_the_language_a_memory_was_stemmed_in() {
    let (_temp, mut store) = spanish_memory_in_an_english_store();
    store.merge_project("leteo", "otro").unwrap();

    assert_eq!(language_of(&store, "El cron ejecuta"), "es");
    let options = SearchOptions {
        project: Some("otro".to_owned()),
        ..Default::default()
    };
    assert_eq!(store.search("ejecutaron", options).unwrap().len(), 1);
}

/// What adoption runs on every project spelling it carries, and the one
/// statement of that shape that reaches `observations` with the stems table
/// already there: adoption itself runs before migration 21 exists.
#[test]
fn normalising_a_project_name_keeps_the_language_a_memory_was_stemmed_in() {
    let (_temp, store) = spanish_memory_in_an_english_store();
    store
        .connection
        .execute("UPDATE OR REPLACE observations SET project = 'Otro'", [])
        .unwrap();

    assert_eq!(language_of(&store, "El cron ejecuta"), "es");
    let copied: String = store
        .connection
        .query_row("SELECT project FROM observation_stems", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(copied, "Otro");
}

/// A replicated update names every column of the row it carries, the text among
/// them, so what keeps the language is that the text did not change.
#[test]
fn a_replicated_update_that_only_moves_the_project_keeps_the_language() {
    let (temp, mut store) = store_in(Interface::Spanish);
    let mutation = |seq: i64, project: &str| SyncMutation {
        seq,
        target_key: "cloud".to_owned(),
        entity: "observation".to_owned(),
        entity_key: "obs-remote-1".to_owned(),
        op: crate::sync::OP_UPSERT.to_owned(),
        payload: serde_json::json!({
            "sync_id": "obs-remote-1",
            "session_id": "s1",
            "type": "discovery",
            "title": "Nota remota",
            "content": "El cron ejecuta la conciliación.",
            "project": project,
            "scope": "project",
            "created_at": "2026-01-01 00:00:00",
            "updated_at": "2026-01-01 00:00:00",
        })
        .to_string(),
        source: "remote".to_owned(),
        project: project.to_owned(),
        occurred_at: "2026-01-01 00:00:00".to_owned(),
        acked_at: None,
    };
    store
        .apply_pulled_sync_mutation("cloud", &mutation(1, "leteo"))
        .unwrap();
    drop(store);

    let mut store = reopen_in(&temp, Interface::English);
    store
        .apply_pulled_sync_mutation("cloud", &mutation(2, "otro"))
        .unwrap();

    assert_eq!(language_of(&store, "Nota remota"), "es");
    let options = SearchOptions {
        project: Some("otro".to_owned()),
        ..Default::default()
    };
    assert_eq!(store.search("ejecutaron", options).unwrap().len(), 1);
}

/// The other half of the rule: a write that changes the text is a new memory as
/// far as stemming goes, and takes the language of the connection that wrote it.
#[test]
fn an_edit_of_the_text_takes_the_language_of_the_connection_that_made_it() {
    let (_temp, mut store) = spanish_memory_in_an_english_store();
    let id: i64 = store
        .connection
        .query_row("SELECT id FROM observations", [], |row| row.get(0))
        .unwrap();
    store
        .update_observation(
            id,
            None,
            UpdateObservation {
                content: Some("Cada madrugada.".to_owned()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(language_of(&store, "El cron ejecuta"), "en");
}

/// `UPDATE OF content` fires for a statement that names the column whether or
/// not its value changed, so the guard is on the values.
#[test]
fn a_statement_that_names_the_text_without_changing_it_keeps_the_language() {
    let (_temp, store) = spanish_memory_in_an_english_store();
    store
        .connection
        .execute(
            "UPDATE observations SET title = title, content = content, type = 'decision'",
            [],
        )
        .unwrap();
    assert_eq!(language_of(&store, "El cron ejecuta"), "es");
}

/// Foreign keys are on for every connection of this crate, so the cascade keeps
/// the table level; a tool that wrote with them off can leave a row whose memory
/// is gone, and the report must be able to say so and the repair to clear it.
#[test]
fn a_stems_row_whose_memory_is_gone_is_reported_and_the_repair_removes_it() {
    let (_temp, mut store) = store_in(Interface::Spanish);
    store
        .add_observation(observation("s1", "El cron ejecuta", "Cada noche."))
        .unwrap();
    store
        .connection
        .execute_batch(
            "PRAGMA foreign_keys = OFF;
             INSERT INTO observation_stems(observation_id, language, title, content)
             VALUES (9999, 'es', 'huerfano', 'huerfano');
             PRAGMA foreign_keys = ON;",
        )
        .unwrap();
    let report = store.doctor().unwrap();
    assert!(
        report
            .checks
            .iter()
            .any(|check| check.code == "observation_stems_sync" && !check.ok),
        "{:?}",
        report.issues
    );

    store.rebuild_full_text_indexes().unwrap();
    assert!(store.doctor().unwrap().healthy);
    assert_eq!(languages(&store).len(), 1);
}

/// A repair of a level store writes nothing, which is what keeps it from firing
/// the index triggers once for every memory ahead of the rebuild. The writes are
/// counted by temporary triggers, because `total_changes` also counts what the
/// rebuild writes into the index.
#[test]
fn a_repair_of_a_store_that_is_level_writes_no_stems() {
    let (_temp, mut store) = store_in(Interface::Spanish);
    store
        .add_observation(observation("s1", "El cron ejecuta", "Cada noche."))
        .unwrap();
    store
        .connection
        .execute_batch(
            "CREATE TEMP TABLE stem_writes(n INTEGER);
             CREATE TEMP TRIGGER count_stem_inserts AFTER INSERT ON main.observation_stems
             BEGIN INSERT INTO stem_writes VALUES (1); END;
             CREATE TEMP TRIGGER count_stem_updates AFTER UPDATE ON main.observation_stems
             BEGIN INSERT INTO stem_writes VALUES (1); END;",
        )
        .unwrap();
    crate::store::schema::rebuild_present_indexes(&store.connection).unwrap();
    let writes: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM stem_writes", [], |row| row.get(0))
        .unwrap();
    assert_eq!(writes, 0);
}

/// The triggers restored after an import are the ones the migrations define,
/// including the one migration 22 replaced.
#[test]
fn the_restore_reads_the_newest_definition_of_a_trigger() {
    let sql = crate::store::schema::full_text_trigger_sql("obs_stems_update").unwrap();
    assert!(sql.contains("WHEN new.title IS NOT old.title"), "{sql}");
    assert!(crate::store::schema::full_text_trigger_sql("obs_stems_identifiers").is_some());
}

/// A change of the memory-writing setting moves nothing already stored.
///
/// The rows keep the language they were written under — which is what the
/// wizard warns about and `doctor` reports — and this is the guard on that
/// promise, at the bytes rather than at the per-row code the test above reads.
/// It fails if a repair ever re-stems an existing row under the setting in
/// force instead of the language the row was written in.
#[test]
fn a_change_of_the_memory_language_setting_moves_no_row_that_is_already_stored() {
    let temp = TempDir::new().unwrap();
    let setting = |language: Interface| {
        crate::settings::save(
            temp.path(),
            &crate::settings::Settings {
                language: Some(language.as_str().to_owned()),
                ..crate::settings::Settings::default()
            },
        )
        .unwrap();
    };

    // A store that wrote Spanish and now writes English, through the settings
    // file the wizard writes rather than through the test's own override.
    setting(Interface::Spanish);
    let mut store = Store::open(StoreConfig::new(temp.path().join("leteo.db"))).unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    store
        .add_observation(observation("s1", "Ejecuta el cron", "Cada noche."))
        .unwrap();
    drop(store);

    setting(Interface::English);
    let mut store = Store::open(StoreConfig::new(temp.path().join("leteo.db"))).unwrap();
    store
        .add_observation(observation("s1", "Runs the nightly job", "Every night."))
        .unwrap();
    let before = stem_rows(&store);
    assert!(
        before.iter().any(|row| row.contains("'es'"))
            && before.iter().any(|row| row.contains("'en'")),
        "the store has to hold both languages for this to prove anything: {before:?}"
    );
    drop(store);

    // The setting changes again, and the repair path runs over the same rows.
    setting(Interface::French);
    let reopened = Store::open(StoreConfig::new(temp.path().join("leteo.db"))).unwrap();
    reopened.rebuild_full_text_indexes().unwrap();

    assert_eq!(
        before,
        stem_rows(&reopened),
        "a change of setting re-stemmed rows it did not write"
    );
}
