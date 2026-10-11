use axum::body::to_bytes;
use serde_json::json;

use super::*;
use crate::cloud::cloudstore::{BrowseMemory, BrowseProject, BrowseSession, Limited};

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

async fn mint_admin(server: &CloudServer, pepper: &str, name: &str, project: &str) -> String {
    let hasher = crate::cloud::ManagedTokenHasher::new(pepper).unwrap();
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
}

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

    let mint = async |name: &str, project: &str| {
        mint_admin(&server, &config.token_pepper, name, project).await
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

    assert_eq!(
        push(token_a.clone(), project_a.clone(), format!("a-{stamp}")).await,
        StatusCode::OK
    );
    assert_eq!(
        push(token_b.clone(), project_b.clone(), format!("b-{stamp}")).await,
        StatusCode::OK
    );
    assert_eq!(
        push(token_a.clone(), project_b.clone(), format!("x-{stamp}")).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        push(token_a.clone(), "*".to_owned(), format!("w-{stamp}")).await,
        StatusCode::FORBIDDEN
    );

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
    assert!(
        projects.contains(&project_a),
        "the pull returned none of the tenant's own rows: {projects:?}"
    );
    assert_eq!(
        projects.len(),
        1,
        "a pull leaked another tenant's projects: {projects:?}"
    );

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
#[test]
fn every_route_authenticates_or_says_why_it_does_not() {
    const SOURCE: &str = include_str!("mod.rs");

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

fn memory(project: &str, key: &str, title: &str, content: &str) -> BrowseMemory {
    BrowseMemory {
        project: project.to_owned(),
        key: key.to_owned(),
        kind: "note".to_owned(),
        title: title.to_owned(),
        content: content.to_owned(),
        topic_key: None,
        session_id: "session-1".to_owned(),
        updated_at: "2026-01-01T00:00:00Z".to_owned(),
    }
}

#[test]
fn query_values_are_encoded_so_they_cannot_add_parameters() {
    use browser::url_encode;
    assert_eq!(url_encode("plain-name_1.~"), "plain-name_1.~");
    assert_eq!(url_encode("a&key=b c/d"), "a%26key%3Db%20c%2Fd");
    assert_eq!(url_encode("caf\u{e9}"), "caf%C3%A9");
}

#[test]
fn every_stored_value_is_escaped_where_the_pages_print_it() {
    let hostile = "<script>alert(1)</script>\"'&";
    let page_for = |html: String| {
        assert!(
            !html.contains("<script>"),
            "an unescaped value reached a page: {html}"
        );
        assert!(
            html.contains("&lt;script&gt;"),
            "the value vanished instead of being escaped"
        );
    };
    fn limited<T>(items: Vec<T>) -> Limited<T> {
        Limited {
            items,
            truncated: false,
        }
    }

    page_for(browser::render_projects(
        hostile,
        &limited(vec![BrowseProject {
            name: hostile.to_owned(),
            last_activity: hostile.to_owned(),
        }]),
    ));
    page_for(browser::render_project(
        "admin",
        hostile,
        &limited(vec![BrowseSession {
            id: hostile.to_owned(),
            directory: hostile.to_owned(),
            started_at: hostile.to_owned(),
            ended_at: Some(hostile.to_owned()),
            summary: Some(hostile.to_owned()),
        }]),
        &limited(vec![memory(hostile, hostile, hostile, hostile)]),
    ));
    page_for(browser::render_memory(
        "admin",
        &memory(hostile, hostile, hostile, hostile),
    ));
    page_for(browser::render_search(
        "admin",
        hostile,
        Some(hostile),
        Some(&limited(vec![memory(hostile, hostile, hostile, hostile)])),
    ));
    page_for(browser::render_search(
        "admin",
        hostile,
        None,
        Some(&limited(vec![])),
    ));
}

#[test]
fn a_link_to_a_memory_carries_its_names_encoded() {
    let html = browser::render_project(
        "admin",
        "p",
        &Limited {
            items: vec![],
            truncated: false,
        },
        &Limited {
            items: vec![memory("a&b", "k=1&x", "t", "c")],
            truncated: false,
        },
    );
    assert!(html.contains("project=a%26b&amp;key=k%3D1%26x"), "{html}");
}

#[test]
fn a_cut_list_and_a_long_memory_say_so() {
    use crate::cloud::cloudstore::MEMORY_SEARCH_LIMIT;
    let long = "x".repeat(1000);
    let html = browser::render_search(
        "admin",
        "x",
        None,
        Some(&Limited {
            items: vec![memory("p", "k", "t", &long)],
            truncated: true,
        }),
    );
    assert!(
        html.contains(&format!("first {MEMORY_SEARCH_LIMIT} matches")),
        "{html}"
    );
    assert!(!html.contains(&long), "a list printed a whole long memory");
    assert!(html.contains("..."));
}

/// A dashboard user sees the projects they hold a grant on and no others.
///
/// An administrator is not exempt: the role lets them sign in, and the grant
/// decides what they may read, the same as for the sync routes. Each page is
/// asked for the other tenant's project directly, by name, by memory key and by
/// search term, because a list that hides a project says nothing about a page
/// that serves it.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to an isolated PostgreSQL database"]
async fn a_dashboard_user_browses_only_the_projects_they_hold_a_grant_on() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    let database_url = std::env::var("TEST_DATABASE_URL").unwrap();
    let stamp = Utc::now().timestamp_micros();
    let (project_a, project_b) = (format!("browse-a-{stamp}"), format!("browse-b-{stamp}"));
    let (word_a, word_b) = (format!("alphaword{stamp}"), format!("betaword{stamp}"));

    let config = CloudConfig {
        database_url,
        dashboard_secret: "a-dashboard-secret-of-at-least-32-bytes".to_owned(),
        token_pepper: "a-token-pepper-of-at-least-32-bytes-long".to_owned(),
        ..CloudConfig::default()
    };
    let server = CloudServer::from_config(config.clone()).await.unwrap();
    server.state.store.migrate().await.unwrap();

    let observation = |project: &str, key: &str, title: &str, content: &str| MutationEntry {
        project: project.to_owned(),
        entity: "observation".to_owned(),
        entity_key: key.to_owned(),
        op: crate::sync::OP_UPSERT.to_owned(),
        payload: json!({
            "sync_id": key,
            "session_id": "session-1",
            "type": "note",
            "scope": "project",
            "title": title,
            "content": content,
        }),
    };
    let session = |project: &str, directory: &str| MutationEntry {
        project: project.to_owned(),
        entity: "session".to_owned(),
        entity_key: "session-1".to_owned(),
        op: crate::sync::OP_UPSERT.to_owned(),
        payload: json!({"id": "session-1", "directory": directory, "started_at": "2026-01-01T00:00:00Z"}),
    };
    server
        .state
        .store
        .insert_mutations(&[
            session(&project_a, &format!("/work/{word_a}")),
            observation(&project_a, "a-key", &format!("<b>{word_a}</b>"), &word_a),
            session(&project_b, &format!("/work/{word_b}")),
            observation(&project_b, "b-key", &word_b, &word_b),
        ])
        .await
        .unwrap();

    let sign_in = async |token: String| {
        let request = Request::builder()
            .method("POST")
            .uri("/dashboard/login")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(format!("token={token}")))
            .unwrap();
        let response = server.router().oneshot(request).await.unwrap();
        assert!(response.status().is_redirection());
        let cookie = response.headers()[header::SET_COOKIE].to_str().unwrap();
        cookie.split(';').next().unwrap().to_owned()
    };
    let get = async |cookie: Option<&str>, uri: &str| {
        let mut request = Request::builder().uri(uri);
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie);
        }
        let response = server
            .router()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap())
    };

    let token_a = mint_admin(
        &server,
        &config.token_pepper,
        &format!("viewer-a-{stamp}"),
        &project_a,
    )
    .await;
    let cookie_a = sign_in(token_a).await;
    let cookie_a = cookie_a.as_str();

    let (status, _) = get(None, "/dashboard/projects").await;
    assert!(
        status.is_redirection(),
        "an anonymous request must be sent to sign in"
    );

    let (status, list) = get(Some(cookie_a), "/dashboard/projects").await;
    assert_eq!(status, StatusCode::OK);
    assert!(list.contains(&project_a));
    assert!(!list.contains(&project_b) && !list.contains(&word_b));

    let (status, page) = get(
        Some(cookie_a),
        &format!("/dashboard/project?project={project_a}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains(&word_a), "the tenant's own memory is missing");
    assert!(
        page.contains(&format!("/work/{word_a}")),
        "the tenant's own session is missing"
    );
    assert!(
        !page.contains(&format!("<b>{word_a}</b>")),
        "a title reached the page unescaped"
    );

    let (status, page) = get(
        Some(cookie_a),
        &format!("/dashboard/project?project={project_b}"),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!page.contains(&word_b));
    let (status, page) = get(
        Some(cookie_a),
        &format!("/dashboard/memory?project={project_b}&key=b-key"),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!page.contains(&word_b));
    let (status, page) = get(
        Some(cookie_a),
        &format!("/dashboard/memory?project={project_a}&key=b-key"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!page.contains(&word_b));
    let (status, page) = get(
        Some(cookie_a),
        &format!("/dashboard/memory?project={project_a}&key=a-key"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains(&word_a));

    let (status, page) = get(Some(cookie_a), &format!("/dashboard/search?q={word_b}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !page.contains("/dashboard/memory?"),
        "a search for another tenant's word found something: {page}"
    );
    let (status, page) = get(
        Some(cookie_a),
        &format!("/dashboard/search?q={word_b}&project={project_b}"),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!page.contains(&word_b));
    let (status, page) = get(Some(cookie_a), &format!("/dashboard/search?q={word_a}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains(&format!("/dashboard/memory?project={project_a}")));

    let token_all = mint_admin(
        &server,
        &config.token_pepper,
        &format!("viewer-all-{stamp}"),
        "*",
    )
    .await;
    let cookie_all = sign_in(token_all).await;
    let (status, page) = get(Some(&cookie_all), &format!("/dashboard/search?q={word_b}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains(&format!("/dashboard/memory?project={project_b}")));
    let (status, _) = get(
        Some(&cookie_all),
        &format!("/dashboard/project?project={project_b}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

/// A cookie outlives the token it was minted from, so each browse page has to
/// ask the store whether the session is still live. The four handlers each
/// carry their own check, which is why every route is requested after the
/// revocation and a failure names the one that served the page.
///
/// The rejected inputs ride along because they need the same signed-in
/// session: a missing project, a blank memory key, and a search term one
/// character over the limit.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to an isolated PostgreSQL database"]
async fn a_revoked_dashboard_session_is_refused_and_bad_browse_input_is_rejected() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    let database_url = std::env::var("TEST_DATABASE_URL").unwrap();
    let stamp = Utc::now().timestamp_micros();
    let project = format!("revoked-{stamp}");
    let name = format!("viewer-revoked-{stamp}");

    let config = CloudConfig {
        database_url,
        dashboard_secret: "a-dashboard-secret-of-at-least-32-bytes".to_owned(),
        token_pepper: "a-token-pepper-of-at-least-32-bytes-long".to_owned(),
        ..CloudConfig::default()
    };
    let server = CloudServer::from_config(config.clone()).await.unwrap();
    server.state.store.migrate().await.unwrap();

    let token = mint_admin(&server, &config.token_pepper, &name, &project).await;
    let request = Request::builder()
        .method("POST")
        .uri("/dashboard/login")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(format!("token={token}")))
        .unwrap();
    let response = server.router().oneshot(request).await.unwrap();
    assert!(response.status().is_redirection());
    let cookie = response.headers()[header::SET_COOKIE].to_str().unwrap();
    let cookie = cookie.split(';').next().unwrap().to_owned();

    let get = async |uri: &str| {
        let request = Request::builder()
            .uri(uri)
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap();
        server.router().oneshot(request).await.unwrap().status()
    };

    let long_term = "x".repeat(MAX_SEARCH_TERM_CHARS + 1);
    let routes = [
        "/dashboard/projects".to_owned(),
        format!("/dashboard/project?project={project}"),
        format!("/dashboard/memory?project={project}&key=absent"),
        format!("/dashboard/search?q=anything&project={project}"),
    ];

    for route in &routes[..2] {
        assert_eq!(get(route).await, StatusCode::OK, "{route} before revoking");
    }
    assert_eq!(
        get(&routes[2]).await,
        StatusCode::NOT_FOUND,
        "{} before revoking",
        routes[2]
    );
    assert_eq!(
        get(&routes[3]).await,
        StatusCode::OK,
        "{} before revoking",
        routes[3]
    );

    assert_eq!(
        get("/dashboard/project").await,
        StatusCode::BAD_REQUEST,
        "a missing project"
    );
    assert_eq!(
        get(&format!("/dashboard/memory?project={project}&key=%20%20")).await,
        StatusCode::NOT_FOUND,
        "a blank memory key"
    );
    assert_eq!(
        get(&format!("/dashboard/search?q={long_term}")).await,
        StatusCode::BAD_REQUEST,
        "a search term over the limit"
    );
    assert_eq!(
        get(&format!(
            "/dashboard/search?q={}",
            "x".repeat(MAX_SEARCH_TERM_CHARS)
        ))
        .await,
        StatusCode::OK,
        "a search term at the limit"
    );

    sqlx::query(
        "UPDATE cloud_principal_tokens SET revoked_at = NOW()
         WHERE principal_id IN (SELECT id FROM cloud_principals WHERE display_name = $1)",
    )
    .bind(&name)
    .execute(server.state.store.pool())
    .await
    .unwrap();

    for route in &routes {
        let status = get(route).await;
        assert!(
            status.is_redirection(),
            "{route} served a session whose token was revoked: {status}"
        );
    }
}
