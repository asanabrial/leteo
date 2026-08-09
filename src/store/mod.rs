use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;

use crate::{
    memory::model::{
        AddObservation, AddOutcome, AddOutcomeKind, AddPrompt, Candidate, CandidateOptions, Caveat,
        CaveatVerb, DeferredRow, DeleteProjectResult, DeleteSessionResult, DoctorCheck,
        DoctorReport, ExportData, ForeignKeyViolation, ImportResult, IndexRebuild,
        JudgeBySemanticParams, JudgeRelationParams, ListDeferredOptions, ListRelationsOptions,
        Listing, MemoryRef, MergeResult, Observation, PassiveCapture, PassiveCaptureResult,
        PendingPair, PendingSide, ProjectStats, Prompt, PruneResult, Relation, RelationListItem,
        RelationStats, ReplayDeferredResult, SaveRelationParams, ScanOptions, ScanResult,
        SearchMode, SearchOptions, SearchResult, Session, SessionSummary, Stats, SyncMutation,
        SyncState, TimelineEntry, TimelineResult, UpdateObservation,
    },
    memory::normalize,
};

/// How much better than an ordinary match the best hit must be.
///
/// Swept against a hundred and twenty prompt-to-memory pairs taken from a real
/// store, where the memory saved in the same session right after a prompt is
/// taken to be what that prompt was about. That is a proxy rather than a label,
/// but it comes from the data instead of from somebody's eye, and there are
/// enough of them to draw a curve:
///
/// | margin | fires | delivers the memory | of what it fires |
/// |--------|-------|---------------------|------------------|
/// | 1.0    | 100%  | 28%                 | 28%              |
/// | 1.4    |  69%  | 24%                 | 34%              |
/// | 1.6    |  52%  | 22%                 | 42%              |
/// | 2.0    |  33%  | 14%                 | 42%              |
/// | 2.5    |  19%  |  9%                 | 47%              |
///
/// 1.6 is the knee. It is no less accurate than 2.0 in what it chooses to say —
/// both are right about two times in five — and it says it half as often again,
/// so half the cases 2.0 threw away come back. Below 1.6 accuracy falls away
/// without buying much more reach, which is the point where a hint turns into
/// noise the agent learns to skip.
///
/// The ceiling is 28%: what the top three give with no test at all. That is the
/// limit of matching words rather than meanings, not of this number — a third
/// of the misses are a prompt written in Spanish against a memory written in
/// English, which no threshold can fix.
const RECALL_MARGIN: f64 = 1.6;

/// The same test, for a memory the session did not open with.
///
/// Looser, because what it buys is different: naming one of the memories the
/// opening block already listed ranks what the agent has, while naming one from
/// further back is the only way it hears of that memory at all. The measurement
/// and the rows that rule out simply lowering `RECALL_MARGIN` instead are in
/// `prompt_matches`.
const RECALL_MARGIN_UNSEEN: f64 = 1.2;

/// How much better than the ordinary a memory must score to be proposed as
/// something a save might contradict.
///
/// `find_candidates` gated on an absolute bm25 floor, and the note that stood
/// where this is now said why that could not work: bm25 grows with how many
/// terms a query has and how rare they are, so no fixed number means the same
/// thing for two different titles, and 399 of 400 real saves got the full three
/// proposals. It also said what was missing — a margin relative to the query,
/// the way `nearest_observations` does it — and that there was no label to
/// choose one against, since the only pairs on record were proposed by this
/// same finder while it compared its floor backwards.
///
/// The label is two sets neither of which this finder chose. The positives are
/// restatements of memories the store already holds, where the right answer is
/// known because it was built from it: sixty-six of them at two difficulties,
/// one dropping a word from the title and one keeping two words in three and
/// reversing them. The control is nineteen memories written about things the
/// project has never had anything to do with — a paella, the moons of Jupiter,
/// a slate roof — where the right answer is silence. Measured through this
/// crate on a copy of a real store, 1,712 memories in the project:
///
/// ```text
///                     restated found      off-domain memories
///                    reworded  rewritten   that got a proposal  proposals
///   absolute floor      92%       95%          19 of 19             57
///   median × 1.15       92%       95%          14 of 19             34
///   median × 1.25       91%       95%           9 of 19             17
///   median × 1.40       89%       94%           8 of 19             11
/// ```
///
/// 1.25 costs one restatement in sixty-six and takes forty of the fifty-seven
/// noise proposals with it. The noise is the expensive side: the skill tells an
/// agent to judge every candidate, so a memory about a paella was raising three
/// questions about git branch flow. When that was measured the skill also sent
/// those questions to the *user*, which is what made the noise unbearable
/// rather than merely wasteful; the verdicts are the agent's own now, and the
/// margin still earns its keep, because every proposal is a `mem_judge` call
/// and forty fewer of them is forty calls nobody makes.
///
/// It halves the noise rather than ending it. Nine of nineteen memories from
/// outside the project still get a proposal, because a title's words always
/// match something in a project of seventeen hundred memories, and no margin
/// on this query fixes that. What would is a different question from "which of
/// these scored best".
const CANDIDATE_MARGIN: f64 = 1.25;

/// How much of a project the opening block is taken to have named.
///
/// The same number `ContextSize::Full` hands over, and deliberately not read
/// from the setting: somebody on `slim` sees twenty, and treating the other
/// thirty as unseen would make the hint noisier for the person who asked for
/// less. The strict floor covers what a full opening would have shown.
const RECALL_RECENT_BLOCK: usize = 50;

/// How many candidates the per-prompt hint scores before it judges any of them.
///
/// This is what "an ordinary match for this query" means: the floor is the
/// median of these, so the number of them decides where the floor lands. It is
/// a real optimum and not a plateau. Over 277 real prompts against a
/// leak-free label, held-out half:
///
/// ```text
///   depth    speaks   right
///      8      49.7%    6.3%
///     12      69.9%   18.2%
///     24      86.7%   31.5%
///     40      87.4%   24.5%
///    100      90.2%   25.9%
/// ```
///
/// Deeper is looser, because bm25 is negative and the tail of a candidate list
/// is close to zero: adding worse matches pulls the median toward zero, which
/// pulls the floor with it. The hint then speaks more and is right less.
///
/// It was `(limit * 8).max(24)`, which is the same 24 at today's limit of
/// three and a trap at any other. Naming five from a sample of 24 measures
/// *better* than naming three — 39.2% against 36.0% — but the old expression
/// would have deepened the sample to 40 at the same time and landed on 24.5%.
/// Somebody raising the limit would have watched the hint get worse and
/// concluded that naming more memories is what hurt. How many are named and
/// what an ordinary match looks like are two questions, and only one of them
/// is the caller's.
pub(crate) const RECALL_SAMPLE: usize = 24;

/// Longest question the widened retry will relax.
///
/// It runs one query per word it leaves out, so the cost is linear in the
/// question. Past a dozen words, omitting one relaxes almost nothing anyway.
const MAX_WIDENED_TERMS: usize = 12;
/// Fewest matches worth calling a distribution.
const MIN_RECALL_SAMPLE: usize = 3;

/// Every column an [`Observation`] is built from.
///
/// Expressions are aliased so the result set always exposes the plain column
/// name, which is what `map_observation` reads. Rows are mapped by name rather
/// than by position: a list this long was mapped by index until adding one
/// column silently shifted a neighbouring value onto the wrong field.
const OBSERVATION_COLUMNS: &str = "id, ifnull(sync_id, '') AS sync_id, session_id, type, title, content, tool_name, project, scope, topic_key, revision_count, duplicate_count, last_seen_at, review_after, prompt_sync_id, pinned, created_at, updated_at, deleted_at";

/// The same columns, qualified for a join where bare names would be ambiguous.
///
/// `observations_fts` shares several column names with `observations`, so the
/// full-text search has to qualify them. The aliases keep the result set
/// identical to [`OBSERVATION_COLUMNS`], and a test holds the two in step.
const OBSERVATION_COLUMNS_JOINED: &str = "o.id, ifnull(o.sync_id, '') AS sync_id, o.session_id, o.type, o.title, o.content, o.tool_name, o.project, o.scope, o.topic_key, o.revision_count, o.duplicate_count, o.last_seen_at, o.review_after, o.prompt_sync_id, o.pinned, o.created_at, o.updated_at, o.deleted_at";

/// The narrowings a listing query applies, and the values they bind.
///
/// Every listing in the store takes an optional project, and the obvious way to
/// serve both cases with one prepared statement is
/// `WHERE (?1 IS NULL OR project = ?1)`. It reads well, and it costs a full
/// table scan on every call — including the calls that do name a project and
/// could have been answered from `idx_obs_project` in microseconds.
///
/// SQLite chooses its plan when the statement is prepared, before it knows what
/// `?1` will be bound to. A column inside a disjunction with a parameter is
/// therefore not a usable index term at plan time, and the plan it settles on
/// is the one that works whichever way the parameter goes: `SCAN`. Binding a
/// project does not rescue it, because the decision was already made.
///
/// On a store of 3,587 memories that is 5.7 ms against 0.015 ms, and the
/// session-opening context pays it four times over — recent sessions, pinned,
/// recent observations, recent prompts.
///
/// So the clause is present or absent rather than conditional, which means
/// building the SQL per call. That gives up one prepared-statement shape for
/// two; SQLite's statement cache holds both, and each is a plan that can use an
/// index.
struct Narrowing<'a> {
    clauses: String,
    values: Vec<&'a dyn rusqlite::ToSql>,
}

impl<'a> Narrowing<'a> {
    fn new() -> Self {
        Self {
            clauses: String::new(),
            values: Vec::new(),
        }
    }

    /// `AND <column> = ?n` when there is a value, and nothing at all when there
    /// is not — never a clause that has to be true for every row.
    fn equals(&mut self, column: &str, value: Option<&'a impl rusqlite::ToSql>) {
        if let Some(value) = value {
            self.values.push(value);
            let index = self.values.len();
            self.clauses.push_str(&format!(" AND {column} = ?{index}"));
        }
    }

    /// A value every call binds, such as `LIMIT`. Returns its placeholder
    /// number, because that depends on how many narrowings came first.
    fn bind(&mut self, value: &'a dyn rusqlite::ToSql) -> usize {
        self.values.push(value);
        self.values.len()
    }

    fn clauses(&self) -> &str {
        &self.clauses
    }

    fn values(self) -> impl Iterator<Item = &'a dyn rusqlite::ToSql> {
        self.values.into_iter()
    }
}

/// What each column of a memory is worth when a query is scored.
///
/// Title, content, tool name, type, project, topic key — in the order the
/// index declares them. Written once because it was written three times, and a
/// ranking rule that lives in three places is one edit away from a search that
/// disagrees with the hint that answers the same question.
///
/// The body is worth half a title word, not a fifth of one, and that number is
/// the measured part. Bodies here average three thousand characters, so a long
/// memory matches a handful of an ordinary question's words by coincidence
/// alone, and bm25's length normalisation does not fully undo it: what the
/// weight does is stop a paragraph that happens to contain your words from
/// outranking a title that states them.
///
/// Over 253 real prompts against a leak-free label — the memories saved
/// earlier in the same session, marked by the clock rather than by bm25:
///
/// ```text
///   content weight   speaks   right   right when it speaks
///        1.0 (was)    87.0%   24.1%          27.7%
///        0.5          91.3%   34.0%          37.2%
///        0.25         91.7%   34.4%          37.5%
///        0.0          89.3%   14.6%          16.4%
/// ```
///
/// It wins while speaking *more*, so this is not a floor loosened in disguise:
/// it is right more often on the prompts it answers. And 0.0 is there to show
/// the shape — the body carries what no title says, and dropping it entirely
/// costs more than the weight ever bought. 0.25 measures a case better than
/// 0.5 out of 253; 0.5 is taken for standing further from that cliff.
///
/// Six words lifted from a body — the case this weight exists for — are
/// answered first 87.7% of the time at 0.5, against 86.8% at 1.0 and 85.5% at
/// 0.0. Six from a title, 98.6% against 98.2%. So nothing is traded away.
pub const BM25_WEIGHTS: &str = "5.0, 0.5, 0.0, 0.0, 0.0, 3.0";

/// Every column a [`Prompt`] is built from.
const PROMPT_COLUMNS: &str = "id, ifnull(sync_id, '') AS sync_id, session_id, content, ifnull(project, '') AS project, created_at";

/// The same columns, qualified for the full-text join.
const PROMPT_COLUMNS_JOINED: &str = "p.id, ifnull(p.sync_id, '') AS sync_id, p.session_id, p.content, ifnull(p.project, '') AS project, p.created_at";

/// Every column a [`Relation`] is built from.
const RELATION_COLUMNS: &str = "id, sync_id, ifnull(source_id, '') AS source_id, ifnull(target_id, '') AS target_id, relation, reason, evidence, confidence, judgment_status, marked_by_actor, marked_by_kind, marked_by_model, session_id, created_at, updated_at";

/// Every column a relation is written with, in the order `params!` must follow.
///
/// The read counterpart above has been shared since it was written; the write
/// side was spelled out twice — once where an import inserts a relation and
/// refuses to overwrite one, once where replication upserts it. The two
/// statements differ on purpose and should not be merged, but the column list
/// is the one part that has to stay identical: adding a column to one and not
/// the other stores a relation with a field missing, and the fourteen
/// placeholders behind it make the mismatch silent rather than a syntax error.
const RELATION_INSERT_COLUMNS: &str = "sync_id, source_id, target_id, relation, reason, evidence, confidence, judgment_status, marked_by_actor, marked_by_kind, marked_by_model, session_id, created_at, updated_at";

/// Every column a [`SyncMutation`] is built from.
const SYNC_MUTATION_COLUMNS: &str = "seq, target_key, entity, entity_key, op, payload, source, ifnull(project, '') AS project, occurred_at, acked_at";

/// The version stamped into an export, and the only one an import accepts.
///
/// This describes the shape of the file, not the build that wrote it. It used
/// to be `CARGO_PKG_VERSION`, which meant the first version bump would have made
/// Leteo refuse both its own earlier exports and every Engram one — the two
/// formats are byte-compatible today only because the numbers happened to
/// coincide. Engram stamps this same value, so the interoperability is now
/// deliberate rather than accidental. Change it only when the format itself
/// changes in a way older readers cannot handle.
const EXPORT_FORMAT_VERSION: &str = "0.1.0";

pub use crate::memory::rules::{
    RELATION_COMPATIBLE, RELATION_CONFLICTS_WITH, RELATION_NOT_CONFLICT, RELATION_RELATED,
    RELATION_SCOPED, RELATION_SUPERSEDES,
};

/// Memories asked about in one caveat lookup.
///
/// Each one costs two bound parameters — it may be either end of a relation —
/// and SQLite refuses a statement past 32766 of them. `leteo context --limit`
/// takes any number, so a project big enough would have turned the whole
/// opening context into an error about SQL variables. Four hundred keeps the
/// statement well inside the oldest limit anybody ships.
const CAVEAT_LOOKUP_CHUNK: usize = 400;

pub const JUDGMENT_STATUS_PENDING: &str = "pending";
pub const JUDGMENT_STATUS_JUDGED: &str = "judged";
pub const JUDGMENT_STATUS_ORPHANED: &str = "orphaned";

pub const LOCAL_SYNC_TARGET: &str = "local";
/// Days an acknowledged journal row is kept before it is pruned.
const ACKED_MUTATION_RETENTION_DAYS: i64 = 7;
/// Deferred relation mutations retried per explicit replay.
const DEFERRED_REPLAY_BATCH: i64 = 50;
/// Attempts after which a deferred relation mutation is retired as dead.
const DEFERRED_DEAD_THRESHOLD: i64 = 5;

#[derive(Debug, Clone)]
pub struct StoreConfig {
    pub database_path: PathBuf,
    /// How long a writer waits for whoever holds the lock before giving up.
    ///
    /// Five seconds for everything that is not a hook. A hook sets its own,
    /// because it is the one caller with somebody standing over it: see
    /// `HookEvent::store_wait`.
    pub busy_timeout: Duration,
    pub max_observation_length: usize,
    pub max_context_results: usize,
    pub max_search_results: usize,
    pub dedupe_window: Duration,
}

impl StoreConfig {
    pub fn new(database_path: impl Into<PathBuf>) -> Self {
        Self {
            database_path: database_path.into(),
            busy_timeout: Duration::from_secs(5),
            max_observation_length: 50_000,
            max_context_results: 20,
            max_search_results: 20,
            dedupe_window: Duration::from_secs(15 * 60),
        }
    }

    pub fn in_data_dir(data_dir: impl AsRef<Path>) -> Self {
        Self::new(data_dir.as_ref().join("leteo.db"))
    }
}

impl StoreError {
    /// Whether this is another writer holding the lock rather than a failure.
    ///
    /// The one store error with a next step: the call did not happen, nothing
    /// is half-written, and doing it again in a moment is the whole of the
    /// remedy. Every other `Database` error is the store being broken, which is
    /// not something a caller can retry its way out of.
    /// What a busy store means, for whoever is reading.
    ///
    /// Three surfaces say it and each said something different: the tool
    /// surface named `store_busy` and a next step, the hooks printed whatever
    /// SQLite said, and the command line handed a person `Error code 5:
    /// database is locked` three times over with a cause chain. That last one
    /// is the prose of a corrupt file, about a store another process was merely
    /// writing to.
    ///
    /// One sentence, so the three agree. The fact behind it is the same
    /// everywhere: the call did not happen, nothing is half-written, and doing
    /// it again in a moment is the whole of the remedy.
    /// What to do when a subagent's learnings could not be written down.
    ///
    /// Beside `BUSY_ADVICE` because it is the same fact with a different
    /// remedy: there, call again; here, the caller has to call something else,
    /// with words only it still holds.
    pub const CAPTURE_RETRY: &'static str = "Leteo could not keep this subagent's learnings: another process was writing to the store. That text is gone with the subagent unless you send it — call mem_capture_passive with the subagent's final message.";

    /// The same loss said out loud, whatever refused the write.
    ///
    /// A busy store is the one cause a retry mends, so it is the one that asks
    /// for one. Every other cause loses the learnings just as completely, and
    /// the agent is told so with the cause it can act on rather than being sent
    /// to make the identical write fail a second time.
    pub fn capture_lost(&self) -> String {
        if self.is_busy() {
            return Self::CAPTURE_RETRY.to_owned();
        }
        format!(
            "Leteo could not keep this subagent's learnings: {self}. That text is gone with the subagent, and sending it again will fail the same way until this is fixed — run `leteo doctor`."
        )
    }
    pub const BUSY_ADVICE: &'static str = "another process is writing to the store, so this did nothing and nothing was half-written; try again in a moment";

    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            Self::Database(rusqlite::Error::SqliteFailure(failure, _))
                if matches!(
                    failure.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                )
        )
    }
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database path must be absolute: {0}")]
    RelativeDatabasePath(PathBuf),
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("observation not found: {0}")]
    ObservationNotFound(i64),
    #[error("search query cannot be empty")]
    EmptySearch,
    #[error("relation not found: {0}")]
    RelationNotFound(String),
    #[error(
        "invalid relation verb {given:?}; expected one of {}",
        crate::memory::rules::RELATION_VERBS.join(", ")
    )]
    InvalidRelationVerb { given: String },
    /// Two memories that do not belong to the same project.
    ///
    /// It said "cross-project relations are not allowed" and nothing else, so
    /// the caller was told the claim was refused and not which of the two ends
    /// was the odd one — with a pair of opaque sync ids in hand, that is a
    /// refusal nobody can act on. Naming both projects is the whole of the next
    /// step: either the wrong memory was named, or one of them is filed in the
    /// wrong place.
    #[error(
        "a relation joins two memories of one project, and these are in {source_project} and {target_project}"
    )]
    CrossProjectRelation {
        source_project: String,
        target_project: String,
    },
    #[error("project not found: {0}")]
    ProjectNotFound(String),
    #[error("session {0} still has {1} observation(s)")]
    SessionHasObservations(String, i64),
    #[error("prompt not found: {0}")]
    PromptNotFound(i64),
    /// The memory is there and it is deleted, which is not the same as absent.
    ///
    /// Five doors refused a soft-deleted memory with `observation_not_found` —
    /// the same words an id that never existed gets — while
    /// `mem_get_observation` handed the same id back with `state: deleted`. So
    /// the store knew the difference and said it in one place out of six. An
    /// agent holding an id from a caveat or an earlier search reads "not found"
    /// as its own mistake and stops, when what happened is somebody deleted it
    /// and the body is still there to read.
    ///
    /// What it says about the way back was wrong, and wrong in the direction
    /// this codebase names: it reported the nearest hopeful state rather than
    /// what happens. "Saving it again is what brings it back" — nothing brings
    /// it back. Both lookups a save does, the hash and the topic key, filter
    /// `deleted_at IS NULL`, so neither can see the deleted row; driven with
    /// the same title, the same body and the same topic key, the store ends
    /// with two rows, one dead under the old id and one live under a new one.
    /// An agent told this and holding an id from a caveat or an earlier search
    /// would follow the advice, get a different memory, and have no way to know
    /// the old id had not moved.
    #[error(
        "observation {id} was deleted on {deleted_at}; its body is still readable with mem_get_observation, and saving the same thing again writes a new memory rather than restoring this one"
    )]
    ObservationDeleted { id: i64, deleted_at: String },
    #[error(
        "this database is at schema version {found}, but this build of Leteo understands {supported}; upgrade Leteo to open it"
    )]
    SchemaTooNew { found: i32, supported: i32 },
    /// The file is Engram's, and Leteo has a command for that.
    ///
    /// Engram stamps `user_version = 1`, which is also what Leteo stamps a
    /// database it has converged to its own baseline — so the version cannot
    /// tell the two apart and the shape has to. Without this, pointing any
    /// command at an Engram database ran migrations written for Leteo's baseline
    /// against Engram's tables and came back with `no such table: prompts`: an
    /// internal name, a SQLite error code, and no mention of the one command
    /// that exists for exactly this file.
    #[error(
        "this is an Engram database, not a Leteo one: it has `user_prompts` where Leteo has `prompts`. Take its memories over with `leteo import --from-engram --source <path>`, which copies them into a Leteo store and leaves this file alone"
    )]
    EngramDatabase,
    /// A caller asked for something the store cannot do — an empty name, an
    /// unknown check code, a value out of range.
    ///
    /// This used to be smuggled inside `rusqlite::Error::InvalidParameterName`,
    /// which made every such message read as "database error: Invalid parameter
    /// name: ...", blamed SQLite for the caller's mistake, and left the HTTP
    /// layer sniffing message prefixes to tell one case from another.
    #[error("{0}")]
    InvalidParameter(String),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

pub struct Store {
    connection: Connection,
    config: StoreConfig,
    /// What `open` had left of its budget once the schema pass was done. See
    /// [`Store::budget_left_after_opening`].
    budget_left_after_opening: Duration,
}

impl Store {
    /// The most results a search will return, whatever it was asked for.
    ///
    /// Exposed because a caller that asked for more has no other way to tell a
    /// clamp from an exhausted store: both come back as a short list.
    pub fn max_search_results(&self) -> usize {
        self.config.max_search_results
    }

    /// The ceiling on a context read, which `mem_timeline` publishes.
    ///
    /// Beside its sibling and not folded into it: the two happen to be twenty
    /// apiece, and a guard that reached for whichever accessor existed would
    /// pass on that coincidence rather than on the number it means. This
    /// repository has already paid for one of those — a context size of twenty
    /// agreed with `slim` by accident and hid a defect for months.
    pub fn max_context_results(&self) -> usize {
        self.config.max_context_results
    }

    pub fn open(config: StoreConfig) -> Result<Self, StoreError> {
        if !config.database_path.is_absolute() {
            return Err(StoreError::RelativeDatabasePath(config.database_path));
        }
        if let Some(parent) = config.database_path.parent() {
            fs::create_dir_all(parent)?;
        }
        // One deadline for the whole open, not one per thing that waits.
        //
        // `prepare` waits out another process converting the journal, and then
        // every statement after it waits again on `busy_timeout`. Given the
        // same number, those two spend it twice: a hook told it had two seconds
        // took four and a half against a held lock — two in the open, two in
        // the write — which is over the three seconds its agent allows before
        // killing it. So the second wait gets what the first one left.
        let deadline = std::time::Instant::now() + config.busy_timeout;
        let connection = Connection::open(&config.database_path)?;
        connection.busy_timeout(config.busy_timeout)?;
        prepare(&connection, config.busy_timeout)?;
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        connection.busy_timeout(left)?;
        Ok(Self {
            connection,
            config,
            budget_left_after_opening: left,
        })
    }

    /// How much of the open budget was left for the statements that follow.
    ///
    /// Kept because it is the only honest way to check the rule above. Timing
    /// the whole open instead measures the machine as much as the store: a
    /// budget spent once and a budget spent twice are one budget apart, and a
    /// loaded test runner stalls by more than that — which is how the guard for
    /// this failed three times over one session while the code was correct,
    /// once at 4.13 seconds against a bound of 3.6 that the same machine had
    /// cleared at 2.04 an hour earlier.
    ///
    /// This is the same fact without the clock in it. Against a lock somebody
    /// else holds, `prepare` spends time and what is left has to be *less* than
    /// the budget it started with; a stall only makes that more true, and the
    /// defect — a second full budget — makes it exactly equal.
    pub fn budget_left_after_opening(&self) -> Duration {
        self.budget_left_after_opening
    }

    /// Begins a transaction that is going to write.
    ///
    /// `BEGIN IMMEDIATE` takes the write lock up front. rusqlite's default is
    /// `BEGIN DEFERRED`, which starts as a reader and upgrades on its first
    /// write — and SQLite answers a failed upgrade with `SQLITE_BUSY`
    /// *without consulting the busy handler*, because blocking there could
    /// deadlock two readers that each want to upgrade. The five-second
    /// `busy_timeout` set in `open` therefore never applies to a deferred
    /// writer, and a second one fails instantly with "database is locked".
    ///
    /// Leteo is multi-writer by design: the MCP server, the HTTP server, the
    /// lifecycle hooks, the CLI and the background autosync thread all open the
    /// same file. Taking the lock at `BEGIN` is what makes the timeout mean
    /// what it says.
    fn write_transaction(&mut self) -> Result<Transaction<'_>, StoreError> {
        Ok(self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?)
    }

    /// The open connection, for callers that need to reach past the store's
    /// own vocabulary.
    ///
    /// Adoption is the one such caller: it attaches another database and moves
    /// rows across wholesale, which no typed method can express.
    pub(crate) fn connection(&self) -> &Connection {
        &self.connection
    }

    pub fn database_path(&self) -> &Path {
        &self.config.database_path
    }
}

pub fn suggest_topic_key(kind: &str, title: &str, content: &str) -> String {
    normalize::suggest_topic_key(kind, title, content)
}

pub fn extract_learnings(text: &str) -> Vec<String> {
    normalize::extract_learnings(text)
}

fn sqlite_now() -> String {
    crate::timestamp::now()
}

fn nonempty_or_now(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        sqlite_now()
    } else {
        value.to_owned()
    }
}

fn sync_payload_is_deleted(deleted: bool, hard_delete: bool, deleted_at: Option<&str>) -> bool {
    deleted || hard_delete || deleted_at.is_some_and(|deleted_at| !deleted_at.trim().is_empty())
}

fn normalize_comparable_timestamp(value: &str) -> String {
    let value = value.trim();
    chrono::DateTime::parse_from_rfc3339(value).map_or_else(
        |_| value.to_owned(),
        |timestamp| crate::timestamp::format(timestamp.with_timezone(&Utc).naive_utc()),
    )
}

fn ensure_session_tx(tx: &Transaction<'_>, id: &str) -> Result<(), StoreError> {
    let exists = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?1)",
        [id],
        |row| row.get::<_, bool>(0),
    )?;
    if exists {
        Ok(())
    } else {
        Err(StoreError::SessionNotFound(id.to_owned()))
    }
}

/// The first column of every row a one-parameter query returns.
///
/// Was two functions with byte-identical bodies, differing only in the type
/// they collected — which is what generics are for.
fn query_column<T: rusqlite::types::FromSql>(
    tx: &Transaction<'_>,
    sql: &str,
    value: &str,
) -> Result<Vec<T>, StoreError> {
    let mut statement = tx.prepare(sql)?;
    let rows = statement.query_map([value], |row| row.get(0))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::from)
}

fn query_count(connection: &Connection, sql: &str) -> Result<i64, StoreError> {
    Ok(connection.query_row(sql, [], |row| row.get(0))?)
}

/// The SQL that gives a replication target a row before anything reads it.
///
/// Four call sites opened with the same statement, because every one of them
/// then reads or writes a column of that row and a missing row reads as "never
/// synced". One constant so the four cannot disagree about what an unsynced
/// target looks like.
const ENSURE_SYNC_TARGET: &str =
    "INSERT OR IGNORE INTO sync_state (target_key, lifecycle, updated_at)
     VALUES (?1, 'idle', datetime('now'))";

/// Marks every relation that pointed at a memory now permanently gone.
///
/// A hard delete leaves relations whose other end no longer exists. Four places
/// remove a memory for good — one by id, one with its session, one with its
/// project, one because a peer removed it — and each carried its own copy of
/// this statement. Four copies of a rule is three chances to forget it, and
/// forgetting it leaves a relation pointing into nothing.
fn orphan_relations_tx(tx: &Transaction<'_>, sync_id: &str) -> Result<(), StoreError> {
    if sync_id.is_empty() {
        return Ok(());
    }
    tx.execute(
        "UPDATE memory_relations SET judgment_status = 'orphaned',
             updated_at = datetime('now')
         WHERE source_id = ?1 OR target_id = ?1",
        [sync_id],
    )?;
    Ok(())
}

/// Marks the pending proposals a memory leaves behind when it changes project.
///
/// A relation joins two memories of one project — `validate_cross_project_guard`
/// refuses anything else — so moving one end somewhere else leaves a proposal
/// that `mem_judge` will decline for as long as the store exists. Measured, not
/// assumed: two memories in `leteo`, a pair proposed, one moved to `otro`, and
/// the judgment comes back "a relation joins two memories of one project, and
/// these are in leteo and otro". Nothing marked it, so it stayed `pending`,
/// counted forever in a queue nobody could ever empty.
///
/// The same mark a hard deletion uses, because the state is the same one: a
/// relation nothing can judge any more. Reusing the word keeps every
/// `!= 'orphaned'` in the crate correct — the replication export has one —
/// where a second word for the same state would be four places to forget it,
/// which is the reason [`orphan_relations_tx`] exists at all.
///
/// Only the **pending** ones, and that narrowing is the whole care of this
/// function. A judged cross-project verdict is still read: `caveats_for` does
/// not filter by project, so a `supersedes` recorded before the move still
/// hangs its caveat on the memory it overturned, and marking that orphaned
/// would take a real warning off six surfaces to tidy up a proposal.
fn strand_relations_tx(
    tx: &Transaction<'_>,
    sync_id: &str,
    project: &str,
) -> Result<(), StoreError> {
    if sync_id.is_empty() {
        return Ok(());
    }
    tx.execute(
        &format!(
            "UPDATE memory_relations SET judgment_status = '{JUDGMENT_STATUS_ORPHANED}',
                 updated_at = datetime('now')
             WHERE judgment_status = '{JUDGMENT_STATUS_PENDING}'
               AND (source_id = ?1 OR target_id = ?1)
               AND ?2 <> ''
               AND EXISTS(
                     SELECT 1 FROM observations o
                      WHERE o.sync_id = CASE WHEN source_id = ?1 THEN target_id ELSE source_id END
                        AND ifnull(o.project, '') <> ''
                        AND o.project <> ?2)"
        ),
        params![sync_id, project],
    )?;
    Ok(())
}

fn invalid_parameter(message: impl Into<String>) -> StoreError {
    StoreError::InvalidParameter(message.into())
}

fn validate_relation_verb(relation: &str) -> Result<(), StoreError> {
    if crate::memory::rules::is_relation_verb(relation) {
        Ok(())
    } else {
        Err(StoreError::InvalidRelationVerb {
            given: relation.to_owned(),
        })
    }
}

fn validate_optional_confidence(confidence: Option<f64>) -> Result<(), StoreError> {
    confidence.map_or(Ok(()), validate_confidence)
}

fn validate_confidence(confidence: f64) -> Result<(), StoreError> {
    if crate::memory::rules::is_confidence(confidence) {
        Ok(())
    } else {
        Err(invalid_parameter("confidence must be between 0.0 and 1.0"))
    }
}

fn candidate_fts_query(title: &str) -> String {
    title
        .split_whitespace()
        .map(|word| word.replace('"', ""))
        .filter(|word| !word.is_empty())
        .map(|word| format!("\"{word}\""))
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn get_relation_tx(tx: &Transaction<'_>, sync_id: &str) -> Result<Relation, StoreError> {
    get_relation_tx_optional(tx, sync_id)?
        .ok_or_else(|| StoreError::RelationNotFound(sync_id.to_owned()))
}

fn get_relation_tx_optional(
    tx: &Transaction<'_>,
    sync_id: &str,
) -> Result<Option<Relation>, StoreError> {
    Ok(tx
        .query_row(
            &format!("SELECT {RELATION_COLUMNS} FROM memory_relations WHERE sync_id = ?1"),
            [sync_id],
            map_relation,
        )
        .optional()?)
}

fn observation_project_tx(tx: &Transaction<'_>, sync_id: &str) -> Result<String, StoreError> {
    Ok(tx
        .query_row(
            "SELECT coalesce(nullif(o.project, ''), s.project, '')
             FROM observations o
             LEFT JOIN sessions s ON s.id = o.session_id
             WHERE o.sync_id = ?1",
            [sync_id],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or_default())
}

/// Both ends of a claim, and the two things that have to be true about them.
///
/// A verdict is a claim about two memories, so both have to be there. The
/// replicated door has always said so — a relation whose ends have not arrived
/// is deferred rather than stored — and the two local doors never asked at all:
/// `observation_project_tx` answers with an empty project for a memory that is
/// not there, and an empty project is exactly what the cross-project check
/// below skips over. So a verdict could be recorded about a memory that had
/// been hard-deleted, and `mem_compare` would answer with a `sync_id` as though
/// something had been judged.
///
/// Found by driving four hundred operations in an order nobody wrote by hand:
/// thirteen relations ended up pointing at memories that no longer existed,
/// none of them marked orphaned, because they were created after the deletion
/// that would have marked them.
///
/// Hard deletion is the case that matters. A soft-deleted memory is still a
/// row, and a judgment about it is still about something the store holds; the
/// row going away is what leaves the claim about nothing.
fn validate_cross_project_guard(
    tx: &Transaction<'_>,
    source_id: &str,
    target_id: &str,
) -> Result<(String, String), StoreError> {
    for (label, sync_id) in [("source_id", source_id), ("target_id", target_id)] {
        let present: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM observations WHERE sync_id = ?1)",
            [sync_id],
            |row| row.get(0),
        )?;
        if !present {
            return Err(invalid_parameter(format!(
                "{label} {sync_id} is not a memory this store holds"
            )));
        }
    }
    let source_project = observation_project_tx(tx, source_id)?;
    let target_project = observation_project_tx(tx, target_id)?;
    if !source_project.is_empty() && !target_project.is_empty() && source_project != target_project
    {
        return Err(StoreError::CrossProjectRelation {
            source_project,
            target_project,
        });
    }
    Ok((source_project, target_project))
}

fn project_merge_variants(raw: &str, normalized: &str, canonical: &str) -> BTreeSet<String> {
    let mut variants = BTreeSet::new();
    let raw = raw.trim().to_lowercase();
    for candidate in [raw, normalized.to_owned()] {
        if !candidate.is_empty() && normalize::project(&candidate) != canonical {
            variants.insert(candidate);
        }
    }
    let parts = normalized
        .split([' ', '-', '_'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() > 1 {
        for separator in [" ", "-", "_"] {
            let candidate = parts.join(separator);
            if normalize::project(&candidate) != canonical {
                variants.insert(candidate);
            }
        }
    }
    variants
}

#[derive(Debug)]
struct TableColumn {
    name: String,
    primary_key: i64,
}

fn collect_sessions_tx(tx: &Transaction<'_>, project: &str) -> Result<Vec<String>, StoreError> {
    query_column(tx, "SELECT id FROM sessions WHERE project = ?1", project)
}

/// Whether this project replicates anywhere.
///
/// An empty project name means the row could not be attributed to one, and
/// those are journalled regardless: dropping a mutation because its project
/// could not be worked out would lose data, while keeping it costs a row.
mod rows;
mod schema;
mod wire;

use rows::*;
pub(crate) use schema::SUMMARY_HEADLINE_CHARS;
use schema::*;
use wire::*;
pub(crate) mod search;
pub(crate) use search::DEFAULT_SEARCH_LIMIT;

mod observations;

mod sessions;

mod prompts;
pub use prompts::PROMPT_ATTRIBUTION_MINUTES;

mod projects;

mod relations;

mod replication;

mod diagnostics;

#[cfg(test)]
mod tests;
