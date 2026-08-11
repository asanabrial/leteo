//! Finding a memory again: full-text ranking and prompt recall.

use super::*;

/// The stemmed index: what makes a differently inflected question find
/// anything at all.
pub const FTS_STEMMED: &str = "observations_fts";
/// The same memories indexed as they were written, with no stemmer.
pub const FTS_EXACT: &str = "observations_exact";

/// How much a place in one ranking is worth when the two are merged.
///
/// Reciprocal rank fusion: a memory is worth `1 / (60 + place)` in each list it
/// appears in, and the sum orders the answer. Sixty is the constant the method
/// is published with, and what it buys is that the top of a list is worth a
/// little more than the rest of it rather than overwhelmingly more — so a
/// memory both indexes like beats one that only one of them loves.
///
/// The scores themselves cannot be added: bm25 is scaled by the index it came
/// from, and these are two indexes with different vocabularies. Places compare;
/// scores do not.
const FUSION_CONSTANT: f64 = 60.0;

/// How many a search returns when the caller does not say.
///
/// Named because two surfaces and `search_with_more` all have to agree about
/// it: a page size written out three times is one edit away from a reply that
/// says "there is more" about a list that ended on its own.
pub(crate) const DEFAULT_SEARCH_LIMIT: usize = 10;

/// The statement [`Store::matching_observations`] runs, built in one place.
///
/// Named rather than inlined so a test can assert on the plan of *this* query.
/// A test that writes its own copy of the SQL proves SQLite plans that copy
/// well and nothing about what the product runs — which is exactly what
/// happened: the first version of the join-order guard kept its own string,
/// and downgrading `CROSS JOIN` here left it green.
///
/// Takes the index because there are two of them, holding the same memories
/// tokenised two ways. See [`Store::fused_observations`].
/// The weights come in rather than being read from the constant, because the
/// retrieval measurement under `tools/` asks what a *different* vector would
/// rank — and a tool that writes its own copy of this query measures a search
/// nobody runs. That is not hypothetical: a hand-written copy with `ifnull(project,
/// '')` where `Narrowing` writes `project =` was measured for an afternoon
/// before anybody noticed the product never issues it.
pub fn matching_observations_sql(index: &str, weights: &str) -> String {
    format!(
        "SELECT o.id, o.type, bm25({index}, {weights}) AS rank
         FROM {index} fts CROSS JOIN observations o ON o.id = fts.rowid
         WHERE {index} MATCH ?1 AND o.deleted_at IS NULL
           AND (?2 IS NULL OR o.type = ?2)
           AND (?3 IS NULL OR LOWER(o.project) = ?3)
           AND (?4 IS NULL OR o.scope = ?4)
         ORDER BY rank LIMIT ?5"
    )
}

/// A memory a stage is still deciding about: what it takes to rank it, drop it
/// and merge it, and nothing else.
///
/// The stages read three times as many memories as they return and then throw
/// most of them away — deeper than the answer so the fusion has places to
/// compare, a sample wide enough to have a median, one query per omitted term.
/// Selecting the whole row to do that read every body twice over: 200 real
/// prompts of one project moved 9.8 MB of memory bodies through
/// `map_observation` to show 392 memories, 91% of it discarded unread.
///
/// So the stages rank ids and the survivors are fetched once, at the end. It
/// is the argument `prompt_matches` and the opening block each already make
/// beside their own queries; this was the third of the three and the only one
/// still reading whole rows to sort them.
///
/// The type comes along because two stages drop session summaries by it, and
/// it is a word rather than a body.
#[derive(Debug, Clone)]
pub(super) struct Candidate {
    id: i64,
    kind: String,
    rank: f64,
    partial: bool,
}

impl Store {
    /// The same search, and whether the store had more than it was asked for.
    ///
    /// A full page and an exhausted one are the same shape. The reply already
    /// says when the *store's* maximum ended a list; nothing said when the
    /// caller's own limit did, and the default limit is ten. Over sixty real
    /// questions — the first four words of a memory's own title, asked through
    /// this binary — eighteen came back with exactly ten, and seventeen of
    /// those eighteen had more the caller was never told about.
    ///
    /// One row more than was asked for, thrown away. That is the whole cost,
    /// and it is the only way to tell the two apart: counting the matches
    /// would mean running the stages again for a number nobody reads.
    ///
    /// At the store's maximum this cannot answer — asking for one past the cap
    /// is clamped back to it — and that end is what the clamped hint is for.
    pub fn search_with_more(
        &self,
        query: &str,
        options: SearchOptions,
    ) -> Result<(Vec<SearchResult>, bool), StoreError> {
        let asked = options
            .limit
            .unwrap_or(DEFAULT_SEARCH_LIMIT)
            .clamp(1, self.config.max_search_results);
        // The probe row has to be allowed past the published cap, or it stops
        // existing exactly where it is needed most.
        //
        // This used to go through `search`, which clamps to
        // `max_search_results` itself: asking it for twenty-one rows got twenty,
        // so `more` could never be true at the cap. A search for twenty on a
        // store holding hundreds of matches came back with a full page and both
        // surfaces said nothing at all — the caller's own limit had not ended
        // the list, and neither had anything that announced itself. That is the
        // same full-page-or-exhausted silence the hint was written for, hiding
        // at the one limit where it cannot be widened away.
        let mut found = self.search_limited(query, options, asked.saturating_add(1))?;
        let more = found.len() > asked;
        found.truncate(asked);
        Ok((found, more))
    }

    pub fn search(
        &self,
        query: &str,
        options: SearchOptions,
    ) -> Result<Vec<SearchResult>, StoreError> {
        let limit = options
            .limit
            .unwrap_or(DEFAULT_SEARCH_LIMIT)
            .clamp(1, self.config.max_search_results);
        self.search_limited(query, options, limit)
    }

    /// The search body, with the row budget decided by the caller.
    ///
    /// Private, and it stays private: `max_search_results` is a published limit
    /// and the only caller allowed past it is [`Self::search_with_more`], which
    /// asks for one row it never returns.
    fn search_limited(
        &self,
        query: &str,
        mut options: SearchOptions,
        limit: usize,
    ) -> Result<Vec<SearchResult>, StoreError> {
        if query.trim().is_empty() {
            return Err(StoreError::EmptySearch);
        }
        options.project = options.project.as_deref().map(normalize::project);
        // A blank filter is no filter, the way a blank project already is.
        // `normalize::scope` folds anything it does not know onto `project`,
        // which is right for a value being *stored* and wrong for one being
        // *asked about*: `scope: ""` narrowed the answer to project scope
        // without saying so, and `type: ""` narrowed it to a type nothing has
        // and blamed the words for the empty result.
        options.scope = normalize::optional(options.scope.as_deref())
            .as_deref()
            .map(normalize::scope)
            .map(str::to_owned);
        // The same word has to mean the same thing going in and coming out.
        //
        // `normalize::kind` folds the synonyms a caller is likely to reach for
        // — `bug`, `design`, `learning`, `setup` — onto the eight the store
        // actually holds, and it did so on the way in only. Saving with
        // `type: "bug"` stores `bugfix` and says so in the reply; searching
        // with `type: "bug"` then compared `bug` against a column that has
        // never held it, came back with nothing, and blamed the words. The
        // fold table exists precisely so that a caller need not know the eight,
        // and applying it at one end of that promise is worse than not having
        // it: the caller is told the memory is not there.
        //
        // Two of the three narrowings were already folded here, on the two
        // lines above. This is the third.
        options.kind = normalize::optional(options.kind.as_deref())
            .as_deref()
            .map(normalize::kind);

        let mut results = Vec::new();
        // A topic key is looked up the way it was stored, not the way it was
        // typed.
        //
        // Every key in the store went through `normalize::topic_key` on its way
        // in — lowercased, whitespace folded to hyphens — but the lookup used
        // to compare the raw query against it. So the exact branch fired only
        // for somebody who had already spelled the key in its normalised form:
        // `architecture/wizard-split` hit it, and `Architecture/Wizard-Split`,
        // which is how a person or an agent writes the same key, fell through
        // to ranked full-text against every other memory in the family — 125 of
        // them under `architecture/` on a real store. It looks like it works,
        // because the title usually matches too and the memory still comes back
        // somewhere in the list. The point of a topic key is that it comes back
        // *first*, and that part was silently gone.
        let topic_key = crate::memory::normalize::topic_key(Some(query));
        if let Some(topic_key) = topic_key.filter(|key| key.contains('/')) {
            let mut statement = self.connection.prepare(&format!(
                "SELECT {OBSERVATION_COLUMNS} FROM observations
                 WHERE topic_key = ?1 AND deleted_at IS NULL
                   AND (?2 IS NULL OR type = ?2)
                   AND (?3 IS NULL OR project = ?3)
                   AND (?4 IS NULL OR scope = ?4)
                 ORDER BY updated_at DESC LIMIT ?5"
            ))?;
            let rows = statement.query_map(
                params![
                    topic_key,
                    options.kind,
                    options.project,
                    options.scope,
                    limit as i64
                ],
                map_observation,
            )?;
            for row in rows {
                results.push(SearchResult {
                    observation: row?,
                    rank: -1000.0,
                    partial: false,
                });
            }
        }

        let any = options.mode == SearchMode::Any;
        let mut matched =
            self.fused_observations(&normalize::fts_query(query, any), &options, limit)?;
        // Every word, and then any of them rather than nothing at all.
        //
        // Requiring all of them is the right first answer — it is what makes
        // the top hit the one somebody meant — but it fails completely rather
        // than partially: one word the store has never seen takes the whole
        // question down with it, and the result is the same empty list as a
        // subject nobody ever wrote about. Measured over two hundred questions
        // drawn from the titles of a real 2,643-memory store, that happened to
        // 4% of short questions and 12% of long ones, and the widened retry
        // found the memory every single time, at rank one every single time.
        // MRR went from 0.856 to 0.981 on the long ones.
        //
        // Only when the strict pass came back with nothing, so a question that
        // matched is never reordered or diluted by one that half-matched. And
        // the results are marked, because "these matched some of your words"
        // is a different claim from "these matched your question" and the
        // agent reading them is entitled to know which it has.
        if matched.is_empty() && results.is_empty() && !any {
            matched = self.widened_observations(query, &options, limit)?;
        }
        // And when that finds nothing either, the closest by relevance.
        //
        // Both stages above are built for a quotation with a word wrong in it,
        // and a question is not that. Asked the 277 real prompts from a live
        // store — the shape an agent actually types into this tool — the two of
        // them together came back **empty for 80.5% of them**, while the
        // per-prompt hint, given the same words and the same store as it stood
        // that day, named something from the asking session 34% of the time.
        // Leteo knew the answer and the tool an agent calls on purpose said
        // nothing.
        //
        // So the last stage is the hint's own rule, which is the one measured
        // on questions: any of the words, and a floor relative to the median of
        // what came back. Over those prompts it turns 80.5% empty into 7.2%,
        // and 6.5% right into 28.2% right.
        //
        // What it costs is stated rather than hidden. Asked a question its
        // project cannot answer — another project's prompt, which is the
        // control — it still speaks 67.9% of the time. A relevance floor is
        // scale-free: it knows what an ordinary match looks like for this
        // query, not whether this store holds the answer at all. That is why
        // these arrive marked `partial`, the same as the stage above, and why
        // no wording here claims a match.
        if matched.is_empty() && results.is_empty() && !any {
            matched = self.nearest_observations(query, &options, limit)?;
        }
        // The candidates that survive are the only ones whose body is read.
        matched.retain(|row| !results.iter().any(|item| item.observation.id == row.id));
        matched.truncate(limit.saturating_sub(results.len()));
        results.extend(self.hydrate(matched)?);
        results.truncate(limit);
        Ok(results)
    }

    /// Both indexes, merged by where each put a memory rather than by score.
    ///
    /// Stemming is what lets a question asked in different words find anything:
    /// on a real store, a question with two of six words re-inflected is
    /// answered 63% of the time by the stemmed index and **0%** by an unstemmed
    /// one, because requiring every word of a conjunction means one changed
    /// ending returns nothing at all. What stemming costs is that more memories
    /// match the same words, so the one somebody quoted is diluted: six words
    /// lifted straight out of a memory find it first 78% of the time here
    /// against 84% unstemmed.
    ///
    /// Both are real and they pull opposite ways, and a tokenizer belongs to
    /// its table, so no single index has both. Reading both and merging:
    ///
    /// ```text
    ///                 quoted words   re-inflected   from a title
    ///   stemmed only      78.0%         37.3%          74.0%
    ///   unstemmed only    84.0%          0.0%          76.7%
    ///   merged            84.3%         37.0%          75.7%
    /// ```
    ///
    /// Measured over 300 memories of a real store, questions built from each.
    /// The merge is better at quoting than either index alone and gives up
    /// nothing that mattered. It costs 0.03 ms a search.
    ///
    /// A memory keeps the score of the stemmed index when it appeared there,
    /// because that is the one every other number in this file is on the scale
    /// of. What orders the answer is the merge, not that score.
    ///
    /// An unreadable second index is not an error. The table arrives in a
    /// migration, and a store that could not run it — read-only media, a
    /// half-finished upgrade — searches the way it did before there were two.
    fn fused_observations(
        &self,
        fts: &str,
        options: &SearchOptions,
        limit: usize,
    ) -> Result<Vec<Candidate>, StoreError> {
        // Deeper than the answer, so the merge has places to compare. A memory
        // ninth in one list and second in the other is exactly the case this
        // exists for, and it cannot be seen from two lists of three.
        let depth = (limit * 3).max(30);
        let stemmed = self.matching_observations(FTS_STEMMED, fts, options, depth, false)?;
        let exact = match self.matching_observations(FTS_EXACT, fts, options, depth, false) {
            Ok(exact) => exact,
            Err(error) => {
                tracing::debug!(%error, "the unstemmed index is unreadable; searching the stemmed one alone");
                Vec::new()
            }
        };
        if exact.is_empty() {
            let mut stemmed = stemmed;
            stemmed.truncate(limit);
            return Ok(stemmed);
        }

        let mut fused: BTreeMap<i64, f64> = BTreeMap::new();
        let mut rows: BTreeMap<i64, Candidate> = BTreeMap::new();
        for list in [stemmed, exact] {
            for (place, result) in list.into_iter().enumerate() {
                *fused.entry(result.id).or_default() +=
                    1.0 / (FUSION_CONSTANT + place as f64 + 1.0);
                // The stemmed list is walked first, so its score is the one
                // kept for a memory both indexes found.
                rows.entry(result.id).or_insert(result);
            }
        }
        let mut merged: Vec<Candidate> = rows.into_values().collect();
        merged.sort_by(|left, right| {
            let left_score = fused.get(&left.id).copied().unwrap_or_default();
            let right_score = fused.get(&right.id).copied().unwrap_or_default();
            right_score
                .total_cmp(&left_score)
                .then_with(|| left.rank.total_cmp(&right.rank))
                .then_with(|| left.id.cmp(&right.id))
        });
        merged.truncate(limit);
        Ok(merged)
    }

    /// The last resort: any of the words, and only what stands out among them.
    ///
    /// Session summaries are left out, for the reason the per-prompt hint
    /// leaves them out: they are long, they match a scattering of any
    /// question's words, and on a real store they were 18.6% of what a
    /// question-shaped query returned. Excluding them is worth five points of
    /// accuracy here (13.7% against 8.7% on one cut of the measurement). A
    /// search that means to find one asks for it by its own words, and the
    /// stages above answer that.
    ///
    /// The floor is `RECALL_MARGIN_UNSEEN`, the same number the hint uses for a
    /// memory the session has not already been shown, and for the same reason:
    /// there is no opening block here to have shown anything.
    fn nearest_observations(
        &self,
        query: &str,
        options: &SearchOptions,
        limit: usize,
    ) -> Result<Vec<Candidate>, StoreError> {
        let terms = normalize::fts_any_of(&normalize::fts_terms(query));
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let mut candidates =
            self.matching_observations(FTS_STEMMED, &terms, options, RECALL_SAMPLE, true)?;
        candidates.retain(|candidate| candidate.kind != crate::memory::model::SESSION_SUMMARY);
        // The median of two is not a distribution, and nothing here is worth
        // saying without one.
        if candidates.len() < MIN_RECALL_SAMPLE {
            return Ok(Vec::new());
        }
        let mut ranks: Vec<f64> = candidates.iter().map(|row| row.rank).collect();
        ranks.sort_by(|left, right| left.total_cmp(right));
        let median = ranks[ranks.len() / 2];
        candidates.retain(|candidate| candidate.rank <= median * RECALL_MARGIN_UNSEEN);
        candidates.truncate(limit);
        Ok(candidates)
    }

    /// The widened retry: every word but one, not any word at all.
    ///
    /// The question this rescues is the one a single unknown term took down —
    /// `CROSS JOIN search performance kubernetes` against a store that has
    /// never heard of Kubernetes. Relaxing to *any* word rescues it, and also
    /// answers questions nobody could answer: a question in a language the
    /// store does not hold matches on function words — `la`, `de`, `no` —
    /// against whichever memories happen to share them. Measured on 22 Spanish
    /// questions against a real English store, the any-word form returned ten
    /// rows for **all 22**, wrong every time. Asking why passive capture saved
    /// nothing came back with a session summary for another project.
    ///
    /// Dropping one term at a time says the same thing far more precisely.
    /// Over 150 body-derived English questions each carrying one unknown word,
    /// against those same 22 Spanish ones:
    ///
    /// | | rescues | MRR | Spanish rows returned | per query |
    /// |---|---|---|---|---|
    /// | any word | 141/150 | 0.7551 | 22/22 | 4.0 ms |
    /// | **all but one** | **140/150** | **0.7976** | **3/22** | **0.4 ms** |
    ///
    /// Better on every axis, including ten times faster: each variant is a
    /// conjunction that matches almost nothing, where the disjunction it
    /// replaces scans everything sharing one common word.
    ///
    /// What it costs, and two ways of making it cheaper that were measured and
    /// are not taken.
    ///
    /// One query per term is the expensive part of a search: a ten-word
    /// question runs ten of them, and through the protocol that is 17ms of a
    /// 20ms `mem_search` against a real store — where a short quotation, which
    /// the strict pass answers, costs 2ms. The queries themselves are not the
    /// cost: the same ten, run against the same file from another SQLite, take
    /// 3.4ms with every column selected. Where the rest goes is not yet known,
    /// and saying so is better than guessing at it.
    ///
    /// **Dropping only the unknown words.** A variant that omits `de` cannot
    /// rescue anything, the reasoning goes, because a word every memory holds
    /// was never why the conjunction failed. It is wrong: a conjunction also
    /// fails when its words are all known and never co-occur, and dropping any
    /// one of them can make the rest meet. Over the 348 real prompts that
    /// reach this stage it rescues 16.7% against 39.9%, losing 81 of them, and
    /// it is not even faster — asking the index whether it holds each word
    /// costs a query per term too.
    ///
    /// **Caching the prepared statement.** No difference at all, which also
    /// says the per-call cost is not preparation.
    ///
    /// Where it does go, after taking the whole thing apart: SQLite's own
    /// execution. Instrumented inside the query, building the SQL costs 0.04ms
    /// and preparing it 0.1ms, while reading the rows costs 0.9 to 2.3ms —
    /// per variant. The same statement, on the same file, with the same plan
    /// and the same pragmas, runs in 0.34ms from another SQLite. The one this
    /// binary carries is *newer* — 3.51.3 bundled against 3.50.4 on the
    /// system — and `bundled` against `bundled-full` changes nothing, so it is
    /// neither a missing feature nor an old engine. That is as far as it has
    /// been taken; the next step is a build of each version to compare, which
    /// is a dependency question rather than a Leteo one.
    ///
    /// Which measurements that invalidates, and which it does not, because the
    /// distinction is the useful part. **Timing does not transfer**: the two
    /// engines differ by four times on the same statement, so a stopwatch held
    /// over one says nothing about the other. **Ranking does.** Sixty real
    /// queries run both ways came back with the same first result sixty times
    /// and the same five in the same order fifty-six — and the four that
    /// differ do so below the first place, because the binary fuses two
    /// indexes where the check replicated only the stemmed one. So the weights,
    /// the floors, the sample depth and the third stage, all chosen against a
    /// replication of the SQL, stand. What had to be re-measured in the binary
    /// was the one thing that was about speed.
    ///
    /// One thing that measurement did settle. Narrowing to the project inside
    /// the `MATCH` was chosen on a measurement taken in the *other* SQLite,
    /// which is exactly the mistake this note is about — so it was checked
    /// again in the binary, on the path that runs before every prompt: 19.13ms
    /// without it against 11.06ms with it. It holds.
    ///
    /// A relevance floor was tried first and is the wrong instrument. It
    /// removed the Spanish noise and took 82 of 141 genuine rescues with it,
    /// because the property that separates the two cases is not the shape of
    /// the score distribution — it is how many of the asked-for words were
    /// actually found.
    fn widened_observations(
        &self,
        query: &str,
        options: &SearchOptions,
        limit: usize,
    ) -> Result<Vec<Candidate>, StoreError> {
        let terms = normalize::fts_terms(query);
        // One term dropped from one term is no query at all, and past a dozen
        // the omission is too small to relax anything while costing a query
        // each. Both ends fall back to what the caller already had: nothing.
        if terms.len() < 2 || terms.len() > MAX_WIDENED_TERMS {
            return Ok(Vec::new());
        }
        // A memory can surface in several variants; it keeps its best rank,
        // which is the one from the variant that dropped the word it was
        // missing.
        let mut best: BTreeMap<i64, Candidate> = BTreeMap::new();
        for omitted in 0..terms.len() {
            let kept = terms
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != omitted)
                .map(|(_, term)| term.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            for result in self.matching_observations(FTS_STEMMED, &kept, options, limit, true)? {
                match best.entry(result.id) {
                    std::collections::btree_map::Entry::Occupied(mut seen) => {
                        if result.rank < seen.get().rank {
                            seen.insert(result);
                        }
                    }
                    std::collections::btree_map::Entry::Vacant(empty) => {
                        empty.insert(result);
                    }
                }
            }
        }
        let mut matched: Vec<Candidate> = best.into_values().collect();
        // And no session summaries, which is the rule the other two relaxed
        // paths already keep.
        //
        // `nearest_observations` and `prompt_matches` both leave them out, for
        // the reason written beside the second: they are the most common
        // memories on a busy project, they all read alike, and they were most
        // of what the relevance test was there to reject. This stage was the
        // one that missed it.
        //
        // What it cost, over 80 real questions asked in their own projects:
        // six were answered by the strict pass and *none* of those led with a
        // summary, while 74 fell through to a relaxed stage and 54 of those —
        // 73% — came back headed by one. A summary never wins on the words
        // somebody actually typed; it wins once the question has been loosened,
        // because a session's worth of prose matches whatever is left of it.
        //
        // The strict pass keeps them. A question whose words genuinely name
        // what a session did should still find that session.
        matched.retain(|result| result.kind != crate::memory::model::SESSION_SUMMARY);
        matched.sort_by(|left, right| left.rank.total_cmp(&right.rank));
        matched.truncate(limit);
        Ok(matched)
    }

    /// The memories a prepared full-text query matches, best first.
    ///
    /// `CROSS JOIN` is not decoration: it is the whole performance of search.
    ///
    /// With a plain `JOIN`, SQLite 3.51.3 picks `observations` as the outer
    /// loop — driven by the index on `deleted_at`, which it reads as selective
    /// when it matches every live row — and re-runs the full-text query once
    /// per row. On a store of 3,400 memories a ten-word question took 4,075 ms;
    /// the same statement with `CROSS JOIN` takes 14.9 ms and returns the same
    /// ten ids in the same order. SQLite 3.50.4 planned the plain join
    /// correctly, so this only appears once the bundled SQLite is new enough,
    /// and it appears as "search got slow", not as a wrong answer.
    ///
    /// `CROSS JOIN` fixes the join order at written order, so the full-text
    /// side always drives and the base table is reached by rowid. Every FTS
    /// query in this file is written that way for the same reason.
    fn matching_observations(
        &self,
        index: &str,
        fts: &str,
        options: &SearchOptions,
        limit: usize,
        partial: bool,
    ) -> Result<Vec<Candidate>, StoreError> {
        // Narrowed to the project inside the index when one was named, and not
        // only in the `WHERE` afterwards: see `normalize::fts_within_project`.
        // The SQL condition below stays and is what actually decides.
        let fts = options
            .project
            .as_deref()
            .and_then(|project| normalize::fts_within_project(fts, project))
            .unwrap_or_else(|| fts.to_owned());
        let mut statement = self
            .connection
            .prepare(&matching_observations_sql(index, BM25_WEIGHTS))?;
        let rows = statement.query_map(
            params![
                fts,
                options.kind,
                options.project,
                options.scope,
                limit as i64
            ],
            |row| {
                Ok(Candidate {
                    id: row.get("id")?,
                    kind: row.get("type")?,
                    rank: row.get("rank")?,
                    partial,
                })
            },
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// The rows behind the ids a stage settled on, in the order it settled on.
    ///
    /// One query for the whole answer rather than one per memory, and the order
    /// is restored here because `IN` does not promise one.
    fn hydrate(&self, candidates: Vec<Candidate>) -> Result<Vec<SearchResult>, StoreError> {
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        let holes = std::iter::repeat_n("?", candidates.len())
            .collect::<Vec<_>>()
            .join(", ");
        let mut statement = self.connection.prepare(&format!(
            "SELECT {OBSERVATION_COLUMNS} FROM observations
             WHERE id IN ({holes}) AND deleted_at IS NULL"
        ))?;
        let rows = statement.query_map(
            rusqlite::params_from_iter(candidates.iter().map(|candidate| candidate.id)),
            map_observation,
        )?;
        let mut by_id: BTreeMap<i64, crate::memory::model::Observation> = BTreeMap::new();
        for row in rows {
            let row = row?;
            by_id.insert(row.id, row);
        }
        // The narrowing the ranking applied, applied again.
        //
        // The stages ask for live memories, so nothing deleted can reach this
        // list — until something is deleted *between* the two queries, and this
        // is where splitting the fetch off from the ranking put a window that
        // did not exist before. There is no transaction around the pair and
        // Leteo is multi-writer by design: a hook, a second agent or a terminal
        // can soft-delete in it, and a soft delete leaves the row exactly where
        // `IN` will find it.
        //
        // The note that stood here said such a memory was "simply not here",
        // which is true of a hard deletion and of nothing else. Deleted
        // memories are never returned — see `memory-model.md` §8 — so the
        // filter travels with the fetch rather than being assumed from the
        // company it keeps.
        Ok(candidates
            .into_iter()
            .filter_map(|candidate| {
                by_id.remove(&candidate.id).map(|observation| SearchResult {
                    observation,
                    rank: candidate.rank,
                    partial: candidate.partial,
                })
            })
            .collect())
    }

    /// Memories a user's prompt is likely about, or nothing.
    ///
    /// For the prompt hook, which sees every message somebody types. Three
    /// things make it worth running there rather than noise:
    ///
    /// Any term rather than all of them. A prompt is a sentence, and requiring
    /// every word of it found something for thirteen prompts in a hundred.
    /// Requiring any word found something for eighty-two — and four out of five
    /// of those were memories the session context had not already handed over.
    ///
    /// A relevance test that is *relative*, not a fixed score. That eighty-two
    /// per cent is rows returned, not rows worth reading: with a dozen terms
    /// joined by OR almost any prompt matches something. Judged by hand over
    /// twenty real prompts about a third were genuinely on topic, and bm25
    /// separated them — but only against its own results. bm25 scales with the
    /// index: the same match scores -0.0 in a store of one memory, -24 at
    /// fifty, and -53 at three thousand. A threshold tuned here would have been
    /// silent forever on anybody's new store. So the best hit has to beat the
    /// median of what the same query matched, which is scale-free by
    /// construction and fires on a third of prompts either way.
    ///
    /// And no session summaries, and nothing untitled. They are the most common
    /// memories on a busy project, they all read alike, and they were most of
    /// what the test was there to reject.
    pub fn prompt_matches(
        &self,
        prompt: &str,
        project: &str,
        limit: usize,
    ) -> Result<Vec<MemoryRef>, StoreError> {
        let project = normalize::project(project);
        // Every word of the prompt it is worth reading, not the rarest few.
        //
        // How many that is belongs to `MAX_ANY_TERMS`, and the two notes
        // have to be read together: the choice below is *not* to rank words by
        // rarity, and the bound there is what keeps a pasted file from
        // becoming a thousand-term query.
        //
        // Ranking the words by how common they are in the project and keeping
        // the six rarest is the obvious improvement, and it does raise how
        // often the right memory reaches the top three — 28% to 34%. It does
        // not survive the relevance test: fewer, rarer terms compress the score
        // distribution, the median moves with the best hit, and the margin
        // stops separating anything. At matched precision it delivered less
        // (20% against 23%), so the obvious improvement was measured and
        // dropped rather than kept for being obvious.
        //
        // It asks the stemmed index alone, and that is measured rather than
        // inherited. `search` fuses both indexes by rank because a quoted
        // phrase is a different question from a prompt: there the unstemmed
        // index is worth eight points. Here, over the same 277 labelled
        // prompts, fusing raised accuracy from 22.4% to 23.1% — two prompts —
        // and took the query from 5.3ms to 9.4ms, on the one path that runs
        // before every single thing the user types.
        let terms = normalize::fts_any_of(&normalize::prompt_terms(prompt));
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        // Narrowed to the project inside the index rather than only after the
        // join: see `normalize::fts_within_project`. Every prompt used to be
        // scored against every memory of every project.
        let terms = normalize::fts_within_project(&terms, &project).unwrap_or(terms);
        // Enough rows to know what an ordinary match looks like for this query,
        // and deliberately not a function of how many the caller wants named:
        // see `RECALL_SAMPLE`, where the measurement is. The median of three is
        // not a distribution, and the median of a hundred is a different query.
        let sample = RECALL_SAMPLE as i64;
        let mut statement = self.connection.prepare(&prompt_recall_sql())?;
        let scored = statement
            .query_map(params![terms, project, sample], |row| {
                Ok((
                    MemoryRef {
                        id: row.get("id")?,
                        sync_id: row.get("sync_id")?,
                        kind: row.get("type")?,
                        title: row.get("title")?,
                    },
                    row.get::<_, f64>("rank")?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        // Too few to judge against: a query that matched one or two things has
        // no ordinary to stand out from, so nothing is claimed.
        if scored.len() < MIN_RECALL_SAMPLE {
            return Ok(Vec::new());
        }
        let mut ranks: Vec<f64> = scored.iter().map(|(_, rank)| *rank).collect();
        ranks.sort_by(|a, b| a.total_cmp(b));
        let median = ranks[ranks.len() / 2];
        // Two floors, because the two kinds of hit are not worth the same.
        //
        // A session opens with an index of the project's most recent memories.
        // Naming one of those again is a ranking service — useful, but the
        // agent already has the line — while naming one from further back is
        // the only way it hears of that memory at all. The cost of a wrong line
        // is the same either way; the value of a right one is not.
        //
        // Measured over 277 real prompts against a label with no leak — the
        // memories saved earlier in the same session, which existed when the
        // prompt was typed and are marked by the clock rather than by bm25:
        //
        // ```text
        //                        speaks   right   right when it speaks
        //   1.6 for everything    58.5%   11.9%          20.4%
        //   1.2 for everything    84.5%   19.5%          23.1%
        //   1.6 / 1.2             80.9%   22.4%          27.7%
        //   1.0 for everything    90.3%   23.5%          26.0%
        //   1.6 / 1.0             89.9%   30.7%          34.1%
        // ```
        //
        // The rows that matter are the pairs at the same reach: split beats
        // flat on both axes, so this is not "relax the floor" wearing a hat.
        // `1.6 / 1.0` is better again and is not taken — no floor at all on
        // that side means the three best candidates are named on nine prompts
        // in ten, and a hint that always speaks is one a reader stops seeing.
        //
        // Where that has since got to, re-measured on the same store 4,013
        // memories later: `1.6 / 1.2` now speaks on 92% of 421 distinct prompts
        // of one project, driven through the built binary's own hook. The
        // operating point chosen at 80.9% has drifted past the 89.9% row that
        // was refused, on the very grounds it was refused for. The bar is
        // relative to the median of the sample, so it says how good a candidate
        // is *against the others this query found* — it was never a promise
        // about how often anything is said, and it does not hold one.
        //
        // Not re-tuned here, and the reason is worth more than the number: the
        // right-hand columns can no longer be measured. The label is "a memory
        // saved earlier in the same session", and on this store 1,408 memories
        // of 4,013 now sit in `manual-save-<project>` buckets while prompts are
        // written under the agent's session, so only a quarter of prompts have
        // any same-session memory to find and the column reads 2% — for hints
        // that are plainly right when read. Moving these two numbers against
        // that label would be tuning against the store's filing, not against
        // relevance. Whoever re-tunes them needs a label first.
        let recent = self.recent_ids(project.as_str(), RECALL_RECENT_BLOCK)?;
        Ok(scored
            .into_iter()
            .filter(|(memory, rank)| worth_naming(*rank, median, recent.contains(&memory.id)))
            .take(limit)
            .map(|(memory, _)| memory)
            .collect())
    }
}

/// The statement the prompt hint ranks with.
///
/// Four columns, not the whole row: these are ranked and mostly thrown away,
/// and their bodies are never read.
///
/// A function rather than a literal inside `prompt_matches` for the reason
/// `matching_observations_sql` is one — the retrieval measurement under
/// `tools/` asks what this stage would do under a different floor, and a
/// harness holding its own copy of the SQL measures a query the product does
/// not issue. That has already cost an afternoon once.
pub(crate) fn prompt_recall_sql() -> String {
    format!(
        "SELECT o.id, ifnull(o.sync_id, '') AS sync_id, o.type, o.title,
                bm25(observations_fts, {BM25_WEIGHTS}) AS rank
         FROM observations_fts fts CROSS JOIN observations o ON o.id = fts.rowid
         WHERE observations_fts MATCH ?1 AND o.deleted_at IS NULL
           AND LOWER(o.project) = ?2
           AND o.type <> 'session_summary'
           AND trim(ifnull(o.title, '')) <> ''
         ORDER BY rank LIMIT ?3"
    )
}

/// Whether a candidate beats the bar for being named.
///
/// Named rather than inlined so the rule can be tested on numbers instead
/// of on a corpus: bm25 needs a varied one for a median to mean anything —
/// fifty near-identical fixtures score every term at nothing, which is how
/// the first attempt at this test passed with both margins equal.
///
/// The bar is relative and the scores are negative, so "better" is more
/// negative and a *smaller* margin is a looser bar.
pub(crate) fn worth_naming(rank: f64, median: f64, already_in_the_opening_block: bool) -> bool {
    let margin = if already_in_the_opening_block {
        RECALL_MARGIN
    } else {
        RECALL_MARGIN_UNSEEN
    };
    rank <= median * margin
}

impl Store {
    /// The ids a session opening would have named, for the project.
    ///
    /// Read rather than assumed: the opening block is the most recent of the
    /// project, and which memories those are changes with every save.
    fn recent_ids(&self, project: &str, limit: usize) -> Result<BTreeSet<i64>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT id FROM observations
              WHERE deleted_at IS NULL AND project = ?1
              ORDER BY datetime(created_at) DESC, id DESC LIMIT ?2",
        )?;
        let rows =
            statement.query_map(params![project, limit as i64], |row| row.get::<_, i64>(0))?;
        rows.collect::<Result<_, _>>().map_err(StoreError::from)
    }
}

#[cfg(test)]
mod hydrate_tests {
    use super::*;

    /// The fetch carries the narrowing the ranking did.
    ///
    /// Splitting the row fetch off from the ranking put a window where none had
    /// been: the stages ask for live memories, and between their query and this
    /// one another writer can soft-delete — which leaves the row exactly where
    /// `IN` finds it. Leteo is multi-writer by design and there is no
    /// transaction around the pair, so the window is reachable by a hook, a
    /// second agent or a terminal.
    ///
    /// Driven straight at `hydrate` rather than through `search`, because
    /// through `search` the stage filters first and nothing would ever reach
    /// the fetch deleted — which is exactly why the missing filter went
    /// unnoticed by every test that goes in the front door.
    #[test]
    fn the_fetch_leaves_behind_what_was_deleted_under_it() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Store::open(StoreConfig::new(temp.path().join("hydrate.db"))).unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        let mut ids = Vec::new();
        for index in 0..3 {
            ids.push(
                store
                    .add_observation(crate::memory::model::AddObservation {
                        session_id: "s1".to_owned(),
                        kind: "discovery".to_owned(),
                        title: format!("Una memoria numero {index} para hidratar"),
                        content: format!("Cuerpo de la memoria {index}."),
                        tool_name: None,
                        project: Some("leteo".to_owned()),
                        scope: "project".to_owned(),
                        topic_key: None,
                        prompt_sync_id: None,
                    })
                    .unwrap()
                    .observation
                    .id,
            );
        }
        let candidatos = |ids: &[i64]| -> Vec<Candidate> {
            ids.iter()
                .map(|id| Candidate {
                    id: *id,
                    kind: "discovery".to_owned(),
                    rank: -1.0,
                    partial: false,
                })
                .collect()
        };

        // Las tres vivas llegan enteras.
        let vivas = store.hydrate(candidatos(&ids)).unwrap();
        assert_eq!(vivas.len(), 3);

        // And the one somebody deleted out from under the ranking is not.
        store.delete_observation(ids[1], false).unwrap();
        let despues = store.hydrate(candidatos(&ids)).unwrap();
        assert_eq!(
            despues.iter().map(|r| r.observation.id).collect::<Vec<_>>(),
            vec![ids[0], ids[2]],
            "una memoria borrada no vuelve de una búsqueda"
        );

        // Un borrado duro tampoco, que es el caso que el comentario anterior
        // creía cubrir con esto y era el único.
        store.delete_observation(ids[0], true).unwrap();
        assert_eq!(store.hydrate(candidatos(&ids)).unwrap().len(), 1);
    }
}
