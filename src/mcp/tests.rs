use std::collections::BTreeSet;

use super::*;

#[test]
fn every_reply_satisfies_the_output_schema_the_tool_declares() {
    // A tool that answers with less than its own schema demands is a tool
    // that fails on any client which validates — OpenCode rejected every
    // `mem_search` and `mem_context` with `data must have required property
    // 'project_path'`. The cause is a Rust distinction that vanishes on the
    // wire: `#[serde(skip_serializing_if = "String::is_empty")]` omits the
    // field while `schemars`, seeing a plain `String`, marks it required.
    // Only `Option` says "may be absent" to both halves.
    let temp = tempfile::tempdir().unwrap();
    let mut store =
        Store::open(crate::store::StoreConfig::new(temp.path().join("mcp.db"))).unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    store
        .add_observation(crate::memory::model::AddObservation {
            session_id: "s1".to_owned(),
            kind: "decision".to_owned(),
            title: "Chose SQLite".to_owned(),
            content: "one writer, many readers".to_owned(),
            tool_name: None,
            project: Some("leteo".to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            prompt_sync_id: None,
        })
        .unwrap();
    for index in 0..4 {
        store
            .add_observation(crate::memory::model::AddObservation {
                session_id: "s1".to_owned(),
                kind: "discovery".to_owned(),
                title: format!("A neighbour on the timeline {index}"),
                content: format!("with a body of its own {index}"),
                tool_name: Some("probe".to_owned()),
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
    }
    store
        .add_prompt(crate::memory::model::AddPrompt {
            session_id: "s1".to_owned(),
            content: "why sqlite?".to_owned(),
            project: None,
        })
        .unwrap();

    let server = LeteoMcpServer::with_options(Arc::new(Mutex::new(store)), McpOptions::default());
    let schemas: std::collections::HashMap<String, serde_json::Value> = server
        .router
        .list_all()
        .into_iter()
        .filter_map(|tool| {
            let schema = tool.output_schema.as_ref()?;
            Some((
                tool.name.to_string(),
                serde_json::Value::Object((**schema).clone()),
            ))
        })
        .collect();

    let search = |project: Option<&str>, all_projects: bool| {
        let Json(output) = server
            .mem_search(Parameters(SearchParams {
                query: "sqlite".to_owned(),
                kind: None,
                project: project.map(str::to_owned),
                all_projects,
                scope: None,
                limit: None,
                match_mode: MatchMode::All,
            }))
            .expect("the search succeeds");
        serde_json::to_value(output).unwrap()
    };
    let call = |value: serde_json::Value| value;
    let saved: i64 = 1;
    let replies = [
        ("mem_search", search(Some("leteo"), false)),
        ("mem_search", search(None, true)),
        (
            "mem_context",
            call(
                serde_json::to_value(
                    server
                        .mem_context(Parameters(
                            serde_json::from_value(json!({ "project": "leteo" })).unwrap(),
                        ))
                        .expect("the context succeeds")
                        .0,
                )
                .unwrap(),
            ),
        ),
        (
            "mem_get_observation",
            call(
                serde_json::to_value(
                    server
                        .mem_get_observation(Parameters(
                            serde_json::from_value(json!({ "id": saved })).unwrap(),
                        ))
                        .expect("the read succeeds")
                        .0,
                )
                .unwrap(),
            ),
        ),
        (
            "mem_timeline",
            call(
                serde_json::to_value(
                    server
                        .mem_timeline(Parameters(
                            serde_json::from_value(json!({ "observation_id": saved })).unwrap(),
                        ))
                        .expect("the timeline succeeds")
                        .0,
                )
                .unwrap(),
            ),
        ),
    ];

    for (tool, reply) in replies {
        let schema = &schemas[tool];
        let mut faults = Vec::new();
        check_against_schema(&reply, schema, schema, tool, &mut faults);
        assert!(
            faults.is_empty(),
            "{tool}: {faults:?}
{reply}"
        );
    }
}

fn check_against_schema(
    value: &serde_json::Value,
    schema: &serde_json::Value,
    root: &serde_json::Value,
    path: &str,
    faults: &mut Vec<String>,
) {
    if let Some(reference) = schema.get("$ref").and_then(serde_json::Value::as_str) {
        let name = reference.trim_start_matches("#/$defs/");
        if let Some(target) = root.get("$defs").and_then(|defs| defs.get(name)) {
            check_against_schema(value, target, root, path, faults);
        }
        return;
    }
    let kinds: Vec<&str> = match schema.get("type") {
        Some(serde_json::Value::String(one)) => vec![one.as_str()],
        Some(serde_json::Value::Array(many)) => {
            many.iter().filter_map(serde_json::Value::as_str).collect()
        }
        _ => Vec::new(),
    };
    if !kinds.is_empty() {
        let actual = match value {
            serde_json::Value::Null => "null",
            serde_json::Value::Bool(_) => "boolean",
            serde_json::Value::Number(number) => {
                if number.is_f64() {
                    "number"
                } else {
                    "integer"
                }
            }
            serde_json::Value::String(_) => "string",
            serde_json::Value::Array(_) => "array",
            serde_json::Value::Object(_) => "object",
        };
        let fits = kinds.contains(&actual) || (actual == "integer" && kinds.contains(&"number"));
        if !fits {
            faults.push(format!("{path} is {actual}, declared {kinds:?}"));
            return;
        }
    }
    if value.is_null() {
        return;
    }
    if let Some(object) = value.as_object() {
        for key in schema
            .get("required")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
        {
            if !object.contains_key(key) {
                faults.push(format!("{path} omitted required {key}"));
            }
        }
        if let Some(properties) = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
        {
            for (key, sub) in object {
                if let Some(declared) = properties.get(key) {
                    check_against_schema(sub, declared, root, &format!("{path}.{key}"), faults);
                }
            }
        }
    }
    if let (Some(list), Some(items)) = (value.as_array(), schema.get("items")) {
        for (index, entry) in list.iter().enumerate() {
            check_against_schema(entry, items, root, &format!("{path}[{index}]"), faults);
        }
    }
}

#[test]
fn tool_schemas_carry_no_format_json_schema_has_never_defined() {
    let server = LeteoMcpServer::with_options(
        Arc::new(Mutex::new(
            Store::open(crate::store::StoreConfig::new(
                tempfile::tempdir().unwrap().path().join("mcp.db"),
            ))
            .unwrap(),
        )),
        McpOptions::default(),
    );

    let mut offenders = Vec::new();
    for tool in server.router.list_all() {
        let schemas = [
            ("input", Some(tool.input_schema.clone())),
            ("output", tool.output_schema.clone()),
        ];
        for (half, schema) in schemas {
            let Some(schema) = schema else { continue };
            let schema = serde_json::Value::Object((*schema).clone());
            let mut found = Vec::new();
            collect_formats(&schema, &mut found);
            for format in found {
                if RUST_NUMERIC_FORMATS.contains(&format.as_str()) {
                    offenders.push(format!("{} {half}: {format}", tool.name));
                }
            }
        }
    }
    assert!(offenders.is_empty(), "{offenders:?}");

    let search = server
        .router
        .list_all()
        .into_iter()
        .find(|tool| tool.name == "mem_search")
        .expect("mem_search is exposed");
    let limit = &search.input_schema["properties"]["limit"];
    assert!(
        limit["minimum"]
            .as_i64()
            .is_some_and(|minimum| minimum >= 0),
        "{limit}"
    );
    assert!(
        limit["type"]
            .as_array()
            .is_some_and(|types| types.contains(&serde_json::json!("integer"))),
        "{limit}"
    );
}

fn collect_formats(value: &serde_json::Value, found: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(format) = map.get("format").and_then(serde_json::Value::as_str) {
                found.push(format.to_owned());
            }
            for nested in map.values() {
                collect_formats(nested, found);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_formats(item, found);
            }
        }
        _ => {}
    }
}

#[test]
fn exposes_exactly_twenty_three_tools_with_output_schemas() {
    let tools = LeteoMcpServer::router().list_all();
    let names: BTreeSet<_> = tools.iter().map(|tool| tool.name.as_ref()).collect();

    assert_eq!(tools.len(), 23);
    assert_eq!(
        names,
        BTreeSet::from([
            "mem_capture_passive",
            "mem_compare",
            "mem_consolidate",
            "mem_context",
            "mem_current_project",
            "mem_delete",
            "mem_doctor",
            "mem_get_observation",
            "mem_judge",
            "mem_merge_projects",
            "mem_pin",
            "mem_review",
            "mem_save",
            "mem_save_prompt",
            "mem_search",
            "mem_session_end",
            "mem_session_start",
            "mem_session_summary",
            "mem_stats",
            "mem_suggest_topic_key",
            "mem_timeline",
            "mem_unpin",
            "mem_update",
        ])
    );
    assert!(tools.iter().all(|tool| tool.output_schema.is_some()));
}

#[test]
fn initialize_metadata_identifies_leteo_package() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(crate::store::StoreConfig::new(
        temp.path().join("metadata.db"),
    ))
    .unwrap();
    let server = LeteoMcpServer::with_options(Arc::new(Mutex::new(store)), McpOptions::default());

    let info = ServerHandler::get_info(&server);

    assert_eq!(info.server_info.name, "leteo");
    assert_eq!(info.server_info.title.as_deref(), Some("Leteo"));
    assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
    assert!(info.capabilities.tools.is_some());
    let instructions = info.instructions.expect("server instructions");
    assert!(
        instructions
            .starts_with("Local-first persistent memory tools backed by the Leteo SQLite store.")
    );
    assert!(instructions.contains("ambiguous_project"));
    assert!(instructions.contains(SOURCE_USER_SELECTED_AFTER_AMBIGUOUS_PROJECT));
    assert!(instructions.contains("unknown_project"));
}

#[test]
fn applies_tool_defaults() {
    let save: SaveParams = serde_json::from_value(json!({
        "title": "title",
        "content": "content"
    }))
    .unwrap();
    let search: SearchParams = serde_json::from_value(json!({ "query": "sqlite" })).unwrap();
    let review: ReviewParams = serde_json::from_value(json!({ "action": "list" })).unwrap();
    let delete: DeleteParams =
        serde_json::from_value(json!({ "id": 1, "expected_project": "leteo" })).unwrap();
    let timeline: TimelineParams = serde_json::from_value(json!({ "observation_id": 1 })).unwrap();
    let capture: CapturePassiveParams = serde_json::from_value(json!({
        "content": "## Key Learnings:\n- Prefer transactions"
    }))
    .unwrap();

    assert_eq!(save.session_id, None);
    assert_eq!(save.content.as_deref(), Some("content"));
    assert_eq!(save.kind, "manual");
    assert_eq!(save.scope, "project");
    assert_eq!(search.match_mode, MatchMode::All);
    assert!(!search.all_projects);
    assert_eq!(SearchMode::from(MatchMode::Any), SearchMode::Any);
    assert_eq!(review.limit, 10);
    assert!(!delete.hard_delete);
    assert_eq!((timeline.before, timeline.after), (5, 5));
    assert_eq!(capture.source, "mcp-passive");
    assert_eq!(manual_session_id("leteo"), "manual-save-leteo");
}

fn test_server(options: McpOptions) -> (tempfile::TempDir, LeteoMcpServer) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(crate::store::StoreConfig::new(temp.path().join("mcp.db"))).unwrap();
    let server = LeteoMcpServer::with_options(Arc::new(Mutex::new(store)), options);
    (temp, server)
}

fn ambiguous_detection() -> ProjectDetection {
    ProjectDetection {
        project: String::new(),
        source: crate::project::SOURCE_AMBIGUOUS.to_owned(),
        path: "C:/workspace".to_owned(),
        available_projects: vec!["alpha".to_owned(), "beta".to_owned()],
        warning: None,
        error_hint: Some("multiple repositories found".to_owned()),
    }
}

fn error_payload(result: &CallToolResult) -> serde_json::Value {
    result
        .structured_content
        .clone()
        .expect("structured error payload")
}

#[test]
fn listings_preview_the_body_and_only_mem_get_observation_returns_it_whole() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store
            .create_session("preview", "leteo", "C:/workspace")
            .unwrap();
    }

    let long = format!("needle {}", "a lot of body text. ".repeat(200))
        .trim()
        .to_owned();
    assert!(long.len() > PREVIEW_BYTES * 4);
    let saved = server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "preview",
                "title": "A memory with a long body",
                "content": long,
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    let id = saved.observation.id;

    let searched = server
        .mem_search(Parameters(
            serde_json::from_value(json!({ "query": "needle", "project": "leteo" })).unwrap(),
        ))
        .unwrap()
        .0;
    let hit = &searched.results[0].observation;
    assert!(hit.content_truncated);
    assert!(hit.content.len() < PREVIEW_BYTES + 32);
    assert!(hit.content.starts_with("needle"));
    assert!(hit.content.ends_with("... [truncated]"));
    assert_eq!(hit.title, "A memory with a long body");

    let context = server
        .mem_context(Parameters(
            serde_json::from_value(json!({ "project": "leteo" })).unwrap(),
        ))
        .unwrap()
        .0;
    assert!(context.observations[0].content_truncated);
    assert!(context.observations[0].content.len() < PREVIEW_BYTES + 32);

    let whole = server
        .mem_get_observation(Parameters(
            serde_json::from_value(json!({ "id": id })).unwrap(),
        ))
        .unwrap()
        .0;
    assert!(!whole.observation.content_truncated);
    assert_eq!(whole.observation.content, long);

    server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "preview",
                "title": "A short one",
                "content": "needle, briefly",
            }))
            .unwrap(),
        ))
        .unwrap();
    let searched = server
        .mem_search(Parameters(
            serde_json::from_value(json!({
                "query": "briefly", "project": "leteo"
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    assert!(!searched.results[0].observation.content_truncated);
    let payload = serde_json::to_value(&searched).unwrap();
    assert!(payload["results"][0].get("content_truncated").is_none());
}

#[test]
fn every_listing_previews_not_just_search_and_context() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store
            .create_session("wide", "leteo", "C:/workspace")
            .unwrap();
    }
    let long = "a lot of body text. ".repeat(200).trim().to_owned();
    let mut ids = Vec::new();
    for n in 0..3 {
        let saved = server
            .mem_save(Parameters(
                serde_json::from_value(json!({
                    "session_id": "wide",
                    "title": format!("A memory with a long body {n}"),
                    "content": long,
                }))
                .unwrap(),
            ))
            .unwrap()
            .0;
        assert!(saved.observation.content_truncated);
        ids.push(saved.observation.id);
    }

    let timeline = server
        .mem_timeline(Parameters(
            serde_json::from_value(json!({ "observation_id": ids[1] })).unwrap(),
        ))
        .unwrap()
        .0;
    assert!(timeline.focus.content_truncated);
    assert!(timeline.focus.content.len() < PREVIEW_BYTES + 32);
    let neighbours = timeline.before.iter().chain(timeline.after.iter());
    assert!(neighbours.clone().count() >= 2);
    for entry in neighbours {
        assert!(entry.content_truncated);
        assert!(entry.content.len() < PREVIEW_BYTES + 32);
    }

    let stored = server
        .lock_store()
        .unwrap()
        .get_observation(ids[0])
        .unwrap();
    let review = ReviewOutput::listing(vec![stored], &Default::default(), 1);
    assert_eq!(review.observations.len(), 1);
    assert!(review.observations[0].content_truncated);
}

#[test]
fn tool_profiles_select_the_registered_tools() {
    assert_eq!(resolve_tools("").unwrap(), None);
    assert_eq!(resolve_tools(" all ").unwrap(), None);
    assert_eq!(resolve_tools("agent,all").unwrap(), None);
    assert_eq!(
        resolve_tools("mem_save, mem_search").unwrap(),
        Some(BTreeSet::from([
            "mem_save".to_owned(),
            "mem_search".to_owned()
        ]))
    );
    assert_eq!(
        resolve_tools("agent").unwrap().map(|tools| tools.len()),
        Some(PROFILE_AGENT.len())
    );
    assert_eq!(
        resolve_tools("agent,admin")
            .unwrap()
            .map(|tools| tools.len()),
        Some(PROFILE_AGENT.len() + PROFILE_ADMIN.len())
    );

    for wrong in [
        "agnet",
        "AGENT",
        "no_existe",
        "agent,no_existe",
        "mem_serch",
    ] {
        let refusal =
            resolve_tools(wrong).expect_err(&format!("{wrong} is not a profile and not a tool"));
        assert!(
            refusal.contains("unknown tool or profile")
                && refusal.contains("agent, admin and all")
                && refusal.contains("mem_search"),
            "the refusal has to name what there is: {refusal}"
        );
    }
    assert_eq!(resolve_tools("  ,  ,  ").unwrap(), None);

    let (_temp, agent) = test_server(McpOptions {
        tools: Some("agent".to_owned()),
        ..McpOptions::default()
    });
    let names: BTreeSet<_> = agent
        .router
        .list_all()
        .iter()
        .map(|tool| tool.name.to_string())
        .collect();
    assert_eq!(names.len(), PROFILE_AGENT.len());
    assert!(names.contains("mem_save"));
    assert!(!names.contains("mem_delete"));

    let (_temp, admin) = test_server(McpOptions {
        tools: Some("admin,mem_search".to_owned()),
        ..McpOptions::default()
    });
    let names: BTreeSet<_> = admin
        .router
        .list_all()
        .iter()
        .map(|tool| tool.name.to_string())
        .collect();
    assert_eq!(names.len(), PROFILE_ADMIN.len() + 1);
    assert!(names.contains("mem_delete"));
    assert!(names.contains("mem_search"));
    assert!(!names.contains("mem_save"));
}

#[test]
fn every_tool_declares_behavior_annotations() {
    let tools = LeteoMcpServer::router().list_all();
    let read_only = BTreeSet::from([
        "mem_context",
        "mem_current_project",
        "mem_doctor",
        "mem_get_observation",
        "mem_search",
        "mem_stats",
        "mem_suggest_topic_key",
        "mem_timeline",
    ]);
    let destructive = BTreeSet::from([
        "mem_consolidate",
        "mem_delete",
        "mem_merge_projects",
        "mem_save",
        "mem_update",
    ]);

    for tool in &tools {
        let annotations = tool
            .annotations
            .as_ref()
            .unwrap_or_else(|| panic!("{} declares annotations", tool.name));
        assert!(
            annotations.title.is_some(),
            "{} declares a title",
            tool.name
        );
        assert_eq!(annotations.open_world_hint, Some(false), "{}", tool.name);
        assert_eq!(
            annotations.read_only_hint,
            Some(read_only.contains(tool.name.as_ref())),
            "{}",
            tool.name
        );
        assert_eq!(
            annotations.destructive_hint,
            Some(destructive.contains(tool.name.as_ref())),
            "{}",
            tool.name
        );
    }
}

#[test]
fn explicit_projects_must_be_backed_by_known_context() {
    let (_temp, server) = test_server(McpOptions::default());
    let mut store = server.lock_store().unwrap();
    // Recorded in another directory on purpose: a session under this project in
    // *this* directory would make the directory drift, and the point of this
    // test is the known-project path, not the drift gate that has its own test
    // below.
    store
        .create_session("known", "known-project", "C:/elsewhere")
        .unwrap();
    let detection = ProjectDetection {
        project: "leteo".to_owned(),
        source: crate::project::SOURCE_GIT_ROOT.to_owned(),
        path: "C:/workspace".to_owned(),
        available_projects: Vec::new(),
        warning: None,
        error_hint: None,
    };

    assert_eq!(
        server
            .resolve_write_project(&store, None, &detection, ProjectChoice::default())
            .unwrap(),
        (
            "leteo".to_owned(),
            crate::project::SOURCE_GIT_ROOT.to_owned()
        )
    );
    assert_eq!(
        server
            .resolve_write_project(
                &store,
                Some("LETEO".to_owned()),
                &detection,
                ProjectChoice::default()
            )
            .unwrap(),
        (
            "leteo".to_owned(),
            crate::project::SOURCE_GIT_ROOT.to_owned()
        )
    );
    assert_eq!(
        server
            .resolve_write_project(
                &store,
                Some("known-project".to_owned()),
                &detection,
                ProjectChoice::default()
            )
            .unwrap(),
        ("known-project".to_owned(), SOURCE_KNOWN_PROJECT.to_owned())
    );

    let error = server
        .resolve_write_project(
            &store,
            Some("invented".to_owned()),
            &detection,
            ProjectChoice::default(),
        )
        .expect_err("invented projects are refused");
    let payload = error_payload(&error);
    assert_eq!(payload["error"]["code"], "unknown_project");
    assert_eq!(payload["detected_project"], "leteo");

    let error = server
        .resolve_write_project(
            &store,
            Some("   ".to_owned()),
            &detection,
            ProjectChoice::default(),
        )
        .expect_err("blank projects are refused");
    assert_eq!(error_payload(&error)["error"]["code"], "invalid_project");
}

/// A directory whose sessions were recorded under another project does not
/// silently take the name detection now resolves to.
///
/// Adding a remote to a repository named by its directory, renaming the remote,
/// or pointing it at a fork changes what `origin` says, and Leteo used to file
/// the next memory under the new name with nothing said — the earlier memories
/// stayed under the old one, and the two halves of the project could no longer
/// see each other. The ambiguity the directory already knows how to report is
/// reused: the same error code, the same candidate list, the same recovery
/// token. Only the silent pick is refused; a caller that names a side has made
/// the choice the ambiguity was asking for and resolves it on its own, because
/// the tools that share this path cannot all send a token — `mem_update` has
/// neither a reason nor a token, and `mem_capture_passive` has no project at
/// all. That half is held by `a_drifted_directory_lets_mem_update_move_a_memory`
/// and `a_passive_capture_in_a_drifted_directory_keeps_the_project_whole`.
#[test]
fn a_remote_that_changed_makes_the_write_ask_which_project() {
    let (_temp, server) = test_server(McpOptions::default());
    let mut store = server.lock_store().unwrap();
    store
        .create_session("old", "old-remote", "C:/repo")
        .unwrap();
    let detection = ProjectDetection {
        project: "new-remote".to_owned(),
        source: crate::project::SOURCE_GIT_REMOTE.to_owned(),
        path: "C:/repo".to_owned(),
        available_projects: Vec::new(),
        warning: None,
        error_hint: None,
    };

    let error = server
        .resolve_write_project(&store, None, &detection, ProjectChoice::default())
        .expect_err("a silent pick would split the project in two");
    let payload = error_payload(&error);
    assert_eq!(payload["error"]["code"], "ambiguous_project");
    assert_eq!(
        payload["available_projects"],
        json!(["old-remote", "new-remote"]),
        "both names are offered: the recorded one first, then what it resolves to now"
    );
    assert!(
        payload["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("projects consolidate")),
        "and the message points at the command that folds them: {}",
        payload["error"]["message"]
    );
    assert!(
        payload["recovery_token"]
            .as_str()
            .is_some_and(|token| !token.is_empty()),
        "the refusal still carries a token, for the tools that can send one: {:?}",
        payload["recovery_token"]
    );

    // Naming either side of the drift resolves it, with no reason and no token:
    // that is the only shape `mem_update` can send, and it must not be left
    // holding an error it cannot clear. The recorded side is a project the store
    // already knows; the detected side is simply detection's own answer.
    assert_eq!(
        server
            .resolve_write_project(
                &store,
                Some("old-remote".to_owned()),
                &detection,
                ProjectChoice::default()
            )
            .unwrap(),
        ("old-remote".to_owned(), SOURCE_KNOWN_PROJECT.to_owned())
    );
    assert_eq!(
        server
            .resolve_write_project(
                &store,
                Some("new-remote".to_owned()),
                &detection,
                ProjectChoice::default()
            )
            .unwrap(),
        (
            "new-remote".to_owned(),
            crate::project::SOURCE_GIT_REMOTE.to_owned()
        )
    );
}

/// The session door is gated the same way the write path is.
///
/// `SERVER_INSTRUCTIONS` tells an agent to open a session with
/// `mem_session_start` before it saves anything, and the session owns its
/// project for the rest of the conversation: `create_session` is
/// `INSERT OR IGNORE`, so a later `mem_save` carrying that id returns the
/// session's project without looking at detection again. A gate only on the
/// write path is therefore bypassed by the door the brief names first, filing a
/// whole conversation under the new name in silence.
#[test]
fn a_remote_that_changed_makes_the_session_door_ask_too() {
    let (temp, server) = test_server(McpOptions::default());
    let workspace = temp.path().join("new-remote");
    std::fs::create_dir_all(&workspace).unwrap();
    // The raw temp spelling on purpose, not `canonicalize`: on macOS the temp
    // directory is `/var/...` while detection resolves it to `/private/var/...`,
    // and the session recorded here has to be found from that resolved form. The
    // spelling alone would not match, which is the symlink case `same_directory`
    // resolves.
    let directory = workspace.to_string_lossy().into_owned();
    {
        let mut store = server.lock_store().unwrap();
        store
            .create_session("old", "old-remote", &directory)
            .unwrap();
    }

    let error = server
        .mem_session_start(Parameters(
            serde_json::from_value(json!({
                "id": "s-drift",
                "directory": directory.clone(),
            }))
            .unwrap(),
        ))
        .err()
        .expect("the documented session door must not file under the new name in silence");
    let payload = error_payload(&error);
    assert_eq!(payload["error"]["code"], "ambiguous_project");
    assert_eq!(
        payload["available_projects"],
        json!(["old-remote", "new-remote"]),
        "both sides are named"
    );
    // `SessionStartParams` is `deny_unknown_fields` and has no
    // `project_choice_reason` or `recovery_token`, so the instructions have to
    // match what this door accepts. A token here would be a recovery no caller
    // could send.
    assert!(
        payload.get("recovery_token").is_none(),
        "the session door has no field for a token, so it must not demand one: {:?}",
        payload.get("recovery_token")
    );
    assert!(
        payload["recovery_instructions"]
            .as_str()
            .is_some_and(|instructions| instructions.contains("project=<choice>")
                && instructions.contains("no recovery_token")),
        "and it says the retry takes the project directly: {:?}",
        payload["recovery_instructions"]
    );

    // An explicit project is the agent having chosen, and it still wins.
    let chosen = server
        .mem_session_start(Parameters(
            serde_json::from_value(json!({
                "id": "s-drift",
                "project": "old-remote",
                "directory": directory,
            }))
            .unwrap(),
        ))
        .expect("an explicit project is the choice the gate is asking for");
    assert_eq!(chosen.0.session.project, "old-remote");
}

/// The process override wins over the drift gate, as it wins over detection.
///
/// The override is the operator's answer for this process, given before any
/// directory was seen. Asking it to replay a choice the operator already made
/// would be a prompt nobody can answer, and it is why the gate sits after the
/// override on the write path rather than before it.
#[test]
fn the_process_override_wins_over_the_drift_gate() {
    let (_temp, server) = test_server(McpOptions {
        default_project: Some("old-remote".to_owned()),
        ..McpOptions::default()
    });
    let mut store = server.lock_store().unwrap();
    store
        .create_session("old", "old-remote", "C:/repo")
        .unwrap();
    let detection = ProjectDetection {
        project: "new-remote".to_owned(),
        source: crate::project::SOURCE_GIT_REMOTE.to_owned(),
        path: "C:/repo".to_owned(),
        available_projects: Vec::new(),
        warning: None,
        error_hint: None,
    };

    // Naming the override, which is also a drift candidate, and taking the
    // silent path that would otherwise have used it: both are the override.
    for requested in [Some("old-remote".to_owned()), None] {
        assert_eq!(
            server
                .resolve_write_project(&store, requested, &detection, ProjectChoice::default())
                .unwrap(),
            (
                "old-remote".to_owned(),
                crate::project::SOURCE_PROCESS_OVERRIDE.to_owned()
            )
        );
    }
}

/// The same override wins on the session door.
#[test]
fn the_process_override_wins_over_the_drift_gate_on_the_session_door() {
    let (temp, server) = test_server(McpOptions {
        default_project: Some("old-remote".to_owned()),
        ..McpOptions::default()
    });
    let workspace = temp.path().join("new-remote");
    std::fs::create_dir_all(&workspace).unwrap();
    let directory = std::fs::canonicalize(&workspace)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    {
        let mut store = server.lock_store().unwrap();
        store
            .create_session("old", "old-remote", &directory)
            .unwrap();
    }

    let started = server
        .mem_session_start(Parameters(
            serde_json::from_value(json!({ "id": "s-override", "directory": directory })).unwrap(),
        ))
        .expect("the process override is the operator's answer, not a choice to replay");
    assert_eq!(started.0.session.project, "old-remote");
}

/// A directory whose sessions agree with detection is not asked about.
///
/// The interception is only for a drift; the ordinary case must pay no extra
/// prompt and gain no new fields, which is the half a fix like this loses when
/// the lookup is done and its emptiness is not checked.
#[test]
fn a_directory_whose_sessions_agree_is_not_asked_which_project() {
    let (_temp, server) = test_server(McpOptions::default());
    let mut store = server.lock_store().unwrap();
    store.create_session("here", "leteo", "C:/repo").unwrap();
    let detection = ProjectDetection {
        project: "leteo".to_owned(),
        source: crate::project::SOURCE_GIT_REMOTE.to_owned(),
        path: "C:/repo".to_owned(),
        available_projects: Vec::new(),
        warning: None,
        error_hint: None,
    };

    assert_eq!(
        server
            .resolve_write_project(&store, None, &detection, ProjectChoice::default())
            .unwrap(),
        (
            "leteo".to_owned(),
            crate::project::SOURCE_GIT_REMOTE.to_owned()
        ),
        "the recorded project is the detected one, so nothing is asked"
    );
}

/// `mem_update` can name a project but has no reason or token to send.
///
/// It shares `resolve_write_project` with `mem_save`, but `UpdateParams` is
/// `deny_unknown_fields` and carries neither `project_choice_reason` nor
/// `recovery_token`. When the write path held an explicit retry to the drift's
/// replay, moving a memory into either side of a drift returned an
/// `ambiguous_project` error whose token the tool could never send — an error no
/// caller could clear. The drift gate belongs on the silent pick; an explicit
/// project resolves on its own, and this drives the real tool to prove it.
#[test]
fn a_drifted_directory_lets_mem_update_move_a_memory() {
    let (_temp, server) = test_server(McpOptions::default());
    let detection = crate::project::detect_current_project();
    let detected = detection.project.clone();
    let recorded = "old-remote";
    assert_ne!(detected, recorded, "the fixture needs a real drift");

    let id = {
        let mut store = server.lock_store().unwrap();
        store
            .create_session("s1", &detected, &detection.path)
            .unwrap();
        // The same directory, already used under another name.
        store
            .create_session("old", recorded, &detection.path)
            .unwrap();
        store
            .add_observation(AddObservation {
                session_id: "s1".to_owned(),
                kind: "decision".to_owned(),
                title: "A memory to move".to_owned(),
                content: "the directory has drifted since this was written".to_owned(),
                tool_name: None,
                project: Some(detected.clone()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap()
            .observation
            .id
    };

    let moved = server
        .mem_update(Parameters(
            serde_json::from_value(json!({
                "id": id,
                "expected_project": detected,
                "project": recorded,
            }))
            .unwrap(),
        ))
        .expect("mem_update has no reason or token, so a drift must not ask it for one")
        .0;
    assert_eq!(moved.observation.project.as_deref(), Some(recorded));
}

/// Passive capture cannot name a project, so it must not be blocked by a drift.
///
/// `mem_capture_passive` takes no project and has nobody to prompt: an agent
/// ends with a Key Learnings block and the hook hands it over. Refusing would
/// drop the learnings, and taking the detected name would split the project the
/// directory's history is already under, so it files under the recorded project
/// instead.
#[test]
fn a_passive_capture_in_a_drifted_directory_keeps_the_project_whole() {
    let (_temp, server) = test_server(McpOptions::default());
    let detection = crate::project::detect_current_project();
    let detected = detection.project.clone();
    let recorded = "old-remote";
    assert_ne!(detected, recorded, "the fixture needs a real drift");

    {
        let mut store = server.lock_store().unwrap();
        store
            .create_session("old", recorded, &detection.path)
            .unwrap();
    }

    let captured = server
        .mem_capture_passive(Parameters(
            serde_json::from_value(json!({
                "content": "## Key Learnings\n- the drift is filed under the name the directory already used",
            }))
            .unwrap(),
        ))
        .expect("passive capture has no project to name and nobody to prompt")
        .0;
    assert_eq!(captured.project_context.project, recorded);
    assert_eq!(captured.saved, 1);
}

/// A capture that names an existing session files under that session's project.
///
/// `passive_capture_project` resolves the drift for a capture that opens a
/// fresh manual session, but an existing session already owns its project:
/// handing `write_session` the recorded name instead would make the two
/// disagree and refuse the capture with `session_project_mismatch`, dropping
/// the learnings. So a named session that exists must be left to
/// `write_session`, which returns the session's own project.
#[test]
fn a_passive_capture_naming_a_session_files_under_that_sessions_project() {
    let (_temp, server) = test_server(McpOptions::default());
    let detection = crate::project::detect_current_project();
    let detected = detection.project.clone();
    let recorded = "old-remote";
    assert_ne!(detected, recorded, "the fixture needs a real drift");

    {
        let mut store = server.lock_store().unwrap();
        // The drift: this directory's history is under another name.
        store
            .create_session("old", recorded, &detection.path)
            .unwrap();
        // The session the capture names, opened under the detected project.
        store
            .create_session("mine", &detected, &detection.path)
            .unwrap();
    }

    let captured = server
        .mem_capture_passive(Parameters(
            serde_json::from_value(json!({
                "content": "## Key Learnings\n- a named session owns its project",
                "session_id": "mine",
            }))
            .unwrap(),
        ))
        .expect("a session that exists owns its project, so the capture must not be refused")
        .0;
    assert_eq!(
        captured.project_context.project, detected,
        "the capture files under the session's project, not the recorded drift"
    );
    assert_eq!(captured.saved, 1);
}

/// `mem_session_start` is published as "create a session, or return it
/// unchanged if its identifier exists", and the drift gate must not stand in
/// front of that: a second start with the same id in a drifted directory
/// returns the session already opened rather than an ambiguity about a choice
/// already made.
#[test]
fn a_repeated_session_start_in_a_drifted_directory_returns_the_session_unchanged() {
    let (_temp, server) = test_server(McpOptions::default());
    let detection = crate::project::detect_current_project();
    let detected = detection.project.clone();
    let recorded = "old-remote";
    assert_ne!(detected, recorded, "the fixture needs a real drift");

    {
        let mut store = server.lock_store().unwrap();
        store
            .create_session("old", recorded, &detection.path)
            .unwrap();
        store
            .create_session("s-repeat", recorded, &detection.path)
            .unwrap();
    }

    let started = server
        .mem_session_start(Parameters(
            serde_json::from_value(json!({
                "id": "s-repeat",
                "directory": detection.path,
            }))
            .unwrap(),
        ))
        .expect("an existing session comes back unchanged, before the drift gate")
        .0;
    assert_eq!(started.session.id, "s-repeat");
    assert_eq!(
        started.session.project, recorded,
        "the session that already exists is returned, not re-resolved under a drift"
    );
}

#[test]
fn ambiguous_projects_require_a_replayed_recovery_token() {
    let (_temp, server) = test_server(McpOptions::default());
    let store = server.lock_store().unwrap();
    let detection = ambiguous_detection();

    let error = server
        .resolve_write_project(&store, None, &detection, ProjectChoice::default())
        .expect_err("ambiguous directories block writes");
    let payload = error_payload(&error);
    assert_eq!(payload["error"]["code"], "ambiguous_project");
    assert_eq!(payload["available_projects"], json!(["alpha", "beta"]));
    let token = payload["recovery_token"]
        .as_str()
        .expect("recovery token")
        .to_owned();
    assert!(!token.is_empty());

    let error = server
        .resolve_write_project(
            &store,
            Some("alpha".to_owned()),
            &detection,
            ProjectChoice::default(),
        )
        .expect_err("a bare choice is not enough");
    assert_eq!(error_payload(&error)["error"]["code"], "ambiguous_project");

    let error = server
        .resolve_write_project(
            &store,
            Some("alpha".to_owned()),
            &detection,
            ProjectChoice {
                reason: Some(SOURCE_USER_SELECTED_AFTER_AMBIGUOUS_PROJECT.to_owned()),
                recovery_token: None,
            },
        )
        .expect_err("the token is required");
    assert_eq!(
        error_payload(&error)["error"]["code"],
        "recovery_token_required"
    );

    let error = server
        .resolve_write_project(
            &store,
            Some("alpha".to_owned()),
            &detection,
            ProjectChoice {
                reason: Some(SOURCE_USER_SELECTED_AFTER_AMBIGUOUS_PROJECT.to_owned()),
                recovery_token: Some("rec-0000000000000000".to_owned()),
            },
        )
        .expect_err("unknown tokens are refused");
    assert_eq!(
        error_payload(&error)["error"]["code"],
        "invalid_recovery_token"
    );

    let error = server
        .resolve_write_project(
            &store,
            Some("gamma".to_owned()),
            &detection,
            ProjectChoice {
                reason: Some(SOURCE_USER_SELECTED_AFTER_AMBIGUOUS_PROJECT.to_owned()),
                recovery_token: Some(token.clone()),
            },
        )
        .expect_err("projects outside the candidate list are refused");
    assert_eq!(
        error_payload(&error)["error"]["code"],
        "invalid_project_choice"
    );

    assert_eq!(
        server
            .resolve_write_project(
                &store,
                Some("alpha".to_owned()),
                &detection,
                ProjectChoice {
                    reason: Some(SOURCE_USER_SELECTED_AFTER_AMBIGUOUS_PROJECT.to_owned()),
                    recovery_token: Some(token.clone()),
                }
            )
            .unwrap(),
        (
            "alpha".to_owned(),
            SOURCE_USER_SELECTED_AFTER_AMBIGUOUS_PROJECT.to_owned()
        )
    );
    let error = server
        .resolve_write_project(
            &store,
            Some("beta".to_owned()),
            &detection,
            ProjectChoice {
                reason: Some(SOURCE_USER_SELECTED_AFTER_AMBIGUOUS_PROJECT.to_owned()),
                recovery_token: Some(token),
            },
        )
        .expect_err("a redeemed token cannot switch projects");
    assert_eq!(
        error_payload(&error)["error"]["code"],
        "invalid_recovery_token"
    );
}

#[test]
fn recovery_tokens_expire_and_stay_bound_to_their_context() {
    let mut tokens = RecoveryTokens::default();
    let detection = ambiguous_detection();
    let token = tokens.issue(&detection);

    let mut moved = detection.clone();
    moved.path = "C:/elsewhere".to_owned();
    assert!(!tokens.redeem(&token, "alpha", &moved));

    let mut changed = detection.clone();
    changed.available_projects.push("gamma".to_owned());
    assert!(!tokens.redeem(&token, "alpha", &changed));

    assert!(tokens.redeem(&token, "alpha", &detection));
    assert!(tokens.redeem(&token, "alpha", &detection));
    assert!(!tokens.redeem(&token, "beta", &detection));

    tokens
        .entries
        .get_mut(&token)
        .expect("issued token")
        .expires_at = chrono::Utc::now() - chrono::TimeDelta::seconds(1);
    assert!(!tokens.redeem(&token, "alpha", &detection));
    assert!(tokens.entries.is_empty());
}

#[test]
fn the_process_override_resolves_writes_and_current_project() {
    let (_temp, server) = test_server(McpOptions {
        default_project: Some("Override--Project".to_owned()),
        ..McpOptions::default()
    });
    let detection = ambiguous_detection();
    {
        let store = server.lock_store().unwrap();
        assert_eq!(
            server
                .resolve_write_project(&store, None, &detection, ProjectChoice::default())
                .unwrap(),
            (
                "override-project".to_owned(),
                crate::project::SOURCE_PROCESS_OVERRIDE.to_owned()
            )
        );
    }

    let current = server
        .mem_current_project(Parameters(NoParams {}))
        .unwrap()
        .0;
    assert_eq!(current.project, "override-project");
    assert_eq!(
        current.project_source,
        crate::project::SOURCE_PROCESS_OVERRIDE.to_owned()
    );
}

#[test]
fn a_save_records_the_prompt_this_process_is_answering() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store
            .create_session("linked", "leteo", "C:/workspace")
            .unwrap();
    }

    let orphan = server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "linked",
                "title": "Saved before any prompt",
                "content": "no request preceded this",
            }))
            .unwrap(),
        ))
        .unwrap();
    assert_eq!(orphan.0.observation.prompt_sync_id, None);

    let prompt = server
        .mem_save_prompt(Parameters(
            serde_json::from_value(json!({
                "session_id": "linked",
                "content": "why is the login slow?",
            }))
            .unwrap(),
        ))
        .unwrap();
    let prompt_sync_id = prompt.0.prompt.sync_id;

    let linked = server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "linked",
                "title": "Fixed the slow login",
                "content": "root cause was an N+1 query",
            }))
            .unwrap(),
        ))
        .unwrap();
    assert_eq!(
        linked.0.observation.prompt_sync_id.as_deref(),
        Some(prompt_sync_id.as_str()),
        "a save should record the request it answered"
    );

    let automated = server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "linked",
                "title": "Pipeline artefact",
                "content": "written by a job, not by a request",
                "capture_prompt": false,
            }))
            .unwrap(),
        ))
        .unwrap();
    assert_eq!(automated.0.observation.prompt_sync_id, None);

    {
        let mut store = server.lock_store().unwrap();
        store
            .create_session("elsewhere", "leteo", "C:/workspace")
            .unwrap();
    }
    let other = server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "elsewhere",
                "title": "Unrelated work",
                "content": "another conversation entirely",
            }))
            .unwrap(),
        ))
        .unwrap();
    assert_eq!(
        other.0.observation.prompt_sync_id, None,
        "a prompt from one session must not attach to another"
    );
}

#[test]
fn every_project_scoped_response_reports_which_project_it_used() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store
            .create_session("envelope", "leteo", "C:/workspace")
            .unwrap();
    }

    let saved = server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "envelope",
                "title": "Envelope memory",
                "content": "the response says where this landed",
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    assert_eq!(saved.project_context.project, "leteo");
    assert_eq!(saved.project_context.project_source, SOURCE_SESSION_PROJECT);
    assert_eq!(
        saved.project_context.project_path.as_deref(),
        Some("C:/workspace")
    );

    let prompt = server
        .mem_save_prompt(Parameters(
            serde_json::from_value(json!({
                "session_id": "envelope",
                "content": "why did we do that?",
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    assert_eq!(
        prompt.project_context.project_source,
        SOURCE_SESSION_PROJECT
    );

    let searched = server
        .mem_search(Parameters(
            serde_json::from_value(json!({ "query": "envelope", "project": "Leteo" })).unwrap(),
        ))
        .unwrap()
        .0;
    assert_eq!(searched.project_context.project, "leteo");
    assert_eq!(searched.project_context.project_source, SOURCE_REQUEST);

    let all = server
        .mem_search(Parameters(
            serde_json::from_value(json!({ "query": "envelope", "all_projects": true })).unwrap(),
        ))
        .unwrap()
        .0;
    assert_eq!(all.project_context.project_source, SOURCE_ALL_PROJECTS);
    assert!(all.project_context.project.is_empty());

    let context = server
        .mem_context(Parameters(serde_json::from_value(json!({})).unwrap()))
        .unwrap()
        .0;
    let standing = server
        .mem_current_project(Parameters(serde_json::from_value(json!({})).unwrap()))
        .unwrap()
        .0;
    assert_eq!(
        context.project_context.project_source,
        standing.project_source
    );
    assert_eq!(context.project_context.project, standing.project);
    assert!(
        !context.project_context.project.is_empty(),
        "a read that names no project still narrows to one"
    );

    let everything = server
        .mem_context(Parameters(
            serde_json::from_value(json!({ "all_projects": true })).unwrap(),
        ))
        .unwrap()
        .0;
    assert_eq!(
        everything.project_context.project_source,
        SOURCE_ALL_PROJECTS
    );
    assert!(everything.project_context.project.is_empty());

    let payload = serde_json::to_value(&saved).unwrap();
    assert_eq!(payload["project"], "leteo");
    assert_eq!(payload["project_source"], SOURCE_SESSION_PROJECT);
    assert_eq!(payload["project_path"], "C:/workspace");
}

#[test]
fn the_override_is_reported_as_the_authority_on_reads() {
    let (_temp, server) = test_server(McpOptions {
        default_project: Some("Override--Project".to_owned()),
        ..McpOptions::default()
    });

    let context = server
        .mem_context(Parameters(serde_json::from_value(json!({})).unwrap()))
        .unwrap()
        .0;

    assert_eq!(context.project_context.project, "override-project");
    assert_eq!(
        context.project_context.project_source,
        crate::project::SOURCE_PROCESS_OVERRIDE
    );
}

#[test]
fn context_includes_recent_sessions_and_prompts() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store
            .create_session("context", "leteo", "C:/workspace")
            .unwrap();
        store
            .add_observation(AddObservation {
                session_id: "context".to_owned(),
                kind: "decision".to_owned(),
                title: "Context observation".to_owned(),
                content: "body".to_owned(),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
        store
            .add_prompt(AddPrompt {
                session_id: "context".to_owned(),
                content: "What did we decide?".to_owned(),
                project: Some("leteo".to_owned()),
            })
            .unwrap();
    }

    let params: ContextParams = serde_json::from_value(json!({ "project": "leteo" })).unwrap();
    assert_eq!((params.session_limit, params.prompt_limit), (5, 10));
    let context = server.mem_context(Parameters(params)).unwrap().0;

    assert_eq!(context.count, 1);
    assert_eq!(context.observations[0].title, "Context observation");
    assert_eq!(context.sessions.len(), 1);
    assert_eq!(context.sessions[0].id, "context");
    assert_eq!(context.sessions[0].observation_count, 1);
    assert_eq!(context.prompts.len(), 1);
    assert_eq!(context.prompts[0].content, "What did we decide?");
}

#[test]
fn returns_machine_readable_store_errors() {
    let result = store_error(StoreError::ObservationNotFound(42));
    let error = result.structured_content.unwrap();

    assert_eq!(error["error"]["code"], "observation_not_found");
    assert_eq!(error["error"]["message"], "observation not found: 42");
    assert_eq!(result.is_error, Some(true));
}

#[test]
fn a_callers_mistake_is_not_reported_as_a_store_failure() {
    let result = store_error(StoreError::InvalidParameter(
        "unknown check \"bogus\"".to_owned(),
    ));
    let error = result.structured_content.unwrap();
    assert_eq!(error["error"]["code"], "invalid_params");
    assert_eq!(error["error"]["message"], "unknown check \"bogus\"");

    let result = store_error(StoreError::PromptNotFound(7));
    let error = result.structured_content.unwrap();
    assert_eq!(error["error"]["code"], "prompt_not_found");
    assert_eq!(error["error"]["message"], "prompt not found: 7");
}

#[test]
fn reading_an_overturned_memory_in_full_still_says_it_was_overturned() {
    let (_temp, server) = test_server(McpOptions::default());
    let (older, newer) = {
        let mut store = server.lock_store().unwrap();
        store.create_session("s1", "leteo", "C:/workspace").unwrap();
        let memory = |title: &str| AddObservation {
            session_id: "s1".to_owned(),
            kind: "decision".to_owned(),
            title: title.to_owned(),
            content: format!("the body of {title}"),
            tool_name: None,
            project: Some("leteo".to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            prompt_sync_id: None,
        };
        let older = store
            .add_observation(memory("We indent with tabs"))
            .unwrap()
            .observation;
        let newer = store
            .add_observation(memory("We indent with spaces now"))
            .unwrap()
            .observation;
        let relation = store
            .save_relation(crate::memory::model::SaveRelationParams {
                sync_id: crate::memory::normalize::sync_id("rel"),
                source_id: newer.sync_id.clone(),
                target_id: older.sync_id.clone(),
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
        (older, newer)
    };

    let read = |id: i64| {
        let params: GetObservationParams = serde_json::from_value(json!({ "id": id })).unwrap();
        server.mem_get_observation(Parameters(params)).unwrap().0
    };

    let overturned = read(older.id);
    assert_eq!(overturned.observation.caveats.len(), 1);
    assert_eq!(overturned.observation.caveats[0].relation, "superseded_by");
    assert_eq!(overturned.observation.caveats[0].other_id, newer.id);
    assert_eq!(
        overturned.observation.caveats[0].other_title,
        "We indent with spaces now"
    );

    let rendered = serde_json::to_value(&overturned).unwrap();
    assert!(
        rendered["observation"].get("caveats").is_some(),
        "{rendered}"
    );

    let standing = read(newer.id);
    assert!(standing.observation.caveats.is_empty());
    let rendered = serde_json::to_value(&standing).unwrap();
    assert!(
        rendered["observation"].get("caveats").is_none(),
        "{rendered}"
    );
}

#[test]
fn a_project_with_as_many_pins_as_the_budget_still_hears_about_recent_work() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store.create_session("s1", "leteo", "C:/workspace").unwrap();
        let memory = |title: &str| AddObservation {
            session_id: "s1".to_owned(),
            kind: "decision".to_owned(),
            title: title.to_owned(),
            content: "body".to_owned(),
            tool_name: None,
            project: Some("leteo".to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            prompt_sync_id: None,
        };
        for index in 0..20 {
            let saved = store
                .add_observation(memory(&format!("Pinned {index}")))
                .unwrap()
                .observation;
            store.pin_observation(saved.id, None).unwrap();
        }
        for index in 0..20 {
            store
                .add_observation(memory(&format!("Recent {index}")))
                .unwrap();
        }
    }

    let params: ContextParams = serde_json::from_value(json!({ "project": "leteo" })).unwrap();
    let out = server.mem_context(Parameters(params)).unwrap().0;

    let named = |prefix: &str| {
        out.observations
            .iter()
            .filter(|observation| observation.title.starts_with(prefix))
            .count()
            + out
                .also_remembered
                .iter()
                .filter(|line| line.title.starts_with(prefix))
                .count()
    };
    assert_eq!(named("Pinned"), 20, "every pin is still listed");
    assert_eq!(
        named("Recent"),
        20,
        "and so is a budget's worth of recent work"
    );
    assert_eq!(out.count, 40);
}

#[test]
fn the_budget_still_bounds_the_recent_memories_it_governs() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store.create_session("s1", "leteo", "C:/workspace").unwrap();
        for index in 0..10 {
            store
                .add_observation(AddObservation {
                    session_id: "s1".to_owned(),
                    kind: "decision".to_owned(),
                    title: format!("Recent {index}"),
                    content: "body".to_owned(),
                    tool_name: None,
                    project: Some("leteo".to_owned()),
                    scope: "project".to_owned(),
                    topic_key: None,
                    prompt_sync_id: None,
                })
                .unwrap();
        }
    }

    let params: ContextParams =
        serde_json::from_value(json!({ "project": "leteo", "limit": 3 })).unwrap();
    let out = server.mem_context(Parameters(params)).unwrap().0;

    assert_eq!(
        out.count, 3,
        "with nothing pinned the budget is the whole answer"
    );
}

#[test]
fn every_tool_belongs_to_exactly_one_profile() {
    let (_temp, server) = test_server(McpOptions::default());
    let declared: BTreeSet<String> = server
        .router
        .list_all()
        .iter()
        .map(|tool| tool.name.to_string())
        .collect();
    let agent: BTreeSet<String> = PROFILE_AGENT
        .iter()
        .map(|tool| (*tool).to_owned())
        .collect();
    let admin: BTreeSet<String> = PROFILE_ADMIN
        .iter()
        .map(|tool| (*tool).to_owned())
        .collect();
    let profiled: BTreeSet<String> = agent.union(&admin).cloned().collect();

    let unreachable: Vec<&String> = declared.difference(&profiled).collect();
    assert!(
        unreachable.is_empty(),
        "no profile offers these, so only `--tools all` reaches them: {unreachable:?}"
    );
    let phantom: Vec<&String> = profiled.difference(&declared).collect();
    assert!(
        phantom.is_empty(),
        "these profiles name tools that do not exist: {phantom:?}"
    );
    let both: Vec<&String> = agent.intersection(&admin).collect();
    assert!(
        both.is_empty(),
        "a tool in both profiles makes the split meaningless: {both:?}"
    );
}

#[test]
fn an_empty_search_explains_itself_and_a_full_one_does_not() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store
            .create_session("hint", "leteo", "C:/workspace")
            .unwrap();
    }
    server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "hint",
                "title": "Line coverage measured after the split",
                "content": "The suite reached 77.1% of lines.",
            }))
            .unwrap(),
        ))
        .unwrap();

    let search = |query: &str| {
        server
            .mem_search(Parameters(
                serde_json::from_value(json!({ "query": query })).unwrap(),
            ))
            .unwrap()
            .0
    };

    let found = search("coverage");
    assert_eq!(found.count, 1);
    assert!(
        found.hint.is_none(),
        "a search that matched should not spend tokens on advice: {:?}",
        found.hint
    );

    let missed = search("cobertura");
    assert_eq!(missed.count, 0);
    let hint = missed.hint.expect("an empty search carries the hint");
    assert!(
        hint.contains("mem_context"),
        "the hint has to name the tool that needs no query: {hint}"
    );

    let widened = search("coverage measured cobertura");
    assert_eq!(widened.count, 1, "{widened:?}");
    assert!(
        widened.results[0].partial,
        "the row that half-matched says so"
    );
    let hint = widened
        .hint
        .expect("a widened search says it did not match every word");
    assert!(
        hint.contains("every word"),
        "the hint has to say what was relaxed: {hint}"
    );
}

/// A search that read a term as another word names every substitution.
///
/// The rows that come back match words other than the ones typed, so an agent
/// that does not read the substitution will take a typo's answer for an exact
/// one. Both surfaces share the sentence; this is the tool's half.
#[test]
fn a_search_names_every_term_it_read_as_another() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store
            .create_session("typo", "leteo", "C:/workspace")
            .unwrap();
    }
    server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "typo",
                "title": "Fixed connection pool exhaustion under load",
                "content": "one pgxpool.Pool is built in main and injected",
            }))
            .unwrap(),
        ))
        .unwrap();

    let search = |query: &str| {
        server
            .mem_search(Parameters(
                serde_json::from_value(json!({ "query": query })).unwrap(),
            ))
            .unwrap()
            .0
    };

    let found = search("conection pool exaustion");
    assert_eq!(found.count, 1, "{found:?}");
    let hint = found.hint.expect("a corrected search says so");
    assert!(
        hint.contains("\"connection\" instead of \"conection\""),
        "the hint has to name the first substitution: {hint}"
    );
    assert!(
        hint.contains("\"exhaustion\" instead of \"exaustion\""),
        "and the second: {hint}"
    );

    // A query the store answers exactly spends nothing on the sentence.
    let exact = search("connection pool exhaustion");
    assert_eq!(exact.count, 1, "{exact:?}");
    assert!(exact.hint.is_none(), "{:?}", exact.hint);
}

#[test]
fn mem_context_spends_its_budget_on_memories_the_way_the_session_opening_does() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store.create_session("s1", "leteo", "C:/workspace").unwrap();
        let mut save = |kind: &str, title: &str, scope: &str| {
            store
                .add_observation(AddObservation {
                    session_id: "s1".to_owned(),
                    kind: kind.to_owned(),
                    title: title.to_owned(),
                    content: format!("the body of {title}"),
                    tool_name: None,
                    project: Some("leteo".to_owned()),
                    scope: scope.to_owned(),
                    topic_key: None,
                    prompt_sync_id: None,
                })
                .unwrap();
        };
        for n in 0..4 {
            save("decision", &format!("A real memory {n}"), "project");
        }
        for n in 0..8 {
            save(
                "session_summary",
                &format!("Session summary {n}"),
                "project",
            );
        }
        save("decision", "A personal note", "personal");
    }

    let context = |value: serde_json::Value| {
        server
            .mem_context(Parameters(serde_json::from_value(value).unwrap()))
            .unwrap()
            .0
    };

    let out = context(json!({ "project": "leteo", "limit": 4 }));
    let titles: Vec<&str> = out
        .observations
        .iter()
        .map(|observation| observation.title.as_str())
        .collect();
    assert!(
        titles
            .iter()
            .all(|title| !title.starts_with("Session summary")),
        "the memory list has to be memories: {titles:?}"
    );
    assert_eq!(titles.len(), 4, "and it has to be full: {titles:?}");

    let personal = context(json!({ "project": "leteo", "scope": "personal", "limit": 4 }));
    assert_eq!(
        personal
            .observations
            .iter()
            .map(|observation| observation.title.as_str())
            .collect::<Vec<_>>(),
        ["A personal note"],
        "a narrowed request has to reach past the rows it filtered out"
    );
}

#[test]
fn a_search_says_when_its_own_maximum_is_what_ended_the_list() {
    let (_temp, server) = test_server(McpOptions::default());
    let cap = server.lock_store().unwrap().max_search_results();
    {
        let mut store = server.lock_store().unwrap();
        store.create_session("s1", "leteo", "C:/workspace").unwrap();
        for n in 0..(cap + 5) {
            store
                .add_observation(AddObservation {
                    session_id: "s1".to_owned(),
                    kind: "decision".to_owned(),
                    title: format!("Migration note {n}"),
                    content: "the body".to_owned(),
                    tool_name: None,
                    project: Some("leteo".to_owned()),
                    scope: "project".to_owned(),
                    topic_key: None,
                    prompt_sync_id: None,
                })
                .unwrap();
        }
    }
    let search = |value: serde_json::Value| {
        server
            .mem_search(Parameters(serde_json::from_value(value).unwrap()))
            .unwrap()
            .0
    };

    let capped = search(json!({ "query": "migration", "limit": cap + 30 }));
    assert_eq!(capped.count, cap);
    let hint = capped.hint.expect("a clamped search says it was clamped");
    assert!(
        hint.contains(&cap.to_string()),
        "the hint has to name the number that stopped it: {hint}"
    );

    let capped_by_caller = search(json!({ "query": "migration", "limit": 3 }));
    assert_eq!(capped_by_caller.count, 3);
    let hint = capped_by_caller
        .hint
        .expect("a page the caller's own limit ended says so too");
    assert!(
        hint.contains("More matched than were returned"),
        "and says which limit it was: {hint}"
    );

    let short = search(json!({ "query": "\"Migration note 1\"", "limit": cap + 30 }));
    assert!(short.count < cap, "{}", short.count);
    assert!(short.hint.is_none(), "{:?}", short.hint);

    let at_the_cap = search(json!({ "query": "migration", "limit": cap }));
    assert_eq!(at_the_cap.count, cap);
    let hint = at_the_cap
        .hint
        .expect("a page the store's own maximum ended says so at the cap too");
    assert!(
        hint.contains(&cap.to_string()) && !hint.contains("higher limit"),
        "at the cap a higher limit is not the remedy: {hint}"
    );

    {
        let mut store = server.lock_store().unwrap();
        for row in store
            .search(
                "migration",
                crate::SearchOptions {
                    limit: Some(cap + 30),
                    ..crate::SearchOptions::default()
                },
            )
            .unwrap()
            .into_iter()
            .take(5)
        {
            store
                .delete_observation(row.observation.id, None, true)
                .unwrap();
        }
    }
    let exactly = search(json!({ "query": "migration", "limit": cap + 30 }));
    assert_eq!(exactly.count, cap, "five of the {} are gone", cap + 5);
    assert!(
        exactly.hint.is_none(),
        "a complete answer that happens to fill the page explains nothing: {:?}",
        exactly.hint
    );
}

#[test]
fn a_memory_saved_without_a_session_still_records_the_question() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store
            .create_session("agent-conversation", "leteo", "C:/workspace")
            .unwrap();
        store
            .add_prompt(AddPrompt {
                session_id: "agent-conversation".to_owned(),
                content: "why does passive capture file the same learning twice?".to_owned(),
                project: Some("leteo".to_owned()),
            })
            .unwrap();
    }

    let saved = server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "title": "The hash was taken before the redaction",
                "content": "So the duplicate check could never match what the store held.",
                "project": "leteo",
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;

    let store = server.lock_store().unwrap();
    let observation = store.get_observation(saved.observation.id).unwrap();
    assert!(
        observation.session_id.starts_with("manual-save-"),
        "the path this is about: {}",
        observation.session_id
    );
    let prompt = store
        .latest_session_prompt_sync_id("agent-conversation")
        .unwrap()
        .expect("the hook's prompt is there");
    assert_eq!(
        observation.prompt_sync_id.as_deref(),
        Some(prompt.as_str()),
        "a memory saved the ordinary way records no question at all"
    );
    drop(store);

    {
        let store = server.lock_store().unwrap();
        store
            .connection()
            .execute(
                "UPDATE prompts SET created_at = datetime('now', '-1 day')",
                [],
            )
            .unwrap();
    }
    let stale = server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "title": "Something learned much later",
                "content": "the same project, a different sitting entirely",
                "project": "leteo",
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    assert_eq!(
        stale.observation.prompt_sync_id, None,
        "a memory was hung on a question from another day"
    );
}

#[test]
fn a_memory_records_the_question_that_produced_it_even_across_processes() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store
            .create_session("linked", "leteo", "C:/workspace")
            .unwrap();
        store
            .add_prompt(AddPrompt {
                session_id: "linked".to_owned(),
                content: "why did the search get slow?".to_owned(),
                project: Some("leteo".to_owned()),
            })
            .unwrap();
    }

    let saved = server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "linked",
                "title": "The CROSS JOIN that made search fast again",
                "content": "SQLite planned the join the other way round.",
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;

    let store = server.lock_store().unwrap();
    let observation = store.get_observation(saved.observation.id).unwrap();
    let prompt = store
        .latest_session_prompt_sync_id("linked")
        .unwrap()
        .expect("the hook's prompt is there");
    assert_eq!(
        observation.prompt_sync_id.as_deref(),
        Some(prompt.as_str()),
        "the memory has to name the question it answers"
    );

    drop(store);
    let unlinked = server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "linked",
                "title": "An automated save answering nobody",
                "content": "body",
                "capture_prompt": false,
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    assert!(
        server
            .lock_store()
            .unwrap()
            .get_observation(unlinked.observation.id)
            .unwrap()
            .prompt_sync_id
            .is_none()
    );
}

#[test]
fn a_session_summary_is_titled_by_what_the_session_was_for() {
    let (_temp, server) = test_server(McpOptions::default());
    server
        .mem_session_start(Parameters(
            serde_json::from_value(json!({
                "id": "s-headline",
                "project": "leteo",
                "directory": "H:/REPO/leteo",
            }))
            .unwrap(),
        ))
        .unwrap();
    let summarize = |content: &str| -> String {
        server
            .mem_session_summary(Parameters(
                serde_json::from_value(json!({
                    "session_id": "s-headline",
                    "project": "leteo",
                    "content": content,
                }))
                .unwrap(),
            ))
            .unwrap()
            .0
            .observation
            .title
    };

    let title = summarize("## Goal\nRestore deterministic chunk ordering after a rebuild\n");
    assert_eq!(
        title, "Restore deterministic chunk ordering after a rebuild",
        "the title is what the session was for, with nothing in front of it"
    );

    assert_eq!(summarize("## Goal\n2026-08-02\n"), "Session summary: leteo");

    let found = server
        .mem_search(Parameters(
            serde_json::from_value(json!({ "query": "deterministic chunk ordering" })).unwrap(),
        ))
        .unwrap()
        .0;
    assert!(
        found.results.iter().any(|result| result
            .observation
            .title
            .contains("deterministic chunk ordering")),
        "a summary titled by its goal has to come back for it: {:?}",
        found
            .results
            .iter()
            .map(|r| &r.observation.title)
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_save_naming_an_ended_session_is_refused() {
    let (_temp, server) = test_server(McpOptions::default());
    server
        .mem_session_start(Parameters(
            serde_json::from_value(json!({
                "id": "s-ended",
                "project": "leteo",
                "directory": "H:/REPO/leteo",
            }))
            .unwrap(),
        ))
        .unwrap();
    server
        .mem_session_end(Parameters(
            serde_json::from_value(json!({ "id": "s-ended" })).unwrap(),
        ))
        .unwrap();

    // A write naming the closed session is refused rather than filed under it,
    // so the session's history and its summary cannot drift apart afterwards.
    let error = server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "s-ended",
                "project": "leteo",
                "title": "Too late",
                "content": "a memory for a session that already closed",
            }))
            .unwrap(),
        ))
        .err()
        .expect("a write naming an ended session is refused");
    assert_eq!(
        error_payload(&error)["error"]["code"],
        "session_already_ended"
    );

    // The manual session a save without an id lands in never ends, so that door
    // stays open.
    server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "project": "leteo",
                "title": "Still accepted",
                "content": "a memory with no session id",
            }))
            .unwrap(),
        ))
        .unwrap();
}

/// Asserts a storage-cut report names the bound and the length kept.
///
/// The kept length is pinned rather than bounded: `stored_bytes < original_bytes`
/// passes for any wrong value below the original, which is how a regression in
/// "by how much" survives a suite that only ever asserts the original.
fn assert_storage_cut(cut: Option<crate::mcp::output::Truncation>, bound: usize, surface: &str) {
    let cut = cut.unwrap_or_else(|| panic!("{surface} reports the cut"));
    assert_eq!(cut.original_bytes, bound + 1, "{surface} original length");
    assert_eq!(cut.stored_bytes, bound, "{surface} stored length");
}

/// Every tool that reports the bound says when it cut what it stored.
///
/// A body over the bound is kept short and the tail is not kept, and the reply
/// said nothing: the caller saw a memory saved and believed it whole.
/// `content_truncated` and its siblings describe the 400-byte preview, which is
/// a different cut, so this needs a field of its own.
#[test]
fn every_write_surface_reports_the_storage_cut() {
    let (_temp, server) = test_server(McpOptions::default());
    let bound = server.lock_store().unwrap().max_observation_length();
    let over = "x".repeat(bound + 1);
    server
        .mem_session_start(Parameters(
            serde_json::from_value(json!({
                "id": "s-cut",
                "project": "leteo",
                "directory": "H:/REPO/leteo",
            }))
            .unwrap(),
        ))
        .unwrap();

    let saved = server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "s-cut",
                "title": "Over the bound",
                "content": over.clone(),
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    assert_storage_cut(saved.storage_truncation, bound, "save");

    // A body under the bound is stored whole and reports nothing.
    let short = server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "s-cut",
                "title": "Under the bound",
                "content": "short",
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    assert!(short.storage_truncation.is_none());

    let updated = server
        .mem_update(Parameters(
            serde_json::from_value(json!({
                "id": saved.observation.id,
                "expected_project": "leteo",
                "content": over.clone(),
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    assert_storage_cut(updated.storage_truncation, bound, "update");

    let prompt = server
        .mem_save_prompt(Parameters(
            serde_json::from_value(json!({
                "session_id": "s-cut",
                "project": "leteo",
                "content": over.clone(),
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    assert_storage_cut(prompt.storage_truncation, bound, "prompt save");

    let summary = server
        .mem_session_summary(Parameters(
            serde_json::from_value(json!({
                "session_id": "s-cut",
                "project": "leteo",
                "content": over.clone(),
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    assert_storage_cut(summary.storage_truncation, bound, "session summary");

    // Redaction is not a storage cut. This body arrives over the bound and the
    // bound sees it after the private span is gone, so it is stored whole, and
    // a report here would send the caller splitting a memory it kept entire.
    let private = format!(
        "{}<private>{}</private>",
        "x".repeat(bound / 2),
        "y".repeat(bound / 2)
    );
    assert!(private.len() > bound);
    let private_saved = server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "s-cut",
                "title": "Mostly private",
                "content": private,
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    assert!(
        private_saved.storage_truncation.is_none(),
        "a private span the bound never saw is not a storage cut"
    );

    let ended = server
        .mem_session_end(Parameters(
            serde_json::from_value(json!({ "id": "s-cut", "summary": over.clone() })).unwrap(),
        ))
        .unwrap()
        .0;
    assert_storage_cut(ended.storage_truncation, bound, "session end");
}

/// And so does a judgment, for both texts it stores.
#[test]
fn a_judgment_reports_the_storage_cut_on_both_texts() {
    let (_temp, server) = test_server(McpOptions::default());
    let bound = server.lock_store().unwrap().max_observation_length();
    let over = "x".repeat(bound + 1);
    let (first, second) = {
        let mut store = server.lock_store().unwrap();
        store.enroll_project("leteo").unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        let mut save = |title: &str| {
            store
                .add_observation(AddObservation {
                    session_id: "s1".to_owned(),
                    kind: "bugfix".to_owned(),
                    title: title.to_owned(),
                    content: format!("the body of {title}"),
                    tool_name: None,
                    project: Some("leteo".to_owned()),
                    scope: "project".to_owned(),
                    topic_key: None,
                    prompt_sync_id: None,
                })
                .unwrap()
                .observation
        };
        (save("The pool leaked"), save("The pool no longer leaks"))
    };
    let judgment = server
        .mem_compare(Parameters(
            serde_json::from_value(json!({
                "memory_id_a": second.id, "memory_id_b": first.id, "relation": "related",
            }))
            .unwrap(),
        ))
        .unwrap()
        .0
        .sync_id;

    let verdict = server
        .mem_judge(Parameters(
            serde_json::from_value(json!({
                "judgment_id": judgment,
                "relation": "supersedes",
                "reason": over.clone(),
                "evidence": over.clone(),
            }))
            .unwrap(),
        ))
        .unwrap()
        .0
        .relation;
    assert_storage_cut(verdict.reason_storage_truncation, bound, "judgment reason");
    assert_storage_cut(
        verdict.evidence_storage_truncation,
        bound,
        "judgment evidence",
    );
}

#[test]
fn mem_context_carries_the_language_memories_are_written_in() {
    let (temp, server) = test_server(McpOptions::default());
    let context = || -> String {
        server
            .mem_context(Parameters(
                serde_json::from_value(json!({ "project": "leteo" })).unwrap(),
            ))
            .unwrap()
            .0
            .memory_language
    };

    let directive = context();
    assert!(
        directive.contains("the language the user is writing in"),
        "auto has to say so: {directive}"
    );

    let data_dir = temp.path();
    crate::settings::save(
        data_dir,
        &crate::settings::Settings {
            language: Some("español".to_owned()),
            ..crate::settings::load(data_dir)
        },
    )
    .unwrap();
    let directive = context();
    assert!(
        directive.contains("español"),
        "a pinned language has to be named: {directive}"
    );
}

#[test]
fn the_server_instructions_name_real_tools_and_describe_real_behaviour() {
    let instructions = super::SERVER_INSTRUCTIONS;

    let declared: std::collections::BTreeSet<String> = crate::mcp::PROFILE_AGENT
        .iter()
        .chain(crate::mcp::PROFILE_ADMIN.iter())
        .map(|tool| (*tool).to_owned())
        .collect();
    let mut named = 0;
    for word in instructions.split(|c: char| !c.is_alphanumeric() && c != '_') {
        if word.starts_with("mem_") {
            let tool = word;
            named += 1;
            assert!(
                declared.contains(tool),
                "the server instructions name {tool}, which is not a tool"
            );
        }
    }
    assert!(named >= 5, "the instructions stopped naming tools at all");

    assert!(
        instructions.contains("first line") && instructions.contains("not a heading"),
        "the summary contract is no longer stated: {instructions}"
    );
    assert_eq!(
        crate::memory::normalize::headline(
            "## Goal\nRetire the duplicated chunk writer\n",
            crate::store::SUMMARY_HEADLINE_CHARS
        )
        .as_deref(),
        Some("Retire the duplicated chunk writer"),
        "the instructions promise a behaviour the code no longer has"
    );
}

/// The server instructions fit the length a client will actually deliver.
///
/// Claude Code cuts them at `SERVER_INSTRUCTIONS_LIMIT` and appends
/// `… [truncated]`, so a block that runs past it reaches the agent with its
/// last rules missing and nothing saying so: the whole SUMMARIES paragraph was
/// lost this way. Counted in Unicode scalar values, which is what the client
/// counts, and held against the one constant the production code defines.
#[test]
fn the_server_instructions_fit_the_limit_a_client_will_deliver() {
    let instructions = super::SERVER_INSTRUCTIONS;
    let length = instructions.chars().count();
    assert!(
        length <= super::SERVER_INSTRUCTIONS_LIMIT,
        "the server instructions render to {length} characters, past the {} a \
         client truncates at; shorten them, or move a rule into the tool \
         description it belongs to",
        super::SERVER_INSTRUCTIONS_LIMIT
    );
}

#[test]
fn saving_the_same_memory_again_asks_no_new_questions() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store
            .create_session("filler", "leteo", "C:/workspace")
            .unwrap();
        for index in 0..40 {
            store
                .add_observation(AddObservation {
                    session_id: "filler".to_owned(),
                    kind: "discovery".to_owned(),
                    title: format!("Unrelated note {index} on deployment windows"),
                    content: format!("Body {index}: staged rollout, canaries, rollback."),
                    tool_name: None,
                    project: Some("leteo".to_owned()),
                    scope: "project".to_owned(),
                    topic_key: None,
                    prompt_sync_id: None,
                })
                .unwrap();
        }
        for (index, ending) in [
            "was kept",
            "the store held",
            "had been stored",
            "was on disk",
        ]
        .into_iter()
        .enumerate()
        {
            store
                .add_observation(AddObservation {
                    session_id: "filler".to_owned(),
                    kind: "bugfix".to_owned(),
                    title: format!(
                        "The passive capture hash was taken before the redaction {index}"
                    ),
                    content: format!("So the duplicate check could never match what {ending}."),
                    tool_name: None,
                    project: Some("leteo".to_owned()),
                    scope: "project".to_owned(),
                    topic_key: None,
                    prompt_sync_id: None,
                })
                .unwrap();
        }
    }

    let save = |title: &str| {
        server
            .mem_save(Parameters(
                serde_json::from_value(json!({
                    "title": title,
                    "content": "The duplicate check therefore never matched what the store kept.",
                    "project": "leteo",
                }))
                .unwrap(),
            ))
            .unwrap()
            .0
    };

    let first = save("The passive capture hash was taken before the redaction ran");
    assert_eq!(first.status, "inserted");
    assert!(!first.candidates.is_empty(), "a new memory is asked about");

    let again = save("The passive capture hash was taken before the redaction ran");
    assert_eq!(again.status, "deduplicated");
    assert!(
        again.candidates.is_empty(),
        "the same memory asked again: {:?}",
        again.candidates
    );

    let store = server.lock_store().unwrap();
    let rows: i64 = store
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM memory_relations WHERE source_id = ?1",
            [&first.observation.sync_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rows as usize, first.candidates.len());
}

#[test]
fn the_context_leaves_out_the_memories_a_later_one_overturned() {
    let (_temp, server) = test_server(McpOptions::default());
    let (old, new) = {
        let mut store = server.lock_store().unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        let old = store
            .add_observation(AddObservation {
                session_id: "s1".to_owned(),
                kind: "decision".to_owned(),
                title: "The store is one SQLite file".to_owned(),
                content: "One writer, many readers.".to_owned(),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap()
            .observation;
        let new = store
            .add_observation(AddObservation {
                session_id: "s1".to_owned(),
                kind: "decision".to_owned(),
                title: "The cloud half is PostgreSQL".to_owned(),
                content: "The single file is the local half only.".to_owned(),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap()
            .observation;
        let relation = store
            .save_relation(crate::memory::model::SaveRelationParams {
                sync_id: crate::memory::normalize::sync_id("rel"),
                source_id: new.sync_id.clone(),
                target_id: old.sync_id.clone(),
            })
            .unwrap();
        store
            .judge_relation(crate::memory::model::JudgeRelationParams {
                judgment_id: relation.sync_id,
                relation: "supersedes".to_owned(),
                marked_by_actor: "agent".to_owned(),
                ..Default::default()
            })
            .unwrap();
        (old, new)
    };

    let Json(context) = server
        .mem_context(Parameters(
            serde_json::from_value(json!({ "project": "leteo" })).unwrap(),
        ))
        .expect("the context is built");
    assert!(
        context
            .observations
            .iter()
            .all(|observation| observation.id != old.id),
        "a decision that was overturned is not handed over as though it still held: {:?}",
        context.observations
    );
    let current = context
        .observations
        .iter()
        .find(|observation| observation.id == new.id)
        .expect("the newer memory is listed");
    assert!(current.caveats.is_empty());

    // Hidden from the listing, not from a direct read: the caveat still reaches
    // the id an agent may already be holding.
    let Json(fetched) = server
        .mem_get_observation(Parameters(GetObservationParams {
            id: old.id,
            include_history: false,
        }))
        .expect("a superseded memory is still readable by id");
    assert_eq!(
        fetched
            .observation
            .caveats
            .iter()
            .map(|caveat| (caveat.relation.as_str(), caveat.other_id))
            .collect::<Vec<_>>(),
        [("superseded_by", new.id)],
        "a fetch by id still says what overturned it"
    );
}

/// The crate-implementation word a published description carries, if it carries
/// one.
///
/// Whole words, because the substring test is wrong three times over the
/// surface as it stands: `Option` is inside "Optional" on twelve legitimate
/// descriptions, `impl` is inside a word in `SaveParams::kind`'s vocabulary
/// list, and `struct` is inside "instructions" in `mem_session_summary`'s own
/// tool description. Measured against all 172 published strings, the whole-word
/// form has no false positive and the substring form has three.
///
/// `#[` is the exception and is read as a substring, because it holds no
/// alphanumeric character and no split can yield it as a word.
fn maintainer_vocabulary(description: &str) -> Option<&'static str> {
    const TERMS: [&str; 9] = [
        "serde", "schemars", "Option", "String", "struct", "enum", "impl", "trait", "Vec",
    ];
    if description.contains("#[") {
        return Some("#[");
    }
    description
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .find_map(|word| TERMS.into_iter().find(|term| *term == word))
}

/// What a published *field* description may weigh.
///
/// Measured on this tree rather than chosen: the longest legitimate field
/// description is 340 bytes, `SaveParams::kind`'s vocabulary of the eight types
/// and what happens outside them, and the paragraph this guard exists for is
/// 478. Nothing at all sits between them, so the ceiling had a 138-byte band to
/// sit in, and it leaves `SaveParams::kind` 60 bytes of that band — six more
/// types' worth of growth — before it needs revisiting. A field ceiling and not
/// a tool one: the longest tool description is `mem_save`'s at 401 bytes, and
/// it earns them.
const PUBLISHED_FIELD_DESCRIPTION_CEILING: usize = 400;

/// A tool's own description is written for the agent choosing between tools.
///
/// These are `#[tool(description = ...)]` literals rather than `///`, and they
/// are not summarised, so the two formatting checks below can still fire on
/// them. That is why they are here and not in the field guard, where the same
/// two could not fire and are gone. The vocabulary check is what asks the
/// question the guard's name asks.
#[test]
fn no_tool_describes_itself_with_the_source_it_was_written_in() {
    let mut offenders = Vec::new();
    for tool in LeteoMcpServer::router().list_all() {
        let Some(description) = tool.description.as_ref() else {
            continue;
        };
        if let Some(term) = maintainer_vocabulary(description) {
            offenders.push(format!("{}: carries `{term}`: {description:?}", tool.name));
        } else if description.lines().count() > 1 || description.contains("  ") {
            offenders.push(format!("{}: {description:?}", tool.name));
        }
    }
    assert!(
        offenders.is_empty(),
        "these descriptions carry their own source code:\n{}",
        offenders.join("\n")
    );
}

/// No published field description is written to a maintainer rather than to the
/// agent that must fill the field in.
///
/// Three formatting checks were the whole guard until #94, and two of them
/// could not fire from any source: `summary_of` runs over every description in
/// the schema before this test sees one, and it ends in
/// `split_whitespace().join(" ")`, so neither a newline nor a double space
/// survives to be found — an attribute-written description no more than a
/// `///`. Two checks that cannot fire are the defect this guard was written
/// about, so they are gone from here; they still hold the tool guard above,
/// which reads the router raw. The intra-doc link survives `summary_of` and
/// stays. What was missing is the question the name asks, and the answer is the
/// vocabulary and the weight: the 478-byte serde/schemars postmortem on
/// `ProjectEnvelope::project_path` shipped six times a session, through a guard
/// that was green, because it is one line with single spaces like every
/// legitimate description beside it.
#[test]
fn no_field_describes_itself_to_an_agent_in_rust() {
    let mut offenders = Vec::new();
    for tool in LeteoMcpServer::with_options(
        Arc::new(Mutex::new(
            Store::open(crate::store::StoreConfig::new(
                tempfile::tempdir().unwrap().path().join("mcp.db"),
            ))
            .unwrap(),
        )),
        McpOptions::default(),
    )
    .router
    .list_all()
    {
        for (half, schema) in [
            ("input", Some(tool.input_schema.clone())),
            ("output", tool.output_schema.clone()),
        ] {
            let Some(schema) = schema else { continue };
            let schema = serde_json::Value::Object((*schema).clone());
            let mut found = Vec::new();
            collect_descriptions(&schema, &mut found);
            for description in found {
                if let Some(term) = maintainer_vocabulary(&description) {
                    offenders.push(format!(
                        "{} {half}: carries `{term}`: {description:?}",
                        tool.name
                    ));
                } else if description.len() > PUBLISHED_FIELD_DESCRIPTION_CEILING {
                    offenders.push(format!(
                        "{} {half}: {} bytes, over the {PUBLISHED_FIELD_DESCRIPTION_CEILING} a field description may weigh: {description:?}",
                        tool.name,
                        description.len()
                    ));
                } else if description.contains("[`") {
                    offenders.push(format!("{} {half}: {description:?}", tool.name));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these field descriptions are written to a maintainer:\n{}",
        offenders.join("\n")
    );
}

fn collect_descriptions(value: &serde_json::Value, found: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(description) = map.get("description").and_then(serde_json::Value::as_str) {
                found.push(description.to_owned());
            }
            for nested in map.values() {
                collect_descriptions(nested, found);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_descriptions(item, found);
            }
        }
        _ => {}
    }
}

#[test]
fn a_preview_is_no_longer_than_the_number_the_description_publishes() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        store
            .add_observation(AddObservation {
                session_id: "s1".to_owned(),
                kind: "discovery".to_owned(),
                title: "A memory far longer than any preview".to_owned(),
                content: "cabaña ".repeat(4_000),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
    }

    let Json(found) = server
        .mem_search(Parameters(SearchParams {
            query: "cabaña".to_owned(),
            kind: None,
            project: Some("leteo".to_owned()),
            all_projects: false,
            scope: None,
            limit: None,
            match_mode: MatchMode::All,
        }))
        .expect("the search succeeds");
    let result = serde_json::to_value(found).unwrap();
    let content = result["results"][0]["content"]
        .as_str()
        .expect("the result carries a preview");
    assert!(
        result["results"][0]["content_truncated"]
            .as_bool()
            .unwrap_or(false),
        "the fixture has to be long enough to be cut"
    );
    assert!(
        content.len() <= PREVIEW_BYTES,
        "the preview is {} bytes against the {PREVIEW_BYTES} the description promises: {}",
        content.len(),
        &content[content.len().saturating_sub(40)..]
    );
    assert!(
        content.chars().count() < PREVIEW_BYTES,
        "the cut is on bytes, so this non-ASCII body arrives in {} characters, fewer than the {PREVIEW_BYTES} bytes the description promises: {content:?}",
        content.chars().count()
    );
    assert!(content.ends_with("[truncated]"), "{content:?}");
}

#[test]
fn only_the_newest_memories_are_quoted_and_the_rest_are_named() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        for index in 0..20 {
            store
                .add_observation(AddObservation {
                    session_id: "s1".to_owned(),
                    kind: "discovery".to_owned(),
                    title: format!("Memoria número {index}"),
                    content: "Un cuerpo cualquiera, con bastantes palabras dentro para que \
                              recitarlo entero costase algo."
                        .to_owned(),
                    tool_name: None,
                    project: Some("leteo".to_owned()),
                    scope: "project".to_owned(),
                    topic_key: None,
                    prompt_sync_id: None,
                })
                .unwrap();
        }
    }

    let params: ContextParams = serde_json::from_value(json!({ "project": "leteo" })).unwrap();
    let out = server.mem_context(Parameters(params)).unwrap().0;

    assert_eq!(
        out.observations.len(),
        crate::recall::DETAILED,
        "only the newest few are quoted"
    );
    assert_eq!(
        out.also_remembered.len(),
        20 - crate::recall::DETAILED,
        "and the rest are named, not dropped"
    );
    assert_eq!(out.count, 20, "the count is of everything handed over");
    assert!(
        out.also_remembered
            .iter()
            .all(|line| line.id > 0 && !line.title.is_empty()),
        "a named memory has to be fetchable"
    );
    assert!(
        out.observations[0].title.contains("19"),
        "the newest is quoted first: {}",
        out.observations[0].title
    );
}

#[test]
fn the_other_tools_spelling_of_an_identifier_is_accepted() {
    let fetched: GetObservationParams =
        serde_json::from_value(json!({"observation_id": 7})).expect("mem_get_observation");
    assert_eq!(fetched.id, 7);
    let deleted: DeleteParams =
        serde_json::from_value(json!({"observation_id": 7, "expected_project": "leteo"}))
            .expect("mem_delete");
    assert_eq!(deleted.id, 7);
    let pinned: PinParams =
        serde_json::from_value(json!({"observation_id": 7, "expected_project": "leteo"}))
            .expect("mem_pin");
    assert_eq!(pinned.id, 7);
    let updated: UpdateParams =
        serde_json::from_value(json!({"observation_id": 7, "expected_project": "leteo"}))
            .expect("mem_update");
    assert_eq!(updated.id, 7);

    let timeline: TimelineParams = serde_json::from_value(json!({"id": 7})).unwrap();
    assert_eq!(timeline.observation_id, 7);

    let started: SessionStartParams = serde_json::from_value(json!({"session_id": "s1"})).unwrap();
    assert_eq!(started.id, "s1");
    let ended: SessionEndParams = serde_json::from_value(json!({"session_id": "s1"})).unwrap();
    assert_eq!(ended.id, "s1");
}

#[test]
fn a_superseded_memory_is_left_out_of_search_results() {
    let (_temp, server) = test_server(McpOptions::default());
    let (old, new) = {
        let mut store = server.lock_store().unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        let memory = |title: &str, content: &str| AddObservation {
            session_id: "s1".to_owned(),
            kind: "decision".to_owned(),
            title: title.to_owned(),
            content: content.to_owned(),
            tool_name: None,
            project: Some("leteo".to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            prompt_sync_id: None,
        };
        let old = store
            .add_observation(memory(
                "Decisión vieja sobre ornitorrincos",
                "Usamos el método antiguo.",
            ))
            .unwrap()
            .observation;
        let new = store
            .add_observation(memory(
                "Decisión nueva sobre ornitorrincos",
                "Cambiamos de método.",
            ))
            .unwrap()
            .observation;
        let proposed = store
            .save_relation(crate::memory::model::SaveRelationParams {
                sync_id: "rel-prueba".to_owned(),
                source_id: new.sync_id.clone(),
                target_id: old.sync_id.clone(),
            })
            .unwrap();
        store
            .judge_relation(crate::memory::model::JudgeRelationParams {
                judgment_id: proposed.sync_id,
                relation: "supersedes".to_owned(),
                reason: Some("la nueva sustituye a la vieja".to_owned()),
                ..Default::default()
            })
            .unwrap();
        (old, new)
    };

    let params: SearchParams =
        serde_json::from_value(json!({ "query": "ornitorrincos", "all_projects": true })).unwrap();
    let out = server.mem_search(Parameters(params)).unwrap().0;

    assert!(
        out.results
            .iter()
            .all(|result| result.observation.id != old.id),
        "a memory a later one overturned is not a search result: {:?}",
        out.results
    );
    let newer = out
        .results
        .iter()
        .find(|result| result.observation.id == new.id)
        .expect("the newer decision is the one that stands");
    assert!(
        newer
            .observation
            .caveats
            .iter()
            .all(|caveat| caveat.relation != "superseded_by"),
        "the newer decision still holds"
    );

    // Hidden from search, not from a direct read.
    let Json(fetched) = server
        .mem_get_observation(Parameters(GetObservationParams {
            id: old.id,
            include_history: false,
        }))
        .expect("a superseded memory is still readable by id");
    assert_eq!(fetched.observation.caveats.len(), 1);
    assert_eq!(fetched.observation.caveats[0].other_id, new.id);
}

#[test]
fn an_untouched_memory_does_not_repeat_itself() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        store
            .add_observation(AddObservation {
                session_id: "s1".to_owned(),
                kind: "discovery".to_owned(),
                title: "Una memoria recién escrita".to_owned(),
                content: "Cuerpo.".to_owned(),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
    }

    let params: SearchParams =
        serde_json::from_value(json!({ "query": "recién escrita", "all_projects": true })).unwrap();
    let out = server.mem_search(Parameters(params)).unwrap().0;
    let payload = serde_json::to_value(&out).unwrap();
    let result = &payload["results"][0];

    for quiet in [
        "revision_count",
        "duplicate_count",
        "state",
        "pinned",
        "updated_at",
        "last_seen_at",
    ] {
        assert!(
            result.get(quiet).is_none(),
            "{quiet} at its default says nothing and is sent on every memory: {result}"
        );
    }
    assert!(result["id"].is_number());
    assert!(result["created_at"].is_string());
}

#[test]
fn a_search_that_names_no_project_does_not_answer_from_the_others() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        for project in ["leteo", "another-thing"] {
            store.enroll_project(project).unwrap();
            store
                .create_session(project, project, &format!("C:/{project}"))
                .unwrap();
            store
                .add_observation(AddObservation {
                    session_id: project.to_owned(),
                    kind: "decision".to_owned(),
                    title: format!("The retry budget in {project}"),
                    content: "three attempts and then it stops".to_owned(),
                    tool_name: None,
                    project: Some(project.to_owned()),
                    scope: "project".to_owned(),
                    topic_key: None,
                    prompt_sync_id: None,
                })
                .unwrap();
        }
    }
    let search = |value: serde_json::Value| {
        server
            .mem_search(Parameters(serde_json::from_value(value).unwrap()))
            .unwrap()
            .0
    };

    let narrowed = search(json!({ "query": "retry budget" }));
    assert_eq!(narrowed.count, 1, "{:?}", narrowed.results);
    assert_eq!(
        narrowed.results[0].observation.project.as_deref(),
        Some("leteo")
    );

    let widened = search(json!({ "query": "retry budget", "all_projects": true }));
    assert_eq!(widened.count, 2, "{:?}", widened.results);
    let asked_for = search(json!({ "query": "retry budget", "project": "another-thing" }));
    assert_eq!(asked_for.count, 1);
    assert_eq!(
        asked_for.results[0].observation.project.as_deref(),
        Some("another-thing")
    );
}

#[test]
fn a_capture_that_extracted_nothing_says_what_shape_it_reads() {
    let (_temp, server) = test_server(McpOptions::default());
    let capture = |content: &str| {
        server
            .mem_capture_passive(Parameters(
                serde_json::from_value(json!({ "content": content })).unwrap(),
            ))
            .unwrap()
            .0
    };

    let inline = capture("Key learnings: the pool was never returned on the error path.");
    assert_eq!(inline.extracted, 0);
    let hint = inline
        .hint
        .expect("a capture that saved nothing has to say why");
    assert!(hint.contains("## Key Learnings"), "{hint}");
    assert!(
        hint.contains("mem_save"),
        "and where to go with one fact: {hint}"
    );

    let worked = capture("## Key Learnings\n- the pool was never returned on the error path");
    assert_eq!(worked.extracted, 1);
    assert_eq!(worked.saved, 1);
    assert!(worked.hint.is_none(), "{:?}", worked.hint);
}

#[test]
fn comparing_two_memories_asks_only_for_what_the_store_keeps() {
    let (_temp, server) = test_server(McpOptions::default());
    let (first, second) = {
        let mut store = server.lock_store().unwrap();
        store.enroll_project("leteo").unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        let save = |store: &mut crate::store::Store, title: &str, body: &str| {
            store
                .add_observation(AddObservation {
                    session_id: "s1".to_owned(),
                    kind: "decision".to_owned(),
                    title: title.to_owned(),
                    content: body.to_owned(),
                    tool_name: None,
                    project: Some("leteo".to_owned()),
                    scope: "project".to_owned(),
                    topic_key: None,
                    prompt_sync_id: None,
                })
                .unwrap()
                .observation
                .id
        };
        let first = save(&mut store, "Three retries", "three attempts, two seconds");
        let second = save(&mut store, "Five retries", "five attempts, two seconds");
        (first, second)
    };
    let compare = |value: serde_json::Value| {
        server.mem_compare(Parameters(serde_json::from_value(value).unwrap()))
    };

    let full = compare(json!({
        "memory_id_a": second, "memory_id_b": first, "relation": "supersedes",
        "confidence": 0.9, "reasoning": "the later one changes the number",
    }))
    .expect("a complete verdict is accepted");
    assert!(!full.0.sync_id.is_empty());

    let bare = compare(json!({
        "memory_id_a": second, "memory_id_b": first, "relation": "supersedes",
    }))
    .expect("a verdict with no confidence and no reason is still a verdict");
    assert_eq!(
        bare.0.sync_id, full.0.sync_id,
        "the same pair is the same relation, rewritten"
    );

    let refused = |value: serde_json::Value| -> String {
        let Err(error) = compare(value) else {
            panic!("this call has to be refused");
        };
        error_payload(&error)["error"]["code"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(
        refused(json!({ "memory_id_a": second, "memory_id_b": first, "relation": "maybe" })),
        "invalid_relation",
        "a verb the graph cannot read"
    );
    assert_eq!(
        refused(json!({
            "memory_id_a": second, "memory_id_b": first,
            "relation": "related", "confidence": 7.5,
        })),
        "invalid_params",
        "a confidence outside 0..=1 is not a probability"
    );
    assert_eq!(
        refused(json!({ "memory_id_a": second, "memory_id_b": 9999, "relation": "related" })),
        "observation_not_found",
        "a pair needs two memories that exist"
    );

    let nothing = compare(json!({
        "memory_id_a": second, "memory_id_b": first, "relation": "not_conflict",
    }))
    .expect("not_conflict is a successful no-op");
    assert!(
        nothing.0.sync_id.is_empty(),
        "nothing to file: {:?}",
        nothing.0.sync_id
    );
}

#[test]
fn a_memory_cannot_be_moved_into_a_project_that_does_not_exist() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store.enroll_project("leteo").unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
    }
    let save = server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "s1",
                "title": "A memory of leteo",
                "content": "it belongs to this project and to no other",
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    let id = save.observation.id;
    let update = |value: serde_json::Value| {
        server.mem_update(Parameters(serde_json::from_value(value).unwrap()))
    };

    let Err(error) = update(
        json!({ "id": id, "expected_project": "leteo", "project": "a-project-that-is-not-here" }),
    ) else {
        panic!("a memory cannot be moved somewhere that does not exist");
    };
    let payload = error_payload(&error);
    assert_eq!(payload["error"]["code"], "unknown_project");
    assert_eq!(
        payload["detected_project"], "leteo",
        "and the refusal says where it is, the way a save's does"
    );

    let renamed = update(json!({
        "id": id,
        "expected_project": "leteo",
        "title": "A memory of leteo, retitled",
    }))
    .expect("an ordinary edit is not a move")
    .0;
    assert_eq!(renamed.observation.project.as_deref(), Some("leteo"));

    server
        .mem_session_start(Parameters(
            serde_json::from_value(
                json!({ "id": "s2", "project": "somewhere-real", "directory": "C:/elsewhere" }),
            )
            .unwrap(),
        ))
        .unwrap();
    let moved =
        update(json!({ "id": id, "expected_project": "leteo", "project": "somewhere-real" }))
            .expect("a project the store knows is a place a memory can move to")
            .0;
    assert_eq!(moved.observation.project.as_deref(), Some("somewhere-real"));
}

/// A server holding one enrolled memory in `alpha`, for the ownership refusals.
///
/// The project is enrolled so a permitted write would queue a sync mutation:
/// with nothing in the queue, "the queue is untouched" would hold whether the
/// check ran or not.
fn ownership_fixture() -> (tempfile::TempDir, LeteoMcpServer, i64, String) {
    let (temp, server) = test_server(McpOptions::default());
    let saved = {
        let mut store = server.lock_store().unwrap();
        store.enroll_project("alpha").unwrap();
        store.create_session("s1", "alpha", "C:/alpha").unwrap();
        store
            .add_observation(AddObservation {
                session_id: "s1".to_owned(),
                kind: "decision".to_owned(),
                title: "A memory of alpha".to_owned(),
                content: "the body only alpha may change".to_owned(),
                tool_name: None,
                project: Some("alpha".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap()
            .observation
    };
    (temp, server, saved.id, saved.sync_id)
}

fn pending_mutations(server: &LeteoMcpServer, sync_id: &str) -> i64 {
    server
        .lock_store()
        .unwrap()
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM sync_mutations WHERE entity_key = ?1",
            [sync_id],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn mem_update_refuses_a_memory_filed_under_another_project() {
    let (_temp, server, id, sync_id) = ownership_fixture();
    let before = server.lock_store().unwrap().get_observation(id).unwrap();
    let queued = pending_mutations(&server, &sync_id);

    let Err(error) = server.mem_update(Parameters(
        serde_json::from_value(json!({
            "id": id,
            "expected_project": "beta",
            "title": "rewritten by the wrong project",
        }))
        .unwrap(),
    )) else {
        panic!("a memory of alpha must not be revised as a memory of beta");
    };
    assert_eq!(error_payload(&error)["error"]["code"], "project_mismatch");

    let after = server.lock_store().unwrap().get_observation(id).unwrap();
    assert_eq!(after.title, before.title, "the body was rewritten anyway");
    assert_eq!(
        after.revision_count, before.revision_count,
        "a refused update still counted as a revision"
    );
    assert_eq!(
        pending_mutations(&server, &sync_id),
        queued,
        "a refused update still queued a sync mutation"
    );
}

#[test]
fn mem_delete_refuses_a_memory_filed_under_another_project() {
    let (_temp, server, id, sync_id) = ownership_fixture();
    let before = server.lock_store().unwrap().get_observation(id).unwrap();
    let queued = pending_mutations(&server, &sync_id);

    let Err(error) = server.mem_delete(Parameters(DeleteParams {
        id,
        expected_project: "beta".to_owned(),
        hard_delete: true,
    })) else {
        panic!("a memory of alpha must not be hard-deleted as a memory of beta");
    };
    assert_eq!(error_payload(&error)["error"]["code"], "project_mismatch");

    let after = server.lock_store().unwrap().get_observation(id).unwrap();
    assert_eq!(after.state(), "active", "the memory was deleted anyway");
    assert_eq!(after.revision_count, before.revision_count);
    assert_eq!(
        pending_mutations(&server, &sync_id),
        queued,
        "a refused delete still queued a sync mutation"
    );
}

#[test]
fn mem_pin_refuses_a_memory_filed_under_another_project() {
    let (_temp, server, id, sync_id) = ownership_fixture();
    let queued = pending_mutations(&server, &sync_id);

    let Err(error) = server.mem_pin(Parameters(PinParams {
        id,
        expected_project: "beta".to_owned(),
    })) else {
        panic!("a memory of alpha must not be pinned as a memory of beta");
    };
    assert_eq!(error_payload(&error)["error"]["code"], "project_mismatch");
    assert!(
        !server
            .lock_store()
            .unwrap()
            .get_observation(id)
            .unwrap()
            .pinned,
        "the memory was pinned anyway"
    );
    assert_eq!(pending_mutations(&server, &sync_id), queued);
}

#[test]
fn mem_unpin_refuses_a_memory_filed_under_another_project() {
    let (_temp, server, id, sync_id) = ownership_fixture();
    server
        .lock_store()
        .unwrap()
        .pin_observation(id, None)
        .unwrap();
    let queued = pending_mutations(&server, &sync_id);

    let Err(error) = server.mem_unpin(Parameters(PinParams {
        id,
        expected_project: "beta".to_owned(),
    })) else {
        panic!("a memory of alpha must not be unpinned as a memory of beta");
    };
    assert_eq!(error_payload(&error)["error"]["code"], "project_mismatch");
    assert!(
        server
            .lock_store()
            .unwrap()
            .get_observation(id)
            .unwrap()
            .pinned,
        "the memory was unpinned anyway"
    );
    assert_eq!(pending_mutations(&server, &sync_id), queued);
}

#[test]
fn the_mutating_tools_refuse_a_missing_expected_project() {
    for (tool, error) in [
        (
            "mem_update",
            serde_json::from_value::<UpdateParams>(json!({ "id": 1 })).err(),
        ),
        (
            "mem_delete",
            serde_json::from_value::<DeleteParams>(json!({ "id": 1 })).err(),
        ),
        (
            "mem_pin",
            serde_json::from_value::<PinParams>(json!({ "id": 1 })).err(),
        ),
        (
            "mem_unpin",
            serde_json::from_value::<PinParams>(json!({ "id": 1 })).err(),
        ),
        (
            "mem_consolidate",
            serde_json::from_value::<ConsolidateParams>(json!({
                "source_ids": [1],
                "title": "t",
                "content": "c",
            }))
            .err(),
        ),
    ] {
        let error = error.unwrap_or_else(|| panic!("{tool} parsed without expected_project"));
        assert!(
            error.to_string().contains("expected_project"),
            "{tool} refused without naming the missing field: {error}"
        );
    }
}

#[test]
fn the_mutating_tools_publish_expected_project_as_required() {
    for name in [
        "mem_update",
        "mem_delete",
        "mem_pin",
        "mem_unpin",
        "mem_consolidate",
    ] {
        let tool = LeteoMcpServer::router()
            .list_all()
            .into_iter()
            .find(|tool| tool.name == name)
            .unwrap_or_else(|| panic!("{name} is exposed"));
        let required = tool.input_schema["required"]
            .as_array()
            .unwrap_or_else(|| panic!("{name} publishes no required list"));
        assert!(
            required.iter().any(|field| field == "expected_project"),
            "{name} does not require expected_project: {required:?}"
        );
    }
}

#[test]
fn judging_a_pair_replaces_the_verdict_and_refuses_what_the_graph_cannot_read() {
    let (_temp, server) = test_server(McpOptions::default());
    let (first, second) = {
        let mut store = server.lock_store().unwrap();
        store.enroll_project("leteo").unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        let mut save = |title: &str| {
            store
                .add_observation(AddObservation {
                    session_id: "s1".to_owned(),
                    kind: "bugfix".to_owned(),
                    title: title.to_owned(),
                    content: format!("the body of {title}"),
                    tool_name: None,
                    project: Some("leteo".to_owned()),
                    scope: "project".to_owned(),
                    topic_key: None,
                    prompt_sync_id: None,
                })
                .unwrap()
                .observation
        };
        (save("The pool leaked"), save("The pool no longer leaks"))
    };
    let judgment = server
        .mem_compare(Parameters(
            serde_json::from_value(json!({
                "memory_id_a": second.id, "memory_id_b": first.id, "relation": "related",
            }))
            .unwrap(),
        ))
        .unwrap()
        .0
        .sync_id;

    let judge = |value: serde_json::Value| {
        server.mem_judge(Parameters(serde_json::from_value(value).unwrap()))
    };
    let verdict = judge(json!({
        "judgment_id": judgment, "relation": "supersedes",
        "reason": "the later one replaces it", "confidence": 0.8,
    }))
    .expect("a verdict on a pair that exists")
    .0
    .relation;
    assert_eq!(verdict.relation, "supersedes");
    assert_eq!(verdict.judgment_status, "judged");
    assert_eq!(verdict.reason.as_deref(), Some("the later one replaces it"));

    let revised = judge(json!({ "judgment_id": judgment, "relation": "related" }))
        .expect("a pair can be judged again")
        .0
        .relation;
    assert_eq!(revised.relation, "related");
    assert_eq!(
        revised.reason, None,
        "the old reason explained a verdict that no longer stands"
    );

    let refused = |value: serde_json::Value| -> String {
        let Err(error) = judge(value) else {
            panic!("this verdict has to be refused");
        };
        error_payload(&error)["error"]["code"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(
        refused(json!({ "judgment_id": judgment, "relation": "maybe" })),
        "invalid_relation"
    );
    assert_eq!(
        refused(json!({ "judgment_id": judgment, "relation": "related", "confidence": 9.9 })),
        "invalid_params"
    );
    assert_eq!(
        refused(json!({ "judgment_id": "rel-nothing", "relation": "related" })),
        "relation_not_found"
    );

    {
        let mut store = server.lock_store().unwrap();
        store.enroll_project("somewhere-else").unwrap();
        store
            .update_observation(
                first.id,
                None,
                UpdateObservation {
                    project: Some("somewhere-else".to_owned()),
                    ..UpdateObservation::default()
                },
            )
            .unwrap();
    }
    assert_eq!(
        refused(json!({ "judgment_id": judgment, "relation": "related" })),
        "cross_project_relation"
    );
}

#[test]
fn reviewing_a_memory_moves_its_date_and_answers_one_shape() {
    let (_temp, server) = test_server(McpOptions::default());
    let saved = {
        let mut store = server.lock_store().unwrap();
        store.enroll_project("leteo").unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        store
            .add_observation(AddObservation {
                session_id: "s1".to_owned(),
                kind: "decision".to_owned(),
                title: "Which store to use".to_owned(),
                content: "sqlite, and this is worth looking at again".to_owned(),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap()
            .observation
    };
    assert!(
        saved.review_after.is_some(),
        "a decision is one of the three that come due"
    );
    let review = |value: serde_json::Value| {
        server.mem_review(Parameters(serde_json::from_value(value).unwrap()))
    };

    let listed = review(json!({ "action": "list" })).unwrap().0;
    assert_eq!(listed.count, listed.observations.len());
    assert_eq!(
        listed.count, 0,
        "nothing is due yet, which is not the same as nothing working"
    );

    {
        let store = server.lock_store().unwrap();
        store
            .connection()
            .execute(
                "UPDATE observations SET review_after = '2020-01-01 00:00:00' WHERE id = ?1",
                [saved.id],
            )
            .unwrap();
    }
    let due = review(json!({ "action": "list" })).unwrap().0;
    assert_eq!(due.count, 1, "a memory past its date is due");
    assert_eq!(due.count, due.observations.len());

    let marked = review(json!({ "action": "mark_reviewed", "observation_id": saved.id }))
        .unwrap()
        .0;
    assert_eq!(
        marked.count,
        marked.observations.len(),
        "count says how many memories this answer carries, whatever was asked"
    );
    let looked_at = marked
        .observation
        .expect("the memory that was marked comes back");
    assert!(
        looked_at.review_after.as_deref() > Some("2020-01-01 00:00:00"),
        "the clock restarts from the day somebody looked: {:?}",
        looked_at.review_after
    );
    assert_eq!(
        review(json!({ "action": "list" })).unwrap().0.count,
        0,
        "and it stops being due"
    );

    let refused = |value: serde_json::Value| -> String {
        let Err(error) = review(value) else {
            panic!("this call has to be refused");
        };
        error_payload(&error)["error"]["code"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(
        refused(json!({ "action": "mark_reviewed", "observation_id": 9999 })),
        "observation_not_found"
    );
    assert_eq!(refused(json!({ "action": "sharpen" })), "invalid_params");
    assert_eq!(
        refused(json!({ "action": "mark_reviewed" })),
        "invalid_params",
        "marking needs to say which memory"
    );
}

#[test]
fn a_save_that_met_another_writer_says_to_ask_again() {
    let (_temp, server) = test_server(McpOptions::default());
    let database = {
        let mut store = server.lock_store().unwrap();
        store.enroll_project("leteo").unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        store.database_path().to_path_buf()
    };
    let holder = rusqlite::Connection::open(&database).unwrap();
    holder
        .busy_timeout(std::time::Duration::from_secs(30))
        .unwrap();
    holder
        .execute_batch("BEGIN IMMEDIATE; UPDATE sessions SET directory = 'held';")
        .unwrap();

    let save = |title: &str| {
        server.mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "s1", "title": title, "content": "the body of it",
            }))
            .unwrap(),
        ))
    };
    let Err(error) = save("While somebody else was writing") else {
        panic!("a write cannot land while another process holds the lock");
    };
    let payload = error_payload(&error);
    assert_eq!(payload["error"]["code"], "store_busy");
    let message = payload["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("Try the same call again"),
        "a refusal with a remedy has to say the remedy: {message}"
    );
    assert!(
        !message.contains("  "),
        "the source's own indentation is not part of the sentence: {message:?}"
    );

    holder.execute_batch("ROLLBACK").unwrap();
    assert!(
        save("While somebody else was writing").is_ok(),
        "the remedy the message names has to be the remedy"
    );
}

#[test]
fn the_skill_says_what_to_do_with_a_busy_store() {
    for bundle in [
        "plugin/claude-code/skills/memory/SKILL.md",
        "plugin/codex/skills/memory/SKILL.md",
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(bundle);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert!(
            text.contains("store_busy"),
            "{bundle} never names the code an agent is meant to retry"
        );
        assert!(
            text.contains("Make the same call again"),
            "{bundle} names the code without saying what to do about it"
        );
    }
}

#[test]
fn no_sentence_an_agent_reads_carries_the_source_it_was_written_in() {
    let settings = crate::settings::Settings::default();
    let sentences: Vec<(&str, String)> = vec![
        ("server instructions", SERVER_INSTRUCTIONS.to_owned()),
        (
            "no match hint",
            crate::mcp::output::NO_MATCH_HINT.to_owned(),
        ),
        (
            "partial match hint",
            crate::mcp::output::PARTIAL_MATCH_HINT.to_owned(),
        ),
        (
            "semantic match hint",
            crate::mcp::output::SEMANTIC_MATCH_HINT.to_owned(),
        ),
        (
            "nothing extracted hint",
            crate::mcp::output::NOTHING_EXTRACTED_HINT.to_owned(),
        ),
        (
            "unnamed summary hint",
            crate::mcp::output::UNNAMED_SUMMARY_HINT.to_owned(),
        ),
        (
            "memory directive",
            crate::setup::MEMORY_DIRECTIVE.to_owned(),
        ),
        ("language directive", settings.language_directive()),
    ];
    let mut offenders = Vec::new();
    for (name, sentence) in sentences {
        for line in sentence.lines() {
            if line.trim_start().contains("   ") {
                offenders.push(format!("{name}: {line:?}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these sentences carry their own source code:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn a_summary_with_no_headline_is_reported_rather_than_quietly_unnamed() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store.enroll_project("leteo").unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
    }
    let summarise = |content: &str| {
        server
            .mem_session_summary(Parameters(
                serde_json::from_value(json!({ "session_id": "s1", "content": content })).unwrap(),
            ))
            .unwrap()
            .0
    };

    let nameless = summarise("## Objetivo\n2026-08-05");
    assert_eq!(
        nameless.observation.title, "Session summary: leteo",
        "there was nothing in it to take a name from"
    );
    let hint = nameless
        .hint
        .expect("a memory nobody can find has to say so");
    assert!(hint.contains("mem_update"), "{hint}");
    assert!(
        !hint.contains("   "),
        "and the sentence is a sentence: {hint:?}"
    );

    let named = summarise("## Goal\nTeach the opening block to fold summaries onto sessions\n");
    assert_eq!(
        named.observation.title,
        "Teach the opening block to fold summaries onto sessions"
    );
    assert!(named.hint.is_none(), "{:?}", named.hint);
}

#[test]
fn a_suggested_topic_key_is_the_key_a_search_looks_for() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store.enroll_project("leteo").unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
    }
    let title = "Una decisión sobre almacenamiento";
    let suggested = server
        .mem_suggest_topic_key(Parameters(
            serde_json::from_value(json!({ "title": title, "type": "decision" })).unwrap(),
        ))
        .unwrap()
        .0
        .topic_key;
    assert!(
        !suggested.contains("decisi-n"),
        "an accent keeps its letter: {suggested}"
    );

    server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "s1", "title": title, "type": "decision",
                "content": "we chose sqlite and this is the note about it",
                "topic_key": suggested,
            }))
            .unwrap(),
        ))
        .unwrap();
    let found = server
        .mem_search(Parameters(
            serde_json::from_value(json!({ "query": suggested })).unwrap(),
        ))
        .unwrap()
        .0;
    assert_eq!(
        found.count, 1,
        "the key the tool gave has to be the key the search normalises to"
    );
    assert_eq!(found.results[0].observation.title, title);
}

#[test]
fn the_descriptions_publish_the_preview_length_the_code_cuts_at() {
    let published = format!("{PREVIEW_BYTES}-byte preview");
    let mut saying = 0;
    for tool in LeteoMcpServer::router().list_all() {
        let Some(description) = tool.description.as_ref() else {
            continue;
        };
        assert!(
            !description.contains("character preview"),
            "{} promises a unit the cut does not use; the preview is cut in bytes and it says {description:?}",
            tool.name
        );
        if !description.contains("preview marked") {
            continue;
        }
        assert!(
            description.contains(&published),
            "{} promises a preview of a length the code does not cut at; it says {description:?}",
            tool.name
        );
        saying += 1;
    }
    for expected in [
        "mem_search",
        "mem_context",
        "mem_timeline",
        "mem_review",
        "mem_update",
        "mem_save_prompt",
        "mem_judge",
        "mem_session_end",
        "mem_consolidate",
    ] {
        assert!(
            LeteoMcpServer::router().list_all().iter().any(|tool| {
                tool.name == expected
                    && tool
                        .description
                        .as_ref()
                        .is_some_and(|text| text.contains(&published))
            }),
            "{expected} returns previews and does not say so"
        );
    }
    assert_eq!(
        saying, 9,
        "a tool started or stopped previewing and the list above did not move"
    );
}

#[test]
fn a_type_outside_the_vocabulary_is_filed_where_a_filter_can_reach_it() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store
            .create_session("unfiled", "leteo", "C:/workspace")
            .unwrap();
    }

    let folded = server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "unfiled",
                "title": "Delay-loading seven DLLs",
                "content": "the loader resolves them on first call",
                "type": "optimization",
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    assert_eq!(
        folded.observation.kind, "discovery",
        "a word no filter can name folds onto one it can"
    );
    assert_eq!(
        folded.hint, None,
        "and there is nothing to warn about: the memory is reachable"
    );

    for (kind, stored) in [
        ("discovery", "discovery"),
        ("bug", "bugfix"),
        ("implementation", "discovery"),
    ] {
        let filed = server
            .mem_save(Parameters(
                serde_json::from_value(json!({
                    "session_id": "unfiled",
                    "title": format!("A memory typed {kind}"),
                    "content": "with a body of its own",
                    "type": kind,
                }))
                .unwrap(),
            ))
            .unwrap()
            .0;
        assert_eq!(filed.observation.kind, stored);
        assert_eq!(filed.hint, None, "{kind} is reachable by filter");
    }

    let summary = server
        .mem_session_summary(Parameters(
            serde_json::from_value(json!({
                "session_id": "unfiled",
                "content": "What the session was for\n\nand what it did",
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    assert_eq!(summary.observation.kind, "session_summary");
    assert_eq!(summary.hint, None);
}

#[test]
fn the_opening_context_dates_a_session_by_its_activity_and_quotes_a_prompt_plainly() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store
            .create_session("old", "leteo", "C:/workspace")
            .unwrap();
        store
            .connection()
            .execute(
                "UPDATE sessions SET started_at = '2026-07-28 12:51:33' WHERE id = 'old'",
                [],
            )
            .unwrap();
        store
            .add_observation(crate::AddObservation {
                session_id: "old".to_owned(),
                kind: "discovery".to_owned(),
                title: "A memory saved much later".to_owned(),
                content: "and it is what dates the session".to_owned(),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
        store
            .add_prompt(crate::AddPrompt {
                session_id: "old".to_owned(),
                content: "why does the widened stage cost so much".to_owned(),
                project: Some("leteo".to_owned()),
            })
            .unwrap();
    }

    let context = server
        .mem_context(Parameters(
            serde_json::from_value(json!({ "project": "leteo" })).unwrap(),
        ))
        .unwrap()
        .0;

    let session = context.sessions.first().expect("the session is listed");
    assert_ne!(
        session.last_activity, "2026-07-28 12:51:33",
        "a session that saved a memory today is not a session from July"
    );

    let listed = serde_json::to_value(context.prompts.first().expect("the prompt is listed"))
        .expect("a prompt serialises");
    let fields: Vec<&str> = listed
        .as_object()
        .expect("a prompt is an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        fields,
        vec!["content", "created_at"],
        "no tool takes a prompt's id, sync_id, session_id or project"
    );
}

#[test]
fn the_warning_against_a_memory_is_in_the_same_place_everywhere() {
    let (_temp, server) = test_server(McpOptions::default());
    let (older, newer) = {
        let mut store = server.lock_store().unwrap();
        store.create_session("s1", "leteo", "C:/workspace").unwrap();
        let memory = |title: &str, content: &str| AddObservation {
            session_id: "s1".to_owned(),
            kind: "decision".to_owned(),
            title: title.to_owned(),
            content: content.to_owned(),
            tool_name: None,
            project: Some("leteo".to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            prompt_sync_id: None,
        };
        let older = store
            .add_observation(memory("Tabs everywhere", "the indentation decision"))
            .unwrap()
            .observation;
        let newer = store
            .add_observation(memory(
                "Spaces everywhere",
                "the indentation decision again",
            ))
            .unwrap()
            .observation;
        let relation = store
            .save_relation(crate::memory::model::SaveRelationParams {
                sync_id: crate::memory::normalize::sync_id("rel"),
                source_id: newer.sync_id.clone(),
                target_id: older.sync_id.clone(),
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
        (older, newer)
    };
    let _ = newer;

    let searched = server
        .mem_search(Parameters(
            serde_json::from_value(json!({ "query": "Tabs everywhere", "project": "leteo" }))
                .unwrap(),
        ))
        .unwrap()
        .0;
    assert!(
        searched
            .results
            .iter()
            .all(|result| result.observation.id != older.id),
        "mem_search hands back no superseded memory to warn about: {:?}",
        searched.results
    );

    let read = server
        .mem_get_observation(Parameters(
            serde_json::from_value(json!({ "id": older.id })).unwrap(),
        ))
        .unwrap()
        .0;
    assert_eq!(read.observation.caveats.len(), 1, "mem_get_observation");

    let updated = server
        .mem_update(Parameters(
            serde_json::from_value(json!({
                "id": older.id,
                "expected_project": "leteo",
                "content": "revised body",
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    assert_eq!(updated.observation.caveats.len(), 1, "mem_update");

    {
        let store = server.lock_store().unwrap();
        store
            .connection()
            .execute(
                "UPDATE observations SET review_after = datetime('now', '-1 day') WHERE id = ?1",
                rusqlite::params![older.id],
            )
            .unwrap();
    }
    let queued = server
        .mem_review(Parameters(
            serde_json::from_value(json!({ "action": "list", "limit": 20 })).unwrap(),
        ))
        .unwrap()
        .0;
    let due = queued
        .observations
        .iter()
        .find(|observation| observation.id == older.id)
        .expect("the overturned decision is due for rereading");
    assert_eq!(due.caveats.len(), 1, "mem_review");

    let timeline = server
        .mem_timeline(Parameters(
            serde_json::from_value(json!({ "observation_id": older.id })).unwrap(),
        ))
        .unwrap()
        .0;
    assert_eq!(timeline.focus.caveats.len(), 1, "mem_timeline");

    for (tool, rendered) in [
        ("mem_get_observation", serde_json::to_value(&read).unwrap()),
        ("mem_update", serde_json::to_value(&updated).unwrap()),
    ] {
        assert!(
            rendered["observation"].get("caveats").is_some(),
            "{tool} must carry the caveat on the memory: {rendered}"
        );
        assert!(
            rendered.get("caveats").is_none(),
            "{tool} must not carry it beside the memory too: {rendered}"
        );
    }
}

#[test]
fn a_prompt_or_a_summary_listed_as_context_is_previewed_like_the_rest() {
    let (_temp, server) = test_server(McpOptions::default());
    let pasted = format!(
        "why does this fail {}",
        "and a long tail of pasted log ".repeat(200)
    );
    assert!(pasted.len() > PREVIEW_BYTES * 4);
    {
        let mut store = server.lock_store().unwrap();
        store.create_session("s1", "leteo", "C:/workspace").unwrap();
        store
            .add_prompt(crate::AddPrompt {
                session_id: "s1".to_owned(),
                content: pasted.clone(),
                project: Some("leteo".to_owned()),
            })
            .unwrap();
        store
            .add_observation(crate::AddObservation {
                session_id: "s1".to_owned(),
                kind: "discovery".to_owned(),
                title: "Something to make the context non-empty".to_owned(),
                content: "a body".to_owned(),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
    }

    let context = server
        .mem_context(Parameters(
            serde_json::from_value(json!({ "project": "leteo" })).unwrap(),
        ))
        .unwrap()
        .0;

    let listed = context.prompts.first().expect("the prompt is listed");
    assert!(
        listed.content.len() <= PREVIEW_BYTES,
        "a pasted prompt must not be quoted whole: {} bytes",
        listed.content.len()
    );
    assert!(
        listed.content.starts_with("why does this fail"),
        "and it is still the question that was asked: {}",
        listed.content
    );
    assert!(
        listed.content.contains("truncated"),
        "and it says it was cut: {}",
        listed.content
    );

    {
        let mut store = server.lock_store().unwrap();
        store
            .add_prompt(crate::AddPrompt {
                session_id: "s1".to_owned(),
                content: "a short question".to_owned(),
                project: Some("leteo".to_owned()),
            })
            .unwrap();
    }
    let context = server
        .mem_context(Parameters(
            serde_json::from_value(json!({ "project": "leteo" })).unwrap(),
        ))
        .unwrap()
        .0;
    assert!(
        context
            .prompts
            .iter()
            .any(|prompt| prompt.content == "a short question"),
        "{:?}",
        context
            .prompts
            .iter()
            .map(|p| &p.content)
            .collect::<Vec<_>>()
    );

    {
        let mut store = server.lock_store().unwrap();
        store.end_session("s1", Some(&pasted)).unwrap();
    }
    let context = server
        .mem_context(Parameters(
            serde_json::from_value(json!({ "project": "leteo" })).unwrap(),
        ))
        .unwrap()
        .0;
    let session = context
        .sessions
        .iter()
        .find(|session| session.id == "s1")
        .expect("the session is listed");
    let summary = session.summary.as_deref().expect("it kept a summary");
    assert!(
        summary.len() <= PREVIEW_BYTES,
        "a pasted summary must not be quoted whole: {} bytes",
        summary.len()
    );
    assert!(
        summary.contains("truncated"),
        "and it says it was cut: {summary}"
    );
}

#[test]
fn the_project_list_is_bounded_and_says_what_it_left_out() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = crate::store::StoreConfig::new(temp.path().join("mcp.db"));
    config.max_context_results = 3;
    let mut store = Store::open(config).unwrap();
    for index in 0..5 {
        let project = format!("project-{index}");
        store
            .create_session(&format!("s{index}"), &project, "C:/repo")
            .unwrap();
        store
            .add_observation(crate::memory::model::AddObservation {
                session_id: format!("s{index}"),
                kind: "decision".to_owned(),
                title: "Una memoria".to_owned(),
                content: "el cuerpo".to_owned(),
                tool_name: None,
                project: Some(project),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
    }
    let server = LeteoMcpServer::with_options(Arc::new(Mutex::new(store)), McpOptions::default());
    let Json(stats) = server.mem_stats(Parameters(NoParams {})).unwrap();
    assert_eq!(stats.projects.len(), 3, "the ceiling is applied: {stats:?}");
    assert_eq!(
        stats.projects_omitted, 2,
        "and what it cut is counted, so a bounded list is not read as the whole store: {stats:?}"
    );
    assert_eq!(
        stats.total_observations, 5,
        "the totals still count the whole store: {stats:?}"
    );
    assert!(
        stats
            .projects
            .iter()
            .all(|project| project.observation_count == 1 && project.session_count == 1),
        "each listed project carries its own counts: {stats:?}"
    );
}

#[test]
fn the_seven_tools_that_had_no_test_of_their_own_answer_what_they_promise() {
    let temp = tempfile::tempdir().unwrap();
    let mut store =
        Store::open(crate::store::StoreConfig::new(temp.path().join("mcp.db"))).unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let saved = |store: &mut Store, title: &str| {
        store
            .add_observation(crate::memory::model::AddObservation {
                session_id: "s1".to_owned(),
                kind: "decision".to_owned(),
                title: title.to_owned(),
                content: format!("el cuerpo de {title}"),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap()
            .observation
            .id
    };
    let first = saved(&mut store, "Una decision");
    let second = saved(&mut store, "Otra decision");
    let server = LeteoMcpServer::with_options(
        Arc::new(Mutex::new(store)),
        McpOptions {
            default_project: Some("leteo".to_owned()),
            ..McpOptions::default()
        },
    );

    let owned = || "leteo".to_owned();
    let Json(pinned) = server
        .mem_pin(Parameters(PinParams {
            id: first,
            expected_project: owned(),
        }))
        .unwrap();
    assert!(pinned.pinned, "{pinned:?}");
    server
        .mem_pin(Parameters(PinParams {
            id: first,
            expected_project: owned(),
        }))
        .unwrap();
    let Json(unpinned) = server
        .mem_unpin(Parameters(PinParams {
            id: first,
            expected_project: owned(),
        }))
        .unwrap();
    assert!(!unpinned.pinned, "{unpinned:?}");
    assert!(
        server
            .mem_pin(Parameters(PinParams {
                id: 9_999,
                expected_project: owned(),
            }))
            .is_err(),
        "pinning a memory that does not exist has to fail"
    );

    let Json(stats) = server.mem_stats(Parameters(NoParams {})).unwrap();
    assert_eq!(stats.total_observations, 2, "{stats:?}");
    assert_eq!(stats.total_sessions, 1, "{stats:?}");
    assert_eq!(stats.projects.len(), 1, "{stats:?}");
    assert_eq!(stats.projects[0].name, "leteo", "{stats:?}");
    assert_eq!(stats.projects[0].observation_count, 2, "{stats:?}");
    assert_eq!(stats.projects[0].session_count, 1, "{stats:?}");
    assert_eq!(stats.projects_omitted, 0, "{stats:?}");

    let Json(report) = server
        .mem_doctor(Parameters(DoctorParams {
            project: None,
            check: None,
        }))
        .unwrap();
    assert!(report.healthy, "{report:?}");
    assert_eq!(
        report.checks.len(),
        crate::memory::model::DoctorCheck::CODES.len()
    );
    assert!(
        server
            .mem_doctor(Parameters(DoctorParams {
                project: None,
                check: Some("no_es_una_comprobacion".to_owned()),
            }))
            .is_err()
    );

    let Json(ended) = server
        .mem_session_end(Parameters(SessionEndParams {
            id: "s1".to_owned(),
            summary: Some("Cerrada. <private>secreto</private> Fin.".to_owned()),
        }))
        .unwrap();
    let summary = ended.session.summary.clone().unwrap_or_default();
    assert!(!summary.contains("secreto"), "{summary:?}");
    assert!(ended.session.ended_at.is_some(), "{:?}", ended.session);

    assert!(
        server
            .mem_merge_projects(Parameters(MergeProjectsParams {
                from: "  ,  ".to_owned(),
                to: "leteo".to_owned(),
            }))
            .is_err(),
        "a list of nothing is a caller's mistake, not a merge of nothing"
    );

    let Json(soft) = server
        .mem_delete(Parameters(DeleteParams {
            id: second,
            expected_project: owned(),
            hard_delete: false,
        }))
        .unwrap();
    assert_eq!(soft.status, "soft_deleted");
    let Json(found) = server
        .mem_get_observation(Parameters(GetObservationParams {
            id: second,
            include_history: false,
        }))
        .unwrap();
    assert_eq!(
        found.observation.state, "deleted",
        "{:?}",
        found.observation
    );
    assert!(
        server
            .mem_delete(Parameters(DeleteParams {
                id: second,
                expected_project: owned(),
                hard_delete: false,
            }))
            .is_err()
    );
    let Json(hard) = server
        .mem_delete(Parameters(DeleteParams {
            id: second,
            expected_project: owned(),
            hard_delete: true,
        }))
        .unwrap();
    assert_eq!(hard.status, "deleted");
}

#[test]
fn an_empty_search_says_whether_the_words_or_the_directory_emptied_it() {
    let temp = tempfile::tempdir().unwrap();
    let mut store =
        Store::open(crate::store::StoreConfig::new(temp.path().join("mcp.db"))).unwrap();
    store
        .create_session("s1", "otro-proyecto", "C:/otro")
        .unwrap();
    store
        .add_observation(crate::memory::model::AddObservation {
            session_id: "s1".to_owned(),
            kind: "decision".to_owned(),
            title: "Elegimos zurriagazo".to_owned(),
            content: "por lo del zurriagazo".to_owned(),
            tool_name: None,
            project: Some("otro-proyecto".to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            prompt_sync_id: None,
        })
        .unwrap();

    let server = LeteoMcpServer::with_options(
        Arc::new(Mutex::new(store)),
        McpOptions {
            default_project: Some("leteo".to_owned()),
            ..McpOptions::default()
        },
    );
    let ask = |query: &str| {
        let Json(output) = server
            .mem_search(Parameters(SearchParams {
                query: query.to_owned(),
                kind: None,
                project: None,
                all_projects: false,
                scope: None,
                limit: None,
                match_mode: MatchMode::default(),
            }))
            .unwrap();
        (output.count, output.hint.unwrap_or_default())
    };

    let (count, hint) = ask("zurriagazo");
    assert_eq!(count, 0, "the memory is in another project");
    assert!(
        hint.contains("elsewhere") && hint.contains("all_projects"),
        "the directory is what emptied this, and the hint has to say so: {hint:?}"
    );
    assert!(
        hint.contains("leteo"),
        "and name the project it looked in: {hint:?}"
    );

    let (count, hint) = ask("garrapinada");
    assert_eq!(count, 0);
    assert!(
        !hint.contains("elsewhere"),
        "a question that comes back empty either way is not a directory problem: {hint:?}"
    );
    assert!(hint.contains("Full-text search"), "{hint:?}");
}

/// `mem_search` reaches the semantic stage through the setting, labels what it
/// finds, and the retry that counts "elsewhere" runs the stage as well — so the
/// number it reports is the number `all_projects` would return, which is the
/// promise the sentence makes.
#[test]
fn mem_search_labels_what_was_found_by_meaning_and_the_setting_turns_it_off() {
    let Some(model) = crate::semantic::tests::repository_model() else {
        eprintln!("skipped: this tree has no assets/model");
        return;
    };
    let temp = tempfile::tempdir().unwrap();
    let mut config = crate::store::StoreConfig::new(temp.path().join("mcp.db"));
    config.model_dir = Some(model);
    let mut store = Store::open(config).unwrap();
    store
        .create_session("s1", "otro-proyecto", "C:/otro")
        .unwrap();
    store
        .add_observation(crate::memory::model::AddObservation {
            session_id: "s1".to_owned(),
            kind: "decision".to_owned(),
            title: "Rotate JWT signing keys every 30 days with kid header".to_owned(),
            content: "Signing keys live in KMS and tokens carry a kid header.".to_owned(),
            tool_name: None,
            project: Some("otro-proyecto".to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            prompt_sync_id: None,
        })
        .unwrap();
    let server = LeteoMcpServer::with_options(
        Arc::new(Mutex::new(store)),
        McpOptions {
            default_project: Some("leteo".to_owned()),
            ..McpOptions::default()
        },
    );
    let ask = |all_projects: bool| {
        let Json(output) = server
            .mem_search(Parameters(SearchParams {
                query: "rotación de las claves de firma".to_owned(),
                kind: None,
                project: None,
                all_projects,
                scope: None,
                limit: None,
                match_mode: MatchMode::default(),
            }))
            .unwrap();
        output
    };

    let everywhere = ask(true);
    assert_eq!(everywhere.count, 1);
    assert!(everywhere.results[0].semantic, "{:?}", everywhere.results);
    assert_eq!(
        everywhere.hint.as_deref(),
        Some(crate::mcp::output::SEMANTIC_MATCH_HINT)
    );

    let here = ask(false);
    assert_eq!(here.count, 0, "the memory is in another project");
    let hint = here.hint.unwrap_or_default();
    assert!(
        hint.contains("elsewhere") && hint.contains("all_projects"),
        "the retry runs the stage, so it finds what `all_projects` would: {hint:?}"
    );

    let database = server.lock_store().unwrap().database_path().to_owned();
    std::fs::write(
        database.parent().unwrap().join("settings.json"),
        r#"{"semantic_search": false}"#,
    )
    .unwrap();
    let off = ask(true);
    assert_eq!(off.count, 0, "the setting is read on every search");
    assert!(
        off.hint.unwrap_or_default().contains("Full-text search"),
        "and an empty answer is the lexical one"
    );
}

/// On a `nearest` page the words found some rows and the stage added others, and
/// the one sentence on the page has to be true of both: it must not say the rows
/// the words found were found by meaning.
#[test]
fn a_mixed_page_is_not_told_that_every_row_was_found_by_meaning() {
    let (_temp, server) = test_server(McpOptions::default());
    let store = server.lock_store().unwrap();
    let observation = |id: i64, partial: bool, semantic: bool| crate::memory::model::SearchResult {
        observation: crate::memory::model::Observation {
            id,
            sync_id: format!("obs-{id}"),
            session_id: "s".to_owned(),
            kind: "discovery".to_owned(),
            title: format!("t{id}"),
            content: "c".to_owned(),
            tool_name: None,
            project: Some("leteo".to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            revision_count: 1,
            duplicate_count: 1,
            last_seen_at: None,
            review_after: None,
            prompt_sync_id: None,
            pinned: false,
            created_at: "2026-01-01 00:00:00".to_owned(),
            updated_at: "2026-01-01 00:00:00".to_owned(),
            deleted_at: None,
        },
        rank: -0.01,
        partial,
        semantic,
    };
    drop(store);
    let output = crate::mcp::output::SearchOutput::new(
        vec![observation(1, true, false), observation(2, false, true)],
        ProjectEnvelope::default(),
        false,
        &Default::default(),
        None,
        false,
        Vec::new(),
    );
    let hint = output.hint.expect("a page the stage touched says so");
    assert!(
        !hint.contains("these were found by meaning"),
        "that is false of the row the words found: {hint}"
    );
    assert!(
        hint.contains("marked semantic") && hint.contains("any other matched only some"),
        "{hint}"
    );
}

#[test]
fn an_empty_context_says_whether_the_store_or_the_directory_is_empty() {
    let temp = tempfile::tempdir().unwrap();
    let mut store =
        Store::open(crate::store::StoreConfig::new(temp.path().join("mcp.db"))).unwrap();
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
    let server = LeteoMcpServer::with_options(
        Arc::new(Mutex::new(store)),
        McpOptions {
            default_project: Some("leteo".to_owned()),
            ..McpOptions::default()
        },
    );
    let context = |project: Option<&str>, all_projects: bool| {
        let Json(output) = server
            .mem_context(Parameters(ContextParams {
                project: project.map(str::to_owned),
                all_projects,
                scope: None,
                limit: None,
                session_limit: 5,
                prompt_limit: 5,
                byte_limit: None,
            }))
            .unwrap();
        output
    };

    let empty = context(None, false);
    assert_eq!(empty.count, 0, "leteo holds nothing in this store");
    let hint = empty.hint.clone().unwrap_or_default();
    assert!(
        hint.contains("elsewhere") && hint.contains("leteo"),
        "an empty context has to say which of its two reasons it is: {hint:?}"
    );

    assert!(context(Some("leteo"), false).hint.is_none());
    assert!(context(None, true).hint.is_none());

    let answered = context(Some("otro-proyecto"), false);
    assert_eq!(answered.count, 1);
    assert!(answered.hint.is_none(), "{:?}", answered.hint);
}

#[test]
fn a_page_that_ended_because_the_caller_asked_for_that_many_says_so() {
    let temp = tempfile::tempdir().unwrap();
    let mut store =
        Store::open(crate::store::StoreConfig::new(temp.path().join("mcp.db"))).unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    for index in 0..6 {
        store
            .add_observation(crate::memory::model::AddObservation {
                session_id: "s1".to_owned(),
                kind: "decision".to_owned(),
                title: format!("Zurriagazo numero {index}"),
                content: format!("el cuerpo del zurriagazo {index}"),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
    }
    let server = LeteoMcpServer::with_options(
        Arc::new(Mutex::new(store)),
        McpOptions {
            default_project: Some("leteo".to_owned()),
            ..McpOptions::default()
        },
    );
    let ask = |limit: Option<usize>| {
        let Json(output) = server
            .mem_search(Parameters(SearchParams {
                query: "zurriagazo".to_owned(),
                kind: None,
                project: None,
                all_projects: false,
                scope: None,
                limit,
                match_mode: MatchMode::default(),
            }))
            .unwrap();
        (output.count, output.hint.unwrap_or_default())
    };

    let (count, hint) = ask(Some(4));
    assert_eq!(count, 4);
    assert!(
        hint.contains("More matched than were returned"),
        "a page cut by the caller's own limit has to say so: {hint:?}"
    );

    let (count, hint) = ask(Some(6));
    assert_eq!(count, 6);
    assert!(hint.is_empty(), "{hint:?}");

    let (count, hint) = ask(Some(20));
    assert_eq!(count, 6);
    assert!(hint.is_empty(), "{hint:?}");
}

#[test]
fn every_field_called_count_says_which_question_it_answers() {
    let mut offenders = Vec::new();
    let mut examined = 0;
    for tool in LeteoMcpServer::with_options(
        Arc::new(Mutex::new(
            Store::open(crate::store::StoreConfig::new(
                tempfile::tempdir().unwrap().path().join("mcp.db"),
            ))
            .unwrap(),
        )),
        McpOptions::default(),
    )
    .router
    .list_all()
    {
        let Some(schema) = tool.output_schema.clone() else {
            continue;
        };
        let schema = serde_json::Value::Object((*schema).clone());
        let Some(properties) = schema.get("properties").and_then(|value| value.as_object()) else {
            continue;
        };
        for (name, property) in properties {
            if name != "count" {
                continue;
            }
            examined += 1;
            let described = property
                .get("description")
                .and_then(|value| value.as_str())
                .is_some_and(|value| !value.trim().is_empty());
            if !described {
                offenders.push(tool.name.to_string());
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these tools answer with a bare `count` and never say what it counts: {offenders:?}"
    );
    assert!(
        examined >= 3,
        "only {examined} `count` fields were examined, so this guard is checking nothing"
    );
}

#[test]
fn no_tool_answers_with_the_whole_of_what_it_was_given() {
    const HUGE: usize = 20_000;
    const ROOM: usize = 8_000;

    let temp = tempfile::tempdir().unwrap();
    let mut store =
        Store::open(crate::store::StoreConfig::new(temp.path().join("mcp.db"))).unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let long = "una frase larga que se repite ".repeat(HUGE / 30);
    let saved = store
        .add_observation(crate::memory::model::AddObservation {
            session_id: "s1".to_owned(),
            kind: "decision".to_owned(),
            title: "Zurriagazo".to_owned(),
            content: long.clone(),
            tool_name: None,
            project: Some("leteo".to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            prompt_sync_id: None,
        })
        .unwrap()
        .observation
        .id;
    store
        .add_prompt(crate::memory::model::AddPrompt {
            session_id: "s1".to_owned(),
            content: long.clone(),
            project: Some("leteo".to_owned()),
        })
        .unwrap();
    let server = LeteoMcpServer::with_options(
        Arc::new(Mutex::new(store)),
        McpOptions {
            default_project: Some("leteo".to_owned()),
            ..McpOptions::default()
        },
    );

    let mut sizes: Vec<(&str, usize)> = Vec::new();
    let mut note = |name: &'static str, value: serde_json::Value| {
        // The reply as the server sends it, which is the answer twice: the JSON
        // in `structuredContent` and the same JSON again in the text block, so
        // a client that reads only text still gets it. Weighed here rather than
        // in a copy of the reply written for this test, so a surface that
        // starts echoing what it was given fails on the shape production uses.
        let entero = rmcp::model::CallToolResult::structured(value);
        sizes.push((
            name,
            serde_json::to_string(&entero).unwrap_or_default().len(),
        ));
    };

    note(
        "mem_save",
        serde_json::to_value(
            server
                .mem_save(Parameters(
                    serde_json::from_value(json!({
                        "title": "otra", "content": long, "type": "decision",
                    }))
                    .unwrap(),
                ))
                .unwrap()
                .0,
        )
        .unwrap(),
    );
    note(
        "mem_update",
        serde_json::to_value(
            server
                .mem_update(Parameters(
                    serde_json::from_value(json!({
                        "id": saved,
                        "expected_project": "leteo",
                        "title": "nuevo",
                    }))
                    .unwrap(),
                ))
                .unwrap()
                .0,
        )
        .unwrap(),
    );
    note(
        "mem_save_prompt",
        serde_json::to_value(
            server
                .mem_save_prompt(Parameters(
                    serde_json::from_value(json!({ "session_id": "s1", "content": long })).unwrap(),
                ))
                .unwrap()
                .0,
        )
        .unwrap(),
    );
    note(
        "mem_session_end",
        serde_json::to_value(
            server
                .mem_session_end(Parameters(
                    serde_json::from_value(json!({ "id": "s1", "summary": long })).unwrap(),
                ))
                .unwrap()
                .0,
        )
        .unwrap(),
    );
    note(
        "mem_search",
        serde_json::to_value(
            server
                .mem_search(Parameters(
                    serde_json::from_value(json!({ "query": "zurriagazo" })).unwrap(),
                ))
                .unwrap()
                .0,
        )
        .unwrap(),
    );
    note(
        "mem_context",
        serde_json::to_value(
            server
                .mem_context(Parameters(serde_json::from_value(json!({})).unwrap()))
                .unwrap()
                .0,
        )
        .unwrap(),
    );
    note(
        "mem_timeline",
        serde_json::to_value(
            server
                .mem_timeline(Parameters(
                    serde_json::from_value(json!({ "observation_id": saved })).unwrap(),
                ))
                .unwrap()
                .0,
        )
        .unwrap(),
    );

    let offenders: Vec<&(&str, usize)> = sizes.iter().filter(|(_, size)| *size > ROOM).collect();
    assert!(
        offenders.is_empty(),
        "these answered with what they were given rather than a preview of it: {offenders:?}"
    );

    let whole = server
        .mem_get_observation(Parameters(
            serde_json::from_value(json!({ "id": saved })).unwrap(),
        ))
        .unwrap()
        .0;
    assert!(
        whole.observation.content.len() > ROOM,
        "mem_get_observation is the one tool that hands a body over whole"
    );
}

#[test]
fn no_write_door_lets_a_private_marker_reach_the_database() {
    const SECRET: &str = "zurriagazoindiscreto";
    let hidden = |field: &str| format!("{field} <private>{SECRET}</private> visible");

    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("mcp.db");
    let mut store = Store::open(crate::store::StoreConfig::new(path.clone())).unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let server = LeteoMcpServer::with_options(
        Arc::new(Mutex::new(store)),
        McpOptions {
            default_project: Some("leteo".to_owned()),
            ..McpOptions::default()
        },
    );

    let first = server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "s1",
                "title": hidden("uno"),
                "content": hidden("cuerpo uno"),
                "type": "decision",
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    let second = server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "s1",
                "title": hidden("dos"),
                "content": hidden("cuerpo dos"),
                "type": "decision",
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    let third = server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "s1",
                "title": hidden("tres"),
                "content": hidden("cuerpo tres"),
                "type": "decision",
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    server
        .mem_update(Parameters(
            serde_json::from_value(json!({
                "id": first.observation.id,
                "expected_project": "leteo",
                "title": hidden("revisado"),
                "content": hidden("cuerpo revisado"),
            }))
            .unwrap(),
        ))
        .unwrap();
    server
        .mem_save_prompt(Parameters(
            serde_json::from_value(json!({ "session_id": "s1", "content": hidden("pregunta") }))
                .unwrap(),
        ))
        .unwrap();
    server
        .mem_session_summary(Parameters(
            serde_json::from_value(json!({ "session_id": "s1", "content": hidden("resumen") }))
                .unwrap(),
        ))
        .unwrap();
    server
        .mem_capture_passive(Parameters(
            serde_json::from_value(json!({
                "session_id": "s1",
                "content": hidden("captura"),
                "source": "prueba",
            }))
            .unwrap(),
        ))
        .unwrap();
    let compared = server
        .mem_compare(Parameters(
            serde_json::from_value(json!({
                "memory_id_a": first.observation.id,
                "memory_id_b": third.observation.id,
                "relation": "related",
                "reasoning": hidden("porque"),
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    let other = server
        .mem_compare(Parameters(
            serde_json::from_value(json!({
                "memory_id_a": first.observation.id,
                "memory_id_b": second.observation.id,
                "relation": "related",
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    server
        .mem_judge(Parameters(
            serde_json::from_value(json!({
                "judgment_id": other.sync_id,
                "relation": "related",
                "reason": hidden("razon juzgada"),
                "evidence": hidden("evidencia juzgada"),
            }))
            .unwrap(),
        ))
        .unwrap();
    let _ = &compared;
    server
        .mem_session_end(Parameters(
            serde_json::from_value(json!({ "id": "s1", "summary": hidden("cierre") })).unwrap(),
        ))
        .unwrap();

    let reader = rusqlite::Connection::open(&path).unwrap();
    let tables: Vec<String> = reader
        .prepare("SELECT name FROM sqlite_master WHERE type IN ('table')")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|name| !name.starts_with("sqlite_"))
        .collect();
    let carrying = |needle: &str| -> Vec<String> {
        let mut found = Vec::new();
        for table in &tables {
            let Ok(mut statement) = reader.prepare(&format!("SELECT * FROM {table}")) else {
                continue;
            };
            let columns = statement.column_count();
            let Ok(rows) = statement.query_map([], |row| {
                let mut carried = false;
                for index in 0..columns {
                    if let Ok(text) = row.get::<_, String>(index)
                        && text.contains(needle)
                    {
                        carried = true;
                    }
                }
                Ok(carried)
            }) else {
                continue;
            };
            if rows.filter_map(Result::ok).any(|carried| carried) {
                found.push(table.clone());
            }
        }
        found
    };

    assert!(
        carrying(SECRET).is_empty(),
        "the private marker reached these tables: {:?}",
        carrying(SECRET)
    );
    let visible = carrying("visible");
    assert!(
        visible.len() >= 3,
        "the scan found the surviving half of the text in only {visible:?}, so it would not have \
         found the private half either"
    );
    assert!(
        tables.len() >= 8,
        "only {} tables were read: {tables:?}",
        tables.len()
    );
}

#[test]
fn no_read_tool_answers_from_a_project_nobody_asked_about() {
    const HERE: &str = "aqui";
    const THERE: &str = "alla";
    let temp = tempfile::tempdir().unwrap();
    let mut store =
        Store::open(crate::store::StoreConfig::new(temp.path().join("mcp.db"))).unwrap();
    for (session, project) in [("s-aqui", HERE), ("s-alla", THERE)] {
        store.create_session(session, project, "C:/repo").unwrap();
        store
            .add_observation(crate::memory::model::AddObservation {
                session_id: session.to_owned(),
                kind: "decision".to_owned(),
                title: format!("Zurriagazo de {project}"),
                content: format!("el cuerpo del zurriagazo de {project}"),
                tool_name: None,
                project: Some(project.to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
        store
            .add_prompt(crate::memory::model::AddPrompt {
                session_id: session.to_owned(),
                content: format!("una pregunta sobre zurriagazo en {project}"),
                project: Some(project.to_owned()),
            })
            .unwrap();
    }
    let server = LeteoMcpServer::with_options(
        Arc::new(Mutex::new(store)),
        McpOptions {
            default_project: Some(HERE.to_owned()),
            ..McpOptions::default()
        },
    );

    let search = |value: serde_json::Value| {
        serde_json::to_string(
            &server
                .mem_search(Parameters(serde_json::from_value(value).unwrap()))
                .unwrap()
                .0,
        )
        .unwrap()
    };
    let context = |value: serde_json::Value| {
        serde_json::to_string(
            &server
                .mem_context(Parameters(serde_json::from_value(value).unwrap()))
                .unwrap()
                .0,
        )
        .unwrap()
    };

    let narrowed = [search(json!({ "query": "zurriagazo" })), context(json!({}))];
    for answer in &narrowed {
        assert!(
            answer.contains(HERE),
            "the project standing here has to answer: {answer}"
        );
        assert!(
            !answer.contains(THERE),
            "and the other one must not: {answer}"
        );
    }

    for answer in [
        search(json!({ "query": "zurriagazo", "all_projects": true })),
        context(json!({ "all_projects": true })),
        search(json!({ "query": "zurriagazo", "project": THERE })),
        context(json!({ "project": THERE })),
    ] {
        assert!(
            answer.contains(THERE),
            "an explicit widening has to reach the other project: {answer}"
        );
    }
}

#[test]
fn the_verbs_a_tool_offers_are_the_verbs_the_store_accepts() {
    let described: Vec<String> = LeteoMcpServer::router()
        .list_all()
        .into_iter()
        .filter(|tool| tool.name == "mem_judge" || tool.name == "mem_compare")
        .filter_map(|tool| {
            let schema = serde_json::Value::Object((*tool.input_schema).clone());
            schema
                .get("properties")?
                .get("relation")?
                .get("description")?
                .as_str()
                .map(str::to_owned)
        })
        .collect();
    assert_eq!(described.len(), 2, "both judging tools describe a relation");
    for description in &described {
        for verb in crate::memory::rules::RELATION_VERBS {
            assert!(
                description.contains(verb),
                "a tool offers no way to say {verb}: {description}"
            );
        }
    }

    let refused = crate::store::StoreError::InvalidRelationVerb {
        given: "supersedes_maybe".to_owned(),
    }
    .to_string();
    for verb in crate::memory::rules::RELATION_VERBS {
        assert!(
            refused.contains(verb),
            "the refusal hides {verb}: {refused}"
        );
    }
    assert!(refused.contains("supersedes_maybe"), "{refused}");
}

#[test]
fn every_tool_that_takes_a_vocabulary_names_it() {
    let mut checked = 0;
    for tool in LeteoMcpServer::router().list_all() {
        let schema = serde_json::Value::Object((*tool.input_schema).clone());
        let Some(properties) = schema.get("properties").and_then(|v| v.as_object()) else {
            continue;
        };
        for (field, expected) in [
            ("type", crate::memory::rules::KINDS),
            ("scope", &["project", "personal"][..]),
        ] {
            let Some(described) = properties
                .get(field)
                .and_then(|v| v.get("description"))
                .and_then(|v| v.as_str())
            else {
                continue;
            };
            for word in expected {
                assert!(
                    described.contains(word),
                    "{}.{field} takes a closed vocabulary and never names {word}: {described}",
                    tool.name
                );
            }
            checked += 1;
        }
    }
    assert!(
        checked >= 7,
        "only {checked} vocabulary fields were examined"
    );
}

#[test]
fn an_output_description_says_what_the_value_cannot() {
    let mut described: std::collections::BTreeMap<String, String> = Default::default();
    fn walk(node: &serde_json::Value, into: &mut std::collections::BTreeMap<String, String>) {
        if let Some(properties) = node.get("properties").and_then(|v| v.as_object()) {
            for (name, field) in properties {
                if let Some(text) = field.get("description").and_then(|v| v.as_str()) {
                    into.insert(name.clone(), text.to_owned());
                }
            }
        }
        match node {
            serde_json::Value::Object(map) => map.values().for_each(|v| walk(v, into)),
            serde_json::Value::Array(list) => list.iter().for_each(|v| walk(v, into)),
            _ => {}
        }
    }
    for tool in LeteoMcpServer::router().list_all() {
        if let Some(schema) = tool.output_schema.clone() {
            walk(
                &serde_json::Value::Object((*schema).clone()),
                &mut described,
            );
        }
    }
    assert!(!described.is_empty(), "no output schema was read");

    for name in [
        "pinned",
        "revision_count",
        "duplicate_count",
        "updated_at",
        "content_truncated",
    ] {
        assert!(
            !described.contains_key(name),
            "{name} is described to every agent and the value says it: {:?}",
            described.get(name)
        );
    }
    for name in ["state", "caveats", "count"] {
        let text = described
            .get(name)
            .unwrap_or_else(|| panic!("{name} needs its description: the value cannot say it"));
        assert!(!text.trim().is_empty(), "{name}");
    }
    assert!(
        described["state"].contains("needs_review"),
        "{:?}",
        described["state"]
    );
}

#[test]
fn a_memory_is_attributed_to_a_question_or_to_none() {
    let temp = tempfile::tempdir().unwrap();
    let mut store =
        Store::open(crate::store::StoreConfig::new(temp.path().join("mcp.db"))).unwrap();
    for (session, project) in [("a", "leteo"), ("b", "leteo"), ("c", "otro")] {
        store.create_session(session, project, "C:/repo").unwrap();
    }
    let server = LeteoMcpServer::with_options(
        Arc::new(Mutex::new(store)),
        McpOptions {
            default_project: Some("leteo".to_owned()),
            ..McpOptions::default()
        },
    );

    let ask = |session: &str, text: &str| {
        server
            .mem_save_prompt(Parameters(
                serde_json::from_value(json!({ "session_id": session, "content": text })).unwrap(),
            ))
            .unwrap()
            .0
            .prompt
            .sync_id
    };
    let save = |value: serde_json::Value| {
        server
            .mem_save(Parameters(serde_json::from_value(value).unwrap()))
            .unwrap()
            .0
            .observation
    };

    let asked_in_a = ask("a", "por que la busqueda ensancha");
    let answered = save(json!({
        "session_id": "a", "title": "Porque una palabra tumba la pregunta entera",
        "content": "el cuerpo", "type": "discovery",
    }));
    assert_eq!(
        answered.prompt_sync_id.as_deref(),
        Some(asked_in_a.as_str()),
        "a memory saved answering a question says which"
    );

    let elsewhere = save(json!({
        "session_id": "b", "title": "Otra cosa", "content": "otro cuerpo", "type": "discovery",
    }));
    assert_eq!(
        elsewhere.prompt_sync_id, None,
        "a memory in another conversation must not borrow its question"
    );

    let unattributed = save(json!({
        "session_id": "a", "title": "Automatica", "content": "sin pregunta detras",
        "type": "discovery", "capture_prompt": false,
    }));
    assert_eq!(unattributed.prompt_sync_id, None);

    let asked_loose = ask("a", "una pregunta reciente del proyecto");
    let bucketed = save(json!({
        "title": "Sin sesion", "content": "guardada en el cubo del proyecto", "type": "discovery",
    }));
    assert_eq!(
        bucketed.prompt_sync_id.as_deref(),
        Some(asked_loose.as_str()),
        "a session-less save may answer the project's last question"
    );

    {
        let store = server.store.lock().unwrap();
        store
            .connection()
            .execute(
                "UPDATE prompts SET created_at = datetime('now', '-2 days')",
                [],
            )
            .unwrap();
    }
    let stale = save(json!({
        "title": "Sin sesion y tarde", "content": "otro cuerpo", "type": "discovery",
    }));
    assert_eq!(
        stale.prompt_sync_id, None,
        "a question asked two days ago is not what this memory answers"
    );
}

#[test]
fn a_recovery_token_is_good_for_one_choice_from_one_directory() {
    let detection = crate::project::ProjectDetection {
        project: String::new(),
        source: "ambiguous".to_owned(),
        path: "C:/repo".to_owned(),
        available_projects: vec!["alpha".to_owned(), "beta".to_owned()],
        warning: None,
        error_hint: None,
    };
    let mut tokens = RecoveryTokens::default();
    let token = tokens.issue(&detection);

    assert!(tokens.redeem(&token, "alpha", &detection));
    assert!(tokens.redeem(&token, "alpha", &detection));
    assert!(!tokens.redeem(&token, "beta", &detection));
    let fresh = tokens.issue(&detection);
    assert!(
        !tokens.redeem(&fresh, "gamma", &detection),
        "a token must not admit a project it was never issued over"
    );
    let moved = crate::project::ProjectDetection {
        path: "C:/otro".to_owned(),
        ..detection.clone()
    };
    let changed = crate::project::ProjectDetection {
        available_projects: vec!["alpha".to_owned(), "gamma".to_owned()],
        ..detection.clone()
    };
    let again = tokens.issue(&detection);
    assert!(!tokens.redeem(&again, "alpha", &moved));
    assert!(!tokens.redeem(&again, "alpha", &changed));
    assert!(!tokens.redeem("rec-nada", "alpha", &detection));
}

#[test]
fn no_description_names_part_of_a_vocabulary() {
    let headings: Vec<&str> = crate::memory::normalize::LEARNING_HEADINGS
        .iter()
        .filter(|(code, _)| *code != "en")
        .flat_map(|(_, headings)| headings.iter().copied())
        .collect();
    assert!(headings.len() > 10, "{headings:?}");
    let scopes: Vec<&str> = crate::memory::normalize::SCOPES.to_vec();
    assert_eq!(scopes.len(), 3, "{scopes:?}");
    let verbs: Vec<&str> = crate::memory::rules::RELATION_VERBS.to_vec();
    assert_eq!(verbs.len(), 6, "{verbs:?}");

    let mut sentences: Vec<(String, String)> = Vec::new();
    for tool in LeteoMcpServer::router().list_all() {
        if let Some(description) = tool.description.as_ref() {
            sentences.push((tool.name.to_string(), description.to_string()));
        }
        let Some(properties) = tool.input_schema.get("properties") else {
            continue;
        };
        let Some(properties) = properties.as_object() else {
            continue;
        };
        for (field, schema) in properties {
            if let Some(description) = schema.get("description").and_then(|d| d.as_str()) {
                sentences.push((format!("{}.{field}", tool.name), description.to_owned()));
            }
        }
    }
    assert!(sentences.len() > 60, "only found {}", sentences.len());

    for (vocabulary, whole, about, from) in [
        (headings, "twelve", None, 1),
        (scopes, "three", Some("scope"), 1),
        (verbs, "six", None, 2),
    ] {
        for (where_it_is, sentence) in &sentences {
            if about.is_some_and(|word| !sentence.contains(word)) {
                continue;
            }
            let named: Vec<&&str> = vocabulary
                .iter()
                .filter(|word| sentence.contains(**word))
                .collect();
            assert!(
                named.len() < from || named.len() == vocabulary.len(),
                "{where_it_is} names {named:?} and not the rest of the {whole}"
            );
        }
    }
}

#[test]
fn no_field_the_reply_may_omit_is_declared_required() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/mcp/output.rs"),
    )
    .expect("read output.rs");
    let mut offenders = Vec::new();
    let mut checked = 0;
    for (number, line) in source.lines().enumerate() {
        if !line.contains("skip_serializing_if") {
            continue;
        }
        checked += 1;
        if !line.contains("default") {
            offenders.push(format!("output.rs:{}: {}", number + 1, line.trim()));
        }
    }
    assert!(
        checked > 30,
        "the scan found only {checked}, so it has stopped matching output.rs"
    );
    assert!(
        offenders.is_empty(),
        "these fields are left out of the reply and declared required by the schema that same \
         reply advertises, so a validating client rejects it. Add `default` beside the skip:
{}",
        offenders.join(
            "
"
        )
    );
}

#[test]
fn no_tool_that_replaces_stored_text_calls_itself_additive() {
    let (_temp, server) = test_server(McpOptions::default());
    server
        .lock_store()
        .unwrap()
        .create_session("chat", "leteo", "C:/workspace")
        .unwrap();

    let body = "the text that was here before anybody called anything";
    let write = |title: &str, content: &str, topic_key: Option<&str>| {
        server
            .mem_save(Parameters(
                serde_json::from_value(json!({
                    "session_id": "chat",
                    "title": title,
                    "content": content,
                    "type": "decision",
                    "topic_key": topic_key,
                }))
                .unwrap(),
            ))
            .unwrap()
            .0
    };
    let survives = |server: &LeteoMcpServer| {
        server
            .lock_store()
            .unwrap()
            .search(
                "\"anybody called anything\"",
                crate::SearchOptions::default(),
            )
            .map(|found| !found.is_empty())
            .unwrap_or(false)
    };

    let destructive: BTreeSet<&str> = LeteoMcpServer::router()
        .list_all()
        .iter()
        .filter(|tool| tool.annotations.as_ref().and_then(|a| a.destructive_hint) == Some(true))
        .map(|tool| tool.name.to_string().leak() as &str)
        .collect();

    let saved = write("Replaced by an update", body, None);
    assert!(survives(&server), "the body is there to begin with");
    server
        .mem_update(Parameters(
            serde_json::from_value(json!({
                "observation_id": saved.observation.id,
                "expected_project": "leteo",
                "content": "something else entirely",
            }))
            .unwrap(),
        ))
        .unwrap();
    assert!(
        !survives(&server),
        "if this still finds it, the guard below has stopped meaning anything"
    );
    assert!(
        destructive.contains("mem_update"),
        "mem_update replaced a stored body and declares only additive updates"
    );

    write("First under the key", body, Some("audit/replacement"));
    assert!(survives(&server), "the body is there to begin with");
    let revised = write(
        "Second under the key",
        "a different body, which is the whole point of a revision",
        Some("audit/replacement"),
    );
    assert_eq!(revised.status, "revised", "the key is what makes it revise");
    assert!(
        !survives(&server),
        "if this still finds it, the guard below has stopped meaning anything"
    );
    assert!(
        destructive.contains("mem_save"),
        "mem_save replaced a stored body under a topic key and declares only \
         additive updates"
    );
}

#[test]
fn zero_leaves_a_section_out_and_the_schema_says_which_zero_it_is() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store.create_session("s1", "leteo", "C:/workspace").unwrap();
        for index in 0..30 {
            store
                .add_observation(AddObservation {
                    session_id: "s1".to_owned(),
                    kind: "decision".to_owned(),
                    title: format!("Memory {index}"),
                    content: "a body".to_owned(),
                    tool_name: None,
                    project: Some("leteo".to_owned()),
                    scope: "project".to_owned(),
                    topic_key: None,
                    prompt_sync_id: None,
                })
                .unwrap();
        }
        store
            .add_prompt(crate::AddPrompt {
                session_id: "s1".to_owned(),
                content: "una pregunta".to_owned(),
                project: Some("leteo".to_owned()),
            })
            .unwrap();
    }
    let context = |value: serde_json::Value| {
        server
            .mem_context(Parameters(serde_json::from_value(value).unwrap()))
            .unwrap()
            .0
    };

    let whole = context(json!({ "project": "leteo" }));
    assert!(!whole.observations.is_empty(), "{whole:?}");
    assert!(!whole.sessions.is_empty(), "{whole:?}");
    assert!(!whole.prompts.is_empty(), "{whole:?}");

    let trimmed = context(json!({
        "project": "leteo",
        "limit": 0,
        "session_limit": 0,
        "prompt_limit": 0,
    }));
    assert!(trimmed.observations.is_empty(), "{trimmed:?}");
    assert!(trimmed.sessions.is_empty(), "{trimmed:?}");
    assert!(trimmed.prompts.is_empty(), "{trimmed:?}");

    let no_prompts = context(json!({ "project": "leteo", "prompt_limit": 0 }));
    assert!(no_prompts.prompts.is_empty(), "{no_prompts:?}");
    assert!(!no_prompts.sessions.is_empty(), "and only that one");
    assert!(!no_prompts.observations.is_empty(), "and only that one");

    let focus = server
        .lock_store()
        .unwrap()
        .recent_memories(Some("leteo"), None, 30)
        .unwrap()[15]
        .id;
    let timeline = server
        .mem_timeline(Parameters(
            serde_json::from_value(json!({
                "observation_id": focus,
                "before": 0,
                "after": 0,
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    assert!(
        timeline.before.is_empty() && timeline.after.is_empty(),
        "{timeline:?}"
    );
    assert!(timeline.before_total > 0, "{timeline:?}");

    let ceiling = server.lock_store().unwrap().max_context_results();
    let wide = server
        .mem_timeline(Parameters(
            serde_json::from_value(json!({
                "observation_id": focus,
                "before": 1_000_000,
                "after": 1_000_000,
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    assert!(
        wide.before.len() <= ceiling && wide.after.len() <= ceiling,
        "a window past the ceiling is the ceiling: {} and {}",
        wide.before.len(),
        wide.after.len()
    );
    let published = LeteoMcpServer::router()
        .list_all()
        .into_iter()
        .find(|tool| tool.name == "mem_timeline")
        .expect("mem_timeline is exposed")
        .input_schema["properties"]["before"]["maximum"]
        .as_u64();
    assert_eq!(
        published,
        Some(ceiling as u64),
        "the ceiling it applies is the ceiling it publishes"
    );

    let floors: Vec<(String, i64)> = LeteoMcpServer::router()
        .list_all()
        .iter()
        .flat_map(|tool| {
            let name = tool.name.to_string();
            tool.input_schema
                .get("properties")
                .and_then(|properties| properties.as_object())
                .map(|properties| {
                    properties
                        .iter()
                        .filter_map(|(field, schema)| {
                            schema
                                .get("minimum")
                                .and_then(|minimum| minimum.as_i64())
                                .map(|minimum| (format!("{name}.{field}"), minimum))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .collect();
    assert!(floors.len() >= 5, "{floors:?}");
    for (field, minimum) in &floors {
        let expected = match field.as_str() {
            "mem_search.limit" | "mem_review.limit" => 1,
            // The floor a `mem_context` answer cannot shrink past, published so
            // a caller can see why a smaller `byte_limit` would not be met.
            "mem_context.byte_limit" => CONTEXT_ENVELOPE_FLOOR as i64,
            _ => 0,
        };
        assert_eq!(
            *minimum, expected,
            "{field} publishes a floor it does not apply"
        );
    }
}

#[test]
fn a_parameter_this_surface_does_not_have_is_refused() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store.create_session("s1", "leteo", "C:/workspace").unwrap();
    }

    server
        .mem_search(Parameters(
            serde_json::from_value(json!({ "query": "anything", "type": "decision" })).unwrap(),
        ))
        .expect("the spelling this surface does have");

    for (tool, arguments) in [
        (
            "mem_search",
            json!({ "query": "anything", "typ": "decision" }),
        ),
        (
            "mem_search",
            json!({ "query": "anything", "proyect": "leteo" }),
        ),
        ("mem_search", json!({ "query": "anything", "limite": 2 })),
        (
            "mem_compare",
            json!({ "memory_id_a": 1, "memory_id_b": 2, "relation": "related", "reason": "x" }),
        ),
        (
            "mem_judge",
            json!({ "judgment_id": "rel-1", "verdict": "related" }),
        ),
        ("mem_context", json!({ "projects": "leteo" })),
    ] {
        let error = serde_json::from_value::<serde_json::Value>(arguments.clone())
            .ok()
            .and_then(|value| parameters_error(tool, value))
            .unwrap_or_else(|| panic!("{tool} accepted a field it does not have: {arguments}"));
        assert!(
            error.contains("unknown field") && error.contains("expected one of"),
            "{tool} refused without saying what it does have: {error}"
        );
    }
}

fn parameters_error(tool: &str, arguments: serde_json::Value) -> Option<String> {
    let result = match tool {
        "mem_search" => serde_json::from_value::<SearchParams>(arguments).err(),
        "mem_compare" => serde_json::from_value::<CompareParams>(arguments).err(),
        "mem_judge" => serde_json::from_value::<JudgeParams>(arguments).err(),
        "mem_context" => serde_json::from_value::<ContextParams>(arguments).err(),
        other => panic!("nobody has taught this test about {other}"),
    };
    result.map(|error| error.to_string())
}

#[test]
fn every_list_mem_context_returns_stops_at_the_ceiling_it_publishes() {
    let temp = tempfile::tempdir().unwrap();
    let mut store =
        Store::open(crate::store::StoreConfig::new(temp.path().join("mcp.db"))).unwrap();
    for session in 0..25 {
        let id = format!("s{session}");
        store.create_session(&id, "leteo", "C:/repo").unwrap();
        store
            .add_prompt(crate::memory::model::AddPrompt {
                session_id: id.clone(),
                content: format!("una pregunta distinta de la sesion {session}"),
                project: Some("leteo".to_owned()),
            })
            .unwrap();
        for index in 0..5 {
            store
                .add_observation(crate::memory::model::AddObservation {
                    session_id: id.clone(),
                    kind: "discovery".to_owned(),
                    title: format!("Memoria {session}-{index} del techo"),
                    content: format!("con un cuerpo propio {session}-{index}"),
                    tool_name: None,
                    project: Some("leteo".to_owned()),
                    scope: "project".to_owned(),
                    topic_key: None,
                    prompt_sync_id: None,
                })
                .unwrap();
        }
    }
    let server = LeteoMcpServer::with_options(Arc::new(Mutex::new(store)), McpOptions::default());

    let wide = server
        .mem_context(Parameters(
            serde_json::from_value(json!({
                "limit": 9999, "session_limit": 9999, "prompt_limit": 9999,
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    let memorias = crate::settings::ContextSize::Deep.memories();
    let listas = server.lock_store().unwrap().max_context_results();
    assert_eq!(wide.count, memorias, "{wide:?}");
    assert_eq!(
        wide.observations.len() + wide.also_remembered.len(),
        memorias,
        "the two memory lists together are the budget"
    );
    assert_eq!(wide.sessions.len(), listas, "{}", wide.sessions.len());
    assert_eq!(wide.prompts.len(), listas, "{}", wide.prompts.len());

    assert!(25 > listas && 125 > memorias);

    let schema = LeteoMcpServer::router()
        .list_all()
        .into_iter()
        .find(|tool| tool.name == "mem_context")
        .expect("mem_context is exposed")
        .input_schema
        .clone();
    for (field, ceiling) in [
        ("limit", memorias),
        ("session_limit", listas),
        ("prompt_limit", listas),
    ] {
        assert_eq!(
            schema["properties"][field]["maximum"].as_u64(),
            Some(ceiling as u64),
            "{field} applies {ceiling} and publishes {:?}",
            schema["properties"][field]["maximum"]
        );
    }

    let ordinaria = server
        .mem_context(Parameters(
            serde_json::from_value(json!({ "limit": 12, "session_limit": 3, "prompt_limit": 4 }))
                .unwrap(),
        ))
        .unwrap()
        .0;
    assert_eq!(ordinaria.count, 12, "{ordinaria:?}");
    assert_eq!(ordinaria.sessions.len(), 3);
    assert_eq!(ordinaria.prompts.len(), 4);
}

#[test]
fn every_budget_publishes_the_ceiling_it_applies() {
    let mut examined = Vec::new();
    let mut naked = Vec::new();
    for tool in LeteoMcpServer::router().list_all() {
        let Some(properties) = tool
            .input_schema
            .get("properties")
            .and_then(|p| p.as_object())
        else {
            continue;
        };
        for (name, schema) in properties {
            let bounds_a_list =
                matches!(name.as_str(), "limit" | "before" | "after") || name.ends_with("_limit");
            let integer = schema.get("type").is_some_and(|kind| match kind {
                serde_json::Value::String(one) => one == "integer",
                serde_json::Value::Array(many) => many.iter().any(|k| k == "integer"),
                _ => false,
            });
            if !bounds_a_list || !integer {
                continue;
            }
            examined.push(format!("{}.{name}", tool.name));
            if schema.get("maximum").is_none() {
                naked.push(format!("{}.{name}", tool.name));
            }
        }
    }
    assert!(naked.is_empty(), "no ceiling published: {naked:?}");
    assert_eq!(
        examined.len(),
        8,
        "budgets changed; check they all still publish a ceiling: {examined:?}"
    );
}

#[test]
fn the_reread_queue_stops_at_the_ceiling_it_publishes() {
    let temp = tempfile::tempdir().unwrap();
    let mut store =
        Store::open(crate::store::StoreConfig::new(temp.path().join("mcp.db"))).unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    for index in 0..40 {
        let saved = store
            .add_observation(crate::memory::model::AddObservation {
                session_id: "s1".to_owned(),
                kind: "decision".to_owned(),
                title: format!("Una decision que toca releer {index}"),
                content: format!("con su cuerpo propio {index}"),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
        store
            .connection()
            .execute(
                "UPDATE observations SET review_after = datetime('now', '-30 days') WHERE id = ?1",
                [saved.observation.id],
            )
            .unwrap();
    }
    let ceiling = store.max_context_results();
    assert!(40 > ceiling, "the fixture has to overrun it");
    let server = LeteoMcpServer::with_options(Arc::new(Mutex::new(store)), McpOptions::default());

    let asked_for_everything = server
        .mem_review(Parameters(
            serde_json::from_value(json!({ "action": "list", "limit": 9999 })).unwrap(),
        ))
        .unwrap()
        .0;
    assert_eq!(
        asked_for_everything.count, ceiling,
        "{asked_for_everything:?}"
    );
    assert_eq!(asked_for_everything.observations.len(), ceiling);

    let published = LeteoMcpServer::router()
        .list_all()
        .into_iter()
        .find(|tool| tool.name == "mem_review")
        .expect("mem_review is exposed")
        .input_schema["properties"]["limit"]["maximum"]
        .as_u64();
    assert_eq!(
        published,
        Some(ceiling as u64),
        "applied {ceiling}, published {published:?}"
    );

    let ordinary = server
        .mem_review(Parameters(
            serde_json::from_value(json!({ "action": "list", "limit": 7 })).unwrap(),
        ))
        .unwrap()
        .0;
    assert_eq!(ordinary.count, 7, "{ordinary:?}");
}

#[test]
fn the_description_publishes_the_attribution_window_the_code_uses() {
    let published = format!("{} minutes", crate::store::PROMPT_ATTRIBUTION_MINUTES);
    let schema = LeteoMcpServer::router()
        .list_all()
        .into_iter()
        .find(|tool| tool.name == "mem_save")
        .expect("mem_save is exposed")
        .input_schema
        .clone();
    let said = schema["properties"]["capture_prompt"]["description"]
        .as_str()
        .expect("the field is described")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(said.contains(&published), "{said:?}");
    assert!(
        said.contains("no session_id") && said.contains("project"),
        "the fallback the number belongs to is named: {said:?}"
    );
}

#[test]
fn an_empty_search_says_its_count_is_a_floor_when_its_limit_is_what_stopped_it() {
    let temp = tempfile::tempdir().unwrap();
    let mut store =
        Store::open(crate::store::StoreConfig::new(temp.path().join("mcp.db"))).unwrap();
    store.create_session("s1", "otro", "C:/otro").unwrap();
    for index in 0..8 {
        store
            .add_observation(crate::memory::model::AddObservation {
                session_id: "s1".to_owned(),
                kind: "discovery".to_owned(),
                title: format!("Una nota sobre zarandajas numero {index}"),
                content: format!("Habla de zarandajas y de nada mas, la {index}."),
                tool_name: None,
                project: Some("otro".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
    }
    store.create_session("s2", "leteo", "C:/repo").unwrap();
    let server = LeteoMcpServer::with_options(
        Arc::new(Mutex::new(store)),
        McpOptions {
            default_project: Some("leteo".to_owned()),
            ..McpOptions::default()
        },
    );
    let buscar = |limite: usize| {
        server
            .mem_search(Parameters(
                serde_json::from_value(json!({ "query": "zarandajas", "limit": limite })).unwrap(),
            ))
            .unwrap()
            .0
    };

    let corto = buscar(2);
    assert_eq!(corto.count, 0, "{corto:?}");
    let dicho = corto.hint.clone().unwrap_or_default();
    assert!(dicho.contains("2 or more elsewhere"), "{dicho}");

    let largo = buscar(20);
    let dicho = largo.hint.clone().unwrap_or_default();
    assert!(
        dicho.contains("8 elsewhere") && !dicho.contains("or more"),
        "ocho son ocho cuando caben todos: {dicho}"
    );
}

#[test]
fn every_tool_refuses_a_field_it_does_not_take() {
    let mut examined = 0;
    let mut lenient = Vec::new();
    for tool in LeteoMcpServer::router().list_all() {
        examined += 1;
        let strict = tool
            .input_schema
            .get("additionalProperties")
            .and_then(serde_json::Value::as_bool)
            == Some(false);
        if !strict {
            lenient.push(tool.name.to_string());
        }
    }
    assert!(
        lenient.is_empty(),
        "these publish a schema that welcomes fields they ignore: {lenient:?}"
    );
    assert_eq!(examined, 23, "the whole surface was examined");
}

#[test]
fn a_tool_that_takes_nothing_still_answers_when_given_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(crate::store::StoreConfig::new(temp.path().join("mcp.db"))).unwrap();
    let server = LeteoMcpServer::with_options(Arc::new(Mutex::new(store)), McpOptions::default());

    let empty: NoParams = serde_json::from_value(json!({})).expect("an empty object is the call");
    assert!(server.mem_stats(Parameters(empty)).is_ok());

    let refused = serde_json::from_value::<NoParams>(json!({ "project": "leteo" }));
    let said = refused
        .expect_err("a field it does not take is refused")
        .to_string();
    assert!(said.contains("unknown field `project`"), "{said}");
}

#[test]
fn an_ambiguous_directory_says_so_whichever_door_was_knocked_on() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(crate::store::StoreConfig::new(temp.path().join("mcp.db"))).unwrap();
    let server = LeteoMcpServer::with_options(Arc::new(Mutex::new(store)), McpOptions::default());
    let ambiguo = crate::project::ProjectDetection {
        project: String::new(),
        source: crate::project::SOURCE_AMBIGUOUS.to_owned(),
        path: "C:/padre".to_owned(),
        available_projects: vec!["hijo-uno".to_owned(), "hijo-dos".to_owned()],
        warning: None,
        error_hint: Some("ambiguous project: multiple git repositories found in cwd".to_owned()),
    };
    let cuerpo = |result: CallToolResult| -> serde_json::Value {
        result.structured_content.expect("the error is structured")
    };

    let sin_self = cuerpo(project_detection_error(&ambiguo));
    assert_eq!(sin_self["error"]["code"], error_code::AMBIGUOUS_PROJECT);
    assert_eq!(sin_self["available_projects"][0], "hijo-uno");
    assert!(
        sin_self["recovery_token"].is_null(),
        "no acuña token: nombrar un proyecto es para lo que sirve"
    );
    let dice = sin_self["recovery_instructions"]
        .as_str()
        .unwrap_or_default();
    assert!(
        dice.contains("project=<choice>") && dice.contains("no recovery_token"),
        "{dice}"
    );

    let con_self = cuerpo(server.project_detection_error(&ambiguo));
    assert_eq!(con_self["error"]["code"], error_code::AMBIGUOUS_PROJECT);
    assert!(
        con_self["recovery_token"]
            .as_str()
            .is_some_and(|t| !t.is_empty()),
        "una escritura tiene que probar que se preguntó: {con_self}"
    );

    let rota = crate::project::ProjectDetection {
        available_projects: Vec::new(),
        ..ambiguo.clone()
    };
    assert_eq!(
        cuerpo(project_detection_error(&rota))["error"]["code"],
        "project_detection_failed"
    );
}

#[test]
fn every_output_schema_accepts_the_error_shape_as_well() {
    let refusal = json!({
        "error": { "code": "observation_not_found", "message": "observation not found: 1" },
        "available_projects": ["uno", "dos"],
        "recovery_token": "rec-abc",
    });
    let mut examined = 0;
    let mut naked = Vec::new();
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(crate::store::StoreConfig::new(temp.path().join("mcp.db"))).unwrap();
    let server = LeteoMcpServer::with_options(Arc::new(Mutex::new(store)), McpOptions::default());
    for tool in server.router.list_all() {
        let Some(schema) = tool.output_schema.as_ref() else {
            continue;
        };
        let schema = serde_json::Value::Object((**schema).clone());
        let Some(branches) = schema.get("anyOf").and_then(serde_json::Value::as_array) else {
            continue;
        };
        examined += 1;
        let takes_an_error = branches.iter().any(|branch| {
            branch
                .get("required")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|fields| fields.iter().any(|field| field == "error"))
        });
        if !takes_an_error {
            naked.push(tool.name.to_string());
        }
        assert_eq!(schema["type"], "object", "{}", tool.name);
        assert!(schema.get("properties").is_some(), "{}", tool.name);
        let success = branches[0]["required"]
            .as_array()
            .map(Vec::len)
            .unwrap_or_default();
        assert!(success > 0, "{} demands nothing at all", tool.name);
        let _ = &refusal;
    }
    assert!(
        naked.is_empty(),
        "these describe only their answer: {naked:?}"
    );
    assert!(
        examined >= 15,
        "only {examined} schemas were examined, so this guard is watching almost nothing"
    );
}

#[test]
fn a_diagnosis_lists_examples_of_the_damage_and_counts_the_rest() {
    let temp = tempfile::tempdir().unwrap();
    let mut store =
        Store::open(crate::store::StoreConfig::new(temp.path().join("mcp.db"))).unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let orphans = VIOLATION_EXAMPLES * 2 + 5;
    for index in 0..orphans {
        store
            .add_observation(crate::memory::model::AddObservation {
                session_id: "s1".to_owned(),
                kind: "discovery".to_owned(),
                title: format!("Una memoria numero {index}"),
                content: "un cuerpo cualquiera".to_owned(),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
    }
    store
        .connection()
        .execute_batch("PRAGMA foreign_keys=OFF; DELETE FROM sessions WHERE id = 's1';")
        .unwrap();

    let report = store.doctor().unwrap();
    assert_eq!(
        report.foreign_key_violations.as_ref().map(Vec::len),
        Some(orphans),
        "the store's own report is the whole list"
    );

    let server = LeteoMcpServer::with_options(Arc::new(Mutex::new(store)), McpOptions::default());
    let Json(answered) = server
        .mem_doctor(Parameters(serde_json::from_value(json!({})).unwrap()))
        .expect("the diagnosis answers");
    assert_eq!(
        answered.foreign_key_violations.as_ref().map(Vec::len),
        Some(VIOLATION_EXAMPLES),
        "the agent sees examples"
    );
    assert_eq!(
        answered.foreign_key_violations_omitted,
        orphans - VIOLATION_EXAMPLES,
        "and how many it does not see"
    );
    assert!(
        answered
            .issues
            .iter()
            .any(|issue| issue.contains(&orphans.to_string())),
        "{:?}",
        answered.issues
    );
}

#[test]
fn the_refusal_that_cannot_be_retried_says_so() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(crate::store::StoreConfig::new(temp.path().join("mcp.db"))).unwrap();
    let shared = Arc::new(Mutex::new(store));
    let envenenar = Arc::clone(&shared);
    let _ = std::thread::spawn(move || {
        let _guard = envenenar.lock().unwrap();
        panic!("algo se rompió con el candado en la mano");
    })
    .join();
    assert!(shared.is_poisoned(), "el candado quedó envenenado");

    let server = LeteoMcpServer::with_options(shared, McpOptions::default());
    let Err(refusal) = server.mem_stats(Parameters(NoParams {})) else {
        panic!("no se puede contestar sin store");
    };
    let cuerpo = refusal
        .structured_content
        .expect("el error es estructurado");
    assert_eq!(cuerpo["error"]["code"], error_code::STORE_UNAVAILABLE);
    let dicho = cuerpo["error"]["message"].as_str().unwrap_or_default();
    assert!(dicho.contains("restarted"), "dice qué hace falta: {dicho}");
    assert!(
        dicho.contains("Retrying will not help"),
        "y que insistir no sirve, que es lo que un agente hace por defecto: {dicho}"
    );
    assert!(
        !dicho.contains("poisoned"),
        "sin la palabra que solo significa algo dentro de Rust: {dicho}"
    );
}

#[test]
fn every_ceiling_a_tool_publishes_is_one_something_applies() {
    let server = LeteoMcpServer::with_options(
        Arc::new(Mutex::new(
            Store::open(crate::store::StoreConfig::new(
                tempfile::tempdir().unwrap().path().join("mcp.db"),
            ))
            .unwrap(),
        )),
        McpOptions::default(),
    );
    let list = crate::store::StoreConfig::new("unused").max_context_results;
    let context = crate::settings::ContextSize::Deep.memories();
    // The byte ceiling a `byte_limit` publishes, which is the third number this
    // surface bounds an answer by: a list of rows, the depth of a context, and
    // now the bytes of one.
    let bytes = crate::settings::ContextSize::Deep.bytes();

    let mut published = Vec::new();
    for tool in server.router.list_all() {
        let schema = serde_json::Value::Object((*tool.input_schema).clone());
        let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) else {
            continue;
        };
        for (field, shape) in properties {
            if let Some(maximum) = shape.get("maximum").and_then(serde_json::Value::as_u64) {
                published.push((tool.name.to_string(), field.clone(), maximum as usize));
            }
        }
    }

    assert!(
        published.len() >= 7,
        "only {} published ceilings found, so this has stopped reading the schemas: {published:?}",
        published.len()
    );
    let strays: Vec<&(String, String, usize)> = published
        .iter()
        .filter(|(_, _, maximum)| *maximum != list && *maximum != context && *maximum != bytes)
        .collect();
    assert!(
        strays.is_empty(),
        "these publish a ceiling none of {list} (a list of rows), {context} (the depth of a \
         context) or {bytes} (the bytes of one): {strays:?}"
    );
    assert_eq!(
        crate::mcp::output::VIOLATION_EXAMPLES,
        list,
        "the diagnosis lists as many rows as every other list on this surface, which is what its \
         own comment says it does"
    );
}

#[test]
fn the_capture_tool_says_how_many_learnings_did_not_fit() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(crate::store::StoreConfig::new(
        temp.path().join("capture.db"),
    ))
    .unwrap();
    let server = LeteoMcpServer::with_options(Arc::new(Mutex::new(store)), McpOptions::default());
    let ceiling = crate::memory::normalize::MAX_LEARNINGS;

    let over = ceiling + 3;
    let mut text = String::from("## Key Learnings\n\n");
    for index in 0..over {
        text.push_str(&format!(
            "{}. The pool number {index} caps at sixteen and waits for it\n",
            index + 1
        ));
    }

    let Json(answer) = server
        .mem_capture_passive(Parameters(crate::mcp::params::CapturePassiveParams {
            content: text,
            session_id: None,
            source: "a-subagent".to_owned(),
        }))
        .expect("the capture answers");

    assert_eq!(answer.extracted, over);
    assert_eq!(answer.saved, ceiling);
    assert_eq!(
        answer.dropped, 3,
        "the numbers add up, or memories go missing between them"
    );
    let hint = answer.hint.unwrap_or_default();
    assert!(
        hint.contains("3 were not stored") && hint.contains("mem_save"),
        "and the caller, which still has the text, is told what to do: {hint}"
    );

    let mut text = String::from(
        "## Key Learnings

",
    );
    for index in 0..ceiling + 1 {
        text.push_str(&format!(
            "{}. The ladder number {index} retries past the deadline it was given
",
            index + 1
        ));
    }
    let Json(answer) = server
        .mem_capture_passive(Parameters(crate::mcp::params::CapturePassiveParams {
            content: text,
            session_id: None,
            source: "a-subagent".to_owned(),
        }))
        .expect("the capture answers");
    assert_eq!(answer.dropped, 1);
    let hint = answer.hint.unwrap_or_default();
    assert!(
        hint.contains("one was not stored") && !hint.contains("1 were"),
        "one of them is one, not 1 were: {hint}"
    );

    let Json(answer) = server
        .mem_capture_passive(Parameters(crate::mcp::params::CapturePassiveParams {
            content: "I read the file and it looked fine to me.".to_owned(),
            session_id: None,
            source: "a-subagent".to_owned(),
        }))
        .expect("the capture answers");
    assert_eq!(answer.dropped, 0);
    assert_eq!(answer.extracted, 0);
    assert_eq!(
        answer.hint.as_deref(),
        Some(crate::mcp::output::NOTHING_EXTRACTED_HINT)
    );
}

#[test]
fn every_number_a_capture_produces_reaches_both_doors() {
    let result = crate::memory::model::PassiveCaptureResult {
        extracted: 9,
        saved: 5,
        duplicates: 3,
        dropped: 1,
    };
    let produced = serde_json::to_value(&result)
        .expect("the result serialises")
        .as_object()
        .expect("as an object")
        .len();

    let tool = serde_json::to_value(CapturePassiveOutput::new(
        result.clone(),
        ProjectEnvelope::default(),
    ))
    .expect("the tool's answer serialises");
    let counted = tool
        .as_object()
        .expect("as an object")
        .values()
        .filter(|value| value.is_u64())
        .count();
    assert_eq!(
        counted, produced,
        "the tool renders {counted} of the {produced} numbers a capture produces: {tool}"
    );

    let mut outcome = crate::hooks::HookOutcome {
        observations_captured: result.saved,
        observations_extracted: Some(result.extracted),
        observations_duplicate: Some(result.duplicates),
        observations_dropped: Some(result.dropped),
        ..crate::hooks::HookOutcome::default()
    };
    outcome.event = "SubagentStop";
    let hook = serde_json::to_value(&outcome).expect("the outcome serialises");
    let counted = hook
        .as_object()
        .expect("as an object")
        .values()
        .filter(|value| value.is_u64())
        .count();
    assert_eq!(
        counted, produced,
        "the hook reports {counted} of the {produced} numbers a capture produces: {hook}"
    );
}

#[test]
fn the_context_tool_bounds_its_pinned_half_by_what_was_asked_for() {
    let temp = tempfile::tempdir().unwrap();
    let mut store =
        Store::open(crate::store::StoreConfig::new(temp.path().join("pins.db"))).unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let deepest = crate::settings::ContextSize::Deep.memories();
    for index in 0..deepest + 20 {
        let saved = store
            .add_observation(crate::memory::model::AddObservation {
                session_id: "s1".to_owned(),
                kind: "discovery".to_owned(),
                title: format!("Pinned {index}"),
                content: "a body worth keeping".to_owned(),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap()
            .observation;
        store.pin_observation(saved.id, None).unwrap();
    }
    let server = LeteoMcpServer::with_options(Arc::new(Mutex::new(store)), McpOptions::default());

    for asked in [5_usize, 20, deepest] {
        let Json(answer) = server
            .mem_context(Parameters(crate::mcp::params::ContextParams {
                project: Some("leteo".to_owned()),
                all_projects: false,
                scope: None,
                limit: Some(asked),
                session_limit: 5,
                prompt_limit: 5,
                byte_limit: None,
            }))
            .expect("the context answers");
        let pinned = answer
            .observations
            .iter()
            .filter(|observation| observation.pinned)
            .count();
        assert_eq!(
            pinned, asked,
            "asked for {asked} and the pinned half handed over {pinned}"
        );
        assert_eq!(
            answer.pinned_omitted,
            deepest + 20 - asked,
            "and what did not fit is counted"
        );
    }
}

/// The context tool never answers with more bytes than the size allows.
///
/// The count ceiling bounds how many memories, not how large the answer is, and
/// a store of long rows outgrows the byte budget with every list still inside
/// its count. The fixture is sized to exceed the default `Full` budget on
/// purpose: a store that fits would pass this without watching anything.
#[test]
fn a_context_answer_is_never_larger_than_the_size_byte_budget() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(crate::store::StoreConfig::new(
        temp.path().join("budget.db"),
    ))
    .unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    for index in 0..80 {
        store
            .add_observation(crate::memory::model::AddObservation {
                session_id: "s1".to_owned(),
                kind: "discovery".to_owned(),
                title: format!("Memoria {index} {}", "漢".repeat(140)),
                content: "漢".repeat(400),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
    }
    let server = LeteoMcpServer::with_options(Arc::new(Mutex::new(store)), McpOptions::default());

    let Json(answer) = server
        .mem_context(Parameters(crate::mcp::params::ContextParams {
            project: Some("leteo".to_owned()),
            all_projects: false,
            scope: None,
            limit: Some(80),
            session_limit: 5,
            prompt_limit: 10,
            byte_limit: None,
        }))
        .expect("the context answers");
    let budget = crate::settings::ContextSize::Full.bytes();
    let serialized = serde_json::to_string(&answer).expect("the answer serializes");
    assert!(
        serialized.len() <= budget,
        "the answer is {} bytes against the {budget}-byte budget",
        serialized.len()
    );
    assert!(
        answer.memories_omitted > 0,
        "the oversized fixture has to make the budget drop something"
    );
    assert_eq!(
        answer.count,
        answer.observations.len() + answer.also_remembered.len() + answer.memories_omitted,
        "what is carried plus what was omitted has to be what was found"
    );
}

#[test]
fn a_scope_leteo_does_not_know_is_refiled_and_the_reply_says_so() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(crate::store::StoreConfig::new(temp.path().join("scope.db"))).unwrap();
    let server = LeteoMcpServer::with_options(Arc::new(Mutex::new(store)), McpOptions::default());

    let save = |server: &LeteoMcpServer, title: &str, kind: &str, scope: &str| {
        let Json(saved) = server
            .mem_save(Parameters(crate::mcp::params::SaveParams {
                title: title.to_owned(),
                content: Some(format!(
                    "A body long enough to be worth keeping, for {title}."
                )),
                kind: kind.to_owned(),
                scope: scope.to_owned(),
                capture_prompt: false,
                session_id: None,
                observation: None,
                tool_name: None,
                project: None,
                project_choice_reason: None,
                recovery_token: None,
                topic_key: None,
            }))
            .expect("the save answers");
        saved
    };

    let both_known = save(&server, "Both known", "decision", "personal");
    assert_eq!(both_known.observation.scope, "personal");
    assert!(both_known.hint.is_none(), "{:?}", both_known.hint);

    // An unknown type folds onto a documented one and is filed quietly; the
    // scope is the value that is replaced, so it is the one that is said.
    let odd_kind = save(&server, "Odd kind", "implementation", "personal");
    assert_eq!(odd_kind.observation.kind, "discovery");
    assert_eq!(odd_kind.observation.scope, "personal");
    assert!(odd_kind.hint.is_none(), "{:?}", odd_kind.hint);

    let odd_scope = save(&server, "Odd scope", "decision", "personnal");
    assert_eq!(
        odd_scope.observation.scope, "project",
        "the value asked for is gone, which is why this has to be said"
    );
    let hint = odd_scope.hint.unwrap_or_default();
    assert!(
        hint.contains("personnal") && hint.contains("filed as project"),
        "the reply names what was asked for and what happened instead: {hint}"
    );
    assert!(
        crate::memory::normalize::SCOPES
            .iter()
            .all(|scope| hint.contains(scope)),
        "and the three it accepts, read from the one list: {hint}"
    );

    let both_odd = save(&server, "Both odd", "implementation", "personnal");
    assert_eq!(both_odd.observation.kind, "discovery");
    let hint = both_odd.hint.unwrap_or_default();
    assert!(
        hint.contains("personnal") && !hint.contains("type"),
        "the scope is still named, and the type is not a mistake any more: {hint}"
    );
}

#[test]
fn the_review_queue_says_how_much_of_itself_this_page_is_not() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(crate::store::StoreConfig::new(
        temp.path().join("review.db"),
    ))
    .unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let ceiling = store.max_context_results();
    let due = ceiling + 7;
    for index in 0..due {
        let saved = store
            .add_observation(crate::memory::model::AddObservation {
                session_id: "s1".to_owned(),
                kind: "decision".to_owned(),
                title: format!("A decision worth rereading {index}"),
                content: "a body worth keeping".to_owned(),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap()
            .observation;
        store
            .connection()
            .execute(
                "UPDATE observations SET review_after = datetime('now', '-3 days') WHERE id = ?1",
                rusqlite::params![saved.id],
            )
            .unwrap();
    }
    store.create_session("s2", "otro", "C:/otro").unwrap();
    for index in 0..5 {
        let saved = store
            .add_observation(crate::memory::model::AddObservation {
                session_id: "s2".to_owned(),
                kind: "decision".to_owned(),
                title: format!("Another project's decision {index}"),
                content: "a body worth keeping".to_owned(),
                tool_name: None,
                project: Some("otro".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap()
            .observation;
        store
            .connection()
            .execute(
                "UPDATE observations SET review_after = datetime('now', '-3 days') WHERE id = ?1",
                rusqlite::params![saved.id],
            )
            .unwrap();
    }
    let counted = store.count_review_due(Some("leteo")).unwrap() as usize;
    assert_eq!(counted, due, "the fixture really is over the ceiling");

    let server = LeteoMcpServer::with_options(Arc::new(Mutex::new(store)), McpOptions::default());
    for asked in [None, Some(1_usize), Some(ceiling)] {
        let Json(listed) = server
            .mem_review(Parameters(crate::mcp::params::ReviewParams {
                action: "list".to_owned(),
                project: Some("leteo".to_owned()),
                limit: asked.unwrap_or(10),
                observation_id: None,
                id: None,
            }))
            .expect("the queue answers");
        assert_eq!(
            listed.count + listed.due_omitted,
            counted,
            "what this page carries plus what it left is the queue the block named: {listed:?}"
        );
        assert_eq!(listed.count, listed.observations.len(), "{listed:?}");
    }

    let Json(listed) = server
        .mem_review(Parameters(crate::mcp::params::ReviewParams {
            action: "list".to_owned(),
            project: Some("leteo".to_owned()),
            limit: 10,
            observation_id: None,
            id: None,
        }))
        .expect("the queue answers");
    assert!(
        listed.due_omitted > 0,
        "the ceiling is below the queue here"
    );
}

/// One call replaces several memories and hides them from every listing.
#[test]
fn consolidating_through_the_tool_merges_and_hides_the_sources() {
    let (_temp, server) = test_server(McpOptions {
        default_project: Some("leteo".to_owned()),
        ..McpOptions::default()
    });
    let (first, second) = {
        let mut store = server.lock_store().unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        let save = |store: &mut Store, title: &str, body: &str| {
            store
                .add_observation(crate::memory::model::AddObservation {
                    session_id: "s1".to_owned(),
                    kind: "decision".to_owned(),
                    title: title.to_owned(),
                    content: body.to_owned(),
                    tool_name: None,
                    project: Some("leteo".to_owned()),
                    scope: "project".to_owned(),
                    topic_key: None,
                    prompt_sync_id: None,
                })
                .unwrap()
                .observation
        };
        (
            save(
                &mut store,
                "Old duplicate one",
                "the mergeword appears in the first",
            ),
            save(
                &mut store,
                "Old duplicate two",
                "the mergeword appears in the second",
            ),
        )
    };

    let Json(outcome) = server
        .mem_consolidate(Parameters(
            serde_json::from_value(json!({
                "expected_project": "leteo",
                "source_ids": [first.id, second.id],
                "type": "decision",
                "title": "The merged decision",
                "content": "the mergeword appears once now",
            }))
            .unwrap(),
        ))
        .expect("a merge the caller asked for");
    assert_eq!(outcome.relations.len(), 2);
    assert_eq!(outcome.source_ids, vec![first.id, second.id]);
    assert_eq!(outcome.observation.title, "The merged decision");
    for relation in &outcome.relations {
        assert_eq!(relation.relation, "supersedes");
        assert_eq!(relation.judgment_status, "judged");
        assert_eq!(relation.source_id, outcome.observation.sync_id);
    }

    let Json(found) = server
        .mem_search(Parameters(
            serde_json::from_value(json!({ "query": "mergeword", "all_projects": true })).unwrap(),
        ))
        .unwrap();
    assert!(
        found.results.iter().all(|result| {
            result.observation.id != first.id && result.observation.id != second.id
        }),
        "a superseded source is not a search result: {:?}",
        found.results
    );
    assert!(
        found
            .results
            .iter()
            .any(|result| result.observation.id == outcome.observation.id),
        "the replacement is: {:?}",
        found.results
    );

    let Json(context) = server
        .mem_context(Parameters(
            serde_json::from_value(json!({ "project": "leteo" })).unwrap(),
        ))
        .unwrap();
    assert!(
        context
            .observations
            .iter()
            .all(|observation| observation.id != first.id && observation.id != second.id),
        "the context leaves the sources out: {:?}",
        context.observations
    );

    let Json(read) = server
        .mem_get_observation(Parameters(GetObservationParams {
            id: first.id,
            include_history: false,
        }))
        .unwrap();
    assert_eq!(read.observation.caveats.len(), 1);
    assert_eq!(read.observation.caveats[0].relation, "superseded_by");
    assert_eq!(read.observation.caveats[0].other_id, outcome.observation.id);
}

#[test]
fn a_merge_with_no_sources_is_refused_by_name() {
    let (_temp, server) = test_server(McpOptions {
        default_project: Some("leteo".to_owned()),
        ..McpOptions::default()
    });
    let Err(error) = server.mem_consolidate(Parameters(
        serde_json::from_value(json!({
            "expected_project": "leteo",
            "source_ids": [],
            "title": "t",
            "content": "c",
        }))
        .unwrap(),
    )) else {
        panic!("a merge of nothing is refused");
    };
    assert_eq!(
        error_payload(&error)["error"]["code"],
        "consolidation_sources"
    );
}

/// A revision made through the tool surface is readable back, byte for byte,
/// and only when it was asked for.
#[test]
fn mem_get_observation_returns_the_versions_it_was_asked_for() {
    let (_temp, server) = test_server(McpOptions {
        default_project: Some("leteo".to_owned()),
        ..McpOptions::default()
    });
    {
        let mut store = server.lock_store().unwrap();
        store
            .create_session("history", "leteo", "C:/workspace")
            .unwrap();
    }

    let first_body = "el cuerpo original, con acentos: ñ";
    let saved = server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "history",
                "title": "Versión original",
                "content": first_body,
                "topic_key": "architecture/history",
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    let id = saved.observation.id;

    let revised = server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "history",
                "title": "Versión revisada",
                "content": "el cuerpo nuevo, con acentos: ñ",
                "topic_key": "architecture/history",
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    assert_eq!(
        revised.replaced_bytes,
        Some(first_body.len()),
        "the save reply reports the size it replaced"
    );
    assert!(
        revised.hint.is_none(),
        "a similar-size revision is no shrink"
    );

    let without = server
        .mem_get_observation(Parameters(
            serde_json::from_value(json!({ "id": id })).unwrap(),
        ))
        .unwrap()
        .0;
    assert!(
        without.versions.is_empty(),
        "the history is opt-in and absent by default"
    );

    let with = server
        .mem_get_observation(Parameters(
            serde_json::from_value(json!({ "id": id, "include_history": true })).unwrap(),
        ))
        .unwrap()
        .0;
    assert_eq!(with.versions.len(), 1);
    assert_eq!(with.versions[0].revision, 1);
    assert_eq!(with.versions[0].title, "Versión original");
    assert_eq!(
        with.versions[0].content.as_bytes(),
        first_body.as_bytes(),
        "the previous body is byte-identical"
    );
    assert!(!with.versions[0].replaced_at.is_empty());
}

/// `mem_update` reports the body it replaced and names the read that returns it.
#[test]
fn mem_update_reports_the_body_it_replaced_and_calls_out_a_shrink() {
    let (_temp, server) = test_server(McpOptions {
        default_project: Some("leteo".to_owned()),
        ..McpOptions::default()
    });
    {
        let mut store = server.lock_store().unwrap();
        store
            .create_session("shrink", "leteo", "C:/workspace")
            .unwrap();
    }

    let long = "a".repeat(400);
    let saved = server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "shrink",
                "title": "Grande",
                "content": long,
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    let updated = server
        .mem_update(Parameters(
            serde_json::from_value(json!({
                "id": saved.observation.id,
                "expected_project": "leteo",
                "content": "b".repeat(5),
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    assert_eq!(updated.replaced_bytes, Some(400));
    let hint = updated.hint.expect("a shrink is called out");
    assert!(hint.contains("400"), "{hint}");
    assert!(hint.contains("include_history"), "{hint}");
}

/// A find/replace that outgrows the storage bound reports the cut from the text
/// it edited, which the caller never held.
///
/// Reading the cut from `params.content` — which is absent for a find/replace —
/// would report nothing and let the caller believe a body the store cut in half
/// was stored whole.
#[test]
fn a_replace_that_outgrows_the_bound_reports_the_cut() {
    let (_temp, server) = test_server(McpOptions {
        default_project: Some("leteo".to_owned()),
        ..McpOptions::default()
    });
    {
        let mut store = server.lock_store().unwrap();
        store
            .create_session("bound", "leteo", "C:/workspace")
            .unwrap();
    }
    let bound = server.lock_store().unwrap().max_observation_length();
    let saved = server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "bound",
                "title": "Una memoria que crece",
                "content": "TOKEN",
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;

    let updated = server
        .mem_update(Parameters(
            serde_json::from_value(json!({
                "id": saved.observation.id,
                "expected_project": "leteo",
                "find": "TOKEN",
                "replace": "x".repeat(bound + 1),
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    assert_storage_cut(updated.storage_truncation, bound, "find/replace update");

    // And a replacement that fits reports nothing, the way a whole-body write
    // under the bound does. A second memory, because the first one's body is
    // now a wall of `x` and no span of it is unique.
    let small = server
        .mem_save(Parameters(
            serde_json::from_value(json!({
                "session_id": "bound",
                "title": "Una memoria que no crece",
                "content": "usa TOKEN aquí",
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    let short = server
        .mem_update(Parameters(
            serde_json::from_value(json!({
                "id": small.observation.id,
                "expected_project": "leteo",
                "find": "TOKEN",
                "replace": "otra cosa",
            }))
            .unwrap(),
        ))
        .unwrap()
        .0;
    assert!(
        short.storage_truncation.is_none(),
        "an edit under the bound is not a storage cut"
    );
}

/// A `byte_limit` below the size setting shrinks the answer and counts what it
/// dropped, so the caller can plan against a bound they asked for.
#[test]
fn mem_context_honours_a_byte_limit_below_the_size() {
    let temp = tempfile::tempdir().unwrap();
    let mut store =
        Store::open(crate::store::StoreConfig::new(temp.path().join("limit.db"))).unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    for index in 0..80 {
        store
            .add_observation(AddObservation {
                session_id: "s1".to_owned(),
                kind: "discovery".to_owned(),
                title: format!("Memoria {index}"),
                content: "cuerpo ".repeat(80),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
    }
    let server = LeteoMcpServer::with_options(Arc::new(Mutex::new(store)), McpOptions::default());

    let limit = 4_000_usize;
    let Json(answer) = server
        .mem_context(Parameters(ContextParams {
            project: Some("leteo".to_owned()),
            all_projects: false,
            scope: None,
            limit: Some(80),
            session_limit: 5,
            prompt_limit: 5,
            byte_limit: Some(limit),
        }))
        .unwrap();
    let serialized = serde_json::to_string(&answer).unwrap();
    assert!(
        serialized.len() <= limit,
        "the answer is {} bytes against the {limit} asked for",
        serialized.len()
    );
    assert!(
        answer.memories_omitted > 0,
        "the fixture has to make the bound drop something: {answer:?}"
    );
    assert!(
        !answer.byte_limit_unmet,
        "a bound above the envelope is met: {answer:?}"
    );
}

/// A `byte_limit` cannot raise the answer above the size setting: the setting is
/// the budget a person chose for this store, and one call is not the place to
/// overrule it.
#[test]
fn mem_context_does_not_grow_past_the_size_setting() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(crate::store::StoreConfig::new(
        temp.path().join("setting.db"),
    ))
    .unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    // Sized to exceed the default `Full` budget, so a ceiling that ignored the
    // setting would answer with more.
    for index in 0..80 {
        store
            .add_observation(AddObservation {
                session_id: "s1".to_owned(),
                kind: "discovery".to_owned(),
                title: format!("Memoria {index} {}", "漢".repeat(140)),
                content: "漢".repeat(400),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
    }
    let server = LeteoMcpServer::with_options(Arc::new(Mutex::new(store)), McpOptions::default());
    let context = |byte_limit: Option<usize>| {
        let Json(answer) = server
            .mem_context(Parameters(ContextParams {
                project: Some("leteo".to_owned()),
                all_projects: false,
                scope: None,
                limit: Some(80),
                session_limit: 5,
                prompt_limit: 5,
                byte_limit,
            }))
            .unwrap();
        answer
    };

    let asked = context(Some(crate::settings::ContextSize::Deep.bytes()));
    let natural = context(None);
    assert!(
        natural.memories_omitted > 0,
        "the fixture has to exceed the setting, or a ceiling above it would not show"
    );
    assert_eq!(
        serde_json::to_string(&asked).unwrap(),
        serde_json::to_string(&natural).unwrap(),
        "a byte_limit above the size setting changes nothing"
    );
}

/// A bound under the envelope every answer carries cannot be kept, and the
/// reply says so rather than reporting a bound it did not meet.
#[test]
fn mem_context_says_when_a_byte_limit_cannot_be_met() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        store
            .add_observation(AddObservation {
                session_id: "s1".to_owned(),
                kind: "discovery".to_owned(),
                title: "Una memoria".to_owned(),
                content: "un cuerpo".to_owned(),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
    }

    let Json(answer) = server
        .mem_context(Parameters(ContextParams {
            project: Some("leteo".to_owned()),
            all_projects: false,
            scope: None,
            limit: None,
            session_limit: 5,
            prompt_limit: 5,
            byte_limit: Some(100),
        }))
        .unwrap();
    assert!(
        answer.byte_limit_unmet,
        "a bound under the envelope is reported, not quietly missed: {answer:?}"
    );
    assert!(
        answer.memories_omitted > 0,
        "everything droppable was dropped trying: {answer:?}"
    );
}
