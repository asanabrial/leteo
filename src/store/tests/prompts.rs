//! One prompt asked once, however many callers record it.

use super::*;

#[test]
fn one_prompt_recorded_twice_stays_one_prompt() {
    // Two lifecycle hooks were registered for one event, and every prompt was
    // stored twice for as long as that lasted — twenty-three pairs in a real
    // store. The registration is guarded now, but the store is the one thing
    // both callers share, so a repeat is refused here whatever causes it.
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let ask = |store: &mut Store| {
        store
            .add_prompt(AddPrompt {
                session_id: "s1".to_owned(),
                content: "what did we decide about the storage engine?".to_owned(),
                project: Some("leteo".to_owned()),
            })
            .unwrap()
    };

    let first = ask(&mut store);
    let echo = ask(&mut store);

    assert_eq!(
        echo.id, first.id,
        "the echo is the prompt, not a second one"
    );
    assert_eq!(echo.sync_id, first.sync_id);
    assert_eq!(
        store.recent_prompts(Some("leteo"), Some(10)).unwrap().len(),
        1
    );
}

#[test]
fn an_echo_is_not_journaled_a_second_time_either() {
    // A duplicate row would have replicated to every peer as a separate
    // prompt, so the mutation has to be refused with it.
    let (_temp, mut store) = store();
    store.enroll_project("leteo").unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let ask = |store: &mut Store| {
        store
            .add_prompt(AddPrompt {
                session_id: "s1".to_owned(),
                content: "dale".to_owned(),
                project: Some("leteo".to_owned()),
            })
            .unwrap()
    };
    ask(&mut store);
    let before: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM sync_mutations WHERE entity = 'prompt'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    ask(&mut store);

    let after: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM sync_mutations WHERE entity = 'prompt'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(after, before);
}

#[test]
fn the_same_words_in_another_session_are_another_prompt() {
    // Two agents working the same project ask the same thing, and both asked.
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    store.create_session("s2", "leteo", "C:/repo").unwrap();
    for session in ["s1", "s2"] {
        store
            .add_prompt(AddPrompt {
                session_id: session.to_owned(),
                content: "continue".to_owned(),
                project: Some("leteo".to_owned()),
            })
            .unwrap();
    }

    assert_eq!(
        store.recent_prompts(Some("leteo"), Some(10)).unwrap().len(),
        2
    );
}

#[test]
fn asking_the_same_thing_again_later_is_asking_again() {
    // The window is seconds, not the fifteen minutes observations use: saying
    // "continue" twice in one conversation is two requests, and losing the
    // second would lose what the person actually did.
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    store
        .add_prompt(AddPrompt {
            session_id: "s1".to_owned(),
            content: "continue".to_owned(),
            project: Some("leteo".to_owned()),
        })
        .unwrap();
    // Pushed back out of the echo window without waiting for a real clock.
    store
        .connection
        .execute(
            "UPDATE prompts SET created_at = datetime('now', '-1 minute')",
            [],
        )
        .unwrap();

    store
        .add_prompt(AddPrompt {
            session_id: "s1".to_owned(),
            content: "continue".to_owned(),
            project: Some("leteo".to_owned()),
        })
        .unwrap();

    assert_eq!(
        store.recent_prompts(Some("leteo"), Some(10)).unwrap().len(),
        2
    );
}

#[test]
fn the_prompt_id_lookup_reads_one_column_and_not_the_whole_row() {
    // The one invariant here that no behavioural test can reach. Widening the
    // `SELECT` returns the same identifier, so asserting on the answer passes
    // whatever the query asks for — a mutation that fetched the whole row
    // survived a full sweep for exactly that reason, and was right to.
    //
    // What it costs is `content`: the largest column in the table, a whole
    // prompt, read off disk on a path that runs on every save that links a
    // memory to the question behind it. So the query itself is what gets
    // checked, because it is the only place the rule is written down.
    let select = Store::LATEST_PROMPT_SYNC_ID
        .split_once("FROM")
        .expect("a SELECT with a FROM")
        .0;
    assert!(
        !select.contains('*'),
        "a star selects the body along with everything else: {select}"
    );
    assert!(
        !select.contains("content"),
        "the body is what this must never read: {select}"
    );
    // Commas at the top level only. `ifnull(sync_id, '')` carries one of its
    // own, and counting that as a second column made this fail on the query it
    // is meant to accept — a test that cannot pass is as useless as one that
    // cannot fail.
    let mut depth = 0_i32;
    let separators = select
        .chars()
        .filter(|character| {
            match character {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            *character == ',' && depth == 0
        })
        .count();
    assert_eq!(
        separators, 0,
        "one column, and a top-level comma means a second one: {select}"
    );
    assert!(
        select.contains("sync_id"),
        "and it still has to be the identifier: {select}"
    );
}

#[test]
fn the_recent_prompts_are_the_recent_distinct_ones() {
    // These ten go into the context every session opens with. Taken by time
    // alone, anything asked twice spends two of the ten places — and the shapes
    // that repeat are the ordinary ones: a slash command, "continue", a loop
    // firing on a timer, a question retyped after a failure.
    //
    // Measured on a real store: five of twelve projects had three or four
    // distinct prompts among their last ten, so most of that section of the
    // context was one sentence printed several times.
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    fn ask(store: &mut Store, content: &str) {
        store
            .add_prompt(AddPrompt {
                session_id: "s1".to_owned(),
                content: content.to_owned(),
                project: Some("leteo".to_owned()),
            })
            .unwrap();
    }
    ask(&mut store, "what broke the search?");
    // Spaced out, because an echo within seconds is already refused on the way
    // in — see `one_prompt_recorded_twice_stays_one_prompt`. What accumulates
    // is the same thing asked again minutes later, which is the ordinary shape
    // of "carry on" and of a command run on a timer.
    for _ in 0..4 {
        ask(&mut store, "carry on");
        store
            .connection
            .execute(
                "UPDATE prompts SET created_at = datetime(created_at, '-5 minutes')",
                [],
            )
            .unwrap();
    }
    ask(&mut store, "why is the queue full?");

    let recent = store
        .recent_distinct_prompts(Some("leteo"), Some(10))
        .unwrap();
    let asked: Vec<&str> = recent.iter().map(|p| p.content.as_str()).collect();
    assert_eq!(
        asked,
        [
            "why is the queue full?",
            "carry on",
            "what broke the search?"
        ],
        "a repeat should take one place, not four"
    );

    // The one that survives is the most recent, so the timestamps stay in
    // order and "when was this last asked" keeps its answer.
    let repeated = recent
        .iter()
        .find(|p| p.content == "carry on")
        .expect("the repeat is still there once");
    let all_ids: Vec<i64> = store
        .connection
        .prepare("SELECT id FROM prompts WHERE content = 'carry on' ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(all_ids.len(), 4, "nothing was deleted, only folded");
    assert_eq!(repeated.id, *all_ids.last().unwrap());
}

#[test]
fn a_prompt_with_nothing_in_it_is_refused_like_a_memory_with_nothing_in_it() {
    // The door that refuses an empty memory did not exist for prompts, and a
    // real store held eleven rows recording that somebody had asked something
    // and not what. `mem_save_prompt` accepted an empty string, spaces, and a
    // newline and a tab, reporting success for all four.
    //
    // It costs more than a wasted row now. A prompt is what a memory is linked
    // to when it records the question it answers, so an empty one is a link to
    // nothing — and it takes one of the ten places the opening context keeps
    // for saying what somebody has been asking about.
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let ask = |store: &mut Store, content: &str| {
        store.add_prompt(AddPrompt {
            session_id: "s1".to_owned(),
            content: content.to_owned(),
            project: Some("leteo".to_owned()),
        })
    };
    for blank in ["", "   ", "\n\t ", "\u{a0}"] {
        assert!(
            ask(&mut store, blank).is_err(),
            "an empty prompt was stored: {blank:?}"
        );
    }
    assert!(ask(&mut store, "why is the queue full?").is_ok());

    // A prompt that was *all* private is not empty: it says a question was
    // asked and that its words are not for keeping.
    let redacted = ask(&mut store, "<private>the token is hunter2</private>").unwrap();
    assert_eq!(redacted.content, "[REDACTED]");

    let stored: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM prompts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(stored, 2);
}

/// A prompt arriving over the wire is held to the same rules as a typed one.
///
/// The observation path was fixed for exactly this and its sibling was left
/// behind: this one normalised nothing at all. A replicated prompt kept its
/// `<private>` spans verbatim, ignored the length cap, and stored the project
/// name however it was spelled — so `Leteo` never matched the `leteo` every
/// query narrows by, and the prompt was invisible to the opening context for
/// ever.
///
/// Redaction is the one that matters most. It is the promise that a secret
/// typed into a prompt is not kept, and a promise that only holds on the
/// machine it was typed on is not one.
#[test]
fn a_replicated_prompt_is_redacted_capped_and_filed_like_a_typed_one() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let long = "palabra ".repeat(store.config.max_observation_length);
    let mutation = SyncMutation {
        seq: 4,
        target_key: "cloud".to_owned(),
        entity: "prompt".to_owned(),
        entity_key: "prompt-from-a-peer".to_owned(),
        op: crate::sync::OP_UPSERT.to_owned(),
        payload: serde_json::json!({
            "sync_id": "prompt-from-a-peer",
            "session_id": "s1",
            "content": format!("¿por qué falla? <private>ghp_secreto</private> {long}"),
            "project": "LETEO",
            "created_at": "2026-08-05 04:00:00",
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

    let stored = store
        .connection
        .query_row(
            "SELECT content, project FROM prompts WHERE sync_id = 'prompt-from-a-peer'",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .unwrap();
    assert!(
        !stored.0.contains("ghp_secreto"),
        "a secret a peer sent is still a secret: {}",
        &stored.0[..80.min(stored.0.len())]
    );
    assert!(
        stored.0.len() <= store.config.max_observation_length + 32,
        "the length cap is the store's, not the sender's: {} bytes",
        stored.0.len()
    );
    assert_eq!(
        stored.1, "leteo",
        "a project spelled another way is the same project, or it is invisible"
    );

    // And invisible is what it was: the opening context narrows by the
    // normalised name.
    let listed = store
        .recent_distinct_prompts(Some("leteo"), Some(10))
        .unwrap();
    assert!(
        listed
            .iter()
            .any(|prompt| prompt.sync_id == "prompt-from-a-peer"),
        "the replicated prompt has to be reachable by its project"
    );
}

/// One question does not spend two of the places a session opens with.
///
/// Deduplicating by the text exactly catches a question retyped and misses the
/// shape its own note named first: a slash command in front of it. `/loop find
/// bugs` and `find bugs` are one request — typed once to start a loop and again
/// by the loop — and on a real store they were the two longest prompts in the
/// section, 444 bytes of 1,278.
#[test]
fn a_question_asked_with_and_without_its_slash_command_is_listed_once() {
    let (_temp, mut store) = store();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let ask = |store: &mut Store, content: &str| {
        store
            .add_prompt(AddPrompt {
                session_id: "s1".to_owned(),
                content: content.to_owned(),
                project: Some("leteo".to_owned()),
            })
            .unwrap();
    };
    ask(&mut store, "busca fallos en el core");
    ask(&mut store, "/loop busca fallos en el core");
    ask(&mut store, "  BUSCA   fallos  en el core  ");
    ask(&mut store, "$task-board busca fallos en el core");
    ask(&mut store, "otra pregunta distinta");

    let listed = store
        .recent_distinct_prompts(Some("leteo"), Some(10))
        .unwrap();
    assert_eq!(
        listed.len(),
        2,
        "four ways of asking one thing, plus one other: {:?}",
        listed.iter().map(|p| &p.content).collect::<Vec<_>>()
    );
    // The most recent wording is the one kept, unchanged.
    assert_eq!(listed[0].content, "otra pregunta distinta");
    assert_eq!(listed[1].content, "$task-board busca fallos en el core");

    // A bare slash command is the whole question, not a prefix on one, so two
    // different commands stay two.
    ask(&mut store, "/doctor");
    ask(&mut store, "/humo");
    let listed = store
        .recent_distinct_prompts(Some("leteo"), Some(10))
        .unwrap();
    assert_eq!(listed.len(), 4, "{listed:?}");
}

#[test]
fn a_question_asked_in_another_sitting_is_not_what_a_memory_answers() {
    // The window is the whole of what makes the project fallback safe, and
    // nothing held it. A save that names no session lands in a per-project
    // bucket prompts are never written to, so the only question available is
    // whatever the project was last asked — which is the right answer minutes
    // later and a fabrication a day later. "No question recorded" is the honest
    // state, and reporting the nearest one instead is exactly what this must
    // not do.
    let (_temp, mut store) = store();
    store.create_session("chat", "leteo", "C:/repo").unwrap();
    let asked = store
        .add_prompt(AddPrompt {
            session_id: "chat".to_owned(),
            content: "why does the clock start from the memory".to_owned(),
            project: Some("leteo".to_owned()),
        })
        .unwrap();

    // Just now: the question this memory answers.
    assert_eq!(
        store.prompt_behind_a_save("manual-save-leteo", "leteo", false),
        Some(asked.sync_id.clone()),
        "a sessionless save takes the project's last question"
    );

    // Older than the window by a minute, which is the only difference.
    store
        .connection
        .execute(
            "UPDATE prompts SET created_at = datetime('now', ?1) WHERE id = ?2",
            rusqlite::params![
                format!("-{} minutes", PROMPT_ATTRIBUTION_MINUTES + 1),
                asked.id
            ],
        )
        .unwrap();
    assert_eq!(
        store.prompt_behind_a_save("manual-save-leteo", "leteo", false),
        None,
        "a question from another sitting is not what this memory answers"
    );

    // And the session rule has no window at all, which is right: a conversation
    // is one conversation however long it has been open, and the bucket above
    // is not one. The same prompt, now hours old, still answers a save that
    // names the session it was asked in.
    assert_eq!(
        store.prompt_behind_a_save("chat", "leteo", true),
        Some(asked.sync_id),
        "a conversation's own question does not expire with the clock"
    );
}
