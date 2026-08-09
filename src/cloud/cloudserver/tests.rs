use axum::body::to_bytes;
use serde_json::json;

use super::*;

/// A mutation the way a client actually sends one: the key and the identifier
/// inside the payload are the same value, because both are built from it.
fn mutation(entity: &str, operation: &str, payload: Value) -> MutationEntry {
    let field = if entity == crate::sync::ENTITY_SESSION {
        "id"
    } else {
        "sync_id"
    };
    let key = payload
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or("entity-1")
        .to_owned();
    let mut payload = payload;
    if let Some(object) = payload.as_object_mut() {
        object
            .entry(field)
            .or_insert_with(|| Value::String(key.clone()));
    }
    MutationEntry {
        project: "proj-a".to_owned(),
        entity: entity.to_owned(),
        entity_key: key,
        op: operation.to_owned(),
        payload,
    }
}

#[test]
fn validation_caps_mutations_at_one_hundred() {
    let entry = mutation("session", crate::sync::OP_UPSERT, json!({}));
    assert!(validate_mutation_entries(&vec![entry.clone(); 100]).is_ok());
    assert!(validate_mutation_entries(&vec![entry; 101]).is_err());
    assert!(validate_mutation_entries(&[]).is_err());
}

#[test]
fn relation_validation_requires_authorship_fields() {
    let valid = mutation(
        "relation",
        crate::sync::OP_UPSERT,
        json!({
            "sync_id": "rel-1",
            "source_id": "obs-1",
            "target_id": "obs-2",
            "relation": "related",
            "judgment_status": "judged",
            "marked_by_actor": "agent",
            "marked_by_kind": "system"
        }),
    );
    assert!(validate_mutation_entries(std::slice::from_ref(&valid)).is_ok());
    let mut invalid = valid;
    invalid.payload.as_object_mut().unwrap().remove("source_id");
    assert!(validate_mutation_entries(&[invalid]).is_err());
}

#[test]
fn unsupported_entities_and_relation_deletes_are_rejected() {
    assert!(
        validate_mutation_entries(&[mutation("unknown", crate::sync::OP_UPSERT, json!({}))])
            .is_err()
    );
    assert!(
        validate_mutation_entries(&[mutation("relation", crate::sync::OP_DELETE, json!({}))])
            .is_err()
    );
}

/// Drives the assembled router the way a client does, so wiring defects are
/// caught and not just the helpers in isolation.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to an isolated PostgreSQL database"]
async fn a_tenant_can_never_reach_another_tenants_project() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    let database_url = std::env::var("TEST_DATABASE_URL").unwrap();
    let stamp = Utc::now().timestamp_micros();
    let (project_a, project_b) = (format!("tenant-a-{stamp}"), format!("tenant-b-{stamp}"));

    let config = CloudConfig {
        database_url: database_url.clone(),
        dashboard_secret: "a-dashboard-secret-of-at-least-32-bytes".to_owned(),
        token_pepper: "a-token-pepper-of-at-least-32-bytes-long".to_owned(),
        ..CloudConfig::default()
    };
    let server = CloudServer::from_config(config.clone()).await.unwrap();
    server.state.store.migrate().await.unwrap();

    // Two principals, each granted exactly one project.
    let hasher = crate::cloud::ManagedTokenHasher::new(&config.token_pepper).unwrap();
    let mint = async |name: &str, project: &str| {
        let id = server
            .state
            .store
            .create_principal("human", name, "admin")
            .await
            .unwrap();
        let token = crate::cloud::ManagedToken::generate("test");
        let verifier = hasher.hash(&token.raw).unwrap();
        server
            .state
            .store
            .store_managed_token(id, &token, &verifier, "test")
            .await
            .unwrap();
        server.state.store.grant_project(id, project).await.unwrap();
        token.raw
    };
    let token_a = mint(&format!("principal-a-{stamp}"), &project_a).await;
    let token_b = mint(&format!("principal-b-{stamp}"), &project_b).await;

    let push = |token: String, project: String, key: String| {
        let router = server.router();
        async move {
            let body = json!({
                "created_by": "test",
                "entries": [{
                    "project": project,
                    "entity": "observation",
                    "entity_key": key,
                    "op": crate::sync::OP_UPSERT,
                    "payload": {
                        "sync_id": key,
                        "session_id": format!("session-{key}"),
                        "project": project,
                        "type": "note",
                        "title": "probe",
                        "content": "tenancy probe",
                        "scope": "project",
                    },
                }],
            });
            let request = Request::builder()
                .method("POST")
                .uri("/sync/mutations/push")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap();
            router.oneshot(request).await.unwrap().status()
        }
    };

    // Each tenant may write its own project.
    assert_eq!(
        push(token_a.clone(), project_a.clone(), format!("a-{stamp}")).await,
        StatusCode::OK
    );
    assert_eq!(
        push(token_b.clone(), project_b.clone(), format!("b-{stamp}")).await,
        StatusCode::OK
    );
    // And no other, including by claiming the wildcard grant.
    assert_eq!(
        push(token_a.clone(), project_b.clone(), format!("x-{stamp}")).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        push(token_a.clone(), "*".to_owned(), format!("w-{stamp}")).await,
        StatusCode::FORBIDDEN
    );

    // A pull is scoped to the grant even though B's rows have higher
    // sequence numbers and would otherwise be the freshest page.
    let request = Request::builder()
        .uri("/sync/mutations/pull?since_seq=0&limit=100")
        .header(header::AUTHORIZATION, format!("Bearer {token_a}"))
        .body(Body::empty())
        .unwrap();
    let response = server.router().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let projects = body["mutations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|mutation| mutation["project"].as_str().unwrap().to_owned())
        .collect::<BTreeSet<_>>();
    // Asserting only the absence of B would also pass on an empty page, so
    // the presence of A is what proves the filter ran rather than the query
    // simply returning nothing.
    assert!(
        projects.contains(&project_a),
        "the pull returned none of the tenant's own rows: {projects:?}"
    );
    assert_eq!(
        projects.len(),
        1,
        "a pull leaked another tenant's projects: {projects:?}"
    );

    // Reading a manifest obeys the same boundary, and an unauthenticated
    // or forged caller gets nothing at all.
    let get = |token: Option<String>, project: &str| {
        let router = server.router();
        let mut request = Request::builder().uri(format!("/sync/pull?project={project}"));
        if let Some(token) = token {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let request = request.body(Body::empty()).unwrap();
        async move { router.oneshot(request).await.unwrap().status() }
    };
    assert_eq!(
        get(Some(token_a.clone()), &project_b).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(get(None, &project_a).await, StatusCode::UNAUTHORIZED);
    assert_eq!(
        get(Some("not-a-real-token".to_owned()), &project_a).await,
        StatusCode::UNAUTHORIZED
    );
}

#[test]
fn dashboard_cookie_is_http_only_and_conditionally_secure() {
    let cookie = dashboard_cookie("signed", false);
    assert!(cookie.contains("HttpOnly"));
    assert!(!cookie.contains("; Secure"));
    assert!(dashboard_cookie("signed", true).contains("; Secure"));
}

#[test]
fn the_session_cookie_is_secure_unless_the_request_is_local() {
    let headers = |pairs: &[(&str, &str)]| {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(
                axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        headers
    };

    // A proxy that forgets X-Forwarded-Proto must not downgrade the cookie.
    assert!(requires_secure_cookie(&headers(&[(
        "host",
        "memory.example.com"
    )])));
    assert!(requires_secure_cookie(&headers(&[(
        "host",
        "memory.example.com:8443"
    )])));
    assert!(requires_secure_cookie(&HeaderMap::new()));
    assert!(requires_secure_cookie(&headers(&[
        ("host", "127.0.0.1:8080"),
        ("x-forwarded-proto", "https"),
    ])));

    // Local development over plain HTTP still stores its cookie.
    for host in [
        "localhost",
        "localhost:8080",
        "127.0.0.1",
        "127.0.0.1:8080",
        "127.5.4.3",
        "[::1]:8080",
    ] {
        assert!(
            !requires_secure_cookie(&headers(&[("host", host)])),
            "{host} is local"
        );
    }
    // A host that merely looks local is not.
    assert!(requires_secure_cookie(&headers(&[(
        "host",
        "localhost.evil.example"
    )])));
}

#[test]
fn dashboard_html_escapes_principal_names() {
    assert_eq!(escape_html("<admin & owner>"), "&lt;admin &amp; owner&gt;");
}

#[tokio::test]
async fn database_errors_are_logged_but_redacted_from_responses() {
    let response = ApiError::from(CloudStoreError::Database(sqlx::Error::Protocol(
        "database-secret".to_owned(),
    )))
    .into_response();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = std::str::from_utf8(&body).unwrap();

    assert!(!body.contains("database-secret"));
    assert!(body.contains("internal server error"));
}

#[test]
fn a_mutation_filed_under_one_key_may_not_be_about_another() {
    // The server orders and deduplicates by `entity_key`; every peer applies
    // by the identifier in the payload. One where they disagree is stored and
    // served as being about one memory and applied to another — the same
    // change arriving twice, or landing on a memory the server believes
    // untouched. Nothing checked it, and 9527 mutations in a real store all
    // agree, so nothing legitimate is turned away.
    let mut crossed = mutation(
        crate::sync::ENTITY_OBSERVATION,
        crate::sync::OP_UPSERT,
        json!({ "sync_id": "obs-a" }),
    );
    crossed.entity_key = "obs-b".to_owned();

    let error = validate_mutation_entries(&[crossed]).unwrap_err();

    assert!(format!("{error:?}").contains("must be the entity_key"));
}

#[test]
fn a_session_is_keyed_on_its_id_and_everything_else_on_its_sync_id() {
    // Sessions are the one entity whose payload names itself `id`. Reading the
    // wrong field would reject every session mutation ever sent.
    assert!(
        validate_mutation_entries(&[mutation(
            crate::sync::ENTITY_SESSION,
            crate::sync::OP_UPSERT,
            json!({ "id": "session-1", "project": "proj-a" }),
        )])
        .is_ok()
    );
    assert!(
        validate_mutation_entries(&[mutation(
            crate::sync::ENTITY_PROMPT,
            crate::sync::OP_DELETE,
            json!({ "sync_id": "prompt-1", "deleted": true }),
        )])
        .is_ok()
    );
}

#[test]
fn a_payload_that_never_names_itself_is_refused() {
    let mut anonymous = mutation(
        crate::sync::ENTITY_OBSERVATION,
        crate::sync::OP_UPSERT,
        json!({ "sync_id": "obs-a" }),
    );
    anonymous.payload.as_object_mut().unwrap().remove("sync_id");

    assert!(validate_mutation_entries(&[anonymous]).is_err());
}

/// Every route is guarded, or is named here as deliberately open.
///
/// `guard.rs` says it in its own header: there is no middleware on this router,
/// every handler calls the checks itself, and "that is a shape where one
/// forgetful handler is an open door". It was right and nothing was watching —
/// each of the six sync routes does authenticate today, and a seventh added
/// tomorrow would not have to.
///
/// Read out of the source rather than from a list kept beside it, because a
/// list is the thing that goes stale. A new `.route(...)` whose handler
/// neither authenticates nor appears below fails here, and the fix is to do
/// one or the other on purpose.
#[test]
fn every_route_authenticates_or_says_why_it_does_not() {
    const SOURCE: &str = include_str!("mod.rs");

    /// Open by design, each for a reason that has to survive being written
    /// down: a health probe has no credentials, and a login page is how you
    /// come by them.
    const PUBLIC: &[(&str, &str)] = &[
        (
            "health",
            "a liveness probe answers before anyone has a token",
        ),
        ("login_page", "the form you sign in with"),
        (
            "login",
            "the sign-in itself, which checks the password instead",
        ),
    ];

    // `.route("/path", get(handler))`, `.post(handler)` and chains of both.
    let mut routed: Vec<(String, String)> = Vec::new();
    for line in SOURCE.lines() {
        let Some(rest) = line.trim().strip_prefix(".route(") else {
            continue;
        };
        let Some((path, rest)) = rest.split_once(',') else {
            continue;
        };
        let path = path.trim().trim_matches('"').to_owned();
        for verb in ["get(", "post(", "put(", "delete(", "patch("] {
            let mut cursor = rest;
            while let Some(at) = cursor.find(verb) {
                cursor = &cursor[at + verb.len()..];
                let handler: String = cursor
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !handler.is_empty() {
                    routed.push((path.clone(), handler));
                }
            }
        }
    }
    assert!(
        routed.len() >= 8,
        "the route table did not parse; this guard would pass on nothing: {routed:?}"
    );

    // A handler's body, from its signature to the closing brace in column one.
    let body_of = |name: &str| -> String {
        let start = SOURCE
            .find(&format!("\nasync fn {name}("))
            .unwrap_or_else(|| panic!("{name} is routed but not defined in this file"));
        let rest = &SOURCE[start + 1..];
        let end = rest.find("\n}\n").map_or(rest.len(), |at| at + 2);
        rest[..end].to_owned()
    };

    let mut unguarded = Vec::new();
    for (path, handler) in &routed {
        if let Some((_, why)) = PUBLIC.iter().find(|(name, _)| name == handler) {
            assert!(!why.is_empty());
            continue;
        }
        let body = body_of(handler);
        // Either a bearer token, or the signed dashboard session. Both end in
        // a `Principal`; what matters is that neither is skipped.
        let guarded = body.contains("authenticate(") || body.contains("parse_dashboard_session(");
        if !guarded {
            unguarded.push(format!("{path} -> {handler}"));
        }
    }
    assert!(
        unguarded.is_empty(),
        "these routes check nobody, and are not listed as deliberately open: {unguarded:?}"
    );
}
