//! The MCP server: every tool an agent can call, and the envelope around it.
//!
//! Tool errors are `rmcp::model::CallToolResult`, which is the protocol's own
//! type and not one Leteo gets to shrink. It crossed clippy's 128-byte
//! threshold when `serde_json` gained `preserve_order` — an `IndexMap` is
//! wider than the sorted map it replaced — and the lint fires on all
//! twenty-eight tool signatures at once.
//!
//! Boxing them would put a `Box<CallToolResult>` in the return type of every
//! tool and at every `?` that produces one, to save copying two hundred bytes
//! on a path taken once per tool call. The signatures are what somebody reads
//! to learn this module; the two hundred bytes are not worth obscuring them.
#![allow(clippy::result_large_err)]

use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex, MutexGuard},
};

use rmcp::{
    Json, ServerHandler, ServiceExt, handler::server::wrapper::Parameters, model::CallToolResult,
    schemars, tool, tool_handler, tool_router,
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

/// Tools an AI agent uses during a coding session.
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
    // The middle of the three-layer retrieval pattern: search finds an id,
    // timeline shows what surrounded it, and get_observation opens it whole.
    // The other two layers are here, and without this one an agent can find a
    // decision but not what led to it.
    "mem_timeline",
    "mem_unpin",
    "mem_update",
];

/// Tools for manual curation, dashboards, and the terminal UI.
pub const PROFILE_ADMIN: &[&str] = &["mem_delete", "mem_merge_projects", "mem_stats"];

/// Process-level MCP configuration supplied by the host command.
#[derive(Debug, Clone, Default)]
pub struct McpOptions {
    /// Trusted project override for this process. It is applied before
    /// directory detection but never overrides a session's own project.
    pub default_project: Option<String>,
    /// Comma-separated profile and tool names. Empty or `all` registers every
    /// tool.
    pub tools: Option<String>,
}

/// Resolves a profile and tool specification into the set of tools to expose.
/// `None` means every tool stays registered.
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
            // A name that is neither a profile nor a tool used to be kept as if
            // it were one, so it matched nothing and every route was removed:
            // `--tools=agnet` started a memory server with no memory tools on
            // it, in silence, and `--tools=AGENT` did the same. The symptom is
            // "Leteo's tools are missing", which the skill tells an agent to
            // fix by reinstalling — a typo sending somebody to reinstall a
            // working install.
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
    /// Trusted process-level project, already normalized.
    default_project: Option<String>,
    recovery: Arc<Mutex<RecoveryTokens>>,
    /// The prompt this process last recorded, so a save made while answering it
    /// can say what it was answering.
    prompt_context: Arc<Mutex<Option<PromptContext>>>,
    router: rmcp::handler::server::router::tool::ToolRouter<Self>,
}

/// The prompt currently being answered, as far as this process knows.
///
/// It is deliberately process-local and never persisted: it exists only to link
/// a memory to the request that produced it, and a stale link is worse than
/// none, so it is scoped to the project and session that recorded it.
#[derive(Debug, Clone)]
struct PromptContext {
    sync_id: String,
    project: String,
    session_id: String,
}

impl PromptContext {
    /// Whether this prompt belongs to the same work as the save being made.
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

    /// Removes the `format` keywords JSON Schema does not define.
    ///
    /// `schemars` describes a Rust `usize` as `"format": "uint"`, an `i64` as
    /// `"int64"` and an `f64` as `"double"`. None of those are registered JSON
    /// Schema formats — `uint` is not even an OpenAPI one — and a client that
    /// validates strictly rejects them. OpenCode reports `unknown format
    /// "uint"` on every tool that takes a limit.
    ///
    /// Nothing is lost by dropping them. `format` is an annotation, not a
    /// constraint, and the schema already says everything that matters: the
    /// type is `integer`, and `usize` also carries `"minimum": 0`. What the
    /// keyword added was a Rust type name leaking into a wire format that has
    /// no word for it.
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
            // And the output schemas, which is where most of them are: a tool
            // takes two or three numbers and hands back a dozen. Sanitising
            // only the input halved the problem and left OpenCode reporting the
            // rest — `unknown format "uint"` on `#/properties/duplicates`.
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

    /// The store, or the one refusal on this surface that nothing can retry.
    ///
    /// A poisoned lock means something panicked while holding the store, so
    /// every call after it fails the same way for as long as the process
    /// lives. The message said "the Leteo store lock is poisoned", which is
    /// the state in Rust's words and not a thing anybody can do: an agent
    /// reading it retries, gets it again, and reports that memory is broken.
    ///
    /// Every other refusal here carries its own remedy — a busy store says to
    /// call again in a moment, a replay without its token says which token —
    /// and this one is the only kind where the remedy is not the caller's at
    /// all. So it says whose it is.
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

    /// Resolves the session a write belongs to, enforcing that the project is
    /// backed by real context instead of an invented name.
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

    /// Decides which project a sessionless write targets.
    ///
    /// Detection stays authoritative. An explicit project is honored only when
    /// it matches the detected project, the process override, or a project the
    /// store already knows. Ambiguous directories require the agent to replay
    /// the user's choice with the recovery token from the previous error.
    /// Returns the project and the authority it came from, so the response
    /// envelope can tell the agent why this project was chosen.
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

    /// Accepts a project the user picked after an `ambiguous_project` error,
    /// but only when the agent replays a valid, unexpired recovery token.
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

    /// Builds the detection failure envelope, issuing a recovery token when the
    /// directory holds more than one candidate project.
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

/// Lifetime of an ambiguous-project recovery token.
const RECOVERY_TOKEN_TTL: chrono::TimeDelta = chrono::TimeDelta::minutes(5);

/// The only accepted value of `project_choice_reason`.
pub const SOURCE_USER_SELECTED_AFTER_AMBIGUOUS_PROJECT: &str =
    "user_selected_after_ambiguous_project";
/// The project came from the session the caller named.
pub const SOURCE_SESSION_PROJECT: &str = "session_project";
/// The project was requested explicitly and the store already knows it.
pub const SOURCE_KNOWN_PROJECT: &str = "known_project";
/// The caller asked for this project on a read.
pub const SOURCE_REQUEST: &str = "request";
/// A read that deliberately spans every project.
pub const SOURCE_ALL_PROJECTS: &str = "all_projects";

/// Tells the agent which project a result belongs to and why, so a memory can
/// never silently land in, or come from, a bucket the agent did not expect.
#[derive(Debug, Clone, Default, Serialize, schemars::JsonSchema)]
pub struct ProjectEnvelope {
    project: String,
    project_source: String,
    /// Where the project lives, when the answer came from a directory.
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
    /// Envelope for a read whose scope the caller chose.
    ///
    /// `fallback` is the project a read narrows to when the caller named none,
    /// with the name of where it came from — an override given on the command
    /// line, or the directory this server was started in. `None` is every
    /// project, which is what an explicit widening asks for and what an
    /// undetectable directory leaves.
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

/// A user project choice replayed by the agent after an ambiguous_project error.
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

    /// Consumes a token for one project choice. The same token may be replayed
    /// for the same project, but never for a different one, another directory,
    /// a changed candidate list, or a name that was never on it.
    fn redeem(&mut self, token: &str, choice: &str, detection: &ProjectDetection) -> bool {
        self.prune();
        let Some(entry) = self.entries.get_mut(token.trim()) else {
            return false;
        };
        let available: BTreeSet<String> = detection.available_projects.iter().cloned().collect();
        if entry.available_projects != available || entry.path != detection.path {
            return false;
        }
        // And the choice has to be one the token was issued over.
        //
        // `resolve_ambiguous_project` checks this before it gets here, and is
        // the only caller — so today this is belt and braces. It is here
        // because the entry holds the list and the check costs a lookup: a
        // rule enforced only by a caller is a rule one new caller away from not
        // existing, which is what put the same guard on `mem_update` after
        // `mem_save` had it alone.
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

#[tool_handler(router = self.router)]
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

/// Removes those keywords wherever they appear, however deeply nested.
/// Keeps the summary sentence of every field description and drops the rest.
///
/// `schemars` builds these from the Rust doc comments, so whatever is written
/// above a serialized field is shipped to every client that lists the tools.
/// Those comments are written for whoever maintains this: they argue about
/// `Option` against a skipped `String`, name Rust types, and carry intra-doc
/// links that arrive as literal brackets. Measured on the running server, 42 of
/// 130 field descriptions were paragraphs of that.
///
/// The first paragraph is the summary by Rust's own convention, and it is the
/// part written for a reader — "What language to write and search memories in",
/// "What the graph says about this memory, when anything does". Keeping only
/// that took `tools/list` from 56,862 bytes to under 48,000, which is about
/// 2,200 tokens off every client connection, and left the descriptions saying
/// what the field is rather than why it is typed the way it is.
///
/// Sanitised here rather than by rewriting forty doc comments: the rule is
/// "agents get the summary, maintainers get the whole thing", and a rule
/// enforced at the boundary cannot be forgotten by the next comment somebody
/// writes.
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

/// The first paragraph of a doc comment, on one line.
fn summary_of(description: &str) -> String {
    description
        .split("\n\n")
        .next()
        .unwrap_or(description)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Lets a tool's schema describe the answer it gives when it fails, too.
///
/// A failure comes back as `structuredContent` — that is what carries
/// `error.code`, the `available_projects` an ambiguous directory offers and the
/// `recovery_token` an agent has to replay — and it carries none of the fields
/// the success shape declares required. So a client that validates
/// `structuredContent` against `outputSchema` rejects every error this server
/// returns: driven through the built binary, twelve error replies out of twelve
/// failed their own tool's schema, each on the first required field of the
/// answer they are not.
///
/// That client is not hypothetical. OpenCode validates, and two defects on this
/// surface were found by it doing so — a `format` JSON Schema has never defined,
/// and a field the reply may omit declared required. Both were about the answer;
/// this is the same defect about the refusal, which is the half an agent most
/// needs to read, because the recovery flows live in it.
///
/// The union is expressed through `required` alone, so the root keeps its
/// `type` and its `properties` for anything that reads a schema rather than
/// validating against it. It costs about sixty bytes a tool in `tools/list`.
fn allow_the_error_shape(value: &mut serde_json::Value) {
    let serde_json::Value::Object(schema) = value else {
        return;
    };
    // Only where the success shape demands something. A schema with no
    // required fields already accepts an error envelope.
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

/// Guidance sent to MCP clients during initialization.
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

/// Run the Leteo MCP server with a process-level project override and an
/// optional tool profile selection.
pub async fn run_stdio_with_options(
    store: Arc<Mutex<Store>>,
    options: McpOptions,
) -> anyhow::Result<()> {
    // Refused here rather than served empty: a server with no tools answers
    // every question by not being asked, and nothing about it says why.
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

/// How much of a memory a listing shows before it costs more than it is worth.
///
/// A list is for choosing, not for reading. Returning every memory in full made
/// one ten-result `mem_search` cost 11,341 tokens against Engram's 2,026 for the
/// same ten memories — the bodies in that single response ran to 2,294, 3,243,
/// 874, 6,170, 1,174, 7,374, 769, 1,644, 8,726 and 4,445 characters, and the
/// agent had asked which of them was relevant, not to read all ten.
///
/// The skill already tells the agent to follow a search with
/// `mem_get_observation` for anything that looks right, so the whole text was
/// always one call away; sending it unasked billed for it twice.
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

// The same numbers the markdown context uses, read from where it reads them.
//
// They were written out again here, a third hand-written copy of two constants
// that already lived in `recall`. They agreed on the day this was noticed, and
// that is the whole hazard: `REVIEW_WINDOWS` and `KINDS` were each consolidated
// in this codebase after a second copy drifted, and `policy` spent a release
// with a review window nothing could ever fire because of it.
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
    /// Whether this landed in the per-project bucket rather than in a session
    /// somebody named.
    ///
    /// The difference decides what a memory may be attributed to. A named
    /// session is one conversation, and a question asked in another one is not
    /// what this memory answers — there is a test that says so. The bucket is
    /// not a conversation at all: it is where every save that named nothing
    /// goes, and prompts are never written to it, so the only question it could
    /// ever be linked to is one asked elsewhere.
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
    // Another writer holding the lock is the one store failure with a next
    // step, and it deserves to be told apart from the store being broken.
    //
    // Leteo is multi-writer by design — the hooks, this server, the CLI and
    // the background sync all open the same file — so a save landing while a
    // hook is writing is ordinary rather than exceptional. What the agent used
    // to get was `store_error: database is locked`, which is SQLite's sentence
    // about itself: nothing in it says the memory was not written, and nothing
    // says that asking again is the whole remedy. Measured with the lock held,
    // a `mem_save` waits its five seconds and then says exactly that.
    //
    // Every other refusal in this file names what to do next. This one now
    // does too.
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
        StoreError::EngramDatabase => "engram_database",
        // The caller's mistake, not the store's, so it must not be reported as
        // a store failure the agent can do nothing about.
        StoreError::InvalidParameter(_) => error_code::INVALID_PARAMS,
        StoreError::Database(_) | StoreError::Io(_) | StoreError::Json(_) => "store_error",
    };
    structured_error(code, error.to_string())
}

/// The `error.code` values MCP tools return.
///
/// Agents branch on these and the memory skill documents them, so they are a
/// contract rather than prose. The tests below keep asserting the literals on
/// purpose: if both sides went through these constants, renaming one would
/// change what goes out on the wire without failing anything.
mod error_code {
    pub const INVALID_PARAMS: &str = "invalid_params";
    /// Another writer holds the lock: nothing happened, and asking again works.
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

/// Builds a structured error carrying extra machine-readable context.
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

/// Reports that an explicit project is not backed by any known context.
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

/// The same failure without a recovery token, for the door that needs none.
///
/// There are two functions of this name — this one and the method above — and
/// which one a call site reaches depends only on whether it has a `self`.
/// `resolve_detected_project` is free, so `mem_session_start` reached this one
/// and answered an ambiguous directory with `project_detection_failed`: a code
/// that says detection is broken for a directory where nothing is broken, and
/// the one code the server instructions tell an agent to recognise so it can
/// ask the user. It named the candidates and then hid what they were for.
///
/// The code is the ambiguous one here too, because that is what happened. What
/// differs is the remedy, and each says its own: a write has to prove the user
/// was asked, with the token the method issues, while creating a session is the
/// sanctioned way to introduce a project — it takes the name directly, one of
/// these or a new one, which is why no token is minted for it.
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
// What a search answer means when it is empty or partial, said the same way on
// both surfaces: the tool puts it in the answer, `leteo search` on stderr.
pub(crate) use output::{
    ELSEWHERE_CAP, MORE_MATCHED_HINT, NO_MATCH_HINT, PARTIAL_MATCH_HINT, UNFILED_KIND_HINT,
    clamped_hint, no_match_here_hint,
};
use params::*;

#[cfg(test)]
mod tests;
