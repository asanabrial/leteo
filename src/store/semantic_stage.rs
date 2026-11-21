//! The semantic stage: memories found by meaning when the words found none.
//!
//! The model and the numbers that belong to it are in [`crate::semantic`]. What
//! lives here is how the store keeps a vector for each memory, how it decides
//! one is out of date, and how the stage reads them back under exactly the
//! visibility every other stage keeps. When it runs, and what it does with the
//! answer, is `search_limited`'s business.

use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use super::search::{Candidate, FUSION_CONSTANT, visible_observations};
use super::*;
use crate::semantic::{self, SemanticModel};

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

/// Whether the vector row `v` is current for the memory `o`, as SQL: made by
/// this model, cut by this windowing, from the text the memory holds now.
/// `first` is the number of the parameter bound to [`semantic::MODEL_ID`]; the
/// next one is bound to [`semantic::WINDOWING`].
///
/// The one rule the scan, the coverage and both stale queries read, so a row
/// the scan would use is never one the backfill calls stale or the other way
/// round. Every row of a memory is written together, so the rows that count
/// memories ask it of row 0 alone and the scan asks it of every row.
fn current(first: usize) -> String {
    format!(
        "v.model = ?{first} AND v.windowing = ?{} AND v.source_key = {SOURCE_KEY}",
        first + 1
    )
}

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
/// is stale -- a number and a short key each -- and then this many memories at
/// a time are read, embedded and written. The text is held one memory at a
/// time: each is read, cut into windows and pooled before the next is read, so
/// the text held at once is one title and one body of at most 50,000
/// characters. What the chunk holds is the vectors, at most
/// [`semantic::MAX_WINDOWS`] kilobytes a memory: 8 MB for a chunk of the
/// longest memories, and about 1.7 MB at the 6.6 windows a memory averaged on
/// the measured store. The model's own memory is separate and is not counted
/// here.
const CHUNK: usize = 256;

/// What a full pass over the store did, and what it left.
#[derive(Debug, Clone, serde::Serialize)]
pub struct VectorPass {
    /// Memories given a vector by this pass.
    pub embedded: usize,
    /// Memories the stage can reach that still have no current vector. `None`
    /// when the coverage could not be read, which is not zero.
    pub without_vector: Option<i64>,
    /// Why the rest have none, when some do not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

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
    /// One per window, in order, never none. A window the model has no token
    /// for is kept as an empty row, so a memory with none at all still has its
    /// row 0 and is not found stale again on every question.
    vectors: Vec<Vec<u8>>,
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
    ///
    /// `found_by_words` are the memories the lexical list beside this one
    /// already holds; [`WindowScores`] scores those by their opening window and
    /// every other memory by its best. On the empty-answer path it is empty.
    ///
    /// One linear scan over every window of every memory in scope. Measured
    /// through the release binary on a synthetic temporary store of 5,000
    /// memories with 420-word bodies, over `leteo mcp`, 200 searches each with
    /// the stage on and off: 6.01 windows a memory, and the stage's share of a
    /// search went from 7.4 and 11.6 ms (two runs at one vector a memory) to
    /// 19.7 and 20.0 ms (two runs). The machine is noisy at that scale -- two
    /// runs of the same lexical path differed by 10 ms -- so the ratio, about
    /// twice, is the number to keep rather than the milliseconds.
    pub(super) fn semantic_candidates(
        &self,
        query: &str,
        options: &SearchOptions,
        limit: usize,
        floor: Option<f32>,
        found_by_words: &HashSet<i64>,
    ) -> Result<Vec<Candidate>, StageError> {
        let model = semantic::load(self.data_dir(), self.model_dir())?;
        let question = semantic::embed(&model, &[query.to_owned()])?.remove(0);
        if question.is_empty() {
            return Ok(Vec::new());
        }
        let visible = visible_observations(1, 2, 3);
        let current = current(5);
        let mut statement = self.connection.prepare(&format!(
            "SELECT o.id, o.type, v.vector, v.ordinal FROM observations o
             CROSS JOIN observation_vectors v ON v.observation_id = o.id
             WHERE {visible} AND (?4 IS NULL OR o.type != ?4) AND {current}"
        ))?;
        let mut scores = WindowScores::new(found_by_words);
        let mut rows = statement.query(params![
            options.kind,
            options.project,
            options.scope,
            super::search::excludes_summaries(options)
                .then_some(crate::memory::model::SESSION_SUMMARY),
            semantic::MODEL_ID,
            semantic::WINDOWING,
        ])?;
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let ordinal: i64 = row.get(3)?;
            let stored = row.get_ref(2)?.as_blob()?;
            let Some(cosine) = semantic::cosine(stored, &question) else {
                continue;
            };
            scores.offer(id, ordinal, cosine, || row.get::<_, String>(1))?;
        }

        let mut scored = scores.ranked();
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

    /// Embeds every memory that needs a vector, a bounded step at a time, and
    /// says what it could not do.
    ///
    /// The one loop `doctor --repair` and the two imports share. An import is a
    /// one-shot process: the bounded step inside it hands the rest to a
    /// background backfill that a command-line run never starts, so whatever
    /// the first step did not reach would stay without a vector until somebody
    /// ran a repair. A model that is not installed is not a failure of the
    /// import, and is reported here rather than turned into a count of zero,
    /// which would read as "nothing was missing".
    pub(crate) fn embed_pending(&self) -> VectorPass {
        let mut embedded = 0usize;
        let mut reason = None;
        if !self.semantic_enabled() {
            reason = Some(
                "semantic search is turned off by the semantic_search setting, so no vector is made"
                    .to_owned(),
            );
        } else {
            loop {
                match self.backfill_vectors(BACKFILL_BUDGET) {
                    Ok(done) => {
                        embedded += done;
                        if done < BACKFILL_BUDGET {
                            break;
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "the vector backfill stopped early");
                        reason = Some(error.to_string());
                        break;
                    }
                }
            }
        }
        let without_vector = self
            .vector_coverage()
            .map(|(covered, total)| total - covered)
            .ok();
        // Left alone when everything has its vector, whatever stopped the loop
        // on the way: the reason explains a gap, and there is none.
        if without_vector == Some(0) {
            reason = None;
        }
        VectorPass {
            embedded,
            without_vector,
            reason,
        }
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
    /// type reads them. So the number is the fraction of
    /// the store the stage can see, not the fraction of every row that holds a
    /// vector. Both counts are of memories and not of windows: `covered` reads
    /// each memory's row 0 and nothing else. `doctor` reports it, and
    /// `(covered, total)` is what lets it say "every one" without a second
    /// query.
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
        let current = current(4);
        let covered: i64 = self.connection.query_row(
            &format!(
                "SELECT COUNT(*) FROM observations o
                 CROSS JOIN observation_vectors v ON v.observation_id = o.id AND v.ordinal = 0
                 WHERE {visible} AND {current}"
            ),
            params![
                Option::<String>::None,
                Option::<String>::None,
                Option::<String>::None,
                semantic::MODEL_ID,
                semantic::WINDOWING,
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
        model: &SemanticModel,
        stale: Vec<(i64, String)>,
    ) -> Result<usize, StageError> {
        if stale.is_empty() {
            return Ok(0);
        }
        // The whole body, because the windows read up to
        // `MAX_WINDOWS * MAX_TOKENS` tokens of it and `embed_document` cuts it
        // to those characters itself; a cut here would be a second copy of
        // that budget, and the 4,096 characters it used to be left every window
        // past the first few unread.
        let mut read = self
            .connection
            .prepare("SELECT o.title, o.content FROM observations o WHERE o.id = ?1")?;
        let mut kept = 0;
        for chunk in stale.chunks(CHUNK) {
            let mut fresh = Vec::with_capacity(chunk.len());
            for (id, key) in chunk {
                // Gone since the ids were read: nothing to embed.
                let Some((title, content)) = read
                    .query_row([id], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .optional()?
                else {
                    continue;
                };
                let vectors = semantic::embed_document(model, &title, &content)?;
                fresh.push(Fresh {
                    id: *id,
                    key: key.clone(),
                    vectors: vectors
                        .iter()
                        .map(|vector| semantic::encode(vector))
                        .collect(),
                });
            }
            if fresh.is_empty() {
                continue;
            }
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
    /// answer, but a query that names the type reads them,
    /// so a vector for one is read rather than wasted.
    fn stale_any(&self, budget: usize) -> Result<Vec<(i64, String)>, StageError> {
        let visible = visible_observations(1, 2, 3);
        let current = current(4);
        let mut statement = self.connection.prepare(&format!(
            "SELECT o.id, {SOURCE_KEY}
             FROM observations o
             LEFT JOIN observation_vectors v ON v.observation_id = o.id AND v.ordinal = 0
             WHERE {visible}
               AND (v.observation_id IS NULL OR NOT ({current}))
             ORDER BY o.id DESC LIMIT ?6"
        ))?;
        let rows = statement.query_map(
            params![
                Option::<String>::None,
                Option::<String>::None,
                Option::<String>::None,
                semantic::MODEL_ID,
                semantic::WINDOWING,
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
        let current = current(4);
        let mut statement = self.connection.prepare(&format!(
            "SELECT o.id, {SOURCE_KEY}
             FROM observations o
             LEFT JOIN observation_vectors v ON v.observation_id = o.id AND v.ordinal = 0
             WHERE {visible} AND o.id IN ({list})
               AND (v.observation_id IS NULL OR NOT ({current}))"
        ))?;
        let rows = statement.query_map(
            params![
                Option::<String>::None,
                Option::<String>::None,
                Option::<String>::None,
                semantic::MODEL_ID,
                semantic::WINDOWING,
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Writes vectors in one transaction, waiting only briefly for the lock.
    ///
    /// A memory's rows are deleted before its windows are inserted, in the same
    /// transaction, because replacing by key alone would leave the windows past
    /// the new last one: a memory edited shorter would keep scoring by the text
    /// it no longer holds. A memory hard-deleted between reading it and getting
    /// here is skipped by the `EXISTS`: the foreign key would refuse the row,
    /// and one memory gone must not cost every other vector in the batch.
    fn keep_vectors(&self, fresh: &[Fresh]) -> Result<(), rusqlite::Error> {
        let before: i64 = self
            .connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))?;
        self.connection.busy_timeout(WRITE_PATIENCE)?;
        let written = (|| {
            let transaction =
                Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
            {
                let mut clear = transaction
                    .prepare("DELETE FROM observation_vectors WHERE observation_id = ?1")?;
                let mut insert = transaction.prepare(
                    "INSERT INTO observation_vectors
                       (observation_id, ordinal, model, windowing, source_key, vector)
                     SELECT ?1, ?2, ?3, ?4, ?5, ?6
                     WHERE EXISTS (SELECT 1 FROM observations WHERE id = ?1)",
                )?;
                for row in fresh {
                    clear.execute([row.id])?;
                    for (ordinal, vector) in row.vectors.iter().enumerate() {
                        insert.execute(params![
                            row.id,
                            ordinal as i64,
                            semantic::MODEL_ID,
                            semantic::WINDOWING,
                            row.key,
                            vector
                        ])?;
                    }
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

/// One score a memory from the cosines of its windows, as the scan streams
/// them: the opening window alone for a memory the words already found, the
/// best window for any other.
///
/// Best window for every memory was measured first, and it broke the merge
/// path. Through this binary, `tools/memory-bench/ratchet.py` fell below two
/// floors (MRR over all 117 questions 0.922 to 0.915 against a floor of 0.917,
/// paraphrases 0.871 to 0.849 against 0.856), and the whole fall was one
/// question that went from first place to sixth: five distractors whose bodies
/// end in a repeated boilerplate paragraph gained a later window of title plus
/// boilerplate, those windows reached the semantic top five, and since the
/// lexical list held the same five, `fuse` summed both places and lifted them
/// over the answer. A memory the words found is already on the page; what the
/// meaning adds about it is what it says it is about, which is its opening.
/// The tail of a body is what lets a memory the words missed be found at all,
/// so that one keeps its best window. Simulated on 420 blind questions of a
/// real store, this keeps first place on natural questions at 78.6% where best
/// window for every memory dropped it to 72.1%, and keeps the top-10 gain on
/// questions about the body (50.7% to 62.1% in the memory's language, 3.6% to
/// 25.7% in another). Do not fold the two cases back into one maximum.
///
/// A memory's windows arrive one after another, because `CROSS JOIN` keeps
/// `observations` as the outer loop and reads `observation_vectors` by its key,
/// so they fold into the last entry without a map and `extra` -- the type, a
/// string -- is read once a memory and not once a window. Should a plan ever
/// interleave them, [`Self::ranked`] still keeps one entry a memory, its best.
struct WindowScores<'a, T> {
    found_by_words: &'a HashSet<i64>,
    scored: Vec<(f32, i64, T)>,
}

impl<'a, T> WindowScores<'a, T> {
    fn new(found_by_words: &'a HashSet<i64>) -> Self {
        Self {
            found_by_words,
            scored: Vec::new(),
        }
    }

    fn offer<E>(
        &mut self,
        id: i64,
        ordinal: i64,
        cosine: f32,
        extra: impl FnOnce() -> Result<T, E>,
    ) -> Result<(), E> {
        if ordinal != 0 && self.found_by_words.contains(&id) {
            return Ok(());
        }
        match self.scored.last_mut() {
            Some(last) if last.1 == id => last.0 = last.0.max(cosine),
            _ => self.scored.push((cosine, id, extra()?)),
        }
        Ok(())
    }

    /// Best first, the lower id first at equal cosine, one entry a memory.
    fn ranked(mut self) -> Vec<(f32, i64, T)> {
        self.scored
            .sort_by(|left, right| right.0.total_cmp(&left.0).then(left.1.cmp(&right.1)));
        let mut seen = HashSet::with_capacity(self.scored.len());
        self.scored.retain(|(_, id, _)| seen.insert(*id));
        self.scored
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
    // At equal score, the memory the words found goes first.
    //
    // The two lists start at the same place, so the best lexical row and the
    // best semantic-only row always tie, and the tie used to fall to the lower
    // id: which of them headed the page was decided by when each memory was
    // saved. The semantic list has no floor here, so its best row is a guess,
    // and a guess was taking first place from an answer. Over 420 questions
    // written blind against a copy of a real store and asked through this
    // binary, a semantic-only row headed the page 102 times and was the memory
    // asked about once, and of the 150 pages where the lexical stage had put
    // the right memory first the merge moved it down on 36.
    //
    // `semantic` is true only for a row the lexical list did not hold, so it
    // is the origin this needs and `false` sorts first.
    merged.sort_by(|left, right| {
        scores[&right.id]
            .total_cmp(&scores[&left.id])
            .then_with(|| left.semantic.cmp(&right.semantic))
            .then_with(|| order[&left.id].cmp(&order[&right.id]))
            .then_with(|| left.id.cmp(&right.id))
    });
    merged.truncate(limit);
    merged
}

#[cfg(test)]
mod window_score_tests {
    use super::*;

    fn ranked(rows: &[(i64, i64, f32)], found_by_words: &[i64]) -> Vec<(f32, i64)> {
        let found: HashSet<i64> = found_by_words.iter().copied().collect();
        let mut scores = WindowScores::new(&found);
        for &(id, ordinal, cosine) in rows {
            scores
                .offer(id, ordinal, cosine, || Ok::<_, ()>(()))
                .unwrap();
        }
        scores
            .ranked()
            .into_iter()
            .map(|(cosine, id, ())| (cosine, id))
            .collect()
    }

    /// The breach this rule exists for, in miniature: memory 2 has a weak
    /// opening and a strong tail window, memory 1 a good opening. Held by the
    /// words, 2 is scored by its opening and stays behind 1; missed by them, it
    /// is scored by its best window and leads.
    #[test]
    fn a_memory_the_words_found_is_scored_by_its_opening_and_any_other_by_its_best() {
        let rows = [(1, 0, 0.5), (2, 0, 0.2), (2, 1, 0.9), (2, 2, 0.4)];
        assert_eq!(ranked(&rows, &[]), vec![(0.9, 2), (0.5, 1)]);
        assert_eq!(ranked(&rows, &[2]), vec![(0.5, 1), (0.2, 2)]);
        assert_eq!(
            ranked(&rows, &[1, 2]),
            vec![(0.5, 1), (0.2, 2)],
            "a memory the words found and whose opening is its best keeps it"
        );
    }

    /// Windows that do not arrive together still leave one entry a memory, at
    /// its best.
    #[test]
    fn interleaved_windows_still_give_one_candidate_a_memory() {
        let rows = [(1, 0, 0.3), (2, 0, 0.4), (1, 1, 0.8), (2, 1, 0.1)];
        assert_eq!(ranked(&rows, &[]), vec![(0.8, 1), (0.4, 2)]);
    }
}

#[cfg(test)]
mod fuse_tests {
    use super::*;

    fn candidate(id: i64, semantic: bool) -> Candidate {
        Candidate {
            id,
            kind: "discovery".to_owned(),
            rank: -1.0,
            partial: !semantic,
            semantic,
        }
    }

    fn ids(page: &[Candidate]) -> Vec<i64> {
        page.iter().map(|candidate| candidate.id).collect()
    }

    /// The ids are chosen so the old rule fails: the semantic rows are the
    /// older memories, and an id tie-break puts them first.
    #[test]
    fn a_tie_goes_to_the_memory_the_words_found() {
        let lexical = vec![candidate(30, false), candidate(40, false)];
        let semantic = vec![candidate(10, true), candidate(20, true)];
        let page = fuse(lexical, semantic, 10);
        assert_eq!(ids(&page), vec![30, 10, 40, 20]);
    }

    #[test]
    fn a_memory_both_lists_hold_still_leads() {
        let lexical = vec![candidate(30, false), candidate(40, false)];
        let semantic = vec![candidate(10, true), candidate(40, true)];
        let page = fuse(lexical, semantic, 10);
        assert_eq!(ids(&page), vec![40, 30, 10]);
        assert!(
            !page[0].semantic,
            "found by its words, so not marked semantic"
        );
    }
}
