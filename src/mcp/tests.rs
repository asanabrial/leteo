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
    // Enough of them that a timeline has something on both sides of its focus.
    //
    // With one memory in the store the timeline came back with two empty lists,
    // so every field of a `TimelineEntryOutput` was declared and never
    // compared against anything: dropping `session_id` from the wire, or
    // sending an `id` as a string, left this test green. A guard is only worth
    // what its fixture reaches.
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
    // A prompt with no project at all, which is the shape that broke
    // `PromptOutput` the same way.
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

    // The replies whose envelope carries no path: a read scoped by request,
    // and a read across every project. Both were broken.
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
    // And every other reply this fixture can produce. Checking one tool of
    // twenty-two is what let the same fault sit in `mem_get_observation` from
    // the first commit: its `caveats` was skipped when empty and marked
    // required, so every reply about a memory nothing has been said against —
    // nearly all of them — failed the schema it declares.
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

/// Whether a reply is the shape its schema says, all the way down.
///
/// The top-level `required` list is the part that broke before and it is the
/// part that was checked; everything under it — the memories inside a search,
/// the checks inside a report, the observation inside a save — was declared and
/// never compared against. That is the same distance the spec calls out
/// between declaring a schema and validating against it, hiding one level in.
///
/// Small on purpose. It follows `$ref` into `$defs`, walks `properties` and
/// `items`, and enforces `required` and `type`; anything it does not know it
/// passes, so it can only ever accuse a reply of something a client would
/// accuse it of too.
fn check_against_schema(
    value: &serde_json::Value,
    schema: &serde_json::Value,
    root: &serde_json::Value,
    path: &str,
    faults: &mut Vec<String>,
) {
    // `$ref` is how `schemars` writes every nested type, so nothing below the
    // first level is reachable without following it.
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
        // An integer satisfies `number`, which is the one widening JSON Schema
        // makes and the only place this would otherwise report a lie.
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
    // `schemars` writes a Rust `usize` as `"format": "uint"`, an `i64` as
    // `"int64"` and an `f64` as `"double"`. None of those are registered
    // JSON Schema formats, and a client that validates strictly refuses
    // them: OpenCode reported `unknown format "uint"` on every tool taking
    // a limit. The schema loses nothing — the type is still `integer`, and
    // `usize` still carries its `minimum: 0`.
    let server = LeteoMcpServer::with_options(
        Arc::new(Mutex::new(
            Store::open(crate::store::StoreConfig::new(
                tempfile::tempdir().unwrap().path().join("mcp.db"),
            ))
            .unwrap(),
        )),
        McpOptions::default(),
    );

    // Both halves. Checking only the input is what let this ship half
    // fixed: a tool takes two or three numbers and hands back a dozen, so
    // most of the offending formats were in the output schema.
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

    // And the constraint the format was standing in for survives, so the
    // stripping is not quietly widening what a tool accepts.
    let search = server
        .router
        .list_all()
        .into_iter()
        .find(|tool| tool.name == "mem_search")
        .expect("mem_search is exposed");
    let limit = &search.input_schema["properties"]["limit"];
    // At least the unsigned floor, and this one is tighter on purpose: zero is
    // a page with nothing on it, which `mem_search` does not answer, so it
    // publishes the one it was already applying. What matters here is that
    // stripping the format did not take the bound with it.
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

/// Every `format` in a schema, however deeply nested.
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
fn exposes_exactly_twenty_two_tools_with_output_schemas() {
    let tools = LeteoMcpServer::router().list_all();
    let names: BTreeSet<_> = tools.iter().map(|tool| tool.name.as_ref()).collect();

    assert_eq!(tools.len(), 22);
    assert_eq!(
        names,
        BTreeSet::from([
            "mem_capture_passive",
            "mem_compare",
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
    let delete: DeleteParams = serde_json::from_value(json!({ "id": 1 })).unwrap();
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

    // Trimmed, because the store trims what it is given and the whole point
    // of the last assertion is that nothing else was changed.
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

    // A search is for choosing which memory to read, so it previews.
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
    // The title is never cut: it is what the agent chooses by.
    assert_eq!(hit.title, "A memory with a long body");

    let context = server
        .mem_context(Parameters(
            serde_json::from_value(json!({ "project": "leteo" })).unwrap(),
        ))
        .unwrap()
        .0;
    assert!(context.observations[0].content_truncated);
    assert!(context.observations[0].content.len() < PREVIEW_BYTES + 32);

    // Asking for one memory by id is asking to read it.
    let whole = server
        .mem_get_observation(Parameters(
            serde_json::from_value(json!({ "id": id })).unwrap(),
        ))
        .unwrap()
        .0;
    assert!(!whole.observation.content_truncated);
    assert_eq!(whole.observation.content, long);

    // A short body is not marked, and the flag stays out of the payload.
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
        // The save echoes back what the caller already sent, so it previews.
        assert!(saved.observation.content_truncated);
        ids.push(saved.observation.id);
    }

    // A timeline previews everything it hands back, focus included.
    //
    // The focus used to arrive whole, on the grounds that the caller named it
    // by id. Two things say otherwise, and both were already written down: the
    // tool's own description promises a 400-character preview and points at
    // `mem_get_observation` for the full body, and the three-layer pattern this
    // module documents puts opening a body whole in that tool rather than this
    // one. With a 20,000-byte body the reply came to 20,851 bytes.
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

    // So is the review queue. Built directly, because what is due for
    // review depends on a date months out and the shape is what matters.
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

    // A name that is neither a profile nor a tool is refused, naming what
    // there is. It used to be kept as though it were a tool, so it matched
    // nothing and every route was removed: `--tools=agnet` started a memory
    // server with no memory tools on it, in silence, and `--tools=AGENT` did
    // the same. What an agent sees then is "Leteo's tools are missing", which
    // the skill answers with "run `leteo setup` and restart" — a typo sending
    // somebody to reinstall an install that was fine.
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
    // And the empty selection stays what it was: everything, rather than a
    // server nobody can ask anything.
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
    // Two of these cannot be undone and one of them can. `mem_delete` writes a
    // tombstone by default and the body stays in the row; `mem_update` and
    // `mem_save`-under-an-existing-`topic_key` replace stored text with nothing
    // kept anywhere. The pair below is driven rather than asserted — see
    // `no_tool_that_replaces_stored_text_calls_itself_additive`, which is what
    // found the two that said they only added.
    let destructive =
        BTreeSet::from(["mem_delete", "mem_merge_projects", "mem_save", "mem_update"]);

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
    store
        .create_session("known", "known-project", "C:/workspace")
        .unwrap();
    let detection = ProjectDetection {
        project: "leteo".to_owned(),
        source: crate::project::SOURCE_GIT_ROOT.to_owned(),
        path: "C:/workspace".to_owned(),
        available_projects: Vec::new(),
        warning: None,
        error_hint: None,
    };

    // The detected project always wins when nothing is requested, and the
    // envelope reports the detection that produced it.
    assert_eq!(
        server
            .resolve_write_project(&store, None, &detection, ProjectChoice::default())
            .unwrap(),
        (
            "leteo".to_owned(),
            crate::project::SOURCE_GIT_ROOT.to_owned()
        )
    );
    // Requesting the detected project, or one the store already knows, works.
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

    // An invented project is refused with the list of real ones.
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

#[test]
fn ambiguous_projects_require_a_replayed_recovery_token() {
    let (_temp, server) = test_server(McpOptions::default());
    let store = server.lock_store().unwrap();
    let detection = ambiguous_detection();

    // A sessionless write into an ambiguous directory fails with a token.
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

    // Choosing a project without the reason or the token stays blocked.
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

    // The replayed choice is accepted, and stays bound to that project.
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

    // Nothing to link to yet: a save before any prompt stays unlinked.
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

    // An automated save says so and stays unlinked.
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

    // A different session is different work, so the link must not leak.
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

    // A write through a session reports the session as the authority.
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

    // Reads report the scope the caller asked for.
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

    // A read that names no project narrows to the one the server is standing
    // in, the same way a write does. It used to answer from every project at
    // once, which made `all_projects` a widening of something already as wide
    // as it goes.
    let context = server
        .mem_context(Parameters(serde_json::from_value(json!({})).unwrap()))
        .unwrap()
        .0;
    // Compared with what the server says about where it is standing, rather
    // than with a route or a name spelled out here. Both were wrong to pin:
    // this asserted `git_root`, which is only what a checkout with no remote
    // detects — the moment this repository had an `origin` the same directory
    // resolved by `git_remote` and the suite failed on a fresh clone, which is
    // the first thing a contributor runs. The name went the same way: it is the
    // remote's when there is one and the directory's when there is not, so a
    // fork under another name, or a second checkout in `leteo-2`, broke it too.
    //
    // What the test is for survives all of that: a read that names no project
    // narrows to the same one a write would, and says which.
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

    // And every project is still one word away.
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

    // The envelope is flattened into the response, matching upstream's shape.
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
    // These used to arrive as `store_error` reading "database error:
    // Invalid parameter name: ...", which blames SQLite for something the
    // agent can actually fix.
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
    // The session context and the per-prompt hint both warn. An agent that
    // follows one of those here to read the whole memory was getting a clean
    // copy with the warning dropped on the way — which is the moment it counts.
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

    // On the memory, not beside it, because that is where every listing puts
    // it — see `the_warning_against_a_memory_is_in_the_same_place_everywhere`.
    let rendered = serde_json::to_value(&overturned).unwrap();
    assert!(
        rendered["observation"].get("caveats").is_some(),
        "{rendered}"
    );

    // The memory that did the superseding still stands, and an empty list is
    // left out of the JSON entirely rather than shipped as `[]`.
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
    // Pinning is a deliberate act. Counting pins against the same budget as
    // recent memories meant the reward for deciding what matters was to stop
    // being told what had happened — from a tool whose description promises
    // "pinned and recent observations".
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
        // Exactly the default budget, so the old rule left no room at all.
        for index in 0..20 {
            let saved = store
                .add_observation(memory(&format!("Pinned {index}")))
                .unwrap()
                .observation;
            store.pin_observation(saved.id).unwrap();
        }
        for index in 0..20 {
            store
                .add_observation(memory(&format!("Recent {index}")))
                .unwrap();
        }
    }

    let params: ContextParams = serde_json::from_value(json!({ "project": "leteo" })).unwrap();
    let out = server.mem_context(Parameters(params)).unwrap().0;

    // Counted across both, because the question is whether a memory is
    // handed over at all. The newest few arrive with their content and the
    // rest as titles, and which side of that line one falls on is not what
    // this test is about.
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
    // `agent` is what `leteo setup` installs, so a tool nobody remembered to
    // put in a profile is invisible to every default install and reachable
    // only with `--tools all`. Nothing else would say so: the router registers
    // it, its schema is valid, and it simply never appears.
    //
    // The other direction matters as much. A profile naming a tool that does
    // not exist — a rename, a typo — silently exposes one fewer tool.
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

/// A search that finds nothing says why, and one that finds something does not.
///
/// An empty result reads exactly like "this was never saved", which is what an
/// agent asking in one language about a store written in another concludes. The
/// hint is worth its tokens only in that case, so a search that matched has to
/// stay silent.
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

    // The Spanish for the word that just matched. Nothing in the store carries
    // it, which is the whole failure this hint exists for.
    let missed = search("cobertura");
    assert_eq!(missed.count, 0);
    let hint = missed.hint.expect("an empty search carries the hint");
    assert!(
        hint.contains("mem_context"),
        "the hint has to name the tool that needs no query: {hint}"
    );

    // And the third case: the words are not all there, so the search widened
    // and found it anyway. That is a weaker claim than an exact match, and the
    // answer has to make the difference visible — on the whole result and on
    // each row, since only some rows of a longer answer may be partial.
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

#[test]
fn mem_context_spends_its_budget_on_memories_the_way_the_session_opening_does() {
    // Two surfaces hand context to an agent: the session-start hook, through
    // `recall::assemble_counted`, and this tool, which the skill names as the
    // way to recover context mid-session. The hook has folded session
    // summaries onto their sessions and fetched generously since a real store
    // showed most of a busy project's recent memories were summaries. This
    // tool did neither, so the tool an agent reaches for deliberately was
    // quietly the worse of the two — on a real project, 7 of the 20 memories
    // it returned were summaries.
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
        // Saved after them, so a budget of four would be filled entirely with
        // summaries if they were truncated before being folded.
        for n in 0..8 {
            save(
                "session_summary",
                &format!("Session summary {n}"),
                "project",
            );
        }
        // And one memory in another scope, behind the whole lot.
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

    // The scope filter used to run after the fetch, so a narrowed request
    // returned whatever survived out of the first `limit` rows rather than the
    // memories that actually match. One personal memory behind twelve project
    // ones is the shape that exposes it.
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
    // The store clamps `limit` to its configured maximum and the parameter
    // documents that — but the reply did not, and a clamped answer is the same
    // shape as an exhausted one. An agent that asked for fifty and got twenty
    // could not tell "that is everything" from "there is more, ask
    // differently", so twenty read as the whole truth.
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

    // More asked for than the store will ever give, and the cap is what ended
    // the list. That has to be said.
    let capped = search(json!({ "query": "migration", "limit": cap + 30 }));
    assert_eq!(capped.count, cap);
    let hint = capped.hint.expect("a clamped search says it was clamped");
    assert!(
        hint.contains(&cap.to_string()),
        "the hint has to name the number that stopped it: {hint}"
    );

    // Asking for less than the cap is the caller's own limit rather than the
    // store's — and that used to be the end of it, on the grounds that a limit
    // somebody chose needs no explanation. It does. A full page and an
    // exhausted one are the same shape either way, and over sixty real
    // questions eighteen came back with exactly the default ten while
    // seventeen of those had more. The sentence names the caller's limit
    // instead of the store's cap; it does not stay silent.
    let capped_by_caller = search(json!({ "query": "migration", "limit": 3 }));
    assert_eq!(capped_by_caller.count, 3);
    let hint = capped_by_caller
        .hint
        .expect("a page the caller's own limit ended says so too");
    assert!(
        hint.contains("More matched than were returned"),
        "and says which limit it was: {hint}"
    );

    // And a query that simply ran out has nothing to explain either, however
    // much was asked for — the cap never came into it.
    let short = search(json!({ "query": "\"Migration note 1\"", "limit": cap + 30 }));
    assert!(short.count < cap, "{}", short.count);
    assert!(short.hint.is_none(), "{:?}", short.hint);

    // Asking for exactly the cap is the case both sentences used to miss, and
    // it is the one an agent that wants everything actually asks. The store's
    // maximum ended the list, so "ask again with a higher limit" is the one
    // piece of advice that cannot work — and the reply said nothing at all,
    // because `more` was decided by a probe row the cap had already thrown
    // away. It is the same shape as the exhausted answer above.
    let at_the_cap = search(json!({ "query": "migration", "limit": cap }));
    assert_eq!(at_the_cap.count, cap);
    let hint = at_the_cap
        .hint
        .expect("a page the store's own maximum ended says so at the cap too");
    assert!(
        hint.contains(&cap.to_string()) && !hint.contains("higher limit"),
        "at the cap a higher limit is not the remedy: {hint}"
    );

    // And the mirror of it, which is what the clamped sentence used to get
    // wrong in the other direction: matching exactly the cap while asking for
    // more is a complete answer, and calling it "not everything that matched"
    // was false. The question is what came back, never what was requested.
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
            store.delete_observation(row.observation.id, true).unwrap();
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
    // The fallback that fixed the cross-process link looks for the prompt in
    // the session the memory lands in, on the reasoning that a session is one
    // conversation. That is true of an agent's session and false of the one
    // most memories actually land in: a save with no `session_id` goes to a
    // stable per-project bucket, `manual-save-<project>`, and prompts are never
    // written there — the hook records them under the agent's own session.
    //
    // Measured on a real store: 1,081 of 3,682 memories sit in a manual-save
    // session, against 4 of 817 prompts. For those the link could not be made
    // whatever the fallback did, which is 29% of everything saved.
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

    // No session named, which is how an agent saves unless it says otherwise.
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

    // The other half of what makes that safe. A question from another sitting
    // is not what this memory answers, and a link nobody can tell from a right
    // one is worse than admitting there is none.
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
    // `capture_prompt` defaults to true and the schema has carried
    // `prompt_sync_id` from the start, but a real store of 3,550 memories held
    // it on exactly none of them. The server keeps the link in memory, set by
    // `mem_save_prompt` — and prompts are captured by the
    // `user-prompt-submit` hook, which is a separate process. The server's
    // copy stayed `None` for every save anybody ever made.
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store
            .create_session("linked", "leteo", "C:/workspace")
            .unwrap();
        // Written the way the hook writes it: straight to the store, with
        // nothing told to this process.
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

    // And a caller that asks not to be linked is not linked.
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

/// A summary has to say which session it was, not merely that there was one.
///
/// Every one of them was called `Session summary: <project>` and nothing else,
/// so on a real store 507 memories shared a name and 9.6% of summaries could be
/// found by their own title, against 99.9% of the memories that had one of
/// their own. Migration `0006` repaired the ones already written; this is the
/// other half, so the next one is not written broken again.
///
/// And the prefix that fix left in front is gone. A title is weighted five
/// times in the ranking, and `Session summary: <project>` put three words that
/// mean nothing about the individual memory across a quarter of the store:
/// searching a summary by its own words measured the same with and without it,
/// while `ledgerly summary` returned ten summaries out of ten with it and one
/// of ten without. What it said is said by the `type` and `project` fields.
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

    // Nothing worth lifting: the plain name beats an invented one, and it is
    // the one case the old prefix was written for.
    assert_eq!(summarize("## Goal\n2026-08-02\n"), "Session summary: leteo");

    // And the point of all of it — the summary can be found by its own words.
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

/// The language setting has to reach the nine clients that run no hooks.
///
/// Of the thirteen agents `leteo setup` configures, four deliver the
/// session-start directive: Claude Code, Codex, ZCode, and OpenCode through
/// its plugin. The other nine — Cursor, Gemini CLI, Windsurf, Kiro, Kilo Code,
/// Qwen, Pi, Antigravity, VS Code Copilot — are configured over MCP alone, so
/// the wizard offered them a language setting that then governed nothing.
///
/// Every instruction file Leteo writes tells the agent to call `mem_context`
/// before acting, which makes it the one route that reaches all of them.
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

    // Unset is auto: whatever the conversation is in.
    let directive = context();
    assert!(
        directive.contains("the language the user is writing in"),
        "auto has to say so: {directive}"
    );

    // And a language that has been pinned is named, on the next call rather
    // than on the next restart.
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

/// The server's own instructions are the only channel every client has.
///
/// Hooks reach four of the thirteen agents `leteo setup` configures, the plugin
/// skill reaches two, and an instruction file reaches twelve — Pi has none, and
/// `--instructions false` removes it for any of the others. What is left is
/// this string, which the MCP handshake hands over before the first call.
///
/// It was unverified prose. Two things are worth holding it to: that every tool
/// it names is a tool, and that a promise about behaviour is one the code keeps.
#[test]
fn the_server_instructions_name_real_tools_and_describe_real_behaviour() {
    let instructions = super::SERVER_INSTRUCTIONS;

    // A renamed tool would leave this naming a phantom, and an agent following
    // the recovery it describes would call something that does not exist.
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

    // And the claim added for the nine clients that never see the skill: a
    // summary's title comes from the first line that is not a heading. Said
    // here and implemented in `normalize::headline`, two files apart.
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

#[test]
fn saving_the_same_memory_again_asks_no_new_questions() {
    // Conflict detection is about a memory arriving, not about a save call.
    // The second save is folded into the row the store already has, and what
    // that row might contradict was asked when it was written.
    //
    // Asking again is not harmless once a settled pair is skipped: the search
    // reaches past it to the next candidates, so the same memory saved ten
    // times would file thirty questions, each worse than the last. Measured
    // against a copy of a real store before this: two saves of one memory,
    // six pending relations from one source.
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
        // Several near-duplicates, not one. With a single match the second
        // save comes back empty whether or not it asked, and the test cannot
        // tell the two apart — verified by removing the guard and watching it
        // still pass. What has to be caught is the search reaching *past* the
        // pairs it already filed to the next ones down.
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
fn the_context_says_which_of_its_memories_were_overturned() {
    // An agent is told a memory has been overturned on a prompt, at a session
    // opening, and when it fetches one whole. Not here — and this is the one
    // that matters most for most clients: of the thirteen agents Leteo
    // configures, four run hooks. The other nine reach context through this
    // tool alone, and every instruction file Leteo writes tells them to call it
    // before acting.
    //
    // So for three quarters of the clients, the only route to context handed
    // over a superseded decision as though it still held.
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
    let listed = context
        .observations
        .iter()
        .find(|observation| observation.id == old.id)
        .expect("the superseded memory is listed");
    assert_eq!(
        listed
            .caveats
            .iter()
            .map(|caveat| (caveat.relation.as_str(), caveat.other_id))
            .collect::<Vec<_>>(),
        [("superseded_by", new.id)],
        "a decision that was overturned was handed over as though it still held"
    );
    // And a memory nothing has been said about carries none, so the field is a
    // warning rather than noise on every entry.
    let current = context
        .observations
        .iter()
        .find(|observation| observation.id == new.id)
        .expect("the newer memory is listed");
    assert!(current.caveats.is_empty());
}

#[test]
fn no_tool_describes_itself_with_the_source_it_was_written_in() {
    // A description is contract: it is what an agent reads to decide which tool
    // answers its question, and some clients render it in a picker. Three of
    // them carried an escaped line break and the twenty-two spaces of Rust
    // indentation that followed it — a hundred and fifteen characters of
    // whitespace between them, and a sentence that broke in the middle wherever
    // it was shown.
    //
    // The same mistake the screen catalogue made once. An escaped line break is
    // a line break in the string; a backslash at the end of a source line eats
    // the break *and* the indentation after it. Only the second one continues a
    // sentence.
    let mut offenders = Vec::new();
    for tool in LeteoMcpServer::router().list_all() {
        let Some(description) = tool.description.as_ref() else {
            continue;
        };
        if description.lines().count() > 1 || description.contains("  ") {
            offenders.push(format!("{}: {description:?}", tool.name));
        }
    }
    assert!(
        offenders.is_empty(),
        "these descriptions carry their own source code:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn no_field_describes_itself_to_an_agent_in_rust() {
    // `schemars` builds these from the doc comments, so whatever is written
    // above a serialized field is shipped to every client that lists the
    // tools. Those comments are for whoever maintains this — they argue about
    // `Option` against a skipped `String`, name Rust types, and carry
    // intra-doc links that arrive as brackets around a name resolving to
    // nothing. Forty-two of a hundred and thirty field descriptions were
    // paragraphs of that, and `tools/list` was 56,862 bytes.
    //
    // The boundary keeps the summary sentence and drops the rest, so this
    // holds the boundary rather than forty comments.
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
                if description.lines().count() > 1
                    || description.contains("  ")
                    || description.contains("[`")
                {
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

/// Every `description` in a schema, however deeply nested.
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
    // "Long bodies come back as a 400-character preview" is what three tool
    // descriptions tell an agent, and the skill tells it the context opens
    // with the first three hundred characters — both so that it knows to fetch
    // the whole memory by id rather than answer from what it can see.
    //
    // The marker that says the text was cut used to be appended *after* the
    // budget, so a caller asking for four hundred got four hundred and
    // fifteen. Nothing noticed: the guard that exists compares the constant
    // against the sentence in the skill, and the delivered length was a third
    // number neither of them mentioned.
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
    // And the cut is still announced inside the text, not only in the flag.
    assert!(content.ends_with("[truncated]"), "{content:?}");
}

/// The context names what is remembered rather than reciting its openings.
///
/// A memory's first four hundred characters are almost never the answer:
/// measured over 2,547 memories of a real store, 91% of the paths,
/// identifiers, numbers and quoted strings fall past them, and not one memory
/// fits inside them — the median runs to 1,991 characters. So the newest few
/// carry a preview and everything behind them is a line, which is what the
/// session-start hook has done since it was measured there and what this tool,
/// the only route the nine clients without hooks have, was still not doing.
///
/// Asserted on the split rather than on a byte count, because the saving is a
/// consequence and the rule is the thing.
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
    // The whole point of the line: it says which memory to ask for.
    assert!(
        out.also_remembered
            .iter()
            .all(|line| line.id > 0 && !line.title.is_empty()),
        "a named memory has to be fetchable"
    );
    // And the quoted ones are the newest, not an arbitrary five.
    assert!(
        out.observations[0].title.contains("19"),
        "the newest is quoted first: {}",
        out.observations[0].title
    );
}

/// The name one tool uses for an identifier is accepted by the others.
///
/// One identifier has four names across twelve tools — `id`,
/// `observation_id`, `memory_id_a`, and a session's `id` where every writing
/// tool says `session_id`. An agent that learned one of them spends a failed
/// call finding out about the next; this was found by making that mistake
/// while reading the store's own output.
///
/// Deserialized rather than called, because what is being asserted is which
/// spellings arrive at all.
#[test]
fn the_other_tools_spelling_of_an_identifier_is_accepted() {
    // One assertion per type rather than a loop, because the four tools that
    // take `id` are four types: a loop that names three tools and
    // deserializes one of them proves nothing about the other two. The first
    // version of this test did exactly that and passed with the alias removed.
    let fetched: GetObservationParams =
        serde_json::from_value(json!({"observation_id": 7})).expect("mem_get_observation");
    assert_eq!(fetched.id, 7);
    let deleted: DeleteParams =
        serde_json::from_value(json!({"observation_id": 7})).expect("mem_delete");
    assert_eq!(deleted.id, 7);
    let pinned: PinParams = serde_json::from_value(json!({"observation_id": 7})).expect("mem_pin");
    assert_eq!(pinned.id, 7);
    let updated: UpdateParams =
        serde_json::from_value(json!({"observation_id": 7})).expect("mem_update");
    assert_eq!(updated.id, 7);

    // And the other way: the tool that says `observation_id` takes `id`.
    let timeline: TimelineParams = serde_json::from_value(json!({"id": 7})).unwrap();
    assert_eq!(timeline.observation_id, 7);

    // A session is a `session_id` everywhere it is written, and an `id` in the
    // two tools that open and close one.
    let started: SessionStartParams = serde_json::from_value(json!({"session_id": "s1"})).unwrap();
    assert_eq!(started.id, "s1");
    let ended: SessionEndParams = serde_json::from_value(json!({"session_id": "s1"})).unwrap();
    assert_eq!(ended.id, "s1");
}

/// A search says when what it found has been overturned.
///
/// Three routes hand a memory to an agent, and each was fixed on its own: the
/// session-start context, then `mem_context`, then this one — which is the
/// most used of them and was the last still quiet. A superseded decision
/// reads exactly like one that still holds.
#[test]
fn a_superseded_memory_is_flagged_in_search_results_too() {
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
        // Proposed and then judged, which is the path a real verdict takes.
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

    let found = out
        .results
        .iter()
        .find(|result| result.observation.id == old.id)
        .expect("the superseded memory is in the results");
    assert_eq!(
        found.observation.caveats.len(),
        1,
        "a search result has to carry what the graph says about it"
    );
    assert_eq!(found.observation.caveats[0].other_id, new.id);
    // And the memory that did the superseding is not itself flagged as stale.
    let newer = out
        .results
        .iter()
        .find(|result| result.observation.id == new.id)
        .expect("both are in the results");
    assert!(
        newer
            .observation
            .caveats
            .iter()
            .all(|caveat| caveat.relation != "superseded_by"),
        "the newer decision still holds"
    );
}

/// A memory says what is true of it, and stays quiet about the rest.
///
/// Four fields and two timestamps were sent on every memory to say nothing:
/// never revised, never duplicated, active, not pinned, and the instant it was
/// written repeated twice more. On a real store that was 1,580 bytes of a
/// 22,620-byte context and 16% of a search result.
///
/// Absence has to mean the default, so the schema marks them optional through
/// `default` — asserted here as well, because dropping the `default` while
/// keeping `skip_serializing_if` produces a schema that requires a field the
/// server never sends.
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
    // What is true of it is still there.
    assert!(result["id"].is_number());
    assert!(result["created_at"].is_string());
}

/// A read with no project named answers about the project it is standing in.
///
/// Writes have detected this from the start; reads never did. With no
/// `--project` on the command line — which is how every installation launches
/// the server — every search and every context answered from every project at
/// once, and `all_projects` widened something already as wide as it goes.
/// Asking 150 real questions of one project on a real store returned another
/// project's memory in the top three 18.7% of the time and pushed one of its
/// own out of the answer 8% of the time.
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

    // These tests run inside Leteo's own repository, so that is the project.
    let narrowed = search(json!({ "query": "retry budget" }));
    assert_eq!(narrowed.count, 1, "{:?}", narrowed.results);
    assert_eq!(
        narrowed.results[0].observation.project.as_deref(),
        Some("leteo")
    );

    // And the other one is not lost, only out of scope until it is asked for.
    let widened = search(json!({ "query": "retry budget", "all_projects": true }));
    assert_eq!(widened.count, 2, "{:?}", widened.results);
    let asked_for = search(json!({ "query": "retry budget", "project": "another-thing" }));
    assert_eq!(asked_for.count, 1);
    assert_eq!(
        asked_for.results[0].observation.project.as_deref(),
        Some("another-thing")
    );
}

/// A capture that saved nothing says why.
///
/// Three zeros is what this answered, and an agent reading them cannot tell
/// "there was nothing worth keeping" from "you wrote that section in a shape I
/// do not read". It is the second far more often: of 872 real subagent outputs
/// on this machine, none carried the heading extraction waits for.
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

    // And a capture that worked says nothing extra.
    let worked = capture("## Key Learnings\n- the pool was never returned on the error path");
    assert_eq!(worked.extracted, 1);
    assert_eq!(worked.saved, 1);
    assert!(worked.hint.is_none(), "{:?}", worked.hint);
}

/// `mem_compare` records a verdict about a pair nobody proposed, and it asks
/// for what the store will keep — no more.
///
/// The one tool in this file no test reached: 41 of its 42 lines were never
/// executed, which is how it came to ask for a `confidence` and a `reasoning`
/// as required fields while `mem_judge`, which records the same verdict about
/// the same kind of pair, has always taken both as optional and the column
/// accepts neither. A number a language model produces because a field is
/// required is noise in a column every reader treats as a probability.
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

    // The whole verdict, and then the same verdict with nothing but the verb.
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

    // What it does refuse.
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

    // And the documented no-op: agreeing that two memories do not conflict is
    // a success that files nothing.
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

/// Moving a memory between projects goes through the door creating one does.
///
/// `mem_save` refuses a project this store has never heard of, and names the
/// one the directory resolves to. `mem_update` took the same string and wrote
/// it, so the guard held for creating a memory and not for moving one.
///
/// It is not a cosmetic difference. Every read narrows by project, so the
/// memory stayed in the store and left every search, every opening context and
/// every hint at once — present, and findable by nobody. Reproduced through
/// the protocol: a save into `proyecto-que-no-existe` is refused, the same
/// name through an update is accepted, and searching the memory's own words
/// then returns zero.
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

    let Err(error) = update(json!({ "id": id, "project": "a-project-that-is-not-here" })) else {
        panic!("a memory cannot be moved somewhere that does not exist");
    };
    let payload = error_payload(&error);
    assert_eq!(payload["error"]["code"], "unknown_project");
    assert_eq!(
        payload["detected_project"], "leteo",
        "and the refusal says where it is, the way a save's does"
    );

    // An update that does not mention the project leaves it alone, rather than
    // asking the door about a move nobody requested.
    let renamed = update(json!({ "id": id, "title": "A memory of leteo, retitled" }))
        .expect("an ordinary edit is not a move")
        .0;
    assert_eq!(renamed.observation.project.as_deref(), Some("leteo"));

    // And a project that does exist is still somewhere a memory can go.
    server
        .mem_session_start(Parameters(
            serde_json::from_value(json!({ "id": "s2", "project": "somewhere-real" })).unwrap(),
        ))
        .unwrap();
    let moved = update(json!({ "id": id, "project": "somewhere-real" }))
        .expect("a project the store knows is a place a memory can move to")
        .0;
    assert_eq!(moved.observation.project.as_deref(), Some("somewhere-real"));
}

/// What `mem_judge` accepts, what it refuses, and what a second verdict does
/// to the first.
///
/// The tool the server instructions tell every agent to use for the candidates
/// a save proposes, with 34 of its 45 lines unexecuted. Everything here was
/// first driven through the protocol against a copy of a real store; this is
/// the same sequence, on a fixture, so it stays true.
///
/// The judgment is created through `mem_compare` rather than through a save,
/// and that is not a shortcut: `find_candidates` scores with bm25, whose idf
/// collapses on a corpus of three, so a small fixture proposes nothing to
/// judge no matter how alike its memories are. What conflict detection is
/// worth is measured against a real store, in `Store::find_candidates`.
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

    // A second verdict replaces the first entirely, reason included — a reason
    // written about `supersedes` does not describe `related`.
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

    // And a pair whose two memories have ended up in different projects cannot
    // be ruled on at all. Nothing on the real store is in that state today —
    // the guard is preventive — but a memory can be moved after a pair is
    // proposed, and a relation that spans projects is a leak between them.
    {
        let mut store = server.lock_store().unwrap();
        store.enroll_project("somewhere-else").unwrap();
        store
            .update_observation(
                first.id,
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

/// The review cycle: what is due, marking one, and the two refusals.
///
/// Three types come due at all — decision, policy, preference — so a listing
/// is empty on almost any young store, which is what it means rather than a
/// sign that nothing works. Marking one pushes its date out from today.
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

    // Backdated behind the store's back, because saving and marking in the
    // same second land on the same six-month date and the assertion below
    // would compare a value with itself. This is also what a memory that is
    // actually due looks like.
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

/// A store another process is writing to is a refusal with a next step.
///
/// Leteo is multi-writer by design — the hooks, this server, the CLI and the
/// background sync all open the same file — so a save landing while a hook
/// writes is ordinary rather than exceptional. What an agent used to get was
/// `store_error: database is locked`, which is SQLite's sentence about itself:
/// nothing in it says the memory was not written, and nothing says that asking
/// again is the whole remedy.
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

    // And it is true: the same call works once the other writer is done.
    holder.execute_batch("ROLLBACK").unwrap();
    assert!(
        save("While somebody else was writing").is_ok(),
        "the remedy the message names has to be the remedy"
    );
}

/// The skill tells agents what `store_busy` means, in both bundles.
///
/// The code is a contract: an agent branches on it, and this is the one
/// failure whose remedy is to send exactly the same call again. A code the
/// skill never mentions is one an agent handles by guessing.
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

/// No sentence an agent reads carries the source it was written in.
///
/// Two guards already cover the schema surface — tool descriptions and field
/// descriptions. Neither sees the free text: the hints a tool answers with,
/// the instructions the server opens with, the block a session starts with.
/// That text has the same hazard and has hit it three times, most recently in
/// a hint written earlier the same day this test was added, which carried
/// eighteen spaces of Rust indentation into the middle of its own sentence.
///
/// A backslash at the end of a source line eats the newline *and* the
/// indentation after it. Without one, both end up in the string. Only the
/// first continues a sentence, and nothing but a test can tell them apart
/// after the fact.
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
            "nothing extracted hint",
            crate::mcp::output::NOTHING_EXTRACTED_HINT.to_owned(),
        ),
        (
            "unnamed summary hint",
            crate::mcp::output::UNNAMED_SUMMARY_HINT.to_owned(),
        ),
        (
            "unfiled kind hint",
            crate::mcp::output::UNFILED_KIND_HINT.to_owned(),
        ),
        (
            "memory directive",
            crate::setup::MEMORY_DIRECTIVE.to_owned(),
        ),
        ("language directive", settings.language_directive()),
    ];
    let mut offenders = Vec::new();
    for (name, sentence) in sentences {
        // A run of spaces *inside* a line, which is the signature. Indentation
        // at the start of one is markdown — the memory directive nests
        // continuation lines under numbered items on purpose — and a paragraph
        // break is newlines rather than spaces. What no sentence ever has is a
        // gap in the middle of it.
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

/// A summary nobody could find again says so, while it can still be fixed.
///
/// A summary takes its title from the first line of its body that is not a
/// heading. When there is none — a heading and a date, which is what the
/// server instructions warn about in as many words — it falls back to
/// `Session summary: <project>`, which is what several hundred of them were
/// called before headlines existed and what made them unfindable: 9.6%
/// retrievable by their own words against 99.9% of memories with a title.
///
/// The agent that wrote it is the only one who can name it, and only while it
/// still remembers what the session was for.
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

    // A summary that says what the session was for is named after itself, and
    // gets no hint, because there is nothing to say.
    let named = summarise("## Goal\nTeach the opening block to fold summaries onto sessions\n");
    assert_eq!(
        named.observation.title,
        "Teach the opening block to fold summaries onto sessions"
    );
    assert!(named.hint.is_none(), "{:?}", named.hint);
}

/// A key the tool suggests is a key a lookup can find.
///
/// Two rules for one thing: a stored key kept everything but whitespace, so
/// `decisión` stayed `decisión`, while `mem_suggest_topic_key` kept only
/// `[a-z0-9]` and gave back `decisi-n`. So the tool the skill points agents at
/// produced keys that a search for the same words could never match — and the
/// exact-key branch is the one that puts a memory *first* rather than ranking
/// it among its family, so what was lost was the whole point of having a key.
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

    // Saved under exactly that key, and looked up by exactly that key.
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

/// The number the descriptions publish is the number the code cuts at.
///
/// Three tool descriptions tell an agent that long bodies come back as a
/// "400-character preview", and that number is why it knows to fetch the whole
/// memory by id rather than answer from what it can see. The guard beside this
/// one checks that what is delivered fits `PREVIEW_BYTES`; nothing checked
/// that `PREVIEW_BYTES` is what the sentence says. Changed to 500, the code
/// would cut at 500, the descriptions would still promise 400, and both tests
/// would stay green.
///
/// `CONTEXT_PREVIEW_CHARS` has had this guard against the skill text since it
/// was written. This is its twin, against the tool descriptions.
#[test]
fn the_descriptions_publish_the_preview_length_the_code_cuts_at() {
    let published = format!("{PREVIEW_BYTES}-character preview");
    let mut saying = 0;
    for tool in LeteoMcpServer::router().list_all() {
        let Some(description) = tool.description.as_ref() else {
            continue;
        };
        // The tools that *make* a preview, not the one whose description
        // mentions previews to say it does not make one.
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
    // Every tool that hands back a previewed body, by name. Four of them did,
    // and only two used to say so: an agent reading a timeline or a review
    // list was answering from a cut body with nothing in the description to
    // warn it. The flag was always there; the sentence that tells an agent to
    // expect the flag was not.
    //
    // `mem_update` is the fifth, and was the one write that echoed the body
    // whole: updating nothing but the title of a memory with a 4,000-byte body
    // sent back 4,556 bytes, which is byte for byte what `mem_get_observation`
    // sends. `mem_get_observation` is the only tool that promises the body in
    // full, and it is the only one that may leave this sentence out.
    for expected in [
        "mem_search",
        "mem_context",
        "mem_timeline",
        "mem_review",
        "mem_update",
        // The sixth, and the same reason as the fifth: the caller typed these
        // words a moment ago. People paste, and the longest prompt on a real
        // store is 13,974 bytes.
        "mem_save_prompt",
        // The seventh and eighth, and the largest of the lot: a judgment with
        // 12,000 bytes of reason and 12,000 of evidence came back as 24,359
        // bytes of the caller's own words, and a 12,000-byte session summary
        // as 12,171.
        "mem_judge",
        "mem_session_end",
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
        saying, 8,
        "a tool started or stopped previewing and the list above did not move"
    );
}

/// A memory filed under a word nothing searches for says so.
///
/// A kind outside the eight is kept verbatim, because folding is only safe for
/// a synonym with one obvious target and `optimization` has none — it could as
/// easily be a bugfix, a decision or a discovery. So the memory survives with a
/// type a search narrowed by type can never return, and the only person who can
/// fix that is the one who just wrote it. A real store held 36 across five
/// words, four of them saved on the day this was written.
#[test]
fn a_memory_filed_under_a_word_nothing_searches_for_is_told_so() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store
            .create_session("unfiled", "leteo", "C:/workspace")
            .unwrap();
    }

    let unfiled = server
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
        unfiled.observation.kind, "optimization",
        "the word survives"
    );
    assert_eq!(
        unfiled.hint.as_deref(),
        Some(crate::mcp::output::UNFILED_KIND_HINT),
        "and the agent is told a filter will never reach it"
    );

    // One of the eight says nothing, and neither does a synonym that folded
    // onto one of them — the memory is filed where a filter looks either way.
    for (kind, stored) in [("discovery", "discovery"), ("bug", "bugfix")] {
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

    // A session summary is not one of the eight and is not a mistake: nothing
    // outside Leteo writes one, and the tools that list them ask by name.
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

/// The typed handover says the same things the markdown one says.
///
/// Two surfaces build the opening context: `recall::assemble_counted` renders
/// the markdown a hook injects, and `mem_context` answers the nine clients of
/// twelve that run no hooks. Every time the rule was written twice, one copy
/// was the worse of the two.
///
/// Both halves here were found by running the two side by side on a real store.
/// A session was dated by when it opened while the list was ordered by when it
/// last did anything — fixed in the markdown that morning and left standing
/// here, on the surface most clients actually read. And a prompt was handed
/// over whole: 227 bytes to carry 42 of what somebody typed, four of the six
/// fields being ids no tool anywhere accepts.
#[test]
fn the_opening_context_dates_a_session_by_its_activity_and_quotes_a_prompt_plainly() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store
            .create_session("old", "leteo", "C:/workspace")
            .unwrap();
        // Opened in July, so that a start date and a last activity cannot be
        // mistaken for one another.
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

    // The prompt is the words and the date, and nothing a caller cannot use.
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

/// The warning against a memory is in the same place whichever tool hands it
/// over.
///
/// `mem_search` flattens the memory, so a caveat arrives among its own fields;
/// `mem_context` puts one on each listed memory. `mem_get_observation` and
/// `mem_update` used to put it beside the memory instead — so an agent that saw
/// "superseded by #2671" in a search and followed the id to read the whole
/// thing looked where it had just seen one and found nothing. Reading it as
/// "nothing is said against this" presents an overturned decision as current,
/// which is the one thing caveats exist to prevent, and it is the deepest read
/// of the four — the one taken when the answer matters enough to fetch it all.
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

    // The path is the same one in all four, so this is what the test asserts:
    // find the memory, and the caveat is on it.
    let searched = server
        .mem_search(Parameters(
            serde_json::from_value(json!({ "query": "Tabs everywhere", "project": "leteo" }))
                .unwrap(),
        ))
        .unwrap()
        .0;
    let found = searched
        .results
        .iter()
        .find(|result| result.observation.id == older.id)
        .expect("the superseded memory is in the results");
    assert_eq!(found.observation.caveats.len(), 1, "mem_search");

    let read = server
        .mem_get_observation(Parameters(
            serde_json::from_value(json!({ "id": older.id })).unwrap(),
        ))
        .unwrap()
        .0;
    assert_eq!(read.observation.caveats.len(), 1, "mem_get_observation");

    let updated = server
        .mem_update(Parameters(
            serde_json::from_value(json!({ "id": older.id, "content": "revised body" })).unwrap(),
        ))
        .unwrap()
        .0;
    assert_eq!(updated.observation.caveats.len(), 1, "mem_update");

    // The review queue, which is the strongest case of the six: it exists to
    // say "a decision may have gone stale, read it again", and when a later
    // memory has already overturned it the answer is written down.
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

    // And a timeline's focus, which is the same whole read mem_get_observation
    // makes.
    let timeline = server
        .mem_timeline(Parameters(
            serde_json::from_value(json!({ "observation_id": older.id })).unwrap(),
        ))
        .unwrap()
        .0;
    assert_eq!(timeline.focus.caveats.len(), 1, "mem_timeline");

    // And in the JSON, which is what an agent actually reads: one path, never
    // beside the memory.
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

/// A prompt or a summary handed back as context is previewed, not quoted whole.
///
/// A prompt is whatever somebody typed, and people paste. The markdown block
/// has cut them to 200 characters since it was written; this surface sent them
/// whole, so the same handover for the same project came to 1,166 bytes of
/// prompts as markdown and 45,807 as JSON — 87% of a 52,765-byte reply, one
/// prompt of it 13,974 bytes long. After the cut the same call is 9,541.
///
/// At `PREVIEW_BYTES` rather than the markdown's 200: the two surfaces are
/// allowed to preview differently, because a tool result is fetched
/// deliberately while the opening blob is spent whether or not it is read. What
/// they are not allowed to be is unbounded.
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

    // A prompt that fits is untouched.
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

    // And its sibling in the same reply. A summary is written by whoever closed
    // the session and nothing bounded it here: five are listed in every opening
    // context, so one long one is the whole answer — 43,499 bytes of 48,840 on
    // a real store, where the markdown rendered the same session in 249.
    //
    // Asserted beside the prompt rather than in a test of its own, because the
    // two are the same shape in the same reply and only one of them was looked
    // at the first time.
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

/// The seven tools nothing had ever called through their own layer.
///
/// Every one of them had its store function tested and its handler untouched:
/// 139 lines of `tools.rs` with no coverage, all of it the part an agent
/// actually reaches — parameter shapes, project gating, the sentence each
/// answer carries. The two previous times an untested handler here was covered
/// it produced a defect apiece, so this is the same sweep finished.
///
/// One test, because they share a store and the interesting part is what each
/// says rather than a fixture per tool.
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

    // Pinning says which way it went, and is idempotent as its annotation
    // claims: pinning twice is not an error.
    let Json(pinned) = server.mem_pin(Parameters(PinParams { id: first })).unwrap();
    assert!(pinned.pinned, "{pinned:?}");
    server.mem_pin(Parameters(PinParams { id: first })).unwrap();
    let Json(unpinned) = server
        .mem_unpin(Parameters(PinParams { id: first }))
        .unwrap();
    assert!(!unpinned.pinned, "{unpinned:?}");
    // And a memory that is not there is refused rather than reported pinned.
    assert!(
        server.mem_pin(Parameters(PinParams { id: 9_999 })).is_err(),
        "pinning a memory that does not exist has to fail"
    );

    // Stats count what the store holds.
    let Json(stats) = server.mem_stats(Parameters(NoParams {})).unwrap();
    assert_eq!(stats.total_observations, 2, "{stats:?}");
    assert_eq!(stats.total_sessions, 1, "{stats:?}");
    assert_eq!(stats.projects, vec!["leteo".to_owned()], "{stats:?}");

    // The doctor is read-only and answers for a healthy store.
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
    // A check nobody reports is refused, not answered "all clear".
    assert!(
        server
            .mem_doctor(Parameters(DoctorParams {
                project: None,
                check: Some("no_es_una_comprobacion".to_owned()),
            }))
            .is_err()
    );

    // Ending a session attaches the summary, redacted like every other door.
    let Json(ended) = server
        .mem_session_end(Parameters(SessionEndParams {
            id: "s1".to_owned(),
            summary: Some("Cerrada. <private>secreto</private> Fin.".to_owned()),
        }))
        .unwrap();
    let summary = ended.session.summary.clone().unwrap_or_default();
    assert!(!summary.contains("secreto"), "{summary:?}");
    assert!(ended.session.ended_at.is_some(), "{:?}", ended.session);

    // Merging demands both ends, and reports what moved.
    assert!(
        server
            .mem_merge_projects(Parameters(MergeProjectsParams {
                from: "  ,  ".to_owned(),
                to: "leteo".to_owned(),
            }))
            .is_err(),
        "a list of nothing is a caller's mistake, not a merge of nothing"
    );

    // Deleting says which kind of deletion it was, and the soft one leaves the
    // row where `mem_get_observation` can still find it.
    let Json(soft) = server
        .mem_delete(Parameters(DeleteParams {
            id: second,
            hard_delete: false,
        }))
        .unwrap();
    assert_eq!(soft.status, "soft_deleted");
    let Json(found) = server
        .mem_get_observation(Parameters(GetObservationParams { id: second }))
        .unwrap();
    assert_eq!(
        found.observation.state, "deleted",
        "{:?}",
        found.observation
    );
    // And deleting it again is refused rather than reported deleted twice.
    assert!(
        server
            .mem_delete(Parameters(DeleteParams {
                id: second,
                hard_delete: false,
            }))
            .is_err()
    );
    let Json(hard) = server
        .mem_delete(Parameters(DeleteParams {
            id: second,
            hard_delete: true,
        }))
        .unwrap();
    assert_eq!(hard.status, "deleted");
}

/// An empty answer says which of its two reasons it is.
///
/// `mem_search` narrows to the project the directory resolves to, so an empty
/// answer means either "the store has never heard of this" or "it is filed
/// somewhere else". The two call for opposite actions, and the tool said the
/// first for both: an agent told to try fewer, more distinctive words rewrites
/// a question that was already right, comes back empty again, and reports that
/// the store does not know.
///
/// `leteo search` has answered this way since the CLI reads were scoped. The
/// tool nine clients out of twelve actually use had not.
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

    // Standing in a project that holds nothing, with the word filed in another.
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

    // A word nothing anywhere holds keeps the original reason.
    let (count, hint) = ask("garrapinada");
    assert_eq!(count, 0);
    assert!(
        !hint.contains("elsewhere"),
        "a question that comes back empty either way is not a directory problem: {hint:?}"
    );
    assert!(hint.contains("Full-text search"), "{hint:?}");
}

/// And the context an agent reads first says the same thing.
///
/// `mem_context` is what every instruction file Leteo writes tells the agent to
/// call before acting, and for the nine clients of twelve that run no hooks it
/// is the first thing they read. An empty, silent answer reads as "there is no
/// memory here" — so an agent in a directory that resolved somewhere quiet
/// works blind past a store holding thousands one project over.
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

    // A caller who named the project asked about that project, and one who
    // asked for everything has already been given everything.
    assert!(context(Some("leteo"), false).hint.is_none());
    assert!(context(None, true).hint.is_none());

    // And a context that answered says nothing extra.
    let answered = context(Some("otro-proyecto"), false);
    assert_eq!(answered.count, 1);
    assert!(answered.hint.is_none(), "{:?}", answered.hint);
}

/// A full page and an exhausted one stop looking the same.
///
/// The reply already said when the *store's* maximum ended a list. Nothing said
/// when the caller's own limit did, and the default limit is ten: over sixty
/// real questions asked through this binary, eighteen came back with exactly
/// ten and seventeen of those had more. An agent reading a full page was, nine
/// times in ten, reading part of an answer and being told nothing.
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

    // Four of six: the page is full because four is what was asked for.
    let (count, hint) = ask(Some(4));
    assert_eq!(count, 4);
    assert!(
        hint.contains("More matched than were returned"),
        "a page cut by the caller's own limit has to say so: {hint:?}"
    );

    // Six of six: the list ended on its own, and saying otherwise would be a
    // lie that costs a round trip to disprove.
    let (count, hint) = ask(Some(6));
    assert_eq!(count, 6);
    assert!(hint.is_empty(), "{hint:?}");

    // And one past the end is still the end.
    let (count, hint) = ask(Some(20));
    assert_eq!(count, 6);
    assert!(hint.is_empty(), "{hint:?}");
}

/// A field called `count` says what it counts.
///
/// The word on its own says nothing, and twice now a `count` has sat directly
/// above a shorter list and answered a different question. `ReviewOutput` said
/// 1 with an empty list beside it, because the number was how many were marked
/// rather than how many were listed; `ContextOutput` says 50 with five in
/// `observations`, because the other forty-five are titles in
/// `also_remembered`. An agent doing the obvious thing with the two together
/// reads part of an answer and believes the rest went missing.
///
/// So the rule is not "get these two right" but "a bare `count` carries a
/// description", which is shipped to every client that lists the tools and is
/// the only place an agent can read what the number means.
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
    // A guard that looked at nothing would pass too. Three tools answer with a
    // `count` today — search, context and review — and if that ever drops to
    // zero it is this test that stopped working, not the problem that went
    // away.
    assert!(
        examined >= 3,
        "only {examined} `count` fields were examined, so this guard is checking nothing"
    );
}

/// No tool answers with the whole of what it was given.
///
/// The list of tools that preview is held to the code by name, which catches a
/// tool that stops previewing and not a tool that never started. Four did not:
/// `mem_update` sent a memory back whole, `mem_save_prompt` sent the paste back,
/// `mem_judge` sent 24,359 bytes to record one verdict, and `mem_session_end`
/// sent the summary. Each was found by reading a reply rather than by a test.
///
/// So this asks the question by behaviour instead: give every surface something
/// twenty thousand bytes long and require the answer to be small. The one tool
/// that promises a body in full is exempt and says so in its own description,
/// which is the only exemption there should ever be.
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
    // What the agent receives, which is the answer twice.
    //
    // A tool returning `Json<T>` becomes a `CallToolResult` carrying the value
    // in `structured_content` *and* the same JSON serialised into a text block,
    // because the protocol asks for the second one so that a client which
    // predates structured output still gets an answer. This measured the struct
    // alone, so the number it held every surface to was half the number an
    // agent pays: a 20-result `mem_search` against a real store is 16,428 bytes
    // of structured content and 16,957 bytes of the same thing as text, 33,385
    // in all.
    //
    // Measuring the whole result is what makes `ROOM` the size of an answer
    // rather than the size of half of one. It also means that if the text half
    // is ever dropped for clients that negotiate a protocol version which has
    // structured output, this is where the change shows up as a number.
    let mut note = |name: &'static str, value: serde_json::Value| {
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
                    serde_json::from_value(json!({ "id": saved, "title": "nuevo" })).unwrap(),
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

    // And the one that promises the body in full still gives it, or this guard
    // would be satisfied by a store that lost the text.
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

/// The private marker survives nowhere, whatever door the text came in by.
///
/// `<private>…</private>` is a promise that something is written and not kept,
/// and it has been broken twice in two different places: replication applied it
/// to memories and not to prompts, and `mem_judge`'s reason and evidence were
/// the last two write doors that never saw it at all. Both were found by hand,
/// one door at a time, because the guard was one door at a time too.
///
/// This asks the question the way the promise is stated. Push the same secret
/// through every text field of every write tool, then read every text column of
/// every table and require it to be gone. A tenth door added tomorrow that
/// forgets to redact fails here without anybody remembering to add it to a
/// list.
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
    // A third, so the manual verdict below lands on a different pair. Judging
    // the pair `mem_compare` just wrote overwrites its `reason` with a redacted
    // one, and the sweep at the end then finds nothing — which is exactly what
    // happened: `mem_compare` was writing its `reasoning` unredacted and this
    // test went green because the next line covered it over.
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
    // A judgment, which is where the last two doors were found.
    let compared = server
        .mem_compare(Parameters(
            serde_json::from_value(json!({
                "memory_id_a": first.observation.id,
                "memory_id_b": third.observation.id,
                "relation": "related",
                // `reasoning`, which is what this tool calls it. It said
                // `reason` and `evidence` — `mem_judge`'s names, one tool over —
                // and serde dropped both without a word, so this door was
                // called with nothing to redact and passed on it for as long as
                // it has existed. `deny_unknown_fields` is why that cannot
                // happen again, and it is what turned this line red.
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

    // Every text column of every table, including the ones nobody thought of:
    // the mutation journal a replica would replay, and the full-text indexes.
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
    // The same scan, looking for something that *is* meant to be there.
    //
    // Without this the test passes on a store nobody wrote to, on a scan that
    // reads no columns, or on a needle no field could ever have held — three
    // ways of proving nothing while looking thorough.
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

/// No read hands over another project's memory unless it was asked to.
///
/// A read that silently answers from somewhere else is worse than an empty one:
/// on a real store, 72% of the CLI's answers came from another project before
/// the reads were scoped. That was fixed one command at a time, and the guard
/// was written one command at a time too — over the CLI, which is the surface
/// three clients of twelve use.
///
/// This asks it of the tools instead, and asks it of all of them at once: two
/// projects holding the same distinctive word, a process standing in one of
/// them, and nothing from the other may come back. Then the widening is asked
/// for explicitly and has to work, or the guard would be satisfied by a store
/// that answers nothing at all.
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

    // Standing in one project, asking a question both could answer.
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

    // Asked for, the widening works — otherwise a store that answers nothing
    // would pass the half above.
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

/// The verbs a tool offers are the verbs the store accepts.
///
/// They were written out four times: the arms of the check, the test that held
/// the check, and the descriptions two tools ship to every agent. Nothing tied
/// them together, so a seventh verb added to the store would have been offered
/// by nobody, and a verb dropped from the store would have gone on being
/// advertised — an agent following the description into a refusal.
///
/// The refusal now names them too, which is the same list a third time and the
/// reason it is a list at all.
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

    // And the refusal lists them, so a caller that guessed wrong is told what
    // to guess instead.
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

/// A vocabulary a tool takes is a vocabulary the tool names.
///
/// The relation verbs were offered in full by one judging tool and not at all
/// by the other, which a guard found the moment one was written. The same shape
/// was one field over: four tools take a `type`, and only `mem_save`
/// said what the eight are. An agent that narrowed a search by a word it made
/// up got an empty answer and no clue — and 0.93% of a real store is already
/// filed under types no filter can reach, which is that mistake made by
/// somebody who had never been shown the list.
///
/// Held to `KINDS` rather than to a copy of it, so a ninth type has to be
/// offered before this passes again.
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
    // Four tools take a type and three take a scope; a guard that found none
    // would pass while checking nothing.
    assert!(
        checked >= 7,
        "only {checked} vocabulary fields were examined"
    );
}

/// An output description earns its bytes or it does not travel.
///
/// `tools/list` is what every agent reads before it can do anything, and 62% of
/// it was output schemas. A fifth of those were descriptions, and two thirds of
/// those were the same sentences over again: the memory type is embedded in
/// eight tools, so `Absent unless pinned.` shipped eight times.
///
/// The rule is what the value cannot say for itself. `state` and a caveat's
/// `relation` name closed vocabularies that appear nowhere else in the schema;
/// `count` sits above a shorter list and has twice been read as its length;
/// `caveats` is an opaque name. Those stay. `pinned`, `revision_count`,
/// `duplicate_count`, `updated_at` and `content_truncated` say themselves — and
/// the last of those is in every previewing tool's own description already,
/// held there by another guard.
///
/// Both halves are asserted. Dropping a description that was carrying meaning
/// is the same defect as shipping one that carries none.
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
    // The two vocabularies an output carries appear nowhere else in the schema,
    // so the description is the only place an agent can read them.
    assert!(
        described["state"].contains("needs_review"),
        "{:?}",
        described["state"]
    );
}

/// A memory says which question it answers, or says nothing.
///
/// The link is made by a chain of three: the prompt this process last recorded,
/// then the last prompt of the same session, then — only for a save that named
/// no session — the last prompt of the same project inside a time window. Each
/// step is a guess that is right more often than not, and the guard on each is
/// what keeps a wrong guess from being written down as a fact.
///
/// Read one way it is obviously safe; the order things happen in is the part
/// reading cannot settle. So this drives the orders: a second session, a second
/// project, a question asked too long ago, and a save that asks not to be
/// attributed at all.
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

    // The question this conversation just asked.
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

    // A different conversation is a different question, even in one project and
    // even when this process still remembers the other one.
    let elsewhere = save(json!({
        "session_id": "b", "title": "Otra cosa", "content": "otro cuerpo", "type": "discovery",
    }));
    assert_eq!(
        elsewhere.prompt_sync_id, None,
        "a memory in another conversation must not borrow its question"
    );

    // And a save that asks not to be attributed is not.
    let unattributed = save(json!({
        "session_id": "a", "title": "Automatica", "content": "sin pregunta detras",
        "type": "discovery", "capture_prompt": false,
    }));
    assert_eq!(unattributed.prompt_sync_id, None);

    // A save that names no session is in no conversation, so it may take the
    // project's last question — but only one asked recently.
    let asked_loose = ask("a", "una pregunta reciente del proyecto");
    let bucketed = save(json!({
        "title": "Sin sesion", "content": "guardada en el cubo del proyecto", "type": "discovery",
    }));
    assert_eq!(
        bucketed.prompt_sync_id.as_deref(),
        Some(asked_loose.as_str()),
        "a session-less save may answer the project's last question"
    );

    // The same save, with that question old enough to be somebody else's.
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

/// A recovery token is good for one choice, from one directory, once.
///
/// It exists so an agent can retry a save after the user picked between the
/// projects a directory holds. Everything about it is a guard: the same token
/// replays for the same project so a retried call is not refused, and never for
/// a different one, another directory, a changed candidate list, or a name that
/// was never on the list.
///
/// That last one is checked by the only caller and now by the token as well.
/// The list is right here, the check is a lookup, and a rule enforced only by a
/// caller is a rule one new caller away from not existing — which is exactly
/// what happened when `mem_save` guarded a project and `mem_update` did not.
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

    // The choice it was issued over, replayed as often as the caller retries.
    assert!(tokens.redeem(&token, "alpha", &detection));
    assert!(tokens.redeem(&token, "alpha", &detection));
    // But never a second choice.
    assert!(!tokens.redeem(&token, "beta", &detection));
    // Nor a name nobody offered.
    let fresh = tokens.issue(&detection);
    assert!(
        !tokens.redeem(&fresh, "gamma", &detection),
        "a token must not admit a project it was never issued over"
    );
    // Nor from somewhere else, nor once the directory holds different projects.
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

/// No description names some of a vocabulary and not the rest.
///
/// `mem_capture_passive` advertised "Key Learnings or Aprendizajes Clave" — two
/// of the twelve languages a subagent may end in — and went on saying so after
/// the other ten started working. An agent reading it would not send a
/// Portuguese subagent's output at all, which is the same silence the regex
/// used to produce, moved one layer up.
///
/// The canonical English heading is what the skill asks a subagent for, so a
/// description may name it. Naming one of the *other* eleven claims a set, and
/// a claimed set has to be the whole one.
///
/// Two vocabularies now, and the second is why this reads the *parameter*
/// descriptions as well as the tools'. Four parameters said "project or
/// personal" while `normalize::scope` accepted a third and `memory-model.md`
/// §11 had named three all along. The guard was watching the wrong half of the
/// schema: nothing an agent reads about scope is in a tool description.
#[test]
fn no_description_names_part_of_a_vocabulary() {
    let headings: Vec<&str> = crate::memory::normalize::LEARNING_HEADINGS
        .iter()
        .filter(|(code, _)| *code != "en")
        .flat_map(|(_, headings)| headings.iter().copied())
        .collect();
    assert!(headings.len() > 10, "{headings:?}");
    // Scope has no exempt member: `project` is the default and naming it alone
    // is still naming one of three.
    let scopes: Vec<&str> = crate::memory::normalize::SCOPES.to_vec();
    assert_eq!(scopes.len(), 3, "{scopes:?}");
    // And the verdicts, which two tools spell out in full. A seventh verb would
    // be accepted by the check, listed by the refusal, and named by neither
    // description — the list is walked everywhere except in the two sentences
    // an agent reads before choosing one.
    let verbs: Vec<&str> = crate::memory::rules::RELATION_VERBS.to_vec();
    assert_eq!(verbs.len(), 6, "{verbs:?}");

    // Every sentence the schema hands an agent, tools and parameters alike.
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

    // A heading is a distinctive string, so naming one is always naming one of
    // the twelve. The scopes are ordinary words — `project` appears in half
    // these sentences meaning the thing a memory belongs to — so the claim
    // being guarded is narrower and has to be detected as such: a sentence that
    // is *about* scope and names any of them is enumerating the vocabulary.
    // How many named words make a sentence an enumeration rather than a
    // mention. A heading is a distinctive string, so one is already a claim. The
    // scopes and the verdicts are ordinary words — `project` appears in half
    // these sentences meaning the thing a memory belongs to, and `related`
    // reads as English — so scope is asked only of a sentence about scope, and
    // a verdict needs a second one beside it before this calls it a list.
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

/// Nothing the reply may leave out is declared required.
///
/// `#[serde(skip_serializing_if = ...)]` omits a field; `schemars` marks it
/// optional only when `#[serde(default)]` says it can be. Three fields had the
/// first and not the second — `partial` on every search result, and
/// `content_truncated` on the memory that `mem_get_observation`, `mem_save` and
/// `mem_session_summary` hand back — so those replies did not validate against
/// the schema the same reply advertises. A fourth field one struct away had it
/// right, which is the whole shape of it.
///
/// A client that validates strictly rejects the answer outright, and that is
/// not hypothetical here: two earlier defects on this surface, a non-standard
/// `format` and a missing field, were both found by clients doing exactly that.
///
/// The rule has no exception, including for `Option`, where `schemars` gets the
/// answer right today without being told. A field that stops being an `Option`
/// and keeps its `skip_serializing_if` would otherwise put the defect straight
/// back, and nothing about that edit looks like it touches a schema.
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

/// A tool that calls itself additive leaves what was already stored readable.
///
/// The table beside this one says which tools are destructive, and the guard
/// above holds each declaration to it — but nothing held the table to what the
/// tools do. Two of them replaced stored text and said they only added to it.
///
/// `mem_update` overwrites a title and a body in place: the previous text is
/// not in the row, not in a tombstone, not in the replication queue. Nowhere.
/// `mem_save` does the same whenever the `topic_key` matches a memory that
/// exists — that is the revision the key is for, and it is still a replacement.
///
/// The contrast is what makes it worth saying: `mem_delete` was already
/// declared destructive and is the *recoverable* one. It writes a tombstone by
/// default and the body stays in the row. The two that could not be undone were
/// the two claiming they only added.
///
/// Driven rather than read, because the declaration is what was wrong. Each
/// tool is called on a memory whose body is known, and the question asked
/// afterwards is whether that body is still in the store.
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

    // `mem_update`, whose whole purpose is replacement.
    let saved = write("Replaced by an update", body, None);
    assert!(survives(&server), "the body is there to begin with");
    server
        .mem_update(Parameters(
            serde_json::from_value(json!({
                "observation_id": saved.observation.id,
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

    // And `mem_save` again under a `topic_key` that already exists, which is
    // the revision the key is for and is still a replacement.
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

/// Zero means none of that section, and every schema says which zero it is.
///
/// Seven integer parameters published `minimum: 0` — `schemars` derives it from
/// `usize`, the way it derived the `format: uint` that made strict clients
/// reject every tool — and six of them handed back one row anyway. A caller who
/// asked `mem_context` for no sessions, no prompts and no memories got one of
/// each; a caller who asked `mem_timeline` for no neighbours got two.
///
/// The rule the two halves share: zero says leave that section of the answer
/// out, and it does not say "a page with nothing on it". So the section budgets
/// honour it, and the two that are a list's own page size — `mem_search` and
/// `mem_review` — publish the floor of one they were already applying.
///
/// It is a bound worth having rather than a tidiness: not sending the sessions
/// and the prompts takes `mem_context` from 14,401 bytes to 10,921 on a real
/// project, and there was no way to ask.
#[test]
fn zero_leaves_a_section_out_and_the_schema_says_which_zero_it_is() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store.create_session("s1", "leteo", "C:/workspace").unwrap();
        // More than the ceiling, so the ceiling below binds on something. With
        // four memories a cap of twenty is a claim about nothing.
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

    // Everything present when nobody asks otherwise, so the zeros below are
    // subtracting something rather than describing an empty store.
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

    // One section at a time, because three zeros passing together would also
    // pass if one budget silently governed all three.
    let no_prompts = context(json!({ "project": "leteo", "prompt_limit": 0 }));
    assert!(no_prompts.prompts.is_empty(), "{no_prompts:?}");
    assert!(!no_prompts.sessions.is_empty(), "and only that one");
    assert!(!no_prompts.observations.is_empty(), "and only that one");

    // The window around a focus is the same kind of budget.
    // In the middle of the session, so both sides of the window have something
    // in them. With the newest memory as the focus, `after` is empty whatever
    // is asked for, and a zero that changed nothing would pass.
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
    // And the totals still say what is there, which is the whole reason zero is
    // a sensible thing to ask for.
    assert!(timeline.before_total > 0, "{timeline:?}");

    // The other end of the same budget, and the one this surface was missing.
    // A window of a million came back with the whole session — 191 KB on a real
    // one — from the tool whose own purpose says a payload that pushes the
    // useful part out of a context window has failed. The ceiling is the
    // store's, and the schema publishes it.
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

    // The other zero, published rather than discovered.
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
            _ => 0,
        };
        assert_eq!(
            *minimum, expected,
            "{field} publishes a floor it does not apply"
        );
    }
}

/// A parameter this surface does not have is refused, not dropped.
///
/// Serde ignores an unknown field by default, so a misspelling arrived as
/// silence: `mem_search` with `typ` instead of `type` answered nine memories
/// where the filter would have given seven, and one with `proyect` answered
/// nothing at all, because the project fell back to the working directory. A
/// wrong call looked exactly like a right one, which is the one shape an agent
/// cannot recover from.
///
/// It is not hypothetical. The guard that holds the private-text promise called
/// `mem_compare` with `reason` and `evidence` — `mem_judge`'s names, one tool
/// over — and both were dropped, so that door was driven with nothing to redact
/// and passed on it for as long as it has existed. `mem_compare` was in fact
/// writing its `reasoning` into the database unredacted the whole time.
///
/// Every parameter type carries `deny_unknown_fields`, and the refusal names
/// the fields there are, so a caller learns the spelling from the error.
#[test]
fn a_parameter_this_surface_does_not_have_is_refused() {
    let (_temp, server) = test_server(McpOptions::default());
    {
        let mut store = server.lock_store().unwrap();
        store.create_session("s1", "leteo", "C:/workspace").unwrap();
    }

    // The real spelling works, so the refusals below are about the name and not
    // about the call.
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

/// Deserialises a tool's parameters and returns the error, if there is one.
///
/// One per tool by hand, because each has its own type and there is no way to
/// ask the router to deserialise into it — which is the same reason the schema
/// is the only place the names are written down for a caller.
fn parameters_error(tool: &str, arguments: serde_json::Value) -> Option<String> {
    let result = match tool {
        "mem_search" => serde_json::from_value::<SearchParams>(arguments).err(),
        "mem_compare" => serde_json::from_value::<CompareParams>(arguments).err(),
        "mem_context" => serde_json::from_value::<ContextParams>(arguments).err(),
        other => panic!("nobody has taught this test about {other}"),
    };
    result.map(|error| error.to_string())
}

/// Every list `mem_context` hands back has a ceiling, and it is the published one.
///
/// `mem_timeline` was given one for exactly this reason — a window of a million
/// came back with a whole session, 191 KB — and this is the tool nine of the
/// twelve clients have as their only route to context. Its three budgets were
/// all open at the top. Asked for 9,999 of each against a real store, it
/// answered with 1,201 memories, 212 sessions and 120 prompts in one reply of
/// 469 KB; with the ceilings, 43.7 KB. A payload that pushes the useful part
/// out of a context window has failed at the one thing this tool is for.
///
/// The fixture has to overrun all three at once, because each is served by its
/// own query and a ceiling missing from one of them is invisible while the
/// other two hold.
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

    // The fixture really does hold more than each ceiling, or the three
    // assertions above would pass on an empty store.
    assert!(25 > listas && 125 > memorias);

    // And what it applies is what it says: a caller reads the schema, not this
    // file, and a ceiling that is only in the code is one nobody can plan for.
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

    // Untouched below the ceiling: this is a bound on a runaway, not a cut.
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

/// Every budget on this surface publishes its ceiling, not only its floor.
///
/// Both `mem_search` and `mem_review` carried the same note beside their
/// `limit` — that `schemars` derives the floor from `usize` and the store
/// clamps to one, "so the floor is published rather than discovered" — and
/// neither published the other end. `mem_search` applied a ceiling nobody could
/// read: its description says the store clamps to its configured maximum, and
/// what that number is could only be found by asking for more and counting.
/// `mem_review` had none at all, applied or published, and it is the one list
/// where a large number is the obvious thing to ask for: an opening block that
/// says two hundred and sixty-nine memories are due invites asking for two
/// hundred and sixty-nine, which a real store answered in 444 KB.
///
/// Written over the whole surface rather than tool by tool, so that the next
/// budget to arrive cannot arrive without one. The names are the convention
/// this crate uses for a list bound; the count is asserted so that a rename
/// makes this test fail rather than quietly stop looking.
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
    // The positive control: this found seven budgets when it was written, and a
    // test that examined none would also have found nothing wrong.
    assert_eq!(
        examined.len(),
        7,
        "budgets changed; check they all still publish a ceiling: {examined:?}"
    );
}

/// The reread queue stops where it says it stops.
///
/// The ceiling is the store's own for a context read rather than the one
/// `mem_search` uses — both are twenty, so a test that took the wrong accessor
/// would pass by coincidence, which has happened here before. This hands
/// memories over to be read, not a ranked answer to a question.
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
        // Overdue: the queue only returns the ones that are already due.
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

    // And below it nothing is cut, because this is a bound on a runaway.
    let ordinary = server
        .mem_review(Parameters(
            serde_json::from_value(json!({ "action": "list", "limit": 7 })).unwrap(),
        ))
        .unwrap()
        .0;
    assert_eq!(ordinary.count, 7, "{ordinary:?}");
}

/// The window the description publishes is the window the code attributes in.
///
/// `capture_prompt` is the field an agent reads to decide whether to opt out of
/// being linked to a question, and it described one of the three guesses: the
/// prompt this process recorded, same session and project. The other two are
/// not details. A save that names no session — which is every `mem_save`
/// without a `session_id`, and 1,081 memories of 3,682 on a real store — is
/// attributed to the last question asked anywhere in the project within
/// `PROMPT_ATTRIBUTION_MINUTES`, including one asked in somebody else's
/// session. Driven through the built binary, a save with no session picked up
/// a question recorded under a named one, which is right for one agent doing
/// two things and wrong for two agents sharing a project, and either way is
/// not what the field said.
///
/// The number is guarded the way `PREVIEW_BYTES` is: changed here, the code
/// would attribute in a different window and the description would go on
/// promising thirty minutes.
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
    // Folded to one line first: a doc comment wraps, and the line breaks
    // travel into the published description, so `no session_id` arrives with a
    // newline in the middle of it.
    let said = schema["properties"]["capture_prompt"]["description"]
        .as_str()
        .expect("the field is described")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(said.contains(&published), "{said:?}");
    // And that the fallback is named at all, not only its number: a sentence
    // holding "30 minutes" while describing something else would pass on the
    // line above alone.
    assert!(
        said.contains("no session_id") && said.contains("project"),
        "the fallback the number belongs to is named: {said:?}"
    );
}

/// An empty search does not report its own limit as a total.
///
/// The sentence says "nothing here, but N elsewhere", and on this path N came
/// from running the search again with the project narrowing lifted and counting
/// the page. The page is the caller's limit, so a query matching 332 memories
/// in other projects said "1 elsewhere" at `limit: 1`, "3" at 3, "10" at 10 and
/// "20" at 20: the number was the question restated, and `ELSEWHERE_CAP` — the
/// hundred that turns the count into "or more" — was unreachable, since a
/// search never returns more than twenty.
///
/// The count still comes from the search rather than from something cheaper,
/// and that was measured: over 20 empty questions from a real store, the hint
/// fires on 8 of them, while a count of memories matching every word elsewhere
/// fires on none and a count matching any word fires on all 20. The relevance
/// floor inside the search is what makes it worth saying at all, and it costs
/// 128% of the empty answer — 12.2ms against 28.5ms — which is the price of
/// the only version that is right.
#[test]
fn an_empty_search_says_its_count_is_a_floor_when_its_limit_is_what_stopped_it() {
    let temp = tempfile::tempdir().unwrap();
    let mut store =
        Store::open(crate::store::StoreConfig::new(temp.path().join("mcp.db"))).unwrap();
    store.create_session("s1", "otro", "C:/otro").unwrap();
    // Ocho memorias en otro proyecto, todas sobre lo mismo.
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

    // Con un límite que la respuesta llena, el número es un suelo.
    let corto = buscar(2);
    assert_eq!(corto.count, 0, "{corto:?}");
    let dicho = corto.hint.clone().unwrap_or_default();
    assert!(dicho.contains("2 or more elsewhere"), "{dicho}");

    // Con un límite que no llega a llenarse, es el número de verdad.
    let largo = buscar(20);
    let dicho = largo.hint.clone().unwrap_or_default();
    assert!(
        dicho.contains("8 elsewhere") && !dicho.contains("or more"),
        "ocho son ocho cuando caben todos: {dicho}"
    );
}

/// Every tool refuses a field it does not take, including the ones that take none.
///
/// A tool declared without a parameter type publishes
/// `{"type":"object","properties":{}}` — an object schema with no
/// `additionalProperties: false`, which tells a client that extra fields are
/// welcome, and rmcp then hands the call through with them dropped. Two were
/// like that. `mem_stats` accepted `project` and answered with the whole
/// store's numbers: 4,015 memories where the project holds 1,712, and nothing
/// in the reply said the narrowing had gone. Asking is the natural mistake,
/// because every other read on this surface takes a project — and the answer
/// exists, under `mem_doctor`, which is where the description now points.
///
/// Written over the surface rather than over those two, so the next tool
/// without parameters cannot arrive without the refusal. The count is asserted
/// as well, or a rename that made this examine nothing would read as a clean
/// surface.
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
    assert_eq!(examined, 22, "the whole surface was examined");
}

/// And the refusal is real, not only published.
///
/// The schema is what a strict client reads; the server is what a lenient one
/// reaches. Both halves were missing on the two tools that take nothing, so
/// both are held: `mem_stats` given a project says so, and `mem_stats` given
/// nothing still answers — which is how every client calls it.
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

/// An ambiguous directory says it is ambiguous, on every door.
///
/// There are two functions named `project_detection_error` — a method that
/// mints a recovery token, and a free one that does not — and which a call site
/// reaches depends only on whether it has a `self`. `resolve_detected_project`
/// is free, so `mem_session_start` reached the plain one and answered
/// `project_detection_failed`: a code that says detection is broken for a
/// directory where nothing is broken, and not the code the server instructions
/// tell an agent to recognise so it can ask the user. It listed the candidates
/// and then hid what they were for.
///
/// The remedies differ and each error says its own. A write proves the user was
/// asked, with the token. `mem_session_start` takes the name directly — one of
/// the candidates or a new one — which is the sanctioned way to introduce a
/// project, so no token is minted for it and none is required.
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

    // The door without `self`, which is where `mem_session_start` comes in.
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

    // The door with `self`, which is where the writes come in.
    let con_self = cuerpo(server.project_detection_error(&ambiguo));
    assert_eq!(con_self["error"]["code"], error_code::AMBIGUOUS_PROJECT);
    assert!(
        con_self["recovery_token"]
            .as_str()
            .is_some_and(|t| !t.is_empty()),
        "una escritura tiene que probar que se preguntó: {con_self}"
    );

    // And a detection that genuinely failed, with no candidates to offer, still
    // says its own thing: not knowing is not the same as having a choice.
    let rota = crate::project::ProjectDetection {
        available_projects: Vec::new(),
        ..ambiguo.clone()
    };
    assert_eq!(
        cuerpo(project_detection_error(&rota))["error"]["code"],
        "project_detection_failed"
    );
}

/// Every tool's schema describes the refusal as well as the answer.
///
/// A failure comes back as `structuredContent` — that is what carries
/// `error.code`, the `available_projects` an ambiguous directory offers and the
/// `recovery_token` an agent has to replay — and it carries none of the fields
/// the success shape declares required. So a client that validates
/// `structuredContent` against `outputSchema` rejected every error this server
/// returns: driven through the built binary, twelve error replies out of twelve
/// failed their own tool's schema, each on the first required field of the
/// answer they are not.
///
/// That client is not hypothetical. Two defects on this surface were found by
/// OpenCode validating — a `format` JSON Schema has never defined, and a field
/// the reply may omit declared required — and both were about the answer. This
/// is the same defect about the refusal, which is the half an agent most needs
/// to read.
///
/// Written over the surface, so the next tool cannot arrive describing only its
/// happy path, and it asserts what it examined: a schema with nothing required
/// would pass this without the union ever being added.
#[test]
fn every_output_schema_accepts_the_error_shape_as_well() {
    let refusal = json!({
        "error": { "code": "observation_not_found", "message": "observation not found: 1" },
        "available_projects": ["uno", "dos"],
        "recovery_token": "rec-abc",
    });
    let mut examined = 0;
    let mut naked = Vec::new();
    // The server's own router, not the bare one: the union is added where the
    // formats are stripped and the descriptions trimmed, when a server is
    // built. Reading `LeteoMcpServer::router()` shows what `schemars` wrote and
    // none of what this crate does to it — which is how this guard first
    // examined nothing at all.
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(crate::store::StoreConfig::new(temp.path().join("mcp.db"))).unwrap();
    let server = LeteoMcpServer::with_options(Arc::new(Mutex::new(store)), McpOptions::default());
    for tool in server.router.list_all() {
        let Some(schema) = tool.output_schema.as_ref() else {
            continue;
        };
        let schema = serde_json::Value::Object((**schema).clone());
        // Only the tools whose answer demands something; the union is what a
        // demand has to be paired with.
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
        // And the root keeps the shape anything that reads rather than
        // validates expects to find.
        assert_eq!(schema["type"], "object", "{}", tool.name);
        assert!(schema.get("properties").is_some(), "{}", tool.name);
        // The demand did not evaporate: the success branch still names its own.
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

/// A diagnosis shows examples of the damage, not an inventory of it.
///
/// `PRAGMA foreign_key_check` answers one row per orphaned row, and the tool
/// carried every one of them into an agent's context: 300 orphans made a
/// 54.7 KB reply, and it scales with the damage — so the reply is largest
/// exactly when something is wrong and an agent is trying to read what.
///
/// Nothing is lost by cutting it. The count is already a sentence in `issues`,
/// the repair is `--repair` rather than anything done per row, and the store's
/// own report stays whole for `leteo doctor`, where a pipe has no context
/// window to spend. That split is the same one `mem_context` and
/// `leteo context` make.
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
    // Genuinely orphaned: the session goes without the keys standing in the way.
    store
        .connection()
        .execute_batch("PRAGMA foreign_keys=OFF; DELETE FROM sessions WHERE id = 's1';")
        .unwrap();

    // El store cuenta todas, que es lo que el terminal imprime.
    let report = store.doctor().unwrap();
    assert_eq!(
        report.foreign_key_violations.len(),
        orphans,
        "el informe del store va entero"
    );

    let server = LeteoMcpServer::with_options(Arc::new(Mutex::new(store)), McpOptions::default());
    let Json(answered) = server
        .mem_doctor(Parameters(serde_json::from_value(json!({})).unwrap()))
        .expect("the diagnosis answers");
    assert_eq!(
        answered.foreign_key_violations.len(),
        VIOLATION_EXAMPLES,
        "el agente ve ejemplos"
    );
    assert_eq!(
        answered.foreign_key_violations_omitted,
        orphans - VIOLATION_EXAMPLES,
        "y cuántas no ve"
    );
    // Y el número entero sigue dicho donde se decía.
    assert!(
        answered
            .issues
            .iter()
            .any(|issue| issue.contains(&orphans.to_string())),
        "{:?}",
        answered.issues
    );
}

/// The one refusal nobody can retry says whose problem it is.
///
/// A poisoned lock means something panicked while holding the store, so every
/// call after it fails the same way for as long as the process lives. The
/// message was "the Leteo store lock is poisoned" — the state in Rust's words,
/// and not a thing anybody can do with it: an agent reading that retries, gets
/// it again, and reports that memory is broken or empty.
///
/// Every other refusal on this surface carries its own remedy. A busy store
/// says to call again in a moment; a replay without its token says which token;
/// an unknown project names the ones that exist. This is the only kind where
/// the remedy is not the caller's at all, so it says that.
#[test]
fn the_refusal_that_cannot_be_retried_says_so() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(crate::store::StoreConfig::new(temp.path().join("mcp.db"))).unwrap();
    let shared = Arc::new(Mutex::new(store));
    // Envenenar el candado es lo que hace un panic con él en la mano.
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
    // What to do, and what not to.
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

/// Every ceiling a tool publishes is a ceiling something actually applies.
///
/// There are two, and only two. A list of rows stops at the store's own ceiling
/// for a context read; the depth of a context stops at what `--context deep`
/// gives, because asking for more than any installation produces is asking for
/// nothing. Both are applied from one place each.
///
/// Published, they are seven hand-written numbers in `schemars` annotations,
/// which cannot read a constant. So nothing tied the two sides together: a
/// one-line change to either applied ceiling — "let it return more" is a
/// plausible motive — would leave six schemas publishing a limit this server no
/// longer has, and a published limit that is not the applied limit is the rule
/// this codebase has broken most often.
///
/// `VIOLATION_EXAMPLES` is here for the same reason. Its own comment says it is
/// "the same number every other list on this surface stops at", and it was a
/// separate literal saying so.
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

    // The fixture is the surface itself, so it cannot quietly stop reaching
    // anything — but it can stop finding it, which is what this says.
    assert!(
        published.len() >= 7,
        "only {} published ceilings found, so this has stopped reading the schemas: {published:?}",
        published.len()
    );
    let strays: Vec<&(String, String, usize)> = published
        .iter()
        .filter(|(_, _, maximum)| *maximum != list && *maximum != context)
        .collect();
    assert!(
        strays.is_empty(),
        "these publish a ceiling neither {list} (a list of rows) nor {context} (the depth of a \
         context): {strays:?}"
    );
    assert_eq!(
        crate::mcp::output::VIOLATION_EXAMPLES,
        list,
        "the diagnosis lists as many rows as every other list on this surface, which is what its \
         own comment says it does"
    );
}

/// The tool says what the hook says: how many learnings did not fit.
///
/// `mem_capture_passive` and `subagent-stop` are the same door one layer apart,
/// and the ceiling was put on the store underneath both. Only the hook was
/// taught to report it. Left alone, this tool answers `extracted: 500, saved:
/// 80, duplicates: 0` — three numbers that do not add up, with four hundred and
/// twenty memories gone and nothing said.
///
/// The sibling rule, which this codebase has paid for often enough to write
/// down: after fixing something, look for the field, path or surface beside it.
#[test]
fn the_capture_tool_says_how_many_learnings_did_not_fit() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(crate::store::StoreConfig::new(
        temp.path().join("capture.db"),
    ))
    .unwrap();
    let server = LeteoMcpServer::with_options(Arc::new(Mutex::new(store)), McpOptions::default());
    let ceiling = crate::memory::normalize::MAX_LEARNINGS;

    // Sized from the ceiling and past it, or the bound is never reached.
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

    // Exactly one over, because "1 were not stored" is what an agent reads on
    // the surface whose whole job is to be read, and a fixture three over never
    // reaches the branch that says otherwise.
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

    // A capture inside the ceiling says nothing about it, and keeps the hint
    // that was there before for the answer that is far commoner.
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

/// Every number a capture produces reaches both doors that render it.
///
/// `mem_capture_passive` and the `subagent-stop` hook are the same door one
/// layer apart, and both turn a `PassiveCaptureResult` into something an agent
/// reads. When the learning ceiling went in, the store learned to count what it
/// dropped and only the hook learned to say it — so the tool answered three
/// numbers that did not add up, with four hundred and twenty memories gone.
///
/// Counted rather than matched by name, because the two doors name the same
/// facts differently — `saved` against `observations_captured` — and a table
/// mapping one to the other would be the second copy this codebase keeps paying
/// for. A count cannot say *which* field was forgotten, but it cannot be
/// satisfied by remembering to update it either: a fifth number added to the
/// result fails both sides until both carry it.
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

    // The hook side, through the outcome it actually builds.
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

/// `mem_context` bounds its pinned half by the number it was asked for.
///
/// The two lists this tool returns — what was pinned and what is recent — take
/// the same ceiling, and the comment beside them said so while the code did
/// not: the pinned one was `ContextSize::Deep`, the deepest anybody is ever
/// configured to open with. Asked for five against a store with a hundred pins,
/// this answered with eighty-five memories and 73 KB.
///
/// The block that `recall` builds had the same line and its own guard. This is
/// the other door, and nothing held it: restoring the fixed ceiling here left
/// the whole suite green.
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
        store.pin_observation(saved.id).unwrap();
    }
    let server = LeteoMcpServer::with_options(Arc::new(Mutex::new(store)), McpOptions::default());

    // Small first, because that is the ask this used to ignore hardest.
    for asked in [5_usize, 20, deepest] {
        let Json(answer) = server
            .mem_context(Parameters(crate::mcp::params::ContextParams {
                project: Some("leteo".to_owned()),
                all_projects: false,
                scope: None,
                limit: Some(asked),
                session_limit: 5,
                prompt_limit: 5,
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

/// A scope Leteo does not know is refiled, and the reply says so.
///
/// The sibling of the unfiled-type hint, and the louder of the two. A type
/// outside the eight is kept verbatim — the word survives and the memory is
/// merely unfilterable — while a scope outside the three is *replaced*, because
/// losing a memory at the door over a label is a worse answer than filing it
/// where almost all of them belong. So the caller's own value is discarded, and
/// a read narrowed to the scope they asked for will never return the memory
/// they believe they filed there.
///
/// One door said so and the other did not. Driven side by side on the same
/// call, `type: implementation` came back with a hint and `scope: personnal`
/// came back with nothing at all.
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

    // The four corners, because the interesting one is the pair: two mistakes
    // in one call are two things to fix, and a reply that mentions the first
    // and swallows the second sends somebody back for a second round.
    let both_known = save(&server, "Both known", "decision", "personal");
    assert_eq!(both_known.observation.scope, "personal");
    assert!(both_known.hint.is_none(), "{:?}", both_known.hint);

    let odd_kind = save(&server, "Odd kind", "implementation", "personal");
    let hint = odd_kind.hint.unwrap_or_default();
    assert!(hint.contains("type is not one of"), "{hint}");
    assert!(
        !hint.contains("Scope"),
        "nothing was said against the scope: {hint}"
    );

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
    let hint = both_odd.hint.unwrap_or_default();
    assert!(
        hint.contains("type is not one of") && hint.contains("personnal"),
        "two mistakes are two sentences: {hint}"
    );
}

/// The queue says how much of itself this page is not.
///
/// The session opening names the whole queue — "eighteen memories to read
/// again, open it with mem_review" — and sends the agent to a tool that answers
/// with its own page: ten by default, against a ceiling of twenty. Driven end
/// to end against a copy of a real store, that is exactly what happened: the
/// block said eighteen, the tool said ten, and nothing in the reply mentioned
/// the other eight. An agent that marks the ten reviewed has emptied a queue
/// that is not empty.
///
/// The same defect `MORE_MATCHED_HINT` exists for on search, and worse here,
/// because there the caller chose the limit and here another surface named a
/// number first. So the two are held to each other: what the tool carries plus
/// what it left is what the block counts, from the same function.
#[test]
fn the_review_queue_says_how_much_of_itself_this_page_is_not() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(crate::store::StoreConfig::new(
        temp.path().join("review.db"),
    ))
    .unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    // More due than the ceiling, so both the default page and the largest one
    // leave something behind. A fixture that fits is a fixture that watches
    // nothing.
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
    // A second project with a queue of its own, or asking for one project and
    // asking for all of them are the same number and nothing here would notice.
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
    // What the opening block counts, which is the number an agent was given.
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

    // And a page that covers the queue says nothing was left, rather than
    // saying nothing at all.
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
