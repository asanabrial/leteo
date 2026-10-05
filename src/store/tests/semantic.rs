//! The semantic stage: found by meaning, under the same visibility as the rest.

use super::*;
use crate::memory::model::{JudgeBySemanticParams, SESSION_SUMMARY};

const JWT_TITLE: &str = "Rotate JWT signing keys every 30 days with kid header";
const JWT_BODY: &str = "Signing keys live in KMS and tokens carry a kid header. The verifier \
    accepts the current and the previous key for a 48 hour overlap window.";

/// The same thing asked in Spanish, which shares no word with `JWT_*` and is
/// nearer to it than to anything else in these stores (0.38 against 0.07).
const ASKED_IN_SPANISH: &str = "rotación de las claves de firma";

/// Nothing in these stores is about this, whatever the model makes of it.
const UNRELATED: &str = "quarterly marketing budget forecast";

fn on() -> SearchOptions {
    SearchOptions {
        semantic: true,
        ..SearchOptions::default()
    }
}

fn vector_rows(store: &Store) -> i64 {
    store
        .connection
        .query_row("SELECT COUNT(*) FROM observation_vectors", [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn ids(found: &[SearchResult]) -> Vec<i64> {
    found.iter().map(|hit| hit.observation.id).collect()
}

/// A store that can use the model, or nothing in a tree that does not carry it.
fn model_store() -> Option<(TempDir, Store)> {
    let model = crate::semantic::tests::repository_model().or_else(|| {
        eprintln!("skipped: this tree has no assets/model (the packaged crate ships none)");
        None
    })?;
    let temp = TempDir::new().unwrap();
    let mut config = StoreConfig::new(temp.path().join("leteo.db"));
    config.model_dir = Some(model);
    let mut store = Store::open(config).unwrap();
    store.enroll_project("leteo").unwrap();
    Some((temp, store))
}

/// A store holding the key-rotation memory, a few that have nothing to do with
/// it, and the session that owns them.
fn store_with_keys() -> Option<(TempDir, Store, Observation)> {
    let (temp, mut store) = model_store()?;
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let keys = store
        .add_observation(observation("s1", JWT_TITLE, JWT_BODY))
        .unwrap()
        .observation;
    for (title, body) in [
        (
            "Product catalog cache stampede on cold start",
            "When the catalog cache expired, hundreds of requests recomputed it at once.",
        ),
        (
            "How to bake sourdough bread",
            "Mix flour and water, let the starter ferment overnight, bake in a hot oven.",
        ),
        (
            "Fixed connection pool exhaustion under load",
            "Every HTTP handler opened its own pool instead of sharing one.",
        ),
    ] {
        store
            .add_observation(observation("s1", title, body))
            .unwrap();
    }
    Some((temp, store, keys))
}

/// The fixture is only a fixture if the words really find nothing: every test
/// below that expects the stage to answer begins here, so a change to the
/// lexical stages that started answering these questions fails loudly in one
/// place instead of making a dozen assertions about the semantic stage vacuous.
#[test]
fn the_questions_these_tests_ask_are_empty_for_the_words_alone() {
    let Some((_temp, store, _)) = store_with_keys() else {
        return;
    };
    for question in [ASKED_IN_SPANISH, UNRELATED] {
        assert!(
            store
                .search(question, SearchOptions::default())
                .unwrap()
                .is_empty(),
            "{question}"
        );
        assert!(
            store.search(question, on()).unwrap().len() <= 1,
            "{question}"
        );
    }
}

/// An empty answer is rescued by meaning, marked as such, and only above the
/// floor: the other question in the store has no neighbour that close, and it
/// stays empty rather than being answered with the least-bad memory.
#[test]
fn an_empty_answer_is_rescued_by_meaning_above_the_floor_and_by_nothing_below_it() {
    let Some((_temp, store, keys)) = store_with_keys() else {
        return;
    };
    let found = store.search(ASKED_IN_SPANISH, on()).unwrap();
    assert_eq!(ids(&found), vec![keys.id], "{found:?}");
    assert!(found[0].semantic && !found[0].partial, "{found:?}");
    assert!(
        found[0].rank < 0.0,
        "rank is on the negative scale: {found:?}"
    );

    assert!(
        store.search(UNRELATED, on()).unwrap().is_empty(),
        "nothing is near enough to be an answer"
    );
}

/// The stage reads the floor from one place, and the answer to a question that
/// is only just under it is the same silence.
#[test]
fn the_floor_is_the_published_one() {
    let Some((_temp, store, _)) = store_with_keys() else {
        return;
    };
    let candidates = store
        .semantic_candidates(ASKED_IN_SPANISH, &on(), 10, None)
        .unwrap();
    assert!(candidates.len() > 1, "no floor, so the page fills");
    assert!(
        candidates
            .iter()
            .any(|candidate| -candidate.rank < f64::from(crate::semantic::FLOOR)),
        "{candidates:?}"
    );
    let floored = store
        .semantic_candidates(ASKED_IN_SPANISH, &on(), 10, Some(crate::semantic::FLOOR))
        .unwrap();
    assert!(
        floored
            .iter()
            .all(|candidate| -candidate.rank >= f64::from(crate::semantic::FLOOR)),
        "{floored:?}"
    );
    assert!(floored.len() < candidates.len());
}

/// A setting off, or a caller that does not ask, is today's search: the same
/// answer, and not one byte written.
#[test]
fn without_the_stage_the_search_is_the_lexical_one_and_writes_nothing() {
    let Some((_temp, store, _)) = store_with_keys() else {
        return;
    };
    assert!(
        store
            .search(ASKED_IN_SPANISH, SearchOptions::default())
            .unwrap()
            .is_empty()
    );
    assert_eq!(vector_rows(&store), 0);

    // `mode: any` asked for a disjunction and gets one, the way every relaxed
    // stage is switched off there.
    let any = SearchOptions {
        mode: SearchMode::Any,
        ..on()
    };
    assert!(store.search(ASKED_IN_SPANISH, any).unwrap().is_empty());
    assert_eq!(vector_rows(&store), 0);
}

/// Every stage above `nearest` is stronger than a cosine, and a search one of
/// them answers neither changes nor embeds anything.
#[test]
fn a_question_a_stronger_stage_answers_is_never_touched() {
    let Some((_temp, store, keys)) = store_with_keys() else {
        return;
    };
    for question in [
        "rotate signing keys",  // every word: the strict pass
        "rotat",                // a word's beginning: the prefix stage
        "rotate signing kyes",  // a typo
        "rotate signing zzzzz", // all but one word: the widened stage
    ] {
        let without = store.search(question, SearchOptions::default()).unwrap();
        let with = store.search(question, on()).unwrap();
        assert_eq!(without, with, "{question}");
        assert!(with.iter().all(|hit| !hit.semantic), "{question}");
    }
    assert!(
        store
            .search("rotate signing keys", on())
            .unwrap()
            .iter()
            .any(|hit| hit.observation.id == keys.id)
    );
    assert_eq!(
        vector_rows(&store),
        0,
        "a search that answered did not so much as load the model"
    );
}

/// On the weakest lexical answer the semantic list is merged in, by place, with
/// no floor — and the memory it adds is marked, the ones the words found are
/// not.
#[test]
fn a_nearest_answer_is_merged_with_the_semantic_list_and_only_what_it_adds_is_marked() {
    let Some((_temp, mut store, keys)) = store_with_keys() else {
        return;
    };
    // One note that is all `service` and three that mention it once among a
    // lot of other words: `nearest` keeps only what stands out from the median
    // of what matched, and four identical notes would have nothing that does.
    store
        .add_observation(observation(
            "s1",
            "Service",
            "service service service service",
        ))
        .unwrap();
    for index in 0..3 {
        store
            .add_observation(observation(
                "s1",
                &format!("Note {index}"),
                &format!(
                    "alpha{index} beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu \
                     xi omicron pi rho sigma tau upsilon phi chi psi omega service restarted"
                ),
            ))
            .unwrap();
    }
    // `service` is the one word anything holds, so the strict pass, the
    // fragments and the widening all find nothing and `nearest` answers with
    // the notes; the key-rotation memory has none of the words.
    let question = format!("service {ASKED_IN_SPANISH}");
    let without = store.search(&question, SearchOptions::default()).unwrap();
    assert!(
        !without.is_empty() && without.iter().all(|hit| hit.partial),
        "{without:?}"
    );
    assert!(!ids(&without).contains(&keys.id));

    let options = SearchOptions {
        limit: Some(10),
        ..on()
    };
    let with = store.search(&question, options).unwrap();
    let added = with
        .iter()
        .find(|hit| hit.observation.id == keys.id)
        .unwrap_or_else(|| panic!("the memory by meaning is on the page: {with:?}"));
    assert!(added.semantic && !added.partial, "{added:?}");
    assert!(
        with.iter().filter(|hit| hit.semantic).count() > 1,
        "there is no floor on this path: the nearest memories fill the page behind it, \
         whatever their cosine — {with:?}"
    );

    assert!(
        with.windows(2).all(|pair| pair[0].rank <= pair[1].rank),
        "one scale for the page, best first: {:?}",
        with.iter().map(|hit| hit.rank).collect::<Vec<_>>()
    );
    // Every row says why it is there, and never both: found by its words, or
    // added by meaning. The list has no floor, so the page fills with the
    // nearest memories behind the one that matters, and they are marked too.
    for hit in &with {
        assert!(hit.partial != hit.semantic, "{hit:?}");
    }
    assert!(
        with.iter().any(|hit| hit.partial),
        "the words still count: {with:?}"
    );
}

/// A topic key is an exact lookup, answered first and complete. The stage has
/// nothing to add to it and does not run.
#[test]
fn a_topic_key_answer_is_exact_and_the_stage_stays_out_of_it() {
    let Some((_temp, mut store)) = model_store() else {
        return;
    };
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let mut keyed = observation("s1", JWT_TITLE, JWT_BODY);
    keyed.topic_key = Some("architecture/auth-tokens".to_owned());
    let keyed = store.add_observation(keyed).unwrap().observation;
    store
        .add_observation(observation(
            "s1",
            "How to bake sourdough bread",
            "flour and water",
        ))
        .unwrap();

    let found = store.search("architecture/auth-tokens", on()).unwrap();
    assert_eq!(ids(&found), vec![keyed.id], "{found:?}");
    assert!(found.iter().all(|hit| !hit.semantic));
    assert_eq!(vector_rows(&store), 0);

    // With the indexes emptied the full-text stages find nothing, so the lookup
    // is the only thing that answered and the stage would be next in line —
    // the state in which "the stage stays out" is a decision and not a
    // consequence of the words having matched.
    store
        .connection
        .execute_batch(
            "INSERT INTO observations_fts(observations_fts) VALUES('delete-all');
             INSERT INTO observations_exact(observations_exact) VALUES('delete-all');",
        )
        .unwrap();
    let found = store.search("architecture/auth-tokens", on()).unwrap();
    assert_eq!(ids(&found), vec![keyed.id], "{found:?}");
    assert_eq!(
        vector_rows(&store),
        0,
        "the lookup answered, so nothing was embedded"
    );
}

/// The answer the semantic stage is not allowed to give: a memory the rest of
/// search has hidden.
#[test]
fn hidden_memories_never_surface_by_meaning() {
    let Some((_temp, mut store, keys)) = store_with_keys() else {
        return;
    };
    assert_eq!(
        ids(&store.search(ASKED_IN_SPANISH, on()).unwrap()),
        vec![keys.id]
    );

    // Superseded by a judged verdict: out of date rather than gone.
    let newer = store
        .add_observation(observation(
            "s1",
            "Quarterly roadmap",
            "plans for next quarter",
        ))
        .unwrap()
        .observation;
    store
        .judge_by_semantic(JudgeBySemanticParams {
            source_id: newer.sync_id.clone(),
            target_id: keys.sync_id.clone(),
            relation: "supersedes".to_owned(),
            confidence: Some(0.9),
            reasoning: Some("replaced".to_owned()),
            ..Default::default()
        })
        .unwrap();
    assert!(
        store.search(ASKED_IN_SPANISH, on()).unwrap().is_empty(),
        "a superseded memory is hidden from every stage, and this is one"
    );

    // Deleted: soft, and then for good — which takes its vector with it.
    let Some((_temp, mut store, keys)) = store_with_keys() else {
        return;
    };
    assert_eq!(
        ids(&store.search(ASKED_IN_SPANISH, on()).unwrap()),
        vec![keys.id]
    );
    store.delete_observation(keys.id, None, false).unwrap();
    assert!(store.search(ASKED_IN_SPANISH, on()).unwrap().is_empty());
    store.delete_observation(keys.id, None, true).unwrap();
    let left: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM observation_vectors WHERE observation_id = ?1",
            [keys.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(left, 0, "a hard delete cascades to the vector");
}

/// Session summaries are long and touch everything, which is why every relaxed
/// stage leaves them out; this one neither returns one nor spends a vector on it.
///
/// Both halves are asserted separately, because each hides the other: with no
/// vector for a summary the scan has nothing to return, so a scan that forgot
/// to exclude them would pass. A summary is therefore given a vector by hand —
/// a copy of the best match's — and has to stay out of the answer on the
/// strength of its type alone.
#[test]
fn a_session_summary_is_neither_returned_nor_embedded() {
    let Some((_temp, mut store, keys)) = store_with_keys() else {
        return;
    };
    let mut summary = observation("s1", JWT_TITLE, JWT_BODY);
    summary.kind = SESSION_SUMMARY.to_owned();
    let summary = store.add_observation(summary).unwrap().observation;
    let found = store.search(ASKED_IN_SPANISH, on()).unwrap();
    assert_eq!(ids(&found), vec![keys.id], "{found:?}");
    let embedded = |store: &Store| -> i64 {
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM observation_vectors WHERE observation_id = ?1",
                [summary.id],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(embedded(&store), 0, "no vector is made for a summary");

    store
        .connection
        .execute(
            "INSERT INTO observation_vectors (observation_id, model, source_key, vector)
             SELECT s.id, k.model, ifnull(s.normalized_hash, '') || '|' || s.title, k.vector
               FROM observations s, observation_vectors k
              WHERE s.id = ?1 AND k.observation_id = ?2",
            [summary.id, keys.id],
        )
        .unwrap();
    assert_eq!(embedded(&store), 1);
    let found = store.search(ASKED_IN_SPANISH, on()).unwrap();
    assert_eq!(
        ids(&found),
        vec![keys.id],
        "a summary with a perfect vector still is not an answer: {found:?}"
    );
}

/// Hiding a deleted memory is done twice — by the clause every stage reads, and
/// again by the fetch — so a test that only asks whether a deleted memory comes
/// back cannot tell whether the first of them works. What the first protects is
/// the page: if it is missing, the best match is a deleted memory, the page of
/// one is spent on it, the fetch throws it away, and a live memory that answers
/// the question is not returned.
#[test]
fn a_deleted_best_match_does_not_use_up_the_page() {
    let Some((_temp, mut store, keys)) = store_with_keys() else {
        return;
    };
    let second = store
        .add_observation(observation(
            "s1",
            "Key rotation schedule",
            "Rotate the signing keys on a fixed schedule; keep the old key during the overlap.",
        ))
        .unwrap()
        .observation;
    let both = store
        .semantic_candidates(ASKED_IN_SPANISH, &on(), 10, Some(crate::semantic::FLOOR))
        .unwrap();
    assert_eq!(both.len(), 2, "both memories are above the floor: {both:?}");
    let best = both[0].id;
    let other = if best == keys.id { second.id } else { keys.id };

    store.delete_observation(best, None, false).unwrap();
    let one = SearchOptions {
        limit: Some(1),
        ..on()
    };
    let found = store.search(ASKED_IN_SPANISH, one).unwrap();
    assert_eq!(ids(&found), vec![other], "{found:?}");
}

/// The narrowing a caller asked for is the narrowing the stage applies, and the
/// retry that lifts the project narrowing finds what the first call could not.
#[test]
fn the_stage_keeps_the_projects_it_is_asked_about_and_widens_when_asked() {
    let Some((_temp, mut store, keys)) = store_with_keys() else {
        return;
    };
    store.create_session("s2", "elsewhere", "C:/other").unwrap();
    let mut other = observation(
        "s2",
        "Rotate the staging certificates",
        "renew them every quarter",
    );
    other.project = Some("Elsewhere".to_owned());
    let other = store.add_observation(other).unwrap().observation;

    let here = SearchOptions {
        project: Some("leteo".to_owned()),
        ..on()
    };
    assert_eq!(
        ids(&store.search(ASKED_IN_SPANISH, here).unwrap()),
        vec![keys.id]
    );

    // Empty here, and the same question with the narrowing lifted is what the
    // two surfaces ask next to say "N elsewhere". It runs the stage too, so
    // the number they report is the number `--all-projects` would return.
    let there = SearchOptions {
        project: Some("elsewhere".to_owned()),
        ..on()
    };
    let narrow = store.search(ASKED_IN_SPANISH, there).unwrap();
    assert!(!ids(&narrow).contains(&keys.id), "{narrow:?}");
    let widened = store.search(ASKED_IN_SPANISH, on()).unwrap();
    assert!(ids(&widened).contains(&keys.id), "{widened:?}");
    assert!(widened.iter().all(|hit| hit.semantic));
    let _ = other;

    let typed = SearchOptions {
        kind: Some("bugfix".to_owned()),
        ..on()
    };
    assert!(store.search(ASKED_IN_SPANISH, typed).unwrap().is_empty());
}

/// A vector made from text that has since changed is replaced the next time
/// the stage fires, whichever write path changed it — the stage reads a key
/// off the row, and no write path has to remember to say so.
#[test]
fn a_vector_whose_text_changed_is_made_again() {
    let Some((_temp, mut store, keys)) = store_with_keys() else {
        return;
    };
    assert_eq!(
        ids(&store.search(ASKED_IN_SPANISH, on()).unwrap()),
        vec![keys.id]
    );
    let stored = |store: &Store| -> (String, Vec<u8>) {
        store
            .connection
            .query_row(
                "SELECT source_key, vector FROM observation_vectors WHERE observation_id = ?1",
                [keys.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
    };
    let (key_before, vector_before) = stored(&store);

    // The body changes to something else entirely.
    store
        .update_observation(
            keys.id,
            None,
            UpdateObservation {
                title: Some("How to bake sourdough bread".to_owned()),
                content: Some(
                    "Mix flour and water and let the starter ferment overnight.".to_owned(),
                ),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        store.search(ASKED_IN_SPANISH, on()).unwrap().is_empty(),
        "the old vector would still have answered"
    );
    let (key_after, vector_after) = stored(&store);
    assert_ne!(key_before, key_after);
    assert_ne!(vector_before, vector_after);

    // The title alone, which the content hash does not cover.
    store
        .update_observation(
            keys.id,
            None,
            UpdateObservation {
                title: Some(JWT_TITLE.to_owned()),
                content: Some(JWT_BODY.to_owned()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        ids(&store.search(ASKED_IN_SPANISH, on()).unwrap()),
        vec![keys.id]
    );
    store
        .update_observation(
            keys.id,
            None,
            UpdateObservation {
                title: Some("How to bake sourdough bread".to_owned()),
                ..Default::default()
            },
        )
        .unwrap();
    let (key_title, _) = stored(&store);
    store.search(ASKED_IN_SPANISH, on()).unwrap();
    let (key_title_after, _) = stored(&store);
    assert_ne!(
        key_title, key_title_after,
        "a changed title is a stale vector"
    );

    // A vector from another model is stale whatever it was made from.
    store
        .connection
        .execute("UPDATE observation_vectors SET model = 'another'", [])
        .unwrap();
    store.search(ASKED_IN_SPANISH, on()).unwrap();
    let models: Vec<String> = store
        .connection
        .prepare("SELECT DISTINCT model FROM observation_vectors")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(models, vec![crate::semantic::MODEL_ID.to_owned()]);
}

/// A vector that was already current is not made again: the second question
/// does not write.
#[test]
fn a_current_vector_is_left_alone() {
    let Some((_temp, store, _)) = store_with_keys() else {
        return;
    };
    store.search(ASKED_IN_SPANISH, on()).unwrap();
    let written = vector_rows(&store);
    assert!(written >= 4);
    let before = store.connection.total_changes();
    store.search(ASKED_IN_SPANISH, on()).unwrap();
    assert_eq!(store.connection.total_changes(), before);
}

/// A store that cannot be written still answers: the vectors are made for the
/// question and thrown away, nothing is kept, and the connection's patience is
/// what it was.
#[test]
fn a_store_that_cannot_keep_vectors_still_answers() {
    let Some((_temp, store, keys)) = store_with_keys() else {
        return;
    };
    let patience = |store: &Store| -> i64 {
        store
            .connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap()
    };
    let before = patience(&store);
    store
        .connection
        .execute_batch("PRAGMA query_only = ON")
        .unwrap();
    let found = store.search(ASKED_IN_SPANISH, on()).unwrap();
    assert_eq!(ids(&found), vec![keys.id], "{found:?}");
    assert_eq!(vector_rows(&store), 0);
    assert_eq!(patience(&store), before);
}

/// Every surface that searches turns the stage on the same way: from the
/// setting, on unless somebody turned it off.
#[test]
fn the_setting_decides_and_defaults_to_on() {
    let temp = TempDir::new().unwrap();
    assert!(crate::settings::load(temp.path()).semantic_search());
    std::fs::write(
        crate::settings::path_in(temp.path()),
        r#"{"semantic_search": false}"#,
    )
    .unwrap();
    assert!(!crate::settings::load(temp.path()).semantic_search());
    std::fs::write(
        crate::settings::path_in(temp.path()),
        r#"{"semantic_search": "no"}"#,
    )
    .unwrap();
    assert!(
        crate::settings::load(temp.path()).semantic_search(),
        "a value that cannot be read is a setting nobody made"
    );
}

#[test]
fn the_migration_creates_the_table_and_a_store_before_it_is_carried_forward() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("leteo.db");
    {
        let store = Store::open(StoreConfig::new(path.clone())).unwrap();
        store
            .connection
            .execute_batch("DROP TABLE observation_vectors; PRAGMA user_version = 19;")
            .unwrap();
    }
    let store = Store::open(StoreConfig::new(path)).unwrap();
    assert_eq!(schema_version(&store.connection).unwrap(), SCHEMA_VERSION);
    let columns = table_info(&store.connection, "observation_vectors").unwrap();
    let names: Vec<&str> = columns.iter().map(|column| column.name.as_str()).collect();
    assert_eq!(names, ["observation_id", "model", "source_key", "vector"]);
}

/// No model, no stage -- and nothing written, nothing failed, and the answer the
/// words give. The state of every install that has not fetched it yet.
#[test]
fn without_a_model_the_stage_is_off_and_the_search_is_the_lexical_one() {
    let (temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    store
        .add_observation(observation("s1", JWT_TITLE, JWT_BODY))
        .unwrap();
    assert!(store.search(ASKED_IN_SPANISH, on()).unwrap().is_empty());
    assert_eq!(vector_rows(&store), 0);
    assert!(matches!(
        crate::semantic::status(temp.path(), None),
        crate::semantic::Status::Missing(_)
    ));
    assert!(
        store
            .search(JWT_TITLE, on())
            .unwrap()
            .iter()
            .all(|hit| !hit.semantic),
        "and the words still find what they find"
    );
}

/// A model that is not the one this build accepts is never loaded: one byte of
/// the weights flipped and the stage is off, with nothing written.
#[test]
fn a_model_that_does_not_verify_is_never_loaded() {
    let Some((temp, mut store)) = model_store() else {
        return;
    };
    let copy = temp.path().join("copy");
    std::fs::create_dir_all(&copy).unwrap();
    for (name, _) in crate::semantic::MODEL_FILES {
        std::fs::copy(
            crate::semantic::tests::repository_model()
                .unwrap()
                .join(name),
            copy.join(name),
        )
        .unwrap();
    }
    let mut weights = std::fs::read(copy.join("model.safetensors")).unwrap();
    weights[200] ^= 0x01;
    std::fs::write(copy.join("model.safetensors"), weights).unwrap();
    store.set_model_dir_for_tests(Some(copy));

    store.create_session("s1", "leteo", "C:/repo").unwrap();
    store
        .add_observation(observation("s1", JWT_TITLE, JWT_BODY))
        .unwrap();
    assert!(store.search(ASKED_IN_SPANISH, on()).unwrap().is_empty());
    assert_eq!(vector_rows(&store), 0);
}

/// More memories than one chunk are all embedded and kept, across the boundary.
#[test]
fn a_first_firing_larger_than_a_chunk_embeds_everything() {
    let Some((_temp, mut store)) = model_store() else {
        return;
    };
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    store
        .add_observation(observation("s1", JWT_TITLE, JWT_BODY))
        .unwrap();
    for index in 0..300 {
        store
            .add_observation(observation(
                "s1",
                &format!("Memory number {index}"),
                &format!("a note about subject {index} and nothing else"),
            ))
            .unwrap();
    }
    store.search(ASKED_IN_SPANISH, on()).unwrap();
    assert_eq!(vector_rows(&store), 301);
}

/// The first firing writes a chunk at a time, which is what bounds its memory:
/// 301 memories are two commits of the vectors, not one. Counted from SQLite's
/// own commit hook, because the bound is a property of how the work is divided
/// and a test of the answer cannot see how it was.
#[test]
fn the_first_firing_keeps_its_vectors_a_chunk_at_a_time() {
    let Some((_temp, mut store)) = model_store() else {
        return;
    };
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    for index in 0..301 {
        store
            .add_observation(observation(
                "s1",
                &format!("Memory number {index}"),
                &format!("a note about subject {index} and nothing else"),
            ))
            .unwrap();
    }
    let commits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = std::sync::Arc::clone(&commits);
    store
        .connection
        .commit_hook(Some(move || {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            false
        }))
        .unwrap();
    let counted = |store: &Store, options: SearchOptions| {
        let before = commits.load(std::sync::atomic::Ordering::SeqCst);
        store.search(ASKED_IN_SPANISH, options).unwrap();
        commits.load(std::sync::atomic::Ordering::SeqCst) - before
    };
    // The lexical stages commit too (the typo stage's vocabulary is a temporary
    // table), so the stage's own commits are what it adds to a search without it.
    counted(&store, SearchOptions::default());
    let without = counted(&store, SearchOptions::default());
    let with = counted(&store, on());
    store.connection.commit_hook(None::<fn() -> bool>).unwrap();
    assert_eq!(vector_rows(&store), 301);
    assert_eq!(
        with - without,
        2,
        "301 memories are one chunk of 256 and one of 45"
    );
}
