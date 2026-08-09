//! Finding one again.

use super::*;

#[test]
fn migrates_old_fts_and_additive_columns_idempotently() {
    let (_temp, config) = legacy_database(PRE_CONFLICT_SCHEMA_WITH_OLD_FTS);
    let store = Store::open(config.clone()).unwrap();

    let observation = store.get_observation(1).unwrap();
    assert_eq!(observation.sync_id, "obs-pre-conflict");
    assert_eq!(
        observation.content,
        "Normalized tokenizer panic on edge case"
    );
    assert_eq!(observation.scope, "project");
    assert_eq!(observation.topic_key, None);
    assert_eq!(observation.revision_count, 1);
    assert_eq!(observation.duplicate_count, 1);
    assert_eq!(observation.updated_at, "2024-03-01 10:00:00");
    assert!(!observation.pinned);
    assert!(observation.review_after.is_none());

    let fts_columns = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_xinfo('observations_fts')
                 WHERE name = 'topic_key'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    assert_eq!(fts_columns, 1);

    // Adoption builds the index the old way and stamps version 1; the
    // numbered migrations then run in order. A database of unknown
    // provenance has to come out the far end stemmed like any other, or
    // an imported store would quietly search worse than a native one.
    let definition: String = store
        .connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name = 'observations_fts'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        definition.contains("porter"),
        "an adopted database ends up stemmed: {definition}"
    );
    assert_eq!(schema_version(&store.connection).unwrap(), SCHEMA_VERSION);
    assert_eq!(
        store
            .search("tokenizer panic", SearchOptions::default())
            .unwrap()[0]
            .observation
            .sync_id,
        "obs-pre-conflict"
    );

    drop(store);
    let mut reopened = Store::open(config).unwrap();
    assert_eq!(
        reopened.get_observation(1).unwrap().sync_id,
        "obs-pre-conflict"
    );
    assert_eq!(
        reopened
            .search("tokenizer", SearchOptions::default())
            .unwrap()
            .len(),
        1
    );

    // Rebuilding the index for the new column drops the full-text triggers,
    // and the baseline finalize step is what puts them back. The rows above
    // were indexed by the rebuild itself, so only a write made afterwards
    // shows whether the triggers actually returned.
    let written = reopened
        .add_observation(AddObservation {
            session_id: "pre-conflict".to_owned(),
            kind: "discovery".to_owned(),
            title: "Cormorant".to_owned(),
            content: "written after the migration".to_owned(),
            tool_name: None,
            project: Some("engram".to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            prompt_sync_id: None,
        })
        .unwrap();
    let hits = reopened
        .search("Cormorant", SearchOptions::default())
        .unwrap();
    assert_eq!(
        hits.len(),
        1,
        "a row written after the migration was not indexed"
    );
    assert_eq!(hits[0].observation.id, written.observation.id);
}

#[test]
fn the_full_text_triggers_follow_every_write() {
    // The index is maintained by triggers, and their definitions live in
    // one migration file while the migration code drops them and relies on
    // that file to put them back. Nothing else here would notice a write
    // path whose trigger went missing: the row would save cleanly and
    // simply never be findable.
    let (_temp, mut store) = store();
    store.create_session("s1", "Leteo", "C:/repo").unwrap();
    let saved = store
        .add_observation(observation("s1", "Kingfisher", "the original body"))
        .unwrap();
    let id = saved.observation.id;

    let hits = store
        .search("Kingfisher", SearchOptions::default())
        .unwrap();
    assert_eq!(hits.len(), 1, "the insert trigger did not index the row");

    store
        .update_observation(
            id,
            UpdateObservation {
                title: Some("Bittern".to_owned()),
                ..UpdateObservation::default()
            },
        )
        .unwrap();
    assert_eq!(
        store
            .search("Bittern", SearchOptions::default())
            .unwrap()
            .len(),
        1,
        "the update trigger did not index the new title"
    );
    assert!(
        store
            .search("Kingfisher", SearchOptions::default())
            .unwrap()
            .is_empty(),
        "the update trigger left the old title in the index"
    );

    store.delete_observation(id, true).unwrap();
    assert!(
        store
            .search("Bittern", SearchOptions::default())
            .unwrap()
            .is_empty(),
        "the delete trigger left the row in the index"
    );
}

#[test]
fn prompts_can_be_listed_and_deleted_with_tombstones() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let first = store
        .add_prompt(AddPrompt {
            session_id: "s1".to_owned(),
            content: "How should SQLite transactions work?".to_owned(),
            project: Some("Leteo".to_owned()),
        })
        .unwrap();
    store
        .add_prompt(AddPrompt {
            session_id: "s1".to_owned(),
            content: "A second unrelated request".to_owned(),
            project: Some("other".to_owned()),
        })
        .unwrap();

    // Asked through the door that has a reader. `list_prompts` and
    // `search_prompts` answered the same questions and nothing reached them —
    // no command, no tool, no hook — so they are gone and this asks
    // `recent_prompts`, which the opening context uses.
    assert_eq!(
        store.recent_prompts(Some("LETEO"), Some(10)).unwrap(),
        vec![first.clone()]
    );

    store.delete_prompt(first.id).unwrap();
    assert!(
        store
            .recent_prompts(Some("leteo"), None)
            .unwrap()
            .is_empty()
    );
    let tombstone: String = store
        .connection
        .query_row(
            "SELECT sync_id FROM prompt_deletions WHERE sync_id = ?1",
            [&first.sync_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tombstone, first.sync_id);
    let operation: String = store
        .connection
        .query_row(
            "SELECT op FROM sync_mutations WHERE entity = 'prompt' AND entity_key = ?1
                 ORDER BY seq DESC LIMIT 1",
            [&first.sync_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(operation, crate::sync::OP_DELETE);
    assert!(store.delete_prompt(first.id).is_err());
}

#[test]
fn hostile_search_queries_never_reach_the_full_text_parser() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    store
        .add_observation(observation("s1", "Injection probe", "needle body"))
        .unwrap();

    // Every one of these is FTS5 syntax that would change the query's
    // meaning, or break it outright, if it were not quoted as a phrase.
    for hostile in [
        "\"",
        "\"\"\"",
        "needle OR 1",
        "needle AND NOT body",
        "NEAR(needle body, 2)",
        "*",
        "^needle",
        "needle*",
        "(needle",
        "{title}: needle",
        "needle\" OR title:\"x",
        "-needle",
        ":",
        "\u{0}needle",
    ] {
        let result = store.search(hostile, SearchOptions::default());
        assert!(
            result.is_ok(),
            "search({hostile:?}) must not fail: {:?}",
            result.err()
        );
    }

    // Whitespace-only input is refused before it can reach SQLite.
    assert!(matches!(
        store.search("   \t\n ", SearchOptions::default()),
        Err(StoreError::EmptySearch)
    ));

    // Storing hostile text must not poison later reads either: the title
    // feeds candidate detection, which builds its own full-text query.
    let saved = store
        .add_observation(observation(
            "s1",
            "Title with \u{0} a NUL and \"quotes\"",
            "content with \u{0} a NUL",
        ))
        .unwrap()
        .observation;
    store
        .find_candidates(saved.id, CandidateOptions::default())
        .expect("candidate detection survives hostile titles");
    store
        .scan_project(ScanOptions {
            project: "leteo".to_owned(),
            ..ScanOptions::default()
        })
        .expect("scanning survives hostile titles");
}

#[test]
fn doctor_reports_sqlite_fts_foreign_keys_and_journal_health() {
    let opening = std::time::Instant::now();
    let (_temp, mut store) = store();
    let opened_in = opening.elapsed();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    store
        .add_observation(observation("s1", "Doctor", "doctor body"))
        .unwrap();
    store
        .add_prompt(AddPrompt {
            session_id: "s1".to_owned(),
            content: "doctor prompt".to_owned(),
            project: Some("leteo".to_owned()),
        })
        .unwrap();

    let report = store.doctor().unwrap();
    assert!(report.healthy, "{:?}", report.issues);
    assert_eq!(report.integrity_check, ["ok"]);
    assert!(report.foreign_key_violations.is_empty());
    assert!(report.observation_fts_ok);
    assert!(report.prompt_fts_ok);
    assert_eq!(report.observations, report.observation_fts_rows);
    assert_eq!(report.prompts, report.prompt_fts_rows);
    assert!(report.pending_mutations >= 3);
    assert_eq!(report.journal_mode.to_lowercase(), "wal");
    // What this store was opened with, less whatever the open spent: the wait
    // is one budget for the whole open now, and a hook opens with a smaller one
    // than the default five seconds.
    //
    // Measured against how long the open actually took rather than against a
    // guess at it. The guess was a flat second, which is a claim about the
    // machine: under a loaded one — the suite running beside a release build —
    // the schema pass took 1.6 s of the five and this failed at 3,361 ms, which
    // is the design working exactly as written.
    //
    // With a few milliseconds of slack, and they are rounding rather than
    // generosity. Both sides are truncated to whole milliseconds — the deadline
    // inside `open` and the `Instant` around it — on two clocks started at
    // different moments, so the sum can come up a millisecond or two short of
    // the budget while nothing at all is wrong. It did: 4,982 left of 5,000
    // after an open that took 17. What this catches is a budget spent twice,
    // and that was 1,639 ms of five seconds.
    const ROUNDING: i64 = 50;
    assert!(report.busy_timeout_ms > 0);
    assert!(
        report.busy_timeout_ms + opened_in.as_millis() as i64 + ROUNDING
            >= store.config.busy_timeout.as_millis() as i64,
        "{} ms left of {} ms after an open that took {} ms",
        report.busy_timeout_ms,
        store.config.busy_timeout.as_millis(),
        opened_in.as_millis()
    );
    assert!(report.busy_timeout_ms <= store.config.busy_timeout.as_millis() as i64);
}

#[test]
fn a_question_with_one_unknown_word_still_finds_what_it_was_about() {
    // Requiring every word fails completely rather than partially: one word
    // the store has never seen takes the whole question down with it, and the
    // caller gets the same empty list as a subject nobody ever wrote about.
    // Measured over two hundred questions drawn from the titles of a real
    // 2,643-memory store, that happened to 12% of the long ones.
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    store
        .add_observation(observation(
            "s1",
            "The CROSS JOIN that made search fast",
            "SQLite planned the join the other way round",
        ))
        .unwrap();

    // Every word is there, so nothing is widened and nothing is marked.
    let exact = store
        .search("CROSS JOIN search", SearchOptions::default())
        .unwrap();
    assert_eq!(exact.len(), 1, "{exact:?}");
    assert!(
        !exact[0].partial,
        "an exact match must not be labelled a partial one"
    );

    // One word nobody ever wrote, and the memory is still found — labelled.
    let widened = store
        .search("CROSS JOIN search kubernetes", SearchOptions::default())
        .unwrap();
    assert_eq!(
        widened.len(),
        1,
        "an unknown word must not take the question down with it: {widened:?}"
    );
    assert!(
        widened[0].partial,
        "a memory that matched some of the words says so"
    );
    assert_eq!(widened[0].observation.id, exact[0].observation.id);

    // A question about nothing in the store is still answered with nothing.
    // The retry widens; it does not invent.
    assert!(
        store
            .search("kubernetes helm istio", SearchOptions::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_question_that_matched_exactly_is_never_diluted_by_one_that_half_matched() {
    // The widened retry runs only when the strict pass came back empty. If it
    // ran alongside, every exact answer would be padded out to the limit with
    // memories that share one common word, and the ranking that makes the top
    // hit the right one would be spent on them.
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    store
        .add_observation(observation(
            "s1",
            "Postgres connection pooling",
            "pgbouncer in transaction mode",
        ))
        .unwrap();
    for n in 0..5 {
        store
            .add_observation(observation(
                "s1",
                &format!("Postgres note {n}"),
                "unrelated body",
            ))
            .unwrap();
    }

    let results = store
        .search("Postgres pooling", SearchOptions::default())
        .unwrap();
    assert_eq!(
        results.len(),
        1,
        "only the memory with both words: {:?}",
        results
            .iter()
            .map(|r| &r.observation.title)
            .collect::<Vec<_>>()
    );
    assert!(!results[0].partial);
}

#[test]
fn a_topic_key_is_found_however_it_was_typed() {
    // Keys are normalised on the way in, and the lookup used to compare the
    // raw query against the normalised column. So the exact branch fired only
    // for a caller who had already spelled the key the way the store holds it,
    // and every other spelling fell through to ranked full-text against the
    // whole family — 125 memories under `architecture/` on a real store.
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let mut filed = observation("s1", "Wizard split", "the body");
    filed.topic_key = Some("Architecture/Wizard Split".to_owned());
    let saved = store.add_observation(filed).unwrap().observation;
    assert_eq!(
        saved.topic_key.as_deref(),
        Some("architecture/wizard-split")
    );

    // Something else under the same family, so a fuzzy answer is visibly
    // different from an exact one rather than accidentally identical.
    let mut sibling = observation("s1", "Wizard split of the other thing", "body");
    sibling.topic_key = Some("architecture/other".to_owned());
    store.add_observation(sibling).unwrap();

    for typed in [
        "architecture/wizard-split",
        "Architecture/Wizard-Split",
        "ARCHITECTURE/WIZARD-SPLIT",
        "  architecture/wizard split  ",
    ] {
        let results = store.search(typed, SearchOptions::default()).unwrap();
        assert_eq!(
            results[0].observation.id, saved.id,
            "{typed:?} must reach the memory filed under that key: {results:?}"
        );
        assert_eq!(
            results[0].rank, -1000.0,
            "{typed:?} must be answered by the exact lookup, not by ranking"
        );
    }

    // A query with no key in it is not a key lookup, whatever else it holds.
    let plain = store
        .search("wizard split", SearchOptions::default())
        .unwrap();
    assert!(
        plain.iter().all(|result| result.rank > -1000.0),
        "an ordinary question must not be treated as a topic key: {plain:?}"
    );
}

#[test]
fn the_full_text_side_drives_the_join_that_search_depends_on() {
    // `CROSS JOIN` is not decoration. With a plain `JOIN`, SQLite 3.51.3 picks
    // `observations` as the outer loop — driven by the index on `deleted_at`,
    // which it reads as selective when it matches every live row — and re-runs
    // the full-text query once per row. On a store of 3,400 memories a
    // ten-word question took 4,075 ms against 14.9 ms, for the same ten ids in
    // the same order.
    //
    // That is why nothing caught it: the answer is identical, only the time
    // changes, so every assertion about results goes on passing. A mutation
    // run downgraded the keyword and the whole suite stayed green.
    //
    // Asserted on the plan rather than on a stopwatch. `SCAN`ning the base
    // table first is the failure; the full-text side has to be the outer loop
    // and `observations` has to be reached by rowid.
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    store
        .add_observation(observation("s1", "Postgres pooling", "pgbouncer"))
        .unwrap();

    // The production statement, not a copy of it. A test that writes its own
    // SQL proves SQLite plans that copy well and nothing about what runs.
    let plan: Vec<String> = store
        .connection
        .prepare(&format!(
            "EXPLAIN QUERY PLAN {}",
            crate::store::search::matching_observations_sql(
                crate::store::search::FTS_STEMMED,
                crate::store::BM25_WEIGHTS,
            )
        ))
        .unwrap()
        .query_map(
            rusqlite::params![
                "\"postgres\"",
                None::<String>,
                None::<String>,
                None::<String>,
                10_i64
            ],
            |row| row.get::<_, String>(3),
        )
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    // SQLite names the aliases, so the plan for a correct query reads:
    //   SCAN fts VIRTUAL TABLE INDEX 0:M6
    //   SEARCH o USING INTEGER PRIMARY KEY (rowid=?)
    let joined = plan.join(" | ");
    let step_of = |needle: &str| plan.iter().position(|step| step.starts_with(needle));
    let fts_driving = step_of("SCAN fts");
    let base_by_rowid = plan
        .iter()
        .position(|step| step.starts_with("SEARCH o ") && step.contains("PRIMARY KEY"));

    assert!(
        fts_driving.is_some(),
        "the full-text side has to be the outer loop, scanned once: {joined}"
    );
    assert!(
        base_by_rowid.is_some(),
        "the base table has to be reached by rowid, not scanned: {joined}"
    );
    assert!(
        fts_driving < base_by_rowid,
        "reversing these is the 273x regression this exists for: {joined}"
    );
}

/// The bm25 field weights, pinned to the measurements that chose them.
///
/// Retuning these looks like free precision and is a trap, because the obvious
/// way to measure it is rigged. Questions drawn from memory *titles* reward
/// weighting titles, and a sweep on 250 of those "improved" MRR from 0.9119 to
/// 0.9448 with `(title 20, content 0.3, topic 6)`. On questions drawn from
/// memory *bodies* the same weights were **worse**, and the gain evaporated on
/// a title set built with a different seed: +0.0019.
///
/// Tuning the other way is no better. Optimised against body questions,
/// `(title 8, content 8, topic 0)` gained +0.0174 on the set it was fitted to,
/// **+0.0016 on a held-out body set** — noise — and lost **0.0315** on titles.
///
/// That is why the content weight moved to 0.5, and why it took a third shape
/// of question to justify it. Both shapes above are quotations: somebody
/// pasting words that are already in the store. The shape neither of them
/// covers is somebody *asking* — which is every prompt, and the only shape the
/// per-prompt hint ever sees. Measured on 253 real prompts with a leak-free
/// label, split in half by prompt id so the second half is held out:
///
/// ```text
///                          speaks   right
///   fitting half   1.0      85.1%   25.6%
///                  0.5      89.3%   33.9%
///   held out       1.0      88.6%   22.7%
///                  0.5      93.2%   34.1%
/// ```
///
/// It holds where the earlier candidates evaporated, and it is larger on the
/// half it was not chosen on. What it costs on the two quotation shapes, on
/// held-out halves under two sampling seeds: bodies +0.0035 and +0.0006,
/// titles +0.0011 and **-0.0072**. So there is a trade and it is a small one —
/// at MRR 0.98 that worst case is about one title question in 140 dropping
/// from first to second — against eleven points on the shape that answers
/// every prompt somebody types.
///
/// Change them again if a measurement says so, and hold it to this bar: a
/// held-out set, of every shape, including the one that is a question rather
/// than a quotation.
///
/// Conflict detection scores with its own weights and deliberately: see
/// `Store::find_candidates`, which zeroes only the project column and is
/// searching for something else.
#[test]
fn the_ranking_weights_are_the_ones_that_were_measured() {
    assert_eq!(
        crate::store::BM25_WEIGHTS,
        "5.0, 0.5, 0.0, 0.0, 0.0, 3.0",
        "the weights above are the ones the tables in this comment measured"
    );
    // And nothing ranks with its own copy of them. Three call sites wrote the
    // vector out by hand, which is how a search and the hint that answers the
    // same question drift apart in one edit.
    //
    // One statement takes them as a parameter, and that is the exception this
    // names rather than a hole in the rule: the retrieval measurement under
    // `tools/` exists to compare weight vectors, and the only way it can do
    // that against the query the product issues is to pass its own. So the
    // interpolation is allowed where the builder writes it, and every caller
    // inside the product has to hand it `BM25_WEIGHTS` — which is what the
    // second loop below checks.
    for (file, source) in [
        ("src/store/search.rs", include_str!("../search.rs")),
        (
            "src/store/observations.rs",
            include_str!("../observations.rs"),
        ),
    ] {
        for (number, line) in source.lines().enumerate() {
            if line.contains("bm25(observations_fts") || line.contains("bm25({index}") {
                assert!(
                    line.contains("{BM25_WEIGHTS}") || line.contains("{weights}"),
                    "{file}:{} ranks with weights of its own: {}",
                    number + 1,
                    line.trim()
                );
            }
            if let Some(at) = line.find("matching_observations_sql(")
                && !line[..at].contains("fn ")
            {
                let call = &line[at..];
                assert!(
                    call.contains("BM25_WEIGHTS") || call.contains(","),
                    "{file}:{} builds the ranking without saying which weights: {}",
                    number + 1,
                    line.trim()
                );
            }
        }
    }
}

#[test]
fn a_question_in_a_language_the_store_does_not_hold_is_answered_with_nothing() {
    // Widening rescues a question one unknown word took down. Left unchecked
    // it also answers a question nobody asked: a question in another language
    // matches on *function words* — `la`, `de`, `no` — against whichever
    // memories happen to share them. Measured on 22 Spanish questions against
    // a real English store, the widened pass returned ten rows for every one
    // and 21 were entirely wrong: asking about passive capture came back with
    // a session summary for an unrelated project.
    //
    // The fixture needs memories the weak query *can* reach, or this passes
    // because nothing matched at all rather than because anything was
    // refused. So a handful are in Spanish, sharing the question's function
    // words and nothing else.
    //
    // What refuses them is that the retry drops **one** term rather than
    // relaxing to any: every variant still demands eight of the nine words,
    // and these memories have none of them.
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    for title in [
        "The retry budget is per host and not per request",
        "The cache is invalidated on write and not on read",
        "The parser is generated and not written by hand",
    ] {
        store
            .add_observation(observation("s1", title, "a body about the same thing"))
            .unwrap();
    }
    // Enough of them that an any-word retry would have plenty to return: on
    // the real store that form gave ten rows for all 22 Spanish questions.
    for n in 0..40 {
        store
            .add_observation(observation(
                "s1",
                &format!("La factura de enero no se genera sin el impuesto {n}"),
                "un cuerpo sobre otra cosa",
            ))
            .unwrap();
    }

    // Every content word is absent; only `de`, `la`, `no` and `se` are shared,
    // which is exactly what the widened pass would answer with.
    let question = "por que no se guarda nada de la memoria pasiva";
    let widened_would_match = store
        .search(
            question,
            SearchOptions {
                mode: SearchMode::Any,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        widened_would_match.len() >= MIN_RECALL_SAMPLE,
        "the fixture has to give the widened pass something to wrongly return, or the floor is \
         never exercised: {widened_would_match:?}"
    );

    let answered = store.search(question, SearchOptions::default()).unwrap();
    assert!(
        answered.is_empty(),
        "a question this store cannot answer has to come back empty: {:?}",
        answered
            .iter()
            .map(|result| &result.observation.title)
            .collect::<Vec<_>>()
    );

    // And the rescue the widening exists for still works.
    let rescued = store
        .search(
            "retry budget per host and not per request kubernetes",
            SearchOptions::default(),
        )
        .unwrap();
    assert!(
        rescued
            .iter()
            .any(|result| result.observation.title.starts_with("The retry budget")),
        "one unknown word must not take the question down: {rescued:?}"
    );
    assert!(rescued.iter().all(|result| result.partial));
}

/// The tokenizer is load-bearing in Spanish too, and now measured there.
///
/// `0003` chose `porter unicode61` on English evidence and offered one Spanish
/// example in passing. Writing memories in the language of the conversation
/// makes that aside structural, so it is checked here rather than assumed.
///
/// Measured against 232 real Spanish messages out of this store: Porter folds
/// 2,117 distinct terms to 1,868. Reading all 314 merged groups by hand, the
/// Spanish ones are right — `aparece`/`aparecer`, `mantener`/`mantenible`,
/// `puede`/`puedes`, `mensaje`/`mensajes` — and it folds across languages as
/// well, putting `agente` and `agentes` on `agent`. No Spanish word was merged
/// with an unrelated one.
///
/// What it does **not** reach is participles (`guardar`/`guardado`), gender
/// (`escrito`/`escrita`) and derivations (`buscar`/`búsqueda`). Those stay
/// apart, which costs recall rather than precision, and no stemmer bundled with
/// FTS5 fixes it — the alternative to Porter here is nothing at all.
///
/// Diacritics fold before any of this: `búsqueda` and `busqueda` are one term,
/// as are `año` and `ano`, so `remove_diacritics 2` would change nothing.
#[test]
fn spanish_plurals_and_verb_forms_reach_the_same_term() {
    // The measurement below is about one tokenizer, so it is worth nothing if
    // the store has quietly moved to another. The baseline is where both
    // indexes get theirs — it was migration `0003` until that was folded in for
    // the first release — and every `fts5` table it creates has to name this
    // one.
    const TOKENIZER: &str = "tokenize = 'porter unicode61'";
    const BASELINE: &str = include_str!("../../../migrations/0001_baseline_tables.sql");
    assert_eq!(
        BASELINE.matches("USING fts5(").count(),
        BASELINE.matches(TOKENIZER).count(),
        "an index without {TOKENIZER} is not the one this test measured"
    );

    let temp = tempfile::tempdir().unwrap();
    let connection = rusqlite::Connection::open(temp.path().join("tokenize.db")).unwrap();
    connection
        .execute_batch(
            "CREATE VIRTUAL TABLE probe USING fts5(body, tokenize = 'porter unicode61');",
        )
        .unwrap();

    let mut insert = connection
        .prepare("INSERT INTO probe(body) VALUES (?1)")
        .unwrap();
    // One word per row, so a match means the two words share a term rather
    // than that they happened to sit in the same sentence.
    let together = [
        // The plural the migration named, and the `-ciones` family it did not:
        // that is most of the vocabulary an agent writes about its own work.
        ("sesion", "sesiones"),
        ("configuracion", "configuraciones"),
        ("version", "versiones"),
        ("memoria", "memorias"),
        // Verb forms, which the migration did not claim at all.
        ("puede", "puedes"),
        ("aparece", "aparecer"),
        // And the accent, which somebody asking a question often leaves off.
        ("busqueda", "búsqueda"),
    ];
    for (first, second) in together {
        insert.execute([first]).unwrap();
        let found: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM probe WHERE probe MATCH ?1",
                [format!("\"{second}\"")],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            found, 1,
            "{first:?} and {second:?} have to be one term, not two"
        );
        connection.execute("DELETE FROM probe", []).unwrap();
    }

    // And the honest other half: these are two terms, and a question using one
    // does not reach a memory written with the other.
    for (first, second) in [("guardar", "guardado"), ("buscar", "busqueda")] {
        insert.execute([first]).unwrap();
        let found: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM probe WHERE probe MATCH ?1",
                [format!("\"{second}\"")],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            found, 0,
            "{first:?} and {second:?} do not fold; if they now do, the \
             tokenizer changed and the measurement above is stale"
        );
        connection.execute("DELETE FROM probe", []).unwrap();
    }
}

/// The hint in front of a prompt reads the whole question, not its opening.
///
/// Words are taken in the order they were typed and cut at
/// `MAX_ANY_TERMS`, which was a dozen — about where an ordinary question
/// stops introducing itself and starts saying what it is about. Measured over
/// 223 real prompts against the memory saved after each: nine gained the right
/// memory when the cut moved out to thirty-two, and none lost it.
///
/// Nothing tested this function at all, which is how a bound that halved what
/// it read went unnoticed while the note above it described a pipeline that no
/// longer existed.
#[test]
fn a_prompt_is_read_past_its_opening_words() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    // Enough memories to be a sample rather than a coincidence: the recall
    // refuses to claim anything from fewer than three rows, and it keeps only
    // what beats the median of what it found.
    for index in 0..8 {
        store
            .add_observation(observation(
                "s1",
                &format!("Nota general sobre el arranque número {index}"),
                "Un cuerpo cualquiera que habla del arranque y de la sesión.",
            ))
            .unwrap();
    }
    store
        .add_observation(observation(
            "s1",
            "El planificador de consultas y sus estadísticas",
            "Por qué un índice sin estadísticas no se elige nunca.",
        ))
        .unwrap();

    // Twelve words of preamble, and then the word that says what is being
    // asked. Every one of the first twelve is in the other memories, so a
    // reader that stops there has plenty to answer with and answers wrong.
    let prompt = "hola buenas mira sobre esto que hablamos antes del arranque \
                  general nota cuerpo estadisticas";
    let matches = store.prompt_matches(prompt, "leteo", 3).unwrap();
    let titles: Vec<&str> = matches
        .iter()
        .map(|found| found.title.as_str())
        .collect::<Vec<_>>();
    assert!(
        titles
            .iter()
            .any(|title| title.contains("planificador de consultas")),
        "the word the question turns on comes after the preamble: {titles:?}"
    );
}

/// Search reads two indexes, and each rescues what the other loses.
///
/// The stemmed index is what lets a question asked with a different ending
/// find anything: `porter` folds `configuracion`/`configuraciones`. What it
/// costs is that more memories match the same words, so the one somebody
/// quoted is diluted — measured on a real store, six words quoted out of a
/// memory find it first 78% of the time stemmed and 84% unstemmed, while a
/// question with two of six words re-inflected is answered 63% of the time
/// stemmed and **0%** unstemmed.
///
/// Both halves are asserted, and each fails with one index alone. The first
/// fails on the stemmed index: twelve plurals crowd out the singular that was
/// quoted word for word. The second fails on the unstemmed one, and not
/// through the widened retry either — one term is below the two the widening
/// needs, so an unstemmed store answers nothing at all.
#[test]
fn both_the_written_word_and_the_inflected_one_find_the_memory() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    // Plurals of every word of the question, in titles shorter than the one
    // that answers it — which is what makes bm25 prefer them once a stemmer
    // has made all of them match.
    for index in 0..12 {
        store
            .add_observation(observation(
                "s1",
                &format!("Memorias perdidas solas {index}"),
                "Cuerpo corto.",
            ))
            .unwrap();
    }
    store
        .add_observation(observation(
            "s1",
            "La memoria perdida sola de una configuracion que nadie encontro",
            "Un cuerpo bastante mas largo que el de los rivales, con relleno suficiente para que \
             su longitud pese en el ranking.",
        ))
        .unwrap();

    let found = |query: &str| {
        store
            .search(query, SearchOptions::default())
            .unwrap()
            .first()
            .map(|result| (result.observation.title.clone(), result.partial))
    };

    // Quoted as written. The stemmed index puts five plurals above it.
    assert_eq!(
        found("memoria perdida sola")
            .map(|(title, _)| title)
            .as_deref(),
        Some("La memoria perdida sola de una configuracion que nadie encontro"),
        "a question quoting the memory word for word has to find it first"
    );
    // And asked with another ending, which only the stemmer can answer. Not
    // partial: this is the strict pass, not the widened rescue.
    assert_eq!(
        found("configuraciones"),
        Some((
            "La memoria perdida sola de una configuracion que nadie encontro".to_owned(),
            false
        )),
        "an inflected question has to find it too; this is what the stemmer is for"
    );
}

/// The hint is stricter about repeating what the session already listed.
///
/// A session opens with an index of the project's most recent memories.
/// Naming one of those again ranks what the agent already has; naming one from
/// further back is the only way it hears of that memory at all. The cost of a
/// wrong line is the same either way and the value of a right one is not, so
/// the bar is not the same on both sides.
///
/// Asserted on numbers rather than on a corpus, and that is deliberate: bm25
/// needs a varied one for a median to mean anything, and fifty near-identical
/// fixtures score every term at nothing. The first version of this test built
/// exactly that and passed with both margins set equal — it proved the query
/// runs, not that the rule exists. What the rule is worth end to end was
/// measured over 277 real prompts against a leak-free label, and is in
/// `prompt_matches`.
#[test]
fn the_bar_is_lower_for_a_memory_the_session_did_not_open_with() {
    use crate::store::search::worth_naming;

    // Scores are negative and better is more negative, so a smaller margin is
    // a looser bar: with a median of -10, the strict side asks for -16 and the
    // other for -12.
    let median = -10.0;

    // Squarely good: named wherever it came from.
    assert!(worth_naming(-20.0, median, true));
    assert!(worth_naming(-20.0, median, false));

    // The whole point: between the two bars.
    assert!(
        !worth_naming(-13.0, median, true),
        "a memory the agent already has listed has to earn the line"
    );
    assert!(
        worth_naming(-13.0, median, false),
        "one it has never seen is worth saying on weaker evidence"
    );

    // And weak is weak on both sides.
    assert!(!worth_naming(-5.0, median, true));
    assert!(!worth_naming(-5.0, median, false));
}

/// Narrowing inside the index must not narrow away the project's own memories.
///
/// The project is now part of the `MATCH`, as a phrase over a tokenised
/// column. That is only a way of not scoring other projects' memories, and it
/// can go wrong in a way the `WHERE` never could: a name the tokenizer splits
/// — `nas.archive`, `example-school.com`, `task-board` — has to keep
/// finding its own memories, and a name with no letters or digits in it at all
/// must not turn the query into a syntax error and silence the search.
#[test]
fn a_project_whose_name_the_tokenizer_splits_still_finds_its_memories() {
    let (_temp, mut store) = store();
    for project in ["nas.archive", "task-board", "---"] {
        store.enroll_project(project).unwrap();
        store
            .create_session(project, project, &format!("C:/{project}"))
            .unwrap();
        let mut memory = observation(project, "The connection pool leaked", "under load");
        memory.project = Some(project.to_owned());
        store.add_observation(memory).unwrap();
    }

    for project in ["nas.archive", "task-board", "---"] {
        let found = store
            .search(
                "connection pool",
                SearchOptions {
                    project: Some(project.to_owned()),
                    ..SearchOptions::default()
                },
            )
            .unwrap();
        assert_eq!(
            found.len(),
            1,
            "the project has to find its own memory: {project}"
        );
        // Normalised on the way in: `---` is stored as `-`, which is exactly
        // the name with nothing for the tokenizer to index.
        assert_eq!(
            found[0].observation.project,
            Some(crate::memory::normalize::project(project))
        );
    }

    // And the narrowing still means something: the memories are identical, so
    // only the project tells them apart.
    let found = store
        .search(
            "connection pool",
            SearchOptions {
                project: Some("task-board".to_owned()),
                ..SearchOptions::default()
            },
        )
        .unwrap();
    assert_eq!(found.len(), 1, "and no other project's copy comes with it");
}

/// Both indexes have to accept the narrowed query, not just the stemmed one.
///
/// `fused_observations` reads two tables and merges them by rank, and it is
/// deliberately forgiving: an unreadable unstemmed index logs a line and
/// searches the stemmed one alone. That is right for a store mid-migration and
/// wrong as a way to find out that a query the code builds is invalid — the
/// search would go on working, quietly back down to one tokenizer, and the
/// only difference would be worse answers.
///
/// So this asserts the narrowing parses against both, which is the thing that
/// could differ between them. What the fusion is worth is measured, not
/// asserted: the table is in `Store::fused_observations`.
#[test]
fn the_project_narrowing_is_valid_against_both_indexes() {
    let (_temp, store) = store();
    let narrowed =
        crate::memory::normalize::fts_within_project("\"pool\" OR \"leak\"", "leteo").unwrap();
    for index in [
        crate::store::search::FTS_STEMMED,
        crate::store::search::FTS_EXACT,
    ] {
        let count: Result<i64, _> = store.connection.query_row(
            &format!("SELECT count(*) FROM {index} WHERE {index} MATCH ?1"),
            [&narrowed],
            |row| row.get(0),
        );
        assert!(
            count.is_ok(),
            "{index} refused the narrowed query: {count:?}"
        );
    }
}

/// How many are named and what an ordinary match looks like are two questions.
///
/// The floor is the median of the candidates scored, so how many are scored
/// decides where it lands — and the sample used to be `(limit * 8).max(24)`,
/// which tied it to what the caller asked for. Measured over 277 real prompts
/// with a leak-free label, on the held-out half, a sample of 24 is right 31.5%
/// of the time and one of 40 is right 24.5%. So raising the limit from three
/// to five would have deepened the sample underneath and cost seven points —
/// while looking exactly like the price of naming more memories, which is the
/// opposite of true: naming five out of a sample of 24 measures *better* than
/// naming three, 39.2% against 36.0%.
///
/// Asserted on the source rather than on behaviour, and that is not laziness.
/// A deeper sample can only *loosen* the floor — bm25 is negative and the tail
/// of a candidate list sits near zero, so more candidates pull the median
/// toward zero — and rank order does not change with depth. So the top three
/// clear both bars and come back identical either way: the damage is to what
/// else is let in, and to what the bar would have been on a different corpus.
/// Two fixtures were written trying to catch it before this was understood,
/// and both passed with the bug in place.
#[test]
fn the_sample_the_floor_is_taken_from_does_not_depend_on_the_limit() {
    let source = include_str!("../search.rs");
    let line = source
        .lines()
        .find(|line| line.contains("let sample"))
        .expect("prompt_matches samples candidates before it judges them");
    assert!(
        line.contains("RECALL_SAMPLE"),
        "the sample has to be the constant, not a function of the caller's limit: {}",
        line.trim()
    );
    assert!(
        !line.contains("limit"),
        "the sample must not mention the limit at all: {}",
        line.trim()
    );
}

/// A question gets an answer, marked for what it is.
///
/// The strict pass wants every word and the widened one wants all but one:
/// both are built for a quotation with a word wrong in it. A question is not
/// that, and on 277 real prompts the two of them together came back empty for
/// 80.5% — while the per-prompt hint, given the same words, named something
/// useful a third of the time. The tool an agent calls on purpose was the one
/// that said nothing.
#[test]
fn a_question_that_matches_no_memory_word_for_word_is_still_answered() {
    let (_temp, mut store) = store();
    store.create_session("s1", "Leteo", "C:/repo").unwrap();
    for (title, body) in [
        (
            "The connection pool leaked",
            "it was never returned on the error path and the process ran out",
        ),
        (
            "The retry budget outlived its timeout",
            "three attempts against a two second deadline, so the last never ran",
        ),
        (
            "SQLite planned the join badly",
            "no statistics existed, so the index added for it was never chosen",
        ),
        (
            "A deleted memory called itself active",
            "the state field looked at the wrong column first",
        ),
    ] {
        store
            .add_observation(observation("s1", title, body))
            .unwrap();
    }

    // Fourteen words, none of them a quotation: this is what somebody asks.
    // Long enough that the all-but-one retry does not run either.
    let asked = "why does the connection get lost when the deadline passes and nobody \
                 retries anything";
    let found = store
        .search(
            asked,
            SearchOptions {
                project: Some("leteo".to_owned()),
                ..SearchOptions::default()
            },
        )
        .unwrap();

    assert!(
        !found.is_empty(),
        "a question about something the store holds must not come back empty"
    );
    assert!(
        found.iter().all(|result| result.partial),
        "and it has to say these matched some of the words, not all of them"
    );
    // Still a floor, not everything that shares a word.
    assert!(
        found.len() <= 3,
        "the closest few, not the whole project: {}",
        found.len()
    );
}

/// A loosened question does not come back headed by a session summary.
///
/// The strict pass keeps them: a question whose words name what a session did
/// should find that session. The relaxed stages do not, which is the rule
/// `nearest_observations` and `prompt_matches` already keep — they are the most
/// common memories on a busy project, they all read alike, and they were most
/// of what the relevance test was there to reject.
///
/// Measured over 80 real questions asked in their own projects: six were
/// answered by the strict pass and none of those led with a summary, while 74
/// fell through to a relaxed stage and 54 of those — 73% — came back headed by
/// one. Removing them cost no question its answer: 80 of 80 still found
/// something.
#[test]
fn a_loosened_question_is_not_answered_by_a_session_summary() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();

    let mut summary = observation(
        "s1",
        "Session summary: leteo",
        "the session went over the connection pool, the retry ladder, the index the \
         planner would not choose, and a great many other things besides",
    );
    summary.kind = crate::memory::model::SESSION_SUMMARY.to_owned();
    let summary = store.add_observation(summary).unwrap().observation;
    let specific = store
        .add_observation(observation(
            "s1",
            "The retry ladder was capped at three",
            "the connection pool gave up too early",
        ))
        .unwrap()
        .observation;

    // Loosened: `kubernetes` is a word this store has never seen, so the strict
    // pass finds nothing and the widened one answers.
    let widened = store
        .search("retry ladder kubernetes", SearchOptions::default())
        .unwrap();
    assert!(widened.iter().all(|hit| hit.partial), "{widened:?}");
    assert!(
        widened.iter().all(|hit| hit.observation.id != summary.id),
        "a loosened question must not be answered by a summary: {:?}",
        widened
            .iter()
            .map(|hit| &hit.observation.title)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        widened.first().map(|hit| hit.observation.id),
        Some(specific.id),
        "the memory that answers it is still there: {widened:?}"
    );

    // And the strict pass still finds a summary by its own words.
    let exact = store
        .search("Session summary leteo", SearchOptions::default())
        .unwrap();
    assert_eq!(
        exact.first().map(|hit| hit.observation.id),
        Some(summary.id),
        "a question that names the session must still find it: {exact:?}"
    );
    assert!(!exact[0].partial);
}

#[test]
fn a_full_page_says_whether_more_is_behind_it_at_every_limit_including_the_cap() {
    // Asked at the cap, the answer used to be silence.
    //
    // `search_with_more` asks for one row past the limit and reports `more` if
    // it comes back, but it asked through `search`, which clamps to
    // `max_search_results` itself. So the probe row was thrown away at exactly
    // the limit where it decides the answer: a search for twenty on a store
    // holding twenty-five matches came back with a full page, `more` false, and
    // neither surface said a word. Below the cap the caller's own limit ended
    // the list and the reply said so; above it the reply said the cap had; at
    // the cap, which is where a caller who wants everything lands, nothing.
    //
    // Driven by the property rather than by the three cases, so a fourth
    // arrives already covered: at every limit, `more` is true exactly when a
    // memory matched that was not handed back.
    let (_temp, mut store) = store();
    let cap = store.config.max_search_results;
    store.create_session("s1", "Leteo", "C:/repo").unwrap();
    let total = cap + 5;
    for n in 0..total {
        store
            .add_observation(observation("s1", &format!("Widget {n}"), "widget"))
            .unwrap();
    }

    for asked in [1, 3, cap - 1, cap, cap + 1, cap * 3] {
        let (found, more) = store
            .search_with_more(
                "widget",
                SearchOptions {
                    limit: Some(asked),
                    ..SearchOptions::default()
                },
            )
            .unwrap();
        let handed_back = asked.min(cap);
        assert_eq!(
            found.len(),
            handed_back,
            "a search for {asked} of {total} matches should hand back {handed_back}"
        );
        assert!(
            more,
            "{total} matched and {handed_back} came back, so something is behind the page"
        );
    }

    // And the other half of the property, which is what keeps the sentence
    // honest: with nothing behind the page, `more` is false however the page
    // was ended. Asking for fifty and matching exactly twenty was called
    // clamped, and "not everything that matched" was a false statement about a
    // complete answer.
    for asked in [total, cap + 1, cap * 3] {
        let (found, more) = store
            .search_with_more(
                "Widget 0",
                SearchOptions {
                    limit: Some(asked),
                    ..SearchOptions::default()
                },
            )
            .unwrap();
        assert_eq!(found.len(), 1, "one memory carries that exact title");
        assert!(!more, "one matched and one came back, so nothing is behind");
    }
}

/// A blank narrowing is no narrowing, on every read that takes one.
///
/// `project` had this right — an empty one falls back to detection — and the
/// two beside it did not, in two different ways. `scope: ""` went through
/// `normalize::scope`, which folds anything it does not recognise onto
/// `project`, so an empty filter quietly narrowed the answer to project scope.
/// `type: ""` went through `normalize::kind`, which leaves it empty, so the
/// answer was narrowed to a type no memory has — and came back empty with the
/// hint that blames the words, which is the one explanation that was not true.
///
/// The fold is right for a value being stored and wrong for one being asked
/// about, and that is the whole distinction: the same normaliser, two jobs.
#[test]
fn a_blank_narrowing_narrows_nothing() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    for (title, scope) in [
        ("A project memory", "project"),
        ("A personal memory", "personal"),
        ("Another project memory", "project"),
    ] {
        store
            .add_observation(AddObservation {
                session_id: "s1".to_owned(),
                kind: "decision".to_owned(),
                title: title.to_owned(),
                content: "a widget body worth finding".to_owned(),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: scope.to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
    }
    store
        .add_observation(observation("s1", "A bugfix", "a widget body worth finding"))
        .unwrap();

    let search = |kind: Option<&str>, scope: Option<&str>| {
        store
            .search(
                "widget",
                SearchOptions {
                    kind: kind.map(str::to_owned),
                    scope: scope.map(str::to_owned),
                    ..SearchOptions::default()
                },
            )
            .unwrap()
            .len()
    };
    let everything = search(None, None);
    assert!(everything >= 4, "all four are findable: {everything}");

    // Blank in either of the two, in either spelling, answers the whole thing.
    for blank in ["", "   "] {
        assert_eq!(
            search(Some(blank), None),
            everything,
            "an empty type is not a type filter"
        );
        assert_eq!(
            search(None, Some(blank)),
            everything,
            "an empty scope is not a scope filter"
        );
    }

    // And the filters still filter, without which the above passes by doing
    // nothing at all.
    assert!(
        search(Some("decision"), None) < everything,
        "a real type narrows"
    );
    assert!(
        search(None, Some("personal")) < everything,
        "a real scope narrows"
    );

    // The other two reads that take a scope, which shared the fold.
    let recent = |scope: Option<&str>| {
        store
            .recent_memories(Some("leteo"), scope, 50)
            .unwrap()
            .len()
    };
    assert_eq!(recent(Some("")), recent(None), "recent takes it too");
    assert!(recent(Some("personal")) < recent(None), "and still narrows");

    store.pin_observation(2).unwrap();
    let pinned = |scope: Option<&str>| {
        store
            .pinned_observations(Some("leteo"), scope, usize::MAX)
            .unwrap()
            .0
            .len()
    };
    assert_eq!(pinned(Some("   ")), pinned(None), "and so does pinned");
    assert!(pinned(None) > 0, "there is a pinned one to count");
}

/// The answer keeps the order the ranking chose, and every field of every row.
///
/// The stages rank ids now and the bodies are fetched once at the end, because
/// reading three times as many whole rows as are returned moved 9.8 MB of
/// memory bodies through `map_observation` to show 392 memories over 200 real
/// prompts — 91% of it discarded unread. Two things can go quietly wrong when
/// a fetch is split off from the ranking that decided it.
///
/// `WHERE id IN (…)` does not promise the order of the list, and SQLite will
/// happily answer it by rowid: an answer sorted by id instead of by rank looks
/// entirely reasonable and is the wrong memory first. So the fixture is built
/// with the ranking deliberately against the ids — the best match is the
/// highest id — and nothing here would notice if the two agreed.
///
/// And the row that comes back has to be the whole row. What the stages carry
/// is an id, a type and a score; every other field arrives only from the
/// fetch, so a body or a project left behind would show up as an empty string
/// rather than as an error.
#[test]
fn the_fetch_keeps_the_ranking_and_brings_the_whole_row() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    // Cuatro memorias sobre lo mismo, la mejor la última: el título repite la
    // palabra buscada y el título pesa 5.0 en `BM25_WEIGHTS`.
    for (index, (title, content)) in [
        ("Una nota lateral", "menciona zarandaja una vez de pasada"),
        ("Otra nota lateral", "menciona zarandaja una vez"),
        ("Zarandaja de refilón", "el cuerpo habla de otra cosa"),
        (
            "Zarandaja, zarandaja y zarandaja",
            "zarandaja por todas partes",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        store
            .add_observation(AddObservation {
                session_id: "s1".to_owned(),
                kind: "decision".to_owned(),
                title: title.to_owned(),
                content: content.to_owned(),
                tool_name: Some(format!("herramienta-{index}")),
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
    }

    let found = store.search("zarandaja", SearchOptions::default()).unwrap();
    assert_eq!(found.len(), 4, "{found:?}");
    // Lo que decide el orden es el rango, y los ids van al revés.
    assert!(
        found[0].observation.id > found[3].observation.id,
        "el mejor casamiento es el id más alto, así que un orden por id se vería aquí: {:?}",
        found
            .iter()
            .map(|r| (r.observation.id, r.rank))
            .collect::<Vec<_>>()
    );
    for pair in found.windows(2) {
        assert!(
            pair[0].rank <= pair[1].rank,
            "bm25 es mejor cuanto más bajo: {:?}",
            found.iter().map(|r| r.rank).collect::<Vec<_>>()
        );
    }

    // And the whole row, not the three columns it was ordered by.
    let mejor = &found[0].observation;
    assert_eq!(mejor.title, "Zarandaja, zarandaja y zarandaja");
    assert_eq!(mejor.content, "zarandaja por todas partes");
    assert_eq!(mejor.kind, "decision");
    assert_eq!(mejor.project.as_deref(), Some("leteo"));
    assert_eq!(mejor.scope, "project");
    assert_eq!(mejor.tool_name.as_deref(), Some("herramienta-3"));
    assert_eq!(mejor.session_id, "s1");
    assert!(!mejor.sync_id.is_empty());
    assert!(!mejor.created_at.is_empty());
}
