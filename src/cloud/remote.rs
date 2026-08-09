use std::{net::IpAddr, time::Duration};

use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use thiserror::Error;

use crate::sync::Manifest;

use super::cloudstore::{MutationEntry, StoredMutation};

/// Largest response body the client buffers, for any endpoint.
///
/// It matches the ceiling on an imported sync chunk, which is the largest thing
/// the cloud legitimately returns.
const MAX_RESPONSE_BYTES: u64 = crate::sync::MAX_UNCOMPRESSED_CHUNK_BYTES;

#[derive(Debug, Clone)]
pub struct RemoteClient {
    base_url: Url,
    token: String,
    client: Client,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PushMutationsResponse {
    pub accepted_seqs: Vec<i64>,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub project_source: String,
    #[serde(default)]
    pub project_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PullMutationsResponse {
    #[serde(default)]
    pub mutations: Vec<StoredMutation>,
    pub has_more: bool,
    pub latest_seq: i64,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub project_source: String,
    #[serde(default)]
    pub project_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpStatusError {
    pub operation: String,
    pub status: u16,
    pub error_class: String,
    pub error_code: String,
    pub message: String,
}

impl std::fmt::Display for HttpStatusError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "cloud {} returned {}: {}",
            self.operation, self.status, self.message
        )
    }
}

impl std::error::Error for HttpStatusError {}

impl HttpStatusError {
    pub fn is_auth_failure(&self) -> bool {
        self.status == 401
    }

    pub fn is_policy_failure(&self) -> bool {
        self.status == 403 || self.status == 409
    }
}

#[derive(Debug, Error)]
pub enum RemoteError {
    #[error("invalid cloud remote URL: {0}")]
    InvalidUrl(String),
    #[error("build cloud HTTP client: {0}")]
    Build(#[source] reqwest::Error),
    #[error("cloud HTTP request: {0}")]
    Request(#[source] reqwest::Error),
    #[error(transparent)]
    Status(#[from] HttpStatusError),
    #[error("invalid cloud response: {0}")]
    InvalidResponse(String),
}

impl RemoteClient {
    pub fn new(base_url: &str, token: &str) -> Result<Self, RemoteError> {
        let base_url = validate_base_url(base_url)?;
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(RemoteError::Build)?;
        Ok(Self {
            base_url,
            token: token.trim().to_owned(),
            client,
        })
    }

    pub async fn health(&self) -> Result<Value, RemoteError> {
        let request = self.client.get(self.endpoint("health")?);
        self.send_json(request, "health").await
    }

    pub async fn pull_manifest(&self, project: &str) -> Result<Manifest, RemoteError> {
        let url = self.endpoint_with_query("sync/pull", &[("project", project.to_owned())])?;
        let request = self.authorized(self.client.get(url));
        self.send_json(request, "pull manifest").await
    }

    pub async fn push_chunk(
        &self,
        project: &str,
        chunk_id: &str,
        created_by: &str,
        client_created_at: &str,
        data: &[u8],
    ) -> Result<String, RemoteError> {
        let data: Value = serde_json::from_slice(data)
            .map_err(|error| RemoteError::InvalidResponse(error.to_string()))?;
        let body = serde_json::json!({
            "chunk_id": chunk_id,
            "created_by": created_by,
            "client_created_at": client_created_at,
            "project": project,
            "data": data,
        });
        let request = self
            .authorized(self.client.post(self.endpoint("sync/push")?))
            .json(&body);
        let response: PushChunkResponse = self.send_json(request, "push chunk").await?;
        Ok(response.chunk_id)
    }

    pub async fn pull_chunk(&self, project: &str, chunk_id: &str) -> Result<Vec<u8>, RemoteError> {
        let path = format!("sync/pull/{chunk_id}");
        let url = self.endpoint_with_query(&path, &[("project", project.to_owned())])?;
        let request = self.authorized(self.client.get(url));
        let response = request.send().await.map_err(RemoteError::Request)?;
        let response = checked_response(response, "pull chunk").await?;
        let bytes = read_bounded(response, MAX_RESPONSE_BYTES).await?;
        if bytes.is_empty() {
            return Err(RemoteError::InvalidResponse(
                "empty chunk response".to_owned(),
            ));
        }
        Ok(bytes)
    }

    pub async fn push_mutations(
        &self,
        entries: &[MutationEntry],
        created_by: &str,
    ) -> Result<PushMutationsResponse, RemoteError> {
        let body = serde_json::json!({"entries": entries, "created_by": created_by});
        let request = self
            .authorized(self.client.post(self.endpoint("sync/mutations/push")?))
            .json(&body);
        self.send_json(request, "push mutations").await
    }

    pub async fn pull_mutations(
        &self,
        since_sequence: i64,
        limit: usize,
    ) -> Result<PullMutationsResponse, RemoteError> {
        let url = self.endpoint_with_query(
            "sync/mutations/pull",
            &[
                ("since_seq", since_sequence.max(0).to_string()),
                ("limit", limit.clamp(1, 100).to_string()),
            ],
        )?;
        let request = self.authorized(self.client.get(url));
        self.send_json(request, "pull mutations").await
    }

    fn endpoint(&self, path: &str) -> Result<Url, RemoteError> {
        self.base_url
            .join(path.trim_start_matches('/'))
            .map_err(|error| RemoteError::InvalidUrl(error.to_string()))
    }

    fn endpoint_with_query(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Url, RemoteError> {
        let mut url = self.endpoint(path)?;
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }
        Ok(url)
    }

    fn authorized(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.token.is_empty() {
            request
        } else {
            request.bearer_auth(&self.token)
        }
    }

    async fn send_json<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
        operation: &str,
    ) -> Result<T, RemoteError> {
        let response = request.send().await.map_err(RemoteError::Request)?;
        let response = checked_response(response, operation).await?;
        let body = read_bounded(response, MAX_RESPONSE_BYTES).await?;
        serde_json::from_slice(&body)
            .map_err(|error| RemoteError::InvalidResponse(error.to_string()))
    }
}

#[derive(Debug, Deserialize)]
struct PushChunkResponse {
    chunk_id: String,
}

/// Reads a response body, refusing to buffer more than `limit` bytes.
///
/// The client validates a chunk's hash only after it has the whole thing, so a
/// compromised or simply broken cloud could hand back an unbounded body and
/// exhaust memory first. Transport compression makes that cheap to send, and
/// reqwest decompresses transparently — streaming is what keeps the cap
/// honest, because the limit then applies to the decompressed bytes as they
/// arrive rather than to what went over the wire.
async fn read_bounded(response: reqwest::Response, limit: u64) -> Result<Vec<u8>, RemoteError> {
    use futures_util::StreamExt;

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(RemoteError::Request)?;
        if body.len() as u64 + chunk.len() as u64 > limit {
            return Err(RemoteError::InvalidResponse(format!(
                "response body exceeds the {limit} byte limit"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn checked_response(
    response: reqwest::Response,
    operation: &str,
) -> Result<reqwest::Response, RemoteError> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let envelope = serde_json::from_str::<ErrorEnvelope>(&body).unwrap_or_default();
    let error_code = if status == StatusCode::NOT_FOUND && envelope.error_code.is_empty() {
        "server_unsupported".to_owned()
    } else {
        envelope.error_code
    };
    Err(HttpStatusError {
        operation: operation.to_owned(),
        status: status.as_u16(),
        error_class: envelope.error_class,
        error_code,
        message: if envelope.error.is_empty() {
            body.trim().to_owned()
        } else {
            envelope.error
        },
    }
    .into())
}

#[derive(Debug, Default, Deserialize)]
struct ErrorEnvelope {
    #[serde(default)]
    error_class: String,
    #[serde(default)]
    error_code: String,
    #[serde(default)]
    error: String,
}

fn validate_base_url(value: &str) -> Result<Url, RemoteError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(RemoteError::InvalidUrl("URL is required".to_owned()));
    }
    let mut url = Url::parse(value).map_err(|error| RemoteError::InvalidUrl(error.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(RemoteError::InvalidUrl(
            "scheme must be http or https and host is required".to_owned(),
        ));
    }
    if url.scheme() == "http" && !is_loopback_host(url.host_str().unwrap_or_default()) {
        return Err(RemoteError::InvalidUrl(
            "http is allowed only for localhost or loopback IP addresses; use https for remote cloud servers"
                .to_owned(),
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(RemoteError::InvalidUrl(
            "query and fragment are not allowed".to_owned(),
        ));
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serves a fixed chunk body on loopback and returns its address.
    async fn chunk_server(bytes: usize) -> std::net::SocketAddr {
        use axum::{Router, routing::get};

        let app = Router::new().route(
            "/sync/pull/{chunk_id}",
            get(move || async move { vec![b'a'; bytes] }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        address
    }

    #[tokio::test]
    async fn an_oversized_response_is_refused_instead_of_buffered() {
        let limit = 4096_u64;
        let address = chunk_server(limit as usize * 4).await;
        let client = RemoteClient::new(&format!("http://{address}"), "token").unwrap();
        let fetch = || {
            client
                .client
                .get(format!("http://{address}/sync/pull/abcdef01"))
                .send()
        };

        let error = read_bounded(fetch().await.unwrap(), limit)
            .await
            .unwrap_err();
        assert!(
            matches!(error, RemoteError::InvalidResponse(ref message) if message.contains("exceeds")),
            "unexpected error: {error}"
        );

        // A body within the ceiling still arrives whole.
        let body = read_bounded(fetch().await.unwrap(), limit * 8)
            .await
            .unwrap();
        assert_eq!(body.len(), limit as usize * 4);
    }

    #[tokio::test]
    async fn a_chunk_within_the_ceiling_is_returned_whole() {
        let address = chunk_server(64).await;
        let client = RemoteClient::new(&format!("http://{address}"), "token").unwrap();

        let chunk = client.pull_chunk("leteo", "abcdef01").await.unwrap();

        assert_eq!(chunk, vec![b'a'; 64]);
    }

    #[test]
    fn remote_url_validation_rejects_unsafe_shapes() {
        for invalid in [
            "",
            "example.com",
            "ftp://example.com",
            "http://",
            "http://example.com",
            "http://192.168.1.10:8080",
            "https://example.com?token=secret",
            "https://example.com/#fragment",
        ] {
            assert!(RemoteClient::new(invalid, "token").is_err(), "{invalid}");
        }
    }

    #[test]
    fn remote_url_allows_plain_http_only_on_loopback() {
        for valid in [
            "http://localhost:8080",
            "http://127.0.0.1:8080",
            "http://[::1]:8080",
        ] {
            assert!(RemoteClient::new(valid, "token").is_ok(), "{valid}");
        }
    }

    #[test]
    fn remote_url_preserves_base_path() {
        let client = RemoteClient::new("https://example.com/api", "token").unwrap();
        assert_eq!(
            client.endpoint("sync/pull").unwrap().as_str(),
            "https://example.com/api/sync/pull"
        );
    }

    #[test]
    fn status_error_classifies_auth_and_policy() {
        let mut error = HttpStatusError {
            operation: "push".to_owned(),
            status: 401,
            error_class: String::new(),
            error_code: String::new(),
            message: "unauthorized".to_owned(),
        };
        assert!(error.is_auth_failure());
        error.status = 409;
        assert!(error.is_policy_failure());
    }
}
