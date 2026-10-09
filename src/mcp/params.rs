//! What each tool accepts.
//!
//! One identifier, four names. `mem_get_observation`, `mem_delete`, `mem_pin`,
//! `mem_unpin` and `mem_update` take `id`; `mem_timeline` takes
//! `observation_id`; `mem_review` takes either; `mem_compare` takes
//! `memory_id_a` and `memory_id_b`. Sessions are the same story — every tool
//! that writes calls one `session_id`, and the two that open and close a
//! session call it `id`.
//!
//! An agent that learned one name from one tool spends a failed call finding
//! out about the next, and the error it gets back — *missing field `id`* — is
//! only useful because it happens to name the field. So the obvious guesses are
//! accepted as aliases, and each field says so.
//!
//! Saying so is the point. A `serde` alias is invisible to the schema and would
//! rescue a guess after it failed; the sentence in the description prevents the
//! guess. It costs about forty bytes a field — some three hundred across the
//! seven — against a whole failed call each time somebody guesses wrong.
//!
//! Renaming them into agreement would be the tidier fix and would break every
//! caller that already works.

use super::*;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct SaveParams {
    /// Session identifier. Defaults to manual-save-{project}.
    pub(super) session_id: Option<String>,
    /// Short searchable title.
    pub(super) title: String,
    /// Full observation content.
    pub(super) content: Option<String>,
    /// Backward-compatible alias for content.
    pub(super) observation: Option<String>,
    /// One of: bugfix, decision, policy, architecture, discovery, pattern,
    /// config, preference. The category is a search filter, and a word outside
    /// this list folds onto `discovery` on the way in, so the memory is still
    /// reachable by a filter. Close synonyms fold onto their own kind.
    #[serde(rename = "type", default = "default_observation_type")]
    pub(super) kind: String,
    /// Name of the tool that produced the observation.
    pub(super) tool_name: Option<String>,
    /// Project owning this observation. Accepted only when it matches the
    /// detected project, the process override, or a project the store knows.
    pub(super) project: Option<String>,
    /// Must be user_selected_after_ambiguous_project, and only after the user
    /// picked one of available_projects from an ambiguous_project error.
    pub(super) project_choice_reason: Option<String>,
    /// Short-lived token returned by an ambiguous_project error. Required with
    /// project_choice_reason.
    pub(super) recovery_token: Option<String>,
    /// Memory scope: project, personal, or global. A label for filtering later — a
    /// personal memory still belongs to this project and reads narrowed to
    /// another one will not return it.
    #[serde(default = "default_scope")]
    pub(super) scope: String,
    /// Stable key used to revise an evolving observation instead of inserting another.
    pub(super) topic_key: Option<String>,
    /// Link this memory to the question it answers: the prompt this process
    /// last recorded, else the session's last one, else — only when no
    /// session_id is given — the project's last from the past 30 minutes.
    /// Defaults to true. Pass false for automated saves that answer no user
    /// request.
    #[serde(default = "default_capture_prompt")]
    pub(super) capture_prompt: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct UpdateParams {
    /// Numeric observation identifier. Also accepted as `observation_id`.
    #[serde(alias = "observation_id")]
    pub(super) id: i64,
    /// The project this memory is expected to belong to before the update. A
    /// memory filed under a different one is refused with `project_mismatch`
    /// and left untouched.
    pub(super) expected_project: String,
    /// New observation category. One of: bugfix, decision, policy, architecture, discovery, pattern, config, preference.
    #[serde(rename = "type")]
    pub(super) kind: Option<String>,
    /// New title.
    pub(super) title: Option<String>,
    /// New content: the whole body. Refused together with `find`.
    pub(super) content: Option<String>,
    /// Exact text in the stored body to replace, once. The body is read and
    /// edited inside the write transaction, so this counts against what is
    /// stored. A `find` that is absent or matches more than once changes
    /// nothing and is refused as `edit_not_found` or `edit_ambiguous`. Covers
    /// the body, not the title. Refused together with `content`.
    pub(super) find: Option<String>,
    /// What replaces the one span `find` names. Absent means the span is
    /// removed. `replace` without `find` is refused.
    pub(super) replace: Option<String>,
    /// Project to move this memory into. `expected_project` names where it is
    /// now, and this names where it goes.
    pub(super) project: Option<String>,
    /// New scope: project, personal, or global.
    pub(super) scope: Option<String>,
    /// New topic key.
    pub(super) topic_key: Option<String>,
}

impl UpdateParams {
    pub(super) fn is_empty(&self) -> bool {
        self.kind.is_none()
            && self.title.is_none()
            && self.content.is_none()
            && self.find.is_none()
            && self.project.is_none()
            && self.scope.is_none()
            && self.topic_key.is_none()
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ReviewParams {
    /// Action: list or mark_reviewed.
    pub(super) action: String,
    /// Optional project filter for list.
    pub(super) project: Option<String>,
    /// Maximum list results.
    ///
    /// One at least: zero is a page with nothing on it, which is not a question
    /// anybody asks of a list — unlike the section budgets on `mem_context` and
    /// `mem_timeline`, where zero means leave that part out. `schemars` derives
    /// `minimum: 0` from `usize` and the store clamps to one, so the floor is
    /// published rather than discovered.
    ///
    /// And a ceiling by the same argument, which was the end this had open. The
    /// queue is the one list where a large number is the obvious thing to ask
    /// for — an opening block that says two hundred and sixty-nine memories are
    /// due invites asking for two hundred and sixty-nine — and it had no
    /// ceiling at all, neither applied nor published: a real store answered
    /// with all 269 in one reply of 444 KB. Twenty, from the store's own
    /// ceiling for a context read, because this hands memories over to be read
    /// rather than ranking an answer.
    #[schemars(range(min = 1, max = 20))]
    #[serde(default = "default_review_limit")]
    pub(super) limit: usize,
    /// Observation identifier for mark_reviewed.
    pub(super) observation_id: Option<i64>,
    /// Backward-compatible alias for observation_id.
    pub(super) id: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct SuggestTopicKeyParams {
    /// Observation category. One of: bugfix, decision, policy, architecture, discovery, pattern, config, preference.
    #[serde(rename = "type", default = "default_observation_type")]
    pub(super) kind: String,
    /// Preferred source for the topic segment.
    #[serde(default)]
    pub(super) title: String,
    /// Fallback source when title is empty.
    #[serde(default)]
    pub(super) content: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct DeleteParams {
    /// Numeric observation identifier. Also accepted as `observation_id`.
    #[serde(alias = "observation_id")]
    pub(super) id: i64,
    /// The project this memory is expected to belong to. A memory filed under
    /// a different one is refused with `project_mismatch` and left untouched.
    pub(super) expected_project: String,
    /// Permanently remove the row instead of soft-deleting it.
    #[serde(default)]
    pub(super) hard_delete: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ConsolidateParams {
    /// The project the memories being replaced are expected to belong to. A
    /// source filed under a different one is refused with `project_mismatch`
    /// and nothing is written.
    pub(super) expected_project: String,
    /// Numeric observation identifiers of the memories this one replaces. Each
    /// gets a judged `supersedes` relation from the new memory and stops
    /// appearing in search and context. At least one is required.
    #[schemars(length(min = 1))]
    pub(super) source_ids: Vec<i64>,
    /// Category of the replacement memory. One of: bugfix, decision, policy,
    /// architecture, discovery, pattern, config, preference.
    #[serde(rename = "type", default = "default_observation_type")]
    pub(super) kind: String,
    /// Short searchable title for the replacement memory.
    pub(super) title: String,
    /// Full body of the replacement memory.
    pub(super) content: String,
    /// Memory scope: project, personal, or global.
    #[serde(default = "default_scope")]
    pub(super) scope: String,
    /// Stable key used to revise an evolving observation instead of inserting another.
    pub(super) topic_key: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct SearchParams {
    /// Full-text query or an exact topic key containing a slash. At most 8192 bytes of raw query; a longer one is refused with `query_too_long` before it is tokenised.
    pub(super) query: String,
    /// Restrict results to this observation category. One of: bugfix, decision, policy, architecture, discovery, pattern, config, preference. Close
    /// synonyms are folded, so `bug` finds a `bugfix`. `session_summary` is also accepted, and is how a summary is found: the relaxed stages leave summaries out unless this names them.
    #[serde(rename = "type")]
    pub(super) kind: Option<String>,
    /// Restrict results to this project.
    pub(super) project: Option<String>,
    /// Search every project and ignore the project filter.
    #[serde(default)]
    pub(super) all_projects: bool,
    /// Restrict results to this memory scope: project, personal, or global.
    pub(super) scope: Option<String>,
    /// Maximum number of results. The store clamps this to its configured maximum.
    ///
    /// One at least: zero is a page with nothing on it, which is not a question
    /// anybody asks of a list — unlike the section budgets on `mem_context` and
    /// `mem_timeline`, where zero means leave that part out. `schemars` derives
    /// `minimum: 0` from `usize` and the store clamps to one, so the floor is
    /// published rather than discovered.
    ///
    /// The ceiling was applied and not published: the description says the
    /// store clamps to its configured maximum, and what that number is could
    /// only be found by asking for more and counting what came back. It is
    /// twenty, and now it says so.
    #[schemars(range(min = 1, max = 20))]
    pub(super) limit: Option<usize>,
    /// Require all query terms or allow any query term.
    #[serde(default)]
    pub(super) match_mode: MatchMode,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct GetObservationParams {
    /// Numeric observation identifier. Also accepted as `observation_id`.
    #[serde(alias = "observation_id")]
    pub(super) id: i64,
    /// Include the titles and bodies earlier writes replaced, newest first.
    /// Defaults to false. The versions come back whole, not previewed.
    #[serde(default)]
    pub(super) include_history: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ContextParams {
    /// Restrict recent observations to this project.
    pub(super) project: Option<String>,
    /// List every project and ignore the detected one.
    #[serde(default)]
    pub(super) all_projects: bool,
    /// Restrict observations to this scope: project, personal, or global.
    pub(super) scope: Option<String>,
    /// Maximum number of recent observations. Eighty at most, which is the
    /// deepest context Leteo itself is ever configured to open with.
    #[schemars(range(min = 0, max = 80))]
    pub(super) limit: Option<usize>,
    /// Maximum number of recent sessions. Twenty at most, the ceiling every
    /// list on this surface has.
    #[schemars(range(min = 0, max = 20))]
    #[serde(default = "default_context_sessions")]
    pub(super) session_limit: usize,
    /// Maximum number of recent user prompts. Twenty at most, the ceiling every
    /// list on this surface has.
    #[schemars(range(min = 0, max = 20))]
    #[serde(default = "default_context_prompts")]
    pub(super) prompt_limit: usize,
    /// Ceiling on the bytes this answer may carry. Also accepted as `max_bytes`.
    /// It only ever shrinks the reply below the size setting, never raises it.
    /// A value below the envelope every answer carries cannot be met, and the
    /// reply says so with `byte_limit_unmet`.
    #[schemars(range(min = CONTEXT_ENVELOPE_FLOOR, max = 49000))]
    #[serde(alias = "max_bytes")]
    pub(super) byte_limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct SavePromptParams {
    /// Session identifier. Defaults to manual-save-{project}.
    pub(super) session_id: Option<String>,
    /// Original user prompt text.
    pub(super) content: String,
    /// Project associated with the prompt. Accepted only when backed by known
    /// context or an ambiguous-project recovery.
    pub(super) project: Option<String>,
    /// Must be user_selected_after_ambiguous_project, and only after the user
    /// picked one of available_projects from an ambiguous_project error.
    pub(super) project_choice_reason: Option<String>,
    /// Short-lived token returned by an ambiguous_project error. Required with
    /// project_choice_reason.
    pub(super) recovery_token: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct SessionStartParams {
    /// Unique session identifier. Also accepted as `session_id`, which is what
    /// every tool that writes to a session calls it.
    #[serde(alias = "session_id")]
    pub(super) id: String,
    /// Optional explicit project; otherwise it is detected from directory or cwd.
    pub(super) project: Option<String>,
    /// Working directory for this session.
    pub(super) directory: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct SessionEndParams {
    /// Identifier of the session to end. Also accepted as `session_id`.
    #[serde(alias = "session_id")]
    pub(super) id: String,
    /// Optional concise session summary.
    pub(super) summary: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct PinParams {
    /// Numeric observation identifier. Also accepted as `observation_id`.
    #[serde(alias = "observation_id")]
    pub(super) id: i64,
    /// The project this memory is expected to belong to. A memory filed under
    /// a different one is refused with `project_mismatch` and left untouched.
    pub(super) expected_project: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct TimelineParams {
    /// Observation identifier at the center of the timeline. Also accepted as
    /// `id`, which is what the tools that fetch, pin or update one call it.
    #[serde(alias = "id")]
    pub(super) observation_id: i64,
    /// Number of observations before the focus.
    ///
    /// Twenty at most, which is the ceiling every list on this surface has and
    /// the only one that used to be missing: `before_total` and `after_total`
    /// say how much lies beyond it.
    #[schemars(range(min = 0, max = 20))]
    #[serde(default = "default_timeline_window")]
    pub(super) before: usize,
    /// Number of observations after the focus.
    ///
    /// Twenty at most, which is the ceiling every list on this surface has and
    /// the only one that used to be missing: `before_total` and `after_total`
    /// say how much lies beyond it.
    #[schemars(range(min = 0, max = 20))]
    #[serde(default = "default_timeline_window")]
    pub(super) after: usize,
    /// Accepted for upstream schema compatibility; timeline is session-scoped.
    pub(super) project: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct SessionSummaryParams {
    /// Structured session summary content.
    pub(super) content: String,
    /// Session identifier. Defaults to manual-save-{project}.
    pub(super) session_id: Option<String>,
    /// Optional explicit project. Accepted only when backed by known context or
    /// an ambiguous-project recovery.
    pub(super) project: Option<String>,
    /// Must be user_selected_after_ambiguous_project, and only after the user
    /// picked one of available_projects from an ambiguous_project error.
    pub(super) project_choice_reason: Option<String>,
    /// Short-lived token returned by an ambiguous_project error. Required with
    /// project_choice_reason.
    pub(super) recovery_token: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct CapturePassiveParams {
    /// Text ending in a Key Learnings section, in any of the thirteen languages
    /// Leteo writes memories in.
    pub(super) content: String,
    /// Session identifier. Defaults to manual-save-{project}.
    pub(super) session_id: Option<String>,
    /// Source identifier.
    #[serde(default = "default_passive_source")]
    pub(super) source: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct MergeProjectsParams {
    /// Comma-separated project names to merge from.
    pub(super) from: String,
    /// Canonical project name to merge into.
    pub(super) to: String,
}

/// A tool that takes nothing, and says so.
///
/// Two tools had no parameter type at all, and what that publishes is
/// `{"type":"object","properties":{}}` — an object schema with no
/// `additionalProperties: false`, which says extra fields are welcome. So
/// `mem_stats` accepted `project` and answered with the whole store's numbers,
/// and the caller had no way to know its narrowing had been dropped. Asking is
/// the natural mistake: every other read on this surface takes a project.
///
/// The other twenty tools refuse an unknown field and name the ones they take.
/// These two now do the same, which is the whole of what this type is for.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct NoParams {}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct DoctorParams {
    /// Project context to report; diagnostics remain store-wide.
    pub(super) project: Option<String>,
    /// Optional diagnostic check code. When given, only that check runs and
    /// the report carries it alone; an unknown code is refused.
    pub(super) check: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct JudgeParams {
    /// Relation sync ID returned as candidates[].judgment_id by mem_save.
    pub(super) judgment_id: String,
    /// How the two observations relate: related, compatible, scoped, conflicts_with, supersedes, or not_conflict.
    pub(super) relation: String,
    /// Optional explanation for the verdict.
    pub(super) reason: Option<String>,
    /// Optional JSON or text evidence.
    pub(super) evidence: Option<String>,
    /// Optional confidence score in the inclusive range 0.0..1.0.
    pub(super) confidence: Option<f64>,
    /// Optional session in which the verdict was made.
    pub(super) session_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct CompareParams {
    /// Integer ID of the first observation.
    pub(super) memory_id_a: i64,
    /// Integer ID of the second observation.
    pub(super) memory_id_b: i64,
    /// How the two observations relate: related, compatible, scoped, conflicts_with, supersedes, or not_conflict.
    pub(super) relation: String,
    /// Optional confidence score in the inclusive range 0.0..1.0.
    ///
    /// Optional here for the reason it is optional on `mem_judge`, which
    /// records the same verdict about the same kind of pair: the column
    /// accepts nothing, and a number produced only because a field was
    /// required is noise in one every reader treats as a probability.
    #[serde(default)]
    pub(super) confidence: Option<f64>,
    /// Optional short explanation for the verdict.
    #[serde(default)]
    pub(super) reasoning: Option<String>,
    /// Optional model identifier stored as provenance.
    pub(super) model: Option<String>,
}
