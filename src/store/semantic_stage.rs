//! The semantic stage: memories found by meaning when the words found none.
//!
//! The model and the numbers that belong to it are in [`crate::semantic`]. What
//! lives here is how the store keeps a vector for each memory, how it decides
//! one is out of date, and how the stage reads them back under exactly the
//! visibility every other stage keeps. When it runs, and what it does with the
//! answer, is `search_limited`'s business and `search.md` §15's.

use std::collections::BTreeMap;
use std::time::Duration;

use super::search::{Candidate, FUSION_CONSTANT, visible_observations};
use super::*;
use crate::semantic;
use model2vec_rs::model::StaticModel;

/// What a vector was computed from, written as SQL over `observations o`.
///
/// The content hash the row already carries on every write path, and the title,
/// which the hash does not cover. Compared in SQL against the stored copy, so
/// finding the stale rows reads two short columns per memory and none of the
/// text. A memory whose key changed is embedded again; nothing on the write
/// paths has to remember to say so, which is the reason the key is a function
/// of the row and not a flag set by whoever changed it.
///
/// A hash folds case and whitespace, which the model's tokenizer does not see
/// either, so a re-wrapped paragraph keeps its vector.
const SOURCE_KEY: &str = "ifnull(o.normalized_hash, '') || '|' || o.title";

/// How much of a body is read out of the database to make a vector from.
///
/// The model reads 128 tokens, a few hundred characters, and the tokenizer cuts
/// by characters before it counts tokens — so everything past this is read from
/// SQLite, copied, and thrown away unseen. A body is capped at 50,000 characters
/// on the way in.
const BODY_CHARS: usize = 4096;

/// How long the write that keeps new vectors waits for the lock.
///
/// Short, because this is a search, and a search should not stand behind
/// another process's write for the five seconds a write may. The vectors it did
/// not get to keep are used for this question anyway, and kept the next time.
const WRITE_PATIENCE: Duration = Duration::from_millis(250);

/// How the stage fails: something it could not do, said, and never a reason to
/// refuse the lexical answer that stands without it.
pub(super) type StageError = Box<dyn std::error::Error + Send + Sync>;

/// How many memories are embedded and kept at a time.
///
/// What bounds the first firing's memory. The store is read for the ids of what
/// is stale -- a number and a short key each -- and then the text of this many
/// memories at a time is read, embedded and written, so the text and the vectors
/// held at once are this many memories whatever the store holds: at most
/// 256 titles and 4,096-character bodies, about a megabyte, plus the vectors of
/// the same 256 (a kilobyte each). The model's own memory is separate and is
/// stated once in `search.md` §15. A store that cannot be written is the
/// exception, because the vectors made for the question are held until it is
/// answered: a kilobyte per memory in scope, which is what the rows would have
/// cost on disk.
const CHUNK: usize = 256;

/// How many memories one bounded backfill call embeds.
///
/// An import embeds this many and hands the rest to the background backfill;
/// `doctor --repair` loops it until the store is caught up. Neither reply may
/// wait on making vectors for a whole store, and this is the bound that keeps
/// each step short enough to sit between two other things.
pub(crate) const BACKFILL_BUDGET: usize = 1024;

struct Fresh {
    id: i64,
    key: String,
    /// Empty for a memory the model has no token for. It is kept as an empty
    /// row so that it is not found stale again on every question.
    vector: Vec<u8>,
}

impl Store {
    /// The memories nearest the question by meaning, best first.
    ///
    /// With a floor, only those at or above it: the answer to an empty
    /// question, where there is nothing to be wrong beside. Without one, the
    /// nearest `limit` regardless: the list that is fused with a lexical answer
    /// that was already being given.
    ///
    /// It reads the vectors that are already stored and writes nothing, so a
    /// search never holds the store's lock for the seconds a first firing on a
    /// large store used to cost. That work belongs to the write paths and the
    /// backfill now ([`Store::embed_written`], [`Store::backfill_step`]); a
    /// memory with no current vector is simply absent from the answer rather
    /// than embedded here.
    pub(super) fn semantic_candidates(
        &self,
        query: &str,
        options: &SearchOptions,
        limit: usize,
        floor: Option<f32>,
    ) -> Result<Vec<Candidate>, StageError> {
        let model = semantic::load(self.data_dir(), self.model_dir())?;
        let question = semantic::embed(&model, &[query.to_owned()])?.remove(0);
        if question.is_empty() {
            return Ok(Vec::new());
        }
        let visible = visible_observations(1, 2, 3);
        let mut statement = self.connection.prepare(&format!(
            "SELECT o.id, o.type, v.vector FROM observations o
             CROSS JOIN observation_vectors v ON v.observation_id = o.id
             WHERE {visible} AND (?4 IS NULL OR o.type != ?4)
               AND v.model = ?5 AND v.source_key = {SOURCE_KEY}"
        ))?;
        let mut scored: Vec<(f32, i64, String)> = Vec::new();
        let mut rows = statement.query(params![
            options.kind,
            options.project,
            options.scope,
            super::search::excludes_summaries(options)
                .then_some(crate::memory::model::SESSION_SUMMARY),
            semantic::MODEL_ID,
        ])?;
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let stored = row.get_ref(2)?.as_blob()?;
            if let Some(cosine) = semantic::cosine(stored, &question) {
                scored.push((cosine, id, row.get(1)?));
            }
        }

        scored.sort_by(|left, right| right.0.total_cmp(&left.0).then(left.1.cmp(&right.1)));
        if let Some(floor) = floor {
            scored.retain(|(cosine, ..)| *cosine >= floor);
        }
        scored.truncate(limit);
        Ok(scored
            .into_iter()
            .map(|(cosine, id, kind)| Candidate {
                id,
                kind,
                // Negative, and more negative is better, like every other rank.
                // The scale is the cosine's, so it compares rows of this page
                // and nothing else -- see `SearchResult::rank`.
                rank: -f64::from(cosine),
                partial: false,
                semantic: true,
            })
            .collect())
    }

    /// Embeds and keeps a current vector for the memories a write just touched.
    ///
    /// Called after the write's own transaction has committed, so the vector is
    /// written outside it and a failure to embed cannot roll back the memory.
    /// The model is loaded here if it is not up yet, so a new memory is
    /// searchable by meaning at once rather than waiting for the backfill: the
    /// first write in a process pays the load and the rest reuse it. The
    /// `semantic_search` setting off means no vectors are made at all.
    pub(crate) fn embed_written(&self, ids: &[i64]) {
        if ids.is_empty() || !self.semantic_enabled() {
            return;
        }
        let outcome = (|| {
            let model = semantic::load(self.data_dir(), self.model_dir())?;
            self.embed_and_keep(&model, self.stale_among(ids)?)
        })();
        if let Err(error) = outcome {
            if error.is::<crate::semantic::Unavailable>() {
                tracing::debug!(%error, "the semantic stage is off; the written memories are left for the backfill");
            } else {
                tracing::warn!(%error, "the semantic stage could not keep vectors for what was just written");
            }
        }
    }

    /// Embeds and keeps a current vector for up to `budget` memories that need
    /// one, newest first, across the whole store.
    ///
    /// The bounded backfill, one step at a time. It never loads the model: a
    /// process that has not searched yet has no reason to pay for it, and the
    /// thread that calls this waits until a search or a save has. Returns how
    /// many it embedded, so a caller looping can tell a full budget — more may
    /// remain — from a short one, which means it has caught up.
    pub(crate) fn backfill_step(&self, budget: usize) -> Result<usize, StageError> {
        if !self.semantic_enabled() || !self.model_loaded() {
            return Ok(0);
        }
        let model = semantic::load(self.data_dir(), self.model_dir())?;
        self.embed_and_keep(&model, self.stale_any(budget)?)
    }

    /// Embeds and keeps a current vector for up to `budget` memories that need
    /// one, loading the model if it is not up.
    ///
    /// `doctor --repair` calls it, because a person asked: the background
    /// backfill is bounded and may take a while to catch up on a store that has
    /// just been adopted or imported, and the repair is the one command that
    /// says "do it now".
    pub(crate) fn backfill_vectors(&self, budget: usize) -> Result<usize, StageError> {
        if !self.semantic_enabled() {
            return Ok(0);
        }
        let model = semantic::load(self.data_dir(), self.model_dir())?;
        self.embed_and_keep(&model, self.stale_any(budget)?)
    }

    /// Whether the `semantic_search` setting asks for vectors at all.
    fn semantic_enabled(&self) -> bool {
        crate::settings::load_beside(self.database_path()).semantic_search()
    }

    /// Whether a model is already up for this store, without loading one.
    fn model_loaded(&self) -> bool {
        semantic::is_loaded(self.data_dir(), self.model_dir())
    }

    /// How many of the memories the stage can reach have a current vector.
    ///
    /// The denominator is the visibility the scan itself uses — live and not
    /// superseded — session summaries included, because a query that names the
    /// type reads them (§15 of `search.md`). So the number is the fraction of
    /// the store the stage can see, not the fraction of every row that holds a
    /// vector. `doctor` reports it, and `(covered, total)` is what lets it say
    /// "every one" without a second query.
    pub(crate) fn vector_coverage(&self) -> Result<(i64, i64), rusqlite::Error> {
        let visible = visible_observations(1, 2, 3);
        let total: i64 = self.connection.query_row(
            &format!("SELECT COUNT(*) FROM observations o WHERE {visible}"),
            params![
                Option::<String>::None,
                Option::<String>::None,
                Option::<String>::None,
            ],
            |row| row.get(0),
        )?;
        let covered: i64 = self.connection.query_row(
            &format!(
                "SELECT COUNT(*) FROM observations o
                 CROSS JOIN observation_vectors v ON v.observation_id = o.id
                 WHERE {visible} AND v.model = ?4 AND v.source_key = {SOURCE_KEY}"
            ),
            params![
                Option::<String>::None,
                Option::<String>::None,
                Option::<String>::None,
                semantic::MODEL_ID,
            ],
            |row| row.get(0),
        )?;
        Ok((covered, total))
    }

    /// Reads, embeds and keeps the given stale memories, a chunk at a time.
    ///
    /// The chunk is what bounds the text and vectors held at once, as before;
    /// what changed is only who calls it. A memory gone since its id was read is
    /// skipped, and a batch whose write loses the lock is logged and dropped —
    /// the backfill's next step makes it again.
    fn embed_and_keep(
        &self,
        model: &StaticModel,
        stale: Vec<(i64, String)>,
    ) -> Result<usize, StageError> {
        if stale.is_empty() {
            return Ok(0);
        }
        let mut read = self.connection.prepare(&format!(
            "SELECT o.title, o.type, substr(o.content, 1, {BODY_CHARS})
             FROM observations o WHERE o.id = ?1"
        ))?;
        let mut kept = 0;
        for chunk in stale.chunks(CHUNK) {
            let mut rows = Vec::with_capacity(chunk.len());
            let mut texts = Vec::with_capacity(chunk.len());
            for (id, key) in chunk {
                // Gone since the ids were read: nothing to embed.
                let Some((title, kind, content)) = read
                    .query_row([id], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    })
                    .optional()?
                else {
                    continue;
                };
                texts.push(semantic::document_text(&title, &content));
                rows.push((*id, key.clone(), kind));
            }
            if rows.is_empty() {
                continue;
            }
            let vectors = semantic::embed(model, &texts)?;
            let fresh: Vec<Fresh> = rows
                .iter()
                .zip(vectors)
                .map(|((id, key, _), vector)| Fresh {
                    id: *id,
                    key: key.clone(),
                    vector: semantic::encode(&vector),
                })
                .collect();
            self.keep_vectors(&fresh)?;
            kept += fresh.len();
        }
        Ok(kept)
    }

    /// Every visible memory whose vector is missing, from another model, or made
    /// from text that has since changed, newest first, up to `budget`.
    ///
    /// Not scoped to a project: the backfill serves the whole store, and a write
    /// that just happened wants its own row found whether or not the writer
    /// named a project. Newest first so a save's own row is the first a bounded
    /// step reaches.
    ///
    /// Session summaries are included. The stage leaves them out of an ordinary
    /// answer, but a query that names the type reads them (§15 of `search.md`),
    /// so a vector for one is read rather than wasted.
    fn stale_any(&self, budget: usize) -> Result<Vec<(i64, String)>, StageError> {
        let visible = visible_observations(1, 2, 3);
        let mut statement = self.connection.prepare(&format!(
            "SELECT o.id, {SOURCE_KEY}
             FROM observations o
             LEFT JOIN observation_vectors v ON v.observation_id = o.id
             WHERE {visible}
               AND (v.observation_id IS NULL OR v.model != ?4 OR v.source_key != {SOURCE_KEY})
             ORDER BY o.id DESC LIMIT ?5"
        ))?;
        let rows = statement.query_map(
            params![
                Option::<String>::None,
                Option::<String>::None,
                Option::<String>::None,
                semantic::MODEL_ID,
                budget as i64,
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// The memories among `ids` whose vector needs making, in the order given.
    ///
    /// The ids come from this crate's own writes and are integers, so they are
    /// written into the `IN` list rather than bound one by one; nothing a caller
    /// supplied reaches this string. An empty list is not a question this asks —
    /// `embed_written` returns before it would.
    fn stale_among(&self, ids: &[i64]) -> Result<Vec<(i64, String)>, StageError> {
        let visible = visible_observations(1, 2, 3);
        let list = ids
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let mut statement = self.connection.prepare(&format!(
            "SELECT o.id, {SOURCE_KEY}
             FROM observations o
             LEFT JOIN observation_vectors v ON v.observation_id = o.id
             WHERE {visible} AND o.id IN ({list})
               AND (v.observation_id IS NULL OR v.model != ?4 OR v.source_key != {SOURCE_KEY})"
        ))?;
        let rows = statement.query_map(
            params![
                Option::<String>::None,
                Option::<String>::None,
                Option::<String>::None,
                semantic::MODEL_ID,
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Writes vectors in one transaction, waiting only briefly for the lock.
    ///
    /// `INSERT OR REPLACE` because a stale row is replaced rather than added to.
    /// A memory hard-deleted between reading it and getting here is skipped by
    /// the `EXISTS`: the foreign key would refuse the row, and one memory gone
    /// must not cost every other vector in the batch.
    fn keep_vectors(&self, fresh: &[Fresh]) -> Result<(), rusqlite::Error> {
        let before: i64 = self
            .connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))?;
        self.connection.busy_timeout(WRITE_PATIENCE)?;
        let written = (|| {
            let transaction =
                Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
            {
                let mut insert = transaction.prepare(
                    "INSERT OR REPLACE INTO observation_vectors
                       (observation_id, model, source_key, vector)
                     SELECT ?1, ?2, ?3, ?4
                     WHERE EXISTS (SELECT 1 FROM observations WHERE id = ?1)",
                )?;
                for row in fresh {
                    insert.execute(params![row.id, semantic::MODEL_ID, row.key, row.vector])?;
                }
            }
            transaction.commit()
        })();
        // Put back what the open decided, whether or not the write happened.
        self.connection
            .busy_timeout(Duration::from_millis(before.max(0) as u64))?;
        written
    }
}

/// A lexical answer and a semantic one, merged by where each put a memory.
///
/// The same reciprocal rank fusion the two full-text indexes are merged with,
/// and for the same reason: the scores cannot be added. A bm25 is scaled by the
/// store it came from and a cosine by the model, so places compare and scores
/// do not.
///
/// Only a memory the lexical list did not have is marked semantic. The semantic
/// list here has no floor, so it holds the nearest `limit` memories whatever
/// their cosine, and a memory it merely agrees with has been found by its words
/// already; marking that one too would label every row of the page and say
/// nothing. A memory in both lists keeps its lexical row and its `partial`, and its
/// place is the sum. Every row's `rank` becomes the negated fused score, because
/// the two lists' own scales cannot sit on one page.
pub(super) fn fuse(
    lexical: Vec<Candidate>,
    semantic: Vec<Candidate>,
    limit: usize,
) -> Vec<Candidate> {
    let mut scores: BTreeMap<i64, f64> = BTreeMap::new();
    let mut rows: BTreeMap<i64, Candidate> = BTreeMap::new();
    let mut order: BTreeMap<i64, usize> = BTreeMap::new();
    for list in [lexical, semantic] {
        for (place, candidate) in list.into_iter().enumerate() {
            *scores.entry(candidate.id).or_default() +=
                1.0 / (FUSION_CONSTANT + place as f64 + 1.0);
            order.entry(candidate.id).or_insert(place);
            rows.entry(candidate.id).or_insert(candidate);
        }
    }
    let mut merged: Vec<Candidate> = rows.into_values().collect();
    // One scale for the page. A lexical row carries a bm25 and a semantic row a
    // cosine, which are not comparable, and the page is ordered by the fused
    // score; so that is what each row says, negated to stay "more negative is
    // better". Left as they were, a reader sorting by `rank` would put a
    // semantic row, at about -0.4, below every lexical one at about -10.
    for row in &mut merged {
        row.rank = -scores[&row.id];
    }
    merged.sort_by(|left, right| {
        scores[&right.id]
            .total_cmp(&scores[&left.id])
            .then_with(|| order[&left.id].cmp(&order[&right.id]))
            .then_with(|| left.id.cmp(&right.id))
    });
    merged.truncate(limit);
    merged
}
