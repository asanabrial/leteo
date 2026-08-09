use super::*;

#[test]
fn manifest_codec_has_stable_json() {
    let manifest = Manifest {
        version: MANIFEST_VERSION,
        chunks: vec![ManifestChunk {
            id: "a3f8c1d2".to_owned(),
            created_by: "alice".to_owned(),
            created_at: "2026-07-27T12:00:00Z".to_owned(),
            sessions: 2,
            memories: 3,
            prompts: 1,
        }],
    };
    let golden = r#"{"version":1,"chunks":[{"id":"a3f8c1d2","created_by":"alice","created_at":"2026-07-27T12:00:00Z","sessions":2,"memories":3,"prompts":1}]}"#;

    assert_eq!(encode_manifest(&manifest).unwrap(), golden.as_bytes());
    assert_eq!(decode_manifest(golden.as_bytes()).unwrap(), manifest);
}

#[test]
fn chunk_codec_has_stable_json_and_omits_empty_mutations() {
    let empty = ChunkData::default();
    let golden = br#"{"sessions":[],"observations":[],"prompts":[]}"#;

    assert_eq!(encode_chunk(&empty).unwrap(), golden);
    assert_eq!(decode_chunk(golden).unwrap(), empty);
}

#[test]
fn chunk_codec_accepts_null_arrays_from_go() {
    let decoded =
        decode_chunk(br#"{"sessions":null,"observations":null,"prompts":null,"mutations":null}"#)
            .unwrap();

    assert_eq!(decoded, ChunkData::default());
}

#[test]
fn chunk_id_hashes_uncompressed_json() {
    assert_eq!(chunk_id(b"hello"), "2cf24dba");
}

#[test]
fn canonicalization_matches_golden_and_derives_entity_key() {
    let raw = br#"{
            "sessions":[{"project":"other","id":"s1","directory":"/tmp/s1"}],
            "observations":[{"sync_id":"obs-direct","session_id":"s1","content":"kept","project":"other"}],
            "mutations":[{
                "entity":" observation ",
                "op":" upsert ",
                "payload":"{\"sync_id\":\" obs-1 \",\"session_id\":\" s1 \",\"type\":\" note \",\"title\":\" Title \",\"content\":\" Body \",\"scope\":\" project \",\"project\":\"other\"}"
            }]
        }"#;
    let golden = r#"{"mutations":[{"entity":"observation","entity_key":"obs-1","occurred_at":"","op":"upsert","payload":"{\"sync_id\":\"obs-1\",\"session_id\":\"s1\",\"type\":\"note\",\"title\":\"Title\",\"content\":\"Body\",\"project\":\"proj-a\",\"scope\":\"project\"}","project":"proj-a","seq":0,"source":"","target_key":""}],"observations":[{"content":"kept","project":"proj-a","session_id":"s1","sync_id":"obs-direct"}],"sessions":[{"directory":"/tmp/s1","id":"s1","project":"proj-a"}]}"#;

    let canonical = canonicalize_for_project(raw, "proj-a").unwrap();
    assert_eq!(canonical, golden.as_bytes());
    assert_eq!(
        canonicalize_for_project(&canonical, "proj-a").unwrap(),
        canonical
    );
}

#[test]
fn closure_only_session_keeps_its_project() {
    let raw = br#"{
            "sessions":[
                {"id":"closure","project":"proj-b","directory":"/tmp/b"},
                {"id":"owned","project":"proj-b","directory":"/tmp/owned"}
            ],
            "mutations":[{
                "entity":"session",
                "entity_key":"owned",
                "op":"upsert",
                "payload":"{\"id\":\"owned\",\"project\":\"proj-b\",\"directory\":\"/tmp/owned\"}"
            }]
        }"#;

    let canonical = canonicalize_for_project(raw, "proj-a").unwrap();
    let value: Value = serde_json::from_slice(&canonical).unwrap();
    let sessions = value["sessions"].as_array().unwrap();
    assert_eq!(sessions[0]["project"], "proj-b");
    assert_eq!(sessions[1]["project"], "proj-a");
}

#[test]
fn relation_mutation_is_validated_and_project_scoped() {
    let raw = br#"{
            "mutations":[{
                "entity":"relation",
                "entity_key":"rel-1",
                "op":"upsert",
                "project":"wrong",
                "payload":"{\"sync_id\":\"rel-1\",\"source_id\":\"obs-a\",\"target_id\":\"obs-b\",\"relation\":\"compatible\",\"judgment_status\":\"judged\",\"marked_by_actor\":\"agent-a\",\"marked_by_kind\":\"agent\",\"project\":\"wrong\"}"
            }]
        }"#;

    let canonical = canonicalize_for_project(raw, "proj-a").unwrap();
    let value: Value = serde_json::from_slice(&canonical).unwrap();
    let mutation = &value["mutations"][0];
    let payload: Value = serde_json::from_str(mutation["payload"].as_str().unwrap()).unwrap();
    assert_eq!(mutation["project"], "proj-a");
    assert_eq!(payload["project"], "proj-a");
}

#[test]
fn mismatched_entity_key_is_rejected() {
    let raw = br#"{
            "mutations":[{
                "entity":"prompt",
                "entity_key":"wrong",
                "op":"upsert",
                "payload":"{\"sync_id\":\"prompt-1\",\"session_id\":\"s1\",\"content\":\"body\"}"
            }]
        }"#;

    let error = canonicalize_for_project(raw, "proj-a").unwrap_err();
    assert!(error.to_string().contains("does not match payload key"));
}

#[test]
fn a_mutation_missing_what_makes_it_a_memory_is_refused_at_the_wire() {
    // These are the doors a peer's payload comes through: a chunk pulled from
    // a repository, a batch pushed to the cloud. Every field checked here is
    // one without which the row is not a memory but a hole — nothing can match
    // it, revise it, or say which conversation it came from — and once written
    // it is indistinguishable from a memory somebody meant.
    //
    // Each of these refusals existed and none of them was tested: removing any
    // one left the whole suite green.
    let refused = |entity: &str, op: &str, payload: serde_json::Value| {
        let error = normalize_mutation_payload(entity, op, &payload.to_string(), "proj")
            .expect_err(&format!("{entity} {op} {payload} was accepted"));
        error.to_string()
    };

    assert!(
        refused(
            "session",
            crate::sync::OP_UPSERT,
            serde_json::json!({"id": "  ", "directory": "/tmp/p"})
        )
        .contains("id is required"),
        "a session with no id belongs to nothing"
    );
    assert!(
        refused(
            "session",
            crate::sync::OP_UPSERT,
            serde_json::json!({"id": "s1", "directory": " "})
        )
        .contains("directory is required")
    );
    for missing in ["title", "content", "scope"] {
        let mut body = serde_json::json!({
            "sync_id": "obs-1", "session_id": "s1", "type": "decision",
            "title": "a title", "content": "a body", "scope": "project",
        });
        body[missing] = serde_json::json!("");
        assert!(
            refused("observation", crate::sync::OP_UPSERT, body).contains(missing),
            "an observation with no {missing} is not a memory"
        );
    }
    assert!(
        refused(
            "prompt",
            crate::sync::OP_UPSERT,
            serde_json::json!({"sync_id": "p-1", "session_id": " ", "content": "q"})
        )
        .contains("session_id is required"),
        "a prompt with no session cannot be placed in a conversation"
    );

    // And a complete one goes through, so this refuses what is broken rather
    // than everything.
    assert!(
        normalize_mutation_payload(
            "observation",
            crate::sync::OP_UPSERT,
            &serde_json::json!({
                "sync_id": "obs-ok", "session_id": "s1", "type": "decision",
                "title": "a title", "content": "a body", "scope": "project",
            })
            .to_string(),
            "proj",
        )
        .is_ok()
    );
}

#[test]
fn a_chunk_id_is_eight_lowercase_hexadecimal_characters_and_nothing_else() {
    // The cloud takes one of these off a URL path, so this is the shape a
    // stranger's input has to match before anything is done with it.
    //
    // It had no test of its own for a while: the one it did have went with the
    // file layer it was written for, and the function outlived it.
    assert!(validate_chunk_id("2cf24dba").is_ok());
    assert!(validate_chunk_id("00000000").is_ok());
    assert!(validate_chunk_id("ffffffff").is_ok());

    for refused in [
        "",          // nothing
        "2cf24db",   // one short
        "2cf24dba1", // one long
        "2CF24DBA",  // upper case is a different string to a lookup
        "2cf24dbg",  // g is not hexadecimal
        "../../etc", // the shape that mattered when these named files
        "2cf2/dba",  // and a separator hiding in a valid-length id
        " 2cf24dba", // untrimmed
        "2cf24db\n", // a newline, which a careless client sends
    ] {
        assert!(
            validate_chunk_id(refused).is_err(),
            "{refused:?} should not pass for a chunk id"
        );
    }
}
