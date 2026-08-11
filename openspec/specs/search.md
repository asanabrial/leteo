# Search

## Purpose

Finding a memory again from words somebody half-remembers. Search is the reason
the store exists: a memory that cannot be found is a memory that was not saved.
This spec covers matching, ranking, widening, and the narrowings that apply
before any of it.

## Behaviour

1. **A query that is a topic key is answered as one.** If the query normalises to
   something containing `/`, the memories under that exact key are returned
   first, ahead of anything ranked. The lookup normalises the query the same way
   the key was normalised on the way in, so `Architecture/Wizard-Split` and
   `architecture/wizard-split` are the same question.

2. **Two indexes, fused.** Every memory is indexed twice: stemmed
   (`porter unicode61`) so that *migrating* finds *migration*, and unstemmed
   (`unicode61`) so that an exact word beats a stem of it. The two result lists
   are merged by reciprocal rank fusion — a memory is worth `1 / (60 + place)`
   in each list it appears in, and the sum orders the answer.

3. **Three stages, in order, stopping at the first that answers.**
   1. every word must match;
   2. failing that, all but one of them;
   3. failing that, any of them, kept only above a floor relative to the median
      rank of what came back.
   Requiring every word is the right first answer, but it fails completely
   rather than partially: one word the store has never seen takes the whole
   question down. Measured over two hundred questions drawn from the titles of a
   real 2,643-memory store, that happened to 4% of short questions and 12% of
   long ones, and the widened retry found the memory every time, at rank one
   every time.

   **The third stage's floor is dimensionless, and that is not a defect to be
   tuned away.** It knows what an ordinary match looks like for this query; it
   does not know whether the project holds an answer. Asked questions belonging
   to another project, it still speaks 90.2% of the time, against 89.2% for
   questions of its own — the control speaks *more*. Three alternatives were
   swept over their whole useful range against 296 home and 399 foreign prompts,
   scoped as reads always are to one project's memories: an absolute bm25 cut,
   a minimum distance below the median, and the shipped ratio itself. None
   separates the two by enough to be worth its cost. The widest lead the score
   ever gives home is 4.8 points, which is about 1.4 standard errors at those
   sample sizes and was picked out of dozens of thresholds, and reaching it
   costs 17 points of real coverage. Word coverage does separate them — twelve
   points, at 3.4 standard errors, which is why the widened stage above leans on
   it — but only at 4.1 points of coverage per point of separation. Do not
   re-tune these floors expecting the control to fall; the finding is that
   nothing lexical is priced within reach, and the answers say `partial` because
   that is the honest thing to put on them.

4. **A widened answer says it is widened, and an empty one says why it is
   empty.** Results that matched only some of the words carry `partial: true`,
   and the answer carries a hint saying so. An empty answer has two possible
   reasons that call for opposite actions — the store has never heard of this,
   or it is filed in another project — and names the right one: where the
   project was inferred from the directory, the same question is asked once
   more unnarrowed, and only if *that* finds something does the reason change.
   Both surfaces share the sentence; each names its own way of widening
   (`--all-projects`, `all_projects`). The retry is paid only on an empty
   answer and only when nobody named a project: measured at 1.1 ms for a search
   that answers and 4.5 ms for one that comes back empty and asks again.

5. **A page that was cut says which limit cut it.** Two things can end a list
   short of what matched: the store's own maximum, and the limit the caller
   asked for. Both are said, and they are different sentences — a page the
   maximum ended must not advise asking again with a higher limit, which is the
   one remedy that cannot work. Which sentence applies is decided by what came
   back and not by what was requested: a request for fifty that matches exactly
   twenty is a complete answer and explains nothing. Both surfaces say it —
   `mem_search` in the reply, `leteo search` on stderr.

   Answered by fetching one row past what was asked and throwing it away —
   counting the matches would mean running the stages again for a number nobody
   reads. That probe row is the single caller allowed past the store's maximum;
   clamped like every other request, it stops existing at exactly the limit that
   decides the sentence, and a full page at the cap says nothing at all. Over
   sixty real questions, eighteen came back with exactly the default ten and
   seventeen of those had more.

6. **Session summaries are excluded from the widened stages.** A summary is long
   and touches everything, which makes it the best partial match for almost any
   question and the right answer to almost none. Measured before the fix: 6
   strict-pass answers led by 0 summaries, against 74 relaxed answers led by 54.
   Strict matches still return summaries — if every word is in one, it is the
   answer.

7. **Every narrowing is normalised before it is compared.** Project, scope, and
   type are folded on the way in exactly as they were folded when the memory was
   written. See [`memory-model.md`](memory-model.md) §3.

   And a blank narrowing is no narrowing, which is the same fold asked a
   different question. `project` had it right; the two beside it did not.
   `scope: ""` went through the fold that puts anything unrecognised onto
   `project`, so an empty filter quietly narrowed the answer to project scope.
   `type: ""` narrowed it to a type no memory has, and the empty result came
   back with the hint that blames the words — the one explanation that was not
   true. Four reads shared the fold and all four now trim first.

8. **Reads are scoped to a project by default.** A search run from a directory
   answers for that directory's project and says how many memories the rest of
   the store holds; `--all-projects` widens it. A read that silently answers from
   another project is worse than an empty one — 72% of the CLI's answers did,
   before the reads were scoped. Guarded on both surfaces now: two projects
   holding the same distinctive word, and neither the tools nor the commands may
   reach across unless asked. The widening is asserted too, so a store that
   answers nothing cannot pass.

9. **Deleted memories are never returned.** See
   [`memory-model.md`](memory-model.md) §8.

10. **A disjunction is bounded; a conjunction is not.** Any stage that joins a
   query's words with `OR` — stage 3 above, `mode: any` from the tool and the
   command line, and the per-prompt hint — takes at most the first thirty-two,
   from one constant that all of them read. A conjunction stays unbounded,
   because two hundred terms joined by `AND` match almost nothing and cost
   almost nothing to find out, while cutting them would answer a different
   question from the one somebody quoted.

   The bound was the hint's alone. Stage 3 documents itself as running the
   hint's own rule and then built its terms with the unbounded helper, so a
   pasted paragraph became one `OR` per word. Over 200 real prompts of one
   project on a 4,016-memory store, in-process through this crate's own search:

   ```text
                            total     p90    silent    silent on another
                                                here     project's prompts
     unbounded             3,323ms  52.6ms   71/200            91/200
     bounded at 64         2,353ms  23.4ms   45/200            91/200
     bounded at 32         1,974ms  17.0ms   40/200            91/200
   ```

   It is faster and it answers more of the questions it should: with a hundred
   words `OR`ed together everything matches something, the sample's scores
   flatten, and nothing clears a floor that is the median of what the query
   found. The right-hand column is the control — prompts from a project this
   one cannot answer, where silence is the right answer — and it does not move,
   so the bound buys the time and the precision without making the stage louder
   where it should say nothing.

   Two neighbouring changes were measured and are *not* taken. Dropping words
   under three characters, which the hint's own term list does, cuts the same
   query from 1,352 matches to 251 and looks like more of the same win — it
   takes the control from 91 silences out of 200 to none at all, which is a
   stage that always speaks. And the double quoting on this path (`fts_terms`
   quotes, `fts_any_of` quotes again) changes nothing: both forms match the
   same 1,352 rows, because FTS5 tokenises the quoted string and the inner
   quotes fall out with the rest of the punctuation.

11. **The stages rank ids; the answer fetches the rows.** Every stage reads
   deeper than it returns — three times the limit so the fusion has places to
   compare, a sample wide enough to have a median, one query per omitted term —
   so most of what it reads it throws away. Reading whole rows to do that moved
   9.8 MB of memory bodies through the row mapper to show 392 memories over
   200 real prompts of one project: 91% of it discarded unread. The stages now
   carry an id, a type and a score, and the survivors' rows are fetched once at
   the end, which is 9% off the whole search (1,967ms and 1,923ms against
   1,770ms and 1,766ms, alternated over the same prompts) for the same answer.

   `WHERE id IN (…)` does not promise an order, and SQLite will answer it by
   rowid, so the fetch restores the ranking's own. An answer sorted by id looks
   entirely reasonable and is the wrong memory first, which is why the guard's
   fixture ranks against the ids rather than with them.

   The fetch carries the ranking's narrowing as well as its order. The stages
   ask for live memories, so nothing deleted can reach the fetch — until
   something is deleted *between* the two queries, which splitting them created
   and no transaction covers. Leteo is multi-writer by design, a soft delete
   leaves the row exactly where `IN` finds it, and deleted memories are never
   returned ([`memory-model.md`](memory-model.md) §8). The guard drives
   `hydrate` directly, because through the front door the stage filters first
   and nothing would ever arrive deleted — which is why every test that goes in
   that way stayed green while the filter was missing.

12. **An empty answer's "elsewhere" count says which of the two it is.** When a
   read narrowed by the directory comes back empty, the reply says how many the
   same question found with the narrowing lifted — and that number has a
   ceiling, which the sentence now names. The opening block and `mem_context`
   count memories outside the project up to a hundred, so a hundred means "a
   hundred or more". A search counts the page that came back, which is the
   caller's own limit: on a query matching 332 memories in other projects it
   said "1 elsewhere" at `limit: 1`, "3" at 3, "10" at 10 and "20" at 20. The
   number was the question restated, and an agent reading "10 elsewhere" had
   been told that widening yields ten.

   The count still comes from running the search again rather than from
   something cheaper, and that was measured rather than assumed. Over 20 empty
   questions from a real store the hint fires on 8; a count of memories
   matching every word elsewhere fires on none, and one matching any word fires
   on all 20 — the relevance floor inside the search is what makes the sentence
   worth saying. It costs 128% of the empty answer, 12.2ms against 28.5ms, with
   the project named explicitly as the control so that the three stages are the
   same on both sides of the comparison.

## Invariants

- The index is kept level with its table by triggers and by nothing else.
  Losing one is silent — an edited row leaves both row counts equal — so
  `doctor` calls the roll of triggers by name. See
  [`store-and-schema.md`](store-and-schema.md) §6.
- An external-content FTS5 table does not notice a plain `UPDATE`. Any migration
  that rewrites `observations` rebuilds the indexes afterwards.
- Ranking transfers between SQLite builds; timing does not, and neither does
  query *construction*. A measurement of search quality made anywhere other than
  through this binary's own query builder is a measurement of something else.

## Where it lives

- `src/store/search.rs` — the stages, the fusion, the floors
- `src/memory/normalize.rs` — `fts_query`, `topic_key`, and the narrowing folds
- `src/store/schema.rs` — the two indexes and the triggers that feed them
- `src/store/tests/search.rs` — the stage-by-stage tests

## Related

- [`memory-model.md`](memory-model.md) — the fields being matched
- [`store-and-schema.md`](store-and-schema.md) — the indexes, triggers, and `doctor`
- [`mcp-tools.md`](mcp-tools.md) — `mem_search`, and the hints §4 describes
- [`hooks.md`](hooks.md) — the prompt nudge, which is a search nobody asked for
