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
    /// Embeds first whatever is in scope and has no current vector, which is
    /// where the one-time cost of a store that has never been asked this lives:
    /// 0.24 s to embed 5,336 memories and 0.1 to 0.2 s to keep them, then the
    /// scan itself, 6.6 ms. A store that never reaches this stage never pays.
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
        let unkept = self.refresh_vectors(&model, options)?;

        let visible = visible_observations(1, 2, 3);
        let mut statement = self.connection.prepare(&format!(
            "SELECT o.id, o.type, v.vector FROM observations o
             CROSS JOIN observation_vectors v ON v.observation_id = o.id
             WHERE {visible} AND o.type != ?4
               AND v.model = ?5 AND v.source_key = {SOURCE_KEY}"
        ))?;
        let mut scored: Vec<(f32, i64, String)> = Vec::new();
        let mut rows = statement.query(params![
            options.kind,
            options.project,
            options.scope,
            crate::memory::model::SESSION_SUMMARY,
            semantic::MODEL_ID,
        ])?;
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            if unkept.contains_key(&id) {
                continue;
            }
            let stored = row.get_ref(2)?.as_blob()?;
            if let Some(cosine) = semantic::cosine(stored, &question) {
                scored.push((cosine, id, row.get(1)?));
            }
        }
        // Vectors made for this question and not kept: the write lost the lock
        // or the file is read-only. They answer it all the same.
        for (id, (kind, vector)) in &unkept {
            if let Some(cosine) = semantic::cosine(vector, &question) {
                scored.push((cosine, *id, kind.clone()));
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

    /// Makes a current vector for every memory in scope that has none, a chunk
    /// at a time.
    ///
    /// Returns the ones it made and could not keep, by id with their type and
    /// bytes, so the question that caused the work is still answered from them.
    fn refresh_vectors(
        &self,
        model: &StaticModel,
        options: &SearchOptions,
    ) -> Result<BTreeMap<i64, (String, Vec<u8>)>, StageError> {
        let stale = self.stale_vectors(options)?;
        let mut unkept = BTreeMap::new();
        if stale.is_empty() {
            return Ok(unkept);
        }
        let mut read = self.connection.prepare(&format!(
            "SELECT o.title, o.type, substr(o.content, 1, {BODY_CHARS})
             FROM observations o WHERE o.id = ?1"
        ))?;
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
            if let Err(error) = self.keep_vectors(&fresh) {
                tracing::warn!(%error, "the semantic stage could not keep its vectors; using them for this question only");
                for (row, (_, _, kind)) in fresh.into_iter().zip(rows) {
                    unkept.insert(row.id, (kind, row.vector));
                }
            }
        }
        Ok(unkept)
    }

    /// The ids, and keys, of the memories in scope whose vector is missing, from
    /// another model, or made from text that has since changed.
    ///
    /// Session summaries are not embedded: the stage never returns one, for the
    /// reason every relaxed stage leaves them out, so a vector for one would be
    /// space and time spent on a row nothing reads.
    fn stale_vectors(&self, options: &SearchOptions) -> Result<Vec<(i64, String)>, StageError> {
        let visible = visible_observations(1, 2, 3);
        let mut statement = self.connection.prepare(&format!(
            "SELECT o.id, {SOURCE_KEY}
             FROM observations o
             LEFT JOIN observation_vectors v ON v.observation_id = o.id
             WHERE {visible} AND o.type != ?4
               AND (v.observation_id IS NULL OR v.model != ?5 OR v.source_key != {SOURCE_KEY})"
        ))?;
        let rows = statement.query_map(
            params![
                options.kind,
                options.project,
                options.scope,
                crate::memory::model::SESSION_SUMMARY,
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
