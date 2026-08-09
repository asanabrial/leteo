use std::collections::BTreeMap;

/// The observation type a session summary is saved under.
///
/// Named in the model rather than beside one of its readers: the store filters
/// on it, the opening context folds on it, and two spellings of the same string
/// is one typo away from a filter that silently matches nothing.
pub const SESSION_SUMMARY: &str = "session_summary";

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub project: String,
    pub directory: String,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSummary {
    pub id: String,
    pub project: String,
    pub started_at: String,
    /// When this session last did anything — its newest memory, or its start if
    /// it saved none. Every listing of sessions orders by it, and a session can
    /// stay open for a week: `manual-save-leteo` on a real store began on the
    /// 28th of July and was still saving on the 5th of August.
    pub last_activity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub observation_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Observation {
    pub id: i64,
    pub sync_id: String,
    pub session_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_key: Option<String>,
    pub revision_count: i64,
    pub duplicate_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_after: Option<String>,
    /// The prompt that motivated this memory, when one was in flight.
    ///
    /// A memory without the request behind it loses why it exists, so a save
    /// that happens while a prompt is known records that link.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_sync_id: Option<String>,
    /// Whether this store shows the memory first.
    ///
    /// Written down, but not sent. The two are different questions and this
    /// field used to answer neither: `#[serde(skip)]` kept it out of the wire
    /// *and* out of an export, so `leteo export` followed by `leteo import`
    /// came back with every pin lost, silently — while the import statement
    /// has always had a column ready for it.
    ///
    /// Replication leaves it alone on purpose, and Engram decided the same
    /// with a test saying so: pinning is where this store looks, not what the
    /// memory is, and one machine's shelf should not rearrange everybody
    /// else's. The wire strips it in `enqueue_observation`, which is where the
    /// rule can be read.
    ///
    /// An export is not another machine's view; it is this store, written
    /// down, and a backup that quietly drops what somebody chose to keep in
    /// front is a lossy backup.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
}

impl Observation {
    /// What condition this memory is in, in one word.
    ///
    /// Deletion is asked about first. A deleted memory is not `active`
    /// whatever its review window says, and it used to answer that it was:
    /// `mem_get_observation` is the one surface that still hands a deleted
    /// memory over — search excludes it, the context excludes it, and
    /// `mem_timeline` refuses outright — so it came back with `deleted_at`
    /// filled in and `state: "active"` beside it, the two fields contradicting
    /// each other in one payload.
    ///
    /// Told rather than refused. An id in an agent's hand is usually one it
    /// read in an older context, and "that memory was deleted" is a more
    /// useful answer than an error, as long as it is said out loud.
    pub fn state(&self) -> &'static str {
        if self.deleted_at.is_some() {
            return "deleted";
        }
        let Some(review_after) = &self.review_after else {
            return "active";
        };
        let Some(deadline) = crate::timestamp::parse(review_after) else {
            return "active";
        };
        if deadline <= chrono::Utc::now().naive_utc() {
            "needs_review"
        } else {
            "active"
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchResult {
    #[serde(flatten)]
    pub observation: Observation,
    pub rank: f64,
    /// Whether this was found by matching only *some* of the words asked for.
    ///
    /// Absent from the output when false, which is the ordinary case. It is
    /// there so the answer can say which kind of match it is: a memory that
    /// matched four words out of eight is worth handing over, and worth
    /// handing over labelled.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub partial: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimelineEntry {
    pub id: i64,
    pub session_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_key: Option<String>,
    pub revision_count: i64,
    pub duplicate_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
    pub is_focus: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimelineResult {
    pub focus: Observation,
    pub before: Vec<TimelineEntry>,
    pub after: Vec<TimelineEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_info: Option<Session>,
    /// How many of the session come before the focus, and how many after.
    ///
    /// Not how many are listed: `before` and `after` are capped by the window
    /// the caller asked for, so a full list and an exhausted one are the same
    /// shape. These say which side has more, which is what decides whether to
    /// ask again — a focus can be the first memory of a long session or the
    /// last.
    ///
    /// They replace a single `total_in_range` that held the whole session's
    /// count: 221 on a real store, for every focus, whatever window was asked
    /// for. The session total is still available — it is these two and the
    /// focus — and now nothing is named after a range it never counted.
    pub before_total: i64,
    pub after_total: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Prompt {
    pub id: i64,
    pub sync_id: String,
    pub session_id: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub project: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddObservation {
    pub session_id: String,
    pub kind: String,
    pub title: String,
    pub content: String,
    pub tool_name: Option<String>,
    pub project: Option<String>,
    pub scope: String,
    pub topic_key: Option<String>,
    /// `sync_id` of the prompt this memory answers, when one is known.
    pub prompt_sync_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateObservation {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddPrompt {
    pub session_id: String,
    pub content: String,
    pub project: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AddOutcomeKind {
    Inserted,
    Revised,
    Deduplicated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AddOutcome {
    pub kind: AddOutcomeKind,
    pub observation: Observation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Candidate {
    pub id: i64,
    pub sync_id: String,
    pub title: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_key: Option<String>,
    pub score: f64,
    pub judgment_id: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CandidateOptions {
    pub project: Option<String>,
    pub scope: Option<String>,
    pub limit: Option<usize>,
    pub bm25_floor: Option<f64>,
    pub skip_insert: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Relation {
    pub id: i64,
    pub sync_id: String,
    pub source_id: String,
    pub target_id: String,
    pub relation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    pub judgment_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marked_by_actor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marked_by_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marked_by_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// One end of a pair somebody still has to rule on.
///
/// Deliberately without the body. A pair is handed over unasked, on a surface
/// whose whole point is to cost less than what it saves, and the body is where
/// the bytes are: two previews of 300 characters made a pair cost about four
/// times the rest of its entry. What a verdict actually turns on is cheaper
/// than that — the category, the title, and above all the topic key, since two
/// memories under one key are a revision of each other and two under different
/// keys almost never conflict. The numeric `id` is here because
/// `mem_get_observation` takes that and not a `sync_id`, so reading either one
/// whole is one call away for the pairs where the titles genuinely are not
/// enough.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingSide {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub topic_key: Option<String>,
}

/// A pair a save proposed and nobody ruled on.
///
/// Either side is `None` when that memory has been deleted since the pair was
/// proposed. Such a pair cannot be ruled on at all — `mem_judge` will not find
/// the memory — so it is named as unjudgeable rather than handed to an agent
/// that would spend two calls discovering the same thing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingPair {
    pub judgment_id: String,
    pub created_at: String,
    pub source: Option<PendingSide>,
    pub target: Option<PendingSide>,
}

impl PendingPair {
    /// Whether both memories are still there to be compared.
    pub fn judgeable(&self) -> bool {
        self.source.is_some() && self.target.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelationListItem {
    pub id: i64,
    pub sync_id: String,
    pub relation: String,
    pub judgment_status: String,
    pub source_id: String,
    pub source_title: String,
    pub target_id: String,
    pub target_title: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelationStats {
    pub project: String,
    pub by_relation: BTreeMap<String, i64>,
    pub by_judgment_status: BTreeMap<String, i64>,
    pub deferred: i64,
    pub dead: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListRelationsOptions {
    pub project: Option<String>,
    pub status: Option<String>,
    pub since: Option<String>,
    pub limit: Option<usize>,
    pub offset: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanOptions {
    pub project: String,
    pub since: Option<String>,
    pub apply: bool,
    pub max_insert: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScanResult {
    pub project: String,
    pub dry_run: bool,
    pub inspected: i64,
    /// Pairs the finder proposed, before anything was ruled out.
    pub candidates_found: i64,
    /// Pairs skipped because the store already holds a relation between them,
    /// judged or not.
    pub already_related: i64,
    /// Relations this scan wrote — or would have written, when `dry_run` is
    /// true.
    ///
    /// One number for one question, with the flag beside it saying whether it
    /// happened. A dry run used to answer zero here and zero in
    /// `already_related` because it never asked, which made the preview a
    /// count of candidates and nothing else: 2,400 proposed on a real project,
    /// 299 of them already known.
    pub relations_inserted: i64,
    /// Whether `max_insert` stopped the scan before it ran out of candidates.
    pub capped: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectStats {
    pub name: String,
    pub observation_count: i64,
    pub session_count: i64,
    pub prompt_count: i64,
    pub directories: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PruneResult {
    pub project: String,
    pub sessions_deleted: i64,
    pub prompts_deleted: i64,
}

/// What a cascading session delete took with it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeleteSessionResult {
    pub session: String,
    pub observations_deleted: i64,
    pub prompts_deleted: i64,
    pub hard_delete: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeleteProjectResult {
    pub project: String,
    pub observations_deleted: i64,
    pub prompts_deleted: i64,
    pub sessions_deleted: i64,
    /// Sessions of this project that still hold rows belonging to another one.
    ///
    /// A session belongs to one project but the rows inside it carry their own,
    /// so an agent that saved a prompt under a different name than the session
    /// it was working in leaves that prompt behind. Removing the session would
    /// orphan it, so the session stays — and this says how many did, because a
    /// delete that quietly left something is worse than one that says it did.
    #[serde(default)]
    pub sessions_kept: i64,
    pub hard_delete: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListDeferredOptions {
    pub status: Option<String>,
    pub limit: Option<usize>,
    pub offset: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplayDeferredResult {
    pub retried: i64,
    pub succeeded: i64,
    pub failed: i64,
    pub dead: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeferredRow {
    pub sync_id: String,
    pub entity: String,
    pub payload: String,
    pub payload_valid: bool,
    pub apply_status: String,
    pub retry_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_attempted_at: Option<String>,
    pub first_seen_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveRelationParams {
    pub sync_id: String,
    pub source_id: String,
    pub target_id: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct JudgeRelationParams {
    pub judgment_id: String,
    pub relation: String,
    pub reason: Option<String>,
    pub evidence: Option<String>,
    pub confidence: Option<f64>,
    pub marked_by_actor: String,
    pub marked_by_kind: String,
    pub marked_by_model: Option<String>,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct JudgeBySemanticParams {
    pub source_id: String,
    pub target_id: String,
    pub relation: String,
    /// How sure, when whoever judged is willing to say.
    ///
    /// Optional because the column is, because `judge_relation` — the other
    /// way the same verdict is recorded — has always taken it that way, and
    /// because a number a language model produces only to satisfy a required
    /// field is noise in a column every reader treats as a probability.
    pub confidence: Option<f64>,
    /// Why, in a sentence, when there is one to give.
    pub reasoning: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SearchMode {
    #[default]
    All,
    Any,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchOptions {
    pub kind: Option<String>,
    pub project: Option<String>,
    pub scope: Option<String>,
    pub limit: Option<usize>,
    pub mode: SearchMode,
}

/// A memory named, without its body.
///
/// For the places that list memories rather than read them: an id, what kind it
/// is, and what it is called. The prompt hook ranked twenty-four candidates to
/// print three lines and was fetching every column of all of them — on a real
/// store, fifty-eight kilobytes of content to use nothing but the titles, which
/// turned a three-millisecond query into a hundred and sixty-five.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryRef {
    pub id: i64,
    /// The replication identifier, which is what relations are keyed on.
    ///
    /// Carried even though nothing prints it, because a memory named without it
    /// cannot be asked what its neighbours say about it.
    pub sync_id: String,
    pub kind: String,
    pub title: String,
}

/// A reason to treat a named memory with care.
///
/// Only the two verbs that change what an agent should do. `related`,
/// `compatible` and `scoped` say two memories belong together, which is true
/// and does not change the answer; a memory that has been overturned or is
/// being argued with is a different matter, and handing one over without
/// saying so is worse than handing over nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caveat {
    /// How it reads from the named memory's side.
    pub verb: CaveatVerb,
    /// The memory on the other end, by the number a person can look up.
    pub other_id: i64,
    pub other_title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaveatVerb {
    SupersededBy,
    ConflictsWith,
}

impl CaveatVerb {
    pub const fn phrase(self) -> &'static str {
        match self {
            Self::SupersededBy => "superseded by",
            Self::ConflictsWith => "conflicts with",
        }
    }
}

/// One page of a list, and how long the whole list is.
///
/// The total is what makes a page navigable. "Page three" on its own says
/// nothing about whether there is a fourth, and a screen that cannot say how
/// much is left is one somebody scrolls hoping. It also lets a heading say
/// `1–100 of 3312` rather than `100`, which was a truncation reported as a
/// count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing<T> {
    pub rows: Vec<T>,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Stats {
    pub total_sessions: i64,
    pub total_observations: i64,
    pub total_prompts: i64,
    /// Projects that hold at least one live memory, most recently written
    /// first.
    ///
    /// Not the projects the store knows, which is what a bare `projects`
    /// beside three totals reads as: one holding only a session or a prompt is
    /// absent, and on a real store of nineteen that is two of them. The order
    /// is the useful part — it answers "where has anything been happening" —
    /// and `leteo projects list` is the inventory.
    pub projects: Vec<String>,
}

// No `Eq`: a relation carries a confidence, and a float has no total equality.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExportData {
    pub version: String,
    pub exported_at: String,
    #[serde(default, deserialize_with = "nullable_sequence")]
    pub sessions: Vec<Session>,
    #[serde(default, deserialize_with = "nullable_sequence")]
    pub observations: Vec<Observation>,
    #[serde(default, deserialize_with = "nullable_sequence")]
    pub prompts: Vec<Prompt>,
    /// The judged graph, which used to be left behind.
    ///
    /// An export was sessions, observations and prompts — so a store exported
    /// and imported back came home without a single relation. That is the
    /// expensive half of the data: a lexical scan proposes each pair and a
    /// language model rules on it, and since recall started reading the graph it
    /// is also what tells an agent a memory has been overturned.
    ///
    /// Adding the field keeps the format readable in both directions. Engram
    /// ignores a key it does not know, and `default` covers an export written
    /// before this existed — which is every Engram one, and every Leteo one so
    /// far.
    #[serde(default, deserialize_with = "nullable_sequence")]
    pub relations: Vec<Relation>,
}

/// Reads a list that may arrive as `null` instead of `[]`.
///
/// Go marshals an empty slice as `null`, so an export produced by Engram — the
/// project Leteo is compatible with — writes `"prompts": null` whenever there
/// are no prompts. `#[serde(default)]` alone does not cover that: it fills in a
/// missing field, not a present-but-null one.
fn nullable_sequence<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportResult {
    pub sessions_imported: i64,
    pub observations_imported: i64,
    pub prompts_imported: i64,
    pub relations_imported: i64,
    /// Relations whose source or target is not in this store.
    ///
    /// Counted rather than dropped in silence: an export of one project can
    /// hold a relation reaching a memory that lives in another, and somebody
    /// restoring it deserves to know the graph came back with holes.
    pub relations_skipped: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SyncMutation {
    pub seq: i64,
    pub target_key: String,
    pub entity: String,
    pub entity_key: String,
    pub op: String,
    pub payload: String,
    pub source: String,
    pub project: String,
    pub occurred_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acked_at: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncState {
    pub target_key: String,
    pub lifecycle: String,
    pub last_enqueued_seq: i64,
    pub last_acked_seq: i64,
    pub last_pulled_seq: i64,
    pub consecutive_failures: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backoff_until: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lease_owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lease_until: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncExportResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_id: Option<String>,
    pub sessions_exported: usize,
    pub observations_exported: usize,
    pub prompts_exported: usize,
    pub mutations_exported: usize,
    pub is_empty: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncImportResult {
    pub chunks_imported: usize,
    pub chunks_skipped: usize,
    pub sessions_imported: usize,
    pub observations_imported: usize,
    pub prompts_imported: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncStatus {
    pub local_chunks: usize,
    pub remote_chunks: usize,
    pub pending_import: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MergeResult {
    pub canonical: String,
    pub sources_merged: Vec<String>,
    pub observations_updated: i64,
    pub sessions_updated: i64,
    pub prompts_updated: i64,
    /// Whether the canonical project had to take over a source's enrolment.
    ///
    /// Reported because it is a change to what leaves this machine, and the
    /// caller asked to merge two names rather than to start replicating one.
    /// Said out loud, it reads as what it is: replication followed the
    /// memories.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub enrolment_moved: bool,
    /// Topic keys that now name more than one live memory in the canonical
    /// project.
    ///
    /// A topic key holds one memory per project and scope, and revising one
    /// finds it by that triple. Two projects may each have a memory under
    /// `architecture/wizard-split` — legitimately, they were different
    /// projects — and merging them puts both under the same triple. Nothing is
    /// lost, but the revision path takes the most recently updated of the two,
    /// so the other stops being reachable by its own key for good.
    ///
    /// Reported rather than resolved. Which of the two is the memory and which
    /// is the twin is a judgment about their contents, and a merge that
    /// silently threw one away would be a worse answer than a merge that says
    /// what it left behind. Zero on a real store of 2,117 keyed memories; it
    /// takes a merge to make one.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub topic_key_collisions: i64,
    /// Whether the name everything moved into held nothing before the merge.
    ///
    /// Merging into a name the store has never seen is a rename, and there is
    /// no other way to perform one, so it is allowed. It is also what a typo in
    /// `to` looks like, and the two are the same call: a whole project walks
    /// into a misspelling, the reply says it succeeded, and the memories are
    /// findable only under the mistake. Every other write refuses a project
    /// name nobody invented — `project_exists` was written for exactly that and
    /// this path never asked it.
    ///
    /// Reported rather than refused, for the reason the collisions above are:
    /// which of the two this was is the caller's to know, and a merge that
    /// refused the rename would remove the only way to do it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub canonical_created: bool,
}

fn is_zero(value: &i64) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ForeignKeyViolation {
    pub table: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_id: Option<i64>,
    pub parent: String,
    pub foreign_key_index: i64,
}

/// One named diagnostic.
///
/// The code is stable and is what `leteo doctor --check` and the `check`
/// argument of `mem_doctor` select on, so a caller can ask about one thing
/// instead of reading the whole report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DoctorCheck {
    pub code: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// What one full-text index held before a rebuild and after it.
///
/// Both numbers, because the one that matters is the difference: a rebuild that
/// changes nothing says the index was fine and the problem is elsewhere, and
/// that is worth as much as one that fills an empty index.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexRebuild {
    pub index: String,
    pub rows_before: i64,
    pub rows_after: i64,
}

impl DoctorCheck {
    pub fn passed(code: &str) -> Self {
        Self {
            code: code.to_owned(),
            ok: true,
            detail: None,
        }
    }

    pub fn failed(code: &str, detail: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            ok: false,
            detail: Some(detail.into()),
        }
    }

    /// Every code the doctor can report, so an unknown `--check` can say what
    /// it accepts instead of silently matching nothing.
    pub const CODES: &'static [&'static str] = &[
        "sqlite_integrity",
        "foreign_keys",
        "observation_fts_integrity",
        "observation_exact_fts_integrity",
        "prompt_fts_integrity",
        "observation_fts_sync",
        "observation_exact_fts_sync",
        "prompt_fts_sync",
        "observation_hash_sync",
        "observation_type_searchable",
        "topic_key_uniqueness",
        "settings_readable",
        "full_text_triggers",
        "journal_mode",
        "busy_timeout",
    ];
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DoctorReport {
    pub healthy: bool,
    /// What this database is stamped at, and what the running build reads.
    ///
    /// Facts rather than a check: a store whose version this build does not
    /// understand is refused at `open`, so by the time a report exists the two
    /// agree. They are here because the numbers were nowhere else at all —
    /// answering "what is my store at?" meant opening it with something that
    /// was not Leteo, and the only place either number ever appeared was in
    /// the error raised by the binary that could not open it.
    ///
    /// That is the moment they are least useful. A newer build migrates a
    /// store the first time it opens one, silently and one way, and every
    /// older binary on the machine stops opening it from then on. Reported
    /// here, somebody can see what they have before installing rather than
    /// after.
    pub schema_version: i32,
    pub schema_supported: i32,
    /// Named diagnostics, in a stable order.
    pub checks: Vec<DoctorCheck>,
    pub integrity_check: Vec<String>,
    pub foreign_key_violations: Vec<ForeignKeyViolation>,
    pub observation_fts_ok: bool,
    pub prompt_fts_ok: bool,
    pub observations: i64,
    pub observation_fts_rows: i64,
    pub prompts: i64,
    pub prompt_fts_rows: i64,
    pub pending_mutations: i64,
    pub journal_mode: String,
    pub busy_timeout_ms: i64,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PassiveCapture {
    pub session_id: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub project: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PassiveCaptureResult {
    pub extracted: usize,
    pub saved: usize,
    pub duplicates: usize,
    /// Learnings past [`normalize::MAX_LEARNINGS`], which this capture did not
    /// keep.
    ///
    /// Counted rather than swallowed. The subagent's context is gone, but the
    /// agent reading the hook's answer still has the text it was handed, so
    /// this is the one number here it can act on.
    ///
    /// [`normalize::MAX_LEARNINGS`]: crate::memory::normalize::MAX_LEARNINGS
    pub dropped: usize,
}
