//! Who is asking, and whether they may.
//!
//! There is no middleware on the router: every handler calls these itself. That
//! is a shape where one forgetful handler is an open door, so they live
//! together where the omission is visible rather than beside the route each
//! guards.

use super::*;

pub(super) async fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Principal, ApiError> {
    let header = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::unauthorized("missing authorization header"))?;
    let mut fields = header.split_whitespace();
    let scheme = fields.next().unwrap_or_default();
    let token = fields.next().unwrap_or_default();
    if !scheme.eq_ignore_ascii_case("bearer") || token.is_empty() || fields.next().is_some() {
        return Err(ApiError::unauthorized(
            "authorization must use a single Bearer token",
        ));
    }
    state
        .auth
        .resolve_bearer(&state.store, token)
        .await
        .map_err(|_| ApiError::unauthorized("invalid bearer token"))
}

pub(super) async fn authorize_project(
    state: &AppState,
    principal: &Principal,
    project: Option<&str>,
) -> Result<String, ApiError> {
    let project = project.unwrap_or_default();
    state
        .auth
        .authorize_project(&state.store, principal, project)
        .await
        .map_err(|error| match error {
            AuthError::ProjectRequired => ApiError::bad_request(error.to_string()),
            _ => ApiError::forbidden("project is not allowed"),
        })
}

pub(super) async fn ensure_not_paused(
    state: &AppState,
    project: &str,
    contributor: &str,
    action: &str,
    entry_count: usize,
) -> Result<(), ApiError> {
    if state.store.is_project_sync_enabled(project).await? {
        return Ok(());
    }
    state
        .store
        .record_audit(AuditEntry {
            contributor,
            project,
            action,
            outcome: "rejected_project_paused",
            entry_count,
            reason_code: Some("sync-paused"),
        })
        .await?;
    Err(ApiError::new(
        StatusCode::CONFLICT,
        "policy",
        "sync-paused",
        format!("sync is paused for project {project:?}"),
    )
    .with_project(project))
}
