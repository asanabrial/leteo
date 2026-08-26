//! Leteo — persistent memory for coding agents.
//!
//! An agent forgets everything when its context ends. Leteo is the local SQLite
//! store that remembers for it: what was decided, what was fixed, what turned
//! out not to be true, and which of those is worth handing back the next time a
//! session opens.
//!
//! The modules below are grouped by what they do rather than by what they are
//! built out of, and read roughly in the order a memory travels.
//!
//! # What a memory is
//!
//! [`memory`] holds the model, the rules and the normalisation, and nothing
//! else. Measured rather than asserted: it imports no storage, no protocol and
//! no interface, which is what lets every rule be exercised without a database
//! and keeps the several paths that persist a memory from each deciding
//! differently.
//!
//! # Where memories live
//!
//! [`store`] is the SQLite adapter — statements, migrations, and the mapping
//! between rows and the model. SQLite is not an implementation detail here, it
//! is the product: local-first, one file, no server.
//!
//! # Getting them back
//!
//! [`recall`] assembles what an agent is handed when a session opens or its
//! context is compacted: recent sessions, pinned memories, and the rest folded
//! and truncated in an order that matters.
//!
//! # Keeping them elsewhere
//!
//! [`sync`] is the replication journal and the wire format; [`cloud`] is the
//! server, the dashboard, and the background push and pull. The local store
//! stays the source of truth and the cloud is a copy of it.
//!
//! # Who talks to it
//!
//! [`mcp`] for agents, [`cli`] for a terminal, [`tui`] for a screen, [`hooks`]
//! for the agent lifecycle, and [`setup`] for putting Leteo into the thirteen
//! coding agents it knows about. These are adapters: they translate, and the
//! behaviour they translate to lives further in.
//!
//! # Everything else
//!
//! [`project`] decides which project a directory belongs to, [`settings`] holds
//! what the person asked for, [`sardi`] is how any of it is said out loud, and
//! [`engram`], [`obsidian`] and [`llm`] reach the world outside.
//!
//! Words a person reads live in two modules and nowhere else: [`sardi`] for the
//! voice, where a count decides the wording, and [`i18n`] for everything with a
//! fixed shape. Prose written inline in a screen is prose no translation will
//! ever find.

pub mod cli;
pub mod cloud;
pub mod engram;
pub mod files;
pub mod hooks;
pub mod i18n;
pub mod llm;
pub mod mcp;
pub mod memory;
pub mod obsidian;
pub mod paths;
pub mod project;
pub mod recall;
pub mod sardi;
pub mod settings;
pub mod setup;
pub mod store;

/// The ranking statement, for the retrieval measurement under `tools/`.
///
/// Behind a feature that is off by default, so nothing here is part of what
/// Leteo publishes. It exists because that tool used to write its own copy of
/// this query — and a measurement of a statement the product does not
/// issue measures nothing. See `store::search::matching_observations_sql`.
#[cfg(feature = "measure")]
pub mod measure {
    pub use crate::memory::normalize::{fts_any_of, fts_terms, fts_within_project, prompt_terms};
    pub use crate::store::BM25_WEIGHTS;
    pub use crate::store::search::{FTS_EXACT, FTS_STEMMED, matching_observations_sql};
    // The prompt hint's own statement, its sample depth and its two floors.
    //
    // Here for the same reason the ranking statement is: "would a different
    // floor separate a store that holds the answer from one that does not" can
    // only be asked of the rule the product applies. Read from the constants
    // rather than copied, so a floor that changes changes the measurement too.
    pub const RECALL_SAMPLE: usize = crate::store::RECALL_SAMPLE;
    pub const MIN_RECALL_SAMPLE: usize = crate::store::MIN_RECALL_SAMPLE;
    pub const RECALL_MARGIN: f64 = crate::store::RECALL_MARGIN;
    pub const RECALL_MARGIN_UNSEEN: f64 = crate::store::RECALL_MARGIN_UNSEEN;

    pub fn prompt_recall_sql() -> String {
        crate::store::search::prompt_recall_sql()
    }

    pub fn worth_naming(rank: f64, median: f64, already_in_the_opening_block: bool) -> bool {
        crate::store::search::worth_naming(rank, median, already_in_the_opening_block)
    }
}
pub mod sync;
pub mod timestamp;
pub mod tui;

pub use memory::model::{
    AddObservation, AddOutcome, AddOutcomeKind, AddPrompt, Observation, Prompt, SearchMode,
    SearchOptions, SearchResult, Session, Stats,
};
pub use store::{Store, StoreConfig, StoreError};
