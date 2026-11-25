use std::{
    collections::BTreeSet,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    BoxError, Json, Router,
    body::Body,
    error_handling::HandleErrorLayer,
    extract::{DefaultBodyLimit, Extension, Form, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tower::ServiceBuilder;

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

/// How long a request has from arrival to its response, queueing for a
/// concurrency slot included; past it the answer is `503` with `Retry-After`.
///
/// Under the sync client's own 30 second timeout on purpose: a server that
/// gives up later than its client does is a server whose answer nobody reads,
/// and the client sees a dropped connection where it could have seen a `503`.
/// Before this bound the only limit on a request was the pool's five-second
/// acquire timeout, which a request already holding a connection never met.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(25);

/// How much of the request's deadline the mutation push must leave for its own
/// answer.
///
/// The request deadline cancels the handler and the layer above answers `503`
/// with `Retry-After`, which for a commit in flight tells the client to append
/// the batch twice. The insert's own bound fires first, in the handler, which
/// can say so; this is the time that answer needs to be written.
const MUTATION_PUSH_MARGIN: Duration = Duration::from_secs(1);

/// When the request reached the router, stamped before it queues for a slot.
///
/// A push that spent most of its deadline waiting cannot spend a fresh one
/// inserting: the outer deadline would cancel the commit first and answer the
/// retry that duplicates it.
#[derive(Clone, Copy)]
struct RequestArrival(Instant);

/// What the insert may still take: the request's own bound, less the time the
/// request has already spent, less the margin its answer needs.
fn push_budget(spent: Duration) -> Duration {
    REQUEST_TIMEOUT
        .saturating_sub(spent)
        .saturating_sub(MUTATION_PUSH_MARGIN)
}

/// How many requests run at once; the next ones wait for a slot, for as long
/// as `REQUEST_TIMEOUT` leaves them, and are then answered `503`.
///
/// Waiting rather than refusing, because a sync client pushing a backlog is
/// better served by a short queue than by a retry storm. The bound sits above
/// the default connection pool, so the pool's own acquire timeout stays the
/// first thing a flood of database work meets.
const MAX_IN_FLIGHT_REQUESTS: usize = 64;

/// The two bounds a request lives under, gathered so a test can use the
/// published ones or shrink them without a second copy of the code applying them.
#[derive(Clone, Copy)]
struct Bounds {
    request_timeout: Duration,
    max_in_flight: usize,
}

impl Bounds {
    const PUBLISHED: Self = Self {
        request_timeout: REQUEST_TIMEOUT,
        max_in_flight: MAX_IN_FLIGHT_REQUESTS,
    };
}

/// The deadline is the outer layer so that time spent queued for a slot counts
/// against it; inside, a request could wait for a slot forever.
///
/// It takes a router whose state has been applied, and only that. On a router
/// still waiting for its state axum builds each route again for every request,
/// the layer with it, and a concurrency limit then gets a fresh semaphore per
/// request: the bound reads as applied and bounds nothing. Measured, as 192
/// handlers running at once under a limit of 64.
///
/// POST /sync/push deduplicates the chunk by project and canonical id in the
/// same transaction as its derived mutations. POST /sync/mutations/push appends
/// new sequences on replay and can duplicate a batch after an uncertain commit.
/// POST /dashboard/login mints a signed cookie without creating a session row.
/// Managed bearer authentication can refresh last_used_at, and paused pushes
/// can log another rejected attempt. Those writes do not replay sync data.
fn bounded(router: Router, bounds: Bounds) -> Router {
    let router = router.layer(
        ServiceBuilder::new()
            // Outside the deadline on purpose: this is arrival, and the
            // deadline it feeds is spent by queueing for a slot.
            .layer(axum::middleware::map_request(
                |mut request: axum::extract::Request| async move {
                    request
                        .extensions_mut()
                        .insert(RequestArrival(Instant::now()));
                    request
                },
            ))
            .layer(HandleErrorLayer::new(|_: BoxError| async {
                ApiError::busy("the request took too long; retry shortly")
            }))
            .timeout(bounds.request_timeout)
            .concurrency_limit(bounds.max_in_flight),
    );

    #[cfg(test)]
    let router = router.layer(axum::middleware::map_response(
        move |mut response: Response| async move {
            response.extensions_mut().insert(bounds);
            response
        },
    ));

    router
}

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
        let router = Router::new()
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
            .with_state(Arc::clone(&self.state));
        bounded(router, Bounds::PUBLISHED)
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

/// Runs the mutation insert under a bound the handler can answer for.
///
/// A future cancelled by the request deadline leaves no handler behind to
/// answer: the timeout layer above says `503`, and a client that retries
/// appends the batch a second time when the commit had already landed. This
/// bound fires first, so the answer is the one that says the outcome is
/// unknown.
async fn insert_mutations_within(
    budget: Duration,
    insert: impl std::future::Future<Output = Result<Vec<i64>, CloudStoreError>>,
) -> Result<Vec<i64>, ApiError> {
    // Nothing was written, so this one is safe to send again.
    if budget.is_zero() {
        return Err(ApiError::busy("the request has no time left"));
    }
    match tokio::time::timeout(budget, insert).await {
        Ok(result) => result.map_err(ApiError::from),
        Err(_) => {
            tracing::warn!("a mutation push outlived its budget with the commit in flight");
            Err(ApiError::outcome_unknown())
        }
    }
}

async fn push_mutations(
    State(state): State<Arc<AppState>>,
    Extension(arrival): Extension<RequestArrival>,
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
    // What is left of the request's own bound, not a fresh one: a push that
    // waited for a slot has spent it.
    let accepted_seqs = insert_mutations_within(
        push_budget(arrival.0.elapsed()),
        state.store.insert_mutations(&request.entries),
    )
    .await?;
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
    /// The projects the peer enrolled, comma-separated.
    ///
    /// A wildcard grant reads every project on the server, so a peer that asked
    /// for one project would otherwise receive all of them. This narrows the
    /// page to the intersection of the grant and this list; omitting it keeps
    /// the old behaviour for a client that does not know about it.
    projects: Option<String>,
}

/// What one pull page may contain: what the token is granted, narrowed to what
/// the peer says it enrolled.
///
/// `None` still means "no scope at all" — a wildcard grant and a client that
/// asked for nothing — and an empty vector means the peer enrolled nothing, so
/// the page is empty rather than unfiltered. A request carrying the wildcard
/// entry is the peer saying it replicates every project, which is the same as
/// asking for nothing; the store's own wildcard rule is
/// `super::remote::allows_every_project`.
fn narrow_pull_scope(granted: Option<Vec<String>>, requested: Option<&str>) -> Option<Vec<String>> {
    let requested = requested.map(super::config::parse_projects);
    if requested
        .as_deref()
        .is_some_and(super::remote::allows_every_project)
    {
        return granted;
    }
    match (granted, requested) {
        (None, None) => None,
        (None, Some(requested)) => Some(requested),
        (granted, None) => granted,
        (Some(granted), Some(requested)) => {
            let granted = granted.into_iter().collect::<BTreeSet<_>>();
            Some(
                requested
                    .into_iter()
                    .filter(|project| granted.contains(project))
                    .collect(),
            )
        }
    }
}

async fn pull_mutations(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<MutationPullQuery>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    let granted = state
        .auth
        .enrolled_projects(&state.store, &principal)
        .await?;
    let scope = narrow_pull_scope(granted, query.projects.as_deref());
    let (mutations, has_more, latest_seq) = state
        .store
        .list_mutations_since(
            query.since_seq.unwrap_or_default().max(0),
            query.limit.unwrap_or(MAX_MUTATION_BATCH_SIZE),
            scope.as_deref(),
        )
        .await?;
    let project = scope
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
        .map_err(|error| {
            ApiError::unless_store(error, |_| ApiError::unauthorized("invalid admin token"))
        })?;
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

#[cfg(test)]
mod pull_scope_tests {
    use super::narrow_pull_scope;

    fn scope(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn a_wildcard_grant_follows_the_projects_the_peer_asked_for() {
        assert_eq!(
            narrow_pull_scope(None, Some("leteo")).unwrap(),
            scope(&["leteo"]),
            "a token granted * is narrowed by what the peer enrolled"
        );
        assert_eq!(
            narrow_pull_scope(None, Some("leteo,otro")).unwrap(),
            scope(&["leteo", "otro"]),
            "and by every project in the list"
        );
    }

    #[test]
    fn a_client_that_asks_for_nothing_keeps_the_old_answer() {
        assert_eq!(narrow_pull_scope(None, None), None);
        assert_eq!(
            narrow_pull_scope(Some(scope(&["leteo"])), None).unwrap(),
            scope(&["leteo"]),
            "a server that has not been told a scope still sends what the grant allows"
        );
    }

    #[test]
    fn a_granted_project_is_never_widened_by_the_request() {
        assert_eq!(
            narrow_pull_scope(Some(scope(&["leteo"])), Some("leteo,otro")).unwrap(),
            scope(&["leteo"]),
            "asking for another tenant's project yields nothing of it"
        );
        assert_eq!(
            narrow_pull_scope(Some(scope(&["leteo"])), Some("otro")).unwrap(),
            Vec::<String>::new(),
            "and an empty page when it is the only one asked for"
        );
    }

    #[test]
    fn an_empty_list_is_an_empty_page_rather_than_no_scope() {
        assert_eq!(
            narrow_pull_scope(None, Some("")).unwrap(),
            Vec::<String>::new(),
            "a peer that enrolled nothing receives nothing, not everything"
        );
    }

    #[test]
    fn a_wildcard_request_is_the_peers_everything_not_a_project_named_star() {
        assert_eq!(
            narrow_pull_scope(None, Some("*")),
            None,
            "a wildcard peer asks for every project the grant allows"
        );
        assert_eq!(
            narrow_pull_scope(Some(scope(&["leteo", "otro"])), Some("*")).unwrap(),
            scope(&["leteo", "otro"]),
            "and the grant still bounds it"
        );
    }
}
