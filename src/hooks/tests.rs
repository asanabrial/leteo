use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tempfile::TempDir;

use super::*;
use crate::{
    memory::model::{AddObservation, PassiveCapture, SearchOptions},
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
            // Most of what the hooks are asked about here is a line Sardi says,
            // and the default voice no longer says those — an unset voice is the
            // reminder alone. The fixture is the loud Sardi so a test about a
            // greeting gets one; a test about what the default does writes its
            // own settings over this.
            voice: crate::settings::Voice::All,
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

/// A checkout whose origin names `name`, without spawning git.
///
/// Detection reads `.git/config` directly, so a test that only needs the name a
/// remote gives it does not need a working repository to say so.
fn git_remote(directory: &Path, name: &str) {
    let dot_git = directory.join(".git");
    std::fs::create_dir_all(&dot_git).unwrap();
    std::fs::write(
        dot_git.join("config"),
        format!("[remote \"origin\"]\n\turl = https://example.com/{name}.git\n"),
    )
    .unwrap();
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
            "UPDATE observations SET created_at = datetime('now', '-200 minutes'), \
                 updated_at = datetime('now', '-200 minutes')",
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
fn a_store_with_no_voice_setting_keeps_the_reminder_and_says_nothing_else() {
    // The default used to be `all`, which put a report line in front of the
    // person on every prompt — for everybody who never opened the settings. The
    // reminder is the one line that does work, so an unset voice keeps it and
    // drops the rest. The fixture pins `all` for the other tests here, so this
    // one builds its own store.
    use crate::settings::{self, Settings, Voice};

    let temp = TempDir::new().unwrap();
    let mut store = Store::open(StoreConfig::new(temp.path().join("default-voice.db"))).unwrap();
    // A file with no `voice` at all, which is what an older Leteo wrote and what
    // somebody hand-editing leaves behind. The field falls back to its default.
    std::fs::write(settings::path_in(temp.path()), r#"{"interface":"english"}"#).unwrap();

    run(&mut store, HookEvent::SessionStart, &input(temp.path())).unwrap();
    store
        .add_observation(memory("Something worth a greeting", "body"))
        .unwrap();

    // A session opening is a report, and the default voice does not report.
    let opened = run(&mut store, HookEvent::SessionStart, &input(temp.path())).unwrap();
    assert!(
        opened.system_message.is_none(),
        "an unset voice says nothing at a session opening: {:?}",
        opened.system_message
    );

    // Nor does a prompt that names a memory: the hint is a report too.
    let mut asking = input(temp.path());
    asking.prompt = "Something worth a greeting".to_owned();
    let plain = run(&mut store, HookEvent::UserPromptSubmit, &asking).unwrap();
    assert!(
        plain.system_message.is_none(),
        "an unset voice says nothing on a plain prompt: {:?}",
        plain.system_message
    );

    // The reminder is not a report and still arrives once the project has gone
    // quiet.
    let aged = rusqlite::Connection::open(store.database_path()).unwrap();
    aged.execute(
        "UPDATE sessions SET started_at = datetime('now', '-200 minutes')",
        [],
    )
    .unwrap();
    aged.execute(
        "UPDATE observations SET created_at = datetime('now', '-200 minutes'), \
                 updated_at = datetime('now', '-200 minutes')",
        [],
    )
    .unwrap();
    let _ = std::fs::remove_file(nudge_state_path(&store, "agent-session").unwrap());
    let prompted = run(&mut store, HookEvent::UserPromptSubmit, &input(temp.path())).unwrap();
    assert!(
        prompted.system_message.is_some(),
        "the reminder survives an unset voice"
    );
    assert_eq!(settings::load(temp.path()).voice, Voice::Reminders);

    // A file that names `all` keeps the reports, which is what a settings file
    // written before this change says.
    settings::save(
        temp.path(),
        &Settings {
            voice: Voice::All,
            ..Settings::default()
        },
    )
    .unwrap();
    let loud = run(&mut store, HookEvent::SessionStart, &input(temp.path())).unwrap();
    assert!(
        loud.system_message.is_some(),
        "a file that names `all` still gets the reports"
    );
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
            semantic_search: None,
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
    assert!(
        !recall.contains("mem_get_observation"),
        "the hint is a maybe and must not order a tool call: {recall}"
    );
    assert!(recall.contains("ignore any that do not fit"), "{recall}");
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

/// A store holding one memory the given words would surface, with the
/// conversation already started.
fn store_with_a_memory_to_surface() -> (TempDir, Store) {
    let (temp, mut store) = store();
    run(&mut store, HookEvent::SessionStart, &input(temp.path())).unwrap();
    // Beside a handful that only share its ordinary words, because the hint
    // speaks only for a memory that stands out among the rest.
    let titles = [(
        "Fixed the paginated closing verification",
        "pagination broke the closing verification step".to_owned(),
    )]
    .into_iter()
    .chain((1..=6).map(|n| {
        (
            "Unrelated change",
            format!("the step was changed again for reason {n}"),
        )
    }));
    for (title, content) in titles {
        store
            .add_observation(AddObservation {
                session_id: "agent-session".to_owned(),
                kind: "bugfix".to_owned(),
                title: title.to_owned(),
                content,
                tool_name: None,
                project: Some("hook-project".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
    }
    (temp, store)
}

fn stored_prompt_count(store: &Store) -> i64 {
    rusqlite::Connection::open(store.database_path())
        .unwrap()
        .query_row("SELECT COUNT(*) FROM prompts", [], |row| row.get(0))
        .unwrap()
}

/// 7,104 of 15,158 prompts on a real store began with `<task-notification>`,
/// and the hint spoke on 31% of 150 replayed ones. The list is read here so a
/// prefix added to it is covered without anybody writing a case for it.
#[test]
fn a_message_the_harness_wrote_is_neither_stored_nor_searched() {
    for prefix in normalize::HARNESS_MESSAGE_PREFIXES {
        let (temp, mut store) = store_with_a_memory_to_surface();
        let mut injected = input(temp.path());
        injected.prompt =
            format!("  {prefix} the paginated closing verification is broken </task>");

        let outcome = run(&mut store, HookEvent::UserPromptSubmit, &injected).unwrap();

        assert_eq!(stored_prompt_count(&store), 0, "{prefix} was stored");
        assert!(!outcome.prompt_saved, "{prefix} reported as saved");
        assert!(
            outcome.additional_context.is_none(),
            "{prefix} was searched for a hint: {:?}",
            outcome.additional_context
        );
    }
}

#[test]
fn a_pasted_prompt_is_stored_and_searched_like_any_typed_one() {
    let (temp, mut store) = store_with_a_memory_to_surface();
    let mut pasted = input(temp.path());
    pasted.prompt = "<pasted_content> the paginated closing verification is broken".to_owned();

    let outcome = run(&mut store, HookEvent::UserPromptSubmit, &pasted).unwrap();

    assert_eq!(stored_prompt_count(&store), 1);
    assert!(outcome.prompt_saved);
    assert!(
        outcome
            .additional_context
            .as_deref()
            .is_some_and(|context| context.contains("Fixed the paginated closing verification")),
        "{:?}",
        outcome.additional_context
    );
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

/// A store whose reads fail, and the connection holding it that way.
///
/// WAL lets a reader through a writer's lock, so the journal goes back to a
/// rollback one and the holder takes the exclusive lock that stops reads. The
/// store's wait is short so the test is about the answer, not the budget.
fn unreadable_store(database: &Path) -> (Store, rusqlite::Connection) {
    let store = Store::open(StoreConfig {
        busy_timeout: Duration::from_millis(50),
        ..StoreConfig::new(database)
    })
    .unwrap();
    store
        .connection()
        .execute_batch("PRAGMA journal_mode = DELETE")
        .unwrap();
    let holder = rusqlite::Connection::open(database).unwrap();
    holder.busy_timeout(Duration::from_secs(30)).unwrap();
    holder.execute_batch("BEGIN EXCLUSIVE").unwrap();
    (store, holder)
}

/// A store that cannot be read is not an empty project.
///
/// The post-compaction block answered a read failure with "No previous memory
/// found for this project." — the same sentence as a project nobody has saved
/// under. An agent told there is nothing proceeds as if starting fresh, when
/// the memories it needed may be there and the store simply could not answer.
#[test]
fn a_store_that_cannot_be_read_says_so_after_a_compaction() {
    let (temp, mut store) = store();
    run(&mut store, HookEvent::SessionStart, &input(temp.path())).unwrap();
    let database = store.database_path().to_path_buf();
    drop(store);

    let (mut store, holder) = unreadable_store(&database);
    let said = run(&mut store, HookEvent::PostCompaction, &input(temp.path()))
        .expect("a store that cannot be read is an answer, not a failed hook")
        .additional_context
        .unwrap();
    holder.execute_batch("ROLLBACK").unwrap();

    assert!(
        said.contains("could not read its store"),
        "a read failure has to say so: {said}"
    );
    assert!(
        !said.contains("No previous memory found"),
        "and must not be reported as an empty project: {said}"
    );
}

/// The session opening says it too, rather than leaving the section out.
///
/// A missing section reads exactly like a fresh install, and it is the first
/// thing an agent learns about the store.
#[test]
fn a_store_that_cannot_be_read_says_so_when_the_session_opens() {
    let (temp, store) = store();
    let database = store.database_path().to_path_buf();
    drop(store);

    let (mut store, holder) = unreadable_store(&database);
    let said = run(&mut store, HookEvent::SessionStart, &input(temp.path()))
        .expect("a store that cannot be read is an answer, not a failed hook")
        .additional_context
        .unwrap();
    holder.execute_batch("ROLLBACK").unwrap();

    assert!(
        said.contains("could not read its store"),
        "the opening has to say the store could not be read: {said}"
    );
}

/// Pi stores the compaction summary before it runs the hook, and was then told/// Pi stores the compaction summary before it runs the hook, and was then told
/// to persist it again. A host that says it already did gets a recovery that
/// leaves the instruction out and keeps everything else.
#[test]
fn post_compaction_does_not_ask_for_a_summary_the_host_already_stored() {
    let (temp, mut store) = store();
    run(&mut store, HookEvent::SessionStart, &input(temp.path())).unwrap();
    store
        .add_observation(memory(
            "Recovered decision",
            "Something the project already decided",
        ))
        .unwrap();

    let mut stored = input(temp.path());
    stored.summary_stored = true;
    let said = run(&mut store, HookEvent::PostCompaction, &stored)
        .unwrap()
        .additional_context
        .unwrap();
    assert!(said.starts_with("Context was compacted"), "{said}");
    assert!(!said.contains("mem_session_summary"), "{said}");
    assert!(said.contains("Recovered decision"), "{said}");

    let asked = run(&mut store, HookEvent::PostCompaction, &input(temp.path()))
        .unwrap()
        .additional_context
        .unwrap();
    assert!(asked.contains("mem_session_summary"), "{asked}");
}

#[test]
fn a_payload_without_summary_stored_leaves_it_off() {
    let parsed: HookInput = serde_json::from_str(r#"{"session_id":"s"}"#).unwrap();
    assert!(!parsed.summary_stored);
    let said: HookInput = serde_json::from_str(r#"{"summary_stored":true}"#).unwrap();
    assert!(said.summary_stored);
}

/// A host that carries the protocol itself asks for the opening without the
/// directive, and gets the rest of it unchanged.
///
/// OpenCode used to take its opening from `post-compaction`, so a session that
/// had just begun was told "Context was compacted" and to summarise it; Pi took
/// `session-start` whole and said the directive on top of its own protocol.
/// What is held here is that the opening is exactly the full one minus its
/// directive, so it cannot be a second builder drifting from the first, and that
/// the default answer — Claude Code's, byte for byte — is untouched.
#[test]
fn an_opening_without_the_directive_is_the_full_opening_minus_it() {
    let (temp, mut store) = store();
    let payload = input(temp.path());
    run(&mut store, HookEvent::SessionStart, &payload).unwrap();
    store
        .add_observation(memory(
            "Opening decision",
            "Something the project already decided",
        ))
        .unwrap();

    let full = run(&mut store, HookEvent::SessionStart, &payload)
        .unwrap()
        .additional_context
        .unwrap();
    let mut bare_payload = payload.clone();
    bare_payload.without_directive = true;
    let bare = run(&mut store, HookEvent::SessionStart, &bare_payload)
        .unwrap()
        .additional_context
        .unwrap();

    let directive = format!("{}\n\n", crate::setup::MEMORY_DIRECTIVE);
    assert!(
        full.starts_with(&directive),
        "the default opening still leads with the directive"
    );
    assert_eq!(full.strip_prefix(directive.as_str()), Some(bare.as_str()));
    assert!(bare.contains("Opening decision"), "{bare}");
    assert!(!bare.contains("Context was compacted"), "{bare}");
    assert!(!bare.contains("Three rules"), "{bare}");

    let recovered = run(&mut store, HookEvent::PostCompaction, &payload)
        .unwrap()
        .additional_context
        .unwrap();
    assert!(
        recovered.starts_with("Context was compacted."),
        "the real post-compaction path keeps its preamble: {recovered}"
    );
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

fn remember_from(store: &mut Store, session: &str, directory: &Path, project: &str, title: &str) {
    store
        .create_session(session, project, &directory.to_string_lossy())
        .unwrap();
    store
        .add_observation(AddObservation {
            session_id: session.to_owned(),
            kind: "decision".to_owned(),
            title: title.to_owned(),
            content: format!("{title}, recorded in {session}"),
            tool_name: None,
            project: Some(project.to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            prompt_sync_id: None,
        })
        .unwrap();
}

fn titles_under(store: &Store, project: &str) -> Vec<String> {
    let mut titles = store
        .recent_observations(Some(project), Some(10), true)
        .unwrap()
        .into_iter()
        .map(|observation| observation.title)
        .collect::<Vec<_>>();
    titles.sort();
    titles
}

#[test]
fn a_name_recorded_here_and_elsewhere_folds_only_what_was_recorded_here() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("api");
    let elsewhere = temp.path().join("other-checkout");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    git_remote(&workspace, "backend");
    let mut store = Store::open(StoreConfig::new(temp.path().join("hooks.db"))).unwrap();
    remember_from(&mut store, "here", &workspace, "api", "Recorded here");
    remember_from(&mut store, "there", &elsewhere, "api", "Recorded there");
    store
        .add_prompt(crate::memory::model::AddPrompt {
            session_id: "there".to_owned(),
            content: "a prompt typed in the other checkout".to_owned(),
            project: Some("api".to_owned()),
        })
        .unwrap();

    let outcome = run(
        &mut store,
        HookEvent::SessionStart,
        &HookInput {
            session_id: "new-session".to_owned(),
            cwd: workspace.to_string_lossy().into_owned(),
            ..HookInput::default()
        },
    )
    .unwrap();

    assert_eq!(titles_under(&store, "backend"), ["Recorded here"]);
    assert_eq!(titles_under(&store, "api"), ["Recorded there"]);
    assert_eq!(store.get_session("here").unwrap().project, "backend");
    assert_eq!(store.get_session("there").unwrap().project, "api");
    assert_eq!(
        store.recent_prompts(Some("api"), Some(10)).unwrap().len(),
        1,
        "the other checkout's prompt stays with it"
    );
    assert!(
        outcome
            .warnings
            .iter()
            .any(|warning| warning.contains("1 session") && warning.contains("api")),
        "the sessions left behind are counted: {:?}",
        outcome.warnings
    );
}

#[test]
fn a_project_spelled_with_an_underscore_is_not_folded_into_a_hyphenated_directory_name() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("my-api");
    let elsewhere = temp.path().join("other-checkout");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    git_remote(&workspace, "backend");
    let mut store = Store::open(StoreConfig::new(temp.path().join("hooks.db"))).unwrap();
    remember_from(&mut store, "here", &workspace, "my-api", "Recorded here");
    remember_from(&mut store, "there", &elsewhere, "my_api", "Spelled apart");

    run(
        &mut store,
        HookEvent::SessionStart,
        &HookInput {
            session_id: "new-session".to_owned(),
            cwd: workspace.to_string_lossy().into_owned(),
            ..HookInput::default()
        },
    )
    .unwrap();

    assert_eq!(titles_under(&store, "backend"), ["Recorded here"]);
    assert_eq!(titles_under(&store, "my_api"), ["Spelled apart"]);
}

/// A project named in the payload is that session's project and nothing else.
///
/// The fold took its target from whatever the payload said, so a start naming
/// `other` moved every session and memory recorded in the directory to `other`,
/// and a traversal spelling moved them under whatever was left of the path. A
/// hook has no business relabelling a directory's history because one session
/// was told a name.
#[test]
fn a_project_named_in_the_payload_moves_nothing_else() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("gamma");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut store = Store::open(StoreConfig::new(temp.path().join("hooks.db"))).unwrap();
    remember_from(&mut store, "first", &workspace, "gamma", "First");
    remember_from(&mut store, "second", &workspace, "gamma", "Second");

    for (session, named, resolved) in [
        ("payload-other", "other", "other"),
        ("payload-traversal", "../../etc/passwd", "passwd"),
    ] {
        let outcome = run(
            &mut store,
            HookEvent::SessionStart,
            &HookInput {
                session_id: session.to_owned(),
                cwd: workspace.to_string_lossy().into_owned(),
                project: Some(named.to_owned()),
                ..HookInput::default()
            },
        )
        .unwrap();
        assert_eq!(outcome.project, resolved);
        assert_eq!(store.get_session(session).unwrap().project, resolved);
    }

    assert_eq!(titles_under(&store, "gamma"), ["First", "Second"]);
    assert_eq!(store.get_session("first").unwrap().project, "gamma");
    assert_eq!(store.get_session("second").unwrap().project, "gamma");
    assert!(titles_under(&store, "other").is_empty());
    assert!(titles_under(&store, "passwd").is_empty());
}

/// A relative `cwd` is resolved against the process directory before it is
/// recorded.
///
/// Stored as it arrived, the session carried a directory that only meant
/// something beside the process that wrote it, and the next hook — from another
/// directory, or another run — could not match it to the history it was
/// supposed to join.
#[test]
fn a_relative_cwd_records_an_absolute_directory() {
    let (_temp, mut store) = store();
    let outcome = run(
        &mut store,
        HookEvent::SessionStart,
        &HookInput {
            session_id: "relative-session".to_owned(),
            cwd: "some/relative/place".to_owned(),
            ..HookInput::default()
        },
    )
    .unwrap();
    let directory = PathBuf::from(&store.get_session("relative-session").unwrap().directory);
    assert!(
        directory.is_absolute(),
        "{outcome:?} recorded {directory:?}"
    );
    assert_eq!(
        directory,
        std::env::current_dir().unwrap().join("some/relative/place")
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

/// A guess is the one detection a directory's history should overrule: a session
/// owns its project for the whole conversation, so a removed worktree opening a
/// session under its folder name would file everything after it there too.
#[test]
fn a_guessed_project_gives_way_to_the_one_the_directory_recorded() {
    let temp = TempDir::new().unwrap();
    let removed = temp.path().join("wt-deleted");
    let mut store = Store::open(StoreConfig::new(temp.path().join("hooks.db"))).unwrap();
    store
        .create_session("earlier", "real-project", &removed.to_string_lossy())
        .unwrap();

    let outcome = run(
        &mut store,
        HookEvent::SessionStart,
        &HookInput {
            session_id: "later".to_owned(),
            cwd: removed.to_string_lossy().into_owned(),
            ..HookInput::default()
        },
    )
    .unwrap();

    assert_eq!(outcome.project, "real-project", "{outcome:?}");
    assert_eq!(store.get_session("later").unwrap().project, "real-project");
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
}

/// A refused id is derived again when the drift fold changes the project.
///
/// The id is derived from the name detection gives the checkout, and then the
/// directory's history replaces that name with the project it recorded.
/// Re-deriving only a blank payload id left the refused one pointing at
/// detection's name, while the spool replay — which marks a refused id as
/// derived — used the recorded project. One capture could land in two sessions.
#[test]
fn a_refused_session_id_is_derived_from_the_project_the_directory_recorded() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("api");
    std::fs::create_dir_all(&workspace).unwrap();
    // Detection reads the remote, so the name it gives is not a guess and the
    // only thing that can move the project is the directory's history.
    git_remote(&workspace, "new-name");
    let mut store = Store::open(StoreConfig::new(temp.path().join("hooks.db"))).unwrap();
    store
        .create_session("earlier", "old-name", &workspace.to_string_lossy())
        .unwrap();

    let outcome = run(
        &mut store,
        HookEvent::SessionStart,
        &HookInput {
            session_id: "s".repeat(10_000),
            cwd: workspace.to_string_lossy().into_owned(),
            ..HookInput::default()
        },
    )
    .unwrap();

    assert_eq!(outcome.project, "old-name", "{outcome:?}");
    let expected = crate::mcp::manual_session_id("old-name");
    assert_eq!(outcome.session_id, expected, "{outcome:?}");
    assert_eq!(
        store.get_session(&expected).unwrap().project,
        "old-name",
        "the capture and its session are under the recorded project"
    );
    assert!(
        outcome
            .warnings
            .iter()
            .any(|warning| warning.contains("at most 128 bytes")),
        "{:?}",
        outcome.warnings
    );
}

/// With no history the guess stands, because dropping the session would drop
/// what the agent captures. What it may not do is pass for a finding.
#[test]
fn a_guess_with_no_history_warns_rather_than_passing_for_a_finding() {
    let temp = TempDir::new().unwrap();
    let removed = temp.path().join("wt-deleted");
    let mut store = Store::open(StoreConfig::new(temp.path().join("hooks.db"))).unwrap();

    let outcome = run(
        &mut store,
        HookEvent::SessionStart,
        &HookInput {
            session_id: "s".to_owned(),
            cwd: removed.to_string_lossy().into_owned(),
            ..HookInput::default()
        },
    )
    .unwrap();

    assert_eq!(outcome.project, "wt-deleted", "{outcome:?}");
    assert_eq!(outcome.warnings, [crate::project::DIRECTORY_GONE]);
}

/// A name the agent gave, and a session that already exists, were never guesses.
#[test]
fn a_guess_does_not_overrule_a_named_project() {
    let temp = TempDir::new().unwrap();
    let removed = temp.path().join("wt-deleted");
    let mut store = Store::open(StoreConfig::new(temp.path().join("hooks.db"))).unwrap();
    store
        .create_session("earlier", "recorded", &removed.to_string_lossy())
        .unwrap();

    let outcome = run(
        &mut store,
        HookEvent::SessionStart,
        &HookInput {
            session_id: "later".to_owned(),
            cwd: removed.to_string_lossy().into_owned(),
            project: Some("named".to_owned()),
            ..HookInput::default()
        },
    )
    .unwrap();

    assert_eq!(outcome.project, "named", "{outcome:?}");
}

#[test]
fn a_renamed_project_folds_the_directory_name_into_the_detected_project() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("legacy-name");
    std::fs::create_dir_all(&workspace).unwrap();
    // The repository took a remote, and with it a name. Nothing below names a
    // project: a payload's name belongs to its own session and must not be the
    // target of a fold.
    git_remote(&workspace, "renamed-project");
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
            ..HookInput::default()
        },
    )
    .unwrap();

    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    assert_eq!(outcome.project, "renamed-project");
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

/// A session-start says when this directory's history is under another name.
///
/// A hook cannot ask which project the user meant, so a warning and a line in
/// the agent's context are what it has instead: the write path refuses and
/// hands over a token, but by the time a session opens the memories are about
/// to be filed, and the agent is the only thing that can put the question to
/// somebody. The project here is not the directory's basename, which is the
/// case `migrate_directory_project` already folds on its own; this is the
/// remote that was renamed, which nothing else notices.
#[test]
fn a_session_start_warns_when_the_directorys_history_is_under_another_name() {
    let (temp, mut store) = store();
    let workspace = temp.path().join("repo");
    std::fs::create_dir_all(&workspace).unwrap();
    git_remote(&workspace, "new-remote");
    store
        .create_session("old", "old-remote", &workspace.to_string_lossy())
        .unwrap();

    let outcome = run(
        &mut store,
        HookEvent::SessionStart,
        &HookInput {
            session_id: "agent-session".to_owned(),
            cwd: workspace.to_string_lossy().into_owned(),
            ..HookInput::default()
        },
    )
    .unwrap();

    assert!(
        outcome.warnings.iter().any(|warning| {
            warning.contains("old-remote")
                && warning.contains("new-remote")
                && warning.contains("projects consolidate")
        }),
        "a drift a hook cannot prompt about still has to be said: {:?}",
        outcome.warnings
    );
    assert_eq!(
        store.get_session("agent-session").unwrap().project,
        "old-remote",
        "the session joins the history, it does not open under the new name"
    );
    let context = outcome
        .additional_context
        .as_deref()
        .expect("a session-start gives the agent a context");
    assert!(
        context.contains("old-remote")
            && context.contains("new-remote")
            && context.contains("projects consolidate"),
        "the question belongs where the agent reads it: {context}"
    );
}

/// A session-start in a directory whose sessions agree says nothing extra.
///
/// The warning is only for a drift. An ordinary repository — the sessions
/// recorded there are the project it still resolves to — must open exactly as
/// it did before, with no line that teaches somebody to skip the warnings.
#[test]
fn a_session_start_in_an_agreeing_directory_says_nothing_about_projects() {
    let (temp, mut store) = store();
    let opening = HookInput {
        session_id: "agent-session".to_owned(),
        cwd: temp.path().to_string_lossy().into_owned(),
        ..HookInput::default()
    };
    let outcome = run(&mut store, HookEvent::SessionStart, &opening).unwrap();
    assert!(
        outcome
            .warnings
            .iter()
            .all(|warning| !warning.contains("projects consolidate")),
        "{:?}",
        outcome.warnings
    );

    // A second start in the same directory, now that a session is recorded
    // there under the name detection resolves to, still agrees.
    let outcome = run(&mut store, HookEvent::SessionStart, &opening).unwrap();
    assert!(
        outcome
            .warnings
            .iter()
            .all(|warning| !warning.contains("projects consolidate")),
        "{:?}",
        outcome.warnings
    );
}

/// A lookup that could not run is not a lookup that found no drift.
///
/// `recent_projects_in_directory` reads `sessions`. If that read fails, the
/// session-start has to say so rather than folding the error into an empty list
/// and passing as agreement — which is how a check that could not run gets
/// reported as one that found nothing. The table is renamed out from under the
/// hook so the statement cannot even be prepared, which is the only way to reach
/// the error branch without a second process holding the store.
#[test]
fn a_session_start_says_when_it_could_not_check_the_directorys_history() {
    let (temp, mut store) = store();
    store
        .connection()
        .execute("ALTER TABLE sessions RENAME TO sessions_moved", [])
        .unwrap();

    // No project named in the payload: a payload's name belongs to its session
    // and the hook does not check a directory's history on its behalf.
    let outcome = run(
        &mut store,
        HookEvent::SessionStart,
        &HookInput {
            session_id: "agent-session".to_owned(),
            cwd: temp.path().to_string_lossy().into_owned(),
            ..HookInput::default()
        },
    )
    .unwrap();
    assert!(
        outcome
            .warnings
            .iter()
            .any(|warning| warning.contains("check this directory's project history")),
        "a check that could not run has to say which it was: {:?}",
        outcome.warnings
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

/// A memory a later one overturned is not handed over at all.
///
/// The hint reads the same `prompt_recall_sql` the search stages do, so the
/// hiding rule reaches it too: a memory that is out of date is not a candidate
/// for the three lines a prompt gets, and the memory that overtook it is. The
/// caveat it used to carry is still there for a direct read by id, which is
/// where an agent that already holds the memory looks.
#[test]
fn a_memory_a_later_one_overturned_is_not_handed_over() {
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
            superseding_id: Some(newer.id),
            ..Default::default()
        })
        .unwrap();

    let after = run(&mut store, HookEvent::UserPromptSubmit, &prompt)
        .unwrap()
        .additional_context
        .expect("the prompt still matches");

    assert!(
        !after.contains(&format!("- #{}", answer.id)),
        "a memory a later one overturned is not named by the hint: {after}"
    );
    assert!(
        after.contains(&format!("- #{}", newer.id)),
        "the memory that overtook it is the one named: {after}"
    );
}

/// A title long enough to be cut, ending in a word nothing else says.
///
/// The marker sits past the 140th character, so a line that carries it was
/// not cut and one that does not was.
fn overlong_title(opening: &str) -> String {
    format!(
        "{opening} {}ENDMARKER",
        "and then some more words ".repeat(12)
    )
}

/// Builds the corpus the hint needs relief from: a crowd that shares a word,
/// and one memory with a title of the given text that answers the question.
fn crowd_and_answer(store: &mut Store, title: &str) -> crate::memory::model::Observation {
    for index in 0..14 {
        store
            .add_observation(memory(
                &format!("Engine note {index}"),
                "A passing remark about the rendering engine and nothing else",
            ))
            .unwrap();
    }
    store
        .add_observation(memory(
            title,
            "The storage engine question was settled: SQLite, chosen for being local-first",
        ))
        .unwrap()
        .observation
}

/// A title in the per-prompt hint is cut where the opening block cuts it.
///
/// The hint printed `one_line(title)`, which folds and cuts nothing, while the
/// block bound the same title at `TITLE_CHARS`: 93 of 4,756 real titles were
/// past it, and the hint is the line printed at every prompt.
#[test]
fn a_long_title_in_the_hint_is_cut_like_the_blocks() {
    let (temp, mut store) = store();
    let payload = input(temp.path());
    run(&mut store, HookEvent::SessionStart, &payload).unwrap();
    let long = overlong_title("We chose SQLite as the storage engine");
    assert!(long.chars().count() > crate::memory::normalize::TITLE_CHARS);
    let answer = crowd_and_answer(&mut store, &long);

    let mut prompt = payload.clone();
    prompt.prompt = "Which storage engine did we choose?".to_owned();
    let hint = run(&mut store, HookEvent::UserPromptSubmit, &prompt)
        .unwrap()
        .additional_context
        .expect("the prompt matches something");
    let line = hint
        .lines()
        .find(|line| line.starts_with(&format!("- #{}", answer.id)))
        .unwrap_or_else(|| panic!("the memory that answers is named: {hint}"));
    assert!(line.contains("We chose SQLite"), "{line}");
    assert!(!line.contains("ENDMARKER"), "the title was not cut: {line}");
    assert!(line.ends_with("..."), "{line}");
}

/// The title of the other memory in a caveat is cut like any other.
#[test]
fn a_long_title_in_a_hint_caveat_is_cut_like_the_blocks() {
    let (temp, mut store) = store();
    let payload = input(temp.path());
    run(&mut store, HookEvent::SessionStart, &payload).unwrap();
    let answer = crowd_and_answer(
        &mut store,
        &overlong_title("We chose SQLite as the storage engine"),
    );
    let newer = store
        .add_observation(memory(
            "We moved the storage engine to Postgres",
            "The storage engine is no longer SQLite",
        ))
        .unwrap()
        .observation;
    let relation = propose(&mut store, &newer.sync_id, &answer.sync_id);
    store
        .judge_relation(crate::memory::model::JudgeRelationParams {
            judgment_id: relation,
            relation: crate::store::RELATION_CONFLICTS_WITH.to_owned(),
            marked_by_actor: "agent".to_owned(),
            marked_by_kind: "agent".to_owned(),
            ..Default::default()
        })
        .unwrap();

    let mut prompt = payload.clone();
    prompt.prompt = "Which storage engine did we choose?".to_owned();
    let hint = run(&mut store, HookEvent::UserPromptSubmit, &prompt)
        .unwrap()
        .additional_context
        .expect("the prompt matches something");
    let caveat = hint
        .lines()
        .find(|line| line.starts_with("  (") && line.contains("We chose SQLite"))
        .unwrap_or_else(|| panic!("a conflicting memory carries the caveat: {hint}"));
    assert!(caveat.contains("We chose SQLite"), "{caveat}");
    assert!(
        !caveat.contains("ENDMARKER"),
        "the title was not cut: {caveat}"
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

/// A title in the verdict handover is cut like the opening block's.
#[test]
fn a_long_title_in_the_handover_is_cut_like_the_blocks() {
    let (temp, mut store) = store();
    let payload = input(temp.path());
    run(&mut store, HookEvent::SessionStart, &payload).unwrap();
    let first = store
        .add_observation(memory(
            &overlong_title("Tabs are the rule"),
            "we indent with tabs",
        ))
        .unwrap()
        .observation;
    let second = store
        .add_observation(memory(
            &overlong_title("Spaces are the rule"),
            "we indent with spaces",
        ))
        .unwrap()
        .observation;
    propose(&mut store, &first.sync_id, &second.sync_id);

    let context = run(&mut store, HookEvent::SessionStart, &payload)
        .unwrap()
        .additional_context
        .expect("a session opening carries context");
    let handed = verdict_block(&context);
    assert!(handed.contains("Tabs are the rule"), "{handed}");
    assert!(handed.contains("Spaces are the rule"), "{handed}");
    assert!(
        !handed.contains("ENDMARKER"),
        "a title was not cut: {handed}"
    );
}

/// The handover is the only place an agent meets a pair it did not just save,
/// so it carries the ids a `supersedes` verdict has to name and says to name
/// one.
#[test]
fn the_handover_gives_an_agent_what_a_supersedes_verdict_needs() {
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
    propose(&mut store, &first.sync_id, &second.sync_id);

    let context = run(&mut store, HookEvent::SessionStart, &payload)
        .unwrap()
        .additional_context
        .expect("a session opening carries context");
    let handed = verdict_block(&context);
    assert!(handed.contains("superseding_id"), "{handed}");
    assert!(handed.contains(&format!("#{}", first.id)), "{handed}");
    assert!(handed.contains(&format!("#{}", second.id)), "{handed}");
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
    assert!(
        !handed.contains("waiting since"),
        "a verdict does not depend on how old the pair is: {handed}"
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
    // The sixth is not handed over and is not counted at the agent either: a
    // backlog number names work no tool can list, and the rest arrive at later
    // openings because the oldest come first.
    assert!(
        !handed.contains("more waiting"),
        "the backlog is not a number the agent can act on: {handed}"
    );
}

/// The block rebuilt after a compaction leaves out the prompts of the session
/// it is rebuilding.
///
/// They are the conversation the host just summarised, so listing them back is
/// text the agent has already read. Prompts from other sessions are what the
/// section is for.
#[test]
fn a_compaction_leaves_out_the_prompts_of_the_session_it_rebuilds() {
    let (temp, mut store) = store();
    let payload = input(temp.path());
    run(&mut store, HookEvent::SessionStart, &payload).unwrap();
    store
        .add_prompt(crate::memory::model::AddPrompt {
            session_id: "agent-session".to_owned(),
            content: "the question this session already holds".to_owned(),
            project: Some("hook-project".to_owned()),
        })
        .unwrap();
    store
        .create_session("older", "hook-project", "C:/elsewhere")
        .unwrap();
    store
        .add_prompt(crate::memory::model::AddPrompt {
            session_id: "older".to_owned(),
            content: "a question from an older session".to_owned(),
            project: Some("hook-project".to_owned()),
        })
        .unwrap();

    let rebuilt = run(&mut store, HookEvent::PostCompaction, &payload)
        .unwrap()
        .additional_context
        .expect("a compaction carries the block");
    assert!(
        !rebuilt.contains("the question this session already holds"),
        "this conversation's own prompt comes back: {rebuilt}"
    );
    assert!(
        rebuilt.contains("a question from an older session"),
        "another session's prompt is what the section is for: {rebuilt}"
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
    store.delete_observation(removed.id, None, false).unwrap();

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

    let opened = run(&mut store, HookEvent::SessionStart, &payload).unwrap();
    let context = opened
        .additional_context
        .as_deref()
        .expect("a session opening carries context");
    assert!(
        !context.contains(&stuck) && !context.contains("Waiting on a verdict"),
        "it is never offered as work: {context}"
    );
    // Not hidden either: the one sentence that names it goes to the person,
    // who is the only one who can do anything with it.
    let person = opened.system_message.unwrap_or_default();
    assert!(
        person.contains("1 pending pair") && person.contains("mem_judge cannot settle"),
        "the person is told what the queue holds: {person}"
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
    let opened = run(&mut store, HookEvent::SessionStart, &payload).unwrap();
    let context = opened
        .additional_context
        .as_deref()
        .expect("a session opening carries context");
    assert!(
        verdict_block(context).contains(&fresh),
        "the older stuck pair does not hold the queue: {context}"
    );
    assert!(
        !verdict_block(context).contains(&stuck),
        "and it is still not offered beside the work: {context}"
    );
    assert!(
        opened
            .system_message
            .unwrap_or_default()
            .contains("1 pending pair"),
        "the person is told about it once"
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
                // The greeting this test reads is a report line, so the voice
                // has to be the loud one rather than the new default.
                voice: crate::settings::Voice::All,
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
            semantic_search: None,
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

/// A hook cannot ask a person to fix a name, so an id or a project it cannot
/// use is replaced by one it can, and the sentence says which.
///
/// The replacement is the point: a 10,000-character id was stored and then
/// named the hint state file, so every later hook in that session printed
/// `File name too long (os error 63)` and lost it; a 100,000-character project
/// made every session-start in that directory print a 100 KB warning.
#[test]
fn an_unusable_session_id_or_project_is_replaced_and_says_so() {
    let detection = crate::project::ProjectDetection {
        project: "leteo".to_owned(),
        source: crate::project::SOURCE_GIT_ROOT.to_owned(),
        path: "H:/REPO/leteo".to_owned(),
        available_projects: Vec::new(),
        warning: None,
        error_hint: None,
    };

    let input = HookInput {
        session_id: "s".repeat(10_000),
        ..HookInput::default()
    };
    let (id, reason) = resolve_session_id(&input, "leteo");
    assert_eq!(id, crate::mcp::manual_session_id("leteo"));
    assert!(
        reason.unwrap().contains("at most 128 bytes"),
        "the refusal names the bound"
    );

    let input = HookInput {
        project: Some("p".repeat(100_000)),
        ..HookInput::default()
    };
    let (project, reason) = resolve_project(&input, &detection);
    assert_eq!(project, "leteo");
    assert!(reason.unwrap().contains("at most 128 bytes"));

    // A blank or control-bearing id is the same refusal, and an id a host
    // actually sends is kept as it is.
    for unusable in ["", "   ", "a\0b", "a\tb"] {
        let input = HookInput {
            session_id: unusable.to_owned(),
            ..HookInput::default()
        };
        let (id, _) = resolve_session_id(&input, "leteo");
        assert_eq!(id, crate::mcp::manual_session_id("leteo"));
    }
    let input = HookInput {
        session_id: "abc-123".to_owned(),
        ..HookInput::default()
    };
    assert_eq!(
        resolve_session_id(&input, "leteo"),
        ("abc-123".to_owned(), None)
    );

    // The spool replay derives the fallback id too. A refused id kept in the
    // entry would make `create_session` refuse the replay and drop it, which
    // is the one copy of a subagent's learning that the spool holds.
    let input = HookInput {
        session_id: "s".repeat(10_000),
        stdout: "a learning".to_owned(),
        cwd: "C:/repo".to_owned(),
        ..HookInput::default()
    };
    let (_, _, unsettled) = crate::hooks::pending_capture(&input).expect("a capture");
    assert!(unsettled.derived_session);
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
            resolve_project(&input, &detection).0,
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
    assert_eq!(resolve_project(&input, &detection).0, "chosen-project");
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
    // rather than of the spelling. the directory comparison folds case only under
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

/// The fold moves memories between projects on the word of a recorded
/// directory, so a relative one must not authorise it: `.` resolved against the
/// hook's own directory and read as "recorded here".
#[test]
fn a_relative_recorded_directory_does_not_authorise_the_fold() {
    let temp = TempDir::new().unwrap();
    let mut store = Store::open(StoreConfig::new(temp.path().join("hooks.db"))).unwrap();
    let here = std::env::current_dir().unwrap();
    let legacy = crate::memory::normalize::project(&here.file_name().unwrap().to_string_lossy());
    store
        .create_session("old", &legacy, "C:/placeholder")
        .unwrap();
    store
        .connection()
        .execute("UPDATE sessions SET directory = '.' WHERE id = 'old'", [])
        .unwrap();

    let mut outcome = HookOutcome::default();
    migrate_directory_project(&mut store, &here, "renamed", &mut outcome);

    assert!(
        store.session_directories("renamed").unwrap().is_empty(),
        "the legacy project was folded into another on a relative directory: {outcome:?}"
    );
    assert_eq!(store.session_directories(&legacy).unwrap(), vec!["."]);
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
                &format!(
                    "UPDATE observations SET created_at = datetime('now', '{observations}'), \
                      updated_at = datetime('now', '{observations}')"
                ),
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

/// Revising a memory is saving, and the reminder says nothing about it.
///
/// A topic-key revision writes `updated_at` and leaves `created_at` where the
/// first write put it. The reminder read the newest row's `created_at`, so an
/// agent that spent half an hour refining one memory was told, on every prompt
/// past the fifth minute, that it had kept nothing — a false reminder that cost
/// tokens each time it fired and taught the agent to ignore the true ones.
#[test]
fn a_revision_inside_the_window_keeps_the_reminder_silent() {
    let (temp, mut store) = store();
    let payload = input(temp.path());
    run(&mut store, HookEvent::SessionStart, &payload).unwrap();
    // One memory, written twice under one key: the second write is a revision.
    for body in ["first words", "second words"] {
        store
            .add_observation(AddObservation {
                session_id: payload.session_id.clone(),
                kind: "decision".to_owned(),
                title: "A memory being refined".to_owned(),
                content: body.to_owned(),
                tool_name: None,
                project: Some("hook-project".to_owned()),
                scope: "project".to_owned(),
                topic_key: Some("refined".to_owned()),
                prompt_sync_id: None,
            })
            .unwrap();
    }

    // The conversation began two hours ago and the memory was created
    // yesterday; only the revision is recent.
    let database = store.database_path().to_path_buf();
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute(
            "UPDATE sessions SET started_at = datetime('now', '-2 hours')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE observations SET created_at = datetime('now', '-1 day')",
            [],
        )
        .unwrap();
    drop(connection);

    if let Some(path) = nudge_state_path(&store, &payload.session_id) {
        let _ = std::fs::remove_file(path);
    }
    let outcome = run(&mut store, HookEvent::UserPromptSubmit, &payload).unwrap();
    assert_eq!(
        outcome.system_message, None,
        "a revision a moment ago is somebody who just saved"
    );
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

/// One wait budget per hook process, not one per statement.
///
/// `busy_timeout` was installed once and applied to every statement, so an event
/// that wrote twice — a session created and then a prompt — could wait the whole
/// budget twice and outlast its agent. A hook's budget is now a deadline for the
/// process: each write gets what is left of it, so a second write after a spent
/// budget fails at once instead of waiting again. Read off the connection rather
/// than timed, because a stopwatch on a loaded runner reports the scheduler —
/// the same reason `budget_left_after_opening` exists.
#[test]
fn a_second_write_spends_what_is_left_of_the_hooks_budget() {
    let (temp, store) = store();
    let database = store.database_path().to_path_buf();
    drop(store);

    let budget = HookEvent::UserPromptSubmit.store_wait();
    let mut store = Store::open(StoreConfig {
        busy_timeout: budget,
        single_busy_budget: true,
        ..StoreConfig::new(&database)
    })
    .unwrap();
    let before: i64 = store
        .connection()
        .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
        .unwrap();
    assert!(
        before > 0,
        "the open leaves a budget for the writes that follow: {before} ms"
    );

    // The lock arrives after the open, so the schema pass did not spend the
    // budget and there is a real one for the two writes to share.
    let holder = rusqlite::Connection::open(&database).unwrap();
    holder
        .busy_timeout(std::time::Duration::from_secs(30))
        .unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();

    // A prompt from a session that does not exist yet: `ensure_session` writes,
    // and then `add_prompt` writes again.
    let asking = HookInput {
        prompt: "una pregunta que hay que guardar".to_owned(),
        ..input(temp.path())
    };
    let outcome = run(&mut store, HookEvent::UserPromptSubmit, &asking).unwrap();
    assert!(!outcome.warnings.is_empty(), "{outcome:?}");

    let after: i64 = store
        .connection()
        .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
        .unwrap();
    assert!(
        after < before,
        "the second write must get what is left rather than the whole budget again: \
         {before} ms before, {after} ms after"
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
            relation: crate::store::RELATION_CONFLICTS_WITH.to_owned(),
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
        overturned.contains(&format!("(conflicts with #{}", newer.id)),
        "and the reason it is named again is the caveat: {overturned}"
    );

    // Said once, though: the agent was told, and the next matching prompt is
    // not a reason to tell it again.
    let again = run(&mut store, HookEvent::UserPromptSubmit, &prompt)
        .unwrap()
        .additional_context;
    assert!(
        again.is_none(),
        "a caveat already handed over does not lift the rule a second time: {}",
        again.unwrap_or_default()
    );

    // A different caveat is something new said against it, and lifts the rule
    // again.
    let latest = store
        .add_observation(memory(
            "We moved the storage engine to Postgres, for the second time",
            "The storage engine decision was revisited and changed again",
        ))
        .unwrap()
        .observation;
    let second_relation = propose(&mut store, &latest.sync_id, &answer.sync_id);
    store
        .judge_relation(crate::memory::model::JudgeRelationParams {
            judgment_id: second_relation,
            relation: crate::store::RELATION_CONFLICTS_WITH.to_owned(),
            marked_by_actor: "agent".to_owned(),
            marked_by_kind: "agent".to_owned(),
            ..Default::default()
        })
        .unwrap();
    let renewed = run(&mut store, HookEvent::UserPromptSubmit, &prompt)
        .unwrap()
        .additional_context
        .expect("a caveat nobody has handed over is named");
    assert!(
        renewed.contains(&format!("(conflicts with #{}", latest.id)),
        "{renewed}"
    );

    // Compaction takes the agent's context away, so the list starts over — and
    // the block it rebuilds lists the memory, which is what now keeps the hint
    // quiet. That the list really starts over is the next test's: this memory
    // is recent enough to be in the block.
    let mut settled = payload.clone();
    settled.prompt = String::new();
    let rebuilt = run(&mut store, HookEvent::PostCompaction, &settled)
        .unwrap()
        .additional_context
        .expect("a compaction hands the memory back");
    assert!(rebuilt.contains(&format!("- #{}", answer.id)), "{rebuilt}");
    let after = run(&mut store, HookEvent::UserPromptSubmit, &prompt)
        .unwrap()
        .additional_context;
    assert!(
        after.is_none(),
        "the rebuilt block already carries it: {}",
        after.unwrap_or_default()
    );
}

/// What the opening block listed is not named again by the first prompt.
///
/// The block and the hint are one agent's context: a memory listed under
/// "Also remembered" and then offered again as "Leteo may have something on
/// this" is the same line twice, a few seconds apart.
#[test]
fn a_memory_the_opening_block_listed_is_not_named_by_the_hint() {
    let (temp, mut store) = store();
    let payload = input(temp.path());
    run(&mut store, HookEvent::SessionStart, &payload).unwrap();
    let answer = crowd_and_answer(&mut store, "We chose SQLite as the storage engine");

    let opening = run(&mut store, HookEvent::SessionStart, &payload)
        .unwrap()
        .additional_context
        .expect("a session opening carries context");
    assert!(
        opening.contains(&format!("- #{} ", answer.id)),
        "the block lists the memory the prompt is about: {opening}"
    );

    let mut prompt = payload.clone();
    prompt.prompt = "Which storage engine did we choose?".to_owned();
    let hint = run(&mut store, HookEvent::UserPromptSubmit, &prompt)
        .unwrap()
        .additional_context;
    assert!(
        hint.is_none(),
        "the agent was handed it a moment ago: {}",
        hint.unwrap_or_default()
    );
}

/// A caveat the opening block printed has been handed over, as the memory it
/// sits under has.
///
/// The hint lifts its already-shown rule for a caveat nobody has delivered, so a
/// block that listed a memory without recording its caveat made the first
/// matching prompt name the memory again to say what the block had just said.
#[test]
fn a_caveat_the_opening_block_printed_does_not_lift_the_rule_for_the_hint() {
    let (temp, mut store) = store();
    let payload = input(temp.path());
    run(&mut store, HookEvent::SessionStart, &payload).unwrap();
    let answer = crowd_and_answer(&mut store, "We chose SQLite as the storage engine");
    let newer = store
        .add_observation(memory(
            "We moved the storage engine to Postgres",
            "The storage engine decision was revisited and changed",
        ))
        .unwrap()
        .observation;
    let relation = propose(&mut store, &newer.sync_id, &answer.sync_id);
    store
        .judge_relation(crate::memory::model::JudgeRelationParams {
            judgment_id: relation,
            relation: crate::store::RELATION_CONFLICTS_WITH.to_owned(),
            marked_by_actor: "agent".to_owned(),
            marked_by_kind: "agent".to_owned(),
            ..Default::default()
        })
        .unwrap();

    let opening = run(&mut store, HookEvent::SessionStart, &payload)
        .unwrap()
        .additional_context
        .expect("a session opening carries context");
    assert!(
        opening.contains(&format!("- #{} ", answer.id))
            && opening.contains(&format!("conflicts with #{}", newer.id)),
        "the block lists the memory and says what is against it: {opening}"
    );

    let mut prompt = payload.clone();
    prompt.prompt = "Which storage engine did we choose?".to_owned();
    let hint = run(&mut store, HookEvent::UserPromptSubmit, &prompt)
        .unwrap()
        .additional_context;
    assert!(
        hint.is_none(),
        "the agent was handed the memory and its caveat a moment ago: {}",
        hint.unwrap_or_default()
    );
}

/// A memory the byte budget cut from the block was not shown, so the hint may
/// name it, and a compaction starts the list and the header over.
///
/// The oldest memory is the first thing the budget gives up, and thirty long
/// titles are past the 4,000 bytes of the default size, so the answer — saved
/// first — is cut while the count alone would have listed it.
#[test]
fn a_memory_the_byte_budget_cut_from_the_block_is_still_named() {
    let (temp, mut store) = store();
    let payload = input(temp.path());
    run(&mut store, HookEvent::SessionStart, &payload).unwrap();
    let answer = store
        .add_observation(memory(
            "We chose SQLite as the storage engine",
            "The storage engine question was settled: SQLite, chosen for being local-first",
        ))
        .unwrap()
        .observation;
    for index in 0..29 {
        store
            .add_observation(memory(
                &overlong_title(&format!("Engine note {index}")),
                "A passing remark about the rendering engine and nothing else",
            ))
            .unwrap();
    }

    let opening = run(&mut store, HookEvent::SessionStart, &payload)
        .unwrap()
        .additional_context
        .expect("a session opening carries context");
    assert!(
        !opening.contains(&format!("- #{} ", answer.id)) && opening.contains("(cut to"),
        "the budget cut the answer from the block: {opening}"
    );

    let mut prompt = payload.clone();
    prompt.prompt = "Which storage engine did we choose?".to_owned();
    let first = run(&mut store, HookEvent::UserPromptSubmit, &prompt)
        .unwrap()
        .additional_context
        .expect("it was never shown, so the hint names it");
    assert!(first.contains(&format!("- #{}", answer.id)), "{first}");

    let again = run(&mut store, HookEvent::UserPromptSubmit, &prompt)
        .unwrap()
        .additional_context;
    assert!(again.is_none(), "and now it has been shown");

    run(&mut store, HookEvent::PostCompaction, &payload).unwrap();
    let after = run(&mut store, HookEvent::UserPromptSubmit, &prompt)
        .unwrap()
        .additional_context
        .expect("after a compaction the hint may speak again");
    assert!(after.contains(&format!("- #{}", answer.id)), "{after}");
    assert!(
        after.contains("These are matches on the words"),
        "and the sentence that was compacted away is said again: {after}"
    );
}

/// The full header is said once, then a short line.
#[test]
fn the_second_hint_of_a_session_carries_a_shorter_header() {
    let (temp, mut store) = store();
    let payload = input(temp.path());
    run(&mut store, HookEvent::SessionStart, &payload).unwrap();
    let engine = crowd_and_answer(&mut store, "We chose SQLite as the storage engine");

    let mut prompt = payload.clone();
    prompt.prompt = "Which storage engine did we choose?".to_owned();
    let first = run(&mut store, HookEvent::UserPromptSubmit, &prompt)
        .unwrap()
        .additional_context
        .expect("the first hint speaks");

    // The second hint is the one a new caveat earns, which is the way a
    // conversation gets a second hint about the same subject.
    let newer = store
        .add_observation(memory(
            "We moved the storage engine to Postgres",
            "The storage engine decision was revisited and changed",
        ))
        .unwrap()
        .observation;
    let relation = propose(&mut store, &newer.sync_id, &engine.sync_id);
    store
        .judge_relation(crate::memory::model::JudgeRelationParams {
            judgment_id: relation,
            relation: crate::store::RELATION_CONFLICTS_WITH.to_owned(),
            marked_by_actor: "agent".to_owned(),
            marked_by_kind: "agent".to_owned(),
            ..Default::default()
        })
        .unwrap();
    let second = run(&mut store, HookEvent::UserPromptSubmit, &prompt)
        .unwrap()
        .additional_context
        .expect("the second hint speaks");

    assert!(first.contains(&format!("- #{}", engine.id)), "{first}");
    assert!(second.contains(&format!("- #{}", engine.id)), "{second}");
    let header = |hint: &str| hint.lines().next().unwrap_or_default().to_owned();
    assert!(
        header(&first).contains("These are matches on the words"),
        "{first}"
    );
    assert!(header(&second).len() < header(&first).len() / 2, "{second}");
    assert!(
        header(&second).contains("ignore any that do not fit"),
        "the hedge stays, because the hint is wrong most of the time it speaks: {second}"
    );
}

/// A host that drops the hint after its turn is not told the memory was shown.
///
/// Pi and OpenCode put the hint in the system prompt of one run. Recorded as
/// shown anyway, a memory named once was withheld for the rest of the session
/// from an agent that no longer held it.
#[test]
fn a_host_that_forgets_the_hint_is_offered_the_memory_again() {
    let (temp, mut store) = store();
    let payload = input(temp.path());
    run(&mut store, HookEvent::SessionStart, &payload).unwrap();
    let answer = crowd_and_answer(&mut store, "We chose SQLite as the storage engine");
    let named = format!("- #{}", answer.id);

    let mut prompt = payload.clone();
    prompt.prompt = "Which storage engine did we choose?".to_owned();
    let hint = |store: &mut Store, prompt: &HookInput| {
        run(store, HookEvent::UserPromptSubmit, prompt)
            .unwrap()
            .additional_context
    };

    let mut transient = prompt.clone();
    transient.transient_hints = true;
    for turn in 0..2 {
        let said = hint(&mut store, &transient)
            .unwrap_or_else(|| panic!("turn {turn}: the host lost the hint, so it is said again"));
        assert!(said.contains(&named), "{said}");
        assert!(
            said.contains("These are matches on the words"),
            "turn {turn}: and so is the sentence that explains it: {said}"
        );
    }

    let (temp, mut store) = self::store();
    let payload = input(temp.path());
    run(&mut store, HookEvent::SessionStart, &payload).unwrap();
    crowd_and_answer(&mut store, "We chose SQLite as the storage engine");
    let mut prompt = payload.clone();
    prompt.prompt = "Which storage engine did we choose?".to_owned();
    assert!(hint(&mut store, &prompt).is_some());
    assert!(
        hint(&mut store, &prompt).is_none(),
        "a host that keeps the hint is still not told twice"
    );
}

/// What an opening block listed is only held by a host that keeps the block.
#[test]
fn an_opening_block_a_host_forgets_is_not_recorded_as_held() {
    let question = "Which storage engine did we choose?";
    for (transient, spoken) in [(true, true), (false, false)] {
        let (temp, mut store) = store();
        let payload = input(temp.path());
        run(&mut store, HookEvent::SessionStart, &payload).unwrap();
        let answer = crowd_and_answer(&mut store, "We chose SQLite as the storage engine");
        let mut opening = payload.clone();
        opening.transient_hints = transient;
        let block = run(&mut store, HookEvent::SessionStart, &opening)
            .unwrap()
            .additional_context
            .expect("a session opening carries context");
        assert!(block.contains(&format!("#{}", answer.id)), "{block}");

        let mut prompt = payload.clone();
        prompt.prompt = question.to_owned();
        prompt.transient_hints = transient;
        let hint = run(&mut store, HookEvent::UserPromptSubmit, &prompt)
            .unwrap()
            .additional_context;
        assert_eq!(hint.is_some(), spoken, "transient={transient}: {hint:?}");

        let mut recovery = payload.clone();
        recovery.transient_hints = transient;
        run(&mut store, HookEvent::PostCompaction, &recovery).unwrap();
        let again = run(&mut store, HookEvent::UserPromptSubmit, &prompt)
            .unwrap()
            .additional_context;
        assert_eq!(
            again.is_some(),
            spoken,
            "after a compaction, transient={transient}"
        );
    }
}

/// Opening a session must leave the file readable by every Leteo that wrote one.
#[test]
fn a_state_file_from_before_the_new_fields_still_reads() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("old.nudge");
    std::fs::write(
        &path,
        r#"{"last_nudge":"2026-01-01 00:00:00","shown":[4,9]}"#,
    )
    .unwrap();
    let state = SessionState::read(Some(&path));
    assert_eq!(state.shown, vec![4, 9]);
    assert!(state.caveats.is_empty() && !state.headed);
    assert_eq!(state.last_nudge.as_deref(), Some("2026-01-01 00:00:00"));

    std::fs::write(&path, "2026-01-01 00:00:00").unwrap();
    assert_eq!(
        SessionState::read(Some(&path)).last_nudge.as_deref(),
        Some("2026-01-01 00:00:00")
    );
    std::fs::write(&path, "").unwrap();
    assert!(SessionState::read(Some(&path)).is_blank());
}

/// A write replaces the file whole and leaves nothing beside it.
#[test]
fn the_hint_state_is_written_without_leaving_a_temporary_file() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("hooks").join("session.nudge");
    let mut state = SessionState::default();
    state.remember([7]);
    state.remember_caveats(["7>8>conflicts with".to_owned()]);
    state.headed = true;
    state.write(Some(&path)).unwrap();
    state.write(Some(&path)).unwrap();

    let read = SessionState::read(Some(&path));
    assert_eq!(read.shown, vec![7]);
    assert_eq!(read.caveats, vec!["7>8>conflicts with".to_owned()]);
    assert!(read.headed);
    let beside: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(beside.len(), 1, "{beside:?}");
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

/// A capture the store refuses says what happened, and what to do about it.
///
/// Every hook that loses to another writer says so — on stderr, in the outcome's
/// warnings, where `--verbose` shows it. The agent gets `{}`. That is right
/// almost everywhere: a prompt that was not recorded or a session that was not
/// closed is not something the agent can do anything about, and a workflow
/// finishing dozens of subagents cannot afford a line each.
///
/// `subagent-stop` is the exception, and it is the one the spec already calls
/// out: the learnings live in the text this hook was handed and nowhere else,
/// because the subagent finishes and its context is discarded. Since #202 a
/// busy store does not lose them at all — the capture is spooled and a later
/// open replays it — so that line says "kept for later" and asks for nothing.
/// A refusal a retry cannot mend still loses them, and that line says what to do.
#[test]
fn a_capture_the_store_refuses_says_what_happened_and_what_to_do() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("busy.db");
    crate::settings::save(
        temp.path(),
        &crate::settings::Settings {
            interface: Some(crate::settings::Interface::English),
            // The line this test reads is a capture report, which the new
            // default voice does not say.
            voice: crate::settings::Voice::All,
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
        said.contains("kept for later"),
        "a busy store keeps the capture rather than losing it: {said:?}"
    );
    assert!(
        !said.contains("mem_capture_passive"),
        "and asks for nothing, because there is nothing to send: {said:?}"
    );
    assert!(
        refused
            .response()
            .get("systemMessage")
            .is_some_and(|value| value.as_str() == Some(said.as_str())),
        "and on the channel the agent reads, not only in the warnings"
    );
    assert_eq!(
        spool::pending(&spool::directory(store.data_dir()))
            .unwrap()
            .entries,
        1,
        "and the capture is beside the store for a later open"
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

/// A capture that meets a held store is kept once the store is free.
///
/// The whole point of the spool: a `subagent-stop` is the one event whose text
/// is the subagent's only copy, and a busy store used to lose it after a
/// warning. The capture is written beside the database, and a later session
/// start, subagent stop or session end replays it through the same door.
#[test]
fn a_capture_that_meets_a_held_store_is_kept_once_it_is_free() {
    let (temp, mut store) = store();
    let directory = temp.path().to_path_buf();
    run(&mut store, HookEvent::SessionStart, &input(&directory)).unwrap();
    let database = store.database_path().to_path_buf();
    let data_dir = store.data_dir().to_path_buf();
    drop(store);

    let holder = rusqlite::Connection::open(&database).unwrap();
    holder
        .busy_timeout(std::time::Duration::from_secs(30))
        .unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();

    let mut store = Store::open(StoreConfig {
        busy_timeout: std::time::Duration::from_millis(50),
        ..StoreConfig::new(&database)
    })
    .unwrap();
    let learned = HookInput {
        session_id: "agent-session".to_owned(),
        cwd: directory.to_string_lossy().into_owned(),
        project: Some("hook-project".to_owned()),
        last_assistant_message: "## Key Learnings:
1. A learning that has to survive a store somebody else was writing"
            .to_owned(),
        ..HookInput::default()
    };
    let refused = run(&mut store, HookEvent::SubagentStop, &learned).unwrap();
    assert_eq!(refused.observations_captured, 0, "{refused:?}");
    let spool_dir = spool::directory(&data_dir);
    assert_eq!(
        spool::pending(&spool_dir).unwrap().entries,
        1,
        "the capture is kept beside the store"
    );

    holder.execute_batch("ROLLBACK").unwrap();

    // The session start replays what the busy hook left, at the end of its own run.
    run(&mut store, HookEvent::SessionStart, &input(&directory)).unwrap();
    assert_eq!(
        spool::pending(&spool_dir).unwrap().entries,
        0,
        "and it is drained once the store can take it"
    );
    assert_eq!(
        store
            .recent_observations(Some("hook-project"), Some(10), true)
            .unwrap()
            .len(),
        1,
        "with the learning in the store"
    );
}

/// A spooled capture can name a session that does not exist yet, and replay
/// creates it from the entry. An entry written by a build that recorded `.`
/// must not bring the relative directory back.
#[test]
fn a_replayed_spool_entry_does_not_record_a_relative_directory() {
    let (_temp, mut store) = store();
    let data_dir = store.data_dir().to_path_buf();
    let capture = PassiveCapture {
        session_id: "late-session".to_owned(),
        project: "hook-project".to_owned(),
        content: "## Key Learnings:
1. A relative directory is never recorded"
            .to_owned(),
        source: "subagent-stop".to_owned(),
    };
    spool::spool(&data_dir, "SubagentStop", &capture, ".").unwrap();

    let drained = spool::drain(&mut store, Instant::now() + Duration::from_secs(5));

    assert_eq!(drained.stored, 1, "{drained:?}");
    assert_eq!(store.get_session("late-session").unwrap().directory, "");
}

fn captured_directories(store: &Store) -> Vec<Option<String>> {
    let mut statement = store
        .connection()
        .prepare("SELECT directory FROM observations WHERE tool_name = 'subagent-stop' ORDER BY id")
        .unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// A subagent that stops in another checkout of the project leaves its learnings
/// under the session the conversation opened, and says where they came from.
#[test]
fn a_captured_learning_records_the_directory_the_subagent_stopped_in() {
    let (temp, mut store) = store();
    let first = temp.path().join("first-checkout");
    let second = temp.path().join("second-checkout");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    run(&mut store, HookEvent::SessionStart, &input(&first)).unwrap();

    let mut stop = input(&second);
    stop.stdout =
        "## Key Learnings:\n1. A learning written in the second checkout of the project\n"
            .to_owned();
    let captured = run(&mut store, HookEvent::SubagentStop, &stop).unwrap();
    assert_eq!(captured.observations_captured, 1);

    let recorded = captured_directories(&store);
    assert_eq!(recorded.len(), 1);
    let recorded = recorded[0].clone().expect("a directory was recorded");
    assert!(crate::project::DirectoryTarget::new(&second).matches(&recorded));
    assert!(
        !crate::project::DirectoryTarget::new(&first).matches(&recorded),
        "the session opened in the first one and says so: {recorded}"
    );
}

/// A hook records where its time went, and the agent's answer carries none of
/// it.
///
/// A hook killed at its budget leaves nothing that says whether opening the
/// store, detecting the project, writing or building the block was the slow
/// part, and the store it is slow on is the user's own — so the numbers have to
/// come from the run that is happening. They go to `--verbose` and to standard
/// error; `response()` is what an agent reads and is unchanged.
#[test]
fn a_hook_records_the_time_of_each_phase() {
    let (temp, mut store) = store();
    let directory = temp.path().join("timed");
    std::fs::create_dir_all(&directory).unwrap();

    let outcome = run(&mut store, HookEvent::SessionStart, &input(&directory)).unwrap();

    let phases: Vec<&str> = outcome.timings.iter().map(|timing| timing.phase).collect();
    for phase in ["detect", "recall", "write"] {
        assert!(
            phases.contains(&phase),
            "{phase} is missing from {phases:?}"
        );
    }
    assert!(
        outcome.response().get("timings").is_none(),
        "the agent's answer carries no timing: {}",
        outcome.response()
    );
}

fn vector_rows(store: &Store) -> i64 {
    store
        .connection()
        .query_row("SELECT COUNT(*) FROM observation_vectors", [], |row| {
            row.get(0)
        })
        .unwrap()
}

/// What a hook's store differs in: its own wait, one budget for the process,
/// and no vector made on the write path.
#[test]
fn a_hooks_store_is_shaped_by_the_event_it_runs() {
    let temp = TempDir::new().unwrap();
    let mut config = StoreConfig::new(temp.path().join("hooks.db"));

    HookEvent::store_config(&mut config, HookEvent::SessionStop);

    assert_eq!(config.busy_timeout, HookEvent::SessionStop.store_wait());
    assert!(config.single_busy_budget);
    assert!(
        !config.embed_on_write,
        "a hook leaves its vectors to the backfill"
    );
}

/// Five learnings and no vector: making one is the backfill's work.
///
/// Embedding loads a 13 MB model and runs it, inside the budget the agent is
/// counting down. The memory is committed before either happens, so a hook that
/// spends its budget on vectors is one the agent kills with the write done and
/// the answer unsent. What the hook leaves is exactly what the backfill is for:
/// the store still holds the words, and a vector is derived from them.
#[test]
fn a_subagent_stop_writes_its_learnings_without_a_vector() {
    // A copy of the model in a directory of its own: `is_loaded` answers per
    // model place, and the shipped one is shared with every test in the binary.
    let Some(shipped) = crate::semantic::tests::repository_model() else {
        return;
    };
    let temp = TempDir::new().unwrap();
    let model = temp.path().join("model");
    std::fs::create_dir_all(&model).unwrap();
    for entry in std::fs::read_dir(&shipped).unwrap().flatten() {
        std::fs::copy(entry.path(), model.join(entry.file_name())).unwrap();
    }
    let mut config = StoreConfig::new(temp.path().join("hooks.db"));
    config.model_dir = Some(model.clone());
    HookEvent::store_config(&mut config, HookEvent::SubagentStop);
    let mut store = Store::open(config).unwrap();

    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut stop = input(&workspace);
    stop.stdout = "## Key Learnings:
1. Rotate JWT signing keys every 30 days with a kid header
2. Signing keys live in KMS and tokens carry the kid of the key that signed them
3. The verifier accepts the current and the previous signing key for 48 hours
4. Retired signing keys are removed from KMS after the overlap window
5. Every signature rotation is logged with the kid it introduced and retired"
        .to_owned();

    let captured = run(&mut store, HookEvent::SubagentStop, &stop).unwrap();
    assert_eq!(captured.observations_captured, 5);

    assert_eq!(vector_rows(&store), 0, "the hook made no vector");
    assert!(
        !crate::semantic::is_loaded(store.data_dir(), Some(&model)),
        "and never opened the model file to make one"
    );

    assert_eq!(
        store.backfill_vectors(16).unwrap(),
        5,
        "the backfill gives the same five memories their vectors"
    );
    assert_eq!(vector_rows(&store), 5);

    // Asked in words the memories do not use, so the words alone find nothing
    // and the vector is what answers.
    let asked = "rotación de las claves de firma";
    let by_words = store.search(asked, SearchOptions::default()).unwrap();
    assert!(
        by_words.is_empty(),
        "the fixture is only a fixture if the words find nothing"
    );
    let by_meaning = store
        .search(
            asked,
            SearchOptions {
                semantic: true,
                ..SearchOptions::default()
            },
        )
        .unwrap();
    assert!(
        !by_meaning.is_empty(),
        "what the hook wrote is found by meaning once the backfill has run"
    );
}

/// A learning spooled while the store was busy is stored with the directory the
/// entry carries, not with none.
#[test]
fn a_replayed_spool_entry_records_the_directory_it_was_captured_in() {
    let (temp, mut store) = store();
    let data_dir = store.data_dir().to_path_buf();
    let directory = temp.path().join("captured-here");
    let capture = PassiveCapture {
        session_id: "late-session".to_owned(),
        project: "hook-project".to_owned(),
        content: "## Key Learnings:
1. A replayed learning remembers where it came from"
            .to_owned(),
        source: "subagent-stop".to_owned(),
    };
    spool::spool(
        &data_dir,
        "SubagentStop",
        &capture,
        &directory.to_string_lossy(),
    )
    .unwrap();

    let drained = spool::drain(&mut store, Instant::now() + Duration::from_secs(5));

    assert_eq!(drained.stored, 1, "{drained:?}");
    assert_eq!(
        captured_directories(&store),
        vec![Some(directory.to_string_lossy().into_owned())]
    );
}

/// A capture drained twice is stored once.
///
/// The drain removes an entry it replayed, so a second drain of the same text
/// can only happen when the entry is written again — a replay of a copy left by
/// a crash, or a hook that spooled the same turn twice. The store's hash check
/// is what makes that a no-op rather than a second memory.
#[test]
fn a_spooled_capture_drained_twice_is_stored_once() {
    let (temp, mut store) = store();
    let directory = temp.path().to_path_buf();
    run(&mut store, HookEvent::SessionStart, &input(&directory)).unwrap();
    let data_dir = store.data_dir().to_path_buf();
    let capture = PassiveCapture {
        session_id: "agent-session".to_owned(),
        project: "hook-project".to_owned(),
        content: "## Key Learnings:
1. A learning a double drain must not store twice"
            .to_owned(),
        source: "subagent-stop".to_owned(),
    };

    spool::spool(&data_dir, "SubagentStop", &capture, "").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let first = spool::drain(&mut store, deadline);
    assert_eq!(first.stored, 1, "{first:?}");

    // Aged past the dedupe window, so the narrow guard inside `add_observation`
    // cannot be what makes the second drain a no-op. The check that holds with
    // no time window under it is the hash.
    rusqlite::Connection::open(store.database_path())
        .unwrap()
        .execute(
            "UPDATE observations SET created_at = datetime('now', '-200 minutes'), \
                 updated_at = datetime('now', '-200 minutes')",
            [],
        )
        .unwrap();

    spool::spool(&data_dir, "SubagentStop", &capture, "").unwrap();
    let second = spool::drain(&mut store, deadline);
    assert_eq!(
        second.stored, 0,
        "the same learning is not stored again: {second:?}"
    );
    assert_eq!(
        store
            .recent_observations(Some("hook-project"), Some(10), true)
            .unwrap()
            .len(),
        1
    );
}

/// Runs the two-drainer race over `entries` freshly spooled captures, in one
/// fresh store, and answers every drain's report in the order they ran.
///
/// The caller's own drain runs last, after both threads have joined, so it takes
/// whatever a drainer that found the store busy handed back. Each race owns its
/// store, so one race's leftovers cannot decide the next one's answer.
fn two_drainer_race(entries: usize) -> (TempDir, Store, Vec<spool::DrainReport>) {
    let (temp, mut store) = store();
    let directory = temp.path().to_path_buf();
    run(&mut store, HookEvent::SessionStart, &input(&directory)).unwrap();
    let database = store.database_path().to_path_buf();
    let data_dir = store.data_dir().to_path_buf();
    for index in 0..entries {
        let capture = PassiveCapture {
            session_id: "agent-session".to_owned(),
            project: "hook-project".to_owned(),
            content: format!(
                "## Key Learnings:
1. Learning number {index} that only one drainer may claim"
            ),
            source: "subagent-stop".to_owned(),
        };
        spool::spool(&data_dir, "SubagentStop", &capture, "").unwrap();
    }
    drop(store);

    let deadline = Instant::now() + Duration::from_secs(10);
    let first = std::thread::spawn({
        let database = database.clone();
        move || {
            let mut store = Store::open(StoreConfig::new(&database)).unwrap();
            spool::drain(&mut store, deadline)
        }
    });
    let second = std::thread::spawn({
        let database = database.clone();
        move || {
            let mut store = Store::open(StoreConfig::new(&database)).unwrap();
            spool::drain(&mut store, deadline)
        }
    });
    let mut store = Store::open(StoreConfig::new(&database)).unwrap();
    let mut reports = vec![first.join().unwrap(), second.join().unwrap()];
    reports.push(spool::drain(
        &mut store,
        Instant::now() + Duration::from_secs(10),
    ));
    (temp, store, reports)
}

/// How many entries a race's reports say were replayed.
///
/// A claim handed back to the spool because the store was busy is not a replay,
/// and the store keeping a replayed learning once means `stored` cannot say it.
fn replayed(reports: &[spool::DrainReport]) -> usize {
    reports
        .iter()
        .map(|report| report.claimed - report.returned)
        .sum()
}

/// Two drainers take different entries rather than both replaying one.
///
/// A claim is a marker plus a rename, so no entry is replayed by both.
/// Reading each entry in place instead would have both drainers replay all of
/// them. Raw claims cannot prove it: a drainer that finds the store busy
/// renames its entry back and stops, and the other may claim it again, which
/// made this test count 9 claims of 8 entries on a Windows runner (#257). Nor
/// can `stored`, because the store keeps a replayed learning once however
/// often it is replayed. Claims net of entries handed back can, once a final
/// drain has taken whatever a busy drainer put back.
#[test]
fn two_drainers_claim_each_entry_once() {
    let count = 8;
    let (_temp, store, reports) = two_drainer_race(count);
    assert_eq!(
        reports.last().unwrap().remaining,
        0,
        "the spool drains completely: {reports:?}"
    );
    assert_eq!(
        replayed(&reports),
        count,
        "each entry is replayed by exactly one drainer: {reports:?}"
    );

    assert_eq!(
        store
            .recent_observations(Some("hook-project"), Some(50), true)
            .unwrap()
            .len(),
        count,
        "and each learning is stored once"
    );
}

/// How many of `iterations` two-drainer races replayed an entry twice.
///
/// Each iteration gets its own store, so this counts races that failed rather
/// than failures that piled up in one spool.
#[cfg(windows)]
fn races_replaying_an_entry_twice(iterations: usize, entries: usize) -> usize {
    let mut failed = 0;
    for _ in 0..iterations {
        let (_temp, _store, reports) = two_drainer_race(entries);
        if replayed(&reports) != entries {
            failed += 1;
        }
    }
    failed
}

/// Two hundred runs of the two-drainer race, where a bare rename used to lose.
///
/// The single race above failed about once in a few `windows-latest` runs
/// (#293): `fs::rename` opens the source and moves that handle, so a drainer
/// that opened an entry just before another renamed it moved the same file
/// afterwards and both replayed it. Two hundred iterations turn "once in a few"
/// into a count; the marker `claim` now holds across the rename is what brings
/// it to zero. Measured on `windows-latest`; the developer's machine cannot run
/// it.
#[cfg(windows)]
#[test]
fn the_race_replays_no_entry_twice_in_two_hundred_runs() {
    let iterations = 200;
    let entries = 8;
    let failed = races_replaying_an_entry_twice(iterations, entries);
    assert_eq!(
        failed, 0,
        "{failed} of {iterations} two-drainer races replayed an entry twice"
    );
}

/// A claim's marker is what excludes, and not decoration.
///
/// The rename moves the entry to a name only this drainer reads, but on Windows
/// `fs::rename` moves a handle a drainer already opened, so the rename alone
/// does not keep a second drainer out. The marker `create_new` makes is what
/// does: an entry whose marker already exists is one this drainer has to leave
/// alone, and taking the marker away lets the claim go through.
#[test]
fn a_claim_leaves_an_entry_another_drainer_has_marked() {
    let (temp, mut store) = store();
    let directory = temp.path().to_path_buf();
    run(&mut store, HookEvent::SessionStart, &input(&directory)).unwrap();
    let data_dir = store.data_dir().to_path_buf();
    let capture = PassiveCapture {
        session_id: "agent-session".to_owned(),
        project: "hook-project".to_owned(),
        content: "## Key Learnings:
1. A learning one drainer's marker has to keep the other out of"
            .to_owned(),
        source: "subagent-stop".to_owned(),
    };
    spool::spool(&data_dir, "SubagentStop", &capture, "").unwrap();
    let spool_dir = spool::directory(&data_dir);
    let entry = std::fs::read_dir(&spool_dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|suffix| suffix == "json"))
        .expect("the entry is spooled");
    let marker = spool::marker_path(&entry).expect("the entry has a marker name");

    // The marker another drainer holds for the whole of its rename.
    std::fs::write(&marker, b"").unwrap();
    assert!(
        spool::claim(&entry).is_none(),
        "a drainer leaves an entry another drainer has marked"
    );
    assert_eq!(
        spool::pending(&spool_dir).unwrap().entries,
        1,
        "and the entry is still in the spool"
    );

    std::fs::remove_file(&marker).unwrap();
    let claimed = spool::claim(&entry).expect("with the marker gone the claim goes through");
    assert!(claimed.exists(), "the claim renamed the entry");
    assert_eq!(
        spool::pending(&spool_dir).unwrap().entries,
        1,
        "and it is still waiting, under the name the claim gave it"
    );
}

/// A drain out of budget writes nothing and leaves the spool for later.
///
/// The deadline is the hook's own, so a drain that runs at the end of an event
/// cannot outlast the agent waiting for the answer. A drain already past it has
/// to stop before taking anything rather than spend a wait it does not have.
#[test]
fn a_drain_with_no_budget_left_writes_nothing() {
    let (temp, mut store) = store();
    let directory = temp.path().to_path_buf();
    run(&mut store, HookEvent::SessionStart, &input(&directory)).unwrap();
    let data_dir = store.data_dir().to_path_buf();
    let capture = PassiveCapture {
        session_id: "agent-session".to_owned(),
        project: "hook-project".to_owned(),
        content: "## Key Learnings:
1. A learning a drain with no budget has to leave alone"
            .to_owned(),
        source: "subagent-stop".to_owned(),
    };
    spool::spool(&data_dir, "SubagentStop", &capture, "").unwrap();

    let report = spool::drain(&mut store, Instant::now() - Duration::from_secs(1));
    assert_eq!(
        report.claimed, 0,
        "nothing is taken past the deadline: {report:?}"
    );
    assert_eq!(report.stored, 0, "{report:?}");
    assert_eq!(
        spool::pending(&spool::directory(&data_dir))
            .unwrap()
            .entries,
        1,
        "the entry is still there for a later drain"
    );
    assert_eq!(
        store
            .recent_observations(Some("hook-project"), Some(10), true)
            .unwrap()
            .len(),
        0
    );
}

/// One capture waiting in the spool of a store whose session already exists.
fn store_with_one_spooled_capture() -> (TempDir, Store, PathBuf) {
    let (temp, mut store) = store();
    let directory = temp.path().to_path_buf();
    run(&mut store, HookEvent::SessionStart, &input(&directory)).unwrap();
    let data_dir = store.data_dir().to_path_buf();
    let capture = PassiveCapture {
        session_id: "agent-session".to_owned(),
        project: "hook-project".to_owned(),
        content: "## Key Learnings:
1. A learning that waits for an event nobody is waiting on"
            .to_owned(),
        source: "subagent-stop".to_owned(),
    };
    spool::spool(&data_dir, "SubagentStop", &capture, "").unwrap();
    let spool_dir = spool::directory(&data_dir);
    (temp, store, spool_dir)
}

/// The prompt a person is waiting on does not replay old captures.
///
/// Replay is bounded by the hook's own budget, which for this event is most of
/// the agent's five seconds, so a backlog turned a keystroke into a wait. The
/// events that replay are the ones nobody is typing behind.
#[test]
fn a_prompt_leaves_the_spool_for_an_event_nobody_waits_behind() {
    let (temp, mut store, spool_dir) = store_with_one_spooled_capture();
    let directory = temp.path().to_path_buf();

    let mut prompt = input(&directory);
    prompt.prompt = "a prompt long enough to be stored and searched".to_owned();
    run(&mut store, HookEvent::UserPromptSubmit, &prompt).unwrap();
    assert_eq!(
        spool::pending(&spool_dir).unwrap().entries,
        1,
        "the prompt did not replay the spool"
    );

    run(&mut store, HookEvent::SessionStop, &input(&directory)).unwrap();
    assert_eq!(
        spool::pending(&spool_dir).unwrap().entries,
        0,
        "the end of the session did"
    );
    assert_eq!(
        store
            .recent_observations(Some("hook-project"), Some(10), true)
            .unwrap()
            .len(),
        1,
        "with the learning in the store"
    );
}

/// The compaction hook does not replay either.
///
/// It holds the conversation at the same point a prompt does: a person has
/// asked for something and the agent cannot continue until the hook answers.
#[test]
fn a_compaction_leaves_the_spool_alone() {
    let (temp, mut store, spool_dir) = store_with_one_spooled_capture();

    run(&mut store, HookEvent::PostCompaction, &input(temp.path())).unwrap();

    assert_eq!(spool::pending(&spool_dir).unwrap().entries, 1);
}

/// Opening a session is where a capture left by a busy store is replayed first.
///
/// The prompt and the compaction leave the spool alone, so the session start is
/// what stands between a spooled capture and a conversation that never ends.
#[test]
fn a_session_start_replays_the_spool() {
    let (temp, mut store, spool_dir) = store_with_one_spooled_capture();

    run(&mut store, HookEvent::SessionStart, &input(temp.path())).unwrap();

    assert_eq!(spool::pending(&spool_dir).unwrap().entries, 0);
    assert_eq!(
        store
            .recent_observations(Some("hook-project"), Some(10), true)
            .unwrap()
            .len(),
        1,
        "with the learning in the store"
    );
}

/// A spool write past the cap drops the oldest entries.
#[test]
fn the_spool_drops_the_oldest_past_its_cap() {
    let (_temp, store) = store();
    let data_dir = store.data_dir().to_path_buf();
    let directory = spool::directory(&data_dir);
    let capture = |index: usize| PassiveCapture {
        session_id: "agent-session".to_owned(),
        project: "hook-project".to_owned(),
        content: format!(
            "## Key Learnings:
1. Learning number {index} written past the cap"
        ),
        source: "subagent-stop".to_owned(),
    };

    // One entry first, so the one the cap has to drop is known by name.
    spool::spool(&data_dir, "SubagentStop", &capture(0), "").unwrap();
    let oldest = std::fs::read_dir(&directory)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|suffix| suffix == "json"))
        .expect("the first entry is on disk");

    for index in 1..=spool::SPOOL_CAP {
        spool::spool(&data_dir, "SubagentStop", &capture(index), "").unwrap();
    }
    assert_eq!(
        spool::pending(&directory).unwrap().entries,
        spool::SPOOL_CAP,
        "the cap is the limit that is applied"
    );
    assert!(
        !oldest.exists(),
        "the oldest entry is the one the cap dropped"
    );
}

/// An entry past its retention is dropped without being replayed.
#[test]
fn the_spool_drops_entries_past_their_retention() {
    let (_temp, mut store) = store();
    let data_dir = store.data_dir().to_path_buf();
    let directory = spool::directory(&data_dir);
    std::fs::create_dir_all(&directory).unwrap();
    let stale = chrono::Utc::now().timestamp_millis()
        - (spool::SPOOL_RETENTION_DAYS + 1) * 24 * 60 * 60 * 1_000;
    let path = directory.join(format!(
        "{stale:013}-{:010}-000000.json",
        std::process::id()
    ));
    let body = serde_json::json!({
        "event": "SubagentStop",
        "at": "2020-01-01T00:00:00Z",
        "session_id": "agent-session",
        "project": "hook-project",
        "directory": "",
        "content": "## Key Learnings:\n1. An old learning retention has to drop",
        "source": "subagent-stop",
    })
    .to_string();
    std::fs::write(&path, &body).unwrap();
    assert_eq!(
        spool::pending(&directory).unwrap().entries,
        1,
        "the old entry is there to begin with"
    );

    // A spool write sweeps it away before adding the new one.
    spool::spool(
        &data_dir,
        "SubagentStop",
        &PassiveCapture {
            session_id: "agent-session".to_owned(),
            project: "hook-project".to_owned(),
            content: "## Key Learnings:
1. A fresh learning written after the old one expired"
                .to_owned(),
            source: "subagent-stop".to_owned(),
        },
        "",
    )
    .unwrap();
    assert!(!path.exists(), "the stale entry is gone");
    assert_eq!(
        spool::pending(&directory).unwrap().entries,
        1,
        "only the fresh one is left"
    );

    // And a drain sweeps it too.
    std::fs::write(&path, &body).unwrap();
    let _ = spool::drain(&mut store, Instant::now() + Duration::from_secs(5));
    assert!(!path.exists(), "the drain drops what it would not replay");
}

/// The doctor counts what is waiting and names the repair.
#[test]
fn doctor_counts_the_spool() {
    let (_temp, store) = store();
    let data_dir = store.data_dir().to_path_buf();
    spool::spool(
        &data_dir,
        "SubagentStop",
        &PassiveCapture {
            session_id: "agent-session".to_owned(),
            project: "hook-project".to_owned(),
            content: "## Key Learnings:
1. A learning waiting for the doctor to count it"
                .to_owned(),
            source: "subagent-stop".to_owned(),
        },
        "",
    )
    .unwrap();

    let check = |store: &Store| {
        store
            .doctor()
            .unwrap()
            .checks
            .into_iter()
            .find(|check| check.code == "hook_spool")
            .expect("the doctor reports the spool")
    };
    let failed = check(&store);
    assert!(!failed.ok, "a non-empty spool is reported");
    let detail = failed.detail.unwrap_or_default();
    assert!(detail.contains('1'), "the count is said: {detail:?}");
    assert!(detail.contains("oldest"), "and the age: {detail:?}");
    assert!(
        detail.contains("doctor --repair"),
        "and the remedy: {detail:?}"
    );

    let directory = spool::directory(&data_dir);
    for entry in std::fs::read_dir(&directory).unwrap().flatten() {
        std::fs::remove_file(entry.path()).unwrap();
    }
    assert!(check(&store).ok, "an empty spool is the ordinary state");
}

/// An entry nothing can read is dropped, and the report says how many went and
/// why.
///
/// `doctor --repair` is the command somebody runs when a capture has gone
/// missing, and `dropped: 1` with no reason tells them only that it did. The
/// count and the map move together through the same call, so a report cannot
/// say one was dropped and name reasons for none.
#[test]
fn a_drain_reports_the_entries_it_dropped_and_why() {
    let (_temp, mut store) = store();
    let data_dir = store.data_dir().to_path_buf();
    let directory = spool::directory(&data_dir);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("00000000000000000001.json"),
        b"not a capture",
    )
    .unwrap();

    let report = spool::drain(&mut store, Instant::now() + Duration::from_secs(5));

    assert_eq!(report.dropped, 1, "{report:?}");
    assert_eq!(
        report
            .dropped_reasons
            .get("the entry could not be read as a capture"),
        Some(&1),
        "{report:?}"
    );
    assert_eq!(
        report.dropped_reasons.values().sum::<usize>(),
        report.dropped,
        "the reasons account for every dropped entry: {report:?}"
    );
    assert_eq!(
        report.remaining, 0,
        "and the litter is not left for the next drain: {report:?}"
    );
}

/// A spooled capture whose session never existed is still stored when drained.
///
/// The busy-open path spools before `ensure_session` could run, so the entry can
/// name a session no store ever created. The drain ensures it from the entry's
/// own session and project, exactly as a live `subagent-stop` does, rather than
/// letting `passive_capture` fail `SessionNotFound` and delete the subagent's
/// only copy — which is the loss the spool exists to prevent.
#[test]
fn a_spooled_capture_without_a_session_is_stored_when_drained() {
    let (_temp, mut store) = store();
    let data_dir = store.data_dir().to_path_buf();
    let capture = PassiveCapture {
        session_id: "never-created".to_owned(),
        project: "hook-project".to_owned(),
        content: "## Key Learnings:
1. A learning whose session was never created before the busy open"
            .to_owned(),
        source: "subagent-stop".to_owned(),
    };
    spool::spool(&data_dir, "SubagentStop", &capture, "C:/repo").unwrap();

    let report = spool::drain(&mut store, Instant::now() + Duration::from_secs(5));
    assert_eq!(report.stored, 1, "the capture is stored: {report:?}");
    assert_eq!(report.dropped, 0, "and not dropped: {report:?}");
    assert_eq!(
        store
            .recent_observations(Some("hook-project"), Some(10), true)
            .unwrap()
            .len(),
        1,
        "the learning reached the store"
    );
    assert!(
        store.get_session("never-created").is_ok(),
        "and the session it named was ensured"
    );
}

/// A session owns its project for the whole conversation, so a capture spooled
/// under the payload's guess is filed where that session lives when it is
/// replayed. The busy-open path could not ask; the replay can.
#[test]
fn a_replayed_capture_is_filed_under_its_session_s_project() {
    let (_temp, mut store) = store();
    let data_dir = store.data_dir().to_path_buf();
    store
        .create_session("shared", "myrepo", "C:/work/wt")
        .unwrap();
    let capture = PassiveCapture {
        session_id: "shared".to_owned(),
        project: "wt-folder".to_owned(),
        content: "## Key Learnings:
1. Filed under the session, not the folder the busy open froze"
            .to_owned(),
        source: "subagent-stop".to_owned(),
    };
    spool::spool(&data_dir, "SubagentStop", &capture, "C:/work/wt").unwrap();

    let report = spool::drain(&mut store, Instant::now() + Duration::from_secs(5));

    assert_eq!(report.stored, 1, "the capture is stored: {report:?}");
    assert_eq!(
        store.get_session("shared").unwrap().project,
        "myrepo",
        "the existing session kept its project"
    );
    assert_eq!(
        store
            .recent_observations(Some("myrepo"), Some(10), true)
            .unwrap()
            .len(),
        1,
        "the learning reached the project the session belongs to"
    );
    assert!(
        store
            .recent_observations(Some("wt-folder"), Some(10), true)
            .unwrap()
            .is_empty(),
        "and not the folder's name"
    );
}

/// The other half the busy-open path could not do: a guess gives way to the
/// project the directory's sessions recorded, exactly as on the live path.
#[test]
fn a_replayed_guess_gives_way_to_the_project_the_directory_recorded() {
    let temp = TempDir::new().unwrap();
    let removed = temp.path().join("wt-deleted");
    let mut store = Store::open(StoreConfig::new(temp.path().join("hooks.db"))).unwrap();
    let data_dir = store.data_dir().to_path_buf();
    store
        .create_session("earlier", "real-project", &removed.to_string_lossy())
        .unwrap();
    let capture = PassiveCapture {
        session_id: "later".to_owned(),
        project: "wt-deleted".to_owned(),
        content: "## Key Learnings:
1. The directory's sessions decide which project this is"
            .to_owned(),
        source: "subagent-stop".to_owned(),
    };
    spool::spool_unsettled(
        &data_dir,
        "SubagentStop",
        &capture,
        &removed.to_string_lossy(),
        spool::Unsettled {
            guessed: true,
            derived_session: false,
        },
    )
    .unwrap();

    let report = spool::drain(&mut store, Instant::now() + Duration::from_secs(5));

    assert_eq!(report.stored, 1, "{report:?}");
    assert_eq!(store.get_session("later").unwrap().project, "real-project");
    assert_eq!(
        store
            .recent_observations(Some("real-project"), Some(10), true)
            .unwrap()
            .len(),
        1
    );
    assert!(
        store
            .recent_observations(Some("wt-deleted"), Some(10), true)
            .unwrap()
            .is_empty()
    );
}

/// When the payload named no session, the id was derived from the guess. A
/// replay that corrects the project derives the id again, or the capture would
/// land in a session the live path would never have opened.
#[test]
fn a_replayed_capture_without_a_named_session_derives_the_id_again() {
    let temp = TempDir::new().unwrap();
    let removed = temp.path().join("wt-deleted");
    let mut store = Store::open(StoreConfig::new(temp.path().join("hooks.db"))).unwrap();
    let data_dir = store.data_dir().to_path_buf();
    store
        .create_session("earlier", "real-project", &removed.to_string_lossy())
        .unwrap();
    let guessed = crate::mcp::manual_session_id("wt-deleted");
    let capture = PassiveCapture {
        session_id: guessed.clone(),
        project: "wt-deleted".to_owned(),
        content: "## Key Learnings:
1. The id follows the project, not the folder"
            .to_owned(),
        source: "subagent-stop".to_owned(),
    };
    spool::spool_unsettled(
        &data_dir,
        "SubagentStop",
        &capture,
        &removed.to_string_lossy(),
        spool::Unsettled {
            guessed: true,
            derived_session: true,
        },
    )
    .unwrap();

    let report = spool::drain(&mut store, Instant::now() + Duration::from_secs(5));

    assert_eq!(report.stored, 1, "{report:?}");
    let resolved = crate::mcp::manual_session_id("real-project");
    assert!(
        store.get_session(&resolved).is_ok(),
        "the session follows the corrected project"
    );
    assert!(
        store.get_session(&guessed).is_err(),
        "the guess's session was not invented"
    );
}

/// A pid that named a process which has already exited.
///
/// Spawned and waited instead of hard-coded, so the test does not depend on
/// the platform's pid range or on a number that happens to be free today.
fn pid_that_has_exited() -> u32 {
    #[cfg(windows)]
    let mut child = std::process::Command::new("cmd")
        .args(["/C", "exit"])
        .spawn()
        .unwrap();
    #[cfg(not(windows))]
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    pid
}

/// The one `.json` entry in the spool, whatever the tests before it left.
fn spooled_entry(data_dir: &Path) -> PathBuf {
    let dir = spool::directory(data_dir);
    std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.to_string_lossy().ends_with(".json"))
        .expect("the capture was spooled")
}

/// The claim is a rename, and a hook killed between it and its marker cleanup
/// leaves the entry under a name `entries` does not read. It is the subagent's
/// only copy, so the next drain has to find it again.
#[test]
fn a_claim_left_by_a_dead_process_is_replayed_by_the_next_drain() {
    let (_temp, mut store) = store();
    let data_dir = store.data_dir().to_path_buf();
    let capture = PassiveCapture {
        session_id: "agent-session".to_owned(),
        project: "hook-project".to_owned(),
        content: "## Key Learnings:
1. A capture a killed drainer left claimed"
            .to_owned(),
        source: "subagent-stop".to_owned(),
    };
    spool::spool(&data_dir, "SubagentStop", &capture, "C:/repo").unwrap();
    let entry = spooled_entry(&data_dir);
    let marker = spool::marker_path(&entry).unwrap();
    std::fs::write(&marker, "").unwrap();
    let claimed = entry.with_file_name(format!(
        "{}.claimed-{}-{}",
        entry.file_name().unwrap().to_string_lossy(),
        pid_that_has_exited(),
        chrono::Utc::now().timestamp_millis()
    ));
    std::fs::rename(&entry, &claimed).unwrap();

    let report = spool::drain(&mut store, Instant::now() + Duration::from_secs(5));

    assert_eq!(
        report.stored, 1,
        "the claimed capture was replayed: {report:?}"
    );
    assert_eq!(
        store
            .recent_observations(Some("hook-project"), Some(10), true)
            .unwrap()
            .len(),
        1,
        "the learning reached the store"
    );
    assert!(!claimed.exists(), "the claim is not left behind");
    assert!(!marker.exists(), "nor the marker the killed hook left");
}

/// `doctor` reads `pending`, and a claimed entry is one of the two states a
/// capture waits in. Counting only unclaimed names said nothing was waiting
/// while the only copy sat under a pid.
#[test]
fn doctor_counts_a_claimed_entry_as_waiting() {
    let (_temp, store) = store();
    let data_dir = store.data_dir().to_path_buf();
    let capture = PassiveCapture {
        session_id: "agent-session".to_owned(),
        project: "hook-project".to_owned(),
        content: "## Key Learnings:
1. A capture waiting in a claim"
            .to_owned(),
        source: "subagent-stop".to_owned(),
    };
    spool::spool(&data_dir, "SubagentStop", &capture, "C:/repo").unwrap();
    let entry = spooled_entry(&data_dir);
    let claimed = entry.with_file_name(format!(
        "{}.claimed-{}",
        entry.file_name().unwrap().to_string_lossy(),
        std::process::id()
    ));
    std::fs::rename(&entry, &claimed).unwrap();

    let pending = spool::pending(&spool::directory(&data_dir)).unwrap();

    assert_eq!(pending.entries, 1, "the claim is waiting: {pending:?}");
    assert!(pending.oldest.is_some(), "and it has an age");
}

/// A claim held by a process that is still running belongs to it. Taking it
/// back would replay one capture twice, which is worse than waiting.
#[test]
fn a_live_claim_is_left_for_its_owner() {
    let (_temp, mut store) = store();
    let data_dir = store.data_dir().to_path_buf();
    let capture = PassiveCapture {
        session_id: "agent-session".to_owned(),
        project: "hook-project".to_owned(),
        content: "## Key Learnings:
1. A capture another drain is replaying right now"
            .to_owned(),
        source: "subagent-stop".to_owned(),
    };
    spool::spool(&data_dir, "SubagentStop", &capture, "C:/repo").unwrap();
    let entry = spooled_entry(&data_dir);
    let claimed = spool::claim(&entry).expect("this process claims the entry");

    let report = spool::drain(&mut store, Instant::now() + Duration::from_secs(5));

    assert_eq!(report.stored, 0, "the claim was not taken: {report:?}");
    assert!(
        claimed.exists(),
        "the live owner's claim is where it left it"
    );
    assert_eq!(
        store
            .recent_observations(Some("hook-project"), Some(10), true)
            .unwrap()
            .len(),
        0
    );
}

/// The age bound answers a pid probe that cannot tell. It has to measure the
/// claim, not the entry: claiming is a rename, and a rename does not touch the
/// mtime, so an entry that had waited past the bound would be taken from a live
/// drainer the moment it was claimed, and two drains would replay it.
#[test]
fn a_claim_past_the_age_bound_is_released_from_a_live_owner() {
    let (_temp, mut store) = store();
    let data_dir = store.data_dir().to_path_buf();
    let capture = PassiveCapture {
        session_id: "agent-session".to_owned(),
        project: "hook-project".to_owned(),
        content: "## Key Learnings:
1. A capture a wedged drainer claimed an hour ago"
            .to_owned(),
        source: "subagent-stop".to_owned(),
    };
    spool::spool(&data_dir, "SubagentStop", &capture, "C:/repo").unwrap();
    let entry = spooled_entry(&data_dir);
    let an_hour_ago = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
        - 3_600_000;
    let claimed = entry.with_file_name(format!(
        "{}.claimed-{}-{an_hour_ago}",
        entry.file_name().unwrap().to_string_lossy(),
        std::process::id()
    ));
    std::fs::rename(&entry, &claimed).unwrap();

    let report = spool::drain(&mut store, Instant::now() + Duration::from_secs(5));

    assert_eq!(
        report.stored, 1,
        "the wedged claim was replayed: {report:?}"
    );
    assert!(!claimed.exists(), "and the claim is gone");
}

/// A temporary file is not an entry, so a concurrent drain never claims one.
///
/// `spool` writes to a `.tmp` name and renames it into place. If the temporary
/// ended in `.json`, `entries` would enumerate it and a concurrent drain could
/// claim a half-written turn, drop it as litter, and leave the writer's rename
/// failing — the capture lost, which is the atomicity the rename exists to buy.
#[test]
fn a_temporary_file_is_not_a_spool_entry() {
    let (_temp, mut store) = store();
    let data_dir = store.data_dir().to_path_buf();
    let directory = spool::directory(&data_dir);
    std::fs::create_dir_all(&directory).unwrap();
    assert!(
        !spool::temporary_name("2026-01-01-0-0.json").ends_with(".json"),
        "the temporary is not a name the drain enumerates"
    );
    let temporary = directory.join("0000000000000-0000000000-000000.json.tmp");
    std::fs::write(&temporary, b"{").unwrap();

    assert_eq!(
        spool::pending(&directory).unwrap().entries,
        0,
        "an in-flight temporary is not counted"
    );
    let report = spool::drain(&mut store, Instant::now() + Duration::from_secs(5));
    assert_eq!(report.claimed, 0, "and never claimed: {report:?}");
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
/// The per-prompt hint promises exactly this — and what makes
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

/// `unwrap_or_default` on the lookup printed the match as if nothing had been
/// said against it, in the line an agent reads at every prompt.
#[test]
fn a_hint_whose_caveat_lookup_failed_says_so_once_and_still_names_the_memory() {
    let (temp, mut store) = store();
    let payload = input(temp.path());
    run(&mut store, HookEvent::SessionStart, &payload).unwrap();
    // The relevance bar is relative to the median of the candidates, so a lone
    // match is not enough for the hint to speak.
    for index in 0..14 {
        store
            .add_observation(memory(
                &format!("Engine note {index}"),
                "A passing remark about the rendering engine and nothing else",
            ))
            .unwrap();
    }
    let listed = crate::recall::unreadable::listed_memory_with_unreadable_caveats(
        &mut store,
        "agent-session",
        "hook-project",
        "We chose SQLite as the storage engine",
        "We moved the storage engine to Postgres",
    );

    let mut prompt = payload.clone();
    prompt.prompt = "Which storage engine did we choose?".to_owned();
    let hint = run(&mut store, HookEvent::UserPromptSubmit, &prompt)
        .unwrap()
        .additional_context
        .expect("the prompt matches the memory that is still listed");

    assert_eq!(
        hint.matches(crate::recall::CAVEATS_UNREAD_HINT).count(),
        1,
        "{hint}"
    );
    assert!(hint.contains(&format!("- #{}", listed.id)), "{hint}");
}
