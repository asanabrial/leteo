use super::*;

#[derive(Debug)]
pub(super) struct ApiError {
    status: StatusCode,
    body: Box<ErrorBody>,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error_class: String,
    error_code: String,
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    project_source: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    project_path: Option<&'static str>,
}

impl ApiError {
    pub(super) fn new(
        status: StatusCode,
        error_class: impl Into<String>,
        error_code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status,
            body: Box::new(ErrorBody {
                error_class: error_class.into(),
                error_code: error_code.into(),
                error: message.into(),
                project: None,
                project_source: None,
                project_path: None,
            }),
        }
    }

    pub(super) fn bad_request(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "repairable",
            "payload_invalid",
            message,
        )
    }

    pub(super) fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "authentication",
            "auth_required",
            message,
        )
    }

    pub(super) fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "policy", "policy_forbidden", message)
    }

    pub(super) fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "blocked",
            "internal",
            "internal server error",
        )
    }

    /// The server could not take the request now and a later one may succeed:
    /// a database that is full or unreachable, a request that outlived its
    /// deadline. A client reads this as "retry", where the 500 it replaces read
    /// as "the server is broken".
    pub(super) fn busy(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "transient",
            "server_busy",
            message,
        )
    }

    /// An error from the store while checking who is asking is the store's
    /// answer, not a verdict on the caller: reported as 401 or 403 it would
    /// tell a client its token is bad and send its sync into the long standing
    /// backoff, when the database was only busy.
    pub(super) fn unless_store(
        error: AuthError,
        otherwise: impl FnOnce(AuthError) -> Self,
    ) -> Self {
        match error {
            AuthError::Store(error) => Self::from(error),
            other => otherwise(other),
        }
    }

    pub(super) fn with_project(mut self, project: &str) -> Self {
        self.body.project = Some(project.to_owned());
        self.body.project_source = Some("request_body");
        self.body.project_path = Some("");
        self
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response<Body> {
        let retry_after = (self.status == StatusCode::SERVICE_UNAVAILABLE)
            .then(|| [(header::RETRY_AFTER, HeaderValue::from(RETRY_AFTER_SECONDS))]);
        (self.status, retry_after, Json(*self.body)).into_response()
    }
}

/// The `503` retry delay matches the pool's five-second acquire timeout.
pub(super) const RETRY_AFTER_SECONDS: u64 = 5;

/// Whether a database error means "not now" rather than "this is broken".
///
/// Pool exhaustion surfaces as `PoolTimedOut` after the acquire timeout, and
/// was answered `500` with the genuine faults. The two SQLSTATEs are the
/// server's own refusals to take a connection (`too_many_connections` and
/// `cannot_connect_now`); an I/O error is a database that cannot be reached.
/// Anything else stays a `500`.
fn database_is_busy(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed | sqlx::Error::Io(_) => true,
        sqlx::Error::Database(failure) => {
            matches!(failure.code().as_deref(), Some("53300" | "57P03"))
        }
        _ => false,
    }
}

impl From<CloudStoreError> for ApiError {
    fn from(error: CloudStoreError) -> Self {
        match error {
            CloudStoreError::Invalid(message) => Self::bad_request(message),
            CloudStoreError::ChunkConflict => Self::new(
                StatusCode::CONFLICT,
                "repairable",
                "chunk_conflict",
                error.to_string(),
            ),
            CloudStoreError::ChunkNotFound => Self::new(
                StatusCode::NOT_FOUND,
                "repairable",
                "chunk_not_found",
                error.to_string(),
            ),
            CloudStoreError::Database(ref failure) if database_is_busy(failure) => {
                tracing::warn!(%error, "cloud database is busy or unreachable");
                Self::busy("the database is busy; retry shortly")
            }
            CloudStoreError::Database(_) => {
                tracing::error!(%error, "cloud database operation failed");
                Self::internal()
            }
        }
    }
}

impl From<AuthError> for ApiError {
    fn from(error: AuthError) -> Self {
        match error {
            AuthError::ProjectRequired => Self::bad_request(error.to_string()),
            AuthError::ProjectForbidden | AuthError::DashboardAdminRequired => {
                Self::forbidden(error.to_string())
            }
            AuthError::Store(error) => Self::from(error),
            _ => Self::unauthorized(error.to_string()),
        }
    }
}
