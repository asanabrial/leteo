//! Finding a memory again: full-text ranking and prompt recall.

use super::*;

/// The stemmed index: what makes a differently inflected question find
/// anything at all.
pub const FTS_STEMMED: &str = "observations_fts";
/// The same memories indexed as they were written, with no stemmer.
pub const FTS_EXACT: &str = "observations_exact";
/// The same memories as their language's Snowball stemmer made them.
///
/// Read with the query stemmed the same way, once for every language the store
/// holds a memory in. See [`crate::stemming`].
pub const FTS_SNOWBALL: &str = "observations_stemmed";

/// The vocabulary of the unstemmed index, as a table this connection reads.
///
/// Created on first use, and only on the one path that needs it — an empty
/// strict pass carrying a word the index does not hold — so a search that
/// answers never pays for it. A TEMP table because the alternative is a schema
/// write on every store, and because `fts5vocab` reads the index directly: it
/// holds no copy, so there is nothing to migrate and no `SCHEMA_VERSION` to
/// bump.
///
/// The three-argument form is not decoration. For a TEMP `fts5vocab`, SQLite
/// resolves the target table in the vocabulary's own schema, so the unqualified
/// form looks for `temp.observations_exact` and fails; the database name has to
/// be passed first. See `fts5VocabInitVtab` in the bundled `sqlite3.c`.
///
/// `pub(super)` so the guard can read the same name the code writes: a test
/// that spelled `fts_vocab` itself would be watching its own copy.
pub(super) const VOCAB_TABLE: &str = "fts_vocab";

/// The most edits a corrected term may be from the word it is read as.
///
/// One for a short word and two for a longer one, because a second edit on five
/// letters or fewer only widens the vocabulary scan: measured on the benchmark's
/// typo set, allowing two edits on short words changes no kind's MRR. The
/// budget bounds the scan rather than deciding which stage answers a fragment —
/// this stage runs after the prefix and substring stages, so a partial word such
/// as `pgxpo` is answered before a correction can reach it.
const TYPO_SHORT_DISTANCE: usize = 1;
const TYPO_LONG_DISTANCE: usize = 2;
/// The length at or below which only [`TYPO_SHORT_DISTANCE`] applies.
const TYPO_SHORT_TERM_CHARS: usize = 5;

/// A term the query asked for that the index does not hold, and the word the
/// vocabulary holds that the search read it as.
///
/// Carried out of the store so both surfaces can name every substitution. The
/// sentence itself is built in one place — see `corrected_terms_hint` — so the
/// tool and the command line cannot come to word it differently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Correction {
    /// The term as the caller wrote it, with its case kept.
    pub asked: String,
    /// The word the unstemmed index holds that the query was taken to mean.
    pub used: String,
}

/// The terms of a query with its unknown words replaced, and what was replaced.
type CorrectedTerms = (Vec<String>, Vec<Correction>);

/// The edit budget for a term of this many characters.
fn typo_budget(chars: usize) -> usize {
    if chars <= TYPO_SHORT_TERM_CHARS {
        TYPO_SHORT_DISTANCE
    } else {
        TYPO_LONG_DISTANCE
    }
}

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
pub(super) const FUSION_CONSTANT: f64 = 60.0;

/// How many a search returns when the caller does not say.
///
/// Named because two surfaces and `search_with_more` all have to agree about
/// it: a page size written out three times is one edit away from a reply that
/// says "there is more" about a list that ended on its own.
pub(crate) const DEFAULT_SEARCH_LIMIT: usize = 10;

/// The most bytes of raw query the store will tokenise.
///
/// The query string had no bound before this, so a pasted log was tokenised
/// whole and the strict stage built one conjunction term per distinct word:
/// 64 KiB of log is thousands of terms joined by `AND`, and the work is paid
/// before any stage can answer. The cap is on the raw bytes, not on the terms,
/// because that is the input a caller controls and the quantity that decides
/// the tokenisation and the size of the built query.
///
/// Measured through this binary's own store on a synthetic 4,000-memory corpus,
/// `All` mode, mostly-distinct tokens, best of five runs. The probe was a
/// temporary `examples/` file, deleted once the number was chosen:
///
/// ```text
///   query bytes   terms   strict pass
///       257        31       3.8 ms
///     1,031       119       7.0 ms
///     8,195       882      18.5 ms
///    32,778     3,304      61.5 ms
///    65,542     6,530     132.5 ms
/// ```
///
/// The cost is close to linear in the terms, so the cap is what bounds it: at
/// 8 KiB the strict pass is under 20 ms on this corpus, while the same query in
/// `Any` mode — bounded at thirty-two terms by `MAX_ANY_TERMS` — costs 3.2 ms at
/// every size. A sentence or short paragraph is well under 1 KiB, so 8 KiB
/// leaves an order of magnitude of margin above any legitimate question and
/// refuses the pasted log that motivated the cap.
///
/// `pub` because the command line's integration test is a separate crate and
/// has to refuse at the same number the store applies rather than a copy of it,
/// the way the store's own test reads the constant. The schema description and
/// `search.md` still state the number literally, because neither can read it.
pub const MAX_QUERY_BYTES: usize = 8192;

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
    let visible = visible_observations(2, 3, 4);
    format!(
        "SELECT o.id, o.type, bm25({index}, {weights}) AS rank
         FROM {index} fts CROSS JOIN observations o ON o.id = fts.rowid
         WHERE {index} MATCH ?1 AND {visible}
         ORDER BY rank LIMIT ?5"
    )
}

/// Which memories a search may return at all, as a `WHERE` fragment over `o`.
///
/// Not deleted, not hidden by a judged verdict, and inside the type, project
/// and scope the caller narrowed to. Every stage that reads `observations` asks
/// this one question — the ranked stages, the title scan, and the semantic
/// stage — so a rule added here reaches all of them, and a stage cannot list a
/// memory the context beside it has hidden. The semantic stage reads vectors
/// rather than the full-text index and was the first to be tempted to restate
/// it.
///
/// The three numbers are the positions of the parameters the caller binds, in
/// the order type, project, scope: the full-text statement binds them as 2, 3
/// and 4 behind its `MATCH`, the others as 1, 2 and 3.
pub(super) fn visible_observations(kind: usize, project: usize, scope: usize) -> String {
    let not_superseded = super::relations::not_superseded();
    format!(
        "o.deleted_at IS NULL
           AND (?{kind} IS NULL OR o.type = ?{kind})
           AND (?{project} IS NULL OR LOWER(o.project) = ?{project})
           AND (?{scope} IS NULL OR o.scope = ?{scope})
           AND {not_superseded}"
    )
}

/// Whether this search leaves session summaries out of its relaxed stages.
///
/// A summary is long and touches everything, so it is the best partial match for
/// almost any question and the right answer to almost none; that is the rule for
/// a query that did not ask for them, measured in `search.md` §6. A query that
/// named the type asked for exactly this, and `visible_observations` already
/// narrows to it — excluding it after that returned nothing at all, which is the
/// contradiction `is_searchable_kind` promised a caller would not hit.
pub(super) fn excludes_summaries(options: &SearchOptions) -> bool {
    options.kind.as_deref() != Some(crate::memory::model::SESSION_SUMMARY)
}

/// The coefficients and scales of the Engram rerank, under measurement rather
/// than adopted.
///
/// Compiled for the measurement and the guard that holds it together, and not
/// in a normal build: the rerank is not wired into any product path yet, so
/// nothing there would use it and the compiler would be right to say so.
///
/// Engram orders a search by
/// `bm25 × (1 + pin·pinned + recency·recent + stability·stable)`, and Leteo as
/// shipped orders by bm25 and reciprocal rank fusion alone. The variant below is
/// that factor applied to the statement [`matching_observations_sql`] builds, so
/// the two orderings can be measured on one corpus through one query builder.
///
/// The sign is the thing to keep in mind, and the query below says it: SQLite's
/// `bm25()` is negative and more negative is better, so a factor of one or more
/// multiplies a good row further from zero and `ORDER BY` ascending puts it
/// first. The weights are positive and the factor is `1 + …` rather than `1 − …`
/// for that reason — the factor boosts.
///
/// `recency` needs a "last seen" Leteo does not have. The row carries
/// `created_at` and `updated_at`, and the later of the two stands in for it.
/// Engram's own `last_seen` moves on every retrieval; Leteo does not write on a
/// read, so `MAX(updated_at, created_at)` is the nearest field and is stated
/// rather than hidden. A row missing both falls back to `now`, so it is treated
/// as current rather than dropped to the top by a NULL sort key.
///
/// The coefficients are named constants so the number measured is the number
/// written down. Whether they ship is a separate decision this does not make.
#[cfg(any(feature = "measure", test))]
pub const RERANK_PIN_WEIGHT: f64 = 0.10;
#[cfg(any(feature = "measure", test))]
pub const RERANK_RECENCY_WEIGHT: f64 = 0.06;
#[cfg(any(feature = "measure", test))]
pub const RERANK_STABILITY_WEIGHT: f64 = 0.04;
/// The recency scale: `1 / (1 + days / 30)`, so a memory touched today scores 1
/// and one untouched for thirty days scores a half.
#[cfg(any(feature = "measure", test))]
pub const RERANK_RECENCY_DAYS: f64 = 30.0;
/// The stability smoothing: `(revisions + duplicates) / (revisions + duplicates
/// + 4)`, so a memory seen once scores a fifth and the term saturates near one.
#[cfg(any(feature = "measure", test))]
pub const RERANK_STABILITY_SMOOTHING: f64 = 4.0;

/// [`matching_observations_sql`], ordered by the Engram rerank instead of bm25.
///
/// The `SELECT`, the join and every `WHERE` clause are the shipped statement's
/// character for character, so the only thing that differs when the two are
/// measured against one corpus is the sort key. A filter that moved would
/// confound the comparison rather than measure the rerank, which is why a test
/// holds the two prefixes together instead of trusting this sentence.
#[cfg(any(feature = "measure", test))]
pub fn matching_observations_reranked_sql(index: &str, weights: &str) -> String {
    let visible = visible_observations(2, 3, 4);
    format!(
        "SELECT o.id, o.type, bm25({index}, {weights}) AS rank
         FROM {index} fts CROSS JOIN observations o ON o.id = fts.rowid
         WHERE {index} MATCH ?1 AND {visible}
         ORDER BY rank * (1.0
           + {RERANK_PIN_WEIGHT} * o.pinned
           + {RERANK_RECENCY_WEIGHT} / (1.0 + (julianday('now') - julianday(
               COALESCE(NULLIF(MAX(COALESCE(o.updated_at, ''), COALESCE(o.created_at, '')), ''), datetime('now'))
             )) / {RERANK_RECENCY_DAYS})
           + {RERANK_STABILITY_WEIGHT} * ((o.revision_count + o.duplicate_count) * 1.0
               / (o.revision_count + o.duplicate_count + {RERANK_STABILITY_SMOOTHING}))
         ) LIMIT ?5"
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
    pub(super) id: i64,
    pub(super) kind: String,
    pub(super) rank: f64,
    pub(super) partial: bool,
    /// Whether the semantic stage put this memory in the answer, as opposed to
    /// the lexical stages having found it.
    pub(super) semantic: bool,
}

/// What one search decided: the page, and any terms it had to correct to get
/// it.
///
/// Private, because the only caller that reads both halves is
/// [`Store::search_with_more_and_corrections`]; every other caller takes the
/// page and drops the corrections.
struct SearchOutcome {
    results: Vec<SearchResult>,
    corrections: Vec<Correction>,
}

/// The number of single-character edits between two words.
///
/// The plain dynamic-programming distance, kept whole rather than banded: a
/// candidate is already bounded by length before this runs, and on a real store
/// the whole vocabulary is 42,538 words — small enough that the budget check
/// after the fact is cheaper than the bookkeeping a banded version needs.
fn levenshtein(left: &str, right: &str) -> usize {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0usize; right.len() + 1];
    for (index, a) in left.iter().enumerate() {
        current[0] = index + 1;
        for (other, b) in right.iter().enumerate() {
            let substitution = previous[other] + usize::from(a != b);
            current[other + 1] = (previous[other + 1] + 1)
                .min(current[other] + 1)
                .min(substitution);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
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
    ///
    /// A caller that has to say which terms were corrected asks
    /// [`Self::search_with_more_and_corrections`] instead; this one throws that
    /// answer away, because most callers have nowhere to put it.
    pub fn search_with_more(
        &self,
        query: &str,
        options: SearchOptions,
    ) -> Result<(Vec<SearchResult>, bool), StoreError> {
        let (found, more, _) = self.search_with_more_and_corrections(query, options)?;
        Ok((found, more))
    }

    /// The same search, with the terms it had to correct to answer.
    ///
    /// Split from its sibling rather than widening it, because the two surfaces
    /// that must name a correction — `mem_search` and `leteo search` — are the
    /// only callers with anywhere to put the list, and every other caller (the
    /// "elsewhere" retry, a test) would carry a value it ignores.
    pub fn search_with_more_and_corrections(
        &self,
        query: &str,
        options: SearchOptions,
    ) -> Result<(Vec<SearchResult>, bool, Vec<Correction>), StoreError> {
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
        let outcome = self.search_limited(query, options, asked.saturating_add(1))?;
        let more = outcome.results.len() > asked;
        let mut found = outcome.results;
        found.truncate(asked);
        Ok((found, more, outcome.corrections))
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
        Ok(self.search_limited(query, options, limit)?.results)
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
    ) -> Result<SearchOutcome, StoreError> {
        if query.trim().is_empty() {
            return Err(StoreError::EmptySearch);
        }
        if query.len() > MAX_QUERY_BYTES {
            return Err(StoreError::QueryTooLong {
                bytes: query.len(),
                cap: MAX_QUERY_BYTES,
            });
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
            let visible = visible_observations(2, 3, 4);
            let mut statement = self.connection.prepare(&format!(
                "SELECT {OBSERVATION_COLUMNS} FROM observations o
                 WHERE o.topic_key = ?1 AND {visible}
                 ORDER BY o.updated_at DESC LIMIT ?5"
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
                    semantic: false,
                });
            }
        }

        let any = options.mode == SearchMode::Any;
        let mut matched =
            self.fused_observations(&normalize::fts_terms(query), any, &options, limit)?;
        // And a word somebody half-remembers, before the question is loosened.
        //
        // The strict pass needs every word whole, so a fragment fails it
        // exactly the way a word the store has never seen does, and the widened
        // retry that follows answers it by dropping the fragment entirely —
        // which finds the memory only if what is left is enough on its own.
        // Opening the words to prefixes is a far smaller claim than dropping
        // one: it says the fragment is the beginning of a word that is there,
        // which is what an agent typing `pgxpo` or `append-onl` means. It runs
        // before the widening for that reason, and its results carry `partial`
        // like every other relaxed stage.
        if matched.is_empty() && results.is_empty() && !any {
            matched = self.prefix_observations(query, &options, limit)?;
        }
        // And a fragment from inside a word, which no prefix can reach.
        //
        // `telemetr` is not the beginning of `OpenTelemetry`, so the prefix
        // stage above cannot match it, and dropping the word leaves nothing to
        // search on. `substring_observations` says what it reads and why.
        if matched.is_empty() && results.is_empty() && !any {
            matched = self.substring_observations(query, &options, limit)?;
        }
        // And a word the store has never seen, read as the word it was meant to
        // be, before the question is loosened.
        //
        // A typo is a different failure from a fragment, and the stages above
        // cannot answer it: the strict pass wants the word whole, and a prefix
        // or a substring asks whether the *typed* letters begin or sit inside a
        // word, which `conection` does not. Measured on the benchmark's typo
        // set, the widening below rescues the ones carrying a single typo and
        // fails every one carrying two, because dropping one bad word leaves
        // the other. Correcting them is the smaller claim: it says this word is
        // that word, one or two edits away, rather than dropping a word and
        // hoping the rest is enough.
        //
        // It runs before the widening and after the fragment stages, so a
        // fragment a prefix already answers is never touched, and only when the
        // strict pass came back empty — a query that matched is never rewritten
        // under the caller. `corrected_fts` refuses the whole correction when
        // any unknown word has no candidate, because a conjunction that still
        // holds an unknown word fails exactly as the original did.
        let mut corrections = Vec::new();
        if matched.is_empty()
            && results.is_empty()
            && !any
            && let Some((corrected, said)) = self.corrected_fts(query)?
        {
            let retried = self.fused_observations(&corrected, any, &options, limit)?;
            if !retried.is_empty() {
                matched = retried;
                corrections = said;
            }
        }
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
        let mut nearest_answered = false;
        if matched.is_empty() && results.is_empty() && !any {
            matched = self.nearest_observations(query, &options, limit)?;
            nearest_answered = !matched.is_empty();
        }
        // And when the words found nothing, or only the weakest of what they
        // can find, the meaning.
        //
        // See `search.md` §15 for what this costs and what it was measured to
        // buy. The shape, in short: on an empty answer it speaks only above a
        // cosine floor, because without one it answers every question, including
        // the ones this store cannot; on a `nearest` answer, which was given
        // anyway and is the weakest lexical claim there is, it is merged in by
        // rank with no floor. Every stage above `nearest` is stronger than
        // anything a cosine can say and is never touched — a memory that
        // matched every word is not improved by a guess about its meaning.
        //
        // Not in `mode: any`, which asked for a disjunction and gets one, the
        // way every other relaxed stage is switched off there. And not when a
        // topic key answered: that is an exact lookup.
        if options.semantic
            && !any
            && results.is_empty()
            && (matched.is_empty() || nearest_answered)
        {
            matched = self.with_semantic_stage(query, &options, limit, matched, nearest_answered);
        }
        // The candidates that survive are the only ones whose body is read.
        matched.retain(|row| !results.iter().any(|item| item.observation.id == row.id));
        matched.truncate(limit.saturating_sub(results.len()));
        results.extend(self.hydrate(matched)?);
        results.truncate(limit);
        Ok(SearchOutcome {
            results,
            corrections,
        })
    }

    /// What the semantic stage makes of an answer the lexical stages gave.
    ///
    /// A stage that cannot run says so and leaves the answer as it was: a
    /// model that will not load, a store it cannot read, a tokenizer that
    /// panicked. The lexical answer stands without it, and refusing a question
    /// that has one over an optional stage would be the wrong trade — but it is
    /// logged at `warn`, not at `debug` as the unreadable second index is,
    /// because unlike that index this has no reason to fail on a healthy build,
    /// and a test holds that it does not.
    fn with_semantic_stage(
        &self,
        query: &str,
        options: &SearchOptions,
        limit: usize,
        lexical: Vec<Candidate>,
        nearest_answered: bool,
    ) -> Vec<Candidate> {
        let floor = (!nearest_answered).then_some(crate::semantic::FLOOR);
        match self.semantic_candidates(query, options, limit, floor) {
            // A `nearest` answer is the weakest lexical claim, and the semantic
            // list beside it is what pads the page: on a question the store
            // cannot answer, one shared word reaches `nearest` and the stage
            // then fills the slots with unrelated memories. The lexical answer
            // stands; the meaning is merged in only as far as it can stand
            // beside it. See `semantic::MERGE_CAP`.
            Ok(mut found) if nearest_answered => {
                found.truncate(crate::semantic::MERGE_CAP);
                super::semantic_stage::fuse(lexical, found, limit)
            }
            Ok(found) => found,
            // No usable model is a state `doctor` reports and `leteo model install`
            // mends, and the stage is simply off; it is not a fault of this search.
            Err(error) if error.is::<crate::semantic::Unavailable>() => {
                tracing::debug!(%error, "the semantic stage is off");
                lexical
            }
            Err(error) => {
                tracing::warn!(%error, "the semantic stage could not run; answering without it");
                lexical
            }
        }
    }

    /// The strict query again, with every word the index has never seen read as
    /// the nearest word it does hold, or nothing when that cannot be done.
    ///
    /// Only the words that match nothing are candidates. A word the index holds
    /// is left exactly as written — correcting it would answer a different
    /// question from the one asked, which is the whole risk of this stage — and
    /// it is the *stemmed* index that decides, not the vocabulary, so an
    /// inflected word the stemmer already reaches (`limitting` for `limit`) is
    /// known and untouched.
    ///
    /// The replacement comes from the *unstemmed* vocabulary, because the word
    /// put back into the query has to be a word somebody could have written:
    /// the stemmed vocabulary holds `limit`, and searching for it would be a
    /// different question again.
    ///
    /// Every unknown word has to be placed, or none is. A conjunction that
    /// still carries one unknown word fails exactly as the original did, so
    /// correcting the rest would cost a vocabulary read and buy no answer while
    /// reporting a substitution that changed nothing.
    fn corrected_fts(&self, query: &str) -> Result<Option<CorrectedTerms>, StoreError> {
        let terms = normalize::fts_terms(query);
        if terms.is_empty() {
            return Ok(None);
        }
        // The vocabulary is read before anything is decided, so that "this
        // stage ran" and "the vocabulary was consulted" are one fact: the guard
        // that a search which already answered never reaches here watches the
        // table this creates.
        //
        // An unreadable vocabulary is not an error, the way an unreadable
        // second index is not one in `fused_observations`: a store that could
        // not build the unstemmed index, or one whose SQLite has no `fts5vocab`
        // module, searches the way it did before this stage existed.
        if let Err(error) = self.ensure_vocabulary() {
            tracing::debug!(%error, "the unstemmed vocabulary is unreadable; searching without correction");
            return Ok(None);
        }
        let mut unknown: Vec<(usize, String)> = Vec::new();
        for (index, term) in terms.iter().enumerate() {
            let word = normalize::unquote_fts_term(term);
            if word.is_empty() || self.term_is_indexed(&word)? {
                continue;
            }
            unknown.push((index, word));
        }
        if unknown.is_empty() {
            return Ok(None);
        }
        let mut corrected = terms;
        let mut said = Vec::with_capacity(unknown.len());
        let folded: Vec<String> = unknown
            .iter()
            .map(|(_, asked)| asked.to_lowercase())
            .collect();
        // The read is guarded as well as the build, because the build does not
        // establish that the index is there: `CREATE VIRTUAL TABLE IF NOT
        // EXISTS ... USING fts5vocab` returns without validating its target, so
        // a store that never had `observations_exact` — or lost it to a
        // half-finished upgrade — fails here, with "no such fts5 table", rather
        // than at the `CREATE`. An unreadable vocabulary is not an error, the
        // way an unreadable second index is not one in `fused_observations`.
        let nearest = match self.nearest_vocabulary_words(&folded) {
            Ok(nearest) => nearest,
            Err(error) => {
                tracing::debug!(%error, "the unstemmed vocabulary is unreadable; searching without correction");
                return Ok(None);
            }
        };
        for ((index, asked), used) in unknown.into_iter().zip(nearest) {
            match used {
                Some(used) => {
                    corrected[index] = normalize::quote_fts_term(&used);
                    said.push(Correction { asked, used });
                }
                // One word the vocabulary cannot place makes the whole
                // corrected conjunction fail exactly as the original did, so
                // there is nothing to gain by correcting the rest and something
                // to lose by reporting a correction that changed no answer.
                None => return Ok(None),
            }
        }
        Ok(Some((corrected, said)))
    }

    /// Whether the stemmed index holds anything for one word.
    ///
    /// The same index the strict pass reads, so a word the stemmer reaches is
    /// known even when the unstemmed vocabulary has never held its spelling.
    /// Deleted memories are excluded, the way every other read excludes them:
    /// a word that survives only in a deleted memory is not one a search can
    /// reach, so it is corrected like any other unknown.
    fn term_is_indexed(&self, word: &str) -> Result<bool, StoreError> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT EXISTS(
                 SELECT 1 FROM {FTS_STEMMED} fts
                 CROSS JOIN observations o ON o.id = fts.rowid
                 WHERE {FTS_STEMMED} MATCH ?1 AND o.deleted_at IS NULL)"
        ))?;
        let known = statement.query_row(params![normalize::quote_fts_term(word)], |row| {
            row.get::<_, bool>(0)
        })?;
        // A word only the Snowball index reaches is known as well: correcting
        // `migraciones` to some other word because `porter` has never seen that
        // stem would answer a different question from the one asked.
        Ok(known
            || !self
                .snowball_candidates(
                    &[normalize::quote_fts_term(word)],
                    false,
                    &SearchOptions::default(),
                    1,
                    false,
                )
                .is_empty())
    }

    /// Creates the vocabulary table this connection reads, once.
    fn ensure_vocabulary(&self) -> Result<(), StoreError> {
        self.connection.execute_batch(&format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS temp.{VOCAB_TABLE}
             USING fts5vocab('main', '{FTS_EXACT}', 'row');"
        ))?;
        Ok(())
    }

    /// The words the unstemmed index holds that are nearest to each of `words`.
    ///
    /// The nearest within each word's own budget, ties broken by the word the
    /// index holds in more memories and then by the lexicographically smaller
    /// one, so the answer does not depend on the order SQLite happens to return
    /// rows in. A word's length bounds its candidates before any distance is
    /// computed, which is what keeps the scan proportional to the words near
    /// the right size rather than to the whole vocabulary.
    ///
    /// One scan of the vocabulary for all the unknown words together, not one
    /// per word: the scan is the expensive part — 21,241 rows in 18 ms on a real
    /// 42,538-term store — and a question with two typos would otherwise pay it
    /// twice.
    fn nearest_vocabulary_words(
        &self,
        words: &[String],
    ) -> Result<Vec<Option<String>>, StoreError> {
        let lengths: Vec<usize> = words.iter().map(|word| word.chars().count()).collect();
        let budgets: Vec<usize> = lengths.iter().map(|length| typo_budget(*length)).collect();
        let low = lengths
            .iter()
            .zip(&budgets)
            .map(|(length, budget)| length.saturating_sub(*budget))
            .min()
            .unwrap_or(1)
            .max(1) as i64;
        let high = lengths
            .iter()
            .zip(&budgets)
            .map(|(length, budget)| length + budget)
            .max()
            .unwrap_or(0) as i64;
        let mut best: Vec<Option<(usize, i64, String)>> = vec![None; words.len()];
        let mut statement = self.connection.prepare(&format!(
            "SELECT term, doc FROM temp.{VOCAB_TABLE} WHERE length(term) BETWEEN ?1 AND ?2"
        ))?;
        let rows = statement.query_map(params![low, high], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (term, doc) = row?;
            let term_length = term.chars().count();
            for (index, word) in words.iter().enumerate() {
                if term_length.abs_diff(lengths[index]) > budgets[index] {
                    continue;
                }
                let distance = levenshtein(word, &term);
                if distance > budgets[index] {
                    continue;
                }
                let better = match &best[index] {
                    None => true,
                    Some((best_distance, best_doc, best_term)) => {
                        distance < *best_distance
                            || (distance == *best_distance
                                && (doc > *best_doc || (doc == *best_doc && term < *best_term)))
                    }
                };
                if better {
                    best[index] = Some((distance, doc, term.clone()));
                }
            }
        }
        Ok(best
            .into_iter()
            .map(|slot| slot.map(|(_, _, term)| term))
            .collect())
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
        terms: &[String],
        any: bool,
        options: &SearchOptions,
        limit: usize,
    ) -> Result<Vec<Candidate>, StoreError> {
        let fts = normalize::fts_join(terms, any);
        // Deeper than the answer, so the merge has places to compare. A memory
        // ninth in one list and second in the other is exactly the case this
        // exists for, and it cannot be seen from two lists of three.
        let depth = (limit * 3).max(30);
        let stemmed = self.matching_observations(FTS_STEMMED, &fts, options, depth, false)?;
        let exact = match self.matching_observations(FTS_EXACT, &fts, options, depth, false) {
            Ok(exact) => exact,
            Err(error) => {
                tracing::debug!(%error, "the unstemmed index is unreadable; searching the stemmed one alone");
                Vec::new()
            }
        };
        let snowball = self.snowball_candidates(terms, any, options, depth, false);
        if exact.is_empty() && snowball.is_empty() {
            let mut stemmed = stemmed;
            stemmed.truncate(limit);
            return Ok(stemmed);
        }

        let mut fused: BTreeMap<i64, f64> = BTreeMap::new();
        let mut rows: BTreeMap<i64, Candidate> = BTreeMap::new();
        for list in [stemmed, exact, snowball] {
            for (place, result) in list.into_iter().enumerate() {
                *fused.entry(result.id).or_default() +=
                    1.0 / (FUSION_CONSTANT + place as f64 + 1.0);
                // The stemmed list is walked first, so its score is the one
                // kept for a memory more than one index found.
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

    /// The languages the store holds a memory in that have a Snowball stemmer.
    ///
    /// Asked of the rows rather than of the setting, because a store keeps the
    /// memories it was written with: a person who switched from Spanish to
    /// English last month still has Spanish memories to find, and the setting no
    /// longer says so. The question is over a small indexed column and is asked
    /// by every stage that reads the Snowball index, which is a few times a
    /// search that falls through to the relaxed stages and once for one the
    /// strict pass answers.
    fn snowball_languages(&self) -> Vec<crate::settings::Interface> {
        let read = || -> Result<Vec<String>, rusqlite::Error> {
            let mut statement = self
                .connection
                .prepare_cached("SELECT DISTINCT language FROM observation_stems")?;
            statement.query_map([], |row| row.get(0))?.collect()
        };
        match read() {
            Ok(codes) => codes
                .iter()
                .filter_map(|code| crate::stemming::from_code(code))
                .filter(|language| crate::stemming::algorithm(*language).is_some())
                .collect(),
            Err(error) => {
                tracing::debug!(%error, "the stems are unreadable; searching without them");
                Vec::new()
            }
        }
    }

    /// What the Snowball index makes of the same terms, best first.
    ///
    /// Each language present in the store stems the query its own way and the
    /// answers are merged on a memory's best rank. Not an error when it cannot
    /// run, the way an unreadable unstemmed index is not one: the stemmed index
    /// answers alone, as it did before this one existed.
    fn snowball_candidates(
        &self,
        terms: &[String],
        any: bool,
        options: &SearchOptions,
        limit: usize,
        partial: bool,
    ) -> Vec<Candidate> {
        let mut best: BTreeMap<i64, Candidate> = BTreeMap::new();
        for language in self.snowball_languages() {
            let stemmed: Vec<String> = terms
                .iter()
                .map(|term| {
                    let stems =
                        crate::stemming::stem_text(&normalize::unquote_fts_term(term), language);
                    if stems.is_empty() {
                        term.clone()
                    } else {
                        normalize::quote_fts_term(&stems)
                    }
                })
                .collect();
            let fts = normalize::fts_join(&stemmed, any);
            match self.matching_observations(FTS_SNOWBALL, &fts, options, limit, partial) {
                Ok(found) => {
                    for candidate in found {
                        match best.entry(candidate.id) {
                            std::collections::btree_map::Entry::Occupied(mut seen) => {
                                if candidate.rank < seen.get().rank {
                                    seen.insert(candidate);
                                }
                            }
                            std::collections::btree_map::Entry::Vacant(empty) => {
                                empty.insert(candidate);
                            }
                        }
                    }
                }
                Err(error) => {
                    tracing::debug!(%error, "the Snowball index is unreadable; searching without it");
                }
            }
        }
        let mut found: Vec<Candidate> = best.into_values().collect();
        found.sort_by(|left, right| left.rank.total_cmp(&right.rank));
        found.truncate(limit);
        found
    }

    /// `porter` and Snowball answers to one set of terms, as one list.
    ///
    /// For the relaxed stages, which keep a memory's best rank rather than fuse
    /// places: they decide on bm25 against a floor, and a memory only the
    /// Snowball index reaches must be able to clear it.
    fn stemmed_candidates(
        &self,
        terms: &[String],
        any: bool,
        options: &SearchOptions,
        limit: usize,
    ) -> Result<Vec<Candidate>, StoreError> {
        let fts = normalize::fts_join(terms, any);
        let mut found = self.matching_observations(FTS_STEMMED, &fts, options, limit, true)?;
        for candidate in self.snowball_candidates(terms, any, options, limit, true) {
            match found.iter_mut().find(|seen| seen.id == candidate.id) {
                Some(seen) if candidate.rank < seen.rank => *seen = candidate,
                Some(_) => {}
                None => found.push(candidate),
            }
        }
        found.sort_by(|left, right| left.rank.total_cmp(&right.rank));
        found.truncate(limit);
        Ok(found)
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
        let terms = normalize::fts_terms(query);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let mut candidates = self.stemmed_candidates(&terms, true, options, RECALL_SAMPLE)?;
        if excludes_summaries(options) {
            candidates.retain(|candidate| candidate.kind != crate::memory::model::SESSION_SUMMARY);
        }
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

    /// Every word as a prefix, for a fragment the strict pass cannot match.
    ///
    /// The one relaxed stage that adds no word and drops no word: it asks
    /// whether each word as typed begins a word the index holds. That is the
    /// question behind a half-remembered identifier — `storyb` for `storybook`,
    /// `append-onl` for `append-only` — and it is why it runs before the widened
    /// retry rather than after it: dropping a word is a much larger claim about
    /// what somebody meant than opening one is.
    ///
    /// The stemmed index alone, like the widened and nearest stages. A prefix is
    /// matched against the tokens the index actually holds, and the stemmed
    /// index is where a fragment meets the stem of the word it belongs to —
    /// `throttl` against the `throttling` Porter reduced to `throttl`. The
    /// unstemmed index is not read here: it is what the strict pass fuses in to
    /// prefer an exact word, and this stage's question is only whether a
    /// fragment begins a word at all.
    ///
    /// Session summaries are left out, the rule the widened and nearest stages
    /// keep: a summary is long and touches everything, so it is the best
    /// fragment match for a loosened question and the right answer to almost
    /// none. A question whose words genuinely name what a session did is
    /// answered by the strict pass, which keeps them.
    fn prefix_observations(
        &self,
        query: &str,
        options: &SearchOptions,
        limit: usize,
    ) -> Result<Vec<Candidate>, StoreError> {
        let terms = normalize::fts_prefix_terms(query);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let mut matched = self.matching_observations(FTS_STEMMED, &terms, options, limit, true)?;
        if excludes_summaries(options) {
            matched.retain(|result| result.kind != crate::memory::model::SESSION_SUMMARY);
        }
        matched.sort_by(|left, right| left.rank.total_cmp(&right.rank));
        matched.truncate(limit);
        Ok(matched)
    }

    /// A fragment from inside a word, which no prefix query can reach.
    ///
    /// `telemetr` is not the beginning of `OpenTelemetry`, so the prefix stage
    /// cannot match it, and dropping the word leaves nothing to search on. This
    /// is the one stage that asks whether a title *contains* a word's fragment,
    /// which is the question behind an identifier somebody half-remembers.
    ///
    /// Over titles, and only titles. A title is where an identifier or a name
    /// lives and it is a fraction of a memory's size, so the scan this needs
    /// stays bounded — over bodies it would read everything the store holds on
    /// every question that reached this far. A trigram index is the indexed way
    /// to ask the same thing, and it was built and measured before this was
    /// written: `tokenize = 'trigram'` over title and content added 18.5 MB to a
    /// 9.3 MB corpus, twice the text, for a stage that runs only once every
    /// indexed stage has already found nothing. See `search.md` for the
    /// measurement.
    ///
    /// Every term has to be inside the title, never any of them. A disjunction
    /// here would answer a question the widened retry is about to answer
    /// properly with whichever title shares one common word — the noise the
    /// widened stage's own note is about.
    fn substring_observations(
        &self,
        query: &str,
        options: &SearchOptions,
        limit: usize,
    ) -> Result<Vec<Candidate>, StoreError> {
        // The same words the hint searches on: the alphanumeric ones, lowercased
        // and deduplicated, with fragments under three characters dropped. A
        // one- or two-character fragment is inside almost every title and would
        // turn this into a scan that always answers.
        let terms = normalize::prompt_terms(query);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let mut conditions = String::new();
        for index in 0..terms.len() {
            conditions.push_str(&format!(" AND instr(lower(o.title), ?{}) > 0", index + 4));
        }
        let visible = visible_observations(1, 2, 3);
        let sql = format!(
            "SELECT o.id, o.type FROM observations o
              WHERE {visible}{conditions}
              ORDER BY datetime(o.created_at) DESC, o.id DESC LIMIT ?{}",
            terms.len() + 4
        );
        let mut values: Vec<rusqlite::types::Value> = vec![
            options.kind.clone().into(),
            options.project.clone().into(),
            options.scope.clone().into(),
        ];
        for term in &terms {
            values.push(term.clone().into());
        }
        values.push((limit as i64).into());
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(values), |row| {
            Ok(Candidate {
                id: row.get("id")?,
                kind: row.get("type")?,
                rank: 0.0,
                partial: true,
                semantic: false,
            })
        })?;
        let mut matched = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)?;
        if excludes_summaries(options) {
            matched.retain(|result| result.kind != crate::memory::model::SESSION_SUMMARY);
        }
        matched.truncate(limit);
        Ok(matched)
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
            let kept: Vec<String> = terms
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != omitted)
                .map(|(_, term)| term.clone())
                .collect();
            for result in self.stemmed_candidates(&kept, false, options, limit)? {
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
        // what a session did should still find that session, and a query that
        // names the type is asking for them, so both keep them.
        if excludes_summaries(options) {
            matched.retain(|result| result.kind != crate::memory::model::SESSION_SUMMARY);
        }
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
                    semantic: false,
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
                    semantic: candidate.semantic,
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
///
/// It keeps excluding summaries even though a typed search no longer must: the
/// hint guesses what this conversation already knows, and §6's measurement is
/// that a summary heads almost none of those. A prompt carries no `type`, so
/// there is nothing here for a caller to name.
pub(crate) fn prompt_recall_sql() -> String {
    let not_superseded = super::relations::not_superseded();
    format!(
        "SELECT o.id, ifnull(o.sync_id, '') AS sync_id, o.type, o.title,
                bm25(observations_fts, {BM25_WEIGHTS}) AS rank
         FROM observations_fts fts CROSS JOIN observations o ON o.id = fts.rowid
         WHERE observations_fts MATCH ?1 AND o.deleted_at IS NULL
           AND LOWER(o.project) = ?2
           AND o.type <> 'session_summary'
           AND trim(ifnull(o.title, '')) <> ''
           AND {not_superseded}
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
                    semantic: false,
                })
                .collect()
        };

        let vivas = store.hydrate(candidatos(&ids)).unwrap();
        assert_eq!(vivas.len(), 3);

        store.delete_observation(ids[1], None, false).unwrap();
        let despues = store.hydrate(candidatos(&ids)).unwrap();
        assert_eq!(
            despues.iter().map(|r| r.observation.id).collect::<Vec<_>>(),
            vec![ids[0], ids[2]],
            "una memoria borrada no vuelve de una búsqueda"
        );

        store.delete_observation(ids[0], None, true).unwrap();
        assert_eq!(store.hydrate(candidatos(&ids)).unwrap().len(), 1);
    }
}
