use std::path::Path;

use assert_cmd::Command;
use serde_json::{Value, json};

fn leteo(database: &Path) -> Command {
    let mut command = Command::cargo_bin("leteo").expect("find leteo test binary");
    command.arg("--database").arg(database);
    command
}

fn run_json(command: &mut Command) -> Value {
    let output = command.assert().success().get_output().stdout.clone();
    serde_json::from_slice(&output).expect("CLI stdout is JSON")
}

#[test]
fn cli_persists_and_queries_an_observation_in_an_absolute_temporary_database() {
    let temp = tempfile::tempdir().expect("create CLI test directory");
    let database = temp.path().join("leteo-cli.db");
    assert!(database.is_absolute());

    let session = run_json(
        leteo(&database)
            .arg("session-start")
            .arg("cli-session")
            .arg("--project")
            .arg("cli-project")
            .arg("--directory")
            .arg(temp.path()),
    );
    assert_eq!(session["id"], json!("cli-session"));
    assert_eq!(session["project"], json!("cli-project"));
    assert_eq!(
        session["directory"],
        json!(temp.path().to_string_lossy().as_ref())
    );

    let saved = run_json(
        leteo(&database)
            .arg("save")
            .arg("Portable CLI lookup")
            .arg("The deterministic marker is clineedle")
            .arg("--session")
            .arg("cli-session")
            .arg("--project")
            .arg("cli-project")
            .arg("--type")
            .arg("discovery"),
    );
    assert_eq!(saved["kind"], json!("inserted"));
    assert_eq!(saved["observation"]["session_id"], json!("cli-session"));
    assert_eq!(saved["observation"]["title"], json!("Portable CLI lookup"));

    let results = run_json(
        leteo(&database)
            .arg("search")
            .arg("clineedle")
            .arg("--project")
            .arg("cli-project"),
    );
    let results = results.as_array().expect("search output is an array");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["title"], json!("Portable CLI lookup"));
    assert_eq!(
        results[0]["content"],
        json!("The deterministic marker is clineedle")
    );

    let stats = run_json(leteo(&database).arg("stats"));
    assert_eq!(stats["total_sessions"], json!(1));
    assert_eq!(stats["total_observations"], json!(1));
    assert_eq!(stats["total_prompts"], json!(0));
    assert_eq!(stats["projects"], json!(["cli-project"]));
}

#[test]
fn cli_covers_timeline_context_doctor_and_export_import_round_trip() {
    let temp = tempfile::tempdir().expect("create CLI test directory");
    let database = temp.path().join("leteo-parity.db");

    run_json(
        leteo(&database)
            .arg("session-start")
            .arg("parity-session")
            .arg("--project")
            .arg("parity-project")
            .arg("--directory")
            .arg(temp.path()),
    );
    let first = run_json(
        leteo(&database)
            .arg("save")
            .arg("First parity note")
            .arg("The deterministic marker is parityneedle")
            .arg("--session")
            .arg("parity-session"),
    );
    let saved_id = first["observation"]["id"].as_i64().expect("observation id");
    assert_eq!(first["observation"]["project"], json!("parity-project"));
    run_json(
        leteo(&database)
            .arg("save")
            .arg("Second parity note")
            .arg("Another marker for the same session")
            .arg("--session")
            .arg("parity-session"),
    );
    run_json(
        leteo(&database)
            .arg("prompt")
            .arg("What did we learn?")
            .arg("--session")
            .arg("parity-session"),
    );

    let timeline = run_json(leteo(&database).arg("timeline").arg(saved_id.to_string()));
    assert_eq!(timeline["focus"]["id"], json!(saved_id));
    assert_eq!(
        timeline["before_total"].as_i64().unwrap_or_default()
            + timeline["after_total"].as_i64().unwrap_or_default()
            + 1,
        2
    );
    assert_eq!(
        timeline["after"].as_array().expect("after entries").len(),
        1
    );

    let context = run_json(leteo(&database).arg("context").arg("parity-project"));
    let context = context["context"].as_str().expect("context markdown");
    assert!(context.contains("### Recent Sessions"));
    assert!(context.contains("First parity note"));
    assert!(context.contains("What did we learn?"));

    let doctor = run_json(leteo(&database).arg("doctor"));
    assert_eq!(doctor["healthy"], json!(true));
    assert_eq!(doctor["observations"], json!(2));

    let export_file = temp.path().join("parity-export.json");
    let exported = run_json(
        leteo(&database)
            .arg("export")
            .arg("--project")
            .arg("parity-project")
            .arg("--output")
            .arg(&export_file),
    );
    assert_eq!(exported["observations"], json!(2));
    assert_eq!(exported["prompts"], json!(1));
    assert!(export_file.exists());

    let target = temp.path().join("leteo-import.db");
    let imported = run_json(leteo(&target).arg("import").arg(&export_file));
    assert_eq!(imported["observations_imported"], json!(2));
    // Across the whole store: what is being checked is that the import landed,
    // not which project the directory running the test belongs to.
    let results = run_json(
        leteo(&target)
            .arg("search")
            .arg("parityneedle")
            .arg("--all-projects"),
    );
    assert_eq!(results.as_array().expect("search results").len(), 1);
}

#[test]
fn cli_saves_into_a_stable_manual_session_when_no_session_is_given() {
    let temp = tempfile::tempdir().expect("create CLI test directory");
    let database = temp.path().join("leteo-manual.db");

    let first = run_json(
        leteo(&database)
            .arg("save")
            .arg("Manual note")
            .arg("Saved without an explicit session")
            .arg("--project")
            .arg("Manual--Project"),
    );
    assert_eq!(
        first["observation"]["session_id"],
        json!("manual-save-manual-project")
    );
    assert_eq!(first["observation"]["project"], json!("manual-project"));

    let second = run_json(
        leteo(&database)
            .arg("save")
            .arg("Second manual note")
            .arg("Reuses the same manual session")
            .arg("--project")
            .arg("manual-project"),
    );
    assert_eq!(
        second["observation"]["session_id"],
        json!("manual-save-manual-project")
    );

    let stats = run_json(leteo(&database).arg("stats"));
    assert_eq!(stats["total_sessions"], json!(1));
    assert_eq!(stats["total_observations"], json!(2));

    leteo(&database)
        .arg("save")
        .arg("Mismatched note")
        .arg("Explicit project disagrees with the session")
        .arg("--session")
        .arg("manual-save-manual-project")
        .arg("--project")
        .arg("other-project")
        .assert()
        .failure();
}

#[test]
fn cli_conflicts_and_projects_groups_report_and_apply_changes() {
    let temp = tempfile::tempdir().expect("create CLI test directory");
    let database = temp.path().join("leteo-groups.db");

    run_json(
        leteo(&database)
            .arg("session-start")
            .arg("group-session")
            .arg("--project")
            .arg("group-project")
            .arg("--directory")
            .arg(temp.path()),
    );
    // Filler, and load-bearing. A scan only proposes a pair that scores past
    // its floor, and a bm25 term weight grows with how rare a word is across
    // the store — so two memories on their own score near zero however alike
    // they are, and the scan under test finds nothing for a reason that has
    // nothing to do with the scan.
    for index in 0..24 {
        run_json(
            leteo(&database)
                .arg("save")
                .arg(format!("Unrelated note {index} on deployment windows"))
                .arg(format!("Body {index}: staged rollout, canaries, rollback."))
                .arg("--session")
                .arg("group-session"),
        );
    }
    for title in ["Retry backoff policy", "Retry backoff policy revisited"] {
        run_json(
            leteo(&database)
                .arg("save")
                .arg(title)
                .arg("The retry backoff policy doubles every attempt")
                .arg("--session")
                .arg("group-session"),
        );
    }

    let dry_run = run_json(
        leteo(&database)
            .arg("conflicts")
            .arg("scan")
            .arg("--project")
            .arg("group-project"),
    );
    assert_eq!(dry_run["dry_run"], json!(true));
    // The preview says what applying would write, and the apply below matches
    // it. Zero was what a dry run used to report because it never asked.
    let would_write = dry_run["relations_inserted"].as_i64().unwrap_or_default();
    assert!(would_write > 0, "{dry_run}");

    let applied = run_json(
        leteo(&database)
            .arg("conflicts")
            .arg("scan")
            .arg("--project")
            .arg("group-project")
            .arg("--apply"),
    );
    assert_eq!(applied["dry_run"], json!(false));
    assert_eq!(applied["relations_inserted"], json!(1));
    assert_eq!(
        applied["relations_inserted"].as_i64().unwrap_or_default(),
        would_write,
        "the preview and the apply have to agree, or the preview is decoration"
    );

    let listed = run_json(
        leteo(&database)
            .arg("conflicts")
            .arg("list")
            .arg("--project")
            .arg("group-project"),
    );
    assert_eq!(listed["total"], json!(1));
    let relation_id = listed["relations"][0]["id"].as_i64().expect("relation id");
    let shown = run_json(
        leteo(&database)
            .arg("conflicts")
            .arg("show")
            .arg(relation_id.to_string()),
    );
    assert_eq!(shown["judgment_status"], json!("pending"));
    // A verdict is useless without its reason, so show reports the judgment
    // fields alongside the observation titles even while they are still empty.
    assert_eq!(shown["source_title"], json!("Retry backoff policy"));
    assert_eq!(shown["reason"], Value::Null);
    assert_eq!(shown["confidence"], Value::Null);
    assert_eq!(shown["marked_by_model"], Value::Null);
    assert!(shown.get("evidence").is_some());

    let stats = run_json(
        leteo(&database)
            .arg("conflicts")
            .arg("stats")
            .arg("--project")
            .arg("group-project"),
    );
    assert_eq!(stats["by_judgment_status"]["pending"], json!(1));
    assert_eq!(stats["deferred"], json!(0));

    let deferred = run_json(leteo(&database).arg("conflicts").arg("deferred"));
    assert!(deferred.as_array().expect("deferred rows").is_empty());

    run_json(
        leteo(&database)
            .arg("session-start")
            .arg("empty-session")
            .arg("--project")
            .arg("empty-project")
            .arg("--directory")
            .arg(temp.path()),
    );
    let projects = run_json(leteo(&database).arg("projects").arg("list"));
    let projects = projects.as_array().expect("project list");
    assert_eq!(projects.len(), 2);
    assert_eq!(projects[0]["name"], json!("group-project"));
    // The pair plus the filler the scan above needed to have anything to score.
    assert_eq!(projects[0]["observation_count"], json!(26));

    let prune_preview = run_json(leteo(&database).arg("projects").arg("prune"));
    assert_eq!(prune_preview["dry_run"], json!(true));
    assert_eq!(
        prune_preview["projects"][0]["project"],
        json!("empty-project")
    );

    let pruned = run_json(leteo(&database).arg("projects").arg("prune").arg("--apply"));
    assert_eq!(
        pruned["projects"][0]["result"]["sessions_deleted"],
        json!(1)
    );
    let projects = run_json(leteo(&database).arg("projects").arg("list"));
    assert_eq!(projects.as_array().expect("project list").len(), 1);

    let deleted = run_json(
        leteo(&database)
            .arg("delete")
            .arg("project")
            .arg("group-project")
            .arg("--hard"),
    );
    assert_eq!(deleted["observations_deleted"], json!(26));
    assert_eq!(deleted["sessions_deleted"], json!(1));
}

#[test]
fn cli_consolidates_similar_projects_into_one_canonical_name() {
    let temp = tempfile::tempdir().expect("create CLI test directory");
    let database = temp.path().join("leteo-consolidate.db");

    for (session, project) in [
        ("canonical-session", "leteo"),
        ("variant-session", "leteo-old"),
    ] {
        run_json(
            leteo(&database)
                .arg("session-start")
                .arg(session)
                .arg("--project")
                .arg(project)
                .arg("--directory")
                .arg(temp.path()),
        );
        run_json(
            leteo(&database)
                .arg("save")
                .arg(format!("Note for {project}"))
                .arg("Body kept across the consolidation")
                .arg("--session")
                .arg(session),
        );
    }

    let preview = run_json(
        leteo(&database)
            .arg("projects")
            .arg("consolidate")
            .arg("--project")
            .arg("leteo"),
    );
    assert_eq!(preview["dry_run"], json!(true));
    assert_eq!(preview["groups"][0]["canonical"], json!("leteo"));
    assert_eq!(preview["groups"][0]["sources"], json!(["leteo-old"]));
    assert_eq!(preview["groups"][0]["result"], Value::Null);

    let grouped = run_json(
        leteo(&database)
            .arg("projects")
            .arg("consolidate")
            .arg("--all"),
    );
    assert_eq!(grouped["dry_run"], json!(true));
    assert_eq!(grouped["groups"].as_array().expect("groups").len(), 1);
    assert_eq!(grouped["groups"][0]["canonical"], json!("leteo"));
    assert_eq!(grouped["groups"][0]["sources"], json!(["leteo-old"]));

    let applied = run_json(
        leteo(&database)
            .arg("projects")
            .arg("consolidate")
            .arg("--project")
            .arg("leteo")
            .arg("--apply"),
    );
    assert_eq!(
        applied["groups"][0]["result"]["observations_updated"],
        json!(1)
    );

    let projects = run_json(leteo(&database).arg("projects").arg("list"));
    let projects = projects.as_array().expect("project list");
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0]["name"], json!("leteo"));
    assert_eq!(projects[0]["observation_count"], json!(2));
}

#[test]
fn cli_hooks_drive_the_session_lifecycle_from_agent_payloads() {
    let temp = tempfile::tempdir().expect("create CLI test directory");
    let database = temp.path().join("leteo-hooks.db");
    let workspace = temp.path().join("hook-workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");

    let payload = |extra: Value| -> String {
        let mut body = json!({
            "session_id": "agent-session",
            "cwd": workspace.to_string_lossy(),
            "project": "hook-project",
        });
        for (key, value) in extra.as_object().expect("object payload") {
            body[key.as_str()] = value.clone();
        }
        body.to_string()
    };

    let started = run_json(
        leteo(&database)
            .arg("hook")
            .arg("session-start")
            .write_stdin(payload(json!({}))),
    );
    let context = started["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("session start injects context");
    assert_eq!(
        started["hookSpecificOutput"]["hookEventName"],
        json!("SessionStart")
    );
    // The short directive, not the whole protocol: the full text ships in the
    // plugin's skill, where its body is paid for when it is needed rather than
    // in every session.
    assert!(context.contains("Leteo memory — active"), "{context}");
    assert!(context.contains("Saving is not replying"), "{context}");

    let prompted = run_json(
        leteo(&database)
            .arg("hook")
            .arg("user-prompt-submit")
            .arg("--verbose")
            .write_stdin(payload(json!({ "prompt": "Why SQLite?" }))),
    );
    assert_eq!(prompted["prompt_saved"], json!(true));
    assert_eq!(prompted["project"], json!("hook-project"));

    let captured = run_json(
        leteo(&database)
            .arg("hook")
            .arg("subagent-stop")
            .arg("--verbose")
            .write_stdin(payload(json!({
                "stdout": "## Key Learnings:\n1. Hooks must never block the agent's critical path\n"
            }))),
    );
    assert_eq!(captured["observations_captured"], json!(1));

    let recovered = run_json(
        leteo(&database)
            .arg("hook")
            .arg("post-compaction")
            .write_stdin(payload(json!({}))),
    );
    let context = recovered["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("compaction recovery injects context");
    assert!(context.contains("Context was compacted"));
    assert!(context.contains("Hooks must never block"));

    let stopped = run_json(
        leteo(&database)
            .arg("hook")
            .arg("session-stop")
            .write_stdin(payload(json!({}))),
    );
    assert_eq!(stopped, json!({}));

    // A malformed payload must not fail the agent's prompt.
    let tolerated = run_json(
        leteo(&database)
            .arg("hook")
            .arg("user-prompt-submit")
            .write_stdin("{ not json"),
    );
    assert_eq!(tolerated, json!({}));

    let stats = run_json(leteo(&database).arg("stats"));
    assert_eq!(stats["total_prompts"], json!(1));
    assert_eq!(stats["total_observations"], json!(1));
}

#[test]
fn cli_cloud_configuration_enrollment_and_status_round_trip() {
    let temp = tempfile::tempdir().expect("create CLI test directory");
    let data_dir = temp.path().join("data");
    let cloud_config = data_dir.join("cloud.json");
    let leteo_with_data = || {
        let mut command = Command::cargo_bin("leteo").expect("find leteo test binary");
        command.arg("--data-dir").arg(&data_dir);
        // The persisted file must win over a stale environment.
        command.env_remove("LETEO_CLOUD_SERVER");
        command.env_remove("LETEO_CLOUD_TOKEN");
        command
    };

    let shown = run_json(leteo_with_data().arg("cloud").arg("config").arg("show"));
    assert_eq!(shown["exists"], json!(false));
    assert_eq!(shown["config"]["runnable"], json!(false));

    let configured = run_json(
        leteo_with_data()
            .arg("cloud")
            .arg("config")
            .arg("set")
            .arg("--server")
            .arg("https://memory.example.com")
            .arg("--token")
            .arg("a-cloud-token-value")
            .arg("--poll-interval")
            .arg("45")
            .arg("--enable"),
    );
    assert_eq!(
        configured["config"]["server"],
        json!("https://memory.example.com")
    );
    assert_eq!(configured["config"]["token_configured"], json!(true));
    assert_eq!(configured["config"]["poll_interval_seconds"], json!(45));
    assert_eq!(
        configured["config"]["runnable"],
        json!(false),
        "no project is enrolled yet"
    );
    let persisted = std::fs::read_to_string(&cloud_config).expect("cloud config file");
    assert!(persisted.contains("a-cloud-token-value"));

    let enrolled = run_json(
        leteo_with_data()
            .arg("cloud")
            .arg("enroll")
            .arg("--project")
            .arg("Cloud--Project"),
    );
    assert_eq!(enrolled["project"], json!("cloud-project"));
    assert_eq!(enrolled["enrolled"], json!(true));
    assert_eq!(enrolled["projects"], json!(["cloud-project"]));

    let status = run_json(leteo_with_data().arg("cloud").arg("status"));
    assert_eq!(status["config"]["runnable"], json!(true));
    assert_eq!(status["enrolled_projects"], json!(["cloud-project"]));
    assert_eq!(status["pending_mutations"], json!(0));
    assert_eq!(status["deferred_count"], json!(0));
    assert_eq!(status["target"]["target_key"], json!("cloud"));
    assert!(
        !status.to_string().contains("a-cloud-token-value"),
        "status never prints the token"
    );

    // A local write queues a mutation for the cloud target.
    run_json(
        leteo_with_data()
            .arg("save")
            .arg("Cloud note")
            .arg("queued for replication")
            .arg("--project")
            .arg("cloud-project"),
    );
    let status = run_json(leteo_with_data().arg("cloud").arg("status"));
    assert!(
        status["pending_mutations"].as_i64().expect("pending count") >= 2,
        "{status}"
    );

    let removed = run_json(
        leteo_with_data()
            .arg("cloud")
            .arg("enroll")
            .arg("--project")
            .arg("cloud-project")
            .arg("--remove"),
    );
    assert_eq!(removed["enrolled"], json!(false));
    assert_eq!(removed["projects"], json!([]));

    let cleared = run_json(leteo_with_data().arg("cloud").arg("config").arg("clear"));
    assert_eq!(cleared["removed"], json!(true));
    assert!(!cloud_config.exists());

    // Naming a project in the config is saying it replicates, and saying that
    // has to be true. The list in the file decides what the loop is allowed to
    // push; the table in the store decides what is written down to push at all.
    // Setting one without the other left `cloud status` reporting an enabled,
    // runnable replication of a project whose every memory was journalled
    // nowhere — verified against the binary, which reported `projects:
    // ["probe"]`, `enabled: true`, `runnable: true` and an empty queue after a
    // save.
    run_json(
        leteo_with_data()
            .arg("cloud")
            .arg("config")
            .arg("set")
            .arg("--project")
            .arg("configured-project"),
    );
    run_json(
        leteo_with_data()
            .arg("save")
            .arg("A memory in the configured project")
            .arg("It has to be written down for the loop to have anything to push")
            .arg("--project")
            .arg("configured-project"),
    );
    let status = run_json(leteo_with_data().arg("cloud").arg("status"));
    assert_eq!(
        status["enrolled_projects"],
        json!(["configured-project"]),
        "configuring a project has to enrol it, or nothing is ever journalled"
    );
    assert!(
        status["pending_mutations"].as_i64().unwrap_or(0) > 0,
        "the memory saved into a replicated project reached no queue: {status}"
    );

    // And dropping it from the list stops the journal, which is what removing
    // it means.
    run_json(
        leteo_with_data()
            .arg("cloud")
            .arg("config")
            .arg("set")
            .arg("--project")
            .arg("something-else"),
    );
    let status = run_json(leteo_with_data().arg("cloud").arg("status"));
    assert_eq!(status["enrolled_projects"], json!(["something-else"]));
}

#[test]
fn stateless_commands_do_not_create_a_local_database() {
    let temp = tempfile::tempdir().expect("create CLI test directory");
    let data_dir = temp.path().join("unused-data");

    Command::cargo_bin("leteo")
        .expect("find leteo test binary")
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("setup")
        .arg("opencode")
        .arg("--dry-run")
        .assert()
        .success();

    assert!(!data_dir.exists());
}

/// Builds a database shaped like Engram's, so adoption can be driven through
/// the real binary rather than only through the library.
///
/// Uses Engram's names, `user_prompts` among them, so the translation into
/// Leteo's own schema is exercised rather than assumed.
fn engram_fixture(path: &Path) {
    let connection = rusqlite::Connection::open(path).expect("create Engram fixture");
    connection
        .execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, project TEXT NOT NULL,
                 directory TEXT NOT NULL, started_at TEXT NOT NULL,
                 ended_at TEXT, summary TEXT);
             CREATE TABLE observations (id INTEGER PRIMARY KEY AUTOINCREMENT,
                 sync_id TEXT, session_id TEXT NOT NULL, type TEXT NOT NULL,
                 title TEXT NOT NULL, content TEXT NOT NULL, tool_name TEXT,
                 project TEXT, created_at TEXT NOT NULL DEFAULT (datetime('now')),
                 updated_at TEXT NOT NULL DEFAULT (datetime('now')));
             CREATE TABLE user_prompts (id INTEGER PRIMARY KEY AUTOINCREMENT,
                 session_id TEXT NOT NULL, content TEXT NOT NULL, project TEXT,
                 created_at TEXT NOT NULL DEFAULT (datetime('now')));
             CREATE TABLE memory_relations (id INTEGER PRIMARY KEY AUTOINCREMENT,
                 sync_id TEXT UNIQUE, source_id TEXT, target_id TEXT,
                 relation TEXT NOT NULL, judgment_status TEXT NOT NULL);
             INSERT INTO sessions (id, project, directory, started_at)
                 VALUES ('s1', 'moving-in', '/tmp/moving-in', datetime('now'));
             INSERT INTO observations (sync_id, session_id, type, title, content, project)
                 VALUES ('obs-1', 's1', 'decision', 'Chose Postgres', 'body', 'moving-in');
             INSERT INTO observations (sync_id, session_id, type, title, content, project)
                 VALUES ('obs-2', 's1', 'bugfix', 'Fixed the N+1', 'body', 'moving-in');
             INSERT INTO user_prompts (session_id, content, project)
                 VALUES ('s1', 'why is it slow?', 'moving-in');
             INSERT INTO memory_relations (sync_id, source_id, target_id, relation, judgment_status)
                 VALUES ('rel-1', 'obs-1', 'obs-2', 'related', 'judged');",
        )
        .expect("populate Engram fixture");
}

fn relation_count(database: &Path) -> i64 {
    rusqlite::Connection::open(database)
        .expect("open database")
        .query_row("SELECT COUNT(*) FROM memory_relations", [], |row| {
            row.get(0)
        })
        .expect("count relations")
}

#[test]
fn cli_adopts_an_engram_installation_and_then_refuses_to_do_it_twice() {
    let temp = tempfile::tempdir().expect("create adoption test directory");
    let engram = temp.path().join("engram.db");
    let database = temp.path().join("leteo.db");
    engram_fixture(&engram);

    // A dry run reports and writes nothing.
    let preview = run_json(
        leteo(&database)
            .arg("import")
            .arg("--from-engram")
            .arg("--source")
            .arg(&engram)
            .arg("--dry-run"),
    );
    assert_eq!(preview["dry_run"], json!(true));
    assert_eq!(preview["found"]["observations"], json!(2));
    assert_eq!(preview["found"]["relations"], json!(1));
    assert!(!database.exists(), "a dry run must not create the database");

    let adopted = run_json(
        leteo(&database)
            .arg("import")
            .arg("--from-engram")
            .arg("--source")
            .arg(&engram),
    );
    assert_eq!(adopted["adopted"]["observations"], json!(2));
    assert_eq!(adopted["adopted"]["prompts"], json!(1));
    // The relation verdicts are what a JSON export would have dropped.
    assert_eq!(adopted["adopted"]["relations"], json!(1));
    assert_eq!(relation_count(&database), 1);

    // The memories are usable, not merely copied. Across the whole store,
    // because the question here is what adoption brought over and not what the
    // directory this runs from belongs to — see
    // `a_listing_answers_about_the_project_the_directory_belongs_to`.
    let found = run_json(
        leteo(&database)
            .arg("search")
            .arg("Postgres")
            .arg("--all-projects"),
    );
    assert_eq!(found.as_array().expect("search results").len(), 1);

    // Adopting again would replace real memories, so it is refused.
    let refusal = leteo(&database)
        .arg("import")
        .arg("--from-engram")
        .arg("--source")
        .arg(&engram)
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&refusal.get_output().stderr).to_string();
    assert!(
        stderr.contains("already holds 2 observations"),
        "the refusal should name what it found: {stderr}"
    );

    // And the source is untouched throughout, so going back is possible.
    assert_eq!(relation_count(&engram), 1);
}

#[test]
fn cli_setup_offers_the_engram_it_finds_and_stops_once_there_is_nothing_to_offer() {
    let temp = tempfile::tempdir().expect("create setup test directory");
    let engram = temp.path().join("engram.db");
    let database = temp.path().join("leteo.db");
    engram_fixture(&engram);

    // `setup` with no agent lists what can be configured. The offer only
    // appears when this store is still empty, which is checked by the library
    // test; here the CLI shape is what matters.
    let listing = run_json(leteo(&database).arg("setup"));
    assert!(
        listing["agents"].as_array().is_some_and(|a| !a.is_empty()),
        "setup should list the agents it can configure"
    );
}

#[test]
fn an_unreadable_hook_payload_says_so_instead_of_pretending_it_was_empty() {
    // Falling back to an empty payload is right — a hook sits on the agent's
    // critical path and must never block somebody's prompt. Falling back
    // *silently* is what hid a real bug: a `serde` alias turned Codex's
    // ordinary payload into a duplicate field, every hook parsed as an empty
    // `HookInput`, and each one reported success having done nothing. The
    // store filled with sessions nobody could find and prompts that were never
    // saved, and nothing anywhere said why.
    let temp = tempfile::tempdir().expect("create CLI test directory");
    let database = temp.path().join("leteo-unreadable.db");

    // Truncated JSON: the shape a schema mismatch arrives in.
    let verbose = run_json(
        leteo(&database)
            .arg("hook")
            .arg("user-prompt-submit")
            .arg("--verbose")
            .write_stdin(r#"{"session_id": "broken", "#),
    );
    let warnings = verbose["warnings"]
        .as_array()
        .expect("an unreadable payload is reported as a warning");
    assert!(
        warnings
            .iter()
            .filter_map(Value::as_str)
            .any(|warning| warning.contains("could not be read")),
        "the warning has to name what happened: {warnings:?}"
    );

    // And the agent still gets a well-formed answer, because refusing to fail
    // is the point — it is only the silence that was wrong.
    //
    // In the mode hooks actually run in. `--verbose` is a debugging flag; the
    // bundles call `leteo hook <event>` plain, and `HookOutcome::response` does
    // not carry `warnings`, so every one of the nine warnings the hooks collect
    // went nowhere at all. They go to standard error, which stays out of the
    // agent's context and is where its own hook logs look.
    let plain = leteo(&database)
        .arg("hook")
        .arg("user-prompt-submit")
        .write_stdin(r#"{"session_id": "broken", "#)
        .assert()
        .success();
    let plain = plain.get_output();
    let response: Value =
        serde_json::from_slice(&plain.stdout).expect("the agent still gets valid JSON");
    assert!(response.is_object(), "{response}");
    assert!(
        String::from_utf8_lossy(&plain.stderr).contains("could not be read"),
        "the reason has to reach somebody: {:?}",
        String::from_utf8_lossy(&plain.stderr)
    );

    // A payload that parses carries no warning at all, so this cannot become
    // noise on every hook.
    let clean = run_json(
        leteo(&database)
            .arg("hook")
            .arg("user-prompt-submit")
            .arg("--verbose")
            .write_stdin(r#"{"session_id":"fine","prompt":"a question"}"#),
    );
    assert!(
        clean.get("warnings").is_none(),
        "a payload that read cleanly has nothing to warn about: {clean}"
    );
}

/// A hook answers even when the store cannot be opened at all.
///
/// The hook module says it plainly — a malformed payload or a store problem
/// must never block the user's prompt — and the store used to be opened before
/// the hook was reached, so the promise was made in one file and broken in
/// another. A database corrupted, a disk that filled, a drive that went away:
/// `leteo hook user-prompt-submit` printed a Rust error to stderr and exited 1,
/// on every prompt, for as long as the file stayed broken.
///
/// `leteo doctor` is the one command that has to keep failing loudly, because
/// it is what somebody runs to find out what is wrong.
#[test]
fn a_hook_answers_over_a_store_it_cannot_open_and_doctor_still_refuses() {
    let temp = tempfile::tempdir().expect("create CLI test directory");
    let database = temp.path().join("leteo-cli.db");
    std::fs::write(&database, b"this is not a database, it is a sentence").unwrap();
    let payload = json!({
        "session_id": "broken",
        "cwd": temp.path().to_string_lossy(),
        "hook_event_name": "UserPromptSubmit",
        "prompt": "something to ask",
    })
    .to_string();

    let hook = leteo(&database)
        .arg("hook")
        .arg("user-prompt-submit")
        .write_stdin(payload.clone())
        .assert()
        .success();
    let output = hook.get_output();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "{}",
        "a hook with nothing to add answers with nothing to add"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("could not be opened"),
        "and says why where a verbose agent will show it: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The same run, asked to explain itself, names it as a warning.
    let verbose = leteo(&database)
        .arg("hook")
        .arg("user-prompt-submit")
        .arg("--verbose")
        .write_stdin(payload)
        .assert()
        .success();
    let reported: Value =
        serde_json::from_slice(&verbose.get_output().stdout.clone()).expect("verbose stdout");
    assert!(
        reported["warnings"][0]
            .as_str()
            .unwrap_or_default()
            .contains("open Leteo store"),
        "{reported}"
    );

    leteo(&database).arg("doctor").assert().failure();
}

/// `leteo search` says why it came back empty, where a person will see it.
///
/// An empty result reads like "this was never saved" and is usually "your
/// words did not match": memories are written by an agent and are usually in
/// English while the question often is not, and on a real store an English
/// term finds up to twenty memories where its Spanish equivalent finds none.
/// The MCP tool has said so since the hint was written. Somebody at a terminal
/// got `[]`.
///
/// On stderr, because stdout is a JSON array that something may be parsing.
#[test]
fn a_command_line_search_explains_an_empty_answer_without_spoiling_its_output() {
    let temp = tempfile::tempdir().expect("create CLI test directory");
    let database = temp.path().join("leteo-cli.db");
    run_json(
        leteo(&database)
            .arg("session-start")
            .arg("s1")
            .arg("--project")
            .arg("leteo")
            .arg("--directory")
            .arg(temp.path()),
    );
    run_json(
        leteo(&database)
            .arg("save")
            .arg("The connection pool leaked under load")
            .arg("it was never returned on the error path")
            .arg("--session")
            .arg("s1"),
    );

    // Nothing matches: the answer is empty and the reason is on stderr.
    let empty = leteo(&database)
        .arg("search")
        .arg("kubernetes helm chart")
        .assert()
        .success();
    let output = empty.get_output();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "[]",
        "stdout stays a JSON array whatever is said beside it"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("matches words, not meanings"),
        "and the reason is where a person reads it: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // An exact match says nothing extra.
    let exact = leteo(&database)
        .arg("search")
        .arg("connection pool leaked")
        .assert()
        .success();
    assert!(
        String::from_utf8_lossy(&exact.get_output().stderr)
            .trim()
            .is_empty(),
        "a search that answered the question has nothing to add"
    );
}

/// A listing answers about the project somebody is standing in.
///
/// `search`, `recent` and `context` passed `--project` straight through, so
/// with nothing named they answered from the whole store while `mem_search`
/// standing in the same directory answered from one project. Over the 114 real
/// questions asked from inside the Leteo repo, 82% came back with at least one
/// memory from another project and 72% led with one — and of those 77, only two
/// would have found nothing had the search been narrowed.
///
/// The store here holds the same words under two projects, so a command that
/// ignores the directory returns both and one that honours it returns one.
#[test]
fn a_listing_answers_about_the_project_the_directory_belongs_to() {
    let temp = tempfile::tempdir().expect("create CLI test directory");
    let database = temp.path().join("scoped.db");

    let here = temp.path().join("alpha");
    std::fs::create_dir_all(here.join(".leteo")).expect("create the project directory");
    std::fs::write(
        here.join(".leteo").join("config.json"),
        br#"{"project_name": "alpha"}"#,
    )
    .expect("name the project this directory belongs to");

    for (session, project) in [("s-alpha", "alpha"), ("s-beta", "beta")] {
        leteo(&database)
            .arg("session-start")
            .arg(session)
            .arg("--project")
            .arg(project)
            .arg("--directory")
            .arg(temp.path())
            .assert()
            .success();
        leteo(&database)
            .arg("save")
            .arg(format!("connection pool leaked in {project}"))
            .arg("the pool was never returned to")
            .arg("--session")
            .arg(session)
            .assert()
            .success();
    }

    // One memory only beta holds, so that a question about it is answered
    // somewhere and not here — which is the case the hint below is about.
    leteo(&database)
        .arg("save")
        .arg("kafka consumer rebalance storm")
        .arg("the consumer group rebalanced every minute")
        .arg("--session")
        .arg("s-beta")
        .assert()
        .success();

    let scoped = run_json(
        leteo(&database)
            .current_dir(&here)
            .arg("search")
            .arg("connection pool leaked"),
    );
    let projects: Vec<&str> = scoped
        .as_array()
        .expect("search returns an array")
        .iter()
        .map(|row| row["project"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        projects,
        vec!["alpha"],
        "a search from alpha's directory answers about alpha"
    );

    let widened = run_json(
        leteo(&database)
            .current_dir(&here)
            .arg("search")
            .arg("connection pool leaked")
            .arg("--all-projects"),
    );
    assert_eq!(
        widened.as_array().map(Vec::len),
        Some(2),
        "--all-projects is the way back to the whole store"
    );

    let recent = run_json(leteo(&database).current_dir(&here).arg("recent"));
    let recent_projects: Vec<&str> = recent
        .as_array()
        .expect("recent returns an array")
        .iter()
        .map(|row| row["project"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(recent_projects, vec!["alpha"], "and so does recent");

    let context = run_json(leteo(&database).current_dir(&here).arg("context"));
    let rendered = context["context"].as_str().unwrap_or_default();
    assert!(
        !rendered.contains("beta"),
        "and so does the context block: {rendered}"
    );

    leteo(&database)
        .current_dir(&here)
        .arg("search")
        .arg("connection pool leaked")
        .arg("--project")
        .arg("beta")
        .arg("--all-projects")
        .assert()
        .failure();

    // An empty answer now has two possible reasons and must not name the wrong
    // one. "Try fewer, more distinctive words" sends somebody to rewrite a
    // question that was already right, when what happened is that they are
    // standing in a directory whose project holds nothing about it.
    let elsewhere = leteo(&database)
        .current_dir(&here)
        .arg("search")
        .arg("kafka consumer rebalance storm")
        .assert()
        .success();
    let said = String::from_utf8_lossy(&elsewhere.get_output().stderr).into_owned();
    assert!(
        // Capitalised, because the sentence is shared with the tool surface,
        // where it stands alone in a JSON field rather than after a prefix.
        said.contains("Nothing in alpha") && said.contains("--all-projects"),
        "the reason must be the directory, not the words: {said}"
    );
    assert!(
        !said.contains("matches words, not meanings"),
        "and not both reasons at once: {said}"
    );

    // And the two listings beside it, which printed `[]` and `""` and left
    // somebody to work out which of the two reasons it was.
    let empty_dir = temp.path().join("gamma");
    std::fs::create_dir_all(&empty_dir).unwrap();
    for command in ["recent", "context"] {
        let answer = leteo(&database)
            .current_dir(&empty_dir)
            .arg(command)
            .assert()
            .success();
        let said = String::from_utf8_lossy(&answer.get_output().stderr).into_owned();
        assert!(
            said.contains("Nothing in gamma") && said.contains("--all-projects"),
            "{command} has to say the store is not what is empty: {said:?}"
        );
    }
    // Naming the project is somebody who knows where they are looking.
    let answer = leteo(&database)
        .current_dir(&empty_dir)
        .arg("recent")
        .arg("--project")
        .arg("gamma")
        .assert()
        .success();
    assert!(
        String::from_utf8_lossy(&answer.get_output().stderr).is_empty(),
        "an explicit project asked about that project"
    );

    // A question nothing anywhere answers keeps the original hint.
    let nowhere = leteo(&database)
        .current_dir(&here)
        .arg("search")
        .arg("kubernetes helm chart")
        .assert()
        .success();
    let nada = String::from_utf8_lossy(&nowhere.get_output().stderr).into_owned();
    assert!(
        nada.contains("matches words, not meanings"),
        "a search that fails everywhere is about the words after all: {nada}"
    );
}

/// `leteo save` says when a type puts a memory out of a filter's reach.
///
/// The same sentence the tool answers with, on the channel a person reads.
/// Both doors, because the last three asymmetries between them were each a
/// defect: a hint written for one surface and never given on the other.
#[test]
fn a_command_line_save_says_when_the_type_is_one_no_filter_reaches() {
    let temp = tempfile::tempdir().expect("create CLI test directory");
    let database = temp.path().join("unfiled.db");

    let unfiled = leteo(&database)
        .arg("save")
        .arg("Delay-loading seven DLLs")
        .arg("the loader resolves them on first call")
        .arg("--project")
        .arg("alpha")
        .arg("--type")
        .arg("optimization")
        .assert()
        .success();
    let said = String::from_utf8_lossy(&unfiled.get_output().stderr).into_owned();
    assert!(
        said.contains("not one of the eight"),
        "a type outside the eight has to be said out loud: {said}"
    );
    // stdout stays what a script parses.
    let printed: Value = serde_json::from_slice(&unfiled.get_output().stdout)
        .expect("stdout is still the save's JSON");
    assert_eq!(printed["observation"]["type"], json!("optimization"));

    let filed = leteo(&database)
        .arg("save")
        .arg("A memory typed properly")
        .arg("with a body of its own")
        .arg("--project")
        .arg("alpha")
        .arg("--type")
        .arg("discovery")
        .assert()
        .success();
    assert!(
        String::from_utf8_lossy(&filed.get_output().stderr)
            .trim()
            .is_empty(),
        "one of the eight has nothing to add"
    );
}

/// `doctor --repair` puts back a full-text index that has gone empty.
///
/// The report could always see this break — `observation FTS row mismatch:
/// table=3769, fts=0` on a real store — and there was nothing anybody could do
/// about it. A store whose index has gone empty answers every search with
/// nothing and tells the caller its words did not match, which is advice no
/// rewording can act on; the only way back was to write to every row until the
/// triggers had caught up.
#[test]
fn doctor_repairs_a_full_text_index_that_has_gone_empty() {
    let temp = tempfile::tempdir().expect("create CLI test directory");
    let database = temp.path().join("repair.db");

    leteo(&database)
        .arg("save")
        .arg("The connection pool leaked")
        .arg("the pool was never returned to")
        .arg("--project")
        .arg("alpha")
        .assert()
        .success();

    let found = run_json(
        leteo(&database)
            .arg("search")
            .arg("connection pool")
            .arg("--project")
            .arg("alpha"),
    );
    assert_eq!(found.as_array().map(Vec::len), Some(1));

    // What a crash at the wrong moment leaves behind.
    let connection = rusqlite::Connection::open(&database).expect("open the store directly");
    connection
        .execute_batch(
            "INSERT INTO observations_fts(observations_fts) VALUES('delete-all');
             INSERT INTO observations_exact(observations_exact) VALUES('delete-all');",
        )
        .expect("empty the indexes");
    drop(connection);

    let broken = run_json(leteo(&database).arg("doctor"));
    assert_eq!(broken["healthy"], json!(false), "{broken}");
    let empty = run_json(
        leteo(&database)
            .arg("search")
            .arg("connection pool")
            .arg("--project")
            .arg("alpha"),
    );
    assert_eq!(
        empty.as_array().map(Vec::len),
        Some(0),
        "an empty index answers nothing: {empty}"
    );

    let repaired = run_json(leteo(&database).arg("doctor").arg("--repair"));
    assert_eq!(repaired["healthy"], json!(true), "{repaired}");
    let rebuilt = repaired["rebuilt"].as_array().expect("what was rebuilt");
    let observations = rebuilt
        .iter()
        .find(|entry| entry["index"] == json!("observations_fts"))
        .expect("the observation index is named");
    assert_eq!(observations["rows_before"], json!(0));
    assert_eq!(observations["rows_after"], json!(1));

    let again = run_json(
        leteo(&database)
            .arg("search")
            .arg("connection pool")
            .arg("--project")
            .arg("alpha"),
    );
    assert_eq!(
        again.as_array().map(Vec::len),
        Some(1),
        "and the memory is findable again: {again}"
    );

    // Repairing a store with nothing wrong changes nothing, and says so.
    let idempotent = run_json(leteo(&database).arg("doctor").arg("--repair"));
    for entry in idempotent["rebuilt"].as_array().expect("what was rebuilt") {
        assert_eq!(entry["rows_before"], entry["rows_after"], "{entry}");
    }
}

#[test]
fn the_search_command_says_which_limit_ended_the_list() {
    // The sentence existed on one of the two surfaces that can reach the cap.
    //
    // `mem_search` told an agent that twenty was the most a search returns;
    // `leteo search --limit 50` printed twenty rows and said nothing, which
    // reads as twenty matches. At `--limit 20` — which is what somebody who
    // wants everything types — both surfaces were silent, because `more` was
    // decided by a probe row the cap had already thrown away.
    let temp = tempfile::tempdir().expect("create CLI test directory");
    let database = temp.path().join("leteo-limits.db");
    run_json(
        leteo(&database)
            .arg("session-start")
            .arg("limits")
            .arg("--project")
            .arg("limits")
            .arg("--directory")
            .arg(temp.path()),
    );
    for n in 0..25 {
        run_json(
            leteo(&database)
                .arg("save")
                .arg("--session")
                .arg("limits")
                .arg("--type")
                .arg("discovery")
                .arg(format!("Widget {n}"))
                .arg("widget body"),
        );
    }

    let hint_for = |limit: &str| {
        let output = leteo(&database)
            .arg("search")
            .arg("--all-projects")
            .arg("--limit")
            .arg(limit)
            .arg("widget")
            .assert()
            .success()
            .get_output()
            .clone();
        String::from_utf8_lossy(&output.stderr).to_string()
    };

    // Below the cap the caller's own limit ended the list, and asking again for
    // more is the remedy that works.
    let own = hint_for("3");
    assert!(
        own.contains("More matched than were returned"),
        "a page the caller's limit ended says so: {own}"
    );

    // At the cap and above it, the store's maximum ended the list and a higher
    // limit is the one thing that cannot help.
    for limit in ["20", "50"] {
        let capped = hint_for(limit);
        assert!(
            capped.contains("the most a single search returns (20)"),
            "--limit {limit} is answered by the cap and has to say so: {capped}"
        );
        assert!(
            !capped.contains("higher limit"),
            "--limit {limit} must not advise the one thing that cannot work: {capped}"
        );
    }

    // And an answer that simply ran out explains nothing, however much was
    // asked for.
    let complete = hint_for("50");
    assert!(complete.contains("the most a single search"), "{complete}");
    let exhausted = leteo(&database)
        .arg("search")
        .arg("--all-projects")
        .arg("--limit")
        .arg("50")
        .arg("\"Widget 1\"")
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(
        String::from_utf8_lossy(&exhausted.stderr).trim().is_empty(),
        "a complete answer says nothing: {}",
        String::from_utf8_lossy(&exhausted.stderr)
    );
}

#[test]
fn a_memory_saved_from_the_terminal_records_the_question_too() {
    // Two doors into one table and only one of them attributed.
    //
    // `mem_save` reaches for the session's last prompt, and then for the
    // project's when the save named no session — sixty lines arguing which of
    // those is safe. `leteo save` wrote `None` with nothing said about why, so
    // the same memory recorded the question it answered or did not depending on
    // which door it came through.
    let temp = tempfile::tempdir().expect("create CLI test directory");
    let database = temp.path().join("leteo-attribution.db");
    run_json(
        leteo(&database)
            .arg("session-start")
            .arg("chat")
            .arg("--project")
            .arg("attribution")
            .arg("--directory")
            .arg(temp.path()),
    );
    let prompt = run_json(
        leteo(&database)
            .arg("prompt")
            .arg("--session")
            .arg("chat")
            .arg("why does the clock start from the memory"),
    );
    let asked = prompt["sync_id"]
        .as_str()
        .expect("the prompt has a sync id");

    // Named session: the question is that conversation's last one.
    let saved = run_json(
        leteo(&database)
            .arg("save")
            .arg("--session")
            .arg("chat")
            .arg("--type")
            .arg("decision")
            .arg("The clock starts from the memory")
            .arg("because that is when it was decided"),
    );
    assert_eq!(
        saved["observation"]["prompt_sync_id"].as_str(),
        Some(asked),
        "a save into a conversation records that conversation's question: {saved}"
    );

    // And a save that names no session lands in the per-project bucket, which
    // prompts are never written to. The project's own last question answers it,
    // inside the window — the case that made this worth having, since it is
    // where most memories go.
    let sessionless = run_json(
        leteo(&database)
            .arg("save")
            .arg("--project")
            .arg("attribution")
            .arg("--type")
            .arg("discovery")
            .arg("Saved with no session at all")
            .arg("and still about the same question"),
    );
    assert_eq!(
        sessionless["observation"]["prompt_sync_id"].as_str(),
        Some(asked),
        "a sessionless save takes the project's last question: {sessionless}"
    );

    // A session with no questions in it has none to give, and that is the
    // honest answer rather than the nearest one: the memory records no
    // question instead of the wrong conversation's.
    run_json(
        leteo(&database)
            .arg("session-start")
            .arg("silent")
            .arg("--project")
            .arg("elsewhere")
            .arg("--directory")
            .arg(temp.path()),
    );
    let unlinked = run_json(
        leteo(&database)
            .arg("save")
            .arg("--session")
            .arg("silent")
            .arg("--type")
            .arg("discovery")
            .arg("Nothing was asked here")
            .arg("so nothing is recorded"),
    );
    assert!(
        unlinked["observation"]["prompt_sync_id"].is_null(),
        "a conversation with no questions attributes nothing: {unlinked}"
    );
}

#[test]
fn every_surface_that_opens_a_context_names_the_size_that_was_configured() {
    // Three surfaces build this same context and one of them was not asked.
    //
    // The session-start hook and `mem_context` both read `context_size`;
    // `leteo context` used a constant twenty. The default is fifty, so on an
    // untouched installation a person at a terminal saw 40% of what their agent
    // was handed — and `leteo setup --context deep` moved two of the three.
    // Twenty is Slim's number, which is why it went unseen: the two agreed
    // exactly when somebody had chosen the smallest size.
    //
    // Driven by the sizes rather than by the constant, so the guard says what
    // the promise is: whatever is configured is what every surface shows.
    let temp = tempfile::tempdir().expect("create CLI test directory");
    let database = temp.path().join("leteo-context-size.db");
    run_json(
        leteo(&database)
            .arg("session-start")
            .arg("chat")
            .arg("--project")
            .arg("sizes")
            .arg("--directory")
            .arg(temp.path()),
    );
    // More than the largest size names, so no size is answered by the store
    // running out.
    for n in 0..90 {
        run_json(
            leteo(&database)
                .arg("save")
                .arg("--session")
                .arg("chat")
                .arg("--type")
                .arg("discovery")
                .arg(format!("Widget {n}"))
                .arg(format!("body of widget {n}")),
        );
    }

    let memories = |text: &str| text.matches("\n- #").count();
    let from_the_command = || {
        let json = run_json(
            leteo(&database)
                .arg("context")
                .arg("--project")
                .arg("sizes"),
        );
        memories(json["context"].as_str().expect("context is a string"))
    };
    let from_the_hook = || {
        let output = leteo(&database)
            .arg("hook")
            .arg("session-start")
            .write_stdin(
                serde_json::json!({
                    "session_id": "chat",
                    "cwd": temp.path().to_string_lossy(),
                    "source": "startup",
                })
                .to_string(),
            )
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let reply: Value = serde_json::from_slice(&output).expect("the hook replies with JSON");
        memories(
            reply["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .expect("the opening block is a string"),
        )
    };

    // Unset is the default, and the default is fifty rather than the twenty
    // that used to be written out here.
    assert_eq!(
        from_the_hook(),
        50,
        "an untouched installation opens at full"
    );
    assert_eq!(
        from_the_command(),
        from_the_hook(),
        "the terminal and the agent are shown the same store"
    );

    for (size, named) in [("slim", 20), ("full", 50), ("deep", 80)] {
        std::fs::write(
            temp.path().join("settings.json"),
            format!("{{\"context_size\":\"{size}\"}}"),
        )
        .expect("write the setting beside the database");
        assert_eq!(
            from_the_command(),
            named,
            "`leteo context` under {size} names what {size} means"
        );
        assert_eq!(
            from_the_hook(),
            named,
            "and so does the block a session opens with"
        );
    }

    // An explicit limit is still the caller's own, whatever is configured.
    let json = run_json(
        leteo(&database)
            .arg("context")
            .arg("--project")
            .arg("sizes")
            .arg("--limit")
            .arg("7"),
    );
    assert_eq!(
        memories(json["context"].as_str().expect("context is a string")),
        7,
        "--limit outranks the setting"
    );
}
