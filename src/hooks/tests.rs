use std::path::Path;

use tempfile::TempDir;

use super::*;
use crate::{
    memory::model::AddObservation,
    store::{Store, StoreConfig},
};

fn store() -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    let store = Store::open(StoreConfig::new(temp.path().join("hooks.db"))).unwrap();
    // The language Sardi speaks falls back to the machine's own when nothing is
    // set, so the assertions below about English greetings would otherwise pass
    // or fail depending on where the machine was bought. A test that means to
    // exercise a language writes its own settings over this one.
    crate::settings::save(
        temp.path(),
        &crate::settings::Settings {
            interface: Some(crate::settings::Interface::English),
            voice_language: None,
            ..crate::settings::Settings::default()
        },
    )
    .unwrap();
    (temp, store)
}

fn input(directory: &Path) -> HookInput {
    HookInput {
        session_id: "agent-session".to_owned(),
        cwd: directory.to_string_lossy().into_owned(),
        project: Some("hook-project".to_owned()),
        ..HookInput::default()
    }
}

#[test]
fn hook_input_tolerates_empty_and_partial_payloads() {
    assert_eq!(read_input("".as_bytes()).unwrap().session_id, "");
    assert_eq!(read_input("   \n".as_bytes()).unwrap().cwd, "");
    let parsed = read_input(r#"{"session_id":"abc","unknown_field":true}"#.as_bytes()).unwrap();
    assert_eq!(parsed.session_id, "abc");
    assert!(read_input("{".as_bytes()).is_err());
}

#[test]
fn the_voice_setting_decides_which_lines_a_hook_shows() {
    use crate::settings::{self, Settings, Voice};

    let (temp, mut store) = store();
    run(&mut store, HookEvent::SessionStart, &input(temp.path())).unwrap();
    store
        .add_observation(AddObservation {
            session_id: "agent-session".to_owned(),
            kind: "decision".to_owned(),
            title: "Something worth greeting somebody with".to_owned(),
            content: "a memory".to_owned(),
            tool_name: None,
            project: Some("hook-project".to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            prompt_sync_id: None,
        })
        .unwrap();

    // A report, and a reminder on a project that has gone quiet. The two
    // are governed separately, so both have to be in play at once.
    let greeting = |store: &mut Store| {
        run(store, HookEvent::SessionStart, &input(temp.path()))
            .unwrap()
            .system_message
    };
    let reminder = |store: &mut Store| {
        // Aged through a second connection, the way the other reminder
        // tests here do it.
        let aged = rusqlite::Connection::open(store.database_path()).unwrap();
        aged.execute(
            "UPDATE sessions SET started_at = datetime('now', '-200 minutes')",
            [],
        )
        .unwrap();
        aged.execute(
            "UPDATE observations SET created_at = datetime('now', '-200 minutes')",
            [],
        )
        .unwrap();
        let _ = std::fs::remove_file(nudge_state_path(store, "agent-session").unwrap());
        run(store, HookEvent::UserPromptSubmit, &input(temp.path()))
            .unwrap()
            .system_message
    };

    for (voice, wants_report, wants_reminder) in [
        (Voice::All, true, true),
        (Voice::Reminders, false, true),
        (Voice::Quiet, false, false),
    ] {
        settings::save(
            temp.path(),
            &Settings {
                voice,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            greeting(&mut store).is_some(),
            wants_report,
            "greeting at {voice:?}"
        );
        assert_eq!(
            reminder(&mut store).is_some(),
            wants_reminder,
            "reminder at {voice:?}"
        );
    }
}

#[test]
fn a_silenced_leteo_does_not_stamp_the_reminder_clock() {
    // The debounce is written as a side effect of deciding to remind. Kept
    // stamping while silent, it would hand somebody a reminder the moment
    // they turned the voice back up — or withhold one they were due.
    use crate::settings::{self, Settings, Voice};

    let (temp, mut store) = store();
    run(&mut store, HookEvent::SessionStart, &input(temp.path())).unwrap();
    rusqlite::Connection::open(store.database_path())
        .unwrap()
        .execute(
            "UPDATE sessions SET started_at = datetime('now', '-200 minutes')",
            [],
        )
        .unwrap();

    settings::save(
        temp.path(),
        &Settings {
            language: None,
            voice: Voice::Quiet,
            interface: Some(settings::Interface::English),
            voice_language: None,
            context_size: None,
        },
    )
    .unwrap();
    run(&mut store, HookEvent::UserPromptSubmit, &input(temp.path())).unwrap();
    // The file may well exist — the hint keeps its own list in it — but the
    // clock inside must not have been stamped, because that is what would
    // withhold the reminder that is due the moment the voice comes back up.
    let state = nudge_state_path(&store, "agent-session").unwrap();
    assert!(
        crate::hooks::nudge::SessionState::read(Some(&state))
            .last_nudge
            .is_none(),
        "a silent run must leave no clock behind"
    );

    settings::save(
        temp.path(),
        &Settings {
            voice: Voice::All,
            ..Default::default()
        },
    )
    .unwrap();
    let spoken = run(&mut store, HookEvent::UserPromptSubmit, &input(temp.path())).unwrap();
    assert!(spoken.system_message.is_some(), "{spoken:?}");
    assert!(state.exists(), "and now the clock is stamped");
}

#[test]
fn a_prompt_surfaces_a_relevant_memory_and_stays_quiet_otherwise() {
    // The point of the relevance test. Searching on every prompt finds
    // *something* four times in five, and reading twenty by hand only about
    // a third were worth surfacing — so a section that always appears is
    // one the agent learns to skip. Silence has to be the common case.
    let (temp, mut store) = store();
    run(&mut store, HookEvent::SessionStart, &input(temp.path())).unwrap();
    let mut save = |title: &str, content: &str| {
        store
            .add_observation(AddObservation {
                session_id: "agent-session".to_owned(),
                kind: "bugfix".to_owned(),
                title: title.to_owned(),
                content: content.to_owned(),
                tool_name: None,
                project: Some("hook-project".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
    };
    // One memory that answers the question, and a handful that merely share
    // its ordinary words — which is what a real store looks like.
    save(
        "Fixed the paginated closing verification",
        "pagination broke the closing verification step",
    );
    for n in 1..=6 {
        save(
            &format!("Unrelated change {n}"),
            &format!("the step was changed again for reason {n}"),
        );
    }

    let mut asked = input(temp.path());
    asked.prompt = "the paginated closing verification is broken".to_owned();
    let hit = run(&mut store, HookEvent::UserPromptSubmit, &asked).unwrap();
    let recall = hit
        .additional_context
        .as_deref()
        .expect("a prompt about the memory surfaces it");
    assert!(
        recall.contains("Fixed the paginated closing verification"),
        "{recall}"
    );
    assert!(recall.contains("mem_get_observation"), "{recall}");
    assert!(
        !recall.contains("Unrelated change"),
        "only what stands out: {recall}"
    );

    let mut unrelated = input(temp.path());
    unrelated.prompt = "buenos dias, que tal va todo hoy".to_owned();
    let quiet = run(&mut store, HookEvent::UserPromptSubmit, &unrelated).unwrap();
    assert!(
        quiet.additional_context.is_none(),
        "an unrelated prompt must not be interrupted: {:?}",
        quiet.additional_context
    );

    assert!(hit.prompt_saved && quiet.prompt_saved);
}

#[test]
fn session_start_creates_the_session_and_injects_protocol_and_memory() {
    let (temp, mut store) = store();
    let outcome = run(&mut store, HookEvent::SessionStart, &input(temp.path())).unwrap();

    assert_eq!(outcome.event, "SessionStart");
    assert_eq!(outcome.project, "hook-project");
    assert!(outcome.session_created);
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    let context = outcome.additional_context.as_deref().unwrap();
    // The short directive, not the whole protocol. The full text lives in
    // the plugin's skill, where it is paid for when it is needed instead of
    // on every session; injecting both cost the tokens twice and left two
    // copies free to drift.
    assert!(context.contains("Leteo memory — active"), "{context}");
    assert!(
        context.contains("Saving is not replying"),
        "the rule that fails silently has to be in the directive: {context}"
    );
    assert!(
        !context.contains("HOW TO WRITE A MEMORY"),
        "reference belongs in the skill, not in every session: {context}"
    );
    assert!(
        context.len() < 1200,
        "the directive is meant to be short, it is {} characters",
        context.len()
    );

    store
        .add_observation(AddObservation {
            session_id: "agent-session".to_owned(),
            kind: "decision".to_owned(),
            title: "Hook decision".to_owned(),
            content: "the hook injects prior memory".to_owned(),
            tool_name: None,
            project: Some("hook-project".to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            prompt_sync_id: None,
        })
        .unwrap();

    let second = run(&mut store, HookEvent::SessionStart, &input(temp.path())).unwrap();
    assert!(!second.session_created, "the session is reused");
    let context = second.additional_context.as_deref().unwrap();
    assert!(context.contains("Hook decision"));

    let response = second.response();
    assert_eq!(
        response["hookSpecificOutput"]["hookEventName"],
        "SessionStart"
    );
    assert!(
        response["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .is_some_and(|value| value.contains("Hook decision"))
    );
}

#[test]
fn post_compaction_recovers_context_without_the_full_protocol() {
    let (temp, mut store) = store();
    run(&mut store, HookEvent::SessionStart, &input(temp.path())).unwrap();

    let outcome = run(&mut store, HookEvent::PostCompaction, &input(temp.path())).unwrap();

    let context = outcome.additional_context.as_deref().unwrap();
    assert!(context.contains("Context was compacted"));
    assert!(context.contains("mem_session_summary"));
    assert!(!context.contains("### Save important work"));
}

#[test]
fn user_prompts_are_captured_and_quiet_projects_are_nudged() {
    let (temp, mut store) = store();
    let mut payload = input(temp.path());
    payload.prompt = "Why did we choose SQLite?".to_owned();
    run(&mut store, HookEvent::SessionStart, &payload).unwrap();

    let outcome = run(&mut store, HookEvent::UserPromptSubmit, &payload).unwrap();
    assert!(outcome.prompt_saved);
    assert!(
        outcome.system_message.is_none(),
        "a new session is not nudged"
    );
    let prompts = store
        .recent_prompts(Some("hook-project"), Some(10))
        .unwrap();
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].content, "Why did we choose SQLite?");

    // Age the session past the warm-up window through a second connection.
    rusqlite::Connection::open(store.database_path())
        .unwrap()
        .execute(
            "UPDATE sessions SET started_at = datetime('now', '-60 minutes')",
            [],
        )
        .unwrap();
    let nudged = run(&mut store, HookEvent::UserPromptSubmit, &payload).unwrap();
    let message = nudged.system_message.as_deref().expect("nudge message");
    assert!(message.contains("mem_save"));

    // The reminder is debounced for the rest of the cooldown.
    let quiet = run(&mut store, HookEvent::UserPromptSubmit, &payload).unwrap();
    assert!(quiet.system_message.is_none());
    assert!(quiet.prompt_saved);

    // Ending the session reclaims its reminder state instead of leaving a
    // file behind for every conversation ever held.
    let state_path = nudge_state_path(&store, "agent-session").expect("a state path");
    assert!(state_path.exists());
    run(&mut store, HookEvent::SessionStop, &payload).unwrap();
    assert!(!state_path.exists());
}

#[test]
fn subagent_output_is_captured_and_session_stop_ends_the_session() {
    let (temp, mut store) = store();
    let mut payload = input(temp.path());
    run(&mut store, HookEvent::SessionStart, &payload).unwrap();

    payload.stdout =
        "## Key Learnings:\n1. Hook payloads must never block the agent's critical path\n"
            .to_owned();
    let captured = run(&mut store, HookEvent::SubagentStop, &payload).unwrap();
    assert_eq!(captured.observations_captured, 1);

    let stopped = run(&mut store, HookEvent::SessionStop, &payload).unwrap();
    assert!(stopped.warnings.is_empty(), "{:?}", stopped.warnings);
    assert!(
        store
            .get_session("agent-session")
            .unwrap()
            .ended_at
            .is_some()
    );
    assert_eq!(stopped.response(), serde_json::json!({}));
}

#[test]
fn an_existing_session_keeps_its_project_when_the_agent_changes_directory() {
    let temp = TempDir::new().unwrap();
    let first = temp.path().join("first-repo");
    let second = temp.path().join("second-repo");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    let mut store = Store::open(StoreConfig::new(temp.path().join("hooks.db"))).unwrap();

    let start = HookInput {
        session_id: "roaming".to_owned(),
        cwd: first.to_string_lossy().into_owned(),
        project: Some("first-project".to_owned()),
        ..HookInput::default()
    };
    run(&mut store, HookEvent::SessionStart, &start).unwrap();

    // The same conversation, now reporting a different directory.
    let moved = HookInput {
        cwd: second.to_string_lossy().into_owned(),
        project: Some("second-project".to_owned()),
        prompt: "still the same conversation".to_owned(),
        ..start.clone()
    };
    let outcome = run(&mut store, HookEvent::UserPromptSubmit, &moved).unwrap();

    assert_eq!(
        outcome.project, "first-project",
        "the session that already exists owns the project"
    );
    assert!(outcome.prompt_saved);
    assert_eq!(
        store
            .recent_prompts(Some("first-project"), Some(10))
            .unwrap()
            .len(),
        1
    );
    assert!(
        store
            .recent_prompts(Some("second-project"), Some(10))
            .unwrap()
            .is_empty(),
        "the conversation is not split across projects"
    );
}

#[test]
fn a_project_recorded_elsewhere_is_never_folded_into_this_one() {
    let temp = TempDir::new().unwrap();
    // A checkout whose directory is named "api" but whose project resolves
    // to something else, and an unrelated "api" project living elsewhere.
    let elsewhere = temp.path().join("other-checkout");
    let workspace = temp.path().join("api");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let mut store = Store::open(StoreConfig::new(temp.path().join("hooks.db"))).unwrap();
    store
        .create_session("unrelated", "api", &elsewhere.to_string_lossy())
        .unwrap();
    store
        .add_observation(AddObservation {
            session_id: "unrelated".to_owned(),
            kind: "decision".to_owned(),
            title: "Someone else's api memory".to_owned(),
            content: "this belongs to a different checkout".to_owned(),
            tool_name: None,
            project: Some("api".to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            prompt_sync_id: None,
        })
        .unwrap();

    let outcome = run(
        &mut store,
        HookEvent::SessionStart,
        &HookInput {
            session_id: "new-session".to_owned(),
            cwd: workspace.to_string_lossy().into_owned(),
            project: Some("backend".to_owned()),
            ..HookInput::default()
        },
    )
    .unwrap();

    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    assert_eq!(
        store
            .recent_observations(Some("api"), Some(10), true)
            .unwrap()
            .len(),
        1,
        "the unrelated project keeps its memories"
    );
    assert!(
        store
            .recent_observations(Some("backend"), Some(10), true)
            .unwrap()
            .is_empty(),
        "nothing was folded into the new project"
    );
}

#[test]
fn an_ambiguous_directory_warns_instead_of_writing_to_a_guessed_project() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    for repository in ["service-a", "service-b"] {
        let path = workspace.join(repository);
        std::fs::create_dir_all(&path).unwrap();
        let initialized = std::process::Command::new("git")
            .arg("init")
            .arg("--quiet")
            .arg(&path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !initialized {
            return; // git is unavailable in this environment
        }
    }
    let mut store = Store::open(StoreConfig::new(temp.path().join("hooks.db"))).unwrap();

    let outcome = run(
        &mut store,
        HookEvent::SessionStart,
        &HookInput {
            session_id: "ambiguous-session".to_owned(),
            cwd: workspace.to_string_lossy().into_owned(),
            ..HookInput::default()
        },
    )
    .unwrap();

    assert!(outcome.project.is_empty());
    assert!(!outcome.session_created);
    assert!(outcome.additional_context.is_none());
    assert_eq!(
        outcome.warnings,
        ["could not determine the project for this hook"]
    );
    assert_eq!(store.stats().unwrap().total_sessions, 0);
}

#[test]
fn a_renamed_project_folds_the_directory_name_into_the_detected_project() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("legacy-name");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut store = Store::open(StoreConfig::new(temp.path().join("hooks.db"))).unwrap();
    store
        .create_session("old", "legacy-name", &workspace.to_string_lossy())
        .unwrap();
    store
        .add_observation(AddObservation {
            session_id: "old".to_owned(),
            kind: "decision".to_owned(),
            title: "Legacy memory".to_owned(),
            content: "saved before the project was renamed".to_owned(),
            tool_name: None,
            project: Some("legacy-name".to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            prompt_sync_id: None,
        })
        .unwrap();

    let outcome = run(
        &mut store,
        HookEvent::SessionStart,
        &HookInput {
            session_id: "renamed-session".to_owned(),
            cwd: workspace.to_string_lossy().into_owned(),
            project: Some("renamed-project".to_owned()),
            ..HookInput::default()
        },
    )
    .unwrap();

    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    assert_eq!(
        store
            .recent_observations(Some("renamed-project"), Some(10), true)
            .unwrap()
            .len(),
        1
    );
    assert!(
        store
            .recent_observations(Some("legacy-name"), Some(10), true)
            .unwrap()
            .is_empty()
    );
}

fn memory(title: &str, content: &str) -> crate::memory::model::AddObservation {
    crate::memory::model::AddObservation {
        session_id: "agent-session".to_owned(),
        kind: "decision".to_owned(),
        title: title.to_owned(),
        content: content.to_owned(),
        tool_name: None,
        project: Some("hook-project".to_owned()),
        scope: "project".to_owned(),
        topic_key: None,
        prompt_sync_id: None,
    }
}

#[test]
fn a_memory_a_later_one_overturned_is_handed_over_saying_so() {
    let (temp, mut store) = store();
    let payload = input(temp.path());
    run(&mut store, HookEvent::SessionStart, &payload).unwrap();

    // The recall threshold names a memory only when it stands out from the
    // ordinary match for that query, so the corpus needs relief: a crowd that
    // shares one word, and one memory that answers the question.
    for index in 0..14 {
        store
            .add_observation(memory(
                &format!("Engine note {index}"),
                "A passing remark about the rendering engine and nothing else",
            ))
            .unwrap();
    }
    let answer = store
        .add_observation(memory(
            "We chose SQLite as the storage engine",
            "The storage engine question was settled: SQLite, chosen for being local-first and a \
             single file",
        ))
        .unwrap()
        .observation;

    let mut prompt = payload.clone();
    prompt.prompt = "Which storage engine did we choose?".to_owned();
    let before = run(&mut store, HookEvent::UserPromptSubmit, &prompt)
        .unwrap()
        .additional_context
        .expect("the prompt matches something");
    assert!(
        !before.contains("superseded by"),
        "nothing has been overturned yet: {before}"
    );

    assert!(
        before.contains(&format!("- #{}", answer.id)),
        "the memory that answers the question is the one named: {before}"
    );

    let newer = store
        .add_observation(memory(
            "We moved the storage engine to Postgres",
            "The storage engine is no longer SQLite",
        ))
        .unwrap()
        .observation;
    let relation = store
        .save_relation(crate::memory::model::SaveRelationParams {
            sync_id: crate::memory::normalize::sync_id("rel"),
            source_id: newer.sync_id.clone(),
            target_id: answer.sync_id,
        })
        .unwrap();
    store
        .judge_relation(crate::memory::model::JudgeRelationParams {
            judgment_id: relation.sync_id,
            relation: crate::store::RELATION_SUPERSEDES.to_owned(),
            marked_by_actor: "agent".to_owned(),
            marked_by_kind: "agent".to_owned(),
            ..Default::default()
        })
        .unwrap();

    let after = run(&mut store, HookEvent::UserPromptSubmit, &prompt)
        .unwrap()
        .additional_context
        .expect("the prompt still matches");

    assert!(
        after.contains(&format!("(superseded by #{}", newer.id)),
        "the agent has to be told the memory was overturned: {after}"
    );
    assert!(
        after.contains("We moved the storage engine to Postgres"),
        "and by what, or the warning is not actionable: {after}"
    );
}

/// Files a pending pair the way a save that found a candidate leaves one, and
/// answers with its `judgment_id`.
fn propose(store: &mut Store, source: &str, target: &str) -> String {
    let sync_id = crate::memory::normalize::sync_id("rel");
    store
        .save_relation(crate::memory::model::SaveRelationParams {
            sync_id: sync_id.clone(),
            source_id: source.to_owned(),
            target_id: target.to_owned(),
        })
        .unwrap();
    sync_id
}

/// Just the handover, out of the whole opening block.
///
/// The assertions below are about what this section carries and — as much —
/// what it does not, and the same memories are listed a few hundred bytes
/// above it under the recent ones. Checked against the whole context, "the
/// body is not handed over" passes or fails on the wrong section.
fn verdict_block(context: &str) -> &str {
    let start = context
        .find("## Waiting on a verdict")
        .unwrap_or_else(|| panic!("no handover in this opening block: {context}"));
    &context[start..]
}

/// Moves a pair's clock back, because the ordering under test is by date and
/// six relations filed in one test all carry the same second.
fn waiting_since(store: &Store, judgment_id: &str, days: i64) {
    store
        .connection()
        .execute(
            "UPDATE memory_relations SET created_at = datetime('now', ?2) WHERE sync_id = ?1",
            rusqlite::params![judgment_id, format!("-{days} days")],
        )
        .unwrap();
}

#[test]
fn a_waiting_verdict_reaches_the_agent_and_never_the_person() {
    // Sardi used to say "1 pair waiting on a verdict" and the sentence was
    // dropped in twelve languages, deliberately. Judging a pair is Leteo's
    // bookkeeping: the agent settles it in the opening turn without asking, so
    // the count was telling somebody about work already being done for them,
    // in a line they could not act on and were not meant to. What is left in
    // the greeting is what is genuinely theirs — how much is remembered, and
    // which memories have come round to be read again.
    let (temp, mut store) = store();
    let payload = input(temp.path());
    run(&mut store, HookEvent::SessionStart, &payload).unwrap();
    let first = store
        .add_observation(memory("Tabs are the rule", "we indent with tabs"))
        .unwrap()
        .observation;
    let second = store
        .add_observation(memory("Spaces are the rule", "we indent with spaces"))
        .unwrap()
        .observation;
    let judgment = propose(&mut store, &first.sync_id, &second.sync_id);

    let opened = run(&mut store, HookEvent::SessionStart, &payload).unwrap();
    let greeting = opened
        .system_message
        .expect("a store with memories still speaks");
    assert!(
        greeting.contains("remembers"),
        "the half of the greeting that is the person's stays: {greeting}"
    );
    for said in ["verdict", "mem_judge", &judgment] {
        assert!(
            !greeting.contains(said),
            "the person is not told about {said:?}: {greeting}"
        );
    }
    assert!(
        opened
            .additional_context
            .expect("a session opening carries context")
            .contains(&judgment),
        "and the pair still reaches the agent, which is the whole point"
    );
}

#[test]
fn a_session_opening_hands_over_the_pairs_it_used_to_only_count() {
    // The count was added and the pairs were not, which left an agent told
    // there was something to do and given nothing to do it with: `mem_judge`
    // takes a `judgment_id` that only `mem_save` ever returned, so a pair
    // missed when it was proposed had no route back short of the command line.
    let (temp, mut store) = store();
    let payload = input(temp.path());
    run(&mut store, HookEvent::SessionStart, &payload).unwrap();
    let first = store
        .add_observation(crate::memory::model::AddObservation {
            topic_key: Some("style/indentation".to_owned()),
            ..memory("Tabs are the rule", "we indent with tabs")
        })
        .unwrap()
        .observation;
    let second = store
        .add_observation(memory("Spaces are the rule", "we indent with spaces"))
        .unwrap()
        .observation;

    let quiet = run(&mut store, HookEvent::SessionStart, &payload)
        .unwrap()
        .additional_context
        .expect("a session opening carries context");
    assert!(
        !quiet.contains("Waiting on a verdict"),
        "nothing has been proposed yet: {quiet}"
    );

    let judgment = propose(&mut store, &first.sync_id, &second.sync_id);
    let context = run(&mut store, HookEvent::SessionStart, &payload)
        .unwrap()
        .additional_context
        .expect("a session opening carries context");
    let handed = verdict_block(&context);

    assert!(
        handed.contains(&judgment),
        "the pair is useless without the id mem_judge takes: {handed}"
    );
    for observation in [&first, &second] {
        assert!(
            handed.contains(&format!("#{}", observation.id)),
            "both ends are named by the id mem_get_observation takes: {handed}"
        );
        assert!(
            handed.contains(&observation.title),
            "and by what they say, or there is nothing to rule on: {handed}"
        );
    }
    assert!(
        handed.contains("(style/indentation)"),
        "the topic key is the cheapest thing a verdict turns on, so it is carried: {handed}"
    );
    // And the bodies are not, which is the whole of why this block is affordable
    // enough to send unasked. Two previews of 300 characters cost about four
    // times the rest of a pair's entry, on a surface whose point is to cost less
    // than it saves.
    assert!(
        !handed.contains("we indent with tabs") && !handed.contains("we indent with spaces"),
        "a pair costs two lines, not two previews: {handed}"
    );
    // No verdict here is the user's. Judging is bookkeeping they never asked
    // for, and a question about two memories they do not remember writing is an
    // interruption charged against the thing Leteo is supposed to be saving.
    for verb in [
        "related",
        "compatible",
        "scoped",
        "conflicts_with",
        "supersedes",
        "not_conflict",
    ] {
        assert!(
            handed.contains(verb),
            "every verdict is the agent's, so every one is named: {handed}"
        );
    }
    assert!(
        handed.contains("Never put a verdict to the user"),
        "and the instruction says so rather than leaving it to be inferred: {handed}"
    );
}

#[test]
fn the_pairs_handed_over_are_the_ones_that_have_waited_longest() {
    // Ordering is the whole of why nothing starves. Newest-first would offer
    // the same recent pairs at every opening while the ones already forgotten
    // stayed forgotten, which is how the oldest on a real store reached eight
    // weeks. Filed newest-first on purpose, so insertion order cannot stand in
    // for the date and let this pass while the ORDER BY says something else.
    let (temp, mut store) = store();
    let payload = input(temp.path());
    run(&mut store, HookEvent::SessionStart, &payload).unwrap();

    let mut judgments = Vec::new();
    for age in [1_i64, 2, 8, 15, 30, 60] {
        let left = store
            .add_observation(memory(
                &format!("Left of the pair {age} days old"),
                "one side of a proposal",
            ))
            .unwrap()
            .observation;
        let right = store
            .add_observation(memory(
                &format!("Right of the pair {age} days old"),
                "the other side of a proposal",
            ))
            .unwrap()
            .observation;
        let judgment = propose(&mut store, &left.sync_id, &right.sync_id);
        waiting_since(&store, &judgment, age);
        judgments.push((age, judgment));
    }

    let handed = run(&mut store, HookEvent::SessionStart, &payload)
        .unwrap()
        .additional_context
        .expect("a session opening carries context");

    for (age, judgment) in &judgments {
        let handed_over = handed.contains(judgment);
        assert_eq!(
            handed_over,
            *age > 1,
            "the five oldest of six are handed over, and the pair {age} days old \
             {}: {handed}",
            if handed_over { "was" } else { "was not" }
        );
    }
    // And what is not in front of the agent is said rather than left to be
    // inferred from a block that happens to hold three.
    assert!(
        handed.contains("1 more waiting"),
        "the rest are counted where the pairs are: {handed}"
    );
}

/// The spec says how many pairs an opening hands over, and the code hands over
/// that many.
///
/// Shaped after `the_skill_promises_the_preview_length_the_code_cuts_at`, for
/// the same reason and against a different document. `hooks.md` §13 is where
/// somebody reads what an opening block contains before touching this code, and
/// a number changed here without the sentence leaves them planning against a
/// size the code stopped using.
#[test]
fn the_spec_publishes_the_number_of_pairs_the_opening_hands_over() {
    let spelled = match crate::hooks::context::VERDICT_HANDOVER {
        2 => "two oldest",
        3 => "three oldest",
        4 => "four oldest",
        5 => "five oldest",
        other => panic!(
            "an opening now hands over {other} pairs; spell it out here and in \
             openspec/specs/hooks.md §13"
        ),
    };
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("openspec/specs/hooks.md");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    assert!(
        text.contains(spelled),
        "hooks.md has to say the opening carries the {spelled} pairs"
    );
}

#[test]
fn a_memory_deleted_since_still_leaves_a_pair_worth_closing() {
    // The claim this replaces was false, and the test that asserted it made
    // the falsehood permanent. A *soft* delete keeps the row, so
    // `validate_cross_project_guard` finds the memory and the verdict records
    // — measured below rather than argued. Telling the agent the pair "cannot
    // be ruled on" parked forever a pair that one call closes.
    let (temp, mut store) = store();
    let payload = input(temp.path());
    run(&mut store, HookEvent::SessionStart, &payload).unwrap();
    let kept = store
        .add_observation(memory(
            "The one that stays",
            "still here to compare against",
        ))
        .unwrap()
        .observation;
    let removed = store
        .add_observation(memory("The one that goes", "deleted after it was proposed"))
        .unwrap()
        .observation;
    let judgment = propose(&mut store, &kept.sync_id, &removed.sync_id);
    // Soft, which is the ordinary one: the row stays and the listing's own
    // `deleted_at IS NULL` is what hides it.
    store.delete_observation(removed.id, false).unwrap();

    let context = run(&mut store, HookEvent::SessionStart, &payload)
        .unwrap()
        .additional_context
        .expect("a session opening carries context");
    let handed = verdict_block(&context);

    assert!(
        handed.contains(&judgment),
        "the pair is offered, because closing it is a call that works: {handed}"
    );
    assert!(
        handed.contains("(deleted since this pair was proposed)"),
        "with the side that is gone named as gone: {handed}"
    );
    assert!(
        handed.contains("The one that stays"),
        "and the side that is left described: {handed}"
    );
    assert!(
        !handed.contains("cannot settle at all"),
        "this is not one of the pairs nothing can settle: {handed}"
    );

    // The measurement the wording rests on. Without this the block is back to
    // asserting what somebody believed about soft deletes.
    store
        .judge_relation(crate::memory::model::JudgeRelationParams {
            judgment_id: judgment,
            relation: "not_conflict".to_owned(),
            ..Default::default()
        })
        .expect("a soft-deleted memory still leaves a pair mem_judge accepts");
}

#[test]
fn a_pair_mem_judge_can_never_settle_is_counted_and_never_offered() {
    // `mem_judge` refuses a relation whose two ends are in different projects,
    // every time and for good. Offered in a queue ordered by age, such a pair
    // takes the head and keeps it — turning "nothing starves" into "nothing
    // else ever gets a turn". So it is filtered out of the work and named in
    // one sentence instead.
    let (temp, mut store) = store();
    let payload = input(temp.path());
    run(&mut store, HookEvent::SessionStart, &payload).unwrap();
    let here = store
        .add_observation(memory("Belongs to this project", "one side"))
        .unwrap()
        .observation;
    let elsewhere = store
        .add_observation(crate::memory::model::AddObservation {
            project: Some("somewhere-else".to_owned()),
            ..memory("Belongs to another project", "the other side")
        })
        .unwrap()
        .observation;
    let stuck = propose(&mut store, &here.sync_id, &elsewhere.sync_id);

    // Refused, measured. The filter exists because of this answer, not because
    // of a reading of the guard.
    let refused = store.judge_relation(crate::memory::model::JudgeRelationParams {
        judgment_id: stuck.clone(),
        relation: "related".to_owned(),
        ..Default::default()
    });
    assert!(
        refused.is_err(),
        "a cross-project pair is what mem_judge will not take"
    );

    let context = run(&mut store, HookEvent::SessionStart, &payload)
        .unwrap()
        .additional_context
        .expect("a session opening carries context");
    let handed = verdict_block(&context);
    assert!(
        !handed.contains(&stuck),
        "it is never offered as work: {handed}"
    );
    assert!(
        handed.contains("1 that mem_judge cannot settle at all"),
        "and it is not hidden either, because silence is what this block exists to end: {handed}"
    );

    // And a judgeable pair filed later still gets its turn, which is the whole
    // point of keeping the stuck one out of the ordering.
    let a = store
        .add_observation(memory("A newer pair", "left"))
        .unwrap()
        .observation;
    let b = store
        .add_observation(memory("Its other half", "right"))
        .unwrap()
        .observation;
    let fresh = propose(&mut store, &a.sync_id, &b.sync_id);
    let context = run(&mut store, HookEvent::SessionStart, &payload)
        .unwrap()
        .additional_context
        .expect("a session opening carries context");
    assert!(
        verdict_block(&context).contains(&fresh),
        "the older stuck pair does not hold the queue: {context}"
    );
}
#[test]
fn the_hook_speaks_in_the_voices_language_rather_than_the_screens() {
    // This is the whole of why the two settings are separate, and it is only
    // observable here. Sardi's lines leave the program: they are written into
    // an agent's conversation, beside whatever language that conversation is
    // being held in. Leteo's own screens are somewhere else entirely, so
    // somebody working with an agent in one language and reading the dashboard
    // in another has to be able to say so.
    let (temp, mut store) = store();
    // The session first, then something for the greeting to be about: a store
    // with nothing in it says nothing at all, in any language.
    run(&mut store, HookEvent::SessionStart, &input(temp.path())).unwrap();
    store.add_observation(memory("Something", "body")).unwrap();
    let mut greeting = |voice_language| {
        crate::settings::save(
            temp.path(),
            &crate::settings::Settings {
                interface: Some(crate::settings::Interface::Spanish),
                voice_language,
                ..crate::settings::Settings::default()
            },
        )
        .unwrap();
        run(&mut store, HookEvent::SessionStart, &input(temp.path()))
            .unwrap()
            .system_message
    };

    let spanish = greeting(None).expect("a store with something in it is greeted");
    assert_eq!(
        spanish,
        crate::sardi::remembers(crate::settings::Interface::Spanish, 1)
            .expect("one memory is worth mentioning"),
        "the voice must follow the screens until it is given a language"
    );

    let english = greeting(Some(crate::settings::Interface::English))
        .expect("still greeted, in the other language");
    assert_eq!(
        english,
        crate::sardi::remembers(crate::settings::Interface::English, 1)
            .expect("one memory is worth mentioning"),
    );
    assert_ne!(
        english, spanish,
        "the setting changed and the line the agent reads did not"
    );
}

#[test]
fn a_silenced_leteo_does_not_mention_the_queue_either() {
    let (temp, mut store) = store();
    crate::settings::save(
        temp.path(),
        &crate::settings::Settings {
            language: None,
            voice: crate::settings::Voice::Quiet,
            interface: Some(crate::settings::Interface::English),
            voice_language: None,
            context_size: None,
        },
    )
    .unwrap();
    let payload = input(temp.path());
    run(&mut store, HookEvent::SessionStart, &payload).unwrap();
    // The reread queue, which is the newest of the lines a person is shown and
    // therefore the one most likely to have been added without the setting in
    // mind. The verdicts used to be checked here too; that line is gone, and
    // the pairs now reach the agent through `additional_context`, which the
    // voice setting has no business silencing.
    let saved = store
        .add_observation(memory("One", "body"))
        .unwrap()
        .observation;
    store
        .connection()
        .execute(
            "UPDATE observations SET review_after = datetime('now', '-1 day') WHERE id = ?1",
            rusqlite::params![saved.id],
        )
        .unwrap();
    assert_eq!(store.count_review_due(Some("hook-project")).unwrap(), 1);

    assert_eq!(
        run(&mut store, HookEvent::SessionStart, &payload)
            .unwrap()
            .system_message,
        None,
        "a new line still has to obey the setting that silences the rest"
    );
}

#[test]
fn a_subagent_stop_from_claude_code_is_captured_the_same_as_one_from_opencode() {
    // Passive capture read `stdout` and nothing else. The OpenCode plugin
    // builds this payload itself and uses that name; Claude Code and Codex
    // send their own schema, where the subagent's final text is
    // `last_assistant_message`. The field defaults, so their payload parsed
    // cleanly, the hook reported success, and `observations_captured` was 0
    // on every subagent — indistinguishable from a subagent that said nothing
    // worth keeping. A real store of 3,530 memories held not one passively
    // captured memory.
    let learning =
        "## Key Learnings:\n1. The retry budget has to be per host and not per request\n";

    // Claude Code's shape, verbatim down to the fields Leteo does not read.
    let payload = format!(
        r#"{{"session_id":"agent-session","cwd":"{}","hook_event_name":"SubagentStop",
            "agent_id":"a1","agent_type":"Explore","transcript_path":"C:/x.jsonl",
            "last_assistant_message":{}}}"#,
        "/tmp/leteo-hooks",
        serde_json::to_string(learning).unwrap()
    );
    let parsed = read_input(payload.as_bytes()).unwrap();
    assert_eq!(
        parsed.output(),
        learning,
        "the subagent's text has to arrive"
    );
    assert_eq!(
        parsed.producer(),
        Some("Explore"),
        "the subagent's own name says more than the event that carried it"
    );

    // Both fixtures up front: `store` is shadowed by the first binding.
    let (temp, mut claude_store) = store();
    let (other_temp, mut opencode_store) = store();
    let claude = HookInput {
        cwd: temp.path().to_string_lossy().into_owned(),
        ..parsed
    };
    let outcome = run(&mut claude_store, HookEvent::SubagentStop, &claude).unwrap();
    assert_eq!(
        outcome.observations_captured, 1,
        "a Claude Code subagent's learnings are kept: {outcome:?}"
    );

    // And kept somewhere a search can reach. `passive_capture` filed everything
    // under the type `passive`, which is not one of the seven words the skill
    // teaches — so no agent ever asks for it and a typed search cannot return
    // one. That went unnoticed while capture was broken and produced nothing.
    // What the memory *is* is a discovery; where it came from is `tool_name`.
    let found = claude_store
        .search(
            "retry budget",
            crate::memory::model::SearchOptions {
                kind: Some("discovery".to_owned()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        found.len(),
        1,
        "a captured learning has to be reachable by a type an agent asks for: {found:?}"
    );
    assert_eq!(
        found[0].observation.tool_name.as_deref(),
        Some("Explore"),
        "and it still says which subagent produced it"
    );

    // The same text under OpenCode's name still works — the alias is an
    // addition, not a replacement.
    let opencode = HookInput {
        session_id: "agent-session".to_owned(),
        cwd: other_temp.path().to_string_lossy().into_owned(),
        stdout: learning.to_owned(),
        ..HookInput::default()
    };

    // Codex sends `source` and `agent_type` in the same payload. Carried as one
    // field with a `serde` alias, that is a duplicate field and serde rejects
    // the whole document — so every Codex hook parsed as an empty `HookInput`,
    // with no session id, no prompt and no capture, and said nothing, because
    // `read_input` falls back to defaults rather than blocking a prompt.
    let both = format!(
        r#"{{"session_id":"codex-session","cwd":"/tmp/x","hook_event_name":"SubagentStop",
            "source":"subagent","agent_id":"a1","agent_type":"Explore",
            "turn_id":"t1","stop_hook_active":false,
            "last_assistant_message":{}}}"#,
        serde_json::to_string(learning).unwrap()
    );
    let codex = read_input(both.as_bytes()).expect("a payload with both names still parses");
    assert_eq!(
        codex.session_id, "codex-session",
        "a rejected payload loses the session id and everything with it"
    );
    assert_eq!(codex.output(), learning);
    assert_eq!(
        codex.producer(),
        Some("Explore"),
        "the subagent wins over the event"
    );
    assert_eq!(
        run(&mut opencode_store, HookEvent::SubagentStop, &opencode)
            .unwrap()
            .observations_captured,
        1
    );
}

#[test]
fn an_agent_is_told_which_language_to_remember_in() {
    // Memories are written by an agent, and an agent left to itself writes
    // English whatever it was asked in — a real store held about 90% English
    // notes against 59% Spanish questions. That was treated as a fact for the
    // search to work around, and it is a defect: a memory in a language its
    // reader did not use is harder to find *and* harder to read, and no
    // cleverness downstream undoes either.
    //
    // It cannot live in the skill, which ships identical to everybody. It is
    // said every session, from the settings file as it is now.
    let (temp, mut store) = store();
    let data_dir = store.database_path().parent().unwrap().to_path_buf();

    // Unset: the language of the conversation, not English by default.
    let opened = run(&mut store, HookEvent::SessionStart, &input(temp.path())).unwrap();
    let context = opened
        .additional_context
        .expect("a session opens with context");
    assert!(
        context.contains("the language the user is writing in"),
        "{context}"
    );

    // Pinned: that language, whatever the conversation is in.
    crate::settings::save(
        &data_dir,
        &crate::settings::Settings {
            language: Some("español".to_owned()),
            ..Default::default()
        },
    )
    .unwrap();
    let pinned = run(&mut store, HookEvent::SessionStart, &input(temp.path()))
        .unwrap()
        .additional_context
        .expect("a session opens with context");
    assert!(
        pinned.contains("Write and search memories in español"),
        "a pinned language governs the search as well as the save, or the agent is told to write \
         in one language and look in another: {pinned}"
    );

    // And after a compaction, which is exactly when the instruction is gone.
    let recovered = run(&mut store, HookEvent::PostCompaction, &input(temp.path()))
        .unwrap()
        .additional_context
        .expect("compaction rebuilds context");
    assert!(recovered.contains("español"), "{recovered}");
}

#[test]
fn an_agent_that_names_no_project_falls_back_to_the_detected_one() {
    // An agent may send `project` and leave it empty — a template filled in
    // with nothing. Taken literally that files the memory under no project at
    // all, which is the one bucket that never appears in a project listing and
    // that every project-scoped read skips. The memory is saved, and it is
    // saved where nobody will look for it.
    let detection = crate::project::ProjectDetection {
        project: "leteo".to_owned(),
        source: crate::project::SOURCE_GIT_ROOT.to_owned(),
        path: "H:/REPO/leteo".to_owned(),
        available_projects: Vec::new(),
        warning: None,
        error_hint: None,
    };
    for empty in [Some(String::new()), Some("   ".to_owned()), None] {
        let input = HookInput {
            project: empty.clone(),
            ..HookInput::default()
        };
        assert_eq!(
            resolve_project(&input, &detection),
            "leteo",
            "{empty:?} must not become the project"
        );
    }

    // And a real one from the agent still wins, because it knows things the
    // directory does not.
    let input = HookInput {
        project: Some("  Chosen-Project  ".to_owned()),
        ..HookInput::default()
    };
    assert_eq!(resolve_project(&input, &detection), "chosen-project");
}

#[test]
fn a_renamed_project_is_recognised_however_the_path_was_spelled() {
    // The directory a session recorded and the one a hook is running in are
    // the same place written twice, and Windows lets them differ in case and
    // separator. Compared literally they look like two directories, the
    // rename is not recognised, and the memories stay under the old project
    // name — silently, because nothing failed.
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("legacy-name");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut store = Store::open(StoreConfig::new(temp.path().join("hooks.db"))).unwrap();

    // Recorded the way another machine, or another Windows API, might have
    // written it: backslashes, a trailing one, and — on Windows — a different
    // case.
    //
    // The case fold is conditional because it is a property of the filesystem
    // rather than of the spelling. `same_directory` folds case only under
    // `cfg!(windows)`, and it is right to: on macOS and Linux `/tmp/Foo` and
    // `/tmp/foo` can be two different directories, so treating them as one
    // would merge the memories of one project into another. Uppercasing here
    // unconditionally asked those platforms to call two directories the same,
    // and the first CI run on macOS said no — correctly.
    let recorded = {
        let spelled = workspace.to_string_lossy().replace('/', "\\") + "\\";
        if cfg!(windows) {
            spelled.to_uppercase()
        } else {
            spelled
        }
    };
    store
        .create_session("old", "legacy-name", &recorded)
        .unwrap();
    store
        .add_observation(AddObservation {
            session_id: "old".to_owned(),
            kind: "decision".to_owned(),
            title: "Legacy memory".to_owned(),
            content: "saved before the project was renamed".to_owned(),
            tool_name: None,
            project: Some("legacy-name".to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            prompt_sync_id: None,
        })
        .unwrap();

    let mut outcome = HookOutcome::default();
    migrate_directory_project(&mut store, &workspace, "renamed", &mut outcome);

    assert_eq!(
        store
            .recent_observations(Some("renamed"), Some(10), true)
            .unwrap()
            .len(),
        1,
        "the memory did not follow the rename: {outcome:?}"
    );
}

#[test]
fn the_reminder_waits_for_the_session_to_start_and_for_the_saving_to_stop() {
    // Two conditions hold the reminder back and each covers a case the other
    // does not — which is why neither was noticed missing: disable one and the
    // other still keeps quiet in the ordinary fixture.
    //
    // The warm-up matters exactly when the cool-down has already passed: a new
    // session on a project last saved to yesterday. The quiet is real and the
    // session is a minute old, so without the warm-up the very first prompt of
    // the day is answered with "you should be saving".
    //
    // The cool-down matters the other way round: an old session where
    // something was saved a moment ago.
    let (temp, mut store) = store();
    let payload = input(temp.path());
    run(&mut store, HookEvent::SessionStart, &payload).unwrap();
    store
        .add_observation(AddObservation {
            session_id: payload.session_id.clone(),
            kind: "decision".to_owned(),
            title: "Saved a while back".to_owned(),
            content: "body".to_owned(),
            tool_name: None,
            project: Some("hook-project".to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            prompt_sync_id: None,
        })
        .unwrap();

    let database = store.database_path().to_path_buf();
    let age = |sessions: &str, observations: &str| {
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute(
                &format!("UPDATE sessions SET started_at = datetime('now', '{sessions}')"),
                [],
            )
            .unwrap();
        connection
            .execute(
                &format!("UPDATE observations SET created_at = datetime('now', '{observations}')"),
                [],
            )
            .unwrap();
    };
    let nudged = |store: &mut Store| {
        // The debounce file would answer for the second call onwards, so it
        // goes between the two questions rather than deciding them.
        if let Some(path) = nudge_state_path(store, &payload.session_id) {
            let _ = std::fs::remove_file(path);
        }
        run(store, HookEvent::UserPromptSubmit, &payload)
            .unwrap()
            .system_message
            .is_some()
    };

    // A minute-old session, quiet since yesterday: the warm-up is the only
    // thing between the person and a reminder they have not earned.
    age("-1 minutes", "-1 day");
    assert!(
        !nudged(&mut store),
        "a session that just opened is not nudged"
    );

    // An old session, saved into a moment ago: now the cool-down is.
    age("-2 hours", "-1 minutes");
    assert!(!nudged(&mut store), "somebody who just saved is not nudged");

    // Old enough and quiet enough, and it speaks.
    age("-2 hours", "-1 day");
    assert!(nudged(&mut store), "a genuinely quiet project is nudged");
}

/// The opening block says what each recent session was for, however long ago
/// that session wrote it down.
///
/// The fold used to be handed whatever summaries fell inside the window of
/// recently-saved memories, so a session that kept working after writing its
/// summary lost it from both lists at once: the sessions list showed a name
/// and a date, and the memory list had already set the summary aside. Measured
/// on a real store at the default budget, that emptied 3 of the 19 recent
/// sessions that had a summary to show.
#[test]
fn a_session_that_kept_working_after_its_summary_still_says_what_it_was_for() {
    let (temp, mut store) = store();
    // The measured budget, said out loud. How far back the old window reached
    // was `budget * 4`, so the size of the opening block decided whether a
    // summary was still in it — which is exactly the accident being removed,
    // and a test that left the size to the default would be asserting on
    // whichever one that happens to be.
    crate::settings::save(
        temp.path(),
        &crate::settings::Settings {
            interface: Some(crate::settings::Interface::English),
            context_size: Some(crate::settings::ContextSize::Slim),
            ..crate::settings::Settings::default()
        },
    )
    .unwrap();
    run(&mut store, HookEvent::SessionStart, &input(temp.path())).unwrap();
    let mut save = |kind: &str, title: &str, content: &str| {
        store
            .add_observation(AddObservation {
                session_id: "agent-session".to_owned(),
                kind: kind.to_owned(),
                title: title.to_owned(),
                content: content.to_owned(),
                tool_name: None,
                project: Some("hook-project".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
    };
    save(
        "session_summary",
        "Session summary: hook-project",
        "## Goal\nTeach the fold to look by session\n",
    );
    // Everything saved afterwards, which is what used to push it out of reach.
    for index in 0..90 {
        save("decision", &format!("Later memory {index}"), "a body");
    }

    let outcome = run(&mut store, HookEvent::SessionStart, &input(temp.path())).unwrap();
    let context = outcome
        .additional_context
        .expect("a session opening carries context");
    assert!(
        context.contains("Teach the fold to look by session"),
        "the session has to say what it was for: {context}"
    );
    assert!(
        !context.contains("Session summary: hook-project"),
        "and it says it beside the session, not as one of the memories: {context}"
    );
}

/// The hint prints the title of a second memory, and it is a title like any
/// other.
///
/// A caveat names the memory that overturned the one being suggested — its id
/// and its title — and that title was the last one printed raw anywhere an
/// agent reads, and the only one with no length on it either. One long title
/// made one long line in a hint that is meant to be three short ones, and a
/// title with a newline in it would have ended the line the way the other two
/// did.
#[test]
fn a_caveat_in_the_hint_prints_one_short_line() {
    let long = format!("Una decisión posterior {}", "y más palabras ".repeat(30));
    let folded = crate::recall::one_line_title(&format!("Primera línea\n- #999 {long}"));
    assert!(!folded.contains('\n'), "{folded:?}");
    assert!(
        folded.chars().count() <= 143,
        "cut like every other title: {} characters",
        folded.chars().count()
    );
    assert!(
        folded.starts_with("Primera línea - #999"),
        "folded rather than cut at the break: {folded:?}"
    );
}

/// A hook that meets a locked store answers within its own budget.
///
/// Leteo has to give up before the agent does. Two clocks were spending the
/// same seconds twice — `prepare` waits out another process converting the
/// journal, and every statement after it waits again — so a hook told it had
/// two seconds took four and a half, which is past the three its agent allows
/// before killing it. A killed hook tells nobody anything; one that answers
/// carries a warning saying what it could not do.
#[test]
fn a_locked_store_costs_a_hook_a_warning_rather_than_its_whole_budget() {
    let (temp, mut store) = store();
    run(&mut store, HookEvent::SessionStart, &input(temp.path())).unwrap();
    let database = store.database_path().to_path_buf();
    drop(store);

    // Another writer, holding the lock for longer than any hook may wait.
    let holder = rusqlite::Connection::open(&database).unwrap();
    holder
        .busy_timeout(std::time::Duration::from_secs(30))
        .unwrap();
    holder
        .execute_batch("BEGIN IMMEDIATE; UPDATE sessions SET directory = 'held';")
        .unwrap();

    let budget = HookEvent::SessionStop.store_wait();
    let mut store = Store::open(StoreConfig {
        busy_timeout: budget,
        ..StoreConfig::new(&database)
    })
    .unwrap();
    let outcome = run(&mut store, HookEvent::SessionStop, &input(temp.path())).unwrap();

    // The defect this test exists for, asked without a clock in the question.
    //
    // The budget belongs to the *process*, and the two things that wait — the
    // schema pass and the write — were each taking the whole of it, so one hook
    // paid for two. Timing the whole thing measures that only indirectly: once
    // and twice are one budget apart, and a loaded runner stalls by more than
    // that. This guard was written against three seconds and failed twice in
    // fourteen runs; rewritten against twice the budget, 3.6 s, it failed again
    // at 4.13 s on a machine that had cleared it at 2.04 s an hour before. Both
    // times the code was right and the guard was reporting the scheduler.
    //
    // What is left of the budget after opening is the same fact with the clock
    // taken out. Against a lock somebody else holds, the schema pass spends
    // time, so what remains must be *less* than what it started with — a stall
    // only makes that more true, while a second full budget makes it exactly
    // equal. See `Store::budget_left_after_opening`.
    assert!(
        store.budget_left_after_opening() < budget,
        "the statements after the schema pass got {:?} of a {budget:?} budget the pass \
         had already spent from",
        store.budget_left_after_opening()
    );
    // And that a hook finishes inside its agent's patience is asserted with no
    // clock at all, for all five events, by
    // `a_hook_gives_up_early_enough_that_the_overshoot_still_fits`.
    assert!(
        outcome
            .warnings
            .iter()
            .any(|warning| warning.ends_with(crate::store::StoreError::BUSY_ADVICE)),
        "and it says what it could not do, in words rather than in SQLite's: {:?}",
        outcome.warnings
    );
    holder.execute_batch("ROLLBACK").unwrap();
}

/// Every hook finishes inside its agent's patience even when the wait overruns.
///
/// The ladder only works if Leteo gives up first: a hook that is killed
/// mid-wait leaves the session unended and tells nobody why, which is the
/// failure `store_wait` exists to avoid. The margin was a flat second, and a
/// second is not what a wait of nine seconds leaves over — SQLite's busy
/// handler sleeps in steps of up to 100 ms and Windows rounds every one of them
/// up, so the overshoot is proportional. Measured on this machine at 10.3% for
/// a nine-second wait and 13.2% for a two-second one.
///
/// So the property is asserted against the worst overshoot seen rather than
/// against the nominal wait, plus room for starting the process and printing
/// the answer.
#[test]
fn a_hook_gives_up_early_enough_that_the_overshoot_still_fits() {
    const WORST_OVERSHOOT: f64 = 1.15;
    const STARTING_AND_ANSWERING: std::time::Duration = std::time::Duration::from_millis(250);

    for event in [
        HookEvent::SessionStart,
        HookEvent::PostCompaction,
        HookEvent::UserPromptSubmit,
        HookEvent::SubagentStop,
        HookEvent::SessionStop,
    ] {
        let patience = std::time::Duration::from_secs(event.agent_timeout_seconds());
        let worst = event.store_wait().mul_f64(WORST_OVERSHOOT) + STARTING_AND_ANSWERING;
        assert!(
            worst < patience,
            "{event:?}: waiting {:?} can take {worst:?} against {patience:?} of patience",
            event.store_wait()
        );
        // And it still waits for most of what it is allowed, or the ladder
        // would be trading a killed hook for one that gives up at once.
        assert!(
            event.store_wait() * 2 > patience,
            "{event:?}: {:?} gives up far too early against {patience:?}",
            event.store_wait()
        );
    }
}

/// The hint does not hand the same conversation the same memory twice.
///
/// A conversation stays about the same thing for a while, so the same memories
/// keep winning: replayed over six real sessions in five projects, 134 of the
/// 273 memories the hint named had already been named in that same session.
/// The agent is holding them, so a repeat spends the room a new one would have
/// taken and teaches whoever reads the hint that it repeats itself.
///
/// Three rules, and the second is the one worth the machinery: a memory
/// something has been said against is named again anyway, because it can be
/// overturned after it was handed over and silence would leave the agent with
/// the version it got before anybody objected. The third resets the list on
/// compaction, where the agent's context — and everything the hint put in it —
/// is gone.
#[test]
fn the_hint_does_not_hand_the_same_conversation_the_same_memory_twice() {
    let (temp, mut store) = store();
    let payload = input(temp.path());
    run(&mut store, HookEvent::SessionStart, &payload).unwrap();

    for index in 0..14 {
        store
            .add_observation(memory(
                &format!("Engine note {index}"),
                "A passing remark about the rendering engine and nothing else",
            ))
            .unwrap();
    }
    let answer = store
        .add_observation(memory(
            "We chose SQLite as the storage engine",
            "The storage engine question was settled: SQLite, chosen for being local-first and a \
             single file",
        ))
        .unwrap()
        .observation;

    let mut prompt = payload.clone();
    prompt.prompt = "Which storage engine did we choose?".to_owned();

    let first = run(&mut store, HookEvent::UserPromptSubmit, &prompt)
        .unwrap()
        .additional_context
        .expect("the first time, the hint speaks");
    assert!(first.contains(&format!("- #{}", answer.id)), "{first}");

    // The same question again, in the same conversation.
    let second = run(&mut store, HookEvent::UserPromptSubmit, &prompt)
        .unwrap()
        .additional_context;
    assert!(
        second.is_none(),
        "the agent already has it: {}",
        second.unwrap_or_default()
    );

    // Unless something is said against it in the meantime.
    let newer = store
        .add_observation(memory(
            "We moved the storage engine to Postgres",
            "The storage engine decision was revisited and changed",
        ))
        .unwrap()
        .observation;
    let relation = store
        .save_relation(crate::memory::model::SaveRelationParams {
            sync_id: crate::memory::normalize::sync_id("rel"),
            source_id: newer.sync_id.clone(),
            target_id: answer.sync_id.clone(),
        })
        .unwrap();
    store
        .judge_relation(crate::memory::model::JudgeRelationParams {
            judgment_id: relation.sync_id,
            relation: crate::store::RELATION_SUPERSEDES.to_owned(),
            marked_by_actor: "agent".to_owned(),
            marked_by_kind: "agent".to_owned(),
            ..Default::default()
        })
        .unwrap();

    let overturned = run(&mut store, HookEvent::UserPromptSubmit, &prompt)
        .unwrap()
        .additional_context
        .expect("a memory with something said against it is named again");
    assert!(
        overturned.contains(&format!("(superseded by #{}", newer.id)),
        "and the reason it is named again is the caveat: {overturned}"
    );

    // Compaction takes the agent's context away, so the list starts over.
    let mut settled = payload.clone();
    settled.prompt = String::new();
    run(&mut store, HookEvent::PostCompaction, &settled).unwrap();
    store.delete_observation(newer.id, true).unwrap();

    let after = run(&mut store, HookEvent::UserPromptSubmit, &prompt)
        .unwrap()
        .additional_context
        .expect("after a compaction the hint may speak again");
    assert!(after.contains(&format!("- #{}", answer.id)), "{after}");
}

/// The block a session opens with is the third surface to answer with silence.
///
/// `mem_search` and `mem_context` were taught to say whether the words or the
/// directory emptied an answer. This is the same question asked earliest: the
/// opening block is what a session begins with, so an empty one is the first
/// thing an agent learns about the store — and a directory that resolved to a
/// project nobody has saved under reads exactly like a fresh install.
#[test]
fn an_opening_block_with_nothing_in_it_says_whether_the_store_is_empty_too() {
    let (temp, mut store) = store();
    // A memory, filed somewhere this directory is not.
    store
        .create_session("s1", "otro-proyecto", "C:/otro")
        .unwrap();
    store
        .add_observation(crate::memory::model::AddObservation {
            session_id: "s1".to_owned(),
            kind: "decision".to_owned(),
            title: "Vive en otro proyecto".to_owned(),
            content: "y este directorio no es ese".to_owned(),
            tool_name: None,
            project: Some("otro-proyecto".to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            prompt_sync_id: None,
        })
        .unwrap();

    let outcome = run(&mut store, HookEvent::SessionStart, &input(temp.path())).unwrap();
    let block = outcome.additional_context.unwrap_or_default();
    assert!(
        block.contains("elsewhere"),
        "an empty block has to say the store is not what is empty: {block:?}"
    );
    assert!(block.contains("--all-projects"), "{block:?}");
}

/// A busy store reads as busy, not as a broken one.
///
/// The tool surface has answered `store_busy` with a next step since it was
/// added; the hooks printed whatever SQLite said. Somebody who typed two
/// prompts at once read `create session: database error: database is locked`
/// in their terminal, which is the prose of a corrupt file about a store that
/// was merely in use — and the hooks are the surface a person actually sees,
/// because their warnings go to stderr in front of them.
#[test]
fn a_hook_warning_says_a_busy_store_can_be_tried_again() {
    let (temp, mut store) = store();
    run(&mut store, HookEvent::SessionStart, &input(temp.path())).unwrap();
    let database = store.database_path().to_path_buf();
    drop(store);

    // Another writer, holding the lock for longer than any hook may wait.
    let holder = rusqlite::Connection::open(&database).unwrap();
    holder
        .busy_timeout(std::time::Duration::from_secs(30))
        .unwrap();
    holder
        .execute_batch("BEGIN IMMEDIATE; UPDATE sessions SET directory = 'held';")
        .unwrap();

    let mut store = Store::open(StoreConfig {
        busy_timeout: HookEvent::UserPromptSubmit.store_wait(),
        ..StoreConfig::new(&database)
    })
    .unwrap();
    let asking = HookInput {
        prompt: "una pregunta que hay que guardar".to_owned(),
        ..input(temp.path())
    };
    let outcome = run(&mut store, HookEvent::UserPromptSubmit, &asking).unwrap();
    assert!(!outcome.warnings.is_empty(), "a held store has to warn");
    for warning in &outcome.warnings {
        assert!(
            warning.ends_with(crate::store::StoreError::BUSY_ADVICE),
            "a warning about a store in use must carry the one sentence the three surfaces share: \
             {warning:?}"
        );
        assert!(
            !warning.contains("database is locked"),
            "and must not hand SQLite prose to a person: {warning:?}"
        );
    }
    holder.execute_batch("ROLLBACK").unwrap();
}

/// The three surfaces say the same thing about a store in use.
///
/// Each said something different and each was found separately: the tools named
/// `store_busy` and a next step, the hooks printed whatever SQLite said, and the
/// command line handed a person `Error code 5: database is locked` three times
/// over with a cause chain. The fact behind all three is one fact, so the
/// sentence is one sentence, and this holds them to it.
#[test]
fn every_surface_says_a_busy_store_the_same_way() {
    let advice = crate::store::StoreError::BUSY_ADVICE;
    assert!(
        advice.contains("again"),
        "the whole point is that it can be retried: {advice}"
    );
    assert!(
        !advice.contains("locked") && !advice.contains("Error code"),
        "a person is not reading SQLite: {advice}"
    );
    assert!(
        !advice.contains("  "),
        "source indentation escaped: {advice:?}"
    );

    // The hook wording is that sentence, not another one beside it.
    let error = crate::store::StoreError::Database(rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(5),
        Some("database is locked".to_owned()),
    ));
    assert!(error.is_busy(), "code 5 is the busy one");
    let said = crate::hooks::said("create session", &error);
    assert!(said.starts_with("create session: "), "{said}");
    assert!(
        said.ends_with(advice),
        "the hook said something of its own: {said}"
    );

    // And an error that is not busy keeps its own words, or this would swallow
    // the failures somebody needs the detail of.
    let broken = crate::store::StoreError::ObservationNotFound(7);
    assert!(!broken.is_busy());
    assert!(!crate::hooks::said("read", &broken).contains("try again"));
}

/// A session opening says when memories have come round for a reread.
///
/// The review dates were computed, migrated and indexed, and nothing ever said
/// the queue existed. `mem_review` reads it; the skill listed that tool among
/// the nineteen without ever saying when to reach for it; no hook named it; the
/// command line has no equivalent. On a real store 269 memories carry a review
/// date and the first falls due in 34 days — at which point nothing would have
/// mentioned it. A window nothing opens is the same defect `policy` had when
/// its own window could never fire.
///
/// Said once, as a session opens, for the reason the pending verdicts are:
/// every prompt would nag, and never is where this was.
#[test]
fn a_session_opening_says_which_memories_have_come_round() {
    let (temp, mut store) = store();
    store
        .create_session(
            "agent-session",
            "hook-project",
            &temp.path().to_string_lossy(),
        )
        .unwrap();
    let saved = store
        .add_observation(AddObservation {
            session_id: "agent-session".to_owned(),
            kind: "decision".to_owned(),
            title: "A decision with a clock on it".to_owned(),
            content: "and a window that comes round".to_owned(),
            tool_name: None,
            project: Some("hook-project".to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            prompt_sync_id: None,
        })
        .unwrap()
        .observation;
    assert!(
        store
            .get_observation(saved.id)
            .unwrap()
            .review_after
            .is_some(),
        "a decision is saved with a date to look at it again"
    );

    // Nothing is due yet, and a zero is silence rather than a line saying so.
    assert_eq!(store.count_review_due(Some("hook-project")).unwrap(), 0);
    let quiet = start_session(&mut store, temp.path());
    assert!(
        !quiet.contains("mem_review"),
        "a store with nothing due says nothing about the queue: {quiet}"
    );

    // Due, by moving the clock rather than by waiting six months.
    store
        .connection()
        .execute(
            "UPDATE observations SET review_after = datetime('now', '-1 day') WHERE id = ?1",
            rusqlite::params![saved.id],
        )
        .unwrap();
    assert_eq!(store.count_review_due(Some("hook-project")).unwrap(), 1);
    let said = start_session(&mut store, temp.path());
    assert!(
        said.contains("mem_review"),
        "a memory that has come round is named at the opening, with the tool that reads it: {said}"
    );

    // And the count is the store's own, not a number the line invented: the
    // list `mem_review` would hand over is the same length. Two of them, so a
    // singular line cannot pass this by accident.
    store
        .add_observation(AddObservation {
            session_id: "agent-session".to_owned(),
            kind: "policy".to_owned(),
            title: "A second one, with a longer window".to_owned(),
            content: "so the plural is exercised".to_owned(),
            tool_name: None,
            project: Some("hook-project".to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            prompt_sync_id: None,
        })
        .unwrap();
    store
        .connection()
        .execute(
            "UPDATE observations SET review_after = datetime('now', '-1 day')
             WHERE project = 'hook-project' AND deleted_at IS NULL",
            [],
        )
        .unwrap();
    let counted = store.count_review_due(Some("hook-project")).unwrap();
    let listed = store
        .review_due(Some("hook-project"), Some(1_000))
        .unwrap()
        .len() as i64;
    assert_eq!(
        counted, listed,
        "the number said and the queue handed over are one question"
    );
    assert!(counted > 1, "with more than one, so a singular cannot pass");
    assert!(
        start_session(&mut store, temp.path()).contains(&counted.to_string()),
        "and it is the number that is said"
    );
}

fn start_session(store: &mut Store, directory: &Path) -> String {
    run(store, HookEvent::SessionStart, &input(directory))
        .unwrap()
        .system_message
        .unwrap_or_default()
}

/// A subagent's learnings lost to a busy store are the one silence worth breaking.
///
/// Every hook that loses to another writer says so — on stderr, in the outcome's
/// warnings, where `--verbose` shows it. The agent gets `{}`. That is right
/// almost everywhere: a prompt that was not recorded or a session that was not
/// closed is not something the agent can do anything about, and a workflow
/// finishing dozens of subagents cannot afford a line each.
///
/// `subagent-stop` is the exception, and it is the one the spec already calls
/// out: the learnings live in the text this hook was handed and nowhere else,
/// because the subagent finishes and its context is discarded. So the agent
/// reading this still holds the only copy, and it is the one case where saying
/// so buys something — a sentence that says what to do rather than what
/// happened.
#[test]
fn a_capture_lost_to_a_busy_store_tells_the_agent_what_to_do() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("busy.db");
    crate::settings::save(
        temp.path(),
        &crate::settings::Settings {
            interface: Some(crate::settings::Interface::English),
            ..crate::settings::Settings::default()
        },
    )
    .unwrap();
    let mut store = Store::open(StoreConfig {
        // Short, because this test is about what is said and not about how long
        // it is waited for; the waiting has its own guard.
        busy_timeout: std::time::Duration::from_millis(50),
        ..StoreConfig::new(path.clone())
    })
    .unwrap();
    store
        .create_session("agent-session", "hook-project", "C:/repo")
        .unwrap();

    let learned = HookInput {
        session_id: "agent-session".to_owned(),
        cwd: "C:/repo".to_owned(),
        project: Some("hook-project".to_owned()),
        last_assistant_message: "## Key Learnings:
1. Something worth keeping from a subagent"
            .to_owned(),
        ..HookInput::default()
    };

    // With nobody else writing, the ordinary line: what was kept.
    let kept = run(&mut store, HookEvent::SubagentStop, &learned)
        .unwrap()
        .system_message
        .unwrap_or_default();
    assert!(kept.contains("Sardi"), "the ordinary report: {kept}");

    // And with the write lock held by somebody else.
    // A different learning, because the same one is deduplicated before any
    // write is attempted — the capture would come back with nothing saved and
    // no error, and this test would be about the dedupe.
    let learned_again = HookInput {
        last_assistant_message: "## Key Learnings:
1. Something else entirely, from later on"
            .to_owned(),
        ..learned.clone()
    };
    let blocker = rusqlite::Connection::open(&path).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let refused = run(&mut store, HookEvent::SubagentStop, &learned_again).unwrap();
    assert_eq!(refused.observations_captured, 0, "{refused:?}");
    let said = refused.system_message.clone().unwrap_or_default();
    assert!(
        said.contains("mem_capture_passive"),
        "the agent still holds the only copy, so it is told to send it: {said:?}"
    );
    assert!(
        refused
            .response()
            .get("systemMessage")
            .is_some_and(|value| value.as_str() == Some(said.as_str())),
        "and on the channel the agent reads, not only in the warnings"
    );

    blocker.execute_batch("ROLLBACK").unwrap();

    // A refusal a retry would not mend loses the learnings just as completely,
    // so it is said too — with its own cause, and without sending the agent to
    // make the identical write fail a second time.
    store
        .connection()
        .execute_batch("PRAGMA query_only = ON")
        .unwrap();
    let stuck = run(&mut store, HookEvent::SubagentStop, &learned_again).unwrap();
    let said = stuck.system_message.clone().unwrap_or_default();
    assert!(
        said.contains("could not keep this subagent's learnings"),
        "the loss is named whatever refused the write: {said:?}"
    );
    assert!(
        said.contains("readonly") || said.contains("read-only"),
        "with the cause it can act on: {said:?}"
    );
    assert!(
        !said.contains("mem_capture_passive"),
        "and not sent to repeat a write that would fail the same way: {said:?}"
    );
    store
        .connection()
        .execute_batch("PRAGMA query_only = OFF")
        .unwrap();

    // The events that lose nothing the agent could put back stay quiet, which
    // is what keeps this one worth reading.
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let asked = HookInput {
        prompt: "una pregunta cualquiera".to_owned(),
        ..learned.clone()
    };
    let quiet = run(&mut store, HookEvent::UserPromptSubmit, &asked).unwrap();
    assert!(
        !quiet.warnings.is_empty(),
        "the prompt was not saved, and the outcome says so: {quiet:?}"
    );
    assert!(
        quiet.response().get("systemMessage").is_none(),
        "and the agent is not told about a prompt it cannot re-send: {:?}",
        quiet.response()
    );
}

/// A payload that parsed and said nothing is not the same as no payload.
///
/// Every field of `HookInput` defaults and there is no `deny_unknown_fields`,
/// which is deliberate: a client adds a field to its own schema and a hook that
/// refused it would break for a change that was none of its business. What that
/// costs is that a payload whose fields are *named* differently parses
/// perfectly into an empty input, and every hook then reports success having
/// done nothing — sessions nobody can find, prompts never saved, and not a word
/// anywhere. That is the shape a `serde` alias once gave Codex's ordinary
/// payload, and the warning written afterwards catches a body that is not JSON,
/// which is the half that announces itself.
#[test]
fn a_payload_that_parsed_into_nothing_says_so() {
    // The ordinary one, and the same one with fields a client added: both are
    // read, and neither complains.
    for body in [
        r#"{"session_id":"s1","cwd":"C:/repo","prompt":"una pregunta"}"#,
        r#"{"session_id":"s1","cwd":"C:/repo","prompt":"una pregunta",
            "hook_event_name":"UserPromptSubmit","transcript_path":"C:/x.jsonl"}"#,
    ] {
        let read = read_input(body.as_bytes()).expect("this one is readable");
        assert_eq!(read.prompt, "una pregunta", "{body}");
        assert_eq!(read.session_id, "s1");
    }

    // The same payload in another spelling: valid JSON, nothing Leteo reads.
    let renamed = read_input(
        r#"{"sessionId":"s1","workingDirectory":"C:/repo","userPrompt":"una pregunta"}"#.as_bytes(),
    );
    let said = format!("{:#}", renamed.expect_err("this one is not readable"));
    assert!(said.contains("carried nothing Leteo reads"), "{said}");
    // And it names where to look, because the caller's next question is which
    // field it should have been.
    assert!(
        said.contains("session_id") && said.contains("prompt"),
        "{said}"
    );

    // An empty payload is not that: it said nothing and there is nothing to
    // report. Every event is driven this way by clients that carry no fields
    // for it, and a warning on each would be noise on the ordinary path.
    assert_eq!(
        read_input("{}".as_bytes()).expect("an empty object is a payload"),
        HookInput::default()
    );
    assert_eq!(
        read_input("".as_bytes()).expect("and so is nothing at all"),
        HookInput::default()
    );

    // A payload carrying only the fields this reads, empty, is the one case
    // where the sentence is arguable — and it still holds, because what Leteo
    // got from it is what it gets from nothing.
    let empty_prompt = read_input(r#"{"prompt":""}"#.as_bytes());
    assert!(empty_prompt.is_err(), "{empty_prompt:?}");
}

/// A subagent that files a list instead of leaving learnings is cut off, and
/// told how much was left.
///
/// There was no bound. Every item of a numbered list became a memory, one row
/// and three full-text triggers each, inside a hook the agent kills after ten
/// seconds. Against a copy of a real store of 4,121 memories, two thousand
/// items cost 4,226 ms, so somewhere past four and a half thousand the hook is
/// killed part way through having written some unknown number of them — each
/// insert is its own transaction, so there is nothing to roll back and nothing
/// anywhere that says what happened.
///
/// The rest are counted rather than swallowed. The subagent's context is gone
/// but the agent reading this still has the text, which is the same reason a
/// capture the store refused says so.
#[test]
fn a_subagent_that_files_a_list_is_cut_off_and_says_how_much_was_left() {
    let (_temp, mut store) = store();
    let over = crate::memory::normalize::MAX_LEARNINGS + 3;
    let mut text = String::from("## Key Learnings\n\n");
    for index in 0..over {
        text.push_str(&format!(
            "{}. The pool number {index} caps at sixteen and waits for it\n",
            index + 1
        ));
    }

    let outcome = run(
        &mut store,
        HookEvent::SubagentStop,
        &HookInput {
            session_id: "s1".to_owned(),
            cwd: "C:/repo".to_owned(),
            // Named rather than detected, because what this test is about is
            // the cap on learnings and the sentence that reports it — the
            // directory is scenery. Left to detection, `C:/repo` is a Windows
            // path and nothing else: on macOS and Linux it does not exist, so
            // resolution falls through to the checkout the runner is standing
            // in, which is a git repository, and the memories landed under
            // *its* project instead of `repo`. The first CI run outside
            // Windows is what said so.
            project: Some("repo".to_owned()),
            last_assistant_message: text,
            ..HookInput::default()
        },
    )
    .unwrap();

    assert_eq!(outcome.observations_extracted, Some(over));
    assert_eq!(
        outcome.observations_captured,
        crate::memory::normalize::MAX_LEARNINGS
    );
    assert_eq!(outcome.observations_dropped, Some(3));
    let said = outcome.system_message.unwrap_or_default();
    assert!(
        said.contains(&over.to_string()) && said.contains("3 were not stored"),
        "the agent is told what was left, and it still has the text: {said}"
    );
    assert!(said.contains("mem_save"), "and what to do about it: {said}");
    assert_eq!(
        store.count_observations(Some("repo")).unwrap() as usize,
        crate::memory::normalize::MAX_LEARNINGS
    );
}

/// And a turn that leaves fewer says nothing about a bound it never met.
#[test]
fn a_capture_inside_the_bound_says_nothing_about_it() {
    let (_temp, mut store) = store();
    let outcome = run(
        &mut store,
        HookEvent::SubagentStop,
        &HookInput {
            session_id: "s1".to_owned(),
            cwd: "C:/repo".to_owned(),
            last_assistant_message: "## Key Learnings\n\n1. The pool caps at sixteen and waits\n\
                                     2. The deadline runs from the first attempt made\n"
                .to_owned(),
            ..HookInput::default()
        },
    )
    .unwrap();

    assert_eq!(outcome.observations_dropped, Some(0));
    assert_eq!(outcome.observations_captured, 2);
    let said = outcome.system_message.unwrap_or_default();
    assert!(
        !said.contains("not stored"),
        "nothing was left behind, so nothing says so: {said}"
    );
}

/// The capture ceiling fits inside what is left of the event's deadline.
///
/// The number itself is a judgement and a guard cannot check a judgement. What
/// it can check is the reason there is a number at all: this hook runs inside a
/// deadline the agent enforces by killing it, and `store_wait` has already
/// promised most of that deadline to waiting out another writer. What is left
/// is what the writing may cost.
///
/// This is the assertion the obvious one cannot make. The guard above sizes its
/// fixture from `MAX_LEARNINGS`, which is right — a fixture that does not
/// overrun the bound never reaches it — but it means the bound itself is
/// invisible to it: raised to a hundred thousand, that test writes a hundred
/// thousand memories and passes, in thirty-three minutes. This one fails in
/// microseconds and says why.
///
/// Three milliseconds a learning, from 2.1 measured against a copy of a real
/// store of 4,121 memories, rounded up: the rate is per machine and this is the
/// slow direction to be wrong in.
#[test]
fn the_capture_ceiling_fits_inside_the_deadline_the_agent_allows() {
    const MILLISECONDS_EACH: u128 = 3;
    let patience = std::time::Duration::from_secs(HookEvent::SubagentStop.agent_timeout_seconds());
    let left = patience
        .checked_sub(HookEvent::SubagentStop.store_wait())
        .expect("a hook gives up before the agent does");
    let cost = crate::memory::normalize::MAX_LEARNINGS as u128 * MILLISECONDS_EACH;
    assert!(
        cost <= left.as_millis(),
        "keeping {} learnings costs about {cost} ms and the event has {} ms left after the wait \
         it may spend on another writer, so a subagent that files a list gets this hook killed \
         part way through writing it",
        crate::memory::normalize::MAX_LEARNINGS,
        left.as_millis()
    );
}

/// A conversation as long as the longest one measured is never handed the same
/// memory twice.
///
/// The per-prompt hint promises exactly this — `hooks.md` §9 — and what makes
/// it true is a list of what this conversation has already been shown, written
/// to disk on every prompt. That list is capped, as it must be, and the cap was
/// 128: sized against sessions of 45 prompts, which was the longest this store
/// held when it was written. It now holds one of 351. Three memories a prompt
/// against that is a thousand ids, so the oldest fell off and the hint could
/// offer them again, in the same conversation, having promised not to.
///
/// The fixture is sized from the *reason* — the longest conversation, times
/// what one prompt may add — and not from `MAX_REMEMBERED`. That distinction is
/// the whole point: a fixture sized from the bound grows with it and can never
/// see it shrink, which is how the capture ceiling's own guard passed a bound
/// raised a thousandfold.
#[test]
fn a_whole_conversation_is_remembered_so_no_hint_is_offered_twice() {
    let handed =
        crate::hooks::context::RECALL_LIMIT * crate::hooks::nudge::LONGEST_CONVERSATION_PROMPTS;
    let mut state = SessionState::default();
    for prompt in 0..crate::hooks::nudge::LONGEST_CONVERSATION_PROMPTS {
        let first = (prompt * crate::hooks::context::RECALL_LIMIT) as i64;
        state.remember(first..first + crate::hooks::context::RECALL_LIMIT as i64);
    }

    assert_eq!(state.shown.len(), handed, "every one of them is distinct");
    assert!(
        state.shown.contains(&0),
        "the first memory of the conversation is still known to have been shown, or the hint \
         offers it again to somebody who has already read it"
    );

    // And it is still bounded: one more conversation's worth does fall off,
    // because a list written on every prompt cannot grow for ever.
    let beyond = handed as i64;
    state.remember(beyond..beyond + handed as i64);
    assert_eq!(state.shown.len(), handed);
    assert!(!state.shown.contains(&0));

    // What that costs, which is the reason there is a bound at all.
    let written = serde_json::to_string(&state).expect("the state serialises");
    assert!(
        written.len() < 16 * 1024,
        "the file is written on every prompt and is now {} bytes",
        written.len()
    );
}
