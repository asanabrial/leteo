use super::*;

#[test]
fn mutation_validation_enforces_real_batch_limit() {
    let entry = MutationEntry {
        project: "proj-a".to_owned(),
        entity: "session".to_owned(),
        entity_key: "session-1".to_owned(),
        op: crate::sync::OP_UPSERT.to_owned(),
        payload: Value::Object(Default::default()),
    };
    assert!(validate_mutation_batch(std::slice::from_ref(&entry)).is_ok());
    assert!(validate_mutation_batch(&[]).is_err());
    assert!(validate_mutation_batch(&vec![entry; 101]).is_err());
}

/// The migrations, applied, create the tables the cloud entities need.
///
/// This used to assert the names appeared as substrings of the migration SQL,
/// which a comment naming a table would satisfy while the table itself was
/// never created. Against a real database the question is the one that matters:
/// after `migrate`, does the catalog hold the table.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to an isolated PostgreSQL database"]
async fn migrations_create_the_tables_the_cloud_entities_need() {
    let database_url = std::env::var("TEST_DATABASE_URL").unwrap();
    let store = CloudStore::connect(&database_url, 2).await.unwrap();
    store.migrate().await.unwrap();

    let present: Vec<String> = sqlx::query(
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema = current_schema()",
    )
    .fetch_all(store.pool())
    .await
    .unwrap()
    .into_iter()
    .map(|row| row.get("table_name"))
    .collect();

    for table in [
        "cloud_principals",
        "cloud_principal_tokens",
        "cloud_project_grants",
        "cloud_chunks",
        "cloud_mutations",
        "cloud_project_controls",
        "cloud_sync_audit_log",
        "cloud_auth_audit_log",
    ] {
        assert!(
            present.iter().any(|name| name == table),
            "the migrations did not create {table}; the catalog holds {present:?}"
        );
    }
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to an isolated PostgreSQL database"]
async fn postgres_migrations_are_idempotent() {
    let database_url = std::env::var("TEST_DATABASE_URL").unwrap();
    let store = CloudStore::connect(&database_url, 2).await.unwrap();
    store.migrate().await.unwrap();
    store.health().await.unwrap();
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to an isolated PostgreSQL database"]
async fn concurrent_migrations_do_not_race_on_the_catalog() {
    let database_url = std::env::var("TEST_DATABASE_URL").unwrap();
    let migrations = (0..6).map(|_| {
        let database_url = database_url.clone();
        tokio::spawn(async move {
            let store = CloudStore::connect(&database_url, 2).await?;
            store.migrate().await?;
            store.health().await
        })
    });

    for migration in migrations {
        migration
            .await
            .expect("the migration task did not panic")
            .expect("concurrent migrations succeed");
    }
}

async fn test_store() -> (CloudStore, String) {
    let database_url = std::env::var("TEST_DATABASE_URL").unwrap();
    let store = CloudStore::connect(&database_url, 4).await.unwrap();
    store.migrate().await.unwrap();
    (store, format!("{}", Utc::now().timestamp_micros()))
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to an isolated PostgreSQL database"]
async fn a_wildcard_grant_covers_every_project_and_a_named_one_does_not() {
    let (store, stamp) = test_store().await;
    let narrow = store
        .create_principal("service_account", &format!("narrow-{stamp}"), "member")
        .await
        .unwrap();
    let broad = store
        .create_principal("service_account", &format!("broad-{stamp}"), "member")
        .await
        .unwrap();
    store
        .grant_project(narrow, &format!("only-{stamp}"))
        .await
        .unwrap();
    store.grant_project(broad, "*").await.unwrap();

    assert!(
        store
            .principal_has_project_grant(narrow, &format!("only-{stamp}"))
            .await
            .unwrap()
    );
    assert!(
        !store
            .principal_has_project_grant(narrow, &format!("other-{stamp}"))
            .await
            .unwrap(),
        "a named grant must not reach a project it does not name"
    );
    assert!(
        store
            .principal_has_project_grant(broad, &format!("anything-{stamp}"))
            .await
            .unwrap()
    );

    store
        .grant_project(narrow, &format!("only-{stamp}"))
        .await
        .unwrap();
    assert_eq!(
        store.list_principal_project_grants(narrow).await.unwrap(),
        vec![format!("only-{stamp}")]
    );

    assert!(
        store
            .revoke_project_grant(narrow, &format!("only-{stamp}"))
            .await
            .unwrap()
    );
    assert!(
        !store
            .principal_has_project_grant(narrow, &format!("only-{stamp}"))
            .await
            .unwrap()
    );
    assert!(
        !store
            .revoke_project_grant(narrow, &format!("only-{stamp}"))
            .await
            .unwrap(),
        "revoking a grant that is already gone reports no change"
    );
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to an isolated PostgreSQL database"]
async fn mutation_scoping_fails_closed_on_an_empty_allowlist() {
    let (store, stamp) = test_store().await;
    let project = format!("scoped-{stamp}");
    let other = format!("unscoped-{stamp}");
    for name in [&project, &other] {
        store
            .insert_mutations(&[MutationEntry {
                project: name.clone(),
                entity: "session".to_owned(),
                entity_key: format!("session-{name}"),
                op: crate::sync::OP_UPSERT.to_owned(),
                payload: serde_json::json!({
                    "id": format!("session-{name}"),
                    "project": name,
                    "directory": "/tmp/scoped",
                    "started_at": Utc::now().to_rfc3339(),
                }),
            }])
            .await
            .unwrap();
    }

    let (none, _, _) = store.list_mutations_since(0, 100, Some(&[])).await.unwrap();
    assert!(none.is_empty(), "an empty allowlist must return no rows");

    let (scoped, _, _) = store
        .list_mutations_since(0, 100, Some(std::slice::from_ref(&project)))
        .await
        .unwrap();
    assert!(!scoped.is_empty(), "the scoped project has rows to return");
    assert!(
        scoped.iter().all(|mutation| mutation.project == project),
        "a scoped pull leaked another project"
    );

    let (all, _, _) = store.list_mutations_since(0, 1_000, None).await.unwrap();
    let projects = all
        .iter()
        .map(|mutation| mutation.project.as_str())
        .collect::<BTreeSet<_>>();
    assert!(projects.contains(project.as_str()) && projects.contains(other.as_str()));
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to an isolated PostgreSQL database"]
async fn a_replayed_chunk_is_accepted_but_a_forged_one_is_not() {
    let (store, stamp) = test_store().await;
    let project = format!("chunk-{stamp}");
    let chunk = ChunkData {
        sessions: vec![crate::memory::model::Session {
            id: format!("session-{stamp}"),
            project: project.clone(),
            directory: "/tmp/chunk".to_owned(),
            started_at: Utc::now().to_rfc3339(),
            ended_at: None,
            summary: None,
        }],
        ..ChunkData::default()
    };
    let payload = encode_chunk(&chunk).unwrap();
    let id = chunk_id(&payload);

    assert_eq!(
        store
            .write_chunk(&project, &id, "test", None, &payload)
            .await
            .unwrap(),
        id
    );
    assert_eq!(
        store
            .write_chunk(&project, &id, "test", None, &payload)
            .await
            .unwrap(),
        id
    );

    let mismatch = store
        .write_chunk(&project, "deadbeef", "test", None, &payload)
        .await
        .unwrap_err();
    assert!(
        matches!(mismatch, CloudStoreError::Invalid(ref message) if message.contains("mismatch")),
        "unexpected error: {mismatch}"
    );

    assert!(
        store
            .known_session_ids(&project)
            .await
            .unwrap()
            .contains(&format!("session-{stamp}"))
    );
    let manifest = store.read_manifest(&project).await.unwrap();
    assert!(manifest.chunks.iter().any(|entry| entry.id == id));
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to an isolated PostgreSQL database"]
async fn pausing_a_project_is_recorded_and_reversible() {
    let (store, stamp) = test_store().await;
    let project = format!("paused-{stamp}");

    assert!(store.is_project_sync_enabled(&project).await.unwrap());

    store
        .set_project_sync_enabled(&project, false, "operator", Some("incident"))
        .await
        .unwrap();
    assert!(!store.is_project_sync_enabled(&project).await.unwrap());

    store
        .set_project_sync_enabled(&project, true, "operator", None)
        .await
        .unwrap();
    assert!(store.is_project_sync_enabled(&project).await.unwrap());

    store
        .record_audit(AuditEntry {
            contributor: "operator",
            project: &project,
            action: "mutation_push",
            outcome: "rejected_project_paused",
            entry_count: 3,
            reason_code: Some("sync-paused"),
        })
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to an isolated PostgreSQL database"]
async fn an_empty_project_name_is_refused_everywhere_it_is_accepted() {
    let (store, stamp) = test_store().await;
    assert!(store.read_manifest("   ").await.is_err());
    assert!(store.known_session_ids("").await.is_err());
    assert!(store.is_project_sync_enabled("  ").await.is_err());
    let principal = store
        .create_principal("service_account", &format!("blank-{stamp}"), "member")
        .await
        .unwrap();
    assert!(store.grant_project(principal, "   ").await.is_err());
    assert!(
        store
            .principal_has_project_grant(principal, "")
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to an isolated PostgreSQL database"]
async fn a_principal_is_found_by_name_and_its_token_resolves_once_stored() {
    let (store, stamp) = test_store().await;
    let name = format!("named-{stamp}");
    let id = store
        .create_principal("human", &name, "admin")
        .await
        .unwrap();
    assert_eq!(store.find_principal_by_name(&name).await.unwrap(), Some(id));
    assert_eq!(
        store
            .find_principal_by_name(&format!("absent-{stamp}"))
            .await
            .unwrap(),
        None
    );
    assert!(store.find_principal_by_name("   ").await.is_err());

    let hasher =
        crate::cloud::ManagedTokenHasher::new("a-token-pepper-of-at-least-32-bytes-long").unwrap();
    let token = crate::cloud::ManagedToken::generate("test");
    let verifier = hasher.hash(&token.raw).unwrap();
    let token_id = store
        .store_managed_token(id, &token, &verifier, "test")
        .await
        .unwrap();

    let found = store
        .find_managed_token(&verifier)
        .await
        .unwrap()
        .expect("the stored token resolves by its hash");
    assert_eq!(found.principal_id, id);
    assert_eq!(found.token_id, token_id);
    assert!(
        store
            .find_managed_token("0".repeat(64).as_str())
            .await
            .unwrap()
            .is_none()
    );
    store.touch_managed_token(token_id).await.unwrap();
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to an isolated PostgreSQL database"]
async fn postgres_chunk_and_mutation_round_trip() {
    let database_url = std::env::var("TEST_DATABASE_URL").unwrap();
    let store = CloudStore::connect(&database_url, 2).await.unwrap();
    let project = format!("pg-roundtrip-{}", Utc::now().timestamp_micros());
    let chunk = ChunkData {
        sessions: vec![crate::memory::model::Session {
            id: "pg-session".to_owned(),
            project: project.clone(),
            directory: "/tmp/pg-session".to_owned(),
            started_at: Utc::now().to_rfc3339(),
            ended_at: None,
            summary: None,
        }],
        ..ChunkData::default()
    };
    let payload = encode_chunk(&chunk).unwrap();
    let id = chunk_id(&payload);
    assert_eq!(
        store
            .write_chunk(&project, &id, "test", None, &payload)
            .await
            .unwrap(),
        id
    );
    assert_eq!(store.read_manifest(&project).await.unwrap().chunks.len(), 1);
    assert_eq!(
        decode_chunk(&store.read_chunk(&project, &id).await.unwrap()).unwrap(),
        chunk
    );
    let entries = [MutationEntry {
        project: project.clone(),
        entity: "session".to_owned(),
        entity_key: "pg-session-2".to_owned(),
        op: crate::sync::OP_UPSERT.to_owned(),
        payload: serde_json::json!({
            "id": "pg-session-2",
            "project": project.clone(),
            "directory": "/tmp/pg-session-2"
        }),
    }];
    assert_eq!(store.insert_mutations(&entries).await.unwrap().len(), 1);
    let (pulled, _, _) = store
        .list_mutations_since(0, 100, Some(std::slice::from_ref(&project)))
        .await
        .unwrap();
    assert!(
        pulled
            .iter()
            .any(|mutation| mutation.entity_key == "pg-session-2")
    );

    sqlx::query("DELETE FROM cloud_mutations WHERE project = $1")
        .bind(&project)
        .execute(store.pool())
        .await
        .unwrap();
    sqlx::query("DELETE FROM cloud_project_sessions WHERE project_name = $1")
        .bind(&project)
        .execute(store.pool())
        .await
        .unwrap();
    sqlx::query("DELETE FROM cloud_chunks WHERE project_name = $1")
        .bind(&project)
        .execute(store.pool())
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to an isolated PostgreSQL database"]
async fn dashboard_session_rechecks_role_and_token_revocation() {
    let database_url = std::env::var("TEST_DATABASE_URL").unwrap();
    let store = CloudStore::connect(&database_url, 2).await.unwrap();
    store.migrate().await.unwrap();
    let suffix = Utc::now().timestamp_micros();
    let principal_id = store
        .create_principal("human", &format!("dashboard-{suffix}"), "admin")
        .await
        .unwrap();
    let token = ManagedToken::generate("test");
    let token_id = store
        .store_managed_token(
            principal_id,
            &token,
            &format!("test-hash-{suffix}"),
            "dashboard test",
        )
        .await
        .unwrap();

    assert!(
        store
            .dashboard_session_valid(principal_id, token_id)
            .await
            .unwrap()
    );
    sqlx::query("UPDATE cloud_principals SET role = 'member' WHERE id = $1")
        .bind(principal_id)
        .execute(store.pool())
        .await
        .unwrap();
    assert!(
        !store
            .dashboard_session_valid(principal_id, token_id)
            .await
            .unwrap()
    );
    sqlx::query("UPDATE cloud_principals SET role = 'admin' WHERE id = $1")
        .bind(principal_id)
        .execute(store.pool())
        .await
        .unwrap();
    sqlx::query("UPDATE cloud_principal_tokens SET revoked_at = NOW() WHERE id = $1")
        .bind(token_id)
        .execute(store.pool())
        .await
        .unwrap();
    assert!(
        !store
            .dashboard_session_valid(principal_id, token_id)
            .await
            .unwrap()
    );

    sqlx::query("DELETE FROM cloud_principals WHERE id = $1")
        .bind(principal_id)
        .execute(store.pool())
        .await
        .unwrap();
}

/// Every query against a tenant's rows narrows to that tenant.
///
/// The behavioural proof of this lives behind `#[ignore]` and a PostgreSQL
/// service. Such tests live across the crate, most of them in this file, and
/// none runs on a developer's machine; `cargo test -- --ignored` is what runs
/// them and what names them. What they cover is not summarised here, and
/// no one of them is singled out as the proof of this property: every summary
/// this comment has carried turned out narrower than the set it described.
///
/// So the structural half is checked here, the way the router's is — read out
/// of the source, so a query added tomorrow either narrows or is written down
/// as deliberately global.
///
/// What makes this worth having: `list_mutations_since` takes
/// `Option<&[String]>`, and `None` means *every project*. That is correct — it
/// is how a `*` grant is served — but it is also what a caller produces by
/// accident from an `unwrap_or_default` or an `.ok().flatten()`, and the
/// resulting query returns another tenant's rows while looking like every
/// other call.
#[test]
fn every_query_over_a_tenants_rows_narrows_to_that_tenant() {
    const SOURCE: &str = include_str!("mod.rs");

    const TENANT_TABLES: &[&str] = &[
        "cloud_chunks",
        "cloud_mutations",
        "cloud_project_sessions",
        "cloud_project_controls",
        "cloud_project_grants",
    ];

    const GLOBAL: &[(&str, &str)] = &[
        (
            "FROM cloud_mutations WHERE seq > $1 ORDER BY seq LIMIT $2",
            "the wildcard branch: reached only when `enrolled_projects` returned \
             None, which it does only for a principal holding the `*` grant",
        ),
        (
            "SELECT DISTINCT project FROM cloud_mutations",
            "resolving a `*` grant to concrete names: reached only for a principal \
             holding it, and it returns project names with no row beneath them",
        ),
        (
            "(SELECT COUNT(*) FROM cloud_chunks) AS chunks",
            "an administrator's totals, which are counts and carry no rows",
        ),
        (
            "(SELECT COUNT(*) FROM cloud_mutations) AS mutations",
            "the same totals",
        ),
        (
            "(SELECT COUNT(*) FROM cloud_project_controls WHERE NOT sync_enabled) AS paused_projects",
            "the same totals",
        ),
    ];

    let mut unnarrowed = Vec::new();
    for (index, _) in SOURCE.match_indices("FROM ") {
        let tail = &SOURCE[index..];
        let statement_end = tail.find('"').unwrap_or(tail.len());
        let statement = &tail[..statement_end];
        let Some(table) = TENANT_TABLES
            .iter()
            .find(|table| statement.starts_with(&format!("FROM {table}")))
        else {
            continue;
        };
        let open = SOURCE[..index].rfind('"').map_or(0, |at| at + 1);
        let close = SOURCE[index..]
            .find('"')
            .map_or(SOURCE.len(), |at| index + at);
        let query = &SOURCE[open..close];
        if GLOBAL
            .iter()
            .any(|(fragment, why)| !why.is_empty() && query.contains(fragment))
        {
            continue;
        }
        let narrowed = query.contains("project_name = ")
            || query.contains("project = ")
            || query.contains("principal_id = ")
            || query.contains("project = ANY(");
        if !narrowed {
            unnarrowed.push(format!("{table}: {}", query.replace('\n', " ")));
        }
    }
    assert!(
        unnarrowed.is_empty(),
        "these read a tenant's rows without narrowing to one, and are not \
         listed as deliberately global:\n{}",
        unnarrowed.join("\n")
    );
}

#[test]
fn a_search_term_matches_literally() {
    assert_eq!(like_pattern("plain"), "%plain%");
    assert_eq!(like_pattern("100%_done\\"), "%100\\%\\_done\\\\%");
}

fn entry(project: &str, entity: &str, key: &str, op: &str, payload: Value) -> MutationEntry {
    MutationEntry {
        project: project.to_owned(),
        entity: entity.to_owned(),
        entity_key: key.to_owned(),
        op: op.to_owned(),
        payload,
    }
}

fn observation(project: &str, key: &str, title: &str, content: &str) -> MutationEntry {
    entry(
        project,
        crate::sync::ENTITY_OBSERVATION,
        key,
        crate::sync::OP_UPSERT,
        serde_json::json!({
            "sync_id": key,
            "session_id": "session-1",
            "type": "note",
            "scope": "project",
            "title": title,
            "content": content,
        }),
    )
}

fn soft_deleted(mut entry: MutationEntry) -> MutationEntry {
    entry.payload["deleted_at"] = serde_json::json!("2026-01-02T00:00:00Z");
    entry
}

/// What the dashboard reads is the current state of the log, one project at a time.
///
/// Four things could each go wrong without a failing status: a superseded
/// version still showing, a deleted memory showing, a search matching an older
/// text than the memory has now, and a project reading another's rows.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to an isolated PostgreSQL database"]
async fn browsing_reads_the_current_state_of_one_project_at_a_time() {
    let database_url = std::env::var("TEST_DATABASE_URL").unwrap();
    let store = CloudStore::connect(&database_url, 2).await.unwrap();
    store.migrate().await.unwrap();
    let stamp = Utc::now().timestamp_micros();
    let (a, b) = (format!("browse-a-{stamp}"), format!("browse-b-{stamp}"));

    store
        .insert_mutations(&[
            entry(
                &a,
                crate::sync::ENTITY_SESSION,
                "session-1",
                crate::sync::OP_UPSERT,
                serde_json::json!({
                    "id": "session-1",
                    "directory": "/work/a",
                    "started_at": "2026-01-01T00:00:00Z",
                    "summary": "first session",
                }),
            ),
            observation(&a, "kept", "old title", "the old words"),
            observation(&a, "kept", "new title", "100%_literal words"),
            observation(&a, "gone", "deleted title", "deleted words"),
            entry(
                &a,
                crate::sync::ENTITY_OBSERVATION,
                "gone",
                crate::sync::OP_DELETE,
                serde_json::json!({"sync_id": "gone"}),
            ),
            soft_deleted(observation(&a, "soft", "soft deleted title", "x")),
            observation(&b, "secret", "b only title", "b only words"),
        ])
        .await
        .unwrap();

    let own = [a.clone()];
    let memories = store.list_memories(&own, None, 10).await.unwrap();
    let titles = memories
        .items
        .iter()
        .map(|memory| memory.title.as_str())
        .collect::<Vec<_>>();
    assert_eq!(titles, ["new title"], "only the current, undeleted memory");
    assert!(!memories.truncated);

    let sessions = store.list_sessions(&a).await.unwrap();
    assert_eq!(sessions.items.len(), 1);
    assert_eq!(sessions.items[0].directory, "/work/a");
    assert_eq!(sessions.items[0].summary.as_deref(), Some("first session"));
    assert!(store.list_sessions(&b).await.unwrap().items.is_empty());

    let found = |term: &'static str, projects: Vec<String>| {
        let store = store.clone();
        async move {
            store
                .list_memories(&projects, Some(term), 10)
                .await
                .unwrap()
                .items
        }
    };
    assert_eq!(found("NEW title", own.to_vec()).await.len(), 1);
    assert!(found("old words", own.to_vec()).await.is_empty());
    assert!(found("deleted words", own.to_vec()).await.is_empty());
    assert!(found("soft deleted", own.to_vec()).await.is_empty());
    assert!(found("b only", own.to_vec()).await.is_empty());
    assert!(found("%", vec![b.clone()]).await.is_empty());
    assert_eq!(found("100%_literal", own.to_vec()).await.len(), 1);
    assert_eq!(found("b only", vec![a.clone(), b.clone()]).await.len(), 1);

    assert!(
        store
            .list_memories(&[], None, 10)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert!(store.get_memory(&a, "secret").await.unwrap().is_none());
    assert!(store.get_memory(&a, "gone").await.unwrap().is_none());
    let read = store.get_memory(&b, "secret").await.unwrap().unwrap();
    assert_eq!(read.content, "b only words");

    let projects = store.list_projects(&own).await.unwrap();
    assert_eq!(
        projects
            .items
            .iter()
            .map(|project| project.name.as_str())
            .collect::<Vec<_>>(),
        [a.as_str()]
    );
    assert!(store.list_projects(&[]).await.unwrap().items.is_empty());
    let every = store.all_project_names().await.unwrap();
    assert!(every.contains(&a) && every.contains(&b));
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to an isolated PostgreSQL database"]
async fn a_list_cut_at_its_limit_says_so() {
    let database_url = std::env::var("TEST_DATABASE_URL").unwrap();
    let store = CloudStore::connect(&database_url, 2).await.unwrap();
    store.migrate().await.unwrap();
    let project = format!("browse-limit-{}", Utc::now().timestamp_micros());
    store
        .insert_mutations(&[
            observation(&project, "one", "one", "x"),
            observation(&project, "two", "two", "x"),
        ])
        .await
        .unwrap();

    let own = [project];
    let cut = store.list_memories(&own, None, 1).await.unwrap();
    assert_eq!((cut.items.len(), cut.truncated), (1, true));
    let whole = store.list_memories(&own, None, 2).await.unwrap();
    assert_eq!((whole.items.len(), whole.truncated), (2, false));
}
