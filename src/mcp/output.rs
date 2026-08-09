//! What each tool sends back, and how much of a memory it shows.

use super::*;

/// Cuts a body down to what a listing shows, and says whether it cut.
///
/// One function rather than one per output type: the two that needed it were
/// written a fortnight apart with identical bodies, which is how a preview
/// length ends up meaning two different things.
fn preview_of(content: String) -> (String, bool) {
    if content.len() <= PREVIEW_BYTES {
        return (content, false);
    }
    (normalize::truncate_content(content, PREVIEW_BYTES), true)
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct SaveOutput {
    pub(super) status: String,
    /// Said when a summary was saved without a name anybody could find it by.
    ///
    /// A summary takes its title from the first line of its body that is not a
    /// heading. When there is no such line — a heading and a date, which is
    /// what the server instructions warn about in as many words — the memory
    /// falls back to `Session summary: <project>`, which is what several
    /// hundred of them were called before they had headlines, and what made
    /// them unfindable: 9.6% could be retrieved by their own words against
    /// 99.9% of memories with a title of their own.
    ///
    /// The agent that wrote it is the only one who can fix it, and only while
    /// it still remembers what the session was for. So it is told there and
    /// then rather than left to notice months later that a summary has no name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) hint: Option<String>,
    #[serde(flatten)]
    pub(super) project_context: ProjectEnvelope,
    pub(super) observation: ObservationOutput,
    pub(super) judgment_required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) judgment_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) judgment_id: Option<String>,
    pub(super) candidates: Vec<CandidateOutput>,
}

impl SaveOutput {
    pub(super) fn new(
        value: AddOutcome,
        candidates: Vec<Candidate>,
        project_context: ProjectEnvelope,
    ) -> Self {
        let judgment_required = !candidates.is_empty();
        let judgment_id = candidates
            .first()
            .map(|candidate| candidate.judgment_id.clone());
        Self {
            status: outcome_label(value.kind).to_owned(),
            hint: None,
            project_context,
            // The caller wrote this text; echoing it whole bills for it twice.
            // On a deduplicated or revised save the memory returned is the one
            // already stored, which the caller may not have — hence a preview
            // and an id, not silence.
            observation: ObservationOutput::from(value.observation).preview(),
            judgment_required,
            judgment_status: judgment_required
                .then(|| crate::store::JUDGMENT_STATUS_PENDING.to_owned()),
            judgment_id,
            candidates: candidates.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct CandidateOutput {
    pub(super) id: i64,
    pub(super) sync_id: String,
    pub(super) title: String,
    #[serde(rename = "type")]
    pub(super) kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) topic_key: Option<String>,
    pub(super) score: f64,
    pub(super) judgment_id: String,
}

impl From<Candidate> for CandidateOutput {
    fn from(value: Candidate) -> Self {
        Self {
            id: value.id,
            sync_id: value.sync_id,
            title: value.title,
            kind: value.kind,
            topic_key: value.topic_key,
            score: value.score,
            judgment_id: value.judgment_id,
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct JudgeOutput {
    pub(super) relation: RelationOutput,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct CompareOutput {
    pub(super) sync_id: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct RelationOutput {
    pub(super) id: i64,
    pub(super) sync_id: String,
    pub(super) source_id: String,
    pub(super) target_id: String,
    pub(super) relation: String,
    /// Why the verdict was given, cut like every other body this surface
    /// sends. Read back rather than echoed: this is the stored form, with any
    /// private markers already removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) reason: Option<String>,
    /// Whether `reason` was cut. Absent when it was not.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) reason_truncated: bool,
    /// What the verdict rests on, cut the same way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) evidence: Option<String>,
    /// Whether `evidence` was cut. Absent when it was not.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) evidence_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) confidence: Option<f64>,
    pub(super) judgment_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) marked_by_actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) marked_by_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) marked_by_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) session_id: Option<String>,
    pub(super) created_at: String,
    pub(super) updated_at: String,
}

impl From<Relation> for RelationOutput {
    fn from(value: Relation) -> Self {
        let reason = value.reason.map(preview_of);
        let evidence = value.evidence.map(preview_of);
        Self {
            id: value.id,
            sync_id: value.sync_id,
            source_id: value.source_id,
            target_id: value.target_id,
            relation: value.relation,
            // The caller wrote both of these in the same call, and they are
            // bounded by the memory budget rather than by a preview — so a
            // judgment with 12,000 bytes of reason and 12,000 of evidence came
            // back as 24,359 bytes of the caller's own words. `mem_compare`,
            // beside it, answers with the id alone and nothing else; this one
            // has a verdict to confirm, so it previews rather than drops.
            reason: reason.clone().map(|(text, _)| text),
            reason_truncated: reason.is_some_and(|(_, cut)| cut),
            evidence: evidence.clone().map(|(text, _)| text),
            evidence_truncated: evidence.is_some_and(|(_, cut)| cut),
            confidence: value.confidence,
            judgment_status: value.judgment_status,
            marked_by_actor: value.marked_by_actor,
            marked_by_kind: value.marked_by_kind,
            marked_by_model: value.marked_by_model,
            session_id: value.session_id,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct ReviewOutput {
    pub(super) action: String,
    /// How many memories this answer carries — the length of `observations`.
    ///
    /// One meaning, not one per action. `mark_reviewed` used to report 1 here
    /// with an empty list beside it, because the number was answering a
    /// different question — how many were marked — in the same field a listing
    /// uses for how many are listed. An agent doing the obvious thing with the
    /// two together, `observations[..count]`, read one and found none.
    ///
    /// The memory that was marked is in `observation`, singular, which is
    /// where a caller that asked to mark one should look.
    pub(super) count: usize,
    /// How many memories are due that this page does not carry.
    ///
    /// The session opening names the whole queue - "eighteen memories to read
    /// again, open it with mem_review" - and the tool it points at answers with
    /// its own page, ten by default against a ceiling of twenty. So an agent
    /// that was told eighteen, obeyed, and saw ten had nothing telling it the
    /// other eight existed: it marks the ten and the queue looks done.
    ///
    /// The same defect `MORE_MATCHED_HINT` exists for on search, and worse
    /// here, because there the caller chose the limit and here another surface
    /// named a number first.
    ///
    /// Counted rather than folded into `count`, which means the length of
    /// `observations` and nothing else - a number that answered a different
    /// question in that field is what this type's own comment above is about.
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub(super) due_omitted: usize,
    pub(super) observations: Vec<ObservationOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) observation: Option<ObservationOutput>,
}

impl ReviewOutput {
    pub(super) fn listing(
        value: Vec<Observation>,
        caveats: &std::collections::BTreeMap<String, Vec<crate::memory::model::Caveat>>,
        due: usize,
    ) -> Self {
        let observations = value
            .into_iter()
            .map(|observation| {
                let said = caveats
                    .get(&observation.sync_id)
                    .cloned()
                    .unwrap_or_default();
                let mut output = ObservationOutput::from(observation).preview();
                output.caveats = said.into_iter().map(Into::into).collect();
                output
            })
            .collect::<Vec<_>>();
        Self {
            action: "list".to_owned(),
            count: observations.len(),
            due_omitted: due.saturating_sub(observations.len()),
            observations,
            observation: None,
        }
    }

    pub(super) fn marked(value: Observation) -> Self {
        Self {
            action: "mark_reviewed".to_owned(),
            count: 0,
            // Nothing was listed, so nothing was left out. Marking one reviewed
            // answers about that one; how much queue remains is what a listing
            // is for, and a number here would be the same mistake `count` made.
            due_omitted: 0,
            observations: Vec::new(),
            observation: Some(value.into()),
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct SuggestTopicKeyOutput {
    pub(super) topic_key: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct DeleteOutput {
    pub(super) id: i64,
    pub(super) hard_delete: bool,
    pub(super) status: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct PinOutput {
    pub(super) id: i64,
    pub(super) sync_id: String,
    pub(super) pinned: bool,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct TimelineOutput {
    pub(super) focus: ObservationOutput,
    pub(super) before: Vec<TimelineEntryOutput>,
    pub(super) after: Vec<TimelineEntryOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) session_info: Option<SessionOutput>,
    /// How many of the session come before the focus, and how many after.
    /// `before` and `after` are capped by the window you asked for, so these
    /// say whether there is more on either side.
    pub(super) before_total: i64,
    pub(super) after_total: i64,
}

impl From<TimelineResult> for TimelineOutput {
    fn from(value: TimelineResult) -> Self {
        Self {
            // Previewed like its neighbours, and this used to be the one
            // exception.
            //
            // The reasoning was that the caller named this memory by id, so it
            // should arrive whole. Two things say otherwise. The tool's own
            // description promises "a 400-character preview marked
            // `content_truncated`; read one in full with mem_get_observation",
            // which was false for the very memory the call is about — a
            // published limit that is not the applied one. And the three-layer
            // pattern this module documents puts opening a body whole in
            // `mem_get_observation`, not here: search finds an id, the timeline
            // shows what surrounded it, and that one opens it.
            //
            // Measured with a 20,000-byte body: the reply was 20,851 bytes,
            // which is the whole memory plus its neighbours' previews. It was
            // found by a guard that gives every surface something huge and
            // requires a small answer, rather than by reading this line.
            focus: ObservationOutput::from(value.focus).preview(),
            before: value
                .before
                .into_iter()
                .map(|entry| TimelineEntryOutput::from(entry).preview())
                .collect(),
            after: value
                .after
                .into_iter()
                .map(|entry| TimelineEntryOutput::from(entry).preview())
                .collect(),
            session_info: value.session_info.map(Into::into),
            before_total: value.before_total,
            after_total: value.after_total,
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct TimelineEntryOutput {
    pub(super) id: i64,
    pub(super) session_id: String,
    #[serde(rename = "type")]
    pub(super) kind: String,
    pub(super) title: String,
    pub(super) content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) project: Option<String>,
    pub(super) scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) topic_key: Option<String>,
    #[serde(default = "once", skip_serializing_if = "is_once")]
    pub(super) revision_count: i64,
    #[serde(default = "once", skip_serializing_if = "is_once")]
    pub(super) duplicate_count: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) last_seen_at: Option<String>,
    pub(super) created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) deleted_at: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) is_focus: bool,
    // Not described: every tool that previews says so in its own description,
    // and a guard holds it there — this was the same sentence a third time, in
    // ten schemas, for 118 bytes each.
    //
    // Present and true when `content` is a preview rather than the whole
    // memory. Read it in full with `mem_get_observation`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) content_truncated: bool,
}

impl TimelineEntryOutput {
    /// The same preview rule as every other listing.
    ///
    /// A timeline of ten neighbours came to 18,251 tokens because each one
    /// carried its whole body — more than a ten-result search cost before any
    /// of this. The memory the caller named by id keeps its full text; its
    /// neighbours are a list to choose from.
    pub(super) fn preview(mut self) -> Self {
        (self.content, self.content_truncated) = preview_of(self.content);
        self
    }
}

impl From<TimelineEntry> for TimelineEntryOutput {
    fn from(value: TimelineEntry) -> Self {
        Self {
            id: value.id,
            session_id: value.session_id,
            kind: value.kind,
            title: value.title,
            content: value.content,
            tool_name: value.tool_name,
            project: value.project,
            scope: value.scope,
            topic_key: value.topic_key,
            revision_count: value.revision_count,
            duplicate_count: value.duplicate_count,
            last_seen_at: value.last_seen_at.filter(|seen| *seen != value.created_at),
            updated_at: Some(value.updated_at).filter(|updated| *updated != value.created_at),
            created_at: value.created_at,
            deleted_at: value.deleted_at,
            is_focus: value.is_focus,
            content_truncated: false,
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct CapturePassiveOutput {
    #[serde(flatten)]
    pub(super) project_context: ProjectEnvelope,
    pub(super) extracted: usize,
    pub(super) saved: usize,
    pub(super) duplicates: usize,
    /// Learnings past the ceiling, which this capture did not keep.
    ///
    /// The hook has said this since the ceiling went in; this door, which is
    /// the same door one layer up, did not. Without it the three numbers above
    /// stop adding up — five hundred extracted, eighty saved, none duplicate —
    /// and four hundred and twenty memories go missing with nothing said.
    ///
    /// Always sent, like the three it belongs with: a zero here is an answer,
    /// not an absence.
    pub(super) dropped: usize,
    /// Why nothing came out, when nothing did.
    ///
    /// Three zeros and no explanation is what this used to answer, and an
    /// agent reading them has no way to tell "there was nothing worth keeping
    /// in that text" from "you wrote the section in a shape I do not read".
    /// It is the second far more often: extraction wants a markdown heading on
    /// its own line and a numbered or bulleted list under it, and text that
    /// says `Key learnings: the pool leaked` inline yields nothing at all.
    ///
    /// Measured on 872 real subagent outputs from this machine's transcripts,
    /// **none** carried that heading — the sections agents actually write are
    /// `Verification`, `Blocking`, `Summary`, `Verdict`. So the silent-zero
    /// answer is the normal answer, not the rare one. `mem_search` has said
    /// why it came back empty since the day it was written; this had not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) hint: Option<String>,
}

impl CapturePassiveOutput {
    pub(super) fn new(value: PassiveCaptureResult, project_context: ProjectEnvelope) -> Self {
        Self {
            project_context,
            extracted: value.extracted,
            saved: value.saved,
            duplicates: value.duplicates,
            dropped: value.dropped,
            // The ceiling first, because it is the one an agent can do
            // something about: the text is still in front of it. Nothing
            // extracted is the commoner answer and the quieter one.
            hint: if value.dropped > 0 {
                Some(learnings_dropped_hint(value.extracted, value.dropped))
            } else {
                (value.extracted == 0).then(|| NOTHING_EXTRACTED_HINT.to_owned())
            },
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct MergeProjectsOutput {
    pub(super) canonical: String,
    pub(super) sources_merged: Vec<String>,
    pub(super) observations_updated: i64,
    pub(super) sessions_updated: i64,
    pub(super) prompts_updated: i64,
    /// Said when the canonical project had to take over a source's enrolment.
    ///
    /// A merge is a request to join two names, not to start replicating one,
    /// and this is the part of it that changes what leaves the machine. Absent
    /// when nothing was being replicated, which is most merges.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) enrolment_moved: bool,
    /// How many topic keys now name two live memories in the merged project.
    ///
    /// Each project may have had its own memory under one key, legitimately.
    /// Together they share the key, and revising one finds only the most
    /// recently updated of the two — so the other stops being reachable by its
    /// own key. Absent when a merge left none, which is most merges.
    #[serde(default, skip_serializing_if = "is_zero_count")]
    pub(super) topic_key_collisions: i64,
    /// Said when everything moved into a name the store did not hold.
    ///
    /// A rename and a typo in `to` are the same call, and this is what tells
    /// them apart. Absent on an ordinary merge into a project that was already
    /// there.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) canonical_created: bool,
}

fn is_zero_count(value: &i64) -> bool {
    *value == 0
}

impl From<MergeResult> for MergeProjectsOutput {
    fn from(value: MergeResult) -> Self {
        Self {
            canonical: value.canonical,
            sources_merged: value.sources_merged,
            enrolment_moved: value.enrolment_moved,
            observations_updated: value.observations_updated,
            sessions_updated: value.sessions_updated,
            prompts_updated: value.prompts_updated,
            topic_key_collisions: value.topic_key_collisions,
            canonical_created: value.canonical_created,
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct CurrentProjectOutput {
    pub(super) project: String,
    pub(super) project_source: String,
    pub(super) project_path: String,
    pub(super) cwd: String,
    pub(super) available_projects: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) warning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) error_hint: Option<String>,
}

impl From<ProjectDetection> for CurrentProjectOutput {
    fn from(value: ProjectDetection) -> Self {
        Self {
            project: value.project,
            project_source: value.source,
            project_path: value.path,
            cwd: std::env::current_dir()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default(),
            available_projects: value.available_projects,
            warning: value.warning,
            error_hint: value.error_hint,
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct DoctorOutput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) check: Option<String>,
    /// Present only when a project was explicitly requested and matched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) project_stats: Option<ProjectStatsOutput>,
    pub(super) healthy: bool,
    /// What the store is stamped at, and what this build reads. Carried so an
    /// agent can say which of the two is behind when a binary refuses a store.
    pub(super) schema_version: i32,
    pub(super) schema_supported: i32,
    pub(super) checks: Vec<DoctorCheckOutput>,
    pub(super) integrity_check: Vec<String>,
    /// The violations, as examples rather than as an inventory.
    ///
    /// `PRAGMA foreign_key_check` answers one row per orphaned row, and this
    /// list carried every one of them into an agent's context: 300 orphans made
    /// a 54.7 KB reply, and the number scales with the damage — which is to say
    /// the reply is largest exactly when something is wrong and an agent is
    /// trying to read what. Nothing is lost by cutting it. The count is already
    /// a sentence in `issues`, the repair is `leteo doctor --repair` rather
    /// than anything done per row, and a person who wants the inventory has
    /// `leteo doctor` at a terminal, where there is no context window to spend
    /// and the store's own report is uncut.
    pub(super) foreign_key_violations: Vec<ForeignKeyViolationOutput>,
    /// How many violations there were beyond the ones listed.
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub(super) foreign_key_violations_omitted: usize,
    pub(super) observation_fts_ok: bool,
    pub(super) prompt_fts_ok: bool,
    pub(super) observations: i64,
    pub(super) observation_fts_rows: i64,
    pub(super) prompts: i64,
    pub(super) prompt_fts_rows: i64,
    pub(super) pending_mutations: i64,
    pub(super) journal_mode: String,
    pub(super) busy_timeout_ms: i64,
    pub(super) issues: Vec<String>,
}

impl DoctorOutput {
    pub(super) fn new(
        report: DoctorReport,
        project: Option<String>,
        check: Option<String>,
        project_stats: Option<crate::memory::model::ProjectStats>,
    ) -> Self {
        Self {
            project,
            check,
            project_stats: project_stats.map(Into::into),
            healthy: report.healthy,
            schema_version: report.schema_version,
            schema_supported: report.schema_supported,
            checks: report.checks.into_iter().map(Into::into).collect(),
            integrity_check: report.integrity_check,
            foreign_key_violations: report
                .foreign_key_violations
                .iter()
                .take(VIOLATION_EXAMPLES)
                .cloned()
                .map(Into::into)
                .collect(),
            foreign_key_violations_omitted: report
                .foreign_key_violations
                .len()
                .saturating_sub(VIOLATION_EXAMPLES),
            observation_fts_ok: report.observation_fts_ok,
            prompt_fts_ok: report.prompt_fts_ok,
            observations: report.observations,
            observation_fts_rows: report.observation_fts_rows,
            prompts: report.prompts,
            prompt_fts_rows: report.prompt_fts_rows,
            pending_mutations: report.pending_mutations,
            journal_mode: report.journal_mode,
            busy_timeout_ms: report.busy_timeout_ms,
            issues: report.issues,
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct DoctorCheckOutput {
    pub(super) code: String,
    pub(super) ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) detail: Option<String>,
}

impl From<crate::memory::model::DoctorCheck> for DoctorCheckOutput {
    fn from(value: crate::memory::model::DoctorCheck) -> Self {
        Self {
            code: value.code,
            ok: value.ok,
            detail: value.detail,
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct ProjectStatsOutput {
    pub(super) name: String,
    pub(super) observation_count: i64,
    pub(super) session_count: i64,
    pub(super) prompt_count: i64,
    pub(super) directories: Vec<String>,
}

impl From<crate::memory::model::ProjectStats> for ProjectStatsOutput {
    fn from(value: crate::memory::model::ProjectStats) -> Self {
        Self {
            name: value.name,
            observation_count: value.observation_count,
            session_count: value.session_count,
            prompt_count: value.prompt_count,
            directories: value.directories,
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct ForeignKeyViolationOutput {
    pub(super) table: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) row_id: Option<i64>,
    pub(super) parent: String,
    pub(super) foreign_key_index: i64,
}

impl From<ForeignKeyViolation> for ForeignKeyViolationOutput {
    fn from(value: ForeignKeyViolation) -> Self {
        Self {
            table: value.table,
            row_id: value.row_id,
            parent: value.parent,
            foreign_key_index: value.foreign_key_index,
        }
    }
}

/// What to say when one turn left more learnings than a turn may leave.
///
/// The bound is `normalize::MAX_LEARNINGS` and the reason it exists is that a
/// capture writes a row and three full-text triggers per learning inside a call
/// somebody is waiting on. What matters here is that the caller still has the
/// text the subagent no longer has, so the rest are not lost yet — the same
/// reasoning the hook uses when the store refuses a capture outright.
pub(super) fn learnings_dropped_hint(extracted: usize, dropped: usize) -> String {
    let rest = if dropped == 1 {
        "one was not stored. If it matters, save it".to_owned()
    } else {
        format!("{dropped} were not stored. If any of them matter, save them")
    };
    format!(
        "That text left {extracted} learnings and Leteo kept the first {}; {rest} with mem_save while you still have it.",
        crate::memory::normalize::MAX_LEARNINGS
    )
}

/// What to say when a capture found nothing to keep.
///
/// Named rather than written where it is used, so the guard that checks every
/// sentence an agent reads can find it — and it needed finding: written inline,
/// it carried eighteen spaces of Rust indentation into the middle of itself.
pub(super) const NOTHING_EXTRACTED_HINT: &str = "No learnings were extracted. This reads a \
    markdown heading on a line of its own - `## Key Learnings` or `## Aprendizajes \
    Clave` - followed by a numbered or bulleted list, and keeps items of at least \
    four words. To save one fact you already have, call mem_save instead.";

/// What to say when a summary could not be named after itself.
pub(super) const UNNAMED_SUMMARY_HINT: &str = "This summary was saved without a \
    headline, so it is called after its project and nobody will find it by what it \
    was about. A summary takes its title from the first line of its body that is \
    not a heading and is not a date. Call mem_update on it with a title, or save \
    it again opening with a line that says what the session was for.";

/// What to say when a memory was filed under a word nothing searches for.
///
/// What to say when the scope somebody sent is not one of the three.
///
/// The sibling of [`UNFILED_KIND_HINT`], and the louder of the two. A type Leteo
/// does not know is kept verbatim: the word survives, and what it costs is that
/// a search narrowed by type will not return the memory. A scope it does not
/// know is *replaced* — `normalize::scope` folds anything else onto `project`,
/// because losing a memory at the door over a label is a worse answer than
/// filing it where almost all of them belong — so the caller's own value is
/// discarded, and a read narrowed to the scope they asked for will never return
/// the memory they believe they filed there.
///
/// One door said so and the other did not. Driven side by side on the same
/// call, `type: implementation` came back with a hint and `scope: personnal`
/// came back with nothing at all, filed as `project`.
pub(crate) fn refiled_scope_hint(asked: &str) -> String {
    format!(
        "Scope {asked:?} is not one of {}, so this memory was filed as {}. A read narrowed to the scope you asked for will not return it.",
        crate::memory::normalize::SCOPES.join(", "),
        crate::memory::normalize::SCOPES[0]
    )
}

/// A kind outside the eight is stored verbatim on purpose, and folding is only
/// safe for a synonym with one obvious target: `bug` is a `bugfix` and nothing
/// else, while `optimization` could as easily be a bugfix, a decision or a
/// discovery, and guessing would file it wrong and say nothing. So the word
/// survives — and the memory becomes one a search narrowed by type can never
/// return, which is the failure `mem_save`'s own `type` description warns
/// about.
///
/// A real store had 36 of them across five words: `implementation` 22,
/// `project` 5, `optimization` 4, `reference` 3, `feature` 2 — and they were
/// still arriving, four on the day this was written. The fold table had been
/// reactive until now: somebody notices a word and adds it, which is why
/// `manual` sat there for eighteen memories before anybody looked. This closes
/// the loop for every word nobody has thought of yet.
///
/// Said to whoever wrote it, while they still know what the memory was, and on
/// both surfaces: an agent gets it in the answer, a person at a terminal on
/// stderr.
pub(crate) const UNFILED_KIND_HINT: &str = "This memory's type is not one of the \
    eight a search can narrow by - bugfix, decision, policy, architecture, \
    discovery, pattern, config, preference - so a search filtered by type will \
    never return it. Call mem_update with the closest of the eight, or leave it \
    if the word matters more than being found by filter.";

/// What to say when a search matched nothing.
///
/// Full-text search matches words, not meanings, and the two languages in play
/// are rarely the same one: memories are written by an agent and are usually in
/// English, while the question often is not. Measured on a real store of 3525
/// memories, an English term finds between 2 and 20 of them where its Spanish
/// equivalent finds none — `test` 20 against `prueba` 0, `warning` 9 against
/// `aviso` 0. Nothing about `{"count": 0}` says any of that, so it reads
/// exactly like "this was never saved" and the reader stops looking.
///
/// Said to whoever asked, on both surfaces. An agent gets it in the answer; a
/// person at a terminal gets it on stderr, because `leteo search` answers with
/// a JSON array that something may be parsing and a sentence does not belong
/// in it.
pub(crate) const NO_MATCH_HINT: &str = "No memory matched those words. Full-text search \
    matches words, not meanings: try identifiers, paths, numbers or error \
    strings, which carry across languages unchanged, or the same idea in the \
    language the memories are written in - the session context says which. \
    mem_context lists recent work without needing a query.";

/// What to say when the words matched, in a project this is not.
///
/// An empty answer has two reasons and they call for opposite actions.
/// [`NO_MATCH_HINT`] says the store has never heard of this — rewrite the
/// question. But when the project was chosen by the directory rather than
/// asked for, the answer may be empty because the memory is filed elsewhere,
/// and telling somebody to use fewer, more distinctive words sends them to
/// rewrite a question that was already right.
///
/// Worse for an agent than for a person: it will rewrite, come back empty
/// again, and report that the store does not know.
///
/// One sentence for both surfaces, with each naming its own way of widening —
/// `--all-projects` at a terminal, `all_projects` in a tool call. The reason is
/// one fact and was written out on one surface only, which is how the two came
/// to disagree about what an empty answer means.
/// `cap` is what the count was measured against, and it decides whether the
/// number is a count or a floor.
///
/// Two callers ask this and they measure differently. The opening block and
/// `mem_context` count memories outside the project up to [`ELSEWHERE_CAP`], so
/// a hundred means "a hundred or more". A search runs itself again with the
/// project narrowing lifted and counts the page that comes back, which is the
/// caller's own limit — so on a query matching 332 memories in other projects
/// it said "1 elsewhere" at `limit: 1`, "3" at 3, "10" at 10 and "20" at 20.
/// The number was the question restated, and an agent reading "10 elsewhere"
/// has been told that widening yields ten.
///
/// It cannot say more than its cap either way, so what it says is that: `N` for
/// a number the cap did not touch, and `N or more` for one it did.
/// How many orphaned rows a diagnosis shows before it starts counting instead.
///
/// Enough to see the shape of the damage — which table, which parent — and few
/// enough that a reply about a broken store does not itself become the problem.
/// The same number every other list on this surface stops at.
pub(super) const VIOLATION_EXAMPLES: usize = 20;

fn is_zero_usize(value: &usize) -> bool {
    *value == 0
}

pub(crate) fn no_match_here_hint(
    project: &str,
    elsewhere: usize,
    cap: usize,
    widen: &str,
) -> String {
    let elsewhere = if elsewhere >= cap {
        format!("{elsewhere} or more")
    } else {
        elsewhere.to_string()
    };
    format!(
        "Nothing in {project}, which is the project this directory belongs to, but {elsewhere} elsewhere - pass {widen} to search the whole store."
    )
}

/// What to say when the caller's own limit is what ended the list.
///
/// A full page and an exhausted one are the same shape, and the reply only ever
/// said which when the *store's* maximum was the one that ended it. The default
/// limit is ten: over sixty real questions eighteen came back with exactly ten
/// and seventeen of those had more, so an agent reading a full page was, nine
/// times in ten, reading part of an answer and being told nothing.
pub(crate) const MORE_MATCHED_HINT: &str = "More matched than were returned. Ask again with a higher limit for the rest, or narrow the query - by type or project - to see the ones that matter first.";

/// What to say when the *store's* maximum is what ended the list.
///
/// Its sibling above, for the caller's own limit. This one lives here rather
/// than inline beside the reply because two surfaces reach the cap and only one
/// used to mention it: `mem_search` said so and `leteo search --limit 50`
/// printed twenty rows in silence, which reads as twenty matches. Asking again
/// with a higher limit — which is what the sibling sentence advises — cannot
/// work here, so the two sentences must not be swapped for one another.
pub(crate) fn clamped_hint(cap: usize) -> String {
    format!(
        "This is the most a single search returns ({cap}), not everything that matched. \
         Narrow the query, or filter by type or project, to see the rest."
    )
}

/// How far the count behind that sentence goes before it stops.
///
/// `project <> ?` is not a range, so no index answers it and an exact count
/// reads every live row of the store. Bounding it makes the answer constant in
/// the size of the store; saying "100 or more" rather than a number that was
/// never counted keeps the published limit and the applied one the same.
pub(crate) const ELSEWHERE_CAP: usize = 100;

/// What to say when the widened retry is what found these.
///
/// Nothing matched every word, so the search asked again for any of them.
/// Measured over two hundred questions against a real store that retry found
/// the right memory every time it ran — but it also answers a question nobody
/// quite asked, and an agent that reads these as exact matches will overstate
/// what the store actually says. So they arrive labelled, one line, once.
pub(crate) const PARTIAL_MATCH_HINT: &str = "No memory matched every word, so these \
    matched some of them — check each one against the question before relying \
    on it. Fewer, more distinctive words usually match exactly.";

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct SearchOutput {
    #[serde(flatten)]
    pub(super) project_context: ProjectEnvelope,
    /// How many memories are in `results` — the length of that list, not how
    /// many matched. When more matched than were returned, the hint says so.
    pub(super) count: usize,
    pub(super) results: Vec<SearchResultOutput>,
    /// Carried when nothing matched, or when only some of the words did.
    ///
    /// The wording is in [`NO_MATCH_HINT`] and [`PARTIAL_MATCH_HINT`]. Below
    /// the blank line because everything above it is shipped to every client
    /// that lists the tools, and an intra-doc link arrives there as brackets
    /// around a name that resolves to nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) hint: Option<String>,
}

impl SearchOutput {
    /// `clamped` says the store's own maximum, not the caller's `limit`, is
    /// what ended this list.
    ///
    /// `caveats` is what the graph says about the memories being handed back.
    /// A superseded decision looks exactly like one that still holds, and this
    /// was the third route to hand one over without saying — the session-start
    /// context and `mem_context` were fixed one at a time, and search, the most
    /// used of the three, was still quiet.
    ///
    /// `elsewhere` is how many the same question found with the project
    /// narrowing lifted, and the limit it was counted against, because that
    /// limit is what the number can reach — see [`no_match_here_hint`]. Only
    /// asked for when the answer came back empty and nobody named a project.
    pub(super) fn new(
        value: Vec<SearchResult>,
        project_context: ProjectEnvelope,
        clamped: bool,
        caveats: &std::collections::BTreeMap<String, Vec<crate::memory::model::Caveat>>,
        elsewhere: Option<(String, usize, usize)>,
        more: bool,
    ) -> Self {
        let widened = value.iter().any(|result| result.partial);
        let results: Vec<SearchResultOutput> = value
            .into_iter()
            .map(|result| {
                let said = caveats
                    .get(&result.observation.sync_id)
                    .cloned()
                    .unwrap_or_default();
                let mut out: SearchResultOutput = result.into();
                out.observation = out.observation.preview();
                out.observation.caveats = said.into_iter().map(Into::into).collect();
                out
            })
            .collect();
        // At most one line of advice, and the one that changes what to do next.
        // An empty result and a clamped one cannot both be true; a widened
        // result that also filled the cap is possible, and being told the words
        // were relaxed matters more than being told there may be a few more of
        // them.
        let hint = if results.is_empty() {
            match elsewhere {
                Some((project, found, cap)) if found > 0 => {
                    Some(no_match_here_hint(&project, found, cap, "all_projects"))
                }
                _ => Some(NO_MATCH_HINT.to_owned()),
            }
        } else if widened {
            Some(PARTIAL_MATCH_HINT.to_owned())
        } else if clamped {
            Some(clamped_hint(results.len()))
        } else if more {
            Some(MORE_MATCHED_HINT.to_owned())
        } else {
            None
        };
        Self {
            project_context,
            count: results.len(),
            hint,
            results,
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct SearchResultOutput {
    #[serde(flatten)]
    pub(super) observation: ObservationOutput,
    pub(super) rank: f64,
    /// Present only on a memory that matched some of the words rather than all
    /// of them, so a widened answer can be read row by row.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) partial: bool,
}

impl From<SearchResult> for SearchResultOutput {
    fn from(value: SearchResult) -> Self {
        Self {
            observation: value.observation.into(),
            rank: value.rank,
            partial: value.partial,
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct ObservationResultOutput {
    /// The memory, carrying whatever the graph says against it.
    ///
    /// The caveats used to sit beside this rather than on it, and that is one
    /// place too many. `mem_search` flattens [`ObservationOutput`], so a
    /// superseded memory comes back with `caveats` among its own fields;
    /// `mem_context` puts them on each listed memory the same way. Here they
    /// were a sibling of `observation`, so an agent that saw the warning in a
    /// search and followed the id here to read the whole thing looked where it
    /// had just seen one and found nothing — and "nothing said against it" is
    /// the reading that presents an overturned decision as current, which is
    /// the single thing caveats exist to prevent.
    ///
    /// One path now: the caveats are on the memory, wherever the memory is.
    pub(super) observation: ObservationOutput,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct CaveatOutput {
    /// `superseded_by` or `conflicts_with`, as this memory sees it.
    pub(super) relation: String,
    pub(super) other_id: i64,
    pub(super) other_title: String,
}

impl From<crate::memory::model::Caveat> for CaveatOutput {
    fn from(value: crate::memory::model::Caveat) -> Self {
        use crate::memory::model::CaveatVerb;
        Self {
            relation: match value.verb {
                CaveatVerb::SupersededBy => "superseded_by",
                CaveatVerb::ConflictsWith => "conflicts_with",
            }
            .to_owned(),
            other_id: value.other_id,
            other_title: value.other_title,
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct ContextOutput {
    #[serde(flatten)]
    pub(super) project_context: ProjectEnvelope,
    /// How many memories this answer carries, across both lists.
    ///
    /// Not the length of `observations`: the newest few come with their bodies
    /// and the rest arrive in `also_remembered` as titles. Asking for fifty
    /// gives `count: 50`, five in `observations` and forty-five in
    /// `also_remembered` — reading only the first list and trusting this
    /// number would look like forty-five went missing.
    pub(super) count: usize,
    /// What language to write and search memories in.
    ///
    /// Carried here because the session-start hook is not a delivery route most
    /// agents have. Of the twelve Leteo configures, three run hooks —
    /// Claude Code, Codex, and OpenCode through its plugin — and the other nine
    /// are configured over MCP alone. The language setting was offered to all
    /// twelve in the setup wizard and reached three of them.
    ///
    /// This is the one place that covers the rest. Every instruction file Leteo
    /// writes tells the agent to call `mem_context` before acting, so it is
    /// what an MCP-only client reads first, and being read from the store on
    /// each call it cannot go stale the way a line written into a file at setup
    /// time would.
    pub(super) memory_language: String,
    /// Carried when this project answered with nothing and the store did not.
    ///
    /// The wording is in `no_match_here_hint`, shared with search: an empty
    /// answer that does not say which of its two reasons it is sends the
    /// reader to solve the wrong problem.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) hint: Option<String>,
    pub(super) observations: Vec<ObservationOutput>,
    /// Everything behind the newest few, as an index rather than a recital.
    ///
    /// A memory's opening is almost never the answer: measured over 2,547
    /// memories of a real store, **91% of the paths, identifiers, numbers and
    /// quoted strings fall past the first 400 characters**, and not one memory
    /// fits inside them — the median runs to 1,991. So a preview of everything
    /// spends the budget reciting openings, where a list of titles says what is
    /// remembered and `mem_get_observation` fetches whichever one matters.
    ///
    /// The session-start hook has worked this way since it was measured there —
    /// "an index of fifty beats a recital of twenty" — and this tool, which is
    /// the only route the nine clients of twelve without hooks have, kept the
    /// recital.
    pub(super) also_remembered: Vec<MemoryLineOutput>,
    /// How many pinned memories did not fit, when the shelf outgrew the block.
    ///
    /// Pins are listed on top of the budget rather than inside it, so that
    /// deciding a memory matters never costs the room recent work needs — but
    /// on top of a bound is not outside every bound. Said rather than
    /// swallowed: dropping a deliberate choice in silence is worse than the
    /// bytes it would have cost.
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub(super) pinned_omitted: usize,
    pub(super) sessions: Vec<SessionSummaryOutput>,
    pub(super) prompts: Vec<PromptLineOutput>,
}

/// A memory named rather than quoted: what it is, and how to fetch it.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct MemoryLineOutput {
    pub(super) id: i64,
    #[serde(rename = "type")]
    pub(super) kind: String,
    pub(super) title: String,
    /// What the graph says about it, when anything does — the same warning the
    /// detailed entries carry, because a superseded decision is no less
    /// superseded for being listed briefly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) caveats: Vec<CaveatOutput>,
}

/// A prompt as the opening context needs it: what was asked, and when.
///
/// The full record answers `mem_save_prompt`, where the caller has just written
/// one and `sync_id` is what a later `mem_save` links itself to. Listed as
/// context it answers nothing: no tool anywhere takes a prompt's `id` or
/// `sync_id`, the `session_id` and `project` are the ones the reply already
/// names once at the top, and an agent reading "what has this person been
/// asking" can do nothing with any of them.
///
/// It is not free. Measured on a real store, the ten prompts in one
/// `mem_context` cost 2,268 bytes to carry 420 bytes of what somebody actually
/// typed — 227 bytes a prompt for 42 of content, and 22.6% of the whole reply.
/// The markdown the session-start hook renders made this choice already: it
/// prints the date and the words and nothing else. This is the same handover in
/// typed form, so it says the same thing.
///
/// [`MemoryLineOutput`] keeps its `id` for the reason this one drops it —
/// `mem_get_observation` takes it, and there is no such tool for a prompt.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct PromptLineOutput {
    pub(super) content: String,
    pub(super) created_at: String,
}

impl From<Prompt> for PromptLineOutput {
    fn from(value: Prompt) -> Self {
        Self {
            // Cut like every other preview this surface sends.
            //
            // A prompt is whatever somebody typed, and people paste. The
            // markdown block has cut them to 200 characters since it was
            // written; this one sent them whole, so the same handover for the
            // same project came to 1,166 bytes of prompts as markdown and
            // 45,807 as JSON — 87% of a 52,765-byte reply, one prompt of it
            // 13,974 bytes long.
            //
            // At `PREVIEW_BYTES` rather than the markdown's 200, because the
            // two surfaces are allowed to preview differently — a tool result
            // is fetched deliberately, the opening blob is spent whether or not
            // it is read. What they are not allowed to be is unbounded.
            content: normalize::truncate_content(value.content, PREVIEW_BYTES),
            created_at: value.created_at,
        }
    }
}

/// What a context answer says about itself rather than about the memories.
///
/// Three fields that travel together and are not content: where the answer
/// looked, what language to write in, and — when it found nothing — whether
/// the store or the directory is what was empty. Grouped because they arrived
/// one at a time and `new` reached eight arguments, which is the point at
/// which a caller starts passing them in the wrong order.
pub(super) struct ContextEnvelope {
    pub(super) project: ProjectEnvelope,
    pub(super) memory_language: String,
    pub(super) elsewhere: Option<(String, usize)>,
}

impl ContextOutput {
    /// `pinned` says how many of `observations` are pinned, and they lead it.
    ///
    /// Pinning is a deliberate act, so a pinned memory keeps its content
    /// however many there are; the split falls after them and after the newest
    /// [`crate::recall::DETAILED`] of the rest. That is the rule the
    /// session-start hook renders, applied to the same handover in typed form.
    pub(super) fn new(
        observations: Vec<Observation>,
        pinned: usize,
        pinned_omitted: usize,
        sessions: Vec<SessionSummary>,
        prompts: Vec<Prompt>,
        envelope: ContextEnvelope,
        caveats: &std::collections::BTreeMap<String, Vec<crate::memory::model::Caveat>>,
    ) -> Self {
        let said = |sync_id: &str| -> Vec<CaveatOutput> {
            caveats
                .get(sync_id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(Into::into)
                .collect()
        };
        let count = observations.len();
        let detailed = pinned
            .saturating_add(crate::recall::DETAILED)
            .min(observations.len());
        let mut observations = observations;
        let listed = observations.split_off(detailed);
        Self {
            project_context: envelope.project,
            count,
            memory_language: envelope.memory_language,
            hint: envelope.elsewhere.map(|(project, held)| {
                no_match_here_hint(&project, held, ELSEWHERE_CAP, "all_projects")
            }),
            pinned_omitted,
            observations: observations
                .into_iter()
                .map(|observation| {
                    let caveats = said(&observation.sync_id);
                    let mut output = ObservationOutput::from(observation).preview();
                    output.caveats = caveats;
                    output
                })
                .collect(),
            also_remembered: listed
                .into_iter()
                .map(|observation| MemoryLineOutput {
                    caveats: said(&observation.sync_id),
                    id: observation.id,
                    kind: observation.kind,
                    title: observation.title,
                })
                .collect(),
            sessions: sessions.into_iter().map(Into::into).collect(),
            prompts: prompts.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct SessionSummaryOutput {
    pub(super) id: String,
    pub(super) project: String,
    /// When the session last did something, not when it opened.
    ///
    /// Every listing of sessions orders by the last activity. The markdown the
    /// hook renders was printing the start date against that order — a list
    /// sorted by one date and labelled with another, which reads as a list that
    /// is not sorted at all — and was fixed; this, the same handover in typed
    /// form, was left saying the start. Side by side on a real store, same
    /// order, same five sessions:
    ///
    /// ```text
    ///   mem_context     markdown
    ///   2026-08-05      2026-08-05     2 memories
    ///   2026-07-28      2026-08-05   148 memories  <- saved one an hour before
    ///   2026-07-31      2026-08-01     6 memories
    /// ```
    ///
    /// This is the surface the nine clients of twelve that run no hooks
    /// actually read, so the misleading date outlived the fix on the one that
    /// nine of them never see.
    pub(super) last_activity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) ended_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) summary: Option<String>,
    pub(super) observation_count: i64,
}

impl From<SessionSummary> for SessionSummaryOutput {
    fn from(value: SessionSummary) -> Self {
        Self {
            id: value.id,
            project: value.project,
            last_activity: value.last_activity,
            ended_at: value.ended_at,
            // Cut like every other preview this surface sends, and for the
            // same reason its sibling above is.
            //
            // A summary is written by whoever closed the session and nothing
            // bounded it here. The markdown block has cut these to 200
            // characters since it was written; five of them are listed in every
            // opening context, so one long one is the whole reply: measured
            // against a copy of a real store, a single session's summary came
            // to 43,499 bytes of a 48,840-byte answer — 91% — where the
            // markdown rendered the same session in 249.
            //
            // Found immediately after fixing the prompt beside it, which is the
            // lesson rather than the bug: the two are the same shape in the
            // same reply, and only one was looked at.
            summary: value
                .summary
                .map(|summary| normalize::truncate_content(summary, PREVIEW_BYTES)),
            observation_count: value.observation_count,
        }
    }
}

/// The value `revision_count` and `duplicate_count` carry when nothing has
/// happened to a memory, which is nearly every memory.
///
/// Omitted rather than repeated: on a real store the four fields defaulted
/// here were 1,580 bytes of a 22,620-byte context and 16% of a search result,
/// saying "this was never revised, never duplicated, is active and is not
/// pinned" once per memory. `default` is what makes the schema mark them
/// optional, so a reader still knows what absence means.
fn once() -> i64 {
    1
}

fn is_once(value: &i64) -> bool {
    *value == 1
}

fn active_state() -> String {
    "active".to_owned()
}

fn is_active_state(value: &str) -> bool {
    value == "active"
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct ObservationOutput {
    pub(super) id: i64,
    pub(super) sync_id: String,
    pub(super) session_id: String,
    #[serde(rename = "type")]
    pub(super) kind: String,
    pub(super) title: String,
    pub(super) content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) project: Option<String>,
    pub(super) scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) topic_key: Option<String>,
    // Not described to the agent: the name carries it, and the sentence cost
    // 99 bytes in each of the eight tools that embed this type. See the note on
    // `preview` about what an output description has to earn.
    //
    // How many times this memory has been revised. Absent when it never has
    // been, which is nearly always.
    #[serde(default = "once", skip_serializing_if = "is_once")]
    pub(super) revision_count: i64,
    // Not described: the name carries it. How many times the same memory was
    // offered again, absent at one.
    #[serde(default = "once", skip_serializing_if = "is_once")]
    pub(super) duplicate_count: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) last_seen_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) review_after: Option<String>,
    /// The prompt this memory answers, when one was in flight when it was saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) prompt_sync_id: Option<String>,
    /// `active`, `needs_review` or `deleted`. Absent when active.
    #[serde(default = "active_state", skip_serializing_if = "is_active_state")]
    pub(super) state: String,
    // Not described: `pinned` says it, and absence says false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) pinned: bool,
    pub(super) created_at: String,
    // Not described: absence is the schema's own word for "not present", and
    // the sentence cost 122 bytes in each of eight tools.
    //
    // Absent when a memory has not been touched since it was written, which is
    // what the same instant in three fields was saying.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) deleted_at: Option<String>,
    // Not described: every tool that previews says so in its own description,
    // and a guard holds it there — this was the same sentence a third time, in
    // ten schemas, for 118 bytes each.
    //
    // Present and true when `content` is a preview rather than the whole
    // memory. Read it in full with `mem_get_observation`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) content_truncated: bool,
    /// What the graph says about this memory, when anything does.
    ///
    /// Empty on nearly every entry and omitted when empty, so it reads as a
    /// warning rather than as noise on every line.
    ///
    /// Filled by the context, which is the surface most clients have: of the
    /// twelve agents Leteo configures, three run hooks and the other nine reach
    /// context through `mem_context` alone, which every instruction file tells
    /// them to call before acting. An agent is told a memory has been
    /// overturned on a prompt, at a session opening and when it fetches one
    /// whole — and that was the one route left handing over a superseded
    /// decision as though it still held.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) caveats: Vec<CaveatOutput>,
}

impl ObservationOutput {
    /// Cuts `content` down to a preview, and says so.
    ///
    /// For the tools that list memories. `mem_get_observation` does not call
    /// this: asking for one memory by id is asking to read it.
    pub(super) fn preview(mut self) -> Self {
        (self.content, self.content_truncated) = preview_of(self.content);
        self
    }
}

impl From<Observation> for ObservationOutput {
    fn from(value: Observation) -> Self {
        let state = value.state().to_owned();
        Self {
            id: value.id,
            sync_id: value.sync_id,
            session_id: value.session_id,
            kind: value.kind,
            title: value.title,
            content: value.content,
            tool_name: value.tool_name,
            project: value.project,
            scope: value.scope,
            topic_key: value.topic_key,
            revision_count: value.revision_count,
            duplicate_count: value.duplicate_count,
            // A memory nobody has touched carries the same instant three
            // times. Only what says something new is sent: 1,520 bytes of a
            // 22,620-byte context were `updated_at` and `last_seen_at`
            // repeating `created_at`.
            last_seen_at: value.last_seen_at.filter(|seen| *seen != value.created_at),
            review_after: value.review_after,
            prompt_sync_id: value.prompt_sync_id,
            state,
            pinned: value.pinned,
            updated_at: Some(value.updated_at).filter(|updated| *updated != value.created_at),
            created_at: value.created_at,
            deleted_at: value.deleted_at,
            content_truncated: false,
            // Filled by the caller that has them, because fetching them per
            // memory would be one query per row: the context asks for all of
            // them in one go.
            caveats: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct PromptResultOutput {
    #[serde(flatten)]
    pub(super) project_context: ProjectEnvelope,
    pub(super) prompt: PromptOutput,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct PromptOutput {
    pub(super) id: i64,
    pub(super) sync_id: String,
    pub(super) session_id: String,
    // Not described, for the reason the other two are not: the name says it,
    // and the tool that previews says so in its own description.
    //
    // Whether `content` was cut. Absent when it was not.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) content_truncated: bool,
    pub(super) content: String,
    /// The project the prompt was asked in, when it carries one.
    ///
    /// `Option` for the same reason as [`ProjectEnvelope::project_path`]: serde
    /// omits an empty `String` while `schemars` declares it required, so a
    /// prompt saved without a project produced a reply its own schema forbade.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) project: Option<String>,
    pub(super) created_at: String,
}

impl From<Prompt> for PromptOutput {
    fn from(value: Prompt) -> Self {
        let preview = preview_of(value.content);
        Self {
            id: value.id,
            sync_id: value.sync_id,
            session_id: value.session_id,
            // Cut, like the prompts a context hands over.
            //
            // This is the reply to `mem_save_prompt`, so the caller typed these
            // words a moment ago and echoing them whole bills for them twice.
            // People paste: on a real store the longest prompt is 13,974 bytes,
            // and carrying prompts in full is what made one `mem_context` reply
            // 52,765 bytes with 87% of it pasted text. That was fixed for the
            // listing and left here.
            //
            // What the caller needs back is the `sync_id`, so a save made while
            // answering this prompt can say what it was answering. The words
            // are its own.
            content_truncated: preview.1,
            content: preview.0,
            project: Some(value.project).filter(|project| !project.is_empty()),
            created_at: value.created_at,
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct SessionResultOutput {
    pub(super) session: SessionOutput,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct SessionOutput {
    pub(super) id: String,
    pub(super) project: String,
    pub(super) directory: String,
    pub(super) started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) ended_at: Option<String>,
    /// The session's summary, cut like every other body this surface sends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) summary: Option<String>,
    /// Whether `summary` was cut. Absent when it was not.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) summary_truncated: bool,
}

impl From<Session> for SessionOutput {
    fn from(value: Session) -> Self {
        // The caller of `mem_session_end` wrote this summary in the same call.
        //
        // It came back whole: a 12,000-byte summary made a 12,171-byte reply,
        // which is the caller's own text billed twice. The listing of sessions
        // a context hands over was bounded when the same thing was found
        // there; this, the reply to writing one, was not.
        let summary = value.summary.map(preview_of);
        let summary_truncated = summary.as_ref().is_some_and(|(_, cut)| *cut);
        Self {
            summary: summary.map(|(text, _)| text),
            summary_truncated,
            id: value.id,
            project: value.project,
            directory: value.directory,
            started_at: value.started_at,
            ended_at: value.ended_at,
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(super) struct StatsOutput {
    pub(super) total_sessions: i64,
    pub(super) total_observations: i64,
    pub(super) total_prompts: i64,
    /// Projects that hold at least one memory, the most recently written
    /// first. Not every project the store knows: one with only a session or a
    /// prompt is absent, which on a real store is two of nineteen.
    pub(super) projects: Vec<String>,
}

impl From<Stats> for StatsOutput {
    fn from(value: Stats) -> Self {
        Self {
            total_sessions: value.total_sessions,
            total_observations: value.total_observations,
            total_prompts: value.total_prompts,
            projects: value.projects,
        }
    }
}
