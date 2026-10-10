use std::{collections::BTreeSet, sync::Arc};

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Form, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::sync::{ChunkData, MutationEntry, canonicalize_for_project, decode_chunk};

use super::{
    MAX_MUTATION_BATCH_SIZE,
    auth::{AuthError, AuthService, Principal, PrincipalRole, PrincipalSource},
    cloudstore::{AuditEntry, CloudStore, CloudStoreError, MEMORY_LIST_LIMIT, MEMORY_SEARCH_LIMIT},
    config::CloudConfig,
};

mod browser;
mod error;
mod guard;
mod payload;
#[cfg(test)]
mod tests;

use browser::{
    cookie_value, dashboard_cookie, escape_html, nonempty_or, render_memory, render_project,
    render_projects, render_search, requires_secure_cookie,
};
use error::ApiError;
use guard::{
    authenticate, authorize_project, dashboard_session_is_live, ensure_not_paused, login_redirect,
    visible_projects,
};
use payload::{validate_chunk_payload, validate_mutation_entries, validate_session_references};

pub(super) const DASHBOARD_COOKIE: &str = "leteo_dashboard";

#[derive(Clone)]
pub struct CloudServer {
    state: Arc<AppState>,
}

struct AppState {
    store: CloudStore,
    auth: AuthService,
    config: CloudConfig,
}

impl CloudServer {
    pub async fn from_config(config: CloudConfig) -> Result<Self, ServerError> {
        config.validate()?;
        let store = CloudStore::connect(&config.database_url, config.max_pool).await?;
        let auth = AuthService::new(
            config.dashboard_secret.as_bytes(),
            (!config.token_pepper.trim().is_empty()).then_some(config.token_pepper.as_str()),
            config.sync_token.clone(),
            config.admin_token.clone(),
            &config.allowed_projects,
        )?;
        Ok(Self::new(store, auth, config))
    }

    fn new(store: CloudStore, auth: AuthService, config: CloudConfig) -> Self {
        Self {
            state: Arc::new(AppState {
                store,
                auth,
                config,
            }),
        }
    }

    pub fn router(&self) -> Router {
        Router::new()
            .route("/health", get(health))
            .route("/sync/pull", get(pull_manifest))
            .route("/sync/pull/{chunk_id}", get(pull_chunk))
            .route("/sync/push", post(push_chunk))
            .route("/sync/mutations/push", post(push_mutations))
            .route("/sync/mutations/pull", get(pull_mutations))
            .route("/dashboard/login", get(login_page).post(login))
            .route("/dashboard", get(dashboard))
            .route("/dashboard/projects", get(browse_projects))
            .route("/dashboard/project", get(browse_project))
            .route("/dashboard/memory", get(browse_memory))
            .route("/dashboard/search", get(browse_search))
            .layer(DefaultBodyLimit::max(self.state.config.max_push_body_bytes))
            .with_state(Arc::clone(&self.state))
    }

    pub async fn serve(self) -> Result<(), ServerError> {
        self.state.config.validate()?;
        let address = format!(
            "{}:{}",
            self.state.config.bind_host.trim(),
            self.state.config.port
        );
        let listener = tokio::net::TcpListener::bind(&address).await?;
        axum::serve(listener, self.router())
            .await
            .map_err(ServerError::Serve)
    }
}

#[derive(Debug, Error)]
pub enum ServerError {
    #[error(transparent)]
    Config(#[from] super::config::ConfigError),
    #[error(transparent)]
    Store(#[from] CloudStoreError),
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error("bind cloud server: {0}")]
    Bind(#[from] std::io::Error),
    #[error("serve cloud server: {0}")]
    Serve(std::io::Error),
}

async fn health(State(state): State<Arc<AppState>>) -> Response {
    match state.store.health().await {
        Ok(()) => {
            Json(serde_json::json!({"status": "ok", "service": "leteo-cloud"})).into_response()
        }
        Err(error) => {
            tracing::error!(%error, "cloud health check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "status": "unhealthy",
                    "service": "leteo-cloud",
                })),
            )
                .into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
struct ProjectQuery {
    project: Option<String>,
}

async fn pull_manifest(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ProjectQuery>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    let project = authorize_project(&state, &principal, query.project.as_deref()).await?;
    let manifest = state.store.read_manifest(&project).await?;
    Ok(Json(manifest).into_response())
}

async fn pull_chunk(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ProjectQuery>,
    Path(chunk_id): Path<String>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    let project = authorize_project(&state, &principal, query.project.as_deref()).await?;
    if crate::sync::validate_chunk_id(chunk_id.trim()).is_err() {
        return Err(ApiError::bad_request(
            "chunk_id must be eight lowercase hexadecimal characters",
        ));
    }
    let payload = match state.store.read_chunk(&project, &chunk_id).await {
        Ok(payload) => payload,
        Err(CloudStoreError::ChunkNotFound) => {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                "repairable",
                "chunk_not_found",
                "chunk not found",
            ));
        }
        Err(error) => return Err(error.into()),
    };
    Ok(([(header::CONTENT_TYPE, "application/json")], payload).into_response())
}

#[derive(Debug, Deserialize)]
struct ChunkPushRequest {
    #[serde(default)]
    chunk_id: String,
    #[serde(default)]
    created_by: String,
    #[serde(default)]
    client_created_at: String,
    project: String,
    data: Value,
}

async fn push_chunk(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<ChunkPushRequest>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    let project = authorize_project(&state, &principal, Some(&request.project)).await?;
    ensure_not_paused(
        &state,
        &project,
        nonempty_or(&request.created_by, &principal.display_name),
        "chunk_push",
        0,
    )
    .await?;
    let raw = serde_json::to_vec(&request.data)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let canonical = canonicalize_for_project(&raw, &project)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let chunk = validate_chunk_payload(&canonical)?;
    let known_sessions = state.store.known_session_ids(&project).await?;
    validate_session_references(&chunk, &known_sessions)?;
    let client_created_at = if request.client_created_at.trim().is_empty() {
        None
    } else {
        Some(
            DateTime::parse_from_rfc3339(request.client_created_at.trim())
                .map_err(|_| ApiError::bad_request("client_created_at must be RFC3339"))?
                .with_timezone(&Utc),
        )
    };
    let canonical_chunk_id = crate::sync::chunk_id(&canonical);
    if !request.chunk_id.trim().is_empty() && request.chunk_id.trim() != canonical_chunk_id {
        tracing::debug!(
            client_chunk_id = request.chunk_id.trim(),
            server_chunk_id = canonical_chunk_id,
            "using server-canonicalized chunk id"
        );
    }
    let chunk_id = state
        .store
        .write_chunk(
            &project,
            &canonical_chunk_id,
            nonempty_or(&request.created_by, &principal.display_name),
            client_created_at,
            &canonical,
        )
        .await?;
    Ok(Json(serde_json::json!({"status": "ok", "chunk_id": chunk_id})).into_response())
}

#[derive(Debug, Deserialize)]
struct MutationPushRequest {
    #[serde(default)]
    entries: Vec<MutationEntry>,
    #[serde(default)]
    created_by: String,
}

async fn push_mutations(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut request): Json<MutationPushRequest>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    validate_mutation_entries(&request.entries)?;
    let mut projects = BTreeSet::new();
    for entry in &mut request.entries {
        let project = authorize_project(&state, &principal, Some(&entry.project)).await?;
        entry.project.clone_from(&project);
        projects.insert(project);
    }
    let contributor = nonempty_or(&request.created_by, &principal.display_name);
    for project in &projects {
        ensure_not_paused(
            &state,
            project,
            contributor,
            "mutation_push",
            request.entries.len(),
        )
        .await?;
    }
    let accepted_seqs = state.store.insert_mutations(&request.entries).await?;
    let project = request
        .entries
        .first()
        .map(|entry| entry.project.clone())
        .unwrap_or_default();
    Ok(Json(serde_json::json!({
        "accepted_seqs": accepted_seqs,
        "project": project,
        "project_source": "request_body",
        "project_path": "",
    }))
    .into_response())
}

#[derive(Debug, Default, Deserialize)]
struct MutationPullQuery {
    since_seq: Option<i64>,
    limit: Option<usize>,
}

async fn pull_mutations(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<MutationPullQuery>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    let allowed = state
        .auth
        .enrolled_projects(&state.store, &principal)
        .await?;
    let (mutations, has_more, latest_seq) = state
        .store
        .list_mutations_since(
            query.since_seq.unwrap_or_default().max(0),
            query.limit.unwrap_or(MAX_MUTATION_BATCH_SIZE),
            allowed.as_deref(),
        )
        .await?;
    let project = allowed
        .as_ref()
        .and_then(|projects| projects.first())
        .cloned()
        .unwrap_or_default();
    Ok(Json(serde_json::json!({
        "mutations": mutations,
        "has_more": has_more,
        "latest_seq": latest_seq,
        "project": project,
        "project_source": "request_body",
        "project_path": "",
    }))
    .into_response())
}

async fn login_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Leteo Cloud Login</title><style>body{font:16px system-ui;max-width:32rem;margin:12vh auto;padding:1rem;background:#f4f1e8;color:#17211b}form{display:grid;gap:1rem;padding:2rem;background:white;border:1px solid #c7c1b3}input,button{font:inherit;padding:.7rem}button{background:#17211b;color:white;border:0}</style></head><body><main><h1>Leteo Cloud</h1><form method="post"><label>Admin bearer token<input name="token" type="password" required autocomplete="current-password"></label><button type="submit">Sign in</button></form></main></body></html>"#,
    )
}

#[derive(Debug, Deserialize)]
struct LoginForm {
    token: String,
}

async fn login(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Result<Response, ApiError> {
    let principal = state
        .auth
        .resolve_bearer(&state.store, form.token.trim())
        .await
        .map_err(|_| ApiError::unauthorized("invalid admin token"))?;
    if principal.role != PrincipalRole::Admin {
        return Err(ApiError::forbidden("admin role is required"));
    }
    let session = state.auth.mint_dashboard_session(&principal)?;
    let cookie = dashboard_cookie(&session, requires_secure_cookie(&headers));
    let mut response = Redirect::to("/dashboard").into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(|error| {
            tracing::error!(%error, "failed to create dashboard cookie");
            ApiError::internal()
        })?,
    );
    Ok(response)
}

async fn dashboard(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let Some(session) = cookie_value(&headers, DASHBOARD_COOKIE) else {
        return Ok(login_redirect());
    };
    let Ok(principal) = state.auth.parse_dashboard_session(session) else {
        return Ok(login_redirect());
    };
    if !dashboard_session_is_live(&state, &principal).await? {
        return Ok(login_redirect());
    }
    let stats = state.store.stats().await?;
    let html = format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Leteo Cloud</title><style>body{{font:16px system-ui;max-width:64rem;margin:3rem auto;padding:1rem;background:#f4f1e8;color:#17211b}}dl{{display:grid;grid-template-columns:repeat(auto-fit,minmax(9rem,1fr));gap:1rem}}div{{padding:1.25rem;background:white;border-top:4px solid #27684a}}dt{{font-size:.8rem;text-transform:uppercase}}dd{{font-size:2rem;margin:.3rem 0}}</style></head><body><header><p>Signed in as {}</p><h1>Cloud runtime</h1></header><dl><div><dt>Principals</dt><dd>{}</dd></div><div><dt>Chunks</dt><dd>{}</dd></div><div><dt>Mutations</dt><dd>{}</dd></div><div><dt>Paused projects</dt><dd>{}</dd></div></dl><p><a href="/dashboard/projects">Browse projects and memories</a></p></body></html>"#,
        escape_html(&principal.display_name),
        stats.principals,
        stats.chunks,
        stats.mutations,
        stats.paused_projects,
    );
    Ok(Html(html).into_response())
}

#[derive(Debug, Deserialize)]
struct BrowseQuery {
    project: Option<String>,
    key: Option<String>,
    q: Option<String>,
}

/// The longest search term accepted, in characters.
const MAX_SEARCH_TERM_CHARS: usize = 200;

async fn browse_projects(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let Some(session) = cookie_value(&headers, DASHBOARD_COOKIE) else {
        return Ok(login_redirect());
    };
    let Ok(principal) = state.auth.parse_dashboard_session(session) else {
        return Ok(login_redirect());
    };
    if !dashboard_session_is_live(&state, &principal).await? {
        return Ok(login_redirect());
    }
    let visible = visible_projects(&state, &principal).await?;
    let projects = state.store.list_projects(&visible).await?;
    Ok(Html(render_projects(&principal.display_name, &projects)).into_response())
}

async fn browse_project(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<BrowseQuery>,
) -> Result<Response, ApiError> {
    let Some(session) = cookie_value(&headers, DASHBOARD_COOKIE) else {
        return Ok(login_redirect());
    };
    let Ok(principal) = state.auth.parse_dashboard_session(session) else {
        return Ok(login_redirect());
    };
    if !dashboard_session_is_live(&state, &principal).await? {
        return Ok(login_redirect());
    }
    let project = authorize_project(&state, &principal, query.project.as_deref()).await?;
    let sessions = state.store.list_sessions(&project).await?;
    let memories = state
        .store
        .list_memories(std::slice::from_ref(&project), None, MEMORY_LIST_LIMIT)
        .await?;
    Ok(Html(render_project(
        &principal.display_name,
        &project,
        &sessions,
        &memories,
    ))
    .into_response())
}

async fn browse_memory(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<BrowseQuery>,
) -> Result<Response, ApiError> {
    let Some(session) = cookie_value(&headers, DASHBOARD_COOKIE) else {
        return Ok(login_redirect());
    };
    let Ok(principal) = state.auth.parse_dashboard_session(session) else {
        return Ok(login_redirect());
    };
    if !dashboard_session_is_live(&state, &principal).await? {
        return Ok(login_redirect());
    }
    let project = authorize_project(&state, &principal, query.project.as_deref()).await?;
    let key = query.key.as_deref().map(str::trim).unwrap_or_default();
    let Some(memory) = state.store.get_memory(&project, key).await? else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "repairable",
            "memory_not_found",
            "no such memory in this project",
        ));
    };
    Ok(Html(render_memory(&principal.display_name, &memory)).into_response())
}

async fn browse_search(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<BrowseQuery>,
) -> Result<Response, ApiError> {
    let Some(session) = cookie_value(&headers, DASHBOARD_COOKIE) else {
        return Ok(login_redirect());
    };
    let Ok(principal) = state.auth.parse_dashboard_session(session) else {
        return Ok(login_redirect());
    };
    if !dashboard_session_is_live(&state, &principal).await? {
        return Ok(login_redirect());
    }
    let term = query.q.as_deref().map(str::trim).unwrap_or_default();
    if term.chars().count() > MAX_SEARCH_TERM_CHARS {
        return Err(ApiError::bad_request(format!(
            "a search term is at most {MAX_SEARCH_TERM_CHARS} characters"
        )));
    }
    let (scope, only_project) = match query
        .project
        .as_deref()
        .filter(|name| !name.trim().is_empty())
    {
        Some(requested) => {
            let project = authorize_project(&state, &principal, Some(requested)).await?;
            (vec![project.clone()], Some(project))
        }
        None => (visible_projects(&state, &principal).await?, None),
    };
    let results = if term.is_empty() {
        None
    } else {
        Some(
            state
                .store
                .list_memories(&scope, Some(term), MEMORY_SEARCH_LIMIT)
                .await?,
        )
    };
    Ok(Html(render_search(
        &principal.display_name,
        term,
        only_project.as_deref(),
        results.as_ref(),
    ))
    .into_response())
}
