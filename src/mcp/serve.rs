//! The local HTTP API: the same tools, over a loopback socket.
//!
//! A script or a plugin that already speaks HTTP reaches the store without
//! becoming an MCP client or forking a `leteo` process per call. The endpoint
//! set is the MCP tool surface — `POST /tools/<name>` with that tool's own
//! arguments — so there is one name and one argument shape per operation
//! rather than a second REST vocabulary to keep in step, and every write takes
//! the same path the MCP and CLI callers take because it calls the same
//! handlers.
//!
//! The transport is HTTP/1.1 written here rather than pulled in. `axum` is in
//! the tree only behind the off-by-default `cloud-server` feature, and the
//! release profile's account of startup cost is an account of *not* linking a
//! framework into a binary that mostly runs one-shot hooks; a loopback socket
//! that answers a request line, a `Content-Length` and a JSON body does not
//! need one. What it does need is to be closed: the header and body are both
//! bounded, and anything past the bound is refused rather than buffered.

use std::net::SocketAddr;

use anyhow::Context;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::*;

/// Where `leteo serve` listens unless `--bind` says otherwise.
///
/// The loopback, because the store holds everything the agent has remembered
/// and the process is otherwise reachable by anything on the network. The port
/// is the one Engram's `engram serve` uses, so a script written against one
/// reaches the other.
pub const DEFAULT_BIND: &str = "127.0.0.1:7437";

/// The most request headers this will read before refusing the request, in
/// bytes. A well-formed request is a few hundred; the bound is what keeps a
/// client that never sends the blank line from growing the buffer without end.
const MAX_HEADER_BYTES: usize = 16 * 1024;

/// The largest request body this will read, in bytes.
///
/// A memory is text, and the store's own bound on one is far below this; the
/// number is here so a body claiming a gigabyte is refused before it is read
/// rather than after.
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// How long a connection has to deliver its whole request, head and body.
///
/// One deadline for the request rather than one per read, so a client that
/// drips a byte at a time holds a connection no longer than one that sends
/// nothing. Without it a socket that never sent a byte was held open for as long
/// as the client cared to keep it, each one costing the server a task.
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// How long a connection has to take its whole response once it has been
/// written to the socket.
///
/// The same ten seconds as the request, deliberately: a client that stops
/// draining holds the connection exactly as long as one that stops sending. A
/// plain `write_all` waited on a reader that never read for as long as that
/// reader cared to keep the socket open, once the response outgrew the kernel's
/// send buffer.
const WRITE_TIMEOUT: std::time::Duration = READ_TIMEOUT;

/// How many connections are served at once; the next one is answered `503` and
/// closed without a task.
///
/// Each connection is a task, a buffer of up to `MAX_HEADER_BYTES` plus
/// `MAX_BODY_BYTES`, and possibly a blocking-pool thread, so without a bound a
/// loop of idle sockets costs memory in proportion to how many the client
/// cares to open. The loopback API serves one agent and its scripts; a
/// hundred and twenty-eight is far past what they run at once.
const MAX_CONNECTIONS: usize = 128;

/// The three bounds one connection lives under, gathered so a test can shrink
/// them without a second copy of the code that applies them.
#[derive(Clone, Copy)]
struct Bounds {
    read_timeout: std::time::Duration,
    write_timeout: std::time::Duration,
    max_connections: usize,
}

impl Bounds {
    const PUBLISHED: Self = Self {
        read_timeout: READ_TIMEOUT,
        write_timeout: WRITE_TIMEOUT,
        max_connections: MAX_CONNECTIONS,
    };
}

pub async fn run(server: LeteoMcpServer, bind: &str) -> anyhow::Result<()> {
    let listener = TcpListener::bind(bind)
        .await
        .with_context(|| format!("bind {bind}"))?;
    let address = listener.local_addr()?;
    announce(address);
    serve(listener, server).await
}

/// Say which address the socket actually took, and say plainly when it is one
/// anybody on the network can reach.
fn announce(address: SocketAddr) {
    eprintln!("{}", listening_message(address));
}

fn listening_message(address: SocketAddr) -> String {
    if address.ip().is_loopback() {
        format!("leteo serve: listening on http://{address}")
    } else {
        format!(
            "leteo serve: listening on http://{address} — this is not the loopback, so any \
             process that can reach this address can read and write every memory"
        )
    }
}

async fn serve(listener: TcpListener, server: LeteoMcpServer) -> anyhow::Result<()> {
    accept_forever(|| listener.accept(), api(server), Bounds::PUBLISHED).await
}

/// The shared state one server answers from: the handlers and the tool list
/// this process exposes.
fn api(server: LeteoMcpServer) -> Arc<Api> {
    let tools = server
        .router
        .list_all()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect();
    Arc::new(Api { server, tools })
}

/// How long the accept loop waits before retrying a failed `accept`.
///
/// The retry is paced rather than immediate so a descriptor table that stays
/// full does not spin a core while answering nothing.
const ACCEPT_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(20);

/// Accept and answer connections forever, retrying a failed `accept` rather
/// than ending.
///
/// A failed `accept` on a listener this process created and never closes is
/// environmental: a client that went away before the kernel handed the socket
/// over, a descriptor table with no room left, a transient buffer shortage.
/// None of those is the listener's own failure, and ending `leteo serve` over
/// one takes down a socket every other caller shares. So the loop logs and
/// retries, and the delay above bounds the cost of a failure that persists.
/// `TooManyOpenFiles` is named in prose rather than matched on: its
/// `ErrorKind` is still unstable, and retrying everything covers it.
async fn accept_forever<F, Fut>(mut accept: F, api: Arc<Api>, bounds: Bounds) -> anyhow::Result<()>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = std::io::Result<(TcpStream, SocketAddr)>>,
{
    let permits = Arc::new(tokio::sync::Semaphore::new(bounds.max_connections));
    loop {
        let (stream, _peer) = match accept().await {
            Ok(accepted) => accepted,
            Err(error) => {
                eprintln!("leteo serve: {error}");
                tokio::time::sleep(ACCEPT_RETRY_DELAY).await;
                continue;
            }
        };
        let Ok(permit) = Arc::clone(&permits).try_acquire_owned() else {
            refuse_connection(stream);
            continue;
        };
        let api = Arc::clone(&api);
        tokio::spawn(async move {
            let _permit = permit;
            if let Err(error) = handle(stream, api, bounds).await {
                // A connection that failed mid-request is the client's, not
                // the server's: the socket is already gone, and the process
                // keeps answering the next one.
                eprintln!("leteo serve: {error}");
            }
        });
    }
}

/// Answer a connection past `MAX_CONNECTIONS` `503` and close it, without
/// awaiting anything.
///
/// The answer is a single non-blocking write on the accept loop's own thread,
/// not a task: a task per refused connection would be the unbounded spawn the
/// bound exists to prevent, and awaiting the write here would let one reader
/// that never reads hold up every accept behind it. A fresh socket's send
/// buffer takes a hundred bytes, and one that does not is simply dropped. What
/// the client already sent is drained first, because closing a socket with
/// unread data resets the connection and a reset can discard the answer.
fn refuse_connection(stream: TcpStream) {
    let Ok(mut socket) = stream.into_std() else {
        return;
    };
    let mut unread = [0_u8; 4096];
    let _ = std::io::Read::read(&mut socket, &mut unread);
    let _ = std::io::Write::write(&mut socket, &render(&Reply::too_many_connections()));
}

struct Api {
    server: LeteoMcpServer,
    /// The tool names this process exposes. `--tools` is a startup flag, so
    /// the set is fixed for the process's lifetime and the endpoint set is the
    /// same list `tools/list` publishes.
    tools: std::collections::BTreeSet<String>,
}

struct Request {
    method: String,
    target: String,
    body: Vec<u8>,
}

struct Reply {
    status: u16,
    body: serde_json::Value,
}

impl Reply {
    fn ok(body: serde_json::Value) -> Self {
        Self { status: 200, body }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: 400,
            body: serde_json::json!({
                "error": { "code": "invalid_request", "message": message.into() },
            }),
        }
    }

    fn request_timeout() -> Self {
        Self {
            status: 408,
            body: serde_json::json!({
                "error": {
                    "code": "request_timeout",
                    "message": "the request did not arrive in full in time",
                },
            }),
        }
    }

    fn too_many_connections() -> Self {
        Self {
            status: 503,
            body: serde_json::json!({
                "error": {
                    "code": "too_many_connections",
                    "message": "the server is serving as many connections as it allows; retry",
                },
            }),
        }
    }

    fn internal() -> Self {
        Self {
            status: 500,
            body: serde_json::json!({
                "error": {
                    "code": "internal_error",
                    "message": "the tool call did not finish",
                },
            }),
        }
    }

    fn not_found(target: &str) -> Self {
        Self {
            status: 404,
            body: serde_json::json!({
                "error": {
                    "code": "not_found",
                    "message": format!("no endpoint or tool is named {target:?}"),
                },
            }),
        }
    }

    fn method_not_allowed() -> Self {
        Self {
            status: 405,
            body: serde_json::json!({
                "error": {
                    "code": "method_not_allowed",
                    "message": "the endpoints are GET /health, GET /tools and POST /tools/<name>",
                },
            }),
        }
    }
}

async fn handle(mut stream: TcpStream, api: Arc<Api>, bounds: Bounds) -> std::io::Result<()> {
    let read = tokio::time::timeout(bounds.read_timeout, read_request(&mut stream)).await;
    let reply = match read {
        Ok(Ok(Some(request))) => route(api, request).await,
        // A connection that closed before sending a request line is a probe,
        // not an error.
        Ok(Ok(None)) => return Ok(()),
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::InvalidData => {
            Reply::bad_request(error.to_string())
        }
        Ok(Err(error)) => return Err(error),
        Err(_) => Reply::request_timeout(),
    };
    write_response(&mut stream, reply, bounds.write_timeout).await
}

async fn read_request(stream: &mut TcpStream) -> std::io::Result<Option<Request>> {
    let mut buffer = Vec::with_capacity(1024);
    let headers_end = loop {
        if let Some(index) = header_end(&buffer) {
            break index;
        }
        if buffer.len() > MAX_HEADER_BYTES {
            return Err(invalid("the request headers are too large"));
        }
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(None);
        }
        buffer.extend_from_slice(&chunk[..read]);
    };
    let head = std::str::from_utf8(&buffer[..headers_end])
        .map_err(|_| invalid("the request headers are not UTF-8"))?;
    // The loop above refuses an unterminated block that grows past the bound.
    // This refuses a *terminated* one: a blank line that arrives in the same
    // read that carries the block past the bound would otherwise be accepted,
    // because the loop looks for the blank line before it weighs the buffer.
    if headers_end > MAX_HEADER_BYTES {
        return Err(invalid("the request headers are too large"));
    }
    let mut lines = head.split("\r\n");
    let request_line = lines.next().ok_or_else(|| invalid("no request line"))?;
    let mut parts = request_line.split(' ');
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default().to_owned();
    if method.is_empty() || target.is_empty() {
        return Err(invalid("the request line is malformed"));
    }

    let mut content_length = 0_usize;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value
                .trim()
                .parse()
                .map_err(|_| invalid("the Content-Length is not a number"))?;
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            // A body that arrives chunked is refused rather than dropped. The
            // parser reads by `Content-Length` alone, so a chunked body would
            // be answered as if none had been sent: a tool with required
            // arguments fails with a confusing missing-field refusal, and one
            // whose arguments are all optional runs with defaults and reports
            // success, with nothing telling the caller its body was lost.
            return Err(invalid(
                "the request body must carry Content-Length; chunked transfer encoding is not \
                 supported",
            ));
        }
    }
    if content_length > MAX_BODY_BYTES {
        return Err(invalid("the request body is too large"));
    }

    // The body begins right after the blank line, and may already have arrived
    // in the same read as the headers.
    let mut body = buffer[headers_end + 4..].to_vec();
    while body.len() < content_length {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(invalid(
                "the request body ended before Content-Length bytes",
            ));
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(content_length);
    Ok(Some(Request {
        method,
        target,
        body,
    }))
}

/// Where the header block ends: the offset of the `\r` that opens the blank
/// line, or `None` while the blank line has not arrived yet.
fn header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn invalid(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}

async fn route(api: Arc<Api>, request: Request) -> Reply {
    let path = request
        .target
        .split_once('?')
        .map_or(request.target.as_str(), |(path, _)| path);
    match (request.method.as_str(), path) {
        ("GET", "/health") => Reply::ok(serde_json::json!({
            "status": "ok",
            "version": crate::build_info::version(),
        })),
        ("GET", "/tools") => Reply::ok(serde_json::json!({
            "tools": api.tools.iter().cloned().collect::<Vec<_>>(),
        })),
        ("POST", path) if path.starts_with("/tools/") => {
            let name = path["/tools/".len()..].to_owned();
            if !api.tools.contains(&name) {
                return Reply::not_found(&name);
            }
            // The handlers are synchronous and wait on the store's lock and on
            // SQLite's busy timeout, so a call made on a runtime worker held
            // that worker for the whole wait and every connection scheduled on
            // it stood still behind one busy store.
            let body = request.body;
            tokio::task::spawn_blocking(move || call_tool(&api.server, &name, &body))
                .await
                .unwrap_or_else(|_| Reply::internal())
        }
        ("GET" | "POST", _) => Reply::not_found(path),
        _ => Reply::method_not_allowed(),
    }
}

/// One tool call, over the same handler the MCP server routes to.
///
/// The body is that tool's arguments object, exactly as MCP's `arguments`
/// carries it, so `deny_unknown_fields` and every other refusal reach an HTTP
/// caller unchanged. A tool-level error keeps the MCP shape — `error.code`,
/// `error.message`, `available_projects`, `recovery_token` — and is answered
/// `400`, because the request was understood and refused rather than
/// mis-addressed — except the two codes `error_status` names, which are the
/// server's own condition and answer `503`.
fn call_tool(server: &LeteoMcpServer, name: &str, body: &[u8]) -> Reply {
    let arguments = if body.is_empty() {
        serde_json::Value::Object(serde_json::Map::new())
    } else {
        match serde_json::from_slice::<serde_json::Value>(body) {
            Ok(value @ serde_json::Value::Object(_)) => value,
            Ok(_) => return Reply::bad_request("the request body must be a JSON object"),
            Err(error) => {
                return Reply::bad_request(format!("the request body is not JSON: {error}"));
            }
        }
    };
    match name {
        "mem_save" => invoke(arguments, |params| server.mem_save(Parameters(params))),
        "mem_update" => invoke(arguments, |params| server.mem_update(Parameters(params))),
        "mem_review" => invoke(arguments, |params| server.mem_review(Parameters(params))),
        "mem_suggest_topic_key" => invoke(arguments, |params| {
            server.mem_suggest_topic_key(Parameters(params))
        }),
        "mem_delete" => invoke(arguments, |params| server.mem_delete(Parameters(params))),
        "mem_consolidate" => invoke(arguments, |params| {
            server.mem_consolidate(Parameters(params))
        }),
        "mem_search" => invoke(arguments, |params| server.mem_search(Parameters(params))),
        "mem_get_observation" => invoke(arguments, |params| {
            server.mem_get_observation(Parameters(params))
        }),
        "mem_context" => invoke(arguments, |params| server.mem_context(Parameters(params))),
        "mem_save_prompt" => invoke(arguments, |params| {
            server.mem_save_prompt(Parameters(params))
        }),
        "mem_session_start" => invoke(arguments, |params| {
            server.mem_session_start(Parameters(params))
        }),
        "mem_session_end" => invoke(arguments, |params| {
            server.mem_session_end(Parameters(params))
        }),
        "mem_pin" => invoke(arguments, |params| server.mem_pin(Parameters(params))),
        "mem_unpin" => invoke(arguments, |params| server.mem_unpin(Parameters(params))),
        "mem_timeline" => invoke(arguments, |params| server.mem_timeline(Parameters(params))),
        "mem_session_summary" => invoke(arguments, |params| {
            server.mem_session_summary(Parameters(params))
        }),
        "mem_capture_passive" => invoke(arguments, |params| {
            server.mem_capture_passive(Parameters(params))
        }),
        "mem_merge_projects" => invoke(arguments, |params| {
            server.mem_merge_projects(Parameters(params))
        }),
        "mem_current_project" => invoke(arguments, |params| {
            server.mem_current_project(Parameters(params))
        }),
        "mem_doctor" => invoke(arguments, |params| server.mem_doctor(Parameters(params))),
        "mem_stats" => invoke(arguments, |params| server.mem_stats(Parameters(params))),
        "mem_judge" => invoke(arguments, |params| server.mem_judge(Parameters(params))),
        "mem_compare" => invoke(arguments, |params| server.mem_compare(Parameters(params))),
        _ => Reply::not_found(name),
    }
}

fn invoke<P, T>(
    arguments: serde_json::Value,
    call: impl FnOnce(P) -> Result<Json<T>, CallToolResult>,
) -> Reply
where
    P: serde::de::DeserializeOwned,
    T: Serialize,
{
    match serde_json::from_value::<P>(arguments) {
        Ok(params) => match call(params) {
            Ok(Json(value)) => {
                Reply::ok(serde_json::to_value(value).unwrap_or(serde_json::Value::Null))
            }
            Err(error) => tool_reply(&error),
        },
        Err(error) => Reply::bad_request(format!("the arguments are not valid: {error}")),
    }
}

fn tool_reply(result: &CallToolResult) -> Reply {
    let body = result_body(result);
    let status = if result.is_error == Some(true) {
        error_status(body["error"]["code"].as_str())
    } else {
        200
    };
    Reply { status, body }
}

/// The status a tool error is answered with, decided from its code.
///
/// The two codes that name a condition of the server and not of the request
/// are `503`, so a client can tell a call worth repeating from one it got
/// wrong. No `Retry-After` goes with them: a busy store has already waited its
/// whole `busy_timeout` before answering, and when the other writer lets go is
/// not something this process knows; a store whose lock was poisoned stays that
/// way until the process is restarted, so no delay would be true.
fn error_status(code: Option<&str>) -> u16 {
    match code {
        Some(error_code::STORE_BUSY | error_code::STORE_UNAVAILABLE) => 503,
        _ => 400,
    }
}

fn result_body(result: &CallToolResult) -> serde_json::Value {
    if let Some(value) = &result.structured_content {
        return value.clone();
    }
    if let Some(text) = result.content.first().and_then(|block| block.as_text())
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text.text)
    {
        return value;
    }
    serde_json::json!({})
}

fn render(reply: &Reply) -> Vec<u8> {
    let body = serde_json::to_vec(&reply.body).unwrap_or_else(|_| b"{}".to_vec());
    let mut bytes = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reply.status,
        reason(reply.status),
        body.len(),
    )
    .into_bytes();
    bytes.extend_from_slice(&body);
    bytes
}

async fn write_response(
    stream: &mut (impl tokio::io::AsyncWrite + Unpin),
    reply: Reply,
    deadline: std::time::Duration,
) -> std::io::Result<()> {
    let bytes = render(&reply);
    let write = async {
        stream.write_all(&bytes).await?;
        stream.flush().await
    };
    tokio::time::timeout(deadline, write)
        .await
        .unwrap_or_else(|_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "the client did not read the response in time",
            ))
        })
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "OK",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::StoreConfig;

    /// A real socket on an ephemeral port, with the store beside it. The
    /// `TempDir` is handed back so it outlives the server task the caller
    /// leaves running.
    async fn spawn_api(project: &str) -> (SocketAddr, tempfile::TempDir) {
        let (server, temp) = test_server(project);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(serve(listener, server));
        (address, temp)
    }

    fn test_server(project: &str) -> (LeteoMcpServer, tempfile::TempDir) {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(StoreConfig::new(temp.path().join("http.db"))).unwrap();
        let server = LeteoMcpServer::with_options(
            Arc::new(Mutex::new(store)),
            McpOptions {
                default_project: Some(project.to_owned()),
                tools: None,
            },
        );
        (server, temp)
    }

    /// One hand-written request over a fresh connection, answering the whole
    /// response as text. `reqwest` fills in `Content-Length` itself, so the
    /// malformed, chunked and oversized cases have to be driven at the socket.
    async fn raw_request(address: SocketAddr, request: &str) -> String {
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        // Closing the write half is what makes an incomplete request finish:
        // the server reads to EOF instead of waiting for a blank line that
        // will never come, so a bound that fails to fire is a wrong answer
        // rather than a hung test.
        stream.shutdown().await.unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        String::from_utf8_lossy(&response).into_owned()
    }

    #[tokio::test]
    async fn a_save_over_http_is_reachable_by_a_search() {
        let (address, _temp) = spawn_api("http-test").await;
        let client = reqwest::Client::new();

        let saved: serde_json::Value = client
            .post(format!("http://{address}/tools/mem_save"))
            .json(&serde_json::json!({
                "title": "Written over HTTP",
                "content": "a body that reached the store through the local API",
                "type": "decision",
            }))
            .send()
            .await
            .expect("the save is answered")
            .json()
            .await
            .expect("the save answers JSON");
        assert_eq!(saved["status"], "inserted", "save answered: {saved}");
        assert!(
            saved["observation"]["id"].is_i64(),
            "the save names the memory it wrote: {saved}"
        );

        let found: serde_json::Value = client
            .post(format!("http://{address}/tools/mem_search"))
            .json(&serde_json::json!({
                "query": "reached the store",
                "all_projects": true,
            }))
            .send()
            .await
            .expect("the search is answered")
            .json()
            .await
            .expect("the search answers JSON");
        assert_eq!(found["count"], 1, "search answered: {found}");
        assert_eq!(found["results"][0]["title"], "Written over HTTP");
    }

    #[tokio::test]
    async fn a_refusal_is_a_bad_request_and_an_unknown_tool_is_not_found() {
        let (address, _temp) = spawn_api("http-test").await;
        let client = reqwest::Client::new();

        // `mem_save` with no content is a tool-level refusal, and the caller
        // gets the same error code MCP returns.
        let refused = client
            .post(format!("http://{address}/tools/mem_save"))
            .json(&serde_json::json!({ "title": "no content" }))
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status(), 400);
        let body: serde_json::Value = refused.json().await.unwrap();
        assert_eq!(body["error"]["code"], "invalid_params", "refused: {body}");

        let missing = client
            .post(format!("http://{address}/tools/mem_nothing"))
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(missing.status(), 404);

        let health: serde_json::Value = client
            .get(format!("http://{address}/health"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(health["status"], "ok");
    }

    #[test]
    fn the_default_bind_is_the_loopback() {
        let address: SocketAddr = DEFAULT_BIND.parse().expect("the default bind parses");
        assert_eq!(address.port(), 7437);
        assert!(
            address.ip().is_loopback(),
            "the default bind must not be reachable from the network: {DEFAULT_BIND}"
        );
    }

    #[test]
    fn a_bind_that_is_not_the_loopback_says_so() {
        let loopback = listening_message("127.0.0.1:7437".parse().unwrap());
        assert!(loopback.contains("127.0.0.1:7437"));
        assert!(
            !loopback.contains("not the loopback"),
            "the loopback needs no warning: {loopback}"
        );

        let exposed = listening_message("0.0.0.0:7437".parse().unwrap());
        assert!(exposed.contains("0.0.0.0:7437"));
        assert!(
            exposed.contains("not the loopback"),
            "a bind anybody can reach has to say so: {exposed}"
        );
    }

    #[tokio::test]
    async fn a_chunked_body_is_refused_rather_than_dropped() {
        let (address, _temp) = spawn_api("http-test").await;
        let response = raw_request(
            address,
            "POST /tools/mem_save HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n\
             5\r\nhello\r\n0\r\n\r\n",
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 400"),
            "a chunked body has to be refused, not read as empty: {response}"
        );
        assert!(
            response.contains("chunked transfer encoding"),
            "the refusal names the chunked body rather than a missing field: {response}"
        );
    }

    #[tokio::test]
    async fn the_header_and_body_bounds_are_refused() {
        let (address, _temp) = spawn_api("http-test").await;

        // Headers past the bound, with no blank line: refused before the
        // buffer grows without end.
        let filler = "x".repeat(MAX_HEADER_BYTES + 1);
        let response = raw_request(
            address,
            &format!("GET /health HTTP/1.1\r\nHost: localhost\r\nX-Filler: {filler}\r\n"),
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 400"),
            "oversized headers: {response}"
        );
        assert!(
            response.contains("headers are too large"),
            "the header bound, not a truncated read: {response}"
        );

        // And the same block *terminated*: the blank line arrives in the read
        // that carries the block past the bound, which the loop's own check
        // cannot see because it looks for the blank line first.
        let response = raw_request(
            address,
            &format!("GET /health HTTP/1.1\r\nHost: localhost\r\nX-Filler: {filler}\r\n\r\n"),
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 400"),
            "a terminated block past the bound: {response}"
        );
        assert!(
            response.contains("headers are too large"),
            "the terminated header bound: {response}"
        );

        // A `Content-Length` past the bound: refused before the body is read,
        // so a claim of a gigabyte costs nothing.
        let response = raw_request(
            address,
            &format!(
                "POST /tools/mem_save HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n",
                MAX_BODY_BYTES + 1
            ),
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 400"),
            "oversized body: {response}"
        );
        assert!(
            response.contains("body is too large"),
            "the body bound, not a body that ended early: {response}"
        );

        // A body that ends before the length it declared: the client sends
        // four bytes and shuts down, so the read reaches EOF short of the ten
        // it promised.
        let response = raw_request(
            address,
            "POST /tools/mem_save HTTP/1.1\r\nHost: localhost\r\nContent-Length: 10\r\n\r\nnope",
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 400"),
            "a body that ends early: {response}"
        );
        assert!(
            response.contains("ended before Content-Length"),
            "the short-body refusal: {response}"
        );
    }

    #[tokio::test]
    async fn a_non_json_body_and_a_bad_method_are_refused() {
        let (address, _temp) = spawn_api("http-test").await;

        let response = raw_request(
            address,
            "POST /tools/mem_save HTTP/1.1\r\nHost: localhost\r\nContent-Length: 4\r\n\r\nnope",
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 400"),
            "a body that is not JSON: {response}"
        );
        assert!(
            response.contains("not JSON"),
            "the refusal names the body rather than a missing field: {response}"
        );

        let response = raw_request(
            address,
            "DELETE /health HTTP/1.1\r\nHost: localhost\r\n\r\n",
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 405"),
            "a method that is neither GET nor POST: {response}"
        );
        assert!(
            response.contains("method_not_allowed"),
            "the refusal names the method: {response}"
        );
    }

    #[tokio::test]
    async fn every_tool_the_list_advertises_can_be_called() {
        let (address, _temp) = spawn_api("http-test").await;
        let client = reqwest::Client::new();

        let listed: serde_json::Value = client
            .get(format!("http://{address}/tools"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let tools = listed["tools"].as_array().expect("the list is an array");
        assert!(!tools.is_empty(), "the list is empty: {listed}");

        // The advertised set and the dispatchable set are one list: a tool the
        // router exposes but the dispatch `match` omits would be named here and
        // answer 404.
        for tool in tools {
            let name = tool.as_str().unwrap();
            let status = client
                .post(format!("http://{address}/tools/{name}"))
                .json(&serde_json::json!({}))
                .send()
                .await
                .unwrap()
                .status();
            assert_ne!(
                status.as_u16(),
                404,
                "{name} is advertised by /tools but not dispatched"
            );
        }
    }

    /// A server whose store gives up on a held write lock after `patience`,
    /// with the session the writes below name already in it.
    fn impatient_server(
        patience: std::time::Duration,
    ) -> (LeteoMcpServer, tempfile::TempDir, std::path::PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("http.db");
        let mut config = StoreConfig::new(path.clone());
        config.busy_timeout = patience;
        let mut store = Store::open(config).unwrap();
        store.enroll_project("http-test").unwrap();
        store.create_session("s1", "http-test", "dir").unwrap();
        let server = LeteoMcpServer::with_options(
            Arc::new(Mutex::new(store)),
            McpOptions {
                default_project: Some("http-test".to_owned()),
                tools: None,
            },
        );
        (server, temp, path)
    }

    async fn spawn_server(server: LeteoMcpServer) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(serve(listener, server));
        address
    }

    #[tokio::test]
    async fn a_store_held_by_another_writer_is_service_unavailable() {
        let (server, _temp, path) = impatient_server(std::time::Duration::from_millis(50));
        let holder = rusqlite::Connection::open(&path).unwrap();
        holder
            .execute_batch("BEGIN IMMEDIATE; UPDATE sessions SET directory = 'held';")
            .unwrap();
        let address = spawn_server(server).await;

        let refused = reqwest::Client::new()
            .post(format!("http://{address}/tools/mem_save"))
            .json(&serde_json::json!({
                "session_id": "s1", "title": "While held", "content": "the body of it",
            }))
            .send()
            .await
            .unwrap();
        let status = refused.status();
        let body: serde_json::Value = refused.json().await.unwrap();
        assert_eq!(body["error"]["code"], "store_busy", "refused: {body}");
        assert_eq!(
            status, 503,
            "a busy store is the server's condition: {body}"
        );
    }

    #[tokio::test]
    async fn a_store_that_cannot_be_reached_is_service_unavailable() {
        let (server, _temp) = test_server("http-test");
        let store = Arc::clone(&server.store);
        let _ = std::thread::spawn(move || {
            let _held = store.lock().unwrap();
            panic!("poison the store's lock");
        })
        .join();
        let address = spawn_server(server).await;

        let refused = reqwest::Client::new()
            .post(format!("http://{address}/tools/mem_stats"))
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        let status = refused.status();
        let body: serde_json::Value = refused.json().await.unwrap();
        assert_eq!(
            body["error"]["code"], "store_unavailable",
            "refused: {body}"
        );
        assert_eq!(status, 503, "{body}");
    }

    /// The accept loop under bounds small enough to reach.
    async fn spawn_bounded(bounds: Bounds) -> (SocketAddr, tempfile::TempDir) {
        let (server, temp) = test_server("http-test");
        let listener = Arc::new(TcpListener::bind("127.0.0.1:0").await.unwrap());
        let address = listener.local_addr().unwrap();
        tokio::spawn(accept_forever(
            move || {
                let listener = Arc::clone(&listener);
                async move { listener.accept().await }
            },
            api(server),
            bounds,
        ));
        (address, temp)
    }

    async fn spawn_impatient_reader(
        read_timeout: std::time::Duration,
    ) -> (SocketAddr, tempfile::TempDir) {
        spawn_bounded(Bounds {
            read_timeout,
            ..Bounds::PUBLISHED
        })
        .await
    }

    #[tokio::test]
    async fn a_client_that_never_reads_is_dropped_at_the_write_deadline() {
        // An in-memory pipe whose far end is held and never read. A loopback
        // socket was tried first and failed on Windows: 64 MiB written to a
        // peer that never read was accepted whole, so the write finished and
        // no deadline was met. The pipe's capacity is a number this test
        // chooses, not one a kernel does.
        let (mut stream, _never_reads) = tokio::io::duplex(64);
        let reply = Reply::ok(serde_json::json!({ "padding": "x".repeat(4096) }));

        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            write_response(&mut stream, reply, std::time::Duration::from_millis(100)),
        )
        .await
        .expect("the write gave up instead of waiting for the reader");
        assert_eq!(
            outcome
                .expect_err("the response cannot fit in the buffers")
                .kind(),
            std::io::ErrorKind::TimedOut
        );
    }

    #[tokio::test]
    async fn a_connection_past_the_bound_is_answered_service_unavailable() {
        let bounds = Bounds {
            max_connections: 2,
            ..Bounds::PUBLISHED
        };
        let (address, _temp) = spawn_bounded(bounds).await;
        // Connections are accepted in the order they were made, so these two
        // hold both permits before the third exists.
        let mut held = Vec::new();
        for _ in 0..bounds.max_connections {
            held.push(TcpStream::connect(address).await.unwrap());
        }

        let mut extra = TcpStream::connect(address).await.unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            extra.read_to_end(&mut response),
        )
        .await
        .expect("the connection past the bound was answered")
        .unwrap();
        let response = String::from_utf8_lossy(&response);
        assert!(
            response.starts_with("HTTP/1.1 503"),
            "a connection past the bound is the server's condition: {response}"
        );
        assert!(response.contains("too_many_connections"), "{response}");
    }

    #[tokio::test]
    async fn a_finished_connection_gives_its_place_back() {
        let bounds = Bounds {
            read_timeout: std::time::Duration::from_millis(50),
            max_connections: 1,
            ..Bounds::PUBLISHED
        };
        let (address, _temp) = spawn_bounded(bounds).await;
        let mut idle = TcpStream::connect(address).await.unwrap();
        let mut response = Vec::new();
        idle.read_to_end(&mut response).await.unwrap();
        assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 408"));

        // The place is returned a moment after the socket closes, so a 503
        // here is retried; the loop ends only on an answer that is not one,
        // and a leaked place would answer 503 for ever.
        let client = reqwest::Client::new();
        let mut status = 503;
        for _ in 0..200 {
            // A refused connection can be reset under a client that was still
            // writing its request, which reads as an error here, not a 503.
            status = match client.get(format!("http://{address}/health")).send().await {
                Ok(response) => response.status().as_u16(),
                Err(_) => 503,
            };
            if status != 503 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert_eq!(status, 200, "the place was never given back");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn a_tool_waiting_on_the_store_does_not_hold_a_runtime_worker() {
        let (server, _temp) = test_server("http-test");
        let store = Arc::clone(&server.store);
        let address = spawn_server(server).await;
        let held = store.lock().unwrap();

        // The call blocks on the store's lock until `held` is dropped below.
        // On a runtime with one worker, a call made on that worker would leave
        // nothing to answer the health check with.
        let mut waiting = std::net::TcpStream::connect(address).unwrap();
        std::io::Write::write_all(
            &mut waiting,
            b"POST /tools/mem_stats HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}",
        )
        .unwrap();
        // Blocking sleep and client below: when the worker is held the
        // runtime's own timers stop with it, so a tokio sleep or timeout here
        // would hang the test instead of failing it.
        std::thread::sleep(std::time::Duration::from_millis(200));
        let mut probe = std::net::TcpStream::connect(address).unwrap();
        probe
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        std::io::Write::write_all(&mut probe, b"GET /health HTTP/1.1\r\n\r\n").unwrap();
        let mut answer = Vec::new();
        let read = std::io::Read::read_to_end(&mut probe, &mut answer);
        drop(held);
        read.expect("the health check was answered while a tool waited on the store");
        assert!(
            String::from_utf8_lossy(&answer).starts_with("HTTP/1.1 200"),
            "{}",
            String::from_utf8_lossy(&answer)
        );
    }

    #[tokio::test]
    async fn a_connection_that_sends_nothing_is_closed() {
        let (address, _temp) = spawn_impatient_reader(std::time::Duration::from_millis(200)).await;
        let mut stream = TcpStream::connect(address).await.unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            stream.read_to_end(&mut response),
        )
        .await
        .expect("the server closed an idle connection")
        .unwrap();
        let response = String::from_utf8_lossy(&response);
        assert!(
            response.starts_with("HTTP/1.1 408"),
            "an idle connection is told why it was closed: {response}"
        );
    }

    #[tokio::test]
    async fn a_request_that_stalls_part_way_is_closed_too() {
        let (address, _temp) = spawn_impatient_reader(std::time::Duration::from_millis(200)).await;
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream
            .write_all(b"POST /tools/mem_save HTTP/1.1\r\nContent-Length: 50\r\n\r\n{")
            .await
            .unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            stream.read_to_end(&mut response),
        )
        .await
        .expect("the server closed a connection that stopped mid-body")
        .unwrap();
        let response = String::from_utf8_lossy(&response);
        assert!(
            response.starts_with("HTTP/1.1 408"),
            "a stalled body is a timeout, not a short body: {response}"
        );
    }

    #[tokio::test]
    async fn the_loop_keeps_accepting_after_a_failed_accept() {
        let (server, _temp) = test_server("http-test");
        let bound = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = bound.local_addr().unwrap();
        // The test holds no handle on the listener: it moves into the loop, so
        // a loop that returned would close the socket and the request below
        // would fail fast instead of hanging.
        let accepted_from = Arc::new(bound);
        // The first `accept` fails, as one does when a client goes away before
        // the kernel hands the socket over; every later one is real.
        let mut inject = true;
        tokio::spawn(accept_forever(
            move || {
                let fail = std::mem::replace(&mut inject, false);
                let listener = Arc::clone(&accepted_from);
                async move {
                    if fail {
                        Err(std::io::Error::from(std::io::ErrorKind::ConnectionAborted))
                    } else {
                        listener.accept().await
                    }
                }
            },
            api(server),
            Bounds::PUBLISHED,
        ));

        // A request answered after the injected failure means the loop went on
        // rather than returning.
        let health: serde_json::Value = reqwest::Client::new()
            .get(format!("http://{address}/health"))
            .send()
            .await
            .expect("the server answers after a failed accept")
            .json()
            .await
            .unwrap();
        assert_eq!(health["status"], "ok");
    }
}
