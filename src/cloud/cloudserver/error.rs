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

    pub(super) fn with_project(mut self, project: &str) -> Self {
        self.body.project = Some(project.to_owned());
        self.body.project_source = Some("request_body");
        self.body.project_path = Some("");
        self
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response<Body> {
        (self.status, Json(*self.body)).into_response()
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
            AuthError::Store(error) => {
                tracing::error!(%error, "cloud authentication store operation failed");
                Self::internal()
            }
            _ => Self::unauthorized(error.to_string()),
        }
    }
}
