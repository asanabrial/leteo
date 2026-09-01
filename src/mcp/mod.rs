//! The MCP server: every tool an agent can call, and the envelope around it.
//!
//! Tool errors are `rmcp::model::CallToolResult`, which is the protocol's own
//! type and not one Leteo gets to shrink. It crossed clippy's 128-byte
//! threshold when `serde_json` gained `preserve_order` — an `IndexMap` is
//! wider than the sorted map it replaced — and the lint fires on all
//! twenty-eight tool signatures at once.
#![allow(clippy::result_large_err)]

use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex, MutexGuard},
};

use rmcp::{
    Json, ServerHandler, ServiceExt, handler::server::wrapper::Parameters, model::CallToolResult,
    schemars, tool, tool_router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    memory::model::{
        AddObservation, AddOutcome, AddOutcomeKind, AddPrompt, Candidate, CandidateOptions,
        DoctorReport, ForeignKeyViolation, JudgeBySemanticParams, JudgeRelationParams, MergeResult,
        Observation, PassiveCapture, PassiveCaptureResult, Prompt, Relation, SearchMode,
        SearchOptions, SearchResult, Session, SessionSummary, Stats, TimelineEntry, TimelineResult,
        UpdateObservation,
    },
    memory::normalize,
    project::{ProjectDetection, detect_current_project, detect_project},
    store::{Store, StoreError, suggest_topic_key},
};

pub const PROFILE_AGENT: &[&str] = &[
    "mem_capture_passive",
    "mem_compare",
    "mem_context",
    "mem_current_project",
    "mem_doctor",
    "mem_get_observation",
    "mem_judge",
    "mem_pin",
    "mem_review",
    "mem_save",
    "mem_save_prompt",
    "mem_search",
    "mem_session_end",
    "mem_session_start",
    "mem_session_summary",
    "mem_suggest_topic_key",
    "mem_timeline",
    "mem_unpin",
    "mem_update",
];

pub const PROFILE_ADMIN: &[&str] = &["mem_delete", "mem_merge_projects", "mem_stats"];

#[derive(Debug, Clone, Default)]
pub struct McpOptions {
    pub default_project: Option<String>,
    pub tools: Option<String>,
}

pub fn resolve_tools(specification: &str) -> Result<Option<BTreeSet<String>>, String> {
    let specification = specification.trim();
    if specification.is_empty() || specification == "all" {
        return Ok(None);
    }
    let mut tools = BTreeSet::new();
    for token in specification.split(',').map(str::trim) {
        match token {
            "" => continue,
            "all" => return Ok(None),
            "agent" => tools.extend(PROFILE_AGENT.iter().map(|tool| (*tool).to_owned())),
            "admin" => tools.extend(PROFILE_ADMIN.iter().map(|tool| (*tool).to_owned())),
            tool if PROFILE_AGENT.contains(&tool) || PROFILE_ADMIN.contains(&tool) => {
                tools.insert(tool.to_owned());
            }
            other => {
                let mut known: Vec<&str> = PROFILE_AGENT
                    .iter()
                    .chain(PROFILE_ADMIN.iter())
                    .copied()
                    .collect();
                known.sort_unstable();
                return Err(format!(
                    "unknown tool or profile {other:?}; the profiles are agent, admin and all, and the tools are {}",
                    known.join(", ")
                ));
            }
        }
    }
    Ok(if tools.is_empty() { None } else { Some(tools) })
}

#[derive(Clone)]
struct LeteoMcpServer {
    store: Arc<Mutex<Store>>,
    default_project: Option<String>,
    recovery: Arc<Mutex<RecoveryTokens>>,
    prompt_context: Arc<Mutex<Option<PromptContext>>>,
    router: rmcp::handler::server::router::tool::ToolRouter<Self>,
}

#[derive(Debug, Clone)]
struct PromptContext {
    sync_id: String,
    project: String,
    session_id: String,
}

impl PromptContext {
    fn matches(&self, project: &str, session_id: &str) -> bool {
        self.project == project && self.session_id == session_id
    }
}

impl LeteoMcpServer {
    fn with_options(store: Arc<Mutex<Store>>, options: McpOptions) -> Self {
        let mut router = Self::router();
        if let Some(allowed) = options
            .tools
            .as_deref()
            .and_then(|specification| resolve_tools(specification).ok())
            .flatten()
        {
            for tool in router
                .list_all()
                .into_iter()
                .map(|tool| tool.name.to_string())
            {
                if !allowed.contains(&tool) {
                    router.remove_route(&tool);
                }
            }
        }
        Self::drop_nonstandard_formats(&mut router);
        Self {
            store,
            default_project: options
                .default_project
                .as_deref()
                .map(normalize::project)
                .filter(|project| !project.is_empty()),
            recovery: Arc::new(Mutex::new(RecoveryTokens::default())),
            prompt_context: Arc::new(Mutex::new(None)),
            router,
        }
    }

    fn drop_nonstandard_formats(
        router: &mut rmcp::handler::server::router::tool::ToolRouter<Self>,
    ) {
        for route in router.map.values_mut() {
            let mut schema = serde_json::Value::Object((*route.attr.input_schema).clone());
            strip_numeric_formats(&mut schema);
            summarise_descriptions(&mut schema);
            if let serde_json::Value::Object(schema) = schema {
                route.attr.input_schema = Arc::new(schema);
            }
            if let Some(existing) = route.attr.output_schema.as_ref() {
                let mut schema = serde_json::Value::Object((**existing).clone());
                strip_numeric_formats(&mut schema);
                summarise_descriptions(&mut schema);
                allow_the_error_shape(&mut schema);
                if let serde_json::Value::Object(schema) = schema {
                    route.attr.output_schema = Some(Arc::new(schema));
                }
            }
        }
    }

    fn lock_store(&self) -> Result<MutexGuard<'_, Store>, CallToolResult> {
        self.store.lock().map_err(|_| {
            structured_error(
                error_code::STORE_UNAVAILABLE,
                "this Leteo server cannot reach its store any more, because something failed \
                 while holding it; every call will fail the same way until it is restarted. \
                 Retrying will not help - say so rather than reporting that memory is empty.",
            )
        })
    }

    fn write_session(
        &self,
        store: &mut Store,
        session_id: Option<String>,
        project: Option<String>,
        choice: ProjectChoice,
    ) -> Result<WriteSession, CallToolResult> {
        if let Some(id) = session_id.filter(|id| !id.trim().is_empty()) {
            let session = store.get_session(&id).map_err(store_error)?;
            let session_project = normalize::project(&session.project);
            if let Some(project) = project {
                let project = normalize::project(&project);
                if project.is_empty() {
                    return Err(structured_error(
                        error_code::INVALID_PROJECT,
                        crate::project::EMPTY_NAME,
                    ));
                }
                if project != session_project {
                    return Err(structured_error(
                        error_code::SESSION_PROJECT_MISMATCH,
                        format!(
                            "session {id:?} belongs to project {session_project:?}, not {project:?}"
                        ),
                    ));
                }
            }
            let envelope = ProjectEnvelope {
                project: session_project.clone(),
                project_source: SOURCE_SESSION_PROJECT.to_owned(),
                project_path: Some(session.directory).filter(|path| !path.is_empty()),
            };
            return Ok(WriteSession {
                id,
                project: session_project,
                envelope,
                named: true,
            });
        }

        let detection = detect_current_project();
        let (project, project_source) =
            self.resolve_write_project(store, project, &detection, choice)?;
        let id = manual_session_id(&project);
        let directory = if detection.path.is_empty() {
            std::env::current_dir()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            detection.path
        };
        store
            .create_session(&id, &project, &directory)
            .map_err(store_error)?;
        let envelope = ProjectEnvelope {
            project: project.clone(),
            project_source,
            project_path: Some(directory).filter(|path| !path.is_empty()),
        };
        Ok(WriteSession {
            id,
            project,
            envelope,
            named: false,
        })
    }

    fn resolve_write_project(
        &self,
        store: &Store,
        requested: Option<String>,
        detection: &ProjectDetection,
        choice: ProjectChoice,
    ) -> Result<(String, String), CallToolResult> {
        let detected = normalize::project(&detection.project);
        let requested = requested.map(|project| normalize::project(&project));

        let Some(requested) = requested else {
            if let Some(default_project) = &self.default_project {
                return Ok((
                    default_project.clone(),
                    crate::project::SOURCE_PROCESS_OVERRIDE.to_owned(),
                ));
            }
            if !detected.is_empty() {
                return Ok((detected, detection.source.clone()));
            }
            return Err(self.project_detection_error(detection));
        };
        if requested.is_empty() {
            return Err(structured_error(
                error_code::INVALID_PROJECT,
                crate::project::EMPTY_NAME,
            ));
        }
        if detected.is_empty() {
            let project = self.accept_ambiguous_choice(&requested, detection, choice)?;
            return Ok((
                project,
                SOURCE_USER_SELECTED_AFTER_AMBIGUOUS_PROJECT.to_owned(),
            ));
        }
        if requested == detected {
            return Ok((requested, detection.source.clone()));
        }
        if self
            .default_project
            .as_deref()
            .is_some_and(|default_project| default_project == requested)
        {
            return Ok((
                requested,
                crate::project::SOURCE_PROCESS_OVERRIDE.to_owned(),
            ));
        }
        let known = store.project_exists(&requested).map_err(store_error)?;
        if known {
            return Ok((requested, SOURCE_KNOWN_PROJECT.to_owned()));
        }
        Err(unknown_project_error(
            &requested,
            &detected,
            store.list_project_names().unwrap_or_default(),
        ))
    }

    fn accept_ambiguous_choice(
        &self,
        requested: &str,
        detection: &ProjectDetection,
        choice: ProjectChoice,
    ) -> Result<String, CallToolResult> {
        if detection.available_projects.is_empty() {
            return Err(self.project_detection_error(detection));
        }
        if choice.reason.as_deref() != Some(SOURCE_USER_SELECTED_AFTER_AMBIGUOUS_PROJECT) {
            return Err(self.project_detection_error(detection));
        }
        let matched = detection
            .available_projects
            .iter()
            .find(|available| normalize::project(available) == requested)
            .cloned();
        let Some(matched) = matched else {
            return Err(structured_error_with(
                error_code::INVALID_PROJECT_CHOICE,
                format!("{requested:?} is not one of the available projects"),
                json!({ "available_projects": detection.available_projects }),
            ));
        };
        let Some(token) = choice
            .recovery_token
            .filter(|token| !token.trim().is_empty())
        else {
            return Err(structured_error_with(
                error_code::RECOVERY_TOKEN_REQUIRED,
                "replaying a user project choice requires the recovery_token from the ambiguous_project error",
                json!({ "available_projects": detection.available_projects }),
            ));
        };
        let accepted = self
            .recovery
            .lock()
            .map(|mut tokens| tokens.redeem(&token, &matched, detection))
            .unwrap_or(false);
        if accepted {
            Ok(normalize::project(&matched))
        } else {
            Err(structured_error_with(
                error_code::INVALID_RECOVERY_TOKEN,
                "the recovery token is unknown, expired, or issued for another project choice",
                json!({ "available_projects": detection.available_projects }),
            ))
        }
    }

    fn project_detection_error(&self, detection: &ProjectDetection) -> CallToolResult {
        if detection.available_projects.is_empty() {
            return project_detection_error(detection);
        }
        let token = self
            .recovery
            .lock()
            .map(|mut tokens| tokens.issue(detection))
            .unwrap_or_default();
        CallToolResult::structured_error(json!({
            "error": {
                "code": error_code::AMBIGUOUS_PROJECT,
                "message": detection
                    .error_hint
                    .as_deref()
                    .unwrap_or("the current directory holds more than one project"),
            },
            "project_path": detection.path,
            "available_projects": detection.available_projects,
            "recovery_token": token,
            "recovery_instructions": format!(
                "ask the user which project this belongs to, then retry with project=<choice>, \
                 project_choice_reason={SOURCE_USER_SELECTED_AFTER_AMBIGUOUS_PROJECT}, and this recovery_token"
            ),
        }))
    }
}

const RECOVERY_TOKEN_TTL: chrono::TimeDelta = chrono::TimeDelta::minutes(5);

pub const SOURCE_USER_SELECTED_AFTER_AMBIGUOUS_PROJECT: &str =
    "user_selected_after_ambiguous_project";
pub const SOURCE_SESSION_PROJECT: &str = "session_project";
pub const SOURCE_KNOWN_PROJECT: &str = "known_project";
pub const SOURCE_REQUEST: &str = "request";
pub const SOURCE_ALL_PROJECTS: &str = "all_projects";

#[derive(Debug, Clone, Default, Serialize, schemars::JsonSchema)]
pub struct ProjectEnvelope {
    project: String,
    project_source: String,
    ///
    /// `Option` rather than a `String` skipped when empty. The two look alike
    /// from Rust and are not the same thing on the wire: serde omits an empty
    /// `String`, while `schemars` sees a plain `String` and marks it *required*
    /// in the output schema. Every read whose scope the caller chose has no
    /// path, so `mem_search` and `mem_context` sent a reply their own schema
    /// forbade — and a client that validates strictly rejected every call with
    /// `data must have required property 'project_path'`.
    #[serde(skip_serializing_if = "Option::is_none")]
    project_path: Option<String>,
}

impl ProjectEnvelope {
    fn for_read(requested: Option<&str>, fallback: Option<&(String, String)>) -> Self {
        match requested.map(normalize::project).filter(|p| !p.is_empty()) {
            Some(project) => Self {
                project,
                project_source: SOURCE_REQUEST.to_owned(),
                project_path: None,
            },
            None => match fallback {
                Some((project, source)) => Self {
                    project: project.clone(),
                    project_source: source.clone(),
                    project_path: None,
                },
                None => Self {
                    project: String::new(),
                    project_source: SOURCE_ALL_PROJECTS.to_owned(),
                    project_path: None,
                },
            },
        }
    }
}

#[derive(Debug, Default)]
struct ProjectChoice {
    reason: Option<String>,
    recovery_token: Option<String>,
}

#[derive(Debug, Default)]
struct RecoveryTokens {
    entries: std::collections::BTreeMap<String, RecoveryEntry>,
}

#[derive(Debug)]
struct RecoveryEntry {
    available_projects: BTreeSet<String>,
    path: String,
    expires_at: chrono::DateTime<chrono::Utc>,
    selected: Option<String>,
}

impl RecoveryTokens {
    fn issue(&mut self, detection: &ProjectDetection) -> String {
        self.prune();
        let token = normalize::sync_id("rec");
        self.entries.insert(
            token.clone(),
            RecoveryEntry {
                available_projects: detection.available_projects.iter().cloned().collect(),
                path: detection.path.clone(),
                expires_at: chrono::Utc::now() + RECOVERY_TOKEN_TTL,
                selected: None,
            },
        );
        token
    }

    fn redeem(&mut self, token: &str, choice: &str, detection: &ProjectDetection) -> bool {
        self.prune();
        let Some(entry) = self.entries.get_mut(token.trim()) else {
            return false;
        };
        let available: BTreeSet<String> = detection.available_projects.iter().cloned().collect();
        if entry.available_projects != available || entry.path != detection.path {
            return false;
        }
        if !entry.available_projects.contains(choice) {
            return false;
        }
        match &entry.selected {
            Some(selected) => selected == choice,
            None => {
                entry.selected = Some(choice.to_owned());
                true
            }
        }
    }

    fn prune(&mut self) {
        let now = chrono::Utc::now();
        self.entries.retain(|_, entry| entry.expires_at > now);
    }
}

impl LeteoMcpServer {
    fn set_pin(&self, id: i64, pinned: bool) -> Result<Json<PinOutput>, CallToolResult> {
        let mut store = self.lock_store()?;
        if pinned {
            store.pin_observation(id).map_err(store_error)?;
        } else {
            store.unpin_observation(id).map_err(store_error)?;
        }
        let observation = store.get_observation(id).map_err(store_error)?;
        Ok(Json(PinOutput {
            id: observation.id,
            sync_id: observation.sync_id,
            pinned: observation.pinned,
        }))
    }
}

/// How long a client may treat this process's `tools/list` as fresh, in
/// milliseconds (SEP-2549).
///
/// The list is fixed for the process's lifetime — the `--tools` profile is a
/// startup flag — so within one process any TTL is honest. What a long TTL
/// cannot survive is the process ending: a restart with a different flag
/// serves a different list, and a client still holding the old one would
/// answer tools from a server that no longer has them. Five minutes bounds
/// how far that mistake can travel while still keeping the list out of every
/// turn a re-listing client spends.
const TOOLS_LIST_TTL_MS: u64 = 5 * 60 * 1000;

impl ServerHandler for LeteoMcpServer {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        rmcp::model::ServerInfo::new(
            rmcp::model::ServerCapabilities::builder()
                .enable_tools()
                .build(),
        )
        .with_server_info(
            rmcp::model::Implementation::new("leteo", env!("CARGO_PKG_VERSION"))
                .with_title("Leteo"),
        )
        .with_instructions(SERVER_INSTRUCTIONS)
    }

    /// The tool list, with the cacheability a `2026-07-28` session requires.
    ///
    /// This replaces `#[tool_handler(router = self.router)]`, whose expansion
    /// filled `ttl_ms` and `cache_scope` with `None` and never serialised
    /// them: a client that negotiated `2026-07-28` (SEP-2549) rejects the
    /// result as missing two required fields and ends the connection with no
    /// tools at all. rmcp strips `resultType` for older peers but fills
    /// nothing for newer ones — compatibility was built backwards only — so
    /// the value is the server's to state.
    ///
    /// The fields are absent for older revisions rather than set-and-stripped:
    /// they did not exist there, and publishing them would rely on every
    /// legacy client tolerating a property its schema never named. The
    /// comparison is lexical for the same reason rmcp's own dispatch is — ISO
    /// dates compare the same either way.
    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::ListToolsResult, rmcp::ErrorData> {
        let mut result = rmcp::model::ListToolsResult {
            result_type: Some(rmcp::model::ResultType::COMPLETE),
            tools: self.router.list_all(),
            meta: None,
            next_cursor: None,
            ttl_ms: None,
            cache_scope: None,
        };
        if context.protocol_version().is_some_and(|version| {
            version.as_str() >= rmcp::model::ProtocolVersion::V_2026_07_28.as_str()
        }) {
            // `public`, not `private`: the list depends on the `--tools` flag
            // the process started with, never on who is asking, and a local
            // stdio server has one caller. There is no authorization context
            // to leak across.
            result.ttl_ms = Some(TOOLS_LIST_TTL_MS);
            result.cache_scope = Some(rmcp::model::CacheScope::Public);
        }
        Ok(result)
    }

    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::CallToolResponse, rmcp::ErrorData> {
        let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        self.router.call(tcc).await
    }

    fn get_tool(&self, name: &str) -> Option<rmcp::model::Tool> {
        self.router.get(name).cloned()
    }
}

/// The `format` values a numeric schema may carry that JSON Schema has never
/// defined, and which strict clients reject.
///
/// Listed rather than "anything on a number", so a format that is genuinely
/// registered — `date-time` on a string, say — is never touched by this.
const RUST_NUMERIC_FORMATS: &[&str] = &[
    "uint", "uint8", "uint16", "uint32", "uint64", "int8", "int16", "int32", "int64", "float",
    "double",
];

fn summarise_descriptions(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(description)) = map.get_mut("description") {
                *description = summary_of(description);
            }
            for nested in map.values_mut() {
                summarise_descriptions(nested);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                summarise_descriptions(item);
            }
        }
        _ => {}
    }
}

fn summary_of(description: &str) -> String {
    description
        .split("\n\n")
        .next()
        .unwrap_or(description)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A failure comes back as `structuredContent` — that is what carries
/// `error.code`, the `available_projects` an ambiguous directory offers and the
/// `recovery_token` an agent has to replay — and it carries none of the fields
/// the success shape declares required. So a client that validates
/// `structuredContent` against `outputSchema` rejects every error this server
/// returns: driven through the built binary, twelve error replies out of twelve
/// failed their own tool's schema, each on the first required field of the
/// answer they are not.
fn allow_the_error_shape(value: &mut serde_json::Value) {
    let serde_json::Value::Object(schema) = value else {
        return;
    };
    let Some(required) = schema.remove("required") else {
        return;
    };
    schema.insert(
        "anyOf".to_owned(),
        serde_json::json!([
            { "required": required },
            { "required": ["error"] },
        ]),
    );
}

fn strip_numeric_formats(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            if map
                .get("format")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|format| RUST_NUMERIC_FORMATS.contains(&format))
            {
                map.remove("format");
            }
            for nested in map.values_mut() {
                strip_numeric_formats(nested);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                strip_numeric_formats(item);
            }
        }
        _ => {}
    }
}

const SERVER_INSTRUCTIONS: &str = "\
Local-first persistent memory tools backed by the Leteo SQLite store.

Save decisions, discoveries, bug fixes, and conventions with mem_save as they \
happen. Start a session with mem_session_start and close it with \
mem_session_summary.

CONTEXT: if this session opened with a block of remembered work, you already \
have it and mem_context repeats it — measured on a real store, the opening \
block named fifty memories in 11 KB and mem_context answered with twenty of \
those same fifty in 22 KB. Call it when no such block arrived, when you have \
lost it mid-session, or for a project other than this one.

PROJECTS: writes belong to the project detected from the working directory, or \
to the project of an explicit session_id. An explicit project argument is \
accepted only when it matches that detection or names a project the store \
already holds; anything else fails with unknown_project. Create a genuinely new \
project with mem_session_start.

AMBIGUOUS DIRECTORIES: when a call returns error code ambiguous_project, the \
directory holds several projects. Ask the user which one this belongs to, then \
retry the same call with project=<their choice>, \
project_choice_reason=user_selected_after_ambiguous_project, and the \
recovery_token from that error. mem_session_start is the exception and its \
error says so: it takes project=<choice> on its own, because naming a project \
is what it is for. Never guess the project yourself.

CONFLICTS: when mem_save reports candidates, judge each one with mem_judge \
using its own judgment_id, in that same turn, and settle every verdict \
yourself — related, compatible, scoped, conflicts_with, supersedes, \
not_conflict. Never put one to the user and never report it: this is Leteo's \
bookkeeping, not work they asked for, and mem_judge replaces a verdict \
wholesale so a wrong one is a correction rather than damage. A pair left \
unjudged is not deferred, it is dropped: nothing raises it again until a \
session opens, which is why an opening that hands you pairs is handing you \
work to finish then.

SUMMARIES: mem_session_summary takes the session's own title from the first \
line of the content that is not a heading, so open with what the session was \
for. A summary beginning with a date is one nobody can find again.";

pub async fn run_stdio_with_options(
    store: Arc<Mutex<Store>>,
    options: McpOptions,
) -> anyhow::Result<()> {
    if let Some(specification) = options.tools.as_deref() {
        resolve_tools(specification).map_err(|why| anyhow::anyhow!("{why}"))?;
    }
    LeteoMcpServer::with_options(store, options)
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}

const fn default_capture_prompt() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Default, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum MatchMode {
    #[default]
    All,
    Any,
}

impl From<MatchMode> for SearchMode {
    fn from(value: MatchMode) -> Self {
        match value {
            MatchMode::All => Self::All,
            MatchMode::Any => Self::Any,
        }
    }
}

pub(crate) const PREVIEW_BYTES: usize = 400;

fn default_observation_type() -> String {
    "manual".to_owned()
}

fn default_scope() -> String {
    "project".to_owned()
}

fn default_review_limit() -> usize {
    10
}

fn default_timeline_window() -> usize {
    5
}

fn default_context_sessions() -> usize {
    crate::recall::RECENT_SESSIONS
}

fn default_context_prompts() -> usize {
    crate::recall::RECENT_PROMPTS
}

fn default_passive_source() -> String {
    "mcp-passive".to_owned()
}

fn nonempty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

struct WriteSession {
    id: String,
    project: String,
    envelope: ProjectEnvelope,
    named: bool,
}

fn resolve_detected_project(
    explicit_project: Option<String>,
    detection: &ProjectDetection,
) -> Result<String, CallToolResult> {
    if let Some(project) = explicit_project {
        let project = normalize::project(&project);
        if !project.is_empty() {
            return Ok(project);
        }
        return Err(structured_error(
            error_code::INVALID_PROJECT,
            crate::project::EMPTY_NAME,
        ));
    }
    let project = normalize::project(&detection.project);
    if !project.is_empty() {
        return Ok(project);
    }
    Err(project_detection_error(detection))
}

pub(crate) fn manual_session_id(project: &str) -> String {
    if project.is_empty() {
        "manual-save".to_owned()
    } else {
        format!("manual-save-{project}")
    }
}

fn outcome_label(kind: AddOutcomeKind) -> &'static str {
    match kind {
        AddOutcomeKind::Inserted => "inserted",
        AddOutcomeKind::Revised => "revised",
        AddOutcomeKind::Deduplicated => "deduplicated",
    }
}

fn store_error(error: StoreError) -> CallToolResult {
    if error.is_busy() {
        return structured_error(
            error_code::STORE_BUSY,
            format!(
                "another Leteo process is writing to the store right now, so this \
                 call did nothing and nothing was half-written: {error}. Try the \
                 same call again."
            ),
        );
    }
    let code = match &error {
        StoreError::SessionNotFound(_) => "session_not_found",
        StoreError::ObservationNotFound(_) => "observation_not_found",
        StoreError::ObservationDeleted { .. } => "observation_deleted",
        StoreError::RelationNotFound(_) => "relation_not_found",
        StoreError::InvalidRelationVerb { .. } => "invalid_relation",
        StoreError::CrossProjectRelation { .. } => "cross_project_relation",
        StoreError::ProjectNotFound(_) => "project_not_found",
        StoreError::SessionHasObservations(_, _) => "session_has_observations",
        StoreError::EmptySearch => "invalid_search",
        StoreError::RelativeDatabasePath(_) => "invalid_database_path",
        StoreError::PromptNotFound(_) => "prompt_not_found",
        StoreError::SchemaTooNew { .. } => "schema_too_new",
        StoreError::SchemaFromPreRelease { .. } => "schema_from_pre_release",
        StoreError::EngramDatabase => "engram_database",
        StoreError::InvalidParameter(_) => error_code::INVALID_PARAMS,
        StoreError::Database(_) | StoreError::Io(_) | StoreError::Json(_) => "store_error",
    };
    structured_error(code, error.to_string())
}

mod error_code {
    pub const INVALID_PARAMS: &str = "invalid_params";
    pub const STORE_BUSY: &str = "store_busy";
    pub const INVALID_PROJECT: &str = "invalid_project";
    pub const INVALID_PROJECT_CHOICE: &str = "invalid_project_choice";
    pub const INVALID_RECOVERY_TOKEN: &str = "invalid_recovery_token";
    pub const RECOVERY_TOKEN_REQUIRED: &str = "recovery_token_required";
    pub const SESSION_PROJECT_MISMATCH: &str = "session_project_mismatch";
    pub const PROJECT_MISMATCH: &str = "project_mismatch";
    pub const UNKNOWN_PROJECT: &str = "unknown_project";
    pub const AMBIGUOUS_PROJECT: &str = "ambiguous_project";
    pub const STORE_UNAVAILABLE: &str = "store_unavailable";
}

fn structured_error(code: &str, message: impl Into<String>) -> CallToolResult {
    CallToolResult::structured_error(json!({
        "error": {
            "code": code,
            "message": message.into(),
        }
    }))
}

fn structured_error_with(
    code: &str,
    message: impl Into<String>,
    context: serde_json::Value,
) -> CallToolResult {
    let mut payload = json!({
        "error": {
            "code": code,
            "message": message.into(),
        }
    });
    if let Some(object) = context.as_object() {
        for (key, value) in object {
            payload[key.as_str()] = value.clone();
        }
    }
    CallToolResult::structured_error(payload)
}

fn unknown_project_error(
    requested: &str,
    detected: &str,
    known_projects: Vec<String>,
) -> CallToolResult {
    structured_error_with(
        error_code::UNKNOWN_PROJECT,
        format!(
            "project {requested:?} is unknown here; this directory resolves to {detected:?}. \
             Pass an existing project, or create it with mem_session_start first."
        ),
        json!({
            "detected_project": detected,
            "available_projects": known_projects,
        }),
    )
}

fn project_detection_error(detection: &ProjectDetection) -> CallToolResult {
    if detection.available_projects.is_empty() {
        return CallToolResult::structured_error(json!({
            "error": {
                "code": "project_detection_failed",
                "message": detection
                    .error_hint
                    .as_deref()
                    .unwrap_or("cannot determine the current project"),
            },
            "project_path": detection.path,
            "available_projects": detection.available_projects,
        }));
    }
    CallToolResult::structured_error(json!({
        "error": {
            "code": error_code::AMBIGUOUS_PROJECT,
            "message": detection
                .error_hint
                .as_deref()
                .unwrap_or("the current directory holds more than one project"),
        },
        "project_path": detection.path,
        "available_projects": detection.available_projects,
        "recovery_instructions":
            "ask the user which project this belongs to, then retry with project=<choice> - \
             one of available_projects, or a new name; this call takes it directly and needs \
             no recovery_token",
    }))
}

mod output;
mod params;
mod tools;

use output::*;
pub(crate) use output::{
    ELSEWHERE_CAP, MORE_MATCHED_HINT, NO_MATCH_HINT, PARTIAL_MATCH_HINT, UNFILED_KIND_HINT,
    clamped_hint, no_match_here_hint,
};
use params::*;

#[cfg(test)]
mod tests;
