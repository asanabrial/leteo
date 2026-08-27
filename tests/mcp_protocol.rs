//! The MCP wire surface, driven through the real binary over stdio.
//!
//! The tool-list result carries two fields that SEP-2549 (protocol revision
//! `2026-07-28`) makes required, and a client speaking that revision rejects
//! the whole list — and therefore the connection — without them. That is a
//! contract between processes, so these tests speak the protocol rather than
//! calling handler methods: the negotiated revision lives in the session, not
//! in any type a unit test can hold.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::json;

/// The initialize/tools/list exchange the issue reproduced by hand, against
/// `--tools=agent`, with the store in a temporary directory. Both results
/// come back: what the server answered about itself, and the tool list.
fn exchange_at(protocol_version: &str) -> (serde_json::Value, serde_json::Value) {
    let temp = tempfile::tempdir().expect("create temporary store directory");
    let mut child = Command::new(env!("CARGO_BIN_EXE_leteo"))
        .arg("--database")
        .arg(temp.path().join("mcp-protocol.db"))
        .arg("mcp")
        .arg("--tools=agent")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn leteo mcp");

    let requests = json!([
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": protocol_version,
                "capabilities": {},
                "clientInfo": {"name": "mcp-protocol-test", "version": "1"}
            }
        },
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}
    ]);
    let stdin = child.stdin.as_mut().expect("open stdin");
    for line in requests.as_array().expect("three requests") {
        writeln!(stdin, "{line}").expect("write MCP request");
    }
    drop(child.stdin.take());

    let output = child.wait_with_output().expect("read MCP server output");
    let stdout = String::from_utf8(output.stdout).expect("MCP stdout is UTF-8");
    let messages: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("each MCP line is JSON"))
        .collect();
    let result = |id: i64| {
        messages
            .iter()
            .find(|message| message.get("id") == Some(&json!(id)))
            .expect("response arrived")
            .get("result")
            .expect("response carries a result")
            .clone()
    };
    temp.close().expect("temporary store removed");
    (result(1), result(2))
}

fn list_tools_at(protocol_version: &str) -> serde_json::Value {
    exchange_at(protocol_version).1
}

/// A session knows what revision it is on only if the server echoed it back:
/// the cache fields below are keyed on the negotiated version, so the echo is
/// what ties each assertion to its cause.
fn assert_echo(requested: &str, initialize: &serde_json::Value) {
    assert_eq!(
        initialize.get("protocolVersion").and_then(|v| v.as_str()),
        Some(requested),
        "initialize did not echo the requested protocol revision"
    );
}

#[test]
fn a_2026_07_28_session_gets_both_cache_fields_on_the_tool_list() {
    let (initialize, listing) = exchange_at("2026-07-28");
    assert_echo("2026-07-28", &initialize);

    let keys = listing
        .as_object()
        .expect("tools/list result is an object")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        keys.contains(&"ttlMs".to_owned()) && keys.contains(&"cacheScope".to_owned()),
        "top-level keys {keys:?} must carry both SEP-2549 cache fields"
    );

    let ttl_ms = listing.get("ttlMs").expect("ttlMs is present");
    let ttl_ms = ttl_ms.as_u64().unwrap_or_else(|| {
        panic!("ttlMs is a non-negative integer, got {ttl_ms}");
    });
    assert!(
        ttl_ms > 0,
        "a zero TTL tells clients the list is never fresh"
    );
    assert_eq!(
        listing.get("cacheScope").and_then(|scope| scope.as_str()),
        Some("public"),
        "the list depends on the --tools flag alone, never on who is asking"
    );
}

#[test]
fn older_revisions_still_get_a_tool_list_without_the_cache_fields() {
    // The fields did not exist before 2026-07-28, so they are absent there
    // rather than published for every legacy client to tolerate. Every
    // revision rmcp knows below it is exercised, 2025-11-25 included — that
    // one is also the fallback for a version the server has never heard of.
    for revision in ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"] {
        let (initialize, listing) = exchange_at(revision);
        assert_echo(revision, &initialize);
        let keys = listing
            .as_object()
            .expect("tools/list result is an object")
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            !keys.contains(&"ttlMs".to_owned()) && !keys.contains(&"cacheScope".to_owned()),
            "{revision} answered {keys:?}; its schema names neither cache field"
        );
        assert_eq!(
            listing
                .get("tools")
                .and_then(|tools| tools.as_array())
                .map(Vec::len),
            Some(19),
            "{revision} still lists the 19 agent tools"
        );
    }
}

#[test]
fn the_cache_fields_are_the_only_change_the_tool_list_gains() {
    // The list itself is byte-identical between a new-revision session and an
    // old one: same 19 tools, same names, same schemas, same descriptions.
    // What changed is the envelope, and only the envelope.
    let current = list_tools_at("2026-07-28");
    let legacy = list_tools_at("2025-06-18");

    let current_tools = current.get("tools").expect("tools in the new listing");
    let legacy_tools = legacy.get("tools").expect("tools in the legacy listing");
    assert_eq!(
        serde_json::to_string(current_tools).expect("serialise new tool list"),
        serde_json::to_string(legacy_tools).expect("serialise legacy tool list"),
        "the added cache fields must not reach into the list itself"
    );
    assert_eq!(current_tools.as_array().map(Vec::len), Some(19));
}

#[test]
fn the_list_publishes_no_name_twice() {
    let listing = list_tools_at("2025-06-18");
    let tools = listing
        .get("tools")
        .and_then(|tools| tools.as_array())
        .expect("tool array");
    assert_eq!(tools.len(), 19);
    let names = tools
        .iter()
        .map(|tool| {
            tool.get("name")
                .and_then(|name| name.as_str())
                .expect("tool name")
        })
        .collect::<Vec<_>>();
    let unique = names
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(unique.len(), names.len(), "no name is published twice");
}

#[test]
fn an_unknown_revision_falls_back_to_the_ceiling_and_gets_no_cache_fields() {
    // 2025-11-25 is the server's real ceiling: a client asking for a version
    // nobody knows is answered there, and that revision names neither cache
    // field — the same shape the known legacy revisions get.
    let (initialize, listing) = exchange_at("9999-99-99");
    assert_echo("2025-11-25", &initialize);
    let keys = listing
        .as_object()
        .expect("tools/list result is an object")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        !keys.contains(&"ttlMs".to_owned()) && !keys.contains(&"cacheScope".to_owned()),
        "a fallback session answered {keys:?}; its revision names neither cache field"
    );
    assert_eq!(
        listing
            .get("tools")
            .and_then(|tools| tools.as_array())
            .map(Vec::len),
        Some(19)
    );
}
